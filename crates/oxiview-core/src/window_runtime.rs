use crate::Error;
use oxiview_platform::WindowWaker;
use oxiview_reactive::{LocalExecutor, Scope};

const LOCAL_TASK_BUDGET: usize = 256;

/// One window's lifetime root and wake-driven, thread-local task executor.
/// Field order disposes scoped futures before destroying their executor.
pub(crate) struct WindowRuntime {
    scope: Scope,
    executor: LocalExecutor,
}
impl WindowRuntime {
    pub(crate) fn new(waker: WindowWaker) -> Result<Self, Error> {
        let scope = Scope::new()?;
        let executor = LocalExecutor::new(move || match waker.wake() {
            Ok(()) | Err(oxiview_platform::Error::WindowDestroyed) => {}
            Err(error) => tracing::error!(%error, "local task could not wake its window"),
        });
        Ok(Self { scope, executor })
    }
    pub(crate) fn enter<T>(&self, callback: impl FnOnce() -> Result<T, Error>) -> Result<T, Error> {
        self.executor.enter(|| self.scope.run(|| Ok(callback())))?
    }
    pub(crate) fn poll_ready(&self) -> Result<(), Error> {
        let report = self.executor.poll_ready(LOCAL_TASK_BUDGET)?;
        if report.polled != 0 {
            tracing::trace!(
                polled = report.polled,
                completed = report.completed,
                canceled = report.canceled,
                pending = report.pending,
                more_ready = report.more_ready,
                "window local tasks processed"
            );
        }
        let mut errors = report.errors.into_iter();
        if let Some(error) = errors.next() {
            for remaining in errors {
                tracing::error!(%remaining, "additional window local task failure");
            }
            return Err(Error::Reactive(error));
        }
        // A nonempty ready remainder already schedules a coalesced WindowWake.
        // Sleeping futures create neither polling deadlines nor frame requests.
        Ok(())
    }
    pub(crate) fn finish(&self, result: Result<(), Error>) -> Result<(), Error> {
        let cleanup = self.scope.dispose().map_err(Error::Reactive);
        match (result, cleanup) {
            (Err(error), Err(cleanup)) => {
                tracing::error!(%cleanup,"window runtime cleanup also failed");
                Err(error)
            }
            (Err(error), Ok(())) => Err(error),
            (Ok(()), result) => result,
        }
    }
}
