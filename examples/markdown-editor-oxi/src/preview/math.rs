use oxiview_base::{
    BezierPath, Color, CornerRadii, Glyph, GlyphFont, GlyphKind, PathCommand, Point, Rect, Size,
};
use oxiview_scene::{Brush, FillRule, Primitive};
use ratex_layout::{LayoutOptions, layout, to_display_list, validate_layout};
use ratex_types::{
    display_item::DisplayItem, math_style::MathStyle, path_command::PathCommand as MathCommand,
};
use resvg::tiny_skia::{PathBuilder, PathSegment, Stroke};
use skrifa::{FontRef, MetadataProvider};
use std::{
    borrow::Cow,
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
};

use super::{Graphic, GraphicContent, GraphicError, ink};

pub(super) const MAX_SOURCE_BYTES: usize = 8 * 1024;

struct Face {
    metrics: FontRef<'static>,
    drawing: GlyphFont,
}

pub(super) fn render(source: &str, display: bool) -> Result<Graphic, GraphicError> {
    if source.len() > MAX_SOURCE_BYTES {
        return Err(GraphicError::new(
            "expression exceeds the preview size limit",
        ));
    }
    let parsed =
        ratex_parser::parse(source.trim()).map_err(|error| GraphicError::new(error.to_string()))?;
    let [r, g, b, a] = ink().to_srgb();
    let options = LayoutOptions {
        style: if display {
            MathStyle::Display
        } else {
            MathStyle::Text
        },
        color: ratex_types::color::Color::new(r, g, b, a),
        ..LayoutOptions::default()
    };
    let formula = layout(&parsed, &options);
    validate_layout(&formula).map_err(GraphicError::new)?;
    let list = to_display_list(&formula);
    let size = Size::new(list.width.max(0.0), list.total_height().max(0.0));
    if !geometry(&[list.width, list.height, list.depth]) {
        return Err(GraphicError::new(
            "expression geometry exceeds the preview size limit",
        ));
    }

    static FACES: OnceLock<Mutex<HashMap<String, Face>>> = OnceLock::new();
    let mut faces = FACES
        .get_or_init(Mutex::default)
        .lock()
        .map_err(|_| GraphicError::new("math font cache is unavailable"))?;
    let mut primitives = Vec::with_capacity(list.items.len());
    let variations: Arc<[i16]> = Arc::new([]);
    for item in list.items {
        match item {
            DisplayItem::GlyphPath {
                x,
                y,
                scale,
                font,
                char_code,
                color: value,
            } => {
                if !geometry(&[x, y, scale]) || scale <= 0.0 {
                    return Err(GraphicError::new("invalid glyph geometry in formula"));
                }
                let face = face(&mut faces, font)?;
                let id = face
                    .metrics
                    .charmap()
                    .map(char_code)
                    .and_then(|id| u16::try_from(id.to_u32()).ok())
                    .ok_or_else(|| {
                        GraphicError::new(format!("math font has no glyph for U+{char_code:04X}"))
                    })?;
                primitives.push(Primitive::Glyph(Glyph {
                    font: face.drawing.clone(),
                    face_index: 0,
                    glyph_id: id,
                    size: scale as f32,
                    origin: Point::new(x, y),
                    color: color(value),
                    variations: variations.clone(),
                    kind: GlyphKind::Outline,
                }));
            }
            DisplayItem::Line {
                x,
                y,
                width,
                thickness,
                dashed,
                color: value,
            } => {
                if !geometry(&[x, y, width, thickness]) {
                    return Err(GraphicError::new("invalid rule geometry in formula"));
                }
                let bounds = Rect::new(x, y - thickness * 0.5, x + width, y + thickness * 0.5);
                let color = color(value);
                if dashed && width > 0.0 && thickness > 0.0 {
                    let dash = (4.0 * thickness).max(width / 4096.0);
                    let count = (width / (2.0 * dash)).ceil() as usize;
                    for index in 0..count {
                        let left = x + index as f64 * 2.0 * dash;
                        let [_, top, right, bottom] = bounds.edges();
                        fill(
                            &mut primitives,
                            Rect::new(left, top, (left + dash).min(right), bottom),
                            color,
                        );
                    }
                } else {
                    fill(&mut primitives, bounds, color);
                }
            }
            DisplayItem::Rect {
                x,
                y,
                width,
                height,
                color: value,
            } => {
                if !geometry(&[x, y, width, height]) {
                    return Err(GraphicError::new("invalid rectangle geometry in formula"));
                }
                fill(
                    &mut primitives,
                    Rect::new(x, y, x + width, y + height),
                    color(value),
                );
            }
            DisplayItem::Path {
                x,
                y,
                commands,
                fill,
                color: value,
            } => {
                if let Some(path) = path(x, y, &commands, fill)? {
                    primitives.push(Primitive::FillPath(
                        path,
                        FillRule::NonZero,
                        Brush::Solid(color(value)),
                    ));
                }
            }
        }
    }
    Ok(Graphic {
        size,
        baseline: list.height,
        content: GraphicContent::Math(primitives),
    })
}

fn face(faces: &mut HashMap<String, Face>, name: String) -> Result<&Face, GraphicError> {
    use std::collections::hash_map::Entry;
    match faces.entry(name) {
        Entry::Occupied(entry) => Ok(entry.into_mut()),
        Entry::Vacant(entry) => {
            let filename = format!("KaTeX_{}.ttf", entry.key());
            let Some(Cow::Borrowed(bytes)) = ratex_katex_fonts::ttf_bytes(&filename) else {
                return Err(GraphicError::new(format!(
                    "unsupported math font {}",
                    entry.key()
                )));
            };
            let metrics =
                FontRef::new(bytes).map_err(|error| GraphicError::new(error.to_string()))?;
            let drawing = GlyphFont::from_shared(Arc::new(bytes));
            Ok(entry.insert(Face { metrics, drawing }))
        }
    }
}

fn color(value: ratex_types::color::Color) -> Color {
    Color::from_srgb([value.r, value.g, value.b, value.a])
}

fn geometry(values: &[f64]) -> bool {
    values.iter().all(|value| value.abs() <= 4096.0)
}

fn fill(output: &mut Vec<Primitive>, bounds: Rect, color: Color) {
    if bounds.size().width() > 0.0 && bounds.size().height() > 0.0 {
        output.push(Primitive::Fill(
            bounds,
            CornerRadii::default(),
            Brush::Solid(color),
        ));
    }
}

fn path(
    x: f64,
    y: f64,
    commands: &[MathCommand],
    filled: bool,
) -> Result<Option<BezierPath>, GraphicError> {
    let mut path = PathBuilder::new();
    for &command in commands {
        match command {
            MathCommand::MoveTo { x: px, y: py } => path.move_to((x + px) as f32, (y + py) as f32),
            MathCommand::LineTo { x: px, y: py } => path.line_to((x + px) as f32, (y + py) as f32),
            MathCommand::QuadTo {
                x1,
                y1,
                x: px,
                y: py,
            } => path.quad_to(
                (x + x1) as f32,
                (y + y1) as f32,
                (x + px) as f32,
                (y + py) as f32,
            ),
            MathCommand::CubicTo {
                x1,
                y1,
                x2,
                y2,
                x: px,
                y: py,
            } => path.cubic_to(
                (x + x1) as f32,
                (y + y1) as f32,
                (x + x2) as f32,
                (y + y2) as f32,
                (x + px) as f32,
                (y + py) as f32,
            ),
            MathCommand::Close => path.close(),
        }
    }
    let Some(path) = path.finish() else {
        return Ok(None);
    };
    let outline = if filled {
        path
    } else {
        let stroke = Stroke {
            width: 0.075,
            ..Stroke::default()
        };
        let Some(outline) = path.stroke(&stroke, 20.0) else {
            return Err(GraphicError::new("invalid stroked path in formula"));
        };
        outline
    };
    let point = |point: resvg::tiny_skia::Point| Point::new(f64::from(point.x), f64::from(point.y));
    let commands = outline
        .segments()
        .map(|segment| match segment {
            PathSegment::MoveTo(p) => PathCommand::MoveTo(point(p)),
            PathSegment::LineTo(p) => PathCommand::LineTo(point(p)),
            PathSegment::QuadTo(a, b) => PathCommand::QuadTo(point(a), point(b)),
            PathSegment::CubicTo(a, b, c) => PathCommand::CubicTo(point(a), point(b), point(c)),
            PathSegment::Close => PathCommand::Close,
        })
        .collect();
    BezierPath::new(commands)
        .map(Some)
        .map_err(|error| GraphicError::new(error.to_string()))
}

#[cfg(test)]
pub(super) fn is_fixture_font(font: &GlyphFont) -> bool {
    [
        "KaTeX_Math-Italic.ttf",
        "KaTeX_Main-Regular.ttf",
        "KaTeX_Size1-Regular.ttf",
        "KaTeX_Size2-Regular.ttf",
        "KaTeX_Size3-Regular.ttf",
        "KaTeX_Size4-Regular.ttf",
    ]
    .iter()
    .any(|name| {
        ratex_katex_fonts::ttf_bytes(name).is_some_and(|bytes| bytes.as_ref() == font.bytes())
    })
}
