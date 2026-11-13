//! Explicit unit-test-only system adapters. Never compiled into the production library.
use crate::{Clock, Error, Rng, Scheduler};
use std::{
    collections::hash_map::RandomState,
    hash::BuildHasher,
    time::{Duration, Instant},
};

pub(crate) struct SystemClock(Instant);
impl SystemClock {
    pub(crate) fn new() -> Self {
        Self(Instant::now())
    }
}
impl Clock for SystemClock {
    fn now(&self) -> Duration {
        self.0.elapsed()
    }
}

pub(crate) struct SystemScheduler;
impl Scheduler for SystemScheduler {
    fn schedule(&mut self, task: Box<dyn FnOnce() + Send + 'static>) {
        // This adapter exists only in unit tests; OS thread-creation failure must fail the test.
        #[allow(clippy::expect_used)]
        let handle = std::thread::Builder::new()
            .spawn(task)
            .expect("could not create a system scheduler thread for the test");
        // The task owns its data; dropping the handle detaches execution.
        drop(handle);
    }
}

pub(crate) struct SystemRng;
impl Rng for SystemRng {
    fn next_u64(&mut self) -> Result<u64, Error> {
        // RandomState obtains OS-seeded keys through std. This is deliberately
        // non-cryptographic; no additional entropy dependency or unsafe FFI.
        Ok(RandomState::new().hash_one(0_u64))
    }
}
