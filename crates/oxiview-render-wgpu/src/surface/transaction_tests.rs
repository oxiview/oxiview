use super::*;
use objc2::{AnyThread, DefinedClass, define_class, msg_send, rc::Retained};
use objc2_foundation::NSSize;
use objc2_quartz_core::{CAMetalLayer, CATransaction};

define_class!(
    // SAFETY: CAMetalLayer supports off-main-thread configuration. The
    // subclass only observes the active transaction and forwards the setter.
    #[unsafe(super = CAMetalLayer)]
    #[ivars = AtomicBool]
    struct ConfigurationTestLayer;

    impl ConfigurationTestLayer {
        #[unsafe(method(setDrawableSize:))]
        fn set_drawable_size(&self, size: NSSize) {
            self.ivars()
                .store(CATransaction::disableActions(), Ordering::Release);
            // SAFETY: Forward the exact inherited CGSize setter to CAMetalLayer.
            unsafe {
                let _: () = msg_send![super(self), setDrawableSize: size];
            }
        }
    }
);

#[test]
fn window_thread_configures_the_metal_layer_in_an_explicit_transaction() -> Result<(), String> {
    objc2::rc::autoreleasepool(|_| {
        let layer: Retained<ConfigurationTestLayer> = {
            let _initialization = LayerConfigurationTransaction::begin();
            let layer = ConfigurationTestLayer::alloc().set_ivars(AtomicBool::new(false));
            // SAFETY: The new CAMetalLayer subclass uses its inherited initializer.
            unsafe { msg_send![super(layer), init] }
        };
        let address = Retained::as_ptr(&layer).expose_provenance();
        let observed = layer.ivars();
        std::thread::scope(|scope| {
            scope
                .spawn(move || {
                    // SAFETY: The parent retains the initialized CAMetalLayer across
                    // the scoped join. Only the worker accesses its native properties.
                    objc2::rc::autoreleasepool(|_| unsafe {
                        configure_on_worker(address, observed)
                    })
                })
                .join()
                .map_err(|_| "configuration worker panicked".to_owned())?
        })
    })
}

/// # Safety
/// The exposed address must identify a CAMetalLayer that outlives this call
/// and permits off-thread configuration without any concurrent mutation.
unsafe fn configure_on_worker(address: usize, observed: &AtomicBool) -> Result<(), String> {
    let mut process = device::Process::new();
    process.ensure_device().map_err(|error| error.to_string())?;
    // SAFETY: The parent holds the initialized CAMetalLayer until the worker
    // joins. Only this worker configures it and the test never attaches it.
    let surface = unsafe {
        process
            .instance
            .create_surface_unsafe(wgpu::SurfaceTargetUnsafe::CoreAnimationLayer(
                std::ptr::with_exposed_provenance_mut(address),
            ))
    }
    .map_err(|error| error.to_string())?;
    let gpu = process.gpu.as_ref().ok_or("missing GPU")?;
    for width in [80, 96] {
        observed.store(false, Ordering::Release);
        let previous_actions = CATransaction::disableActions();
        configure(&surface, gpu, PixelSize { width, height: 64 }, true)
            .map_err(|error| error.to_string())?;
        if !observed.load(Ordering::Acquire) {
            return Err("Metal layer setter ran outside the configuration transaction".to_owned());
        }
        if CATransaction::disableActions() != previous_actions {
            return Err("Metal layer configuration left a transaction open".to_owned());
        }
    }
    Ok(())
}
