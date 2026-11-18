use crate::{
    Arena, Key, Owner, ReactiveError, Result, current_owner, hold_resource, release_resource,
};
use std::{
    cell::RefCell,
    collections::VecDeque,
    future::Future,
    pin::Pin,
    rc::{Rc, Weak},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    task::{Context, Poll, Wake, Waker},
    thread::{self, JoinHandle},
};

type LocalFuture = Pin<Box<dyn Future<Output = Result<()>>>>;
struct Task {
    owner: Owner,
    future: Option<LocalFuture>,
    wake: Arc<TaskWake>,
    resource: Option<Key>,
}
struct ExecutorState {
    tasks: Arena<Task>,
    ready: Arc<ReadyQueue>,
    polling: bool,
    closed: bool,
}
struct ReadyQueue {
    keys: Mutex<VecDeque<Key>>,
    closed: AtomicBool,
    notify: Box<dyn Fn() + Send + Sync>,
}
struct TaskWake {
    key: Key,
    ready: Arc<ReadyQueue>,
    queued: AtomicBool,
}
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poison) => poison.into_inner(),
    }
}
impl Wake for TaskWake {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        if self.ready.closed.load(Ordering::Acquire) || self.queued.swap(true, Ordering::AcqRel) {
            return;
        }
        {
            let mut queue = lock(&self.ready.keys);
            if self.ready.closed.load(Ordering::Acquire) {
                return;
            }
            queue.push_back(self.key);
        }
        if !self.ready.closed.load(Ordering::Acquire) {
            (self.ready.notify)();
        }
    }
}
thread_local! {static CURRENT:RefCell<Weak<RefCell<ExecutorState>>>=const {RefCell::new(Weak::new())};}
struct EnterGuard(Weak<RefCell<ExecutorState>>);
impl Drop for EnterGuard {
    fn drop(&mut self) {
        let _ = CURRENT.try_with(|current| {
            if let Ok(mut current) = current.try_borrow_mut() {
                *current = self.0.clone();
            }
        });
    }
}
struct PollGuard(Rc<RefCell<ExecutorState>>);
impl Drop for PollGuard {
    fn drop(&mut self) {
        if let Ok(mut state) = self.0.try_borrow_mut() {
            state.polling = false;
        }
    }
}
struct ScopeTask {
    executor: Weak<RefCell<ExecutorState>>,
    key: Key,
}
impl Drop for ScopeTask {
    fn drop(&mut self) {
        if let Some(executor) = self.executor.upgrade() {
            let _ = cancel(&executor, self.key);
        }
    }
}

/// A local task handle. The owning scope and executor control task lifetime.
#[derive(Clone)]
pub struct LocalTask {
    executor: Weak<RefCell<ExecutorState>>,
    key: Key,
}
impl LocalTask {
    /// Cancels pending work and drops its future; returns false after completion.
    pub fn cancel(&self) -> Result<bool> {
        match self.executor.upgrade() {
            Some(executor) => cancel(&executor, self.key),
            None => Ok(false),
        }
    }
    /// Reports whether the task has completed, been canceled, or lost its executor.
    pub fn is_finished(&self) -> Result<bool> {
        let Some(executor) = self.executor.upgrade() else {
            return Ok(true);
        };
        let state = executor
            .try_borrow()
            .map_err(|_| ReactiveError::BorrowConflict)?;
        Ok(state.tasks.get(self.key).is_err())
    }
}
fn cancel(executor: &Rc<RefCell<ExecutorState>>, key: Key) -> Result<bool> {
    let task = {
        let mut state = executor
            .try_borrow_mut()
            .map_err(|_| ReactiveError::BorrowConflict)?;
        state.tasks.remove(key)
    };
    let Some(task) = task else {
        return Ok(false);
    };
    if let Some(resource) = task.resource {
        release_resource(resource)?;
    }
    drop(task);
    Ok(true)
}

/// A single-threaded executor driven only by wake notifications.
pub struct LocalExecutor {
    state: Rc<RefCell<ExecutorState>>,
}
/// Work and task errors produced by a bounded executor poll.
#[derive(Debug, Default)]
pub struct PollReport {
    /// Futures actually polled.
    pub polled: usize,
    /// Futures which completed during this poll.
    pub completed: usize,
    /// Tasks canceled because their scopes had already been disposed.
    pub canceled: usize,
    /// Remaining live tasks, including sleeping tasks.
    pub pending: usize,
    /// Whether ready work remains after the budget.
    pub more_ready: bool,
    /// Failures returned by completed local futures.
    pub errors: Vec<ReactiveError>,
}
impl LocalExecutor {
    /// Creates an executor whose thread-safe wake callback schedules its window.
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Self {
        let ready = Arc::new(ReadyQueue {
            keys: Mutex::new(VecDeque::new()),
            closed: AtomicBool::new(false),
            notify: Box::new(wake),
        });
        Self {
            state: Rc::new(RefCell::new(ExecutorState {
                tasks: Arena::default(),
                ready,
                polling: false,
                closed: false,
            })),
        }
    }
    /// Makes this executor current for free `spawn_local` calls during a closure.
    pub fn enter<R>(&self, f: impl FnOnce() -> Result<R>) -> Result<R> {
        let previous = CURRENT
            .try_with(|current| {
                let mut current = current
                    .try_borrow_mut()
                    .map_err(|_| ReactiveError::BorrowConflict)?;
                let previous = current.clone();
                *current = Rc::downgrade(&self.state);
                Ok(previous)
            })
            .map_err(|_| ReactiveError::RuntimeUnavailable)??;
        let guard = EnterGuard(previous);
        let result = f();
        drop(guard);
        result
    }
    /// Spawns a non-Send future owned by the current reactive scope.
    pub fn spawn_local(
        &self,
        future: impl Future<Output = Result<()>> + 'static,
    ) -> Result<LocalTask> {
        spawn_on(&self.state, Box::pin(future))
    }
    /// Polls up to `budget` ready entries, without polling sleeping tasks.
    pub fn poll_ready(&self, budget: usize) -> Result<PollReport> {
        if budget == 0 {
            return Err(ReactiveError::InvalidBudget);
        }
        let ready = {
            let mut state = self
                .state
                .try_borrow_mut()
                .map_err(|_| ReactiveError::BorrowConflict)?;
            if state.closed {
                return Err(ReactiveError::ExecutorUnavailable);
            }
            if state.polling {
                return Err(ReactiveError::ReentrantPoll);
            }
            state.polling = true;
            Arc::clone(&state.ready)
        };
        let _poll = PollGuard(Rc::clone(&self.state));
        let mut report = PollReport::default();
        for _ in 0..budget {
            let Some(key) = lock(&ready.keys).pop_front() else {
                break;
            };
            let current = {
                let mut state = self
                    .state
                    .try_borrow_mut()
                    .map_err(|_| ReactiveError::BorrowConflict)?;
                match state.tasks.get_mut(key) {
                    Ok(task) => {
                        task.wake.queued.store(false, Ordering::Release);
                        task.future
                            .take()
                            .map(|future| (task.owner, Arc::clone(&task.wake), future))
                    }
                    Err(_) => None,
                }
            };
            let Some((owner, wake, mut future)) = current else {
                continue;
            };
            report.polled += 1;
            let waker = Waker::from(wake);
            let mut context = Context::from_waker(&waker);
            let result = self
                .enter(|| owner.run(|| crate::untrack(|| Ok(future.as_mut().poll(&mut context)))));
            match result {
                Ok(Poll::Pending) => {
                    let mut pending = Some(future);
                    {
                        let mut state = self
                            .state
                            .try_borrow_mut()
                            .map_err(|_| ReactiveError::BorrowConflict)?;
                        if let Ok(task) = state.tasks.get_mut(key) {
                            task.future = pending.take();
                        }
                    }
                    drop(pending);
                }
                Ok(Poll::Ready(result)) => {
                    cancel(&self.state, key)?;
                    report.completed += 1;
                    if let Err(error) = result {
                        report.errors.push(error);
                    }
                }
                Err(ReactiveError::Disposed) => {
                    cancel(&self.state, key)?;
                    report.canceled += 1;
                }
                Err(error) => {
                    cancel(&self.state, key)?;
                    report.errors.push(error);
                }
            }
        }
        report.pending = self
            .state
            .try_borrow()
            .map_err(|_| ReactiveError::BorrowConflict)?
            .tasks
            .len();
        report.more_ready = !lock(&ready.keys).is_empty();
        if report.more_ready {
            (ready.notify)();
        }
        Ok(report)
    }
}
impl Drop for LocalExecutor {
    fn drop(&mut self) {
        let tasks = if let Ok(mut state) = self.state.try_borrow_mut() {
            state.closed = true;
            state.ready.closed.store(true, Ordering::Release);
            lock(&state.ready.keys).clear();
            let keys = state
                .tasks
                .slots
                .iter()
                .enumerate()
                .filter_map(|(index, slot)| {
                    slot.value.as_ref().map(|_| Key {
                        index,
                        generation: slot.generation,
                    })
                })
                .collect::<Vec<_>>();
            keys.into_iter()
                .filter_map(|key| state.tasks.remove(key))
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        for task in tasks {
            if let Some(resource) = task.resource {
                let _ = release_resource(resource);
            }
            drop(task);
        }
    }
}
fn spawn_on(executor: &Rc<RefCell<ExecutorState>>, future: LocalFuture) -> Result<LocalTask> {
    let owner = current_owner()?;
    let key = {
        let mut state = executor
            .try_borrow_mut()
            .map_err(|_| ReactiveError::BorrowConflict)?;
        if state.closed {
            return Err(ReactiveError::ExecutorUnavailable);
        }
        let placeholder = Key {
            index: usize::MAX,
            generation: 0,
        };
        let wake = Arc::new(TaskWake {
            key: placeholder,
            ready: Arc::clone(&state.ready),
            queued: AtomicBool::new(false),
        });
        let key = state.tasks.insert(Task {
            owner,
            future: Some(future),
            wake,
            resource: None,
        });
        let ready = Arc::clone(&state.ready);
        state.tasks.get_mut(key)?.wake = Arc::new(TaskWake {
            key,
            ready,
            queued: AtomicBool::new(false),
        });
        key
    };
    let guard = ScopeTask {
        executor: Rc::downgrade(executor),
        key,
    };
    let resource = match hold_resource(owner, Box::new(guard)) {
        Ok(resource) => resource,
        Err(error) => {
            let _ = cancel(executor, key);
            return Err(error);
        }
    };
    let wake = {
        let mut state = executor
            .try_borrow_mut()
            .map_err(|_| ReactiveError::BorrowConflict)?;
        let task = state.tasks.get_mut(key)?;
        task.resource = Some(resource);
        Arc::clone(&task.wake)
    };
    wake.wake_by_ref();
    Ok(LocalTask {
        executor: Rc::downgrade(executor),
        key,
    })
}
/// Spawns on the executor entered by `LocalExecutor::enter` or its current poll.
pub fn spawn_local(future: impl Future<Output = Result<()>> + 'static) -> Result<LocalTask> {
    let executor = CURRENT
        .try_with(|current| {
            current
                .try_borrow()
                .map_err(|_| ReactiveError::BorrowConflict)?
                .upgrade()
                .ok_or(ReactiveError::ExecutorUnavailable)
        })
        .map_err(|_| ReactiveError::RuntimeUnavailable)??;
    spawn_on(&executor, Box::pin(future))
}

/// Bounded background execution failures.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum BackgroundError {
    /// Workers and queue capacity must be positive.
    #[error("background worker count and queue capacity must be positive")]
    InvalidConfiguration,
    /// No more queued work can currently be accepted.
    #[error("background work queue is full")]
    QueueFull,
    /// The pool or task has closed.
    #[error("background pool or task is closed")]
    Closed,
    /// A task result has already been consumed.
    #[error("background task result was already consumed")]
    AlreadyTaken,
    /// A worker could not be created.
    #[error("cannot spawn background worker: {0}")]
    Spawn(String),
    /// A background callback panicked.
    #[error("background callback panicked")]
    Panicked,
}
type Job = Box<dyn FnOnce() + Send>;
struct BackgroundState<T> {
    result: Option<std::result::Result<T, BackgroundError>>,
    waker: Option<Waker>,
    canceled: bool,
}
/// An owned future delivering a background result to the awaiting thread.
pub struct BackgroundTask<T> {
    state: Arc<Mutex<BackgroundState<T>>>,
    consumed: bool,
}
impl<T> Future for BackgroundTask<T> {
    type Output = std::result::Result<T, BackgroundError>;
    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        if this.consumed {
            return Poll::Ready(Err(BackgroundError::AlreadyTaken));
        }
        let mut state = lock(&this.state);
        if let Some(result) = state.result.take() {
            this.consumed = true;
            return Poll::Ready(result);
        }
        if state.canceled {
            this.consumed = true;
            return Poll::Ready(Err(BackgroundError::Closed));
        }
        if state
            .waker
            .as_ref()
            .is_none_or(|waker| !waker.will_wake(context.waker()))
        {
            state.waker = Some(context.waker().clone());
        }
        Poll::Pending
    }
}
impl<T> Drop for BackgroundTask<T> {
    fn drop(&mut self) {
        let (result, waker) = {
            let mut state = lock(&self.state);
            state.canceled = true;
            (state.result.take(), state.waker.take())
        };
        drop(result);
        drop(waker);
    }
}
/// A fixed-size background pool with a bounded, nonblocking submission queue.
///
/// Dropping the pool closes submissions; workers finish accepted work and exit.
/// `shutdown` additionally joins them and should run outside latency-sensitive
/// callbacks. Dropping a task suppresses its result and skips work not yet begun.
pub struct BackgroundPool {
    sender: Option<mpsc::SyncSender<Job>>,
    workers: Vec<JoinHandle<()>>,
}
impl BackgroundPool {
    /// Starts `workers` threads with at most `queue_capacity` waiting jobs.
    pub fn new(
        workers: usize,
        queue_capacity: usize,
    ) -> std::result::Result<Self, BackgroundError> {
        if workers == 0 || queue_capacity == 0 {
            return Err(BackgroundError::InvalidConfiguration);
        }
        let (sender, receiver) = mpsc::sync_channel::<Job>(queue_capacity);
        let receiver = Arc::new(Mutex::new(receiver));
        let mut handles = Vec::new();
        for index in 0..workers {
            let receiver = Arc::clone(&receiver);
            match thread::Builder::new()
                .name(format!("oxiview-worker-{index}"))
                .spawn(move || {
                    loop {
                        let next = lock(&receiver).recv();
                        match next {
                            Ok(job) => job(),
                            Err(_) => break,
                        }
                    }
                }) {
                Ok(handle) => handles.push(handle),
                Err(error) => {
                    drop(sender);
                    for handle in handles {
                        let _ = handle.join();
                    }
                    return Err(BackgroundError::Spawn(error.to_string()));
                }
            }
        }
        Ok(Self {
            sender: Some(sender),
            workers: handles,
        })
    }
    /// Enqueues owned Send work without capturing thread-local signal handles.
    pub fn spawn<T: Send + 'static>(
        &self,
        work: impl FnOnce() -> T + Send + 'static,
    ) -> std::result::Result<BackgroundTask<T>, BackgroundError> {
        let sender = self.sender.as_ref().ok_or(BackgroundError::Closed)?;
        let state = Arc::new(Mutex::new(BackgroundState {
            result: None,
            waker: None,
            canceled: false,
        }));
        let output = Arc::clone(&state);
        let job: Job = Box::new(move || {
            if lock(&output).canceled {
                return;
            }
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work))
                .map_err(|_| BackgroundError::Panicked);
            let mut result = Some(result);
            let waker = {
                let mut state = lock(&output);
                if state.canceled {
                    None
                } else {
                    state.result = result.take();
                    state.waker.take()
                }
            };
            drop(result);
            if let Some(waker) = waker {
                waker.wake();
            }
        });
        match sender.try_send(job) {
            Ok(()) => Ok(BackgroundTask {
                state,
                consumed: false,
            }),
            Err(mpsc::TrySendError::Full(_)) => Err(BackgroundError::QueueFull),
            Err(mpsc::TrySendError::Disconnected(_)) => Err(BackgroundError::Closed),
        }
    }
    /// Closes submissions and waits for accepted jobs and workers to finish.
    pub fn shutdown(mut self) -> std::result::Result<(), BackgroundError> {
        self.sender.take();
        let mut failed = false;
        for worker in self.workers.drain(..) {
            if worker.join().is_err() {
                failed = true;
            }
        }
        if failed {
            Err(BackgroundError::Panicked)
        } else {
            Ok(())
        }
    }
}
impl Drop for BackgroundPool {
    fn drop(&mut self) {
        self.sender.take();
    }
}
/// Submits work to an explicit bounded pool and returns its awaiting future.
pub fn spawn_background<T: Send + 'static>(
    pool: &BackgroundPool,
    work: impl FnOnce() -> T + Send + 'static,
) -> std::result::Result<BackgroundTask<T>, BackgroundError> {
    pool.spawn(work)
}

#[cfg(test)]
mod tests;
