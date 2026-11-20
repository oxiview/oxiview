use crate::{Error, PixelSize, RenderContext, acquisition::Acquisition, device};
use oxiview_base::{NativeSurfaceHandle, NativeSurfaceKind};
use oxiview_scene::{NodeId, Scene};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::task::Poll;

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
/// Create, configure, paint and present on the window thread. Ordinary
/// frames may acquire one texture on a worker after their display opportunity.
/// Resize retires an older acquisition before wgpu reconfiguration and paints
/// synchronously inside the native resize transaction. The platform manages
/// presentsWithTransaction and its bounded resize acknowledgement wait.
pub struct WindowSurface {
    acquisition: Option<Acquisition<wgpu::CurrentSurfaceTexture>>,
    prepared: Option<wgpu::CurrentSurfaceTexture>,
    context: RenderContext,
    handle: NativeSurfaceHandle,
    surface: Arc<wgpu::Surface<'static>>,
    size: PixelSize,
    display_sync: bool,
    configuration: Option<Configuration>,
}

struct Configuration {
    generation: Arc<AtomicBool>,
    device: wgpu::Device,
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
            acquisition: None,
            prepared: None,
            context,
            handle,
            surface: Arc::new(surface),
            size: PixelSize {
                width: 0,
                height: 0,
            },
            display_sync: false,
            configuration: None,
        })
    }

    /// Sets physical dimensions on the window thread; either zero suspends.
    /// Invalid dimensions leave the previous size intact. GPU loss is recoverable
    /// through the shared context; the next use configures the replacement device.
    pub fn configure(&mut self, size: PixelSize, display_sync: bool) -> Result<(), Error> {
        self.prepared.take();
        if size.width == 0 || size.height == 0 {
            if let Some(worker) = &mut self.acquisition {
                worker.stop();
            }
            self.size = size;
            self.configuration = None;
            return Ok(());
        }
        // wgpu requires all old surface textures to be released before
        // reconfiguration. Join outside the shared device lock. Metal's
        // acquisition timeout also applies when the old window is occluded.
        self.acquisition.take();
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
        self.configuration = Some(configure(&self.surface, gpu, size, display_sync)?);
        self.display_sync = display_sync;
        self.size = size;
        Ok(())
    }

    /// Requests one actual surface texture for an ordinary frame. The
    /// window thread can dispatch input while Metal waits for a free drawable.
    /// The callback runs after the acquisition result is available. Synchronous
    /// resize and recovery retain their direct path.
    pub fn prepare_frame(
        &mut self,
        notify: impl FnOnce() + Send + 'static,
    ) -> Result<Poll<()>, Error> {
        if self.prepared.is_some() {
            return Ok(Poll::Ready(()));
        }
        if let Some(worker) = &mut self.acquisition
            && worker.pending()
        {
            return match worker.poll()? {
                Poll::Ready(acquired) => {
                    self.prepared = Some(acquired);
                    Ok(Poll::Ready(()))
                }
                Poll::Pending => Ok(Poll::Pending),
            };
        }
        let Some(config) = &self.configuration else {
            return Ok(Poll::Ready(()));
        };
        if config.generation.load(Ordering::Acquire) {
            return Ok(Poll::Ready(()));
        }
        let surface = Arc::clone(&self.surface);
        let device = config.device.clone();
        let lost = Arc::clone(&config.generation);
        let worker = match &mut self.acquisition {
            Some(worker) => worker,
            slot @ None => slot.insert(Acquisition::new()?),
        };
        worker.start(
            move || {
                let _span = tracing::trace_span!("drawable_acquisition").entered();
                device::scoped(&device, &lost, || surface.get_current_texture())
            },
            notify,
        )?;
        Ok(Poll::Pending)
    }

    fn take_preparation(&mut self) -> Result<Option<wgpu::CurrentSurfaceTexture>, Error> {
        if let Some(acquired) = self.prepared.take() {
            return Ok(Some(acquired));
        }
        if let Some(worker) = &mut self.acquisition {
            return worker.take();
        }
        Ok(None)
    }

    /// Renders and presents on the window thread, or reports suspension/skipping.
    /// Outdated/lost surfaces are reconfigured and acquisition retried once.
    /// Loss also recreates the native surface, as required by wgpu 30.
    pub fn render(&mut self, scene: &Scene, root: NodeId) -> Result<FrameOutcome, Error> {
        if self.size.width == 0 || self.size.height == 0 {
            return Ok(FrameOutcome::Suspended);
        }
        let mut prepared = self.take_preparation()?;
        let encoded = scene
            .encode(root)
            .map_err(|e| Error::SceneEncoding(e.to_string()))?;
        let mut process = self
            .context
            .process
            .lock()
            .map_err(|e| Error::DeviceLost(e.to_string()))?;
        process.ensure_device()?;
        let device::Process { instance, gpu } = &mut *process;
        let gpu = gpu
            .as_mut()
            .ok_or_else(|| Error::DeviceLost("device unavailable".into()))?;
        if !self
            .configuration
            .as_ref()
            .is_some_and(|config| Arc::ptr_eq(&config.generation, &gpu.lost))
        {
            drop(prepared.take());
            self.configuration = Some(configure(&self.surface, gpu, self.size, self.display_sync)?);
        }
        let mut frame = None;
        for attempt in 0..2 {
            let acquired = match prepared.take() {
                Some(acquired) => acquired,
                None => scoped(gpu, || self.surface.get_current_texture())?,
            };
            match acquired {
                wgpu::CurrentSurfaceTexture::Success(texture) => {
                    frame = Some(texture);
                    break;
                }
                wgpu::CurrentSurfaceTexture::Suboptimal(texture) => {
                    self.configuration = None;
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
                    self.surface = Arc::new(create(instance, self.handle)?);
                    self.configuration =
                        Some(configure(&self.surface, gpu, self.size, self.display_sync)?);
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
        match gpu.render_scene(&encoded, self.size, Some(&frame.texture), false) {
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
) -> Result<Configuration, Error> {
    if !validate_size(size, gpu.device.limits().max_texture_dimension_2d)? {
        return Err(Error::InvalidSize(size));
    }
    let capabilities = surface.get_capabilities(&gpu.adapter);
    let mut config = surface
        .get_default_config(&gpu.adapter, size.width, size.height)
        .ok_or_else(|| Error::Surface("surface incompatible with shared adapter".into()))?;
    config.format = crate::copy::SURFACE_FORMATS
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
    // block for an interval. Actual drawable availability paces painting.
    config.desired_maximum_frame_latency = 2;
    config.usage = wgpu::TextureUsages::RENDER_ATTACHMENT;
    scoped(gpu, || surface.configure(&gpu.device, &config))?;
    Ok(Configuration {
        generation: Arc::clone(&gpu.lost),
        device: gpu.device.clone(),
    })
}

// Catch validation/resource errors at every native GPU operation. Resource
// failure invalidates shared state synchronously; all surfaces notice the new
// device identity on next use, even before wgpu delivers its loss callback.
fn scoped<T>(gpu: &device::Gpu, operation: impl FnOnce() -> T) -> Result<T, Error> {
    device::scoped(&gpu.device, &gpu.lost, operation)
}
