use crate::Error;
use std::{collections::VecDeque, time::Duration};

/// An injected clock measured from an implementation-defined origin.
pub trait Clock {
    /// Returns elapsed time at the current replay or system position.
    fn now(&self) -> Duration;
}
/// A clock whose time changes only when the caller supplies a value.
#[derive(Clone, Copy, Debug, Default)]
pub struct ManualClock {
    now: Duration,
}
impl ManualClock {
    /// Creates a clock at the supplied time.
    pub const fn new(now: Duration) -> Self {
        Self { now }
    }
    /// Sets replay time, including backwards seeks.
    pub fn set(&mut self, now: Duration) {
        self.now = now;
    }
}
impl Clock for ManualClock {
    fn now(&self) -> Duration {
        self.now
    }
}

/// An injected executor for queued, one-shot work.
pub trait Scheduler {
    /// Submits a task. Execution policy belongs to the implementation.
    fn schedule(&mut self, task: Box<dyn FnOnce() + Send + 'static>);
}
/// FIFO executor that runs work only when explicitly driven by the caller.
#[derive(Default)]
pub struct ManualScheduler {
    tasks: VecDeque<Box<dyn FnOnce() + Send + 'static>>,
}
impl ManualScheduler {
    /// Creates an empty queue.
    pub fn new() -> Self {
        Self::default()
    }
    /// Returns the number of pending tasks.
    pub fn pending(&self) -> usize {
        self.tasks.len()
    }
    /// Executes at most one task, returning whether a task ran.
    pub fn run_next(&mut self) -> bool {
        if let Some(task) = self.tasks.pop_front() {
            task();
            true
        } else {
            false
        }
    }
}
impl Scheduler for ManualScheduler {
    fn schedule(&mut self, task: Box<dyn FnOnce() + Send + 'static>) {
        self.tasks.push_back(task);
    }
}

/// An injected source of random values; not a cryptographic interface.
pub trait Rng {
    /// Returns the next value or an explicit replay exhaustion error.
    fn next_u64(&mut self) -> Result<u64, Error>;
}
/// A finite, caller-supplied random stream with no implicit seed or fallback.
#[derive(Clone, Debug, Default)]
pub struct ManualRng {
    values: VecDeque<u64>,
}
impl ManualRng {
    /// Creates a replay stream in iteration order.
    pub fn new(values: impl IntoIterator<Item = u64>) -> Self {
        Self {
            values: values.into_iter().collect(),
        }
    }
    /// Appends a value to the replay stream.
    pub fn push(&mut self, value: u64) {
        self.values.push_back(value);
    }
}
impl Rng for ManualRng {
    fn next_u64(&mut self) -> Result<u64, Error> {
        self.values.pop_front().ok_or(Error::RandomValuesExhausted)
    }
}
