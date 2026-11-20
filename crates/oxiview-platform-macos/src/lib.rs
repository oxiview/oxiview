#![allow(unsafe_code)]
//! Native AppKit platform, requiring macOS 14 or later.
//!
//! AppKit and layer ownership remain on the process main thread. Endpoints carry
//! only synchronized queues and owned handles to the window thread. Native calls
//! and Objective-C callback boundaries are confined to this adapter.

#[cfg(target_os = "macos")]
mod macos;
#[cfg(all(target_os = "macos", feature = "diagnostics"))]
pub use macos::diagnostic_m5_input_probe;
#[cfg(target_os = "macos")]
pub use macos::{MacOsControl, MacOsPlatform};
