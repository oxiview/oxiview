use super::*;
use crate::{Scope, runtime_stats, signal};
use std::{cell::Cell, sync::atomic::AtomicUsize, time::Duration};

struct PendingProbe {
    drops: Rc<Cell<usize>>,
    polls: Rc<Cell<usize>>,
    waker: Rc<RefCell<Option<Waker>>>,
}
impl Future for PendingProbe {
    type Output = Result<()>;
    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        self.polls.set(self.polls.get() + 1);
        *self.waker.borrow_mut() = Some(context.waker().clone());
        Poll::Pending
    }
}
impl Drop for PendingProbe {
    fn drop(&mut self) {
        self.drops.set(self.drops.get() + 1);
    }
}

#[test]
fn scope_disposal_cancels_sleeping_tasks_and_stale_wakes_cannot_revive_them() -> Result<()> {
    let executor = LocalExecutor::new(|| {});
    let scope = Scope::new()?;
    let drops = Rc::new(Cell::new(0));
    let polls = Rc::new(Cell::new(0));
    let waker = Rc::new(RefCell::new(None));
    let task = scope.run(|| {
        executor.spawn_local(PendingProbe {
            drops: Rc::clone(&drops),
            polls: Rc::clone(&polls),
            waker: Rc::clone(&waker),
        })
    })?;
    assert_eq!(executor.poll_ready(8)?.polled, 1);
    assert_eq!(executor.poll_ready(8)?.polled, 0);
    assert_eq!(polls.get(), 1);
    scope.dispose()?;
    assert_eq!(drops.get(), 1);
    assert!(task.is_finished()?);
    assert_eq!(runtime_stats()?.resources, 0);
    let replacement_scope = Scope::new()?;
    let completed = Rc::new(Cell::new(0));
    let count = Rc::clone(&completed);
    replacement_scope.run(|| {
        executor.spawn_local(async move {
            count.set(count.get() + 1);
            Ok(())
        })
    })?;
    if let Some(waker) = waker.borrow().as_ref() {
        waker.wake_by_ref();
    }
    let report = executor.poll_ready(8)?;
    assert_eq!(report.completed, 1);
    assert_eq!(completed.get(), 1);
    assert_eq!(polls.get(), 1);
    assert_eq!(drops.get(), 1);
    assert_eq!(runtime_stats()?.resources, 0);
    Ok(())
}

#[test]
fn background_result_wakes_the_originating_local_task_and_updates_its_signal()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let pool = BackgroundPool::new(2, 4)?;
    let (sender, receiver) = mpsc::channel();
    let wakes = Arc::new(AtomicUsize::new(0));
    let wake_count = Arc::clone(&wakes);
    let executor = LocalExecutor::new(move || {
        wake_count.fetch_add(1, Ordering::Relaxed);
        let _ = sender.send(());
    });
    let scope = Scope::new()?;
    let value = scope.run(|| signal(0))?;
    let work = pool.spawn(|| 42)?;
    let task = scope.run(|| {
        executor.enter(|| {
            spawn_local(async move {
                let result = work
                    .await
                    .map_err(|error| ReactiveError::User(error.to_string()))?;
                value.set(result)
            })
        })
    })?;
    for _ in 0..8 {
        let report = executor.poll_ready(8)?;
        assert!(report.errors.is_empty());
        if task.is_finished()? {
            break;
        }
        receiver.recv_timeout(Duration::from_secs(2))?;
    }
    assert!(task.is_finished()?);
    assert_eq!(value.get()?, 42);
    assert_eq!(runtime_stats()?.resources, 0);
    let count = wakes.load(Ordering::Relaxed);
    for _ in 0..8 {
        assert_eq!(executor.poll_ready(8)?.polled, 0);
    }
    assert_eq!(wakes.load(Ordering::Relaxed), count);
    pool.shutdown()?;
    Ok(())
}

#[test]
fn bounded_pool_rejects_excess_work_and_skips_canceled_waiting_jobs()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let pool = BackgroundPool::new(1, 1)?;
    let (started, waiting) = mpsc::channel();
    let (release, blocked) = mpsc::channel();
    let first = pool.spawn(move || {
        let _ = started.send(());
        let _ = blocked.recv();
        1
    })?;
    waiting.recv_timeout(Duration::from_secs(2))?;
    let calls = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&calls);
    let queued = pool.spawn(move || {
        count.fetch_add(1, Ordering::Relaxed);
        2
    })?;
    assert!(matches!(pool.spawn(|| 3), Err(BackgroundError::QueueFull)));
    drop(queued);
    release.send(())?;
    pool.shutdown()?;
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    drop(first);
    Ok(())
}

#[test]
fn local_task_errors_are_reported_without_losing_other_tasks() -> Result<()> {
    let executor = LocalExecutor::new(|| {});
    let scope = Scope::new()?;
    let done = Rc::new(Cell::new(false));
    let marker = Rc::clone(&done);
    scope.run(|| {
        executor.spawn_local(async { Err(ReactiveError::User("task failure".into())) })?;
        executor.spawn_local(async move {
            marker.set(true);
            Ok(())
        })?;
        Ok(())
    })?;
    let first = executor.poll_ready(1)?;
    assert_eq!(first.errors.len(), 1);
    assert!(first.more_ready);
    let second = executor.poll_ready(1)?;
    assert_eq!(second.completed, 1);
    assert!(done.get());
    assert_eq!(second.pending, 0);
    assert_eq!(runtime_stats()?.resources, 0);
    Ok(())
}

#[test]
fn recursively_polling_the_executor_is_a_checked_error() -> Result<()> {
    let executor = Rc::new(LocalExecutor::new(|| {}));
    let scope = Scope::new()?;
    let nested = Rc::clone(&executor);
    scope.run(|| {
        executor.spawn_local(async move {
            assert!(matches!(
                nested.poll_ready(1),
                Err(ReactiveError::ReentrantPoll)
            ));
            Ok(())
        })
    })?;
    assert_eq!(executor.poll_ready(1)?.completed, 1);
    Ok(())
}

#[test]
#[allow(clippy::panic)]
fn a_panicked_background_callback_returns_an_error_and_worker_survives()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let pool = BackgroundPool::new(1, 2)?;
    let task = pool.spawn(|| panic!("intentional background boundary test"))?;
    let continued = pool.spawn(|| 7)?;
    let executor = LocalExecutor::new(|| {});
    let scope = Scope::new()?;
    let saw_error = Rc::new(Cell::new(false));
    let observed = Rc::clone(&saw_error);
    let handle = scope.run(|| {
        executor.spawn_local(async move {
            observed.set(
                matches!(task.await, Err(BackgroundError::Panicked))
                    && matches!(continued.await, Ok(7)),
            );
            Ok(())
        })
    })?;
    pool.shutdown()?;
    executor.poll_ready(8)?;
    assert!(handle.is_finished()?);
    assert!(saw_error.get());
    Ok(())
}

#[test]
fn self_disposal_drops_a_polling_future_outside_executor_borrows() -> Result<()> {
    struct CancelOther {
        task: LocalTask,
        succeeded: Rc<Cell<bool>>,
    }
    impl Drop for CancelOther {
        fn drop(&mut self) {
            self.succeeded.set(matches!(self.task.cancel(), Ok(true)));
        }
    }
    let executor = LocalExecutor::new(|| {});
    let other_scope = Scope::new()?;
    let other = other_scope.run(|| {
        executor.spawn_local(async {
            std::future::pending::<()>().await;
            Ok(())
        })
    })?;
    let scope = Rc::new(Scope::new()?);
    let disposing = Rc::clone(&scope);
    let succeeded = Rc::new(Cell::new(false));
    let cleanup = CancelOther {
        task: other.clone(),
        succeeded: Rc::clone(&succeeded),
    };
    let current = scope.run(|| {
        executor.spawn_local(async move {
            disposing.dispose()?;
            std::future::pending::<()>().await;
            drop(cleanup);
            Ok(())
        })
    })?;
    executor.poll_ready(8)?;
    assert!(current.is_finished()?);
    assert!(other.is_finished()?);
    assert!(succeeded.get());
    assert_eq!(runtime_stats()?.resources, 0);
    Ok(())
}
