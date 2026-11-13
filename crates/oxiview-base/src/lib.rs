//! Owned values and deterministic services for the shared-foundation context (role foundation).
//!
//! System adapters exist only in unit-test builds. No Cargo feature enables them.

mod color;
mod geometry;
mod services;
mod surface;
#[cfg(test)]
mod system;
#[cfg(test)]
mod tests;

pub use color::Color;
pub use geometry::{Affine, CornerRadii, Point, Rect, Size};
pub use services::{Clock, ManualClock, ManualRng, ManualScheduler, Rng, Scheduler};
pub use surface::{NativeSurfaceHandle, NativeSurfaceKind};

/// Recoverable foundation failures, independent of implementation libraries.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// An affine matrix has no finite inverse.
    #[error("affine transform has no finite inverse")]
    NonInvertibleTransform,
    /// The caller must supply more replay values.
    #[error("manual random value stream is exhausted")]
    RandomValuesExhausted,
}

/// Restores the previous thread-local tracing subscriber when dropped.
///
/// Drop nested guards in reverse installation order. This guard does not install
/// a process-global subscriber or propagate a subscriber to other threads.
#[must_use = "keep the guard alive while tracing is needed"]
pub struct TracingGuard {
    _guard: tracing::subscriber::DefaultGuard,
}

/// Installs the supplied subscriber for this thread until the guard is dropped.
///
/// Repeated calls nest without panicking, even when a global subscriber exists.
pub fn init_tracing(subscriber: impl tracing::Subscriber + Send + Sync + 'static) -> TracingGuard {
    TracingGuard {
        _guard: tracing::subscriber::set_default(subscriber),
    }
}
