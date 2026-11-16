use crate::{Error, PixelSize, RenderContext, device, encoding};
use oxiview_base::{NativeSurfaceHandle, NativeSurfaceKind};
use oxiview_scene::{NodeId, Scene};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

/// Result of a synchronous window-thread presentation attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameOutcome {
    /// GPU work was submitted and presentation requested.
    Presented,
    /// Either physical dimension is zero.
    Suspended,
    /// Acquisition timed out or the window is occluded; request a later frame.
    Skipped,
}

/// A per-window surface sharing the process-wide instance, adapter, and device.
///
/// Create, configure, render and drop on the window thread. The platform adapter
/// keeps presentsWithTransaction off in steady state and enables it only during
/// the main-thread resize transaction, resetting it before commit. This adapter
/// never changes it. wgpu-hal 30.0.2's metal/surface.rs acquire_texture snapshots
/// the property after nextDrawable; main-thread Resized/display-link Frame event
/// publication orders the flag update before window-thread acquisition. A resize
/// timeout can leave a transaction-enabled drawable presenting late; the platform
/// traces this exception.
pub struct WindowSurface {
    context: RenderContext,
    handle: NativeSurfaceHandle,
    surface: wgpu::Surface<'static>,
    size: PixelSize,
    display_sync: bool,
    generation: Option<Arc<AtomicBool>>,
}

pub(crate) fn validate_kind(kind: NativeSurfaceKind, metal: bool) -> Result<(), Error> {
    match kind {
        NativeSurfaceKind::MacosMetalLayer if metal => Ok(()),
        _ => Err(Error::UnsupportedSurface),
    }
}

fn create(
    instance: &wgpu::Instance,
    handle: NativeSurfaceHandle,
) -> Result<wgpu::Surface<'static>, Error> {
    validate_kind(handle.kind(), cfg!(target_os = "macos"))?;
    #[cfg(target_os = "macos")]
    {
        let pointer = std::ptr::with_exposed_provenance_mut(handle.address().get());
        // SAFETY: NativeSurfaceHandle::from_macos_metal_layer's contract
        // (ADR-0025) guarantees a live initialized CAMetalLayer, exposed pointer
        // provenance, process-lifetime validity even for stale copies, attachment
        // through destruction and surface release, synchronized native mutations,
        // and off-main-thread creation/presentation including copied handles.
        // We reconstruct only the validated Metal kind and retain the surface
        // on its window thread; no extra lifetime obligations reach safe callers.
        unsafe {
            instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::CoreAnimationLayer(pointer))
        }
        .map_err(|e| Error::Surface(e.to_string()))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = instance;
        Err(Error::UnsupportedSurface)
    }
}

/// Returns false for suspension; no zero-sized configuration reaches wgpu.
pub(crate) fn validate_size(size: PixelSize, limit: u32) -> Result<bool, Error> {
    if size.width == 0 || size.height == 0 {
        return Ok(false);
    }
    if size.width > limit || size.height > limit {
        return Err(Error::InvalidSize(size));
    }
    Ok(true)
}

impl WindowSurface {
    pub(crate) fn new(context: RenderContext, handle: NativeSurfaceHandle) -> Result<Self, Error> {
        let surface = {
            let process = context
                .process
                .lock()
                .map_err(|e| Error::DeviceLost(e.to_string()))?;
            create(&process.instance, handle)?
        };
        Ok(Self {
            context,
            handle,
            surface,
            size: PixelSize {
                width: 0,
                height: 0,
            },
            display_sync: false,
            generation: None,
        })
    }

    /// Sets physical dimensions on the window thread; either zero suspends.
    /// Invalid dimensions leave the previous size intact. GPU loss is recoverable
    /// through the shared context; the next use configures the replacement device.
    pub fn configure(&mut self, size: PixelSize, display_sync: bool) -> Result<(), Error> {
        let mut process = self
            .context
            .process
            .lock()
            .map_err(|e| Error::DeviceLost(e.to_string()))?;
        process.ensure_device()?;
        let gpu = process
            .gpu
            .as_ref()
            .ok_or_else(|| Error::DeviceLost("device unavailable".into()))?;
        if !validate_size(size, gpu.device.limits().max_texture_dimension_2d)? {
            self.size = size;
            self.generation = None;
            return Ok(());
        }
        configure(&self.surface, gpu, size, display_sync)?;
        self.display_sync = display_sync;
        self.size = size;
        self.generation = Some(Arc::clone(&gpu.lost));
        Ok(())
    }

    /// Renders and presents on the window thread, or reports suspension/skipping.
    /// Outdated/lost surfaces are reconfigured and acquisition retried once.
    /// Loss also recreates the native surface, as required by wgpu 30.
    pub fn render(&mut self, scene: &Scene, root: NodeId) -> Result<FrameOutcome, Error> {
        if self.size.width == 0 || self.size.height == 0 {
            return Ok(FrameOutcome::Suspended);
        }
        let encoded = scene
            .encode(root)
            .map_err(|e| Error::SceneEncoding(e.to_string()))?;
        let commands = encoding::Commands::new(&encoded, self.size)?;
        let mut process = self
            .context
            .process
            .lock()
            .map_err(|e| Error::DeviceLost(e.to_string()))?;
        process.ensure_device()?;
        let gpu = process
            .gpu
            .as_ref()
            .ok_or_else(|| Error::DeviceLost("device unavailable".into()))?;
        if !self
            .generation
            .as_ref()
            .is_some_and(|generation| Arc::ptr_eq(generation, &gpu.lost))
        {
            configure(&self.surface, gpu, self.size, self.display_sync)?;
            self.generation = Some(Arc::clone(&gpu.lost));
        }
        let mut frame = None;
        for attempt in 0..2 {
            match scoped(gpu, || self.surface.get_current_texture())? {
                wgpu::CurrentSurfaceTexture::Success(texture) => {
                    frame = Some(texture);
                    break;
                }
                wgpu::CurrentSurfaceTexture::Suboptimal(texture) => {
                    self.generation = None;
                    frame = Some(texture);
                    break;
                }
                wgpu::CurrentSurfaceTexture::Timeout => {
                    tracing::warn!("surface acquisition timed out; skipping frame");
                    return Ok(FrameOutcome::Skipped);
                }
                wgpu::CurrentSurfaceTexture::Occluded => return Ok(FrameOutcome::Skipped),
                wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost
                    if attempt == 0 =>
                {
                    // Recreating also handles Lost; no drawable is held here.
                    self.surface = create(&process.instance, self.handle)?;
                    configure(&self.surface, gpu, self.size, self.display_sync)?;
                }
                other => {
                    if gpu.lost.load(Ordering::Acquire) {
                        return Err(Error::DeviceLost(
                            "surface acquisition after device loss".into(),
                        ));
                    }
                    return Err(Error::Surface(format!(
                        "acquisition failed after bounded retry: {other:?}"
                    )));
                }
            }
        }
        let frame = frame.ok_or_else(|| Error::Surface("no drawable after retry".into()))?;
        match gpu.render(&commands, self.size, Some(&frame.texture), false) {
            Ok(_) => {
                scoped(gpu, || gpu.queue.present(frame))?;
                Ok(FrameOutcome::Presented)
            }
            Err(error) => {
                drop(frame);
                if matches!(error, Error::DeviceLost(_)) {
                    process.lose_device();
                }
                Err(error)
            }
        }
    }
}

fn configure(
    surface: &wgpu::Surface<'_>,
    gpu: &device::Gpu,
    size: PixelSize,
    display_sync: bool,
) -> Result<(), Error> {
    if !validate_size(size, gpu.device.limits().max_texture_dimension_2d)? {
        return Ok(());
    }
    let capabilities = surface.get_capabilities(&gpu.adapter);
    let mut config = surface
        .get_default_config(&gpu.adapter, size.width, size.height)
        .ok_or_else(|| Error::Surface("surface incompatible with shared adapter".into()))?;
    config.format = [
        wgpu::TextureFormat::Bgra8UnormSrgb,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        wgpu::TextureFormat::Bgra8Unorm,
        wgpu::TextureFormat::Rgba8Unorm,
    ]
    .into_iter()
    .find(|format| capabilities.formats.contains(format))
    .ok_or_else(|| Error::Surface("no supported RGBA8 surface format".into()))?;
    if !capabilities
        .present_modes
        .contains(&wgpu::PresentMode::Fifo)
    {
        return Err(Error::Surface("FIFO presentation unavailable".into()));
    }
    config.present_mode = if !display_sync
        && capabilities
            .present_modes
            .contains(&wgpu::PresentMode::Immediate)
    {
        wgpu::PresentMode::Immediate
    } else {
        wgpu::PresentMode::Fifo
    };
    // Metal maps latency N to N + 1 drawables. Keep three in both modes:
    // reducing the pool after fullscreen can make even Immediate acquisition
    // block for an interval. The macOS scheduler bounds outstanding FIFO frames
    // before painting, allowing input dispatch to continue while the pool is full.
    config.desired_maximum_frame_latency = 2;
    config.usage = wgpu::TextureUsages::RENDER_ATTACHMENT;
    scoped(gpu, || surface.configure(&gpu.device, &config))
}

// Catch validation/resource errors at every native GPU operation. Resource
// failure invalidates shared state synchronously; all surfaces notice the new
// device identity on next use, even before wgpu delivers its loss callback.
fn scoped<T>(gpu: &device::Gpu, operation: impl FnOnce() -> T) -> Result<T, Error> {
    let validation = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
    let memory = gpu.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    let internal = gpu.device.push_error_scope(wgpu::ErrorFilter::Internal);
    let result = operation();
    let errors = [
        device::block_on(internal.pop()),
        device::block_on(memory.pop()),
        device::block_on(validation.pop()),
    ];
    if let Some(error) = errors.into_iter().flatten().next() {
        gpu.lost.store(true, Ordering::Release);
        return Err(Error::DeviceLost(error.to_string()));
    }
    if gpu.lost.load(Ordering::Acquire) {
        return Err(Error::DeviceLost(
            "device lost during surface operation".into(),
        ));
    }
    Ok(result)
}
