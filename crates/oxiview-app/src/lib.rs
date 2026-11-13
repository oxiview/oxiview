//! Unstable composition root for headless CPU and native macOS GPU windows.
//! Application and presenter state are initialized on each window thread.

use oxiview_core::{Application, PaintedScene, PresentationOutcome, Presenter};
use oxiview_platform::{EventLoopContext, PhysicalSize, Platform, WindowAttributes, WindowId};
use oxiview_platform_headless::HeadlessPlatform;
use oxiview_render_cpu::{CpuRenderer, Renderer};
use oxiview_scene::{PixelSize, RgbaImage};
use std::{
    sync::{Arc, Mutex},
    thread,
};

/// Composition failures.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Platform startup or shutdown failed.
    #[error("platform failed: {0}")]
    Platform(#[from] oxiview_platform::Error),
    /// Window-thread execution failed.
    #[error("window failed: {0}")]
    Core(#[from] oxiview_core::Error),
    /// Thread creation, joining, or observation failed.
    #[error("headless composition failed: {0}")]
    Operation(String),
}
/// CPU presentation state, copied on demand by tests.
#[derive(Clone, Default)]
pub struct Output {
    /// Last successfully rendered physical-pixel image.
    pub image: Option<RgbaImage>,
    /// Successful presentations, including initial resize.
    pub presents: u64,
}
/// Cloneable observation handle. Does not keep the window alive.
#[derive(Clone, Default)]
pub struct HeadlessOutput(Arc<Mutex<Output>>);
impl HeadlessOutput {
    /// Copies the current image and count; poisoned state is an explicit error.
    pub fn snapshot(&self) -> Result<Output, Error> {
        self.0
            .lock()
            .map(|state| state.clone())
            .map_err(|_| Error::Operation("output lock poisoned".into()))
    }
}
struct CpuPresenter {
    renderer: CpuRenderer,
    size: PhysicalSize,
    output: HeadlessOutput,
}
impl Presenter for CpuPresenter {
    fn configure(
        &mut self,
        size: PhysicalSize,
        _scale_factor: f64,
    ) -> Result<(), oxiview_core::Error> {
        self.size = size;
        Ok(())
    }
    fn present(
        &mut self,
        scene: &PaintedScene,
    ) -> Result<PresentationOutcome, oxiview_core::Error> {
        let image = self
            .renderer
            .render(
                &scene.scene,
                scene.root,
                PixelSize {
                    width: self.size.width,
                    height: self.size.height,
                },
            )
            .map_err(|error| oxiview_core::Error::Present(error.to_string()))?;
        let mut output = self
            .output
            .0
            .lock()
            .map_err(|_| oxiview_core::Error::Present("output lock poisoned".into()))?;
        output.presents = output
            .presents
            .checked_add(1)
            .ok_or_else(|| oxiview_core::Error::Present("present counter exhausted".into()))?;
        output.image = Some(image);
        Ok(PresentationOutcome::Presented)
    }
}
/// Creates one window and runs its core loop on a new window thread.
/// Call on the thread that created `platform`. `application` constructs state on
/// the window thread, so the application itself need not be Send. `ready` runs on
/// the main thread during initialization and can publish identity/output to a
/// driver thread or request exit. It must return before initial resize delivery.
/// Scene coordinates are physical pixels; the application applies scale in paint.
pub fn run_headless<A: Application>(
    platform: HeadlessPlatform,
    attributes: WindowAttributes,
    application: impl FnOnce() -> Result<A, oxiview_core::Error> + Send + 'static,
    ready: impl FnOnce(
        &mut dyn EventLoopContext,
        WindowId,
        HeadlessOutput,
    ) -> Result<(), oxiview_platform::Error>,
) -> Result<(), Error> {
    let mut worker = None;
    let result = platform.run(|context| {
        let window = context.create_window(attributes)?;
        let id = window.main.id();
        let output = HeadlessOutput::default();
        let observed = output.clone();
        worker = Some(
            thread::Builder::new()
                .name("oxiview-window".into())
                .spawn(move || {
                    oxiview_core::run_window(window.endpoint, move |_| {
                        Ok((
                            application()?,
                            CpuPresenter {
                                renderer: CpuRenderer,
                                size: PhysicalSize {
                                    width: 0,
                                    height: 0,
                                },
                                output,
                            },
                        ))
                    })
                })
                .map_err(|error| oxiview_platform::Error::OperationFailed(error.to_string()))?,
        );
        ready(context, id, observed)
    });
    let joined = match worker {
        Some(worker) => worker
            .join()
            .map_err(|_| Error::Operation("window thread unwind escaped boundary".into()))?
            .map_err(Error::Core),
        None => Ok(()),
    };
    result?;
    joined
}

#[cfg(target_os = "macos")]
struct SurfacePresenter(oxiview_render_wgpu::WindowSurface);
#[cfg(target_os = "macos")]
impl Presenter for SurfacePresenter {
    fn configure(&mut self, size: PhysicalSize, _: f64) -> Result<(), oxiview_core::Error> {
        self.0
            .configure(PixelSize {
                width: size.width,
                height: size.height,
            })
            .map_err(|e| oxiview_core::Error::Present(e.to_string()))
    }
    fn present(
        &mut self,
        scene: &PaintedScene,
    ) -> Result<PresentationOutcome, oxiview_core::Error> {
        use oxiview_render_wgpu::FrameOutcome;
        match self
            .0
            .render(&scene.scene, scene.root)
            .map_err(|e| oxiview_core::Error::Present(e.to_string()))?
        {
            FrameOutcome::Presented => Ok(PresentationOutcome::Presented),
            FrameOutcome::Skipped | FrameOutcome::Suspended => Ok(PresentationOutcome::Skipped),
        }
    }
}
/// Runs one native macOS window. Call on the process main thread.
/// The application factory and GPU surface initialization run on the window thread.
#[cfg(target_os = "macos")]
pub fn run_macos<A: Application>(
    platform: oxiview_platform_macos::MacOsPlatform,
    attributes: WindowAttributes,
    application: impl FnOnce() -> Result<A, oxiview_core::Error> + Send + 'static,
) -> Result<(), Error> {
    let mut worker = None;
    let result = platform.run(|context| {
        let window = context.create_window(attributes)?;
        worker = Some(
            thread::Builder::new()
                .name("oxiview-window".into())
                .spawn(move || {
                    oxiview_core::run_window(window.endpoint, |endpoint| {
                        let handle = endpoint.native_surface_handle()?.ok_or_else(|| {
                            oxiview_core::Error::Present("native surface unavailable".into())
                        })?;
                        let surface = oxiview_render_wgpu::RenderContext::new()
                            .and_then(|context| context.create_surface(handle))
                            .map_err(|error| oxiview_core::Error::Present(error.to_string()))?;
                        Ok((application()?, SurfacePresenter(surface)))
                    })
                })
                .map_err(|e| oxiview_platform::Error::OperationFailed(e.to_string()))?,
        );
        Ok(())
    });
    let joined = match worker {
        Some(worker) => worker
            .join()
            .map_err(|_| Error::Operation("window thread unwind escaped boundary".into()))?
            .map_err(Error::Core),
        None => Ok(()),
    };
    result?;
    joined
}
