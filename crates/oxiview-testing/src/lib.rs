//! Pixel snapshot test harness for native desktop platforms.
//!
//! Uses owned RGBA8 images without a renderer. Callers explicitly provide absolute
//! crate and workspace paths; tests only read baselines. Update with `cargo xtask snapshot --accept`.
//! See `docs/conventions/testing-snapshots.md` in the repository for usage.

use serde::Serialize;
use std::{
    fs,
    io::{BufReader, BufWriter},
    path::{Path, PathBuf},
};
use thiserror::Error;

/// Snapshot input, codec, file operation, or comparison errors.
#[derive(Debug, Error)]
pub enum SnapshotError {
    /// Input violates image, configuration, or path constraints.
    #[error("Invalid snapshot input: {0}")]
    InvalidInput(String),
    /// File read or write failed.
    #[error("Snapshot file operation failed: {0}")]
    Io(#[from] std::io::Error),
    /// PNG encoding or decoding failed; third-party types are not exposed to callers.
    #[error("PNG processing failed: {0}")]
    Png(String),
    /// Pending metadata encoding failed.
    #[error("Snapshot metadata encoding failed: {0}")]
    Metadata(String),
    /// Baseline missing or comparison failed; artifacts were written to this directory.
    #[error(
        "Pixel snapshot failed: {reason}; artifacts: {artifacts}; run cargo xtask snapshot --review"
    )]
    Mismatch {
        /// Failure reason.
        reason: String,
        /// Directory containing actual, diff, and pending artifacts.
        artifacts: PathBuf,
    },
}

/// Row-major RGBA8 image with non-premultiplied alpha.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RgbaImage {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl RgbaImage {
    /// Create a nonempty image; buffer length must equal width × height × 4.
    pub fn new(width: u32, height: u32, pixels: Vec<u8>) -> Result<Self, SnapshotError> {
        let size = u64::from(width)
            .checked_mul(u64::from(height))
            .and_then(|n| n.checked_mul(4));
        if width == 0 || height == 0 || size != Some(pixels.len() as u64) {
            return Err(SnapshotError::InvalidInput(
                "Invalid dimensions or RGBA8 buffer length".into(),
            ));
        }
        Ok(Self {
            width,
            height,
            pixels,
        })
    }

    /// Image width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }
    /// Image height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }
    /// Pixel buffer in RGBA order.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// Write an RGBA8 PNG; the parent directory must exist.
    pub fn write_png(&self, path: &Path) -> Result<(), SnapshotError> {
        let mut encoder = png::Encoder::new(
            BufWriter::new(fs::File::create(path)?),
            self.width,
            self.height,
        );
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|e| SnapshotError::Png(e.to_string()))?;
        writer
            .write_image_data(&self.pixels)
            .map_err(|e| SnapshotError::Png(e.to_string()))?;
        writer
            .finish()
            .map_err(|e| SnapshotError::Png(e.to_string()))
    }

    /// Read an RGBA8 PNG; reject other color types and bit depths explicitly.
    pub fn read_png(path: &Path) -> Result<Self, SnapshotError> {
        let decoder = png::Decoder::new(BufReader::new(fs::File::open(path)?));
        let mut reader = decoder
            .read_info()
            .map_err(|e| SnapshotError::Png(e.to_string()))?;
        let size = reader
            .output_buffer_size()
            .ok_or_else(|| SnapshotError::Png("Image too large".into()))?;
        let mut pixels = vec![0; size];
        let info = reader
            .next_frame(&mut pixels)
            .map_err(|e| SnapshotError::Png(e.to_string()))?;
        if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
            return Err(SnapshotError::Png("Baseline must be an RGBA8 PNG".into()));
        }
        pixels.truncate(info.buffer_size());
        Self::new(info.width, info.height, pixels)
    }

    fn pixel(&self, x: u32, y: u32) -> Option<&[u8]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let index = (u64::from(y) * u64::from(self.width) + u64::from(x)) * 4;
        let start = usize::try_from(index).ok()?;
        self.pixels.get(start..start + 4)
    }
}

/// Pixel comparison configuration, requiring exact equality by default.
#[derive(Clone, Copy, Debug, Default)]
pub struct CompareConfig {
    /// Inclusive absolute difference allowed per channel, including alpha.
    pub channel_tolerance: u8,
    /// Allowed ratio of pixels exceeding channel tolerance; must be finite and within 0–1.
    pub allowed_diff_ratio: f64,
}

/// Serializable pixel difference metrics.
#[derive(Clone, Debug, Serialize)]
pub struct Comparison {
    /// Passes when dimensions match and the differing-pixel ratio is within the limit.
    pub passed: bool,
    /// Whether both images have the same dimensions.
    pub dimensions_match: bool,
    /// Number of pixels, not channels, exceeding channel tolerance.
    pub diff_pixels: u64,
    /// Differing pixels divided by the pixel count of both images' bounding rectangle.
    pub diff_ratio: f64,
    /// Maximum absolute channel difference, including differences within tolerance; missing pixels count as 255.
    pub max_channel_diff: u8,
}

/// Compare images and generate a diff: fade matching pixels within tolerance; highlight differences in opaque magenta.
///
/// Different dimensions always fail; the diff spans both images' bounding rectangle, with areas absent from both transparent.
pub fn compare(
    actual: &RgbaImage,
    expected: &RgbaImage,
    config: &CompareConfig,
) -> Result<(Comparison, RgbaImage), SnapshotError> {
    if !config.allowed_diff_ratio.is_finite() || !(0.0..=1.0).contains(&config.allowed_diff_ratio) {
        return Err(SnapshotError::InvalidInput(
            "Allowed difference ratio must be between 0 and 1".into(),
        ));
    }
    let width = actual.width.max(expected.width);
    let height = actual.height.max(expected.height);
    let mut diff = Vec::new();
    let mut diff_pixels = 0;
    let mut max_channel_diff = 0;
    for y in 0..height {
        for x in 0..width {
            let a = actual.pixel(x, y);
            let b = expected.pixel(x, y);
            let delta = match (a, b) {
                (Some(a), Some(b)) => a
                    .iter()
                    .zip(b)
                    .map(|(a, b)| a.abs_diff(*b))
                    .max()
                    .unwrap_or(0),
                (None, None) => 0,
                _ => 255,
            };
            max_channel_diff = max_channel_diff.max(delta);
            if a.is_some() != b.is_some() || delta > config.channel_tolerance {
                diff_pixels += 1;
                diff.extend_from_slice(&[255, 0, 255, 255]);
            } else if let Some(pixel) = a {
                diff.extend_from_slice(&[
                    pixel[0] / 4 + 191,
                    pixel[1] / 4 + 191,
                    pixel[2] / 4 + 191,
                    255,
                ]);
            } else {
                diff.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
    }
    let dimensions_match = actual.width == expected.width && actual.height == expected.height;
    let diff_ratio = diff_pixels as f64 / (u64::from(width) * u64::from(height)) as f64;
    let comparison = Comparison {
        passed: dimensions_match && diff_ratio <= config.allowed_diff_ratio,
        dimensions_match,
        diff_pixels,
        diff_ratio,
        max_channel_diff,
    };
    Ok((comparison, RgbaImage::new(width, height, diff)?))
}

/// Snapshot assertions with fixed caller and artifact paths, independent of the process working directory.
pub struct SnapshotHarness {
    workspace_root: PathBuf,
    crate_root: PathBuf,
    crate_name: String,
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_".contains(&c))
}

#[derive(Serialize)]
struct Pending<'a> {
    name: &'a str,
    crate_name: &'a str,
    crate_root: &'a Path,
    reason: &'a str,
    comparison: Option<Comparison>,
}

impl SnapshotHarness {
    /// Create a harness using existing absolute directories. The crate must be inside the workspace.
    /// Crate and snapshot names allow only ASCII letters, digits, hyphens, and underscores.
    pub fn new(
        workspace_root: &Path,
        crate_root: &Path,
        crate_name: &str,
    ) -> Result<Self, SnapshotError> {
        if !workspace_root.is_absolute() || !crate_root.is_absolute() || !valid_name(crate_name) {
            return Err(SnapshotError::InvalidInput(
                "Absolute roots and a valid crate name are required".into(),
            ));
        }
        let workspace_root = workspace_root.canonicalize()?;
        let crate_root = crate_root.canonicalize()?;
        if !crate_root.starts_with(&workspace_root) {
            return Err(SnapshotError::InvalidInput(
                "Crate is outside the workspace".into(),
            ));
        }
        Ok(Self {
            workspace_root,
            crate_root,
            crate_name: crate_name.into(),
        })
    }

    /// Assert a snapshot; failures write actual.png, diff.png, and pending.json, but never the baseline.
    /// On success, remove stale pending records to prevent accepting obsolete failure images.
    pub fn assert_snapshot(
        &self,
        name: &str,
        image: &RgbaImage,
        config: &CompareConfig,
    ) -> Result<(), SnapshotError> {
        if !valid_name(name) || name == "all" {
            return Err(SnapshotError::InvalidInput(
                "Invalid snapshot name; all is reserved".into(),
            ));
        }
        // Validate comparison settings first, even when the baseline is missing.
        if !config.allowed_diff_ratio.is_finite()
            || !(0.0..=1.0).contains(&config.allowed_diff_ratio)
        {
            return Err(SnapshotError::InvalidInput(
                "Allowed difference ratio must be between 0 and 1".into(),
            ));
        }
        let baseline = self
            .crate_root
            .join("tests/snapshots")
            .join(format!("{name}.png"));
        let artifacts = self
            .workspace_root
            .join("target/xtask/snapshots")
            .join(&self.crate_name)
            .join(name);
        let pending_path = artifacts.join("pending.json");
        let (reason, comparison, diff) = match RgbaImage::read_png(&baseline) {
            Ok(expected) => {
                let (result, diff) = compare(image, &expected, config)?;
                if result.passed {
                    match fs::remove_file(&pending_path) {
                        Ok(()) => {}
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                        Err(e) => return Err(e.into()),
                    }
                    return Ok(());
                }
                (
                    if result.dimensions_match {
                        "Tolerance exceeded"
                    } else {
                        "Dimensions differ"
                    },
                    Some(result),
                    diff,
                )
            }
            Err(SnapshotError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
                let pixels = image
                    .pixels
                    .chunks_exact(4)
                    .flat_map(|_| [255, 0, 255, 255])
                    .collect();
                (
                    "Missing",
                    None,
                    RgbaImage::new(image.width, image.height, pixels)?,
                )
            }
            Err(e) => return Err(e),
        };
        fs::create_dir_all(&artifacts)?;
        // Remove the old record first; publish new pending metadata only after both images are written.
        match fs::remove_file(&pending_path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        image.write_png(&artifacts.join("actual.png"))?;
        diff.write_png(&artifacts.join("diff.png"))?;
        let relative_root = self
            .crate_root
            .strip_prefix(&self.workspace_root)
            .map_err(|e| SnapshotError::InvalidInput(e.to_string()))?;
        let pending = Pending {
            name,
            crate_name: &self.crate_name,
            crate_root: relative_root,
            reason,
            comparison,
        };
        fs::write(
            &pending_path,
            serde_json::to_vec_pretty(&pending)
                .map_err(|e| SnapshotError::Metadata(e.to_string()))?,
        )?;
        Err(SnapshotError::Mismatch {
            reason: reason.into(),
            artifacts,
        })
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
