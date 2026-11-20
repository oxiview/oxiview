use crate::{Error, PixelSize, RgbaImage, copy, encoding::Commands, straight, vector};
use oxiview_scene::EncodedScene;
use std::{
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll, Wake, Waker},
};
use wgpu::util::DeviceExt;

pub(crate) struct Process {
    pub(crate) instance: wgpu::Instance,
    pub(crate) gpu: Option<Gpu>,
}
pub(crate) struct Gpu {
    pub(crate) adapter: wgpu::Adapter,
    pub(crate) device: wgpu::Device,
    pub(crate) queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
    blitter: copy::Blitter,
    pub(crate) vector: Option<vector::Backend>,
    pub(crate) lost: Arc<AtomicBool>,
}
impl Process {
    pub(crate) fn new() -> Self {
        Self {
            instance: wgpu::Instance::new(wgpu::InstanceDescriptor {
                backends: wgpu::Backends::METAL | wgpu::Backends::VULKAN | wgpu::Backends::DX12,
                ..wgpu::InstanceDescriptor::new_without_display_handle()
            }),
            gpu: None,
        }
    }
    pub(crate) fn ensure_device(&mut self) -> Result<(), Error> {
        if self
            .gpu
            .as_ref()
            .is_some_and(|gpu| !gpu.lost.load(Ordering::Acquire))
        {
            return Ok(());
        }
        // Drop all resources from the previous generation before requesting another
        // adapter/device. A generation's callback cannot invalidate its successor.
        self.gpu = None;
        let adapter = block_on(
            self.instance
                .request_adapter(&wgpu::RequestAdapterOptions::default()),
        )
        .map_err(|e| Error::NoAdapter(e.to_string()))?;
        let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
            .map_err(|e| Error::DeviceRequest(e.to_string()))?;
        let lost = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&lost);
        device.set_device_lost_callback(move |_, _| {
            flag.store(true, Ordering::Release);
        });
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let memory = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("oxiview SDF and polygon rasterizer"),
            source: wgpu::ShaderSource::Wgsl(include_str!("raster.wgsl").into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("oxiview offscreen"),
            layout: None,
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let blitter = copy::Blitter::new(&device);
        let errors = [
            block_on(internal.pop()),
            block_on(memory.pop()),
            block_on(validation.pop()),
        ];
        if let Some(error) = errors.into_iter().flatten().next() {
            return Err(Error::DeviceRequest(error.to_string()));
        }
        self.gpu = Some(Gpu {
            adapter,
            device,
            queue,
            pipeline,
            blitter,
            vector: None,
            lost,
        });
        Ok(())
    }
    pub(crate) fn lose_device(&mut self) {
        if let Some(gpu) = &self.gpu {
            gpu.device.destroy();
            // Mark synchronously as callbacks may be delivered only by a later poll.
            gpu.lost.store(true, Ordering::Release);
        }
    }
    #[cfg(test)]
    pub(crate) fn render(
        &mut self,
        commands: &Commands,
        size: PixelSize,
    ) -> Result<RgbaImage, Error> {
        self.ensure_device()?;
        let gpu = self
            .gpu
            .as_ref()
            .ok_or_else(|| Error::DeviceLost("device unavailable".into()))?;
        let result = gpu
            .render(commands, size, None, true)
            .and_then(|image| image.ok_or_else(|| Error::DeviceLost("missing readback".into())));
        if matches!(result, Err(Error::DeviceLost(_))) {
            self.lose_device();
        }
        result
    }
}
impl Gpu {
    pub(crate) fn render_scene(
        &mut self,
        scene: &EncodedScene,
        size: PixelSize,
        target: Option<&wgpu::Texture>,
        read: bool,
    ) -> Result<Option<RgbaImage>, Error> {
        let limits = self.device.limits();
        if !crate::surface::validate_size(size, limits.max_texture_dimension_2d)? {
            return Err(Error::InvalidSize(size));
        }
        if !vector::needs_vello(scene) {
            // Large opacity stacks can exceed the SDF storage-buffer budget
            // even when the render target itself is supported.
            let result = self.render(&Commands::new(scene, size)?, size, target, read);
            if !matches!(result, Err(Error::InvalidSize(_))) {
                return result;
            }
        }

        let vector = &mut self.vector;
        scoped(&self.device, &self.lost, || {
            let linear = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("oxiview Vello linear color"),
                size: extent(size),
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: vector::FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let mut encoder = self.device.create_command_encoder(&Default::default());
            let backend =
                vector.get_or_insert_with(|| vector::Backend::new(&self.device, &self.queue));
            let view = linear.create_view(&Default::default());
            backend.encode(&self.device, &self.queue, &mut encoder, scene, &view, size)?;

            let offscreen;
            let output = match target {
                Some(target) => target,
                None => {
                    offscreen = self.device.create_texture(&wgpu::TextureDescriptor {
                        label: Some("oxiview straight sRGB readback"),
                        size: extent(size),
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                            | wgpu::TextureUsages::COPY_SRC,
                        view_formats: &[],
                    });
                    &offscreen
                }
            };
            let alpha = if target.is_some() {
                copy::Alpha::Premultiplied
            } else {
                copy::Alpha::Straight
            };
            self.blitter
                .encode(&self.device, &mut encoder, &view, output, alpha)?;
            Self::finish(
                &self.device,
                &self.queue,
                encoder,
                output,
                read,
                target.is_some(),
            )
        })?
    }
    pub(crate) fn render(
        &self,
        commands: &Commands,
        size: PixelSize,
        target: Option<&wgpu::Texture>,
        read: bool,
    ) -> Result<Option<RgbaImage>, Error> {
        let limits = self.device.limits();
        let stack_size = commands.opacity_stack_bytes(size)?;
        let data = commands.bytes();
        if !crate::surface::validate_size(size, limits.max_texture_dimension_2d)?
            || stack_size > limits.max_storage_buffer_binding_size
            || stack_size > limits.max_buffer_size
            || data.len() as u64 > limits.max_storage_buffer_binding_size
            || size.width.div_ceil(8) > limits.max_compute_workgroups_per_dimension
            || size.height.div_ceil(8) > limits.max_compute_workgroups_per_dimension
        {
            return Err(Error::InvalidSize(size));
        }
        scoped(&self.device, &self.lost, || {
            self.submit(&data, stack_size, size, target, read)
        })?
    }
    fn submit(
        &self,
        data: &[u8],
        stack_size: u64,
        size: PixelSize,
        target: Option<&wgpu::Texture>,
        read: bool,
    ) -> Result<Option<RgbaImage>, Error> {
        let commands = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("scene words"),
                contents: data,
                usage: wgpu::BufferUsages::STORAGE,
            });
        let stack = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("opacity stack"),
            size: stack_size,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let extent = extent(size);
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offscreen RGBA"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("offscreen inputs"),
            layout: &self.pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: commands.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: stack.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
            ],
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(size.width.div_ceil(8), size.height.div_ceil(8), 1);
        }
        if let Some(target) = target {
            self.blitter.encode(
                &self.device,
                &mut encoder,
                &view,
                target,
                copy::Alpha::Encoded,
            )?;
        }
        Self::finish(
            &self.device,
            &self.queue,
            encoder,
            target.unwrap_or(&texture),
            read,
            true,
        )
    }

    fn finish(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        mut encoder: wgpu::CommandEncoder,
        output: &wgpu::Texture,
        read: bool,
        premultiplied: bool,
    ) -> Result<Option<RgbaImage>, Error> {
        if !read {
            queue.submit([encoder.finish()]);
            return Ok(None);
        }
        let size = PixelSize {
            width: output.width(),
            height: output.height(),
        };
        let (padded_row, readback_size) = readback_layout(size, &device.limits())?;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pixel readback"),
            size: readback_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let extent = extent(size);
        encoder.copy_texture_to_buffer(
            output.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_row),
                    rows_per_image: Some(size.height),
                },
            },
            extent,
        );
        let submission = queue.submit([encoder.finish()]);
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result);
            });
        device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .map_err(|e| Error::DeviceLost(e.to_string()))?;
        receiver
            .recv_timeout(std::time::Duration::from_secs(30))
            .map_err(|e| Error::DeviceLost(e.to_string()))?
            .map_err(|e| Error::DeviceLost(e.to_string()))?;
        let mapped = readback
            .slice(..)
            .get_mapped_range()
            .map_err(|e| Error::DeviceLost(e.to_string()))?;
        let mut pixels = Vec::with_capacity(size.width as usize * size.height as usize * 4);
        for row in mapped.chunks_exact(padded_row as usize) {
            for pixel in row.chunks_exact(4).take(size.width as usize) {
                let [r, g, b, a] = *pixel else {
                    continue;
                };
                let (r, b) = if matches!(
                    output.format(),
                    wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
                ) {
                    (b, r)
                } else {
                    (r, b)
                };
                pixels.extend(if premultiplied {
                    [straight(r, a), straight(g, a), straight(b, a), a]
                } else {
                    [r, g, b, a]
                });
            }
        }
        drop(mapped);
        readback.unmap();
        RgbaImage::new(size, pixels)
            .map(Some)
            .map_err(|e| Error::SceneEncoding(e.to_string()))
    }
}

fn extent(size: PixelSize) -> wgpu::Extent3d {
    wgpu::Extent3d {
        width: size.width,
        height: size.height,
        depth_or_array_layers: 1,
    }
}

fn readback_layout(size: PixelSize, limits: &wgpu::Limits) -> Result<(u32, u64), Error> {
    let row = (u64::from(size.width) * 4).div_ceil(256) * 256;
    let bytes = row
        .checked_mul(u64::from(size.height))
        .ok_or(Error::InvalidSize(size))?;
    if bytes > limits.max_buffer_size {
        return Err(Error::InvalidSize(size));
    }
    Ok((
        u32::try_from(row).map_err(|_| Error::InvalidSize(size))?,
        bytes,
    ))
}

// wgpu scopes are thread-local, including those around worker acquisition.
pub(crate) fn scoped<T>(
    device: &wgpu::Device,
    lost: &AtomicBool,
    operation: impl FnOnce() -> T,
) -> Result<T, Error> {
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let memory = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
    let result = operation();
    let errors = [
        block_on(internal.pop()),
        block_on(memory.pop()),
        block_on(validation.pop()),
    ];
    if let Some(error) = errors.into_iter().flatten().next() {
        lost.store(true, Ordering::Release);
        return Err(Error::DeviceLost(error.to_string()));
    }
    if lost.load(Ordering::Acquire) {
        return Err(Error::DeviceLost("device loss callback".into()));
    }
    Ok(result)
}

// Native adapter/device requests and error scopes need an executor, not a runtime.
// Wake uses the standard library and requires no unsafe executor code.
pub(crate) fn block_on<F: Future>(future: F) -> F::Output {
    struct ThreadWake(std::thread::Thread);
    impl Wake for ThreadWake {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Waker::from(Arc::new(ThreadWake(std::thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(value) => return value,
            Poll::Pending => std::thread::park(),
        }
    }
}
