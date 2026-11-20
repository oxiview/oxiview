use crate::Error;
use std::{collections::HashMap, sync::OnceLock};

// The same pass is used for native surfaces and surface-format readback tests.
#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub(crate) enum Alpha {
    Encoded,
    Straight,
    Premultiplied,
}

pub(crate) const SURFACE_FORMATS: [wgpu::TextureFormat; 4] = [
    wgpu::TextureFormat::Bgra8UnormSrgb,
    wgpu::TextureFormat::Rgba8UnormSrgb,
    wgpu::TextureFormat::Bgra8Unorm,
    wgpu::TextureFormat::Rgba8Unorm,
];

pub(crate) struct Blitter {
    shader: wgpu::ShaderModule,
    pipelines: HashMap<(wgpu::TextureFormat, Alpha), OnceLock<wgpu::RenderPipeline>>,
}

impl Blitter {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("oxiview output conversion"),
            source: wgpu::ShaderSource::Wgsl(include_str!("copy.wgsl").into()),
        });
        let pipelines = SURFACE_FORMATS
            .into_iter()
            .flat_map(|format| {
                [Alpha::Encoded, Alpha::Premultiplied]
                    .map(|alpha| ((format, alpha), OnceLock::new()))
            })
            .chain([(
                (wgpu::TextureFormat::Rgba8Unorm, Alpha::Straight),
                OnceLock::new(),
            )])
            .collect();
        Self { shader, pipelines }
    }

    pub(crate) fn encode(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::TextureView,
        target: &wgpu::Texture,
        alpha: Alpha,
    ) -> Result<(), Error> {
        let format = target.format();
        let pipeline = self
            .pipelines
            .get(&(format, alpha))
            .ok_or_else(|| Error::Surface("unsupported output conversion".into()))?
            .get_or_init(|| create_pipeline(device, &self.shader, format, alpha));
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("surface copy"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(source),
            }],
        });
        let view = target.create_view(&Default::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.draw(0..3, 0..1);
        Ok(())
    }
}

fn create_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    format: wgpu::TextureFormat,
    alpha: Alpha,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("surface copy"),
        layout: None,
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vertex"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(match (alpha, format.is_srgb()) {
                (Alpha::Encoded, true) => "srgb",
                (Alpha::Encoded, false) => "copy_unorm",
                (Alpha::Straight, _) => "straight_unorm",
                (Alpha::Premultiplied, true) => "premultiplied_srgb",
                (Alpha::Premultiplied, false) => "premultiplied_unorm",
            }),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}
