//! Unstable platform port for main-thread events and window-thread presentation.
//!
//! Native macOS and deterministic headless backends implement this port. The backend owns
//! the event channel; consumers receive ordered per-window events. No rendering,
//! scene, or native library types cross this boundary.
//! A window thread can wait for events, a deadline on the backend's monotonic
//! clock, or a coalesced cross-thread `WindowWaker` notification. Headless deadline
//! waits respond to virtual-clock advancement rather than wall-clock sleeping.

mod interface;

pub use interface::*;

/// Platform failures in owned, platform-neutral terms.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A requested frame policy is unavailable; the active policy is unchanged.
    #[error("unsupported frame policy: {0:?}")]
    UnsupportedFramePolicy(FramePolicy),
    /// Window title, logical size, or resize timeout is invalid.
    #[error("invalid window attributes: {0}")]
    InvalidWindowAttributes(String),
    /// The window has already been destroyed.
    #[error("window has been destroyed")]
    WindowDestroyed,
    /// The event channel disconnected unexpectedly, before destruction delivery.
    #[error("window event channel disconnected")]
    EventChannelClosed,
    /// A resize token is stale, unknown, or belongs to another window.
    #[error("invalid resize token")]
    InvalidResizeToken,
    /// The platform could not complete an operation.
    #[error("platform operation failed: {0}")]
    OperationFailed(String),
    /// AppKit presentation must run on the process main thread.
    #[error("window presentation requires the main thread")]
    MainThreadRequired,
}
