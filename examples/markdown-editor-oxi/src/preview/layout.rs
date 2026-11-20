mod inline;

use oxiview_base::{Affine, BezierPath, Color, CornerRadii, PathCommand, Point, Rect};
use oxiview_scene::{Brush, Primitive};
use oxiview_text::{
    Error, GlyphDraw, TextAlign, TextDecorationKind, TextLayout, TextStyle, TextSystem,
};
use std::sync::Arc;

use super::{
    Block, BlockKind, CellAlignment, Diagnostic, Graphic, GraphicContent, ListItem,
    PreviewDocument, Table, ink,
};

const BODY_SIZE: f64 = 16.0;
const PADDING_TOP: f64 = 24.0;
const BLOCK_GAP: f64 = 15.0;

fn border_color() -> Color {
    Color::from_rgba8(211, 220, 220, 255)
}

fn surface_color() -> Color {
    Color::from_rgba8(245, 248, 247, 255)
}

fn muted_color() -> Color {
    Color::from_rgba8(79, 103, 100, 255)
}

fn error_color() -> Color {
    Color::from_rgba8(153, 43, 43, 255)
}

fn body_style() -> TextStyle {
    TextStyle {
        font_size: BODY_SIZE,
        color: ink(),
        family: Some(crate::view::SANS_FAMILIES.into()),
        line_height: 1.45,
        ..TextStyle::default()
    }
}

enum Paint {
    Text(TextLayout),
    Graphic(Arc<Graphic>, f64),
    Primitive(Primitive),
}

struct Item {
    bounds: Rect,
    paint: Paint,
}

/// Native preview measurements for one document revision and viewport width.
pub struct PreviewLayout {
    height: f64,
    items: Vec<Item>,
}

impl PreviewDocument {
    /// Measures the document at the viewport's full logical width, including padding.
    pub fn layout(&self, text: &mut TextSystem, width: f64) -> Result<PreviewLayout, Error> {
        if !width.is_finite() || width < 0.0 {
            return Err(Error::InvalidInput);
        }
        if width == 0.0 || self.blocks.is_empty() {
            return Ok(PreviewLayout {
                height: 0.0,
                items: Vec::new(),
            });
        }
        let inset = (width * 0.09).min(30.0);
        let content = (width - 2.0 * inset).clamp(1.0, 760.0);
        let mut measure = Measure {
            items: Vec::new(),
            text,
            diagnostics: &self.diagnostics,
        };
        let bottom = measure.blocks(&self.blocks, (width - content) * 0.5, PADDING_TOP, content)?;
        Ok(PreviewLayout {
            height: bottom + PADDING_TOP,
            items: measure.items,
        })
    }
}

impl PreviewLayout {
    /// Full height of the document in logical pixels.
    pub fn height(&self) -> f64 {
        self.height
    }

    /// Generates viewport-relative logical draws, culling invisible content.
    ///
    /// Attach the primitives below a Scene clip using the supplied viewport.
    /// Scale is the current logical-to-device scale and selects native glyph
    /// rasterization without changing placement or scroll measurements.
    pub fn primitives(
        &self,
        text: &mut TextSystem,
        viewport: Rect,
        scroll_y: f64,
        scale: f64,
    ) -> Result<Vec<Primitive>, Error> {
        self.primitives_in_scene(text, viewport, scroll_y, scale, Point::default())
    }

    /// Renders local draws that the scene will translate by the pane's window origin.
    pub(crate) fn primitives_in_scene(
        &self,
        text: &mut TextSystem,
        viewport: Rect,
        scroll_y: f64,
        scale: f64,
        window_origin: Point,
    ) -> Result<Vec<Primitive>, Error> {
        let device_x = window_origin.x() * scale;
        let device_y = window_origin.y() * scale;
        if !viewport.edges().into_iter().all(f64::is_finite)
            || ![scroll_y, scale, device_x, device_y]
                .into_iter()
                .all(f64::is_finite)
            || scale <= 0.0
        {
            return Err(Error::InvalidInput);
        }
        let size = viewport.size();
        if size.width() <= 0.0 || size.height() <= 0.0 {
            return Ok(Vec::new());
        }
        let scroll_y = scroll_y.clamp(0.0, (self.height - size.height()).max(0.0));
        let offset = Point::new(viewport.origin().x(), viewport.origin().y() - scroll_y);
        let device = Affine::new([scale, 0.0, 0.0, scale, device_x, device_y]);
        let bottom = scroll_y + size.height();
        let mut primitives = Vec::new();
        for item in &self.items {
            let [x0, y0, x1, y1] = item.bounds.edges();
            // Leave a small allowance for italic ink and font accent overhangs.
            if y1 + 3.0 < scroll_y || y0 - 3.0 > bottom || x1 + 3.0 < 0.0 || x0 - 3.0 > size.width()
            {
                continue;
            }
            let origin = Point::new(x0 + offset.x(), y0 + offset.y());
            match &item.paint {
                Paint::Text(layout) => paint_text(text, layout, origin, device, &mut primitives)?,
                Paint::Graphic(graphic, scale) => match &graphic.content {
                    GraphicContent::Raster(image) => primitives.push(Primitive::Image(
                        map_rect(item.bounds, offset, 1.0),
                        image.clone(),
                    )),
                    GraphicContent::Math(math) => {
                        for primitive in math {
                            primitives.push(map_primitive(primitive, origin, *scale)?);
                        }
                    }
                },
                Paint::Primitive(primitive) => {
                    primitives.push(map_primitive(primitive, offset, 1.0)?)
                }
            }
        }
        Ok(primitives)
    }
}

struct Measure<'a> {
    items: Vec<Item>,
    text: &'a mut TextSystem,
    diagnostics: &'a [Diagnostic],
}

impl Measure<'_> {
    fn blocks(&mut self, blocks: &[Block], x: f64, y: f64, width: f64) -> Result<f64, Error> {
        let mut bottom = y;
        for (index, block) in blocks.iter().enumerate() {
            if index != 0 {
                bottom += BLOCK_GAP;
            }
            bottom = self.block(block, x, bottom, width)?;
        }
        Ok(bottom)
    }

    fn block(&mut self, block: &Block, x: f64, y: f64, width: f64) -> Result<f64, Error> {
        match &block.kind {
            BlockKind::Paragraph(inlines) => {
                let paragraph =
                    inline::paragraph(self.text, inlines, self.diagnostics, &body_style(), width)?;
                let height = paragraph.height;
                self.paragraph(paragraph, x, y);
                Ok(y + height)
            }
            BlockKind::Heading(level, inlines) => {
                let style = TextStyle {
                    font_size: match level {
                        1 => 34.0,
                        2 => 26.0,
                        3 => 22.0,
                        4 => 19.0,
                        _ => 17.0,
                    },
                    line_height: 1.2,
                    weight: 700,
                    ..body_style()
                };
                let top = y + if *level == 1 { 0.0 } else { 5.0 };
                let paragraph =
                    inline::paragraph(self.text, inlines, self.diagnostics, &style, width)?;
                let bottom = top + paragraph.height;
                self.paragraph(paragraph, x, top);
                Ok(bottom)
            }
            BlockKind::Code(code) => self.code(code, x, y, width),
            BlockKind::Quote(children) => self.quote(children, x, y, width),
            BlockKind::List(start, items) => self.list(*start, items, x, y, width),
            BlockKind::Table(table) => self.table(table, x, y, width),
            BlockKind::Rule => {
                self.rectangle(
                    Rect::new(x, y + 4.0, x + width, y + 5.0),
                    0.0,
                    border_color(),
                );
                Ok(y + 9.0)
            }
            BlockKind::Graphic { graphic, math } => {
                let em = if *math { 21.0 } else { 1.0 };
                let piece = inline::Piece::graphic(graphic.clone(), em, width);
                let left = x + (width - piece.size.width()).max(0.0) * 0.5;
                let bottom = y + piece.size.height() + 3.0;
                self.piece(piece, Point::new(left, y + 3.0));
                Ok(bottom + 3.0)
            }
            BlockKind::Diagnostic(index) => self.diagnostic(*index, x, y, width),
        }
    }

    fn code(&mut self, code: &str, x: f64, y: f64, width: f64) -> Result<f64, Error> {
        let start = self.items.len();
        let style = TextStyle {
            font_size: 14.0,
            family: Some("monospace".into()),
            line_height: 1.4,
            ..body_style()
        };
        let inset = (width * 0.12).min(14.0);
        let mut bottom = y + 10.0;
        for line in code.split('\n') {
            let line = line.trim_end_matches('\r');
            let height = self.text(
                line,
                &style,
                x + inset,
                bottom,
                (width - inset * 2.0).max(1.0),
            )?;
            bottom += height.max(style.font_size * style.line_height);
        }
        bottom += 10.0;
        self.items.insert(
            start,
            rectangle(Rect::new(x, y, x + width, bottom), 6.0, surface_color()),
        );
        Ok(bottom)
    }

    fn quote(&mut self, children: &[Block], x: f64, y: f64, width: f64) -> Result<f64, Error> {
        let start = self.items.len();
        let inset = (width * 0.15).min(17.0);
        let bottom =
            self.blocks(children, x + inset, y + 8.0, (width - inset * 2.0).max(1.0))? + 8.0;
        self.items.insert(
            start,
            rectangle(Rect::new(x, y, x + width, bottom), 4.0, surface_color()),
        );
        self.rectangle(
            Rect::new(x, y, x + 3.0_f64.min(width), bottom),
            1.5,
            muted_color(),
        );
        Ok(bottom)
    }

    fn list(
        &mut self,
        start: Option<u64>,
        items: &[ListItem],
        x: f64,
        y: f64,
        width: f64,
    ) -> Result<f64, Error> {
        let style = TextStyle {
            color: muted_color(),
            ..body_style()
        };
        let ordinal = start.map_or(0, |start| start.saturating_add(items.len() as u64));
        let label_width = if start.is_some() {
            self.text
                .layout(&format!("{ordinal}."), &style, None)?
                .size()
                .width()
                + 10.0
        } else {
            27.0
        };
        let indent = label_width.max(27.0).min(width * 0.45);
        let mut bottom = y;
        for (index, item) in items.iter().enumerate() {
            if index != 0 {
                bottom += 5.0;
            }
            if let Some(checked) = item.task {
                self.checkbox(x + 2.0, bottom + 5.0, checked)?;
            } else {
                let label = start.map_or_else(
                    || "•".into(),
                    |start| format!("{}.", start.saturating_add(index as u64)),
                );
                self.text(&label, &style, x + 1.0, bottom, indent.max(1.0))?;
            }
            let next = self.blocks(&item.blocks, x + indent, bottom, (width - indent).max(1.0))?;
            bottom = next.max(bottom + BODY_SIZE * 1.45);
        }
        Ok(bottom)
    }

    fn checkbox(&mut self, x: f64, y: f64, checked: bool) -> Result<(), Error> {
        let bounds = Rect::new(x, y, x + 13.0, y + 13.0);
        let color = Color::from_rgba8(55, 105, 93, 255);
        if checked {
            self.rectangle(bounds, 3.0, color);
            let points = [
                (2.8, 6.5),
                (4.0, 5.5),
                (5.7, 7.6),
                (9.3, 3.9),
                (10.4, 4.9),
                (5.6, 9.9),
            ];
            let vertices = points.map(|(dx, dy)| Point::new(x + dx, y + dy));
            let mut commands = vec![PathCommand::MoveTo(vertices[0])];
            commands.extend(vertices[1..].iter().copied().map(PathCommand::LineTo));
            commands.push(PathCommand::Close);
            self.items.push(Item {
                bounds,
                paint: Paint::Primitive(Primitive::FillPath(
                    BezierPath::new(commands).map_err(|_| Error::InvalidInput)?,
                    oxiview_scene::FillRule::NonZero,
                    Brush::Solid(Color::from_rgba8(255, 255, 255, 255)),
                )),
            });
        } else {
            self.items.push(Item {
                bounds,
                paint: Paint::Primitive(Primitive::Stroke(
                    bounds,
                    CornerRadii::uniform(3.0),
                    1.2,
                    Brush::Solid(color),
                )),
            });
        }
        Ok(())
    }

    fn table(&mut self, table: &Table, x: f64, y: f64, width: f64) -> Result<f64, Error> {
        let columns = table.alignments.len();
        if columns == 0 {
            return Ok(y);
        }
        let cell_width = width / columns as f64;
        let inset = (cell_width * 0.14).min(10.0);
        let mut top = y + 1.0;
        for (index, row) in table.rows.iter().enumerate() {
            let mut height = 20.0_f64;
            let mut cells = Vec::new();
            for (runs, alignment) in row.iter().zip(&table.alignments) {
                let style = TextStyle {
                    font_size: 14.0,
                    weight: if index == 0 { 700 } else { 400 },
                    align: match alignment {
                        CellAlignment::Start => TextAlign::Start,
                        CellAlignment::Center => TextAlign::Center,
                        CellAlignment::End => TextAlign::End,
                    },
                    ..body_style()
                };
                let cell = inline::paragraph(
                    self.text,
                    runs,
                    self.diagnostics,
                    &style,
                    (cell_width - inset * 2.0).max(1.0),
                )?;
                height = height.max(cell.height);
                cells.push(cell);
            }
            let bottom = top + height + 13.0;
            if index == 0 || index % 2 == 0 {
                let fill = if index == 0 {
                    Color::from_rgba8(232, 239, 237, 255)
                } else {
                    surface_color()
                };
                self.rectangle(Rect::new(x, top, x + width, bottom), 0.0, fill);
            }
            for (column, cell) in cells.into_iter().enumerate() {
                self.paragraph(cell, x + column as f64 * cell_width + inset, top + 6.0);
            }
            self.rectangle(
                Rect::new(x, bottom - 1.0, x + width, bottom),
                0.0,
                border_color(),
            );
            top = bottom;
        }
        self.rectangle(Rect::new(x, y, x + width, y + 1.0), 0.0, border_color());
        for column in 0..=columns {
            let left = (x + column as f64 * cell_width).min(x + width - 1.0);
            self.rectangle(Rect::new(left, y, left + 1.0, top), 0.0, border_color());
        }
        Ok(top)
    }

    fn diagnostic(&mut self, index: usize, x: f64, y: f64, width: f64) -> Result<f64, Error> {
        let Some(diagnostic) = self.diagnostics.get(index) else {
            return Ok(y);
        };
        let start = self.items.len();
        let style = TextStyle {
            color: error_color(),
            font_size: 14.0,
            ..body_style()
        };
        let inset = (width * 0.12).min(12.0);
        let height = self.text(
            &diagnostic.to_string(),
            &style,
            x + inset,
            y + 8.0,
            (width - inset * 2.0).max(1.0),
        )?;
        let bottom = y + height + 16.0;
        let fill = Color::from_rgba8(254, 241, 239, 255);
        self.items.insert(
            start,
            rectangle(Rect::new(x, y, x + width, bottom), 6.0, fill),
        );
        Ok(bottom)
    }

    fn paragraph(&mut self, paragraph: inline::Paragraph, x: f64, mut y: f64) {
        for line in paragraph.lines {
            let mut left = x + line.offset;
            for piece in line.pieces {
                let top = y + line.baseline - piece.baseline;
                let width = piece.size.width();
                self.piece(piece, Point::new(left, top));
                left += width;
            }
            y += line.height;
        }
    }

    fn piece(&mut self, piece: inline::Piece, origin: Point) {
        self.items.push(Item {
            bounds: Rect::from_origin_size(origin, piece.size),
            paint: piece.paint,
        });
    }

    fn text(
        &mut self,
        value: &str,
        style: &TextStyle,
        x: f64,
        y: f64,
        width: f64,
    ) -> Result<f64, Error> {
        let layout = self.text.layout(value, style, Some(width))?;
        let size = layout.size();
        self.items.push(Item {
            bounds: Rect::from_origin_size(Point::new(x, y), size),
            paint: Paint::Text(layout),
        });
        Ok(size.height())
    }

    fn rectangle(&mut self, bounds: Rect, radius: f64, color: Color) {
        self.items.push(rectangle(bounds, radius, color));
    }
}

fn rectangle(bounds: Rect, radius: f64, color: Color) -> Item {
    Item {
        bounds,
        paint: Paint::Primitive(Primitive::Fill(
            bounds,
            CornerRadii::uniform(radius),
            Brush::Solid(color),
        )),
    }
}

fn paint_text(
    system: &mut TextSystem,
    layout: &TextLayout,
    origin: Point,
    device: Affine,
    primitives: &mut Vec<Primitive>,
) -> Result<(), Error> {
    let decorations = |kind| {
        layout
            .decorations()
            .filter(move |decoration| decoration.kind == kind)
            .map(|decoration| {
                Primitive::Fill(
                    map_rect(decoration.rect, origin, 1.0),
                    CornerRadii::default(),
                    Brush::Solid(decoration.color),
                )
            })
    };
    primitives.extend(decorations(TextDecorationKind::Underline));
    primitives.extend(system.glyph_draws(layout, origin, device)?.into_iter().map(
        |draw| match draw {
            GlyphDraw::Image(glyph) => Primitive::Image(glyph.rect, glyph.image),
            GlyphDraw::Vector(glyph) => Primitive::Glyph(glyph),
            GlyphDraw::Path(path, color) => {
                Primitive::FillPath(path, oxiview_scene::FillRule::NonZero, Brush::Solid(color))
            }
        },
    ));
    primitives.extend(decorations(TextDecorationKind::Strikethrough));
    Ok(())
}

fn map_rect(rect: Rect, offset: Point, scale: f64) -> Rect {
    let [left, top, right, bottom] = rect.edges();
    Rect::new(
        left * scale + offset.x(),
        top * scale + offset.y(),
        right * scale + offset.x(),
        bottom * scale + offset.y(),
    )
}

fn map_primitive(primitive: &Primitive, offset: Point, scale: f64) -> Result<Primitive, Error> {
    let point = |point: Point| {
        Point::new(
            point.x() * scale + offset.x(),
            point.y() * scale + offset.y(),
        )
    };
    let rect = |rect| map_rect(rect, offset, scale);
    let radii = |radii: CornerRadii| {
        let [tl, tr, br, bl] = radii.values().map(|radius| radius * scale);
        CornerRadii::new(tl, tr, br, bl)
    };
    let brush = |brush: &Brush| match brush {
        Brush::Solid(color) => Brush::Solid(*color),
        Brush::LinearGradient(start, end, stops) => {
            Brush::LinearGradient(point(*start), point(*end), stops.clone())
        }
    };
    Ok(match primitive {
        Primitive::Fill(value, corners, color) => {
            Primitive::Fill(rect(*value), radii(*corners), brush(color))
        }
        Primitive::Stroke(value, corners, width, color) => {
            Primitive::Stroke(rect(*value), radii(*corners), width * scale, brush(color))
        }
        Primitive::Image(value, image) => Primitive::Image(rect(*value), image.clone()),
        Primitive::FillPath(path, rule, color) => {
            let commands = path
                .commands()
                .iter()
                .map(|command| match *command {
                    PathCommand::MoveTo(p) => PathCommand::MoveTo(point(p)),
                    PathCommand::LineTo(p) => PathCommand::LineTo(point(p)),
                    PathCommand::QuadTo(a, b) => PathCommand::QuadTo(point(a), point(b)),
                    PathCommand::CubicTo(a, b, c) => {
                        PathCommand::CubicTo(point(a), point(b), point(c))
                    }
                    PathCommand::Close => PathCommand::Close,
                })
                .collect();
            Primitive::FillPath(
                BezierPath::new(commands).map_err(|_| Error::InvalidInput)?,
                *rule,
                brush(color),
            )
        }
        Primitive::Glyph(glyph) => {
            let mut glyph = glyph.clone();
            glyph.origin = point(glyph.origin);
            glyph.size *= scale as f32;
            Primitive::Glyph(glyph)
        }
    })
}
