// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Straight sRGB image inputs and frame output for a linear float compositor.

use alloc::vec::Vec;
use vello_common::TextureId;
use vello_common::color::{ColorSpace, Srgb};
use vello_common::paint::{Image, ImageSource, TextureRegion};
use vello_common::peniko::{ImageQuality, ImageSampler};

use crate::fine::{Numeric, TILE_HEIGHT_COMPONENTS};
use crate::region::Region;

/// Source images for one invocation of
/// [`crate::RenderContext::render_linear`].
#[derive(Debug)]
pub struct LinearImages<'a> {
    images: Vec<LinearImage<'a>>,
    decode: [f32; 256],
}

impl Default for LinearImages<'_> {
    fn default() -> Self {
        Self {
            images: Vec::new(),
            decode: core::array::from_fn(|i| Srgb::to_linear_srgb([i as f32 / 255.0; 3])[0]),
        }
    }
}

impl<'a> LinearImages<'a> {
    /// Registers straight sRGB RGBA8 bytes for padded bilinear sampling.
    pub fn insert_srgb(&mut self, width: u16, height: u16, bytes: &'a [u8]) -> Option<Image> {
        let (pixels, remainder) = bytes.as_chunks::<4>();
        if !remainder.is_empty() || !valid_size(width, height, pixels.len()) {
            return None;
        }
        let translucent = pixels.iter().any(|pixel| pixel[3] != 255);
        Some(self.insert(
            LinearImage {
                width,
                height,
                pixels: Pixels::Srgb(pixels),
            },
            ImageQuality::Medium,
            translucent,
        ))
    }

    /// Registers a device-aligned tile of premultiplied linear float pixels.
    pub fn insert_tile(&mut self, width: u16, height: u16, components: Vec<f32>) -> Option<Image> {
        let (pixels, remainder) = components.as_chunks::<4>();
        if !remainder.is_empty() || !valid_size(width, height, pixels.len()) {
            return None;
        }
        let translucent = pixels.iter().any(|pixel| pixel[3] != 1.0);
        Some(self.insert(
            LinearImage {
                width,
                height,
                pixels: Pixels::Premultiplied(components),
            },
            ImageQuality::Low,
            translucent,
        ))
    }

    fn insert(
        &mut self,
        image: LinearImage<'a>,
        quality: ImageQuality,
        translucent: bool,
    ) -> Image {
        let id = TextureId(self.images.len() as u64);
        self.images.push(image);
        Image {
            image: ImageSource::external_texture(id, TextureRegion::Full, translucent),
            sampler: ImageSampler::new().with_quality(quality),
        }
    }

    pub(crate) fn get(&self, id: TextureId) -> Option<(&LinearImage<'_>, &[f32; 256])> {
        let index = usize::try_from(id.0).ok()?;
        Some((self.images.get(index)?, &self.decode))
    }
}

fn valid_size(width: u16, height: u16, count: usize) -> bool {
    count != 0 && count == usize::from(width) * usize::from(height)
}

#[derive(Debug)]
pub(crate) struct LinearImage<'a> {
    pub(crate) width: u16,
    pub(crate) height: u16,
    pub(crate) pixels: Pixels<'a>,
}

#[derive(Debug)]
pub(crate) enum Pixels<'a> {
    Srgb(&'a [[u8; 4]]),
    Premultiplied(Vec<f32>),
}

pub(crate) fn pack_srgb<N: Numeric>(scratch: &[N], width: usize, region: &mut Region<'_>) {
    // Consecutive identical pixels are common in the interior of solid fills.
    let mut previous = [N::ZERO; 4];
    let mut encoded = [0; 4];
    for y in region.tile_rows() {
        let row = &mut region.row_mut(y)[..width * 4];
        for (x, out) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let offset = x * TILE_HEIGHT_COMPONENTS + usize::from(y) * 4;
            let pixel = scratch[offset..][..4].as_chunks::<4>().0[0];
            if pixel != previous {
                let [r, g, b, alpha] = pixel.map(Numeric::normalized);
                let a = byte(alpha);
                encoded = if a == 0 {
                    [0; 4]
                } else {
                    let [r, g, b] = Srgb::from_linear_srgb([r / alpha, g / alpha, b / alpha]);
                    [byte(r), byte(g), byte(b), a]
                };
                previous = pixel;
            }
            *out = encoded;
        }
    }
}

fn byte(channel: f32) -> u8 {
    (channel * 255.0 + 0.5) as u8
}
