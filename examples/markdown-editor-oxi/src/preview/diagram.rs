use fermaid::{Limits, MetricsMeasurer, Palette, RenderOptions, Style, TextMeasurer};
use oxiview_base::{PixelSize, RgbaImage, Size};
use oxiview_text::{RichText, TextSpan, TextSpanStyle, TextStyle, TextSystem};
use resvg::{tiny_skia, usvg};
use std::{
    cell::RefCell,
    sync::{Arc, OnceLock},
};

use super::{Graphic, GraphicError};

const FONTS: &[&[u8]] = &[
    include_bytes!("../../../../crates/oxiview-text/tests/fonts/NotoSans.ttf"),
    include_bytes!("../../../../crates/oxiview-text/tests/fonts/NotoSansArabic.ttf"),
];
const FONT_FAMILY: &str = "Noto Sans";
const MAX_SOURCE_BYTES: usize = 24 * 1024;

fn font_database() -> Arc<usvg::fontdb::Database> {
    static DATABASE: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    DATABASE
        .get_or_init(|| {
            let mut database = usvg::fontdb::Database::new();
            for bytes in FONTS {
                database.load_font_data(bytes.to_vec());
            }
            database.set_sans_serif_family(FONT_FAMILY);
            database.load_system_fonts();
            Arc::new(database)
        })
        .clone()
}

thread_local! {
    static LABEL_TEXT: RefCell<TextSystem> = RefCell::new({
        let mut text = TextSystem::new();
        for bytes in FONTS {
            let _ = text.register_font(bytes.to_vec());
        }
        text
    });
}

struct LabelMeasurer;

impl TextMeasurer for LabelMeasurer {
    fn width(&self, value: &str, size: f64, marks: Style) -> f64 {
        if value.is_empty() {
            return 0.0;
        }
        let family = if marks.has(Style::MONO) {
            "monospace"
        } else {
            FONT_FAMILY
        };
        let style = TextStyle {
            font_size: size,
            family: Some(family.into()),
            weight: if marks.has(Style::BOLD) { 700 } else { 400 },
            ..TextStyle::default()
        };
        let value = value.replace(' ', "\u{a0}");
        let span = TextSpan {
            range: 0..value.len(),
            style: TextSpanStyle {
                italic: marks.has(Style::ITALIC),
                ..TextSpanStyle::default()
            },
        };
        let measured = RichText::new(value.as_str(), vec![span]).and_then(|rich| {
            LABEL_TEXT.with_borrow_mut(|text| text.layout_rich(&rich, &style, None))
        });
        measured.map_or_else(
            |_| MetricsMeasurer.width(&value, size, marks),
            |layout| layout.size().width(),
        )
    }
}

pub(super) fn render(source: &str) -> Result<Graphic, GraphicError> {
    if source.len() > MAX_SOURCE_BYTES || source.lines().count() > 512 {
        return Err(GraphicError::new(
            "diagram source exceeds the preview size limit",
        ));
    }

    let mut palette = Palette::light();
    palette.fg = "#253b39".into();
    palette.line = "#496760".into();
    palette.accent = "#346c62".into();
    palette.surface = "#edf6f1".into();
    palette.surface_alt = "#f5f8f6".into();
    palette.border = "#8aa39c".into();
    let options = RenderOptions {
        font_family: Some(format!("{FONT_FAMILY}, sans-serif")),
        font_family_mono: Some("monospace".into()),
        links: false,
        images: false,
        responsive: false,
        palette: Some(palette),
        measurer: Some(Box::new(LabelMeasurer)),
        limits: Limits {
            source: MAX_SOURCE_BYTES,
            ..Limits::default()
        },
        ..RenderOptions::default()
    };
    let diagram = fermaid::render(source, &options).map_err(|error| GraphicError {
        location: source_position(source, &error),
        message: error.message,
    })?;

    let options = usvg::Options {
        font_family: FONT_FAMILY.into(),
        fontdb: font_database(),
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_str(&diagram.svg, &options)
        .map_err(|error| GraphicError::new(error.to_string()))?;
    if tree.root().children().is_empty() {
        return Err(GraphicError::new("diagram produced no drawing"));
    }
    let size = tree.size();
    let width = size.width();
    let height = size.height();
    // Keep enough samples for 2× displays and bound allocations for large graphs.
    let scale = 2.0_f32
        .min(4096.0 / width.max(height))
        .min((4_000_000.0 / (width * height)).sqrt());
    let pixel_width = (width * scale).ceil() as u32;
    let pixel_height = (height * scale).ceil() as u32;
    let mut pixmap = tiny_skia::Pixmap::new(pixel_width, pixel_height)
        .ok_or_else(|| GraphicError::new("unable to allocate diagram image"))?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let image = RgbaImage::new(
        PixelSize {
            width: pixel_width,
            height: pixel_height,
        },
        pixmap.take_demultiplied(),
    )
    .map_err(|error| GraphicError::new(error.to_string()))?;
    Ok(Graphic::raster(
        image,
        Size::new(f64::from(width), f64::from(height)),
    ))
}

fn source_position(source: &str, error: &fermaid::Error) -> Option<(usize, usize)> {
    let line = (error.line as usize).checked_sub(1)?;
    let column = error.column as usize;
    let lines: Vec<_> = source.lines().collect();
    let snippet = error.snippet.lines().next()?;
    if snippet.is_empty() && line >= lines.len() {
        // Fermaid appends a newline before parsing. Move EOF diagnostics back
        // to the end of the last written statement.
        let (index, last) = lines
            .iter()
            .enumerate()
            .rfind(|(_, line)| !line.trim().is_empty())?;
        return Some((index + 1, last.trim_end().chars().count() + 1));
    }

    // The diagram parser can remove front matter or comments before lexing.
    // Its snippet lets us locate the reported line in the original fence.
    let (index, raw) = if lines
        .get(line)
        .is_some_and(|line| line.trim() == snippet.trim())
    {
        (line, lines[line])
    } else {
        let mut matches = lines
            .iter()
            .copied()
            .enumerate()
            .filter(|(_, line)| line.trim() == snippet.trim());
        let unique = matches.next()?;
        if matches.next().is_some() {
            return None;
        }
        unique
    };
    let indentation = |text: &str| text.chars().take_while(|ch| ch.is_whitespace()).count();
    let column = (column + indentation(raw))
        .saturating_sub(indentation(snippet))
        .max(1);
    if column > raw.chars().count()
        && let Some(next) = lines.get(index + 1)
        && next.trim_start().starts_with("-->")
    {
        // The flowchart lexer can include the preceding newline in an arrow
        // token. Point at the arrow when its endpoint is missing.
        return Some((index + 2, indentation(next) + 1));
    }
    Some((index + 1, column))
}
