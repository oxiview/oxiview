use crate::{Error, PixelSize, RgbaImage};
use guillotiere::{AllocId, AtlasAllocator, size2};
use half::f16;
use std::{
    collections::{HashMap, HashSet},
    sync::OnceLock,
};
use vello_gpu::{ImageSource, RectU16, TextureBindings, TextureId, TextureRegion};

const SIDE: u16 = 1024;
const MAX_ENTRIES: usize = 4096;
const ATLAS_ID: TextureId = TextureId(0);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum Key {
    Image(usize, u32, u32),
    Solid([u32; 4]),
}

pub(crate) enum Source<'a> {
    Image(&'a RgbaImage),
    Solid([f32; 4]),
}

impl Source<'_> {
    pub(crate) fn key(&self) -> Key {
        match self {
            Self::Image(image) => Key::Image(
                image.pixels().as_ptr().addr(),
                image.width(),
                image.height(),
            ),
            Self::Solid(color) => Key::Solid(color.map(f32::to_bits)),
        }
    }

    fn size(&self) -> PixelSize {
        match self {
            Self::Image(image) => PixelSize {
                width: image.width(),
                height: image.height(),
            },
            Self::Solid(_) => PixelSize {
                width: 1,
                height: 1,
            },
        }
    }

    fn pixels(&self) -> Vec<u8> {
        let premul = |[r, g, b, a]: [f32; 4]| {
            [r * a, g * a, b * a, a].map(|value| f16::from_f32(value).to_le_bytes())
        };
        match self {
            Self::Image(image) => {
                let (pixels, _) = image.pixels().as_chunks::<4>();
                let linear = srgb_to_linear();
                pixels
                    .iter()
                    .flat_map(|&[r, g, b, a]| {
                        let [r, g, b] = [r, g, b].map(|channel| linear[usize::from(channel)]);
                        premul([r, g, b, f32::from(a) / 255.0])
                            .into_iter()
                            .flatten()
                    })
                    .collect()
            }
            Self::Solid(rgba) => premul(*rgba).into_iter().flatten().collect(),
        }
    }
}

fn srgb_to_linear() -> &'static [f32; 256] {
    static CHANNELS: OnceLock<[f32; 256]> = OnceLock::new();
    CHANNELS.get_or_init(|| {
        std::array::from_fn(|channel| {
            let encoded = channel as f32 / 255.0;
            oxiview_base::Color::from_srgb([encoded, 0.0, 0.0, 1.0]).to_linear()[0]
        })
    })
}

struct Entry {
    alloc: AllocId,
    region: RectU16,
    last_used: u64,
    // A live allocation keeps the pixel pointer in Key::Image unique.
    _image: Option<RgbaImage>,
}

#[derive(Default, Debug, Clone, Copy)]
pub(crate) struct Counters {
    pub(crate) hits: u64,
    pub(crate) uploads: u64,
    pub(crate) evictions: u64,
    pub(crate) overflow_uploads: u64,
}

/// One fixed-size GPU texture per device generation. Overflow views live only for
/// the submission that uses them. Vello samples the atlas directly via TextureBindings.
pub(crate) struct Atlas {
    texture: wgpu::Texture,
    allocator: AtlasAllocator,
    entries: HashMap<Key, Entry>,
    clock: u64,
    pub(crate) counters: Counters,
}

pub(crate) struct Frame {
    pub(crate) bindings: TextureBindings,
    overflow: HashMap<Key, TextureId>,
    required: HashSet<Key>,
}

impl Atlas {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let edge = i32::from(SIDE);
        Self {
            texture: texture(
                device,
                u32::from(SIDE),
                u32::from(SIDE),
                "oxiview glyph and image atlas",
            ),
            allocator: AtlasAllocator::new(size2(edge, edge)),
            entries: HashMap::new(),
            clock: 0,
            counters: Counters::default(),
        }
    }

    pub(crate) fn frame(&self, required: HashSet<Key>) -> Frame {
        let mut bindings = TextureBindings::new();
        bindings.insert(ATLAS_ID, self.texture.create_view(&Default::default()));
        Frame {
            bindings,
            overflow: HashMap::new(),
            required,
        }
    }

    pub(crate) fn resident_count(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn image(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame: &mut Frame,
        source: Source<'_>,
    ) -> Result<ImageSource, Error> {
        self.clock = self.clock.saturating_add(1);
        let key = source.key();
        frame.required.insert(key);
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.last_used = self.clock;
            self.counters.hits += 1;
            return Ok(atlas_region(entry.region));
        }
        if let Some(id) = frame.overflow.get(&key) {
            return Ok(ImageSource::external_texture(
                *id,
                TextureRegion::Full,
                true,
            ));
        }
        let size = source.size();
        if size.width == 0 || size.height == 0 {
            return Err(Error::SceneEncoding("empty image source".into()));
        }
        if size.width <= u32::from(SIDE) && size.height <= u32::from(SIDE) {
            let dimensions = size2(size.width as i32, size.height as i32);
            loop {
                if self.entries.len() < MAX_ENTRIES
                    && let Some(allocation) = self.allocator.allocate(dimensions)
                {
                    let bounds = allocation.rectangle;
                    let region = RectU16::new(
                        bounds.min.x as u16,
                        bounds.min.y as u16,
                        bounds.max.x as u16,
                        bounds.max.y as u16,
                    );
                    upload(
                        queue,
                        &self.texture,
                        u32::from(region.x0),
                        u32::from(region.y0),
                        size,
                        &source.pixels(),
                    );
                    let image = match source {
                        Source::Image(image) => Some(image.clone()),
                        Source::Solid(_) => None,
                    };
                    self.entries.insert(
                        key,
                        Entry {
                            alloc: allocation.id,
                            region,
                            last_used: self.clock,
                            _image: image,
                        },
                    );
                    self.counters.uploads += 1;
                    return Ok(atlas_region(region));
                }
                let victim = self
                    .entries
                    .iter()
                    .filter(|(key, _)| !frame.required.contains(key))
                    .min_by_key(|(_, entry)| entry.last_used)
                    .map(|(&key, _)| key);
                let Some(entry) = victim.and_then(|key| self.entries.remove(&key)) else {
                    break;
                };
                self.allocator.deallocate(entry.alloc);
                self.counters.evictions += 1;
            }
        }
        let limit = device
            .limits()
            .max_texture_dimension_2d
            .min(u32::from(u16::MAX));
        if size.width > limit || size.height > limit {
            return Err(Error::InvalidSize(size));
        }
        let texture = texture(device, size.width, size.height, "oxiview standalone image");
        upload(queue, &texture, 0, 0, size, &source.pixels());
        let id = TextureId(frame.overflow.len() as u64 + 1);
        frame
            .bindings
            .insert(id, texture.create_view(&Default::default()));
        frame.overflow.insert(key, id);
        self.counters.overflow_uploads += 1;
        Ok(ImageSource::external_texture(id, TextureRegion::Full, true))
    }
}

fn atlas_region(region: RectU16) -> ImageSource {
    ImageSource::external_texture(ATLAS_ID, TextureRegion::Rect(region), true)
}

fn texture(device: &wgpu::Device, width: u32, height: u32, label: &'static str) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

fn upload(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    x: u32,
    y: u32,
    size: PixelSize,
    data: &[u8],
) {
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d { x, y, z: 0 },
            aspect: wgpu::TextureAspect::All,
        },
        data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(size.width * 8),
            rows_per_image: None,
        },
        wgpu::Extent3d {
            width: size.width,
            height: size.height,
            depth_or_array_layers: 1,
        },
    );
}
