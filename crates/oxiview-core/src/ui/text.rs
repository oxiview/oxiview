use super::{NodeId, NodeMeasure, UiError, UiTree, View, node};
use oxiview_base::{Affine, CornerRadii, Point, Rect, Size};
use oxiview_layout::{AvailableSpace, Dimensions, LayoutBox};
use oxiview_reactive::{ReactiveError, effect};
use oxiview_scene::{Brush, FillRule, Primitive};
use oxiview_text::{
    GlyphDraw, RichText, TextDecorationKind, TextLayout, TextSpan, TextSpanStyle, TextStyle,
    TextSystem,
};
use std::{cell::RefCell, collections::VecDeque, rc::Rc};

const PARAGRAPH_CACHE_CAPACITY: usize = 4;

struct Cached {
    width: Option<f32>,
    layout: TextLayout,
}
struct RasterCache {
    width: f64,
    left: f64,
    top: f64,
    scale: f64,
    phase_x: f64,
    phase_y: f64,
    primitives: Vec<Primitive>,
}
pub(super) struct TextNode {
    system: Rc<RefCell<TextSystem>>,
    source: RefCell<RichText>,
    style: TextStyle,
    cached: RefCell<VecDeque<Cached>>,
    raster: RefCell<Option<RasterCache>>,
}
impl TextNode {
    pub(super) fn invalidate(&self) -> Result<(), UiError> {
        let mut cached = self
            .cached
            .try_borrow_mut()
            .map_err(|_| UiError::HandlerBusy)?;
        let mut raster = self
            .raster
            .try_borrow_mut()
            .map_err(|_| UiError::HandlerBusy)?;
        cached.clear();
        *raster = None;
        Ok(())
    }
    fn paragraph(&self, width: Option<f64>) -> Result<TextLayout, UiError> {
        let width_key = width.map(|width| width as f32);
        let mut cache = self
            .cached
            .try_borrow_mut()
            .map_err(|_| UiError::HandlerBusy)?;
        if let Some(cached) = cache.iter().find(|cached| cached.width == width_key) {
            return Ok(cached.layout.clone());
        }
        let layout = if let Some(cached) = cache.back() {
            cached.layout.rewrap(width)
        } else {
            let source = self.source.try_borrow().map_err(|_| UiError::HandlerBusy)?;
            self.system
                .try_borrow_mut()
                .map_err(|_| UiError::HandlerBusy)?
                .layout_rich(&source, &self.style, width)
        }
        .map_err(|error| UiError::Text(error.to_string()))?;
        if cache.len() == PARAGRAPH_CACHE_CAPACITY {
            cache.pop_front();
        }
        cache.push_back(Cached {
            width: width_key,
            layout: layout.clone(),
        });
        Ok(layout)
    }
    pub(super) fn primitives(
        &self,
        geometry: LayoutBox,
        window_origin: Point,
        scale: f64,
    ) -> Result<Vec<Primitive>, UiError> {
        let left = geometry.border.left + geometry.padding.left;
        let top = geometry.border.top + geometry.padding.top;
        let width = (geometry.size.width() - left - geometry.border.right - geometry.padding.right)
            .max(0.0);
        let device_x = (window_origin.x() + left) * scale;
        let device_y = (window_origin.y() + top) * scale;
        let phase_x = device_x - device_x.floor();
        let phase_y = device_y - device_y.floor();
        if let Some(cached) = self
            .raster
            .try_borrow()
            .map_err(|_| UiError::HandlerBusy)?
            .as_ref()
            && cached.width == width
            && cached.left == left
            && cached.top == top
            && cached.scale == scale
            && cached.phase_x == phase_x
            && cached.phase_y == phase_y
        {
            return Ok(cached.primitives.clone());
        }
        let layout = self.paragraph(Some(width))?;
        let glyphs = self
            .system
            .try_borrow_mut()
            .map_err(|_| UiError::HandlerBusy)?
            .glyph_draws(
                &layout,
                Point::new(left, top),
                Affine::new([
                    scale,
                    0.0,
                    0.0,
                    scale,
                    window_origin.x() * scale,
                    window_origin.y() * scale,
                ]),
            )
            .map_err(|error| UiError::Text(error.to_string()))?
            .into_iter()
            .map(|draw| match draw {
                GlyphDraw::Image(glyph) => Primitive::Image(glyph.rect, glyph.image),
                GlyphDraw::Vector(glyph) => Primitive::Glyph(glyph),
                GlyphDraw::Path(path, color) => {
                    Primitive::FillPath(path, FillRule::NonZero, Brush::Solid(color))
                }
            });
        let mut primitives = Vec::new();
        let mut strikethroughs = Vec::new();
        for decoration in layout.decorations() {
            let [x0, y0, x1, y1] = decoration.rect.edges();
            let primitive = Primitive::Fill(
                Rect::new(x0 + left, y0 + top, x1 + left, y1 + top),
                CornerRadii::default(),
                Brush::Solid(decoration.color),
            );
            match decoration.kind {
                TextDecorationKind::Underline => primitives.push(primitive),
                TextDecorationKind::Strikethrough => strikethroughs.push(primitive),
            }
        }
        primitives.extend(glyphs);
        primitives.extend(strikethroughs);
        *self
            .raster
            .try_borrow_mut()
            .map_err(|_| UiError::HandlerBusy)? = Some(RasterCache {
            width,
            left,
            top,
            scale,
            phase_x,
            phase_y,
            primitives: primitives.clone(),
        });
        Ok(primitives)
    }
}
impl NodeMeasure for Rc<TextNode> {
    fn measure(
        &self,
        known: Dimensions<Option<f64>>,
        available: Dimensions<AvailableSpace>,
    ) -> Dimensions<f64> {
        let width = match available.width {
            AvailableSpace::Definite(width) => Some(width.max(0.0)),
            AvailableSpace::MinContent => Some(0.0),
            AvailableSpace::MaxContent => known.width,
        };
        match self.paragraph(width) {
            Ok(layout) => {
                let size = layout.size();
                Dimensions {
                    width: known.width.unwrap_or(size.width()),
                    height: known.height.unwrap_or(size.height()),
                }
            }
            Err(error) => {
                tracing::error!(%error, "text measurement failed");
                Dimensions {
                    width: f64::NAN,
                    height: f64::NAN,
                }
            }
        }
    }

    fn first_baseline(&self, content_width: f64) -> Option<f64> {
        match self.paragraph(Some(content_width)) {
            Ok(layout) => layout.first_baseline(),
            Err(error) => {
                tracing::error!(%error, "text baseline measurement failed");
                Some(f64::NAN)
            }
        }
    }
}

/// Creates a real shaped-text node. Measurement uses logical available width;
/// glyph drawing uses the actual scale supplied when the tree is painted.
pub fn text(system: Rc<RefCell<TextSystem>>, source: impl Into<String>, style: TextStyle) -> View {
    rich_text(system, RichText::plain(source.into()), style)
}

/// Creates a shaped-text node from validated rich content and a paragraph style.
///
/// ```
/// use oxiview_core::{RichText, TextSpan, TextSpanStyle, TextStyle, TextSystem, rich_text};
/// use std::{cell::RefCell, rc::Rc};
///
/// let content = RichText::new("Hello world", vec![TextSpan {
///     range: 6..11,
///     style: TextSpanStyle { weight: Some(700), italic: true, ..Default::default() },
/// }])?;
/// let system = Rc::new(RefCell::new(TextSystem::new()));
/// let view = rich_text(system, content, TextStyle::default());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn rich_text(system: Rc<RefCell<TextSystem>>, source: RichText, style: TextStyle) -> View {
    let text = Rc::new(TextNode {
        system,
        source: RefCell::new(source),
        style,
        cached: RefCell::new(VecDeque::new()),
        raster: RefCell::new(None),
    });
    let mut view = node().measure(Rc::clone(&text));
    view.setup.push(Box::new(move |tree, id, _| {
        tree.node_mut(id)?.text = Some(text);
        Ok(())
    }));
    view
}
impl View {
    /// Binds the source string of a `text` node without remounting it.
    pub fn bind_text(
        self,
        mut compute: impl FnMut() -> Result<String, ReactiveError> + 'static,
    ) -> Self {
        self.bind_rich_text(move || compute().map(RichText::plain))
    }

    /// Binds paragraph content and span styles without remounting the text node.
    pub fn bind_rich_text(
        mut self,
        mut compute: impl FnMut() -> Result<RichText, ReactiveError> + 'static,
    ) -> Self {
        self.bindings.push(Box::new(move |id, sender| {
            effect(move || {
                sender
                    .set_rich_text(id, compute()?)
                    .map_err(|error| ReactiveError::User(error.to_string()))
            })
            .map_err(|error| UiError::Reactive(error.to_string()))?;
            Ok(())
        }));
        self
    }
}
impl UiTree {
    /// Replaces the text source with plain content and clears any span overrides.
    pub fn set_text(&mut self, id: NodeId, source: impl Into<String>) -> Result<(), UiError> {
        self.set_rich_text(id, RichText::plain(source.into()))
    }

    /// Replaces the paragraph's content and invalidates affected layout and painting caches.
    /// Foreground and decoration edits reuse the committed geometry.
    pub fn set_rich_text(&mut self, id: NodeId, source: RichText) -> Result<(), UiError> {
        let text = self
            .node(id)?
            .text
            .as_ref()
            .cloned()
            .ok_or_else(|| UiError::Text("node has no text content".into()))?;
        let mut old = text
            .source
            .try_borrow_mut()
            .map_err(|_| UiError::HandlerBusy)?;
        if *old != source {
            let layout_changed = old.as_str() != source.as_str()
                || !metric_overrides(&old).eq(metric_overrides(&source));
            text.invalidate()?;
            if layout_changed {
                self.bump_measure_revision(id)?;
            } else {
                self.dirty_paint();
            }
            *old = source;
            self.node_mut(id)?.painting = None;
        }
        Ok(())
    }
    /// Reads a text node's current source string.
    pub fn text_source(&self, id: NodeId) -> Result<String, UiError> {
        self.rich_text_source(id)
            .map(|source| source.as_str().into())
    }

    /// Reads a text node's current source and validated spans.
    pub fn rich_text_source(&self, id: NodeId) -> Result<RichText, UiError> {
        let text = self
            .node(id)?
            .text
            .as_ref()
            .ok_or_else(|| UiError::Text("node has no text content".into()))?;
        Ok(text
            .source
            .try_borrow()
            .map_err(|_| UiError::HandlerBusy)?
            .clone())
    }
    /// Returns the last committed text node border-box size.
    pub fn text_size(&self, id: NodeId) -> Result<Size, UiError> {
        if self.node(id)?.text.is_none() {
            return Err(UiError::Text("node has no text content".into()));
        }
        Ok(self.node(id)?.layout.size)
    }
}

fn metric_overrides(
    source: &RichText,
) -> impl Iterator<Item = (&std::ops::Range<usize>, SpanMetrics<'_>)> {
    source
        .spans()
        .iter()
        .filter_map(|TextSpan { range, style }| {
            let TextSpanStyle {
                family,
                font_size,
                weight,
                italic,
                color: _,
                underline: _,
                strikethrough: _,
                letter_spacing,
                word_spacing,
                line_height,
            } = style;
            let metrics = SpanMetrics {
                family: family.as_deref(),
                font_size: *font_size,
                weight: *weight,
                italic: *italic,
                letter_spacing: *letter_spacing,
                word_spacing: *word_spacing,
                line_height: *line_height,
            };
            (metrics != SpanMetrics::default()).then_some((range, metrics))
        })
}

#[derive(Default, PartialEq)]
struct SpanMetrics<'a> {
    family: Option<&'a str>,
    font_size: Option<f64>,
    weight: Option<u16>,
    italic: bool,
    letter_spacing: Option<f64>,
    word_spacing: Option<f64>,
    line_height: Option<f64>,
}
