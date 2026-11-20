//! GPU rendering adapter with offscreen readback and per-window surfaces.
//!
//! The instance, adapter, and device are process-wide and shared by every context.
//! Device loss discards the old device and rebuilds it for every context on next
//! use. Offscreen rendering needs no window. The SDF fast path uses analytic
//! coverage for UI geometry under similarities (translation, uniform scale,
//! rotation and reflection). Vello renders general affine geometry, images and
//! vector glyphs. The device retains an atlas of uploaded image pixels across
//! frames. Both paths composite in linear light and expose straight sRGB RGBA8
//! offscreen. Native presentation preserves the surface's premultiplied alpha.

// Native surface creation is the adapter's only unsafe boundary.
#![allow(unsafe_code)]

mod acquisition;
mod atlas;
mod copy;
mod device;
mod glyph;
mod surface;
mod vector;
pub use surface::{FrameOutcome, WindowSurface};
mod encoding;

use oxiview_scene::{NodeId, Scene};
use std::sync::{Arc, Mutex, OnceLock};

pub use oxiview_scene::{PixelSize, RgbaImage};

/// Failures at the scene, resource, or device boundary. No GPU types escape.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The handle kind is unsupported on this platform.
    #[error("unsupported native surface kind")]
    UnsupportedSurface,
    /// Surface creation, compatibility, configuration, or acquisition failed.
    #[error("surface operation failed: {0}")]
    Surface(String),
    /// No suitable native adapter is available (a supported configuration).
    #[error("no GPU adapter available: {0}")]
    NoAdapter(String),
    /// An adapter could not create the required device.
    #[error("GPU device request failed: {0}")]
    DeviceRequest(String),
    /// The root cannot be encoded, or geometry exceeds GPU representation limits.
    #[error("scene encoding failed: {0}")]
    SceneEncoding(String),
    /// Zero dimensions or texture/readback/compositing storage exceeds device limits.
    #[error("unsupported GPU render size: {0:?}")]
    InvalidSize(PixelSize),
    /// Device loss or a failed GPU operation; the next use rebuilds the device.
    #[error("GPU device lost: {0}")]
    DeviceLost(String),
}

static PROCESS: OnceLock<Arc<Mutex<device::Process>>> = OnceLock::new();

/// A cheap-to-clone handle to the process-wide instance, adapter, and device.
///
/// Both clones and independently constructed contexts share one device. Rendering
/// is serialized. Device loss drops the old device and rebuilds it for **every**
/// context on next use; no context retains a stale device. Construction requests
/// an adapter and device on first use. All operations are synchronous.
#[derive(Clone)]
pub struct RenderContext {
    process: Arc<Mutex<device::Process>>,
}
impl RenderContext {
    /// Acquires the shared device, returning `NoAdapter` on machines without one.
    pub fn new() -> Result<Self, Error> {
        let process =
            Arc::clone(PROCESS.get_or_init(|| Arc::new(Mutex::new(device::Process::new()))));
        process
            .lock()
            .map_err(|e| Error::DeviceLost(e.to_string()))?
            .ensure_device()?;
        Ok(Self { process })
    }

    /// Creates a per-window surface on the window thread, initially suspended.
    /// The producing adapter's handle contract supplies native lifetime guarantees.
    pub fn create_surface(
        &self,
        handle: oxiview_base::NativeSurfaceHandle,
    ) -> Result<WindowSurface, Error> {
        WindowSurface::new(self.clone(), handle)
    }

    /// Encodes and renders `root` on transparent pixels, then reads straight RGBA8.
    /// A loss during rendering is reported; the next call rebuilds shared state.
    pub fn render(&self, scene: &Scene, root: NodeId, size: PixelSize) -> Result<RgbaImage, Error> {
        if size.width == 0 || size.height == 0 {
            return Err(Error::InvalidSize(size));
        }
        let encoded = scene
            .encode(root)
            .map_err(|e| Error::SceneEncoding(e.to_string()))?;
        let mut process = self
            .process
            .lock()
            .map_err(|e| Error::DeviceLost(e.to_string()))?;
        process.ensure_device()?;
        let gpu = process
            .gpu
            .as_mut()
            .ok_or_else(|| Error::DeviceLost("missing device".into()))?;
        let result = gpu
            .render_scene(&encoded, size, None, true)
            .and_then(|image| image.ok_or_else(|| Error::DeviceLost("missing readback".into())));
        if matches!(result, Err(Error::DeviceLost(_))) {
            process.lose_device();
        }
        result
    }

    /// Destroys the shared device to exercise the real loss/recreation path.
    ///
    /// This affects every context in this process. Their next operation acquires
    /// the replacement device; callers should coordinate this diagnostic action.
    pub fn simulate_device_loss(&self) -> Result<(), Error> {
        self.process
            .lock()
            .map_err(|e| Error::DeviceLost(e.to_string()))?
            .lose_device();
        Ok(())
    }
}

fn straight(channel: u8, alpha: u8) -> u8 {
    if alpha == 0 {
        return 0;
    }
    ((u32::from(channel) * 255 + u32::from(alpha) / 2) / u32::from(alpha)).min(255) as u8
}

#[cfg(test)]
mod tests;
