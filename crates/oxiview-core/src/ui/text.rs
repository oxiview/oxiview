use super::{NodeId, NodeMeasure, UiError, UiTree, View, node};
use oxiview_base::{Point, Rect, Size};
use oxiview_layout::{AvailableSpace, Dimensions, LayoutBox};
use oxiview_reactive::{ReactiveError, effect};
use oxiview_scene::Primitive;
use oxiview_text::{TextLayout, TextStyle, TextSystem};
use std::{cell::RefCell, rc::Rc};

struct Cached {
    width: Option<f64>,
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
    source: RefCell<String>,
    style: TextStyle,
    cached: RefCell<Option<Cached>>,
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
        *cached = None;
        *raster = None;
        Ok(())
    }
    fn paragraph(&self, width: Option<f64>) -> Result<TextLayout, UiError> {
        if let Some(cached) = self
            .cached
            .try_borrow()
            .map_err(|_| UiError::HandlerBusy)?
            .as_ref()
            && cached.width == width
        {
            return Ok(cached.layout.clone());
        }
        let source = self.source.try_borrow().map_err(|_| UiError::HandlerBusy)?;
        let layout = self
            .system
            .try_borrow_mut()
            .map_err(|_| UiError::HandlerBusy)?
            .layout(&source, &self.style, width)
            .map_err(|error| UiError::Text(error.to_string()))?;
        *self
            .cached
            .try_borrow_mut()
            .map_err(|_| UiError::HandlerBusy)? = Some(Cached {
            width,
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
        let content_origin = Point::new(window_origin.x() + left, window_origin.y() + top);
        let device_x = content_origin.x() * scale;
        let device_y = content_origin.y() * scale;
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
        let primitives = self
            .system
            .try_borrow_mut()
            .map_err(|_| UiError::HandlerBusy)?
            .glyph_images(&layout, content_origin, scale)
            .map_err(|error| UiError::Text(error.to_string()))?
            .into_iter()
            .map(|glyph| {
                // Raster placement is snapped in window/device space. The Scene
                // still applies this node's content translation exactly once.
                let [left, top, right, bottom] = glyph.rect.edges();
                Primitive::Image(
                    Rect::new(
                        left - window_origin.x(),
                        top - window_origin.y(),
                        right - window_origin.x(),
                        bottom - window_origin.y(),
                    ),
                    glyph.image,
                )
            })
            .collect::<Vec<_>>();
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
        let width = known.width.or(match available.width {
            AvailableSpace::Definite(width) => Some(width.max(0.0)),
            AvailableSpace::MinContent => Some(0.0),
            AvailableSpace::MaxContent => None,
        });
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
}

/// Creates a real shaped-text node. Measurement uses logical available width;
/// glyph rasterization uses the actual scale supplied when the tree is painted.
pub fn text(system: Rc<RefCell<TextSystem>>, source: impl Into<String>, style: TextStyle) -> View {
    let source = source.into();
    let text = Rc::new(TextNode {
        system,
        source: RefCell::new(source),
        style,
        cached: RefCell::new(None),
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
        mut self,
        mut compute: impl FnMut() -> Result<String, ReactiveError> + 'static,
    ) -> Self {
        self.bindings.push(Box::new(move |id, sender| {
            effect(move || {
                sender
                    .set_text(id, compute()?)
                    .map_err(|error| ReactiveError::User(error.to_string()))
            })
            .map_err(|error| UiError::Reactive(error.to_string()))?;
            Ok(())
        }));
        self
    }
}
impl UiTree {
    /// Replaces the text source and invalidates its measurement and painting.
    pub fn set_text(&mut self, id: NodeId, source: impl Into<String>) -> Result<(), UiError> {
        let text = self
            .node(id)?
            .text
            .as_ref()
            .cloned()
            .ok_or_else(|| UiError::Text("node has no text content".into()))?;
        let source = source.into();
        let mut old = text
            .source
            .try_borrow_mut()
            .map_err(|_| UiError::HandlerBusy)?;
        if *old != source {
            text.invalidate()?;
            self.bump_measure_revision(id)?;
            *old = source;
            self.node_mut(id)?.painting = None;
        }
        Ok(())
    }
    /// Reads a text node's current source string.
    pub fn text_source(&self, id: NodeId) -> Result<String, UiError> {
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
