//! 原生桌面平台的像素快照测试 harness。
//!
//! 使用自有 RGBA8 图像，不依赖渲染器。调用方显式提供 crate 与 workspace
//! 的绝对路径；默认测试只读取基准图，更新需运行 `cargo xtask snapshot --accept`。
//! 详细用法见仓库 `docs/conventions/testing-snapshots.md`。

use serde::Serialize;
use std::{
    fs,
    io::{BufReader, BufWriter},
    path::{Path, PathBuf},
};
use thiserror::Error;

/// 快照的输入、编解码、文件操作或比较错误。
#[derive(Debug, Error)]
pub enum SnapshotError {
    /// 输入不满足图像、配置或路径约束。
    #[error("快照输入无效：{0}")]
    InvalidInput(String),
    /// 读写文件失败。
    #[error("快照文件操作失败：{0}")]
    Io(#[from] std::io::Error),
    /// PNG 编解码失败；不向调用方暴露第三方类型。
    #[error("PNG 处理失败：{0}")]
    Png(String),
    /// pending 元数据编码失败。
    #[error("快照元数据编码失败：{0}")]
    Metadata(String),
    /// 基准缺失或比较失败，产物已写入此目录。
    #[error("像素快照失败：{reason}；产物：{artifacts}；运行 cargo xtask snapshot --review")]
    Mismatch {
        /// 失败原因。
        reason: String,
        /// actual、diff 与 pending 所在目录。
        artifacts: PathBuf,
    },
}

/// 按行排列、非预乘 alpha 的 RGBA8 图像。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RgbaImage {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl RgbaImage {
    /// 创建非空图像；缓冲长度必须恰好为宽 × 高 × 4。
    pub fn new(width: u32, height: u32, pixels: Vec<u8>) -> Result<Self, SnapshotError> {
        let size = u64::from(width)
            .checked_mul(u64::from(height))
            .and_then(|n| n.checked_mul(4));
        if width == 0 || height == 0 || size != Some(pixels.len() as u64) {
            return Err(SnapshotError::InvalidInput(
                "尺寸或 RGBA8 缓冲长度不合法".into(),
            ));
        }
        Ok(Self {
            width,
            height,
            pixels,
        })
    }

    /// 图像宽度（像素）。
    pub fn width(&self) -> u32 {
        self.width
    }
    /// 图像高度（像素）。
    pub fn height(&self) -> u32 {
        self.height
    }
    /// 按 RGBA 顺序排列的像素缓冲。
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// 写入 RGBA8 PNG；父目录须已存在。
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

    /// 读取 RGBA8 PNG；其他色彩类型及位深明确报错。
    pub fn read_png(path: &Path) -> Result<Self, SnapshotError> {
        let decoder = png::Decoder::new(BufReader::new(fs::File::open(path)?));
        let mut reader = decoder
            .read_info()
            .map_err(|e| SnapshotError::Png(e.to_string()))?;
        let size = reader
            .output_buffer_size()
            .ok_or_else(|| SnapshotError::Png("图像过大".into()))?;
        let mut pixels = vec![0; size];
        let info = reader
            .next_frame(&mut pixels)
            .map_err(|e| SnapshotError::Png(e.to_string()))?;
        if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
            return Err(SnapshotError::Png("基准必须是 RGBA8 PNG".into()));
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

/// 像素比较配置，默认要求完全一致。
#[derive(Clone, Copy, Debug, Default)]
pub struct CompareConfig {
    /// 每个通道允许的绝对差（包含边界），包含 alpha 通道。
    pub channel_tolerance: u8,
    /// 允许超出通道容差的像素比例，必须是 0–1 范围内的有限数。
    pub allowed_diff_ratio: f64,
}

/// 可序列化的像素差异指标。
#[derive(Clone, Debug, Serialize)]
pub struct Comparison {
    /// 尺寸相同且差异比例不超限时通过。
    pub passed: bool,
    /// 两张图的尺寸是否一致。
    pub dimensions_match: bool,
    /// 超出通道容差的像素数，非通道数。
    pub diff_pixels: u64,
    /// 差异像素数除以两张图的包围矩形像素数。
    pub diff_ratio: f64,
    /// 所有像素的最大通道绝对差（含容差内差异）；缺失像素视为 255。
    pub max_channel_diff: u8,
}

/// 比较图像并生成差异图：一致（含容差内）像素淡化，差异像素用不透明品红高亮。
///
/// 尺寸不同始终失败；差异图覆盖两图的包围矩形，不存在于任一图的区域为透明。
pub fn compare(
    actual: &RgbaImage,
    expected: &RgbaImage,
    config: &CompareConfig,
) -> Result<(Comparison, RgbaImage), SnapshotError> {
    if !config.allowed_diff_ratio.is_finite() || !(0.0..=1.0).contains(&config.allowed_diff_ratio) {
        return Err(SnapshotError::InvalidInput(
            "允许差异比例必须在 0–1 之间".into(),
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

/// 固定调用方与产物路径的快照断言入口，不读取进程当前目录。
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
    /// 使用已存在的绝对目录创建入口。crate 必须位于 workspace 内。
    /// crate 名及快照名只允许 ASCII 字母、数字、连字符、下划线。
    pub fn new(
        workspace_root: &Path,
        crate_root: &Path,
        crate_name: &str,
    ) -> Result<Self, SnapshotError> {
        if !workspace_root.is_absolute() || !crate_root.is_absolute() || !valid_name(crate_name) {
            return Err(SnapshotError::InvalidInput(
                "需要绝对根目录及合法 crate 名".into(),
            ));
        }
        let workspace_root = workspace_root.canonicalize()?;
        let crate_root = crate_root.canonicalize()?;
        if !crate_root.starts_with(&workspace_root) {
            return Err(SnapshotError::InvalidInput(
                "crate 不在 workspace 内".into(),
            ));
        }
        Ok(Self {
            workspace_root,
            crate_root,
            crate_name: crate_name.into(),
        })
    }

    /// 断言快照；失败写入 actual.png、diff.png、pending.json，但永远不写入基准。
    /// 成功时删除该快照的旧 pending 记录，防止接受已经过期的失败图像。
    pub fn assert_snapshot(
        &self,
        name: &str,
        image: &RgbaImage,
        config: &CompareConfig,
    ) -> Result<(), SnapshotError> {
        if !valid_name(name) || name == "all" {
            return Err(SnapshotError::InvalidInput(
                "快照名不合法，all 为保留字".into(),
            ));
        }
        // 即使基准缺失，也先验证比较配置。
        if !config.allowed_diff_ratio.is_finite()
            || !(0.0..=1.0).contains(&config.allowed_diff_ratio)
        {
            return Err(SnapshotError::InvalidInput(
                "允许差异比例必须在 0–1 之间".into(),
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
                        "超出容差"
                    } else {
                        "尺寸不一致"
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
                    "缺失",
                    None,
                    RgbaImage::new(image.width, image.height, pixels)?,
                )
            }
            Err(e) => return Err(e),
        };
        fs::create_dir_all(&artifacts)?;
        // 先撤销旧记录，只有两张图均写入成功后才发布新的 pending。
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
