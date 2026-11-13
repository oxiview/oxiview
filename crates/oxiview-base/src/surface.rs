use std::num::NonZeroUsize;

/// The native presentation object described by a [`NativeSurfaceHandle`].
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NativeSurfaceKind {
    /// A macOS Core Animation Metal layer.
    MacosMetalLayer,
}

/// An owned, platform-neutral description of a native presentation object.
///
/// Unstable: a second platform implementation has not validated this boundary.
/// Copying this value does not retain or release the native object. The producing
/// adapter guarantees its lifetime; rendering adapters own all unsafe use.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NativeSurfaceHandle {
    kind: NativeSurfaceKind,
    address: NonZeroUsize,
}

impl NativeSurfaceHandle {
    /// Describes a macOS `CAMetalLayer`. Call only on the main thread.
    ///
    /// # Safety
    ///
    /// `address` must be the exposed address of a live, initialized `CAMetalLayer`.
    /// The producer must permit surface creation and presentation from a window
    /// thread, including concurrent use of copied handles, and preserve pointer
    /// provenance for reconstruction by the consuming rendering adapter.
    /// The layer must remain alive and attached to its window until destruction
    /// has been delivered to the window thread AND that thread has released every
    /// surface created from this handle. Destruction delivery must not invalidate
    /// outstanding surfaces. Native mutations must be synchronized with their use.
    ///
    /// Because this handle is Copy and has no lifetime, a copy may be consumed
    /// even after window destruction. The producer must keep the layer alive and
    /// valid for surface creation and off-main-thread presentation for the rest
    /// of the process (attachment may end after the condition above). This
    /// stronger retention guarantee lets a renderer expose safe surface creation
    /// without imposing any additional lifetime obligation on its caller.
    // ADR-0025 permits exactly this unsafe declaration, not unsafe operations.
    #[allow(unsafe_code)]
    pub unsafe fn from_macos_metal_layer(address: NonZeroUsize) -> Self {
        Self {
            kind: NativeSurfaceKind::MacosMetalLayer,
            address,
        }
    }

    /// Returns the object kind. May be called on any thread.
    pub const fn kind(self) -> NativeSurfaceKind {
        self.kind
    }

    /// Returns the exposed address. May be called on any thread.
    /// Dereferencing it remains the rendering adapter's unsafe responsibility.
    pub const fn address(self) -> NonZeroUsize {
        self.address
    }
}
