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

/// Result of preparing the encoded native presentation.
#[cfg(target_os = "macos")]
pub enum MetalFrameOutcome<'a> {
    /// The encoded final command for this exact resource.
    Ready(crate::MetalPresentation<'a>),
    /// A stale opportunity, suspended surface, or incompatible resource.
    Skipped,
    /// The caller must release the old drawable and reconfigure before retrying.
    Reconfigure,
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
    #[cfg(target_os = "macos")]
    metal_drawables: bool,
    configuration: Option<Configuration>,
}

struct Configuration {
    generation: Arc<AtomicBool>,
    device: wgpu::Device,
    #[cfg(target_os = "macos")]
    format: wgpu::TextureFormat,
}

// wgpu configures CAMetalLayer properties on the window thread. That thread
// has no Core Animation run loop to commit an implicit colorspace or size change.
#[cfg(target_os = "macos")]
struct LayerConfigurationTransaction;

#[cfg(target_os = "macos")]
impl LayerConfigurationTransaction {
    fn begin() -> Self {
        objc2_quartz_core::CATransaction::begin();
        objc2_quartz_core::CATransaction::setDisableActions(true);
        Self
    }
}

#[cfg(target_os = "macos")]
impl Drop for LayerConfigurationTransaction {
    fn drop(&mut self) {
        objc2_quartz_core::CATransaction::commit();
    }
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
            #[cfg(target_os = "macos")]
            metal_drawables: false,
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
            .as_mut()
            .ok_or_else(|| Error::DeviceLost("device unavailable".into()))?;
        let config = configure(&self.surface, gpu, size, display_sync)?;
        #[cfg(target_os = "macos")]
        if self.metal_drawables {
            gpu.metal_blitter.warm_up(&gpu.device, config.format)?;
        }
        self.configuration = Some(config);
        self.display_sync = display_sync;
        self.size = size;
        Ok(())
    }

    /// Prepares the native output pipelines at the next surface configuration.
    #[cfg(target_os = "macos")]
    pub fn enable_metal_drawables(&mut self) {
        self.metal_drawables = true;
    }

    /// Reconfigures the existing surface after the old drawable has been released.
    #[cfg(target_os = "macos")]
    pub fn reconfigure_metal(&mut self) -> Result<(), Error> {
        self.configure(self.size, self.display_sync)
    }

    /// Encodes the final output command for the exact platform drawable.
    /// The caller presents it only through the platform's submission guard.
    #[cfg(target_os = "macos")]
    pub fn render_metal(
        &self,
        scene: &Scene,
        root: NodeId,
        resource: &dyn std::any::Any,
        can_submit: impl Fn() -> bool,
        observer: Option<&crate::MetalRenderObserver>,
    ) -> Result<MetalFrameOutcome<'_>, Error> {
        objc2::rc::autoreleasepool(|_| {
            let checks = crate::metal_output::FrameChecks {
                can_submit: &can_submit,
                observer,
            };
            self.render_metal_frame(scene, root, resource, &checks)
        })
    }

    #[cfg(target_os = "macos")]
    fn render_metal_frame(
        &self,
        scene: &Scene,
        root: NodeId,
        resource: &dyn std::any::Any,
        checks: &crate::metal_output::FrameChecks<'_>,
    ) -> Result<MetalFrameOutcome<'_>, Error> {
        use crate::metal_output::{MetalPresentation, MetalRenderEvent, Target, drawable_resource};

        if self.size.width == 0 || self.size.height == 0 || !(checks.can_submit)() {
            return Ok(MetalFrameOutcome::Skipped);
        }
        let Some(drawable) = drawable_resource(resource) else {
            return Ok(MetalFrameOutcome::Skipped);
        };
        let mut process = self
            .context
            .process
            .lock()
            .map_err(|e| Error::DeviceLost(e.to_string()))?;
        let (Some(gpu), Some(config)) = (process.gpu.as_mut(), self.configuration.as_ref()) else {
            return Ok(MetalFrameOutcome::Reconfigure);
        };
        if gpu.lost.load(Ordering::Acquire) || !Arc::ptr_eq(&config.generation, &gpu.lost) {
            return Ok(MetalFrameOutcome::Reconfigure);
        }
        if !(checks.can_submit)() {
            return Ok(MetalFrameOutcome::Skipped);
        }
        let Some(mut target) = Target::new(drawable, &gpu.device, self.size, config.format)? else {
            return Ok(MetalFrameOutcome::Skipped);
        };
        let encoded = scene
            .encode(root)
            .map_err(|e| Error::SceneEncoding(e.to_string()))?;
        if let Some(observe) = checks.observer {
            observe(MetalRenderEvent::SceneEncoded);
        }
        if let Err(error) = gpu.render_metal(&encoded, self.size, &mut target, checks) {
            if matches!(error, Error::DeviceLost(_)) {
                tracing::warn!(%error, "reconfiguring native drawable surface after device loss");
                process.lose_device();
                return Ok(MetalFrameOutcome::Reconfigure);
            }
            return Err(error);
        }
        let Some(command) = target.command else {
            return Ok(MetalFrameOutcome::Skipped);
        };
        Ok(MetalFrameOutcome::Ready(MetalPresentation::new(
            process,
            command,
            drawable,
            checks.observer,
        )))
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
    scoped(gpu, || {
        #[cfg(target_os = "macos")]
        let _layer_changes = LayerConfigurationTransaction::begin();
        surface.configure(&gpu.device, &config);
    })?;
    Ok(Configuration {
        generation: Arc::clone(&gpu.lost),
        device: gpu.device.clone(),
        #[cfg(target_os = "macos")]
        format: config.format,
    })
}

// Catch validation/resource errors at every native GPU operation. Resource
// failure invalidates shared state synchronously; all surfaces notice the new
// device identity on next use, even before wgpu delivers its loss callback.
fn scoped<T>(gpu: &device::Gpu, operation: impl FnOnce() -> T) -> Result<T, Error> {
    device::scoped(&gpu.device, &gpu.lost, operation)
}

#[cfg(all(test, target_os = "macos"))]
mod transaction_tests;
