//! Offscreen GPU adapter for the rendering context.
//!
//! The instance, adapter, and device are process-wide and shared by every context.
//! Device loss discards the old device and rebuilds it for every context on next
//! use. No window is needed. Geometry is flattened to polygons and rasterized on
//! the GPU with four coverage samples per pixel; this is not an SDF fast path.
//! Output is straight sRGB RGBA8, without a byte-for-byte CPU equivalence promise.

mod device;
mod encoding;

use oxiview_scene::{NodeId, Scene};
use std::sync::{Arc, Mutex, OnceLock};

/// Integer output dimensions in physical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelSize {
    /// Nonzero width, limited by the device's texture and buffer limits.
    pub width: u32,
    /// Nonzero height, limited by the device's texture and buffer limits.
    pub height: u32,
}

/// Owned row-major, straight (non-premultiplied) sRGB RGBA8 pixels.
///
/// This has the CPU renderer's image shape. A shared image type is deferred
/// because rendering adapters cannot depend on each other.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RgbaImage {
    size: PixelSize,
    pixels: Vec<u8>,
}
impl RgbaImage {
    /// Output width in pixels.
    pub fn width(&self) -> u32 {
        self.size.width
    }
    /// Output height in pixels.
    pub fn height(&self) -> u32 {
        self.size.height
    }
    /// Straight RGBA8 bytes; transparent pixels have zero RGB.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }
}

/// Failures at the scene, resource, or device boundary. No GPU types escape.
#[derive(Debug, thiserror::Error)]
pub enum Error {
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

    /// Encodes and renders `root` on transparent pixels, then reads straight RGBA8.
    /// A loss during rendering is reported; the next call rebuilds shared state.
    pub fn render(&self, scene: &Scene, root: NodeId, size: PixelSize) -> Result<RgbaImage, Error> {
        if size.width == 0 || size.height == 0 {
            return Err(Error::InvalidSize(size));
        }
        let encoded = scene
            .encode(root)
            .map_err(|e| Error::SceneEncoding(e.to_string()))?;
        let commands = encoding::Commands::new(&encoded, size)?;
        self.process
            .lock()
            .map_err(|e| Error::DeviceLost(e.to_string()))?
            .render(&commands, size)
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
