use crate::{Error, PixelSize, copy::Alpha};
use block2::RcBlock;
use objc2::{
    rc::Retained,
    runtime::{AnyObject, ProtocolObject},
};
use objc2_foundation::{NSString, ns_string};
use objc2_metal::{
    MTLCommandBuffer, MTLCommandEncoder, MTLCommandQueue, MTLDevice, MTLHazardTrackingMode,
    MTLLibrary, MTLLoadAction, MTLPixelFormat, MTLPrimitiveType, MTLRenderCommandEncoder,
    MTLRenderPassDescriptor, MTLRenderPipelineDescriptor, MTLRenderPipelineState, MTLResource,
    MTLStoreAction, MTLTexture,
};
use objc2_quartz_core::CAMetalDrawable;
use std::{
    any::Any,
    collections::HashMap,
    ptr::{self, NonNull},
    sync::{Arc, MutexGuard, atomic::Ordering},
};

type Pipeline = Retained<ProtocolObject<dyn MTLRenderPipelineState>>;
type CommandBuffer = Retained<ProtocolObject<dyn MTLCommandBuffer>>;
pub(crate) type Drawable = Retained<ProtocolObject<dyn CAMetalDrawable>>;

pub(crate) fn drawable_resource(resource: &dyn Any) -> Option<&Drawable> {
    resource.downcast_ref()
}

/// Optional progress observation for the display-link diagnostics.
pub type MetalRenderObserver = Arc<dyn Fn(MetalRenderEvent) + Send + Sync>;

/// Stages of native rendering observed by the optional timing collector.
pub enum MetalRenderEvent {
    /// The scene was encoded.
    SceneEncoded,
    /// The wgpu commands were submitted.
    SceneSubmitted,
    /// Metal scheduled the final output command.
    Scheduled,
    /// Metal finished the final output command.
    Completed {
        /// GPU host-clock start time in seconds.
        start: f64,
        /// GPU host-clock end time in seconds.
        end: f64,
        /// Native completion status value.
        status: usize,
    },
}

pub(crate) struct FrameChecks<'a> {
    pub can_submit: &'a dyn Fn() -> bool,
    pub observer: Option<&'a MetalRenderObserver>,
}

#[derive(Default)]
pub(crate) struct Blitter {
    pipelines: HashMap<(wgpu::TextureFormat, Alpha), Pipeline>,
}

impl Blitter {
    pub(crate) fn warm_up(
        &mut self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
    ) -> Result<(), Error> {
        // SAFETY: Read the Metal device during the HAL guard's lifetime.
        let hal = unsafe { device.as_hal::<wgpu::hal::api::Metal>() }
            .ok_or_else(|| Error::Surface("Metal device unavailable".into()))?;
        if self.pipelines.contains_key(&(format, Alpha::Encoded)) {
            return Ok(());
        }
        let native = hal.raw_device();
        let library = native
            .newLibraryWithSource_options_error(
                &NSString::from_str(include_str!("copy.metal")),
                None,
            )
            .map_err(|e| Error::Surface(format!("Metal output library: {e}")))?;
        let mut pair = Vec::with_capacity(2);
        for (alpha, shader) in [
            (
                Alpha::Encoded,
                if format.is_srgb() {
                    "copy_srgb"
                } else {
                    "copy_unorm"
                },
            ),
            (
                Alpha::Premultiplied,
                if format.is_srgb() {
                    "premultiplied_srgb"
                } else {
                    "premultiplied_unorm"
                },
            ),
        ] {
            let name = NSString::from_str(shader);
            let vertex = library.newFunctionWithName(ns_string!("vertex_copy"));
            let fragment = library.newFunctionWithName(&name);
            if vertex.is_none() || fragment.is_none() {
                return Err(Error::Surface("Metal output shader unavailable".into()));
            }
            let desc = MTLRenderPipelineDescriptor::new();
            desc.setVertexFunction(vertex.as_deref());
            desc.setFragmentFunction(fragment.as_deref());
            // SAFETY: A render pipeline descriptor always has color slot 0.
            unsafe { desc.colorAttachments().objectAtIndexedSubscript(0) }
                .setPixelFormat(native_format(format)?);
            let pipeline = native
                .newRenderPipelineStateWithDescriptor_error(&desc)
                .map_err(|e| Error::Surface(format!("Metal output pipeline: {e}")))?;
            pair.push(((format, alpha), pipeline));
        }
        self.pipelines.extend(pair);
        Ok(())
    }

    fn encode(
        &self,
        queue: &wgpu::Queue,
        source: &wgpu::Texture,
        target: &ProtocolObject<dyn MTLTexture>,
        format: wgpu::TextureFormat,
        alpha: Alpha,
    ) -> Result<CommandBuffer, Error> {
        // SAFETY: Borrow the live source and queue without mutating their HAL
        // state. The retained-reference native command buffer owns all encoded
        // Metal resources through completion. Submissions are serialized by Gpu.
        let texture = unsafe { source.as_hal::<wgpu::hal::api::Metal>() }
            .ok_or_else(|| Error::Surface("Metal output source unavailable".into()))?;
        // SAFETY: Same lifetime and synchronization contract as the source guard.
        let queue = unsafe { queue.as_hal::<wgpu::hal::api::Metal>() }
            .ok_or_else(|| Error::Surface("Metal output queue unavailable".into()))?;
        let source = texture.raw_handle();
        // The preceding wgpu scene commands and this final pass access the
        // identical directly bound texture on the same MTLCommandQueue. Metal
        // tracks that write/read dependency across command buffers.
        if source.hazardTrackingMode() != MTLHazardTrackingMode::Tracked {
            return Err(Error::Surface(
                "Metal output needs a hazard-tracked scene texture".into(),
            ));
        }
        let command = queue.as_raw().commandBuffer().ok_or_else(|| {
            Error::DeviceLost("could not allocate the Metal output command buffer".into())
        })?;
        command.setLabel(Some(ns_string!("oxiview display-link output")));
        let desc = MTLRenderPassDescriptor::renderPassDescriptor();
        // SAFETY: A render pass descriptor always has color slot 0.
        let color = unsafe { desc.colorAttachments().objectAtIndexedSubscript(0) };
        color.setTexture(Some(target));
        color.setLoadAction(MTLLoadAction::Clear);
        color.setStoreAction(MTLStoreAction::Store);
        let pipeline = self
            .pipelines
            .get(&(format, alpha))
            .ok_or_else(|| Error::Surface("Metal output pipeline was not configured".into()))?;
        let pass = command
            .renderCommandEncoderWithDescriptor(&desc)
            .ok_or_else(|| Error::Surface("could not encode the Metal output pass".into()))?;
        pass.setRenderPipelineState(pipeline);
        // SAFETY: The source matches fragment slot 0 and covers the attachment;
        // this vertex shader reads only its built-in ID for three vertices.
        unsafe {
            pass.setFragmentTexture_atIndex(Some(source), 0);
            pass.drawPrimitives_vertexStart_vertexCount(MTLPrimitiveType::Triangle, 0, 3);
        }
        pass.endEncoding();
        Ok(command)
    }
}

fn native_format(format: wgpu::TextureFormat) -> Result<MTLPixelFormat, Error> {
    Ok(match format {
        wgpu::TextureFormat::Bgra8UnormSrgb => MTLPixelFormat::BGRA8Unorm_sRGB,
        wgpu::TextureFormat::Bgra8Unorm => MTLPixelFormat::BGRA8Unorm,
        wgpu::TextureFormat::Rgba8UnormSrgb => MTLPixelFormat::RGBA8Unorm_sRGB,
        wgpu::TextureFormat::Rgba8Unorm => MTLPixelFormat::RGBA8Unorm,
        _ => return Err(Error::Surface("unsupported Metal output format".into())),
    })
}

pub(crate) struct Target {
    texture: Retained<ProtocolObject<dyn MTLTexture>>,
    format: wgpu::TextureFormat,
    pub(crate) command: Option<CommandBuffer>,
}

impl Target {
    pub(crate) fn new(
        drawable: &Drawable,
        device: &wgpu::Device,
        size: PixelSize,
        format: wgpu::TextureFormat,
    ) -> Result<Option<Self>, Error> {
        // SAFETY: Read the native device identity while holding the HAL guard.
        let hal = unsafe { device.as_hal::<wgpu::hal::api::Metal>() }
            .ok_or_else(|| Error::Surface("Metal device unavailable".into()))?;
        let device: &ProtocolObject<dyn MTLDevice> = hal.raw_device();
        Self::for_device(drawable, device.as_ref(), size, format)
    }

    fn for_device(
        drawable: &Drawable,
        device: &AnyObject,
        size: PixelSize,
        format: wgpu::TextureFormat,
    ) -> Result<Option<Self>, Error> {
        let texture = drawable.texture();
        let actual_device = texture.device();
        let actual_device: &ProtocolObject<dyn MTLDevice> = &actual_device;
        if texture.width() != size.width as usize
            || texture.height() != size.height as usize
            || texture.pixelFormat() != native_format(format)?
            || !ptr::eq(actual_device.as_ref(), device)
        {
            return Ok(None);
        }
        Ok(Some(Self {
            texture,
            format,
            command: None,
        }))
    }

    pub(crate) fn submit_scene(
        &mut self,
        blitter: &Blitter,
        queue: &wgpu::Queue,
        encoder: wgpu::CommandEncoder,
        source: &wgpu::Texture,
        alpha: Alpha,
        checks: &FrameChecks<'_>,
    ) -> Result<(), Error> {
        self.command = None;
        if !(checks.can_submit)() {
            return Ok(());
        }
        let command = blitter.encode(queue, source, &self.texture, self.format, alpha)?;
        if (checks.can_submit)() {
            queue.submit([encoder.finish()]);
            if let Some(observe) = checks.observer {
                observe(MetalRenderEvent::SceneSubmitted);
            }
            self.command = Some(command);
        }
        Ok(())
    }
}

/// A final command buffer for one exact drawable, consumed by presentation.
/// While it is live, it serializes submission with the shared render device.
/// Submit or drop it before calling other rendering APIs.
#[must_use]
pub struct MetalPresentation<'a> {
    process: MutexGuard<'a, crate::device::Process>,
    command: CommandBuffer,
    drawable: Drawable,
}

impl<'a> MetalPresentation<'a> {
    pub(crate) fn new(
        process: MutexGuard<'a, crate::device::Process>,
        command: CommandBuffer,
        drawable: &Drawable,
        observer: Option<&MetalRenderObserver>,
    ) -> Self {
        if let Some(observer) = observer {
            let observe = Arc::clone(observer);
            let scheduled =
                RcBlock::new(move |_: NonNull<ProtocolObject<dyn MTLCommandBuffer>>| {
                    observe_from_metal(&observe, MetalRenderEvent::Scheduled);
                });
            let observe = Arc::clone(observer);
            let completed = RcBlock::new(
                move |command: NonNull<ProtocolObject<dyn MTLCommandBuffer>>| {
                    // SAFETY: Metal owns this live command buffer through the callback.
                    let command = unsafe { command.as_ref() };
                    observe_from_metal(
                        &observe,
                        MetalRenderEvent::Completed {
                            start: command.GPUStartTime(),
                            end: command.GPUEndTime(),
                            status: command.status().0,
                        },
                    );
                },
            );
            // SAFETY: Metal copies both heap blocks before returning. Both closures
            // use the exact callback ABI and capture a Send + Sync observer.
            unsafe {
                command.addScheduledHandler(RcBlock::as_ptr(&scheduled));
                command.addCompletedHandler(RcBlock::as_ptr(&completed));
            }
        }
        Self {
            process,
            command,
            drawable: drawable.clone(),
        }
    }

    /// Commits the final output command using the same resource supplied by the
    /// platform lease's submission callback. Returns false on a stale device or
    /// if the resource is for a different opportunity.
    pub fn present(self, resource: &dyn Any) -> bool {
        if !drawable_resource(resource)
            .is_some_and(|drawable| ptr::eq(&**drawable, &*self.drawable))
            || self
                .process
                .gpu
                .as_ref()
                .is_none_or(|gpu| gpu.lost.load(Ordering::Acquire))
        {
            return false;
        }
        self.command
            .presentDrawable(ProtocolObject::from_ref(&*self.drawable));
        self.command.commit();
        true
    }
}

// Observer code may run on a Metal callback thread across the Objective-C ABI.
fn observe_from_metal(observer: &MetalRenderObserver, event: MetalRenderEvent) {
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| observer(event))).is_err() {
        tracing::error!("Metal timing observer panicked");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RenderContext, RgbaImage, copy::SURFACE_FORMATS, straight};
    use objc2::runtime::NSObject;
    use objc2_foundation::NSSize;
    use objc2_metal::{
        MTLCommandBufferStatus, MTLOrigin, MTLRegion, MTLSize, MTLStorageMode,
        MTLTextureDescriptor, MTLTextureUsage,
    };
    use objc2_quartz_core::CAMetalLayer;
    use oxiview_base::{Color, CornerRadii, Rect};
    use oxiview_scene::{Brush, NodeKind, Primitive, Scene};

    #[test]
    fn native_resource_validation_checks_type_extent_format_and_device()
    -> Result<(), Box<dyn std::error::Error>> {
        objc2::rc::autoreleasepool(|_| {
            let context = RenderContext::new()?;
            let process = context.process.lock().map_err(|e| e.to_string())?;
            let gpu = process.gpu.as_ref().ok_or("missing Metal device")?;
            // SAFETY: Keep the native device and queue alive with their HAL guards.
            let device = unsafe { gpu.device.as_hal::<wgpu::hal::api::Metal>() }
                .ok_or("missing Metal device HAL")?;
            // SAFETY: Access the queue only for this test while the process is locked.
            let queue = unsafe { gpu.queue.as_hal::<wgpu::hal::api::Metal>() }
                .ok_or("missing Metal queue HAL")?;
            let size = PixelSize {
                width: 16,
                height: 17,
            };
            let format = wgpu::TextureFormat::Bgra8Unorm;
            let layer = CAMetalLayer::new();
            layer.setDevice(Some(device.raw_device()));
            layer.setPixelFormat(native_format(format)?);
            layer.setDrawableSize(NSSize::new(f64::from(size.width), f64::from(size.height)));
            let first = layer
                .nextDrawable()
                .ok_or("missing first detached drawable")?;
            let second = layer
                .nextDrawable()
                .ok_or("missing second detached drawable")?;
            let drawable = drawable_resource(&first).ok_or("native drawable downcast failed")?;
            assert!(ptr::eq(&**drawable, &*first));
            let native_fake = Retained::as_ptr(&first).addr();
            assert!(drawable_resource(&native_fake).is_none());
            assert!(drawable_resource(&*first).is_none());
            assert!(drawable_resource(&NSObject::new()).is_none());

            let target = Target::new(drawable, &gpu.device, size, format)?;
            assert!(target.is_some());
            assert!(
                Target::new(
                    drawable,
                    &gpu.device,
                    PixelSize { width: 17, ..size },
                    format
                )?
                .is_none()
            );
            assert!(
                Target::new(
                    drawable,
                    &gpu.device,
                    size,
                    wgpu::TextureFormat::Bgra8UnormSrgb
                )?
                .is_none()
            );
            // A separate NSObject supplies a distinct native object identity
            // even on a machine with a single Metal device.
            let other = NSObject::new();
            assert!(Target::for_device(drawable, other.as_ref(), size, format)?.is_none());
            let command = queue
                .as_raw()
                .commandBuffer()
                .ok_or("missing command buffer")?;
            drop(queue);
            drop(device);

            let presentation = MetalPresentation::new(process, command.clone(), drawable, None);
            assert!(!presentation.present(&second));
            assert_eq!(command.status(), MTLCommandBufferStatus::NotEnqueued);

            let process = context.process.lock().map_err(|e| e.to_string())?;
            let presentation = MetalPresentation::new(process, command.clone(), drawable, None);
            assert!(presentation.present(&first));
            assert_ne!(command.status(), MTLCommandBufferStatus::NotEnqueued);
            Ok(())
        })
    }

    #[test]
    fn metal_output_draws_current_scene_into_distinct_textures()
    -> Result<(), Box<dyn std::error::Error>> {
        let size = PixelSize {
            width: 17,
            height: 19,
        };
        let context = RenderContext::new()?;
        let mut process = context.process.lock().map_err(|e| e.to_string())?;
        let gpu = process.gpu.as_mut().ok_or("missing Metal device")?;
        let mut scene = Scene::new();
        let first = scene.insert(
            None,
            0,
            NodeKind::Draw(Primitive::Fill(
                Rect::new(1.0, 2.0, 9.0, 10.0),
                CornerRadii::uniform(2.0),
                Brush::Solid(Color::from_rgba8(85, 46, 218, 204)),
            )),
        )?;
        let second = scene.insert(
            None,
            0,
            NodeKind::Draw(Primitive::Image(
                Rect::new(8.0, 7.0, 16.0, 18.0),
                RgbaImage::new(
                    PixelSize {
                        width: 1,
                        height: 1,
                    },
                    vec![14, 207, 95, 153],
                )?,
            )),
        )?;
        let first = scene.encode(first)?;
        let second = scene.encode(second)?;
        assert!(!crate::vector::needs_vello(&first));
        assert!(crate::vector::needs_vello(&second));
        for format in SURFACE_FORMATS {
            gpu.metal_blitter.warm_up(&gpu.device, format)?;
            // SAFETY: Obtain and retain the matching native wgpu device.
            let device = unsafe { gpu.device.as_hal::<wgpu::hal::api::Metal>() }
                .ok_or("missing Metal HAL")?
                .raw_device()
                .clone();
            // SAFETY: The small nonzero extent and format are valid Metal arguments.
            let desc = unsafe {
                MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                    native_format(format)?,
                    size.width as usize,
                    size.height as usize,
                    false,
                )
            };
            desc.setStorageMode(MTLStorageMode::Shared);
            desc.setUsage(MTLTextureUsage::RenderTarget);
            let a = device
                .newTextureWithDescriptor(&desc)
                .ok_or("first Metal test texture")?;
            let b = device
                .newTextureWithDescriptor(&desc)
                .ok_or("second Metal test texture")?;
            assert_ne!(Retained::as_ptr(&a), Retained::as_ptr(&b));
            let mut targets = [a, b].map(|texture| Target {
                texture,
                format,
                command: None,
            });

            // Queue the scenes without a CPU-side GPU wait between them.
            for (encoded, target) in [&first, &second].into_iter().zip(&mut targets) {
                gpu.render_metal(
                    encoded,
                    size,
                    target,
                    &FrameChecks {
                        can_submit: &|| true,
                        observer: None,
                    },
                )?;
                target
                    .command
                    .as_ref()
                    .ok_or("missing test render command")?
                    .commit();
            }
            for (encoded, target) in [&first, &second].into_iter().zip(&targets) {
                let command = target.command.as_ref().ok_or("missing submitted command")?;
                command.waitUntilCompleted();
                assert_eq!(command.status(), MTLCommandBufferStatus::Completed);
                let reference = gpu.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("Metal output parity reference"),
                    size: wgpu::Extent3d {
                        width: size.width,
                        height: size.height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                });
                let expected = gpu
                    .render_scene(encoded, size, Some(&reference), true)?
                    .ok_or("missing reference pixels")?;
                let mut raw = vec![0u8; size.width as usize * size.height as usize * 4];
                // SAFETY: The completed command wrote every pixel. The shared
                // RGBA/BGRA8 texture and full region exactly fit this buffer.
                unsafe {
                    target.texture.getBytes_bytesPerRow_fromRegion_mipmapLevel(
                        NonNull::new(raw.as_mut_ptr().cast()).ok_or("missing pixel buffer")?,
                        size.width as usize * 4,
                        MTLRegion {
                            origin: MTLOrigin { x: 0, y: 0, z: 0 },
                            size: MTLSize {
                                width: size.width as usize,
                                height: size.height as usize,
                                depth: 1,
                            },
                        },
                        0,
                    );
                }
                for p in raw.chunks_exact_mut(4) {
                    if matches!(
                        format,
                        wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
                    ) {
                        p.swap(0, 2);
                    }
                    let alpha = p[3];
                    for channel in &mut p[..3] {
                        *channel = straight(*channel, alpha);
                    }
                }
                assert_eq!(
                    raw,
                    expected.pixels(),
                    "Metal output differs for {format:?}"
                );
            }
        }
        Ok(())
    }
}
