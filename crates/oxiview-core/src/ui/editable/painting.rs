use super::{EditorNode, Rect, UiError, translate_rect, ui_layout_error};
use oxiview_base::{Affine, Color, CornerRadii, Point};
use oxiview_scene::{Brush, Clip, FillRule, NodeId, NodeKind, Primitive, Scene};
use oxiview_text::GlyphDraw;
use std::ops::Range;

pub(in crate::ui) struct EditorDraws {
    clip: NodeId,
    selection: DrawGroup,
    glyphs: DrawGroup,
    underline: DrawGroup,
    caret: NodeId,
    glyph_key: Option<DrawKey>,
    selection_key: Option<(DrawKey, Range<usize>, bool)>,
    underline_key: Option<(DrawKey, Option<Range<usize>>)>,
    caret_rect: Option<Rect>,
}

#[derive(Clone, Copy, PartialEq)]
struct DrawKey {
    text_revision: u64,
    origin: Point,
    viewport: Rect,
}

impl EditorDraws {
    pub(in crate::ui) fn mount(
        scene: &mut Scene,
        parent: NodeId,
        editor: &EditorNode,
        viewport: Rect,
        scale: f64,
    ) -> Result<Self, UiError> {
        let clip = append(scene, parent, NodeKind::Clip(Clip::Rect(viewport)))?;
        let selection = DrawGroup::mount(scene, clip)?;
        let glyphs = DrawGroup::mount(scene, clip)?;
        let underline = DrawGroup::mount(scene, clip)?;
        let caret = append(scene, clip, NodeKind::Draw(hidden_caret()))?;
        let mut draws = Self {
            clip,
            selection,
            glyphs,
            underline,
            caret,
            glyph_key: None,
            selection_key: None,
            underline_key: None,
            caret_rect: None,
        };
        draws.update(scene, editor, viewport, scale)?;
        Ok(draws)
    }

    pub(in crate::ui) fn update(
        &mut self,
        scene: &mut Scene,
        editor: &EditorNode,
        viewport: Rect,
        scale: f64,
    ) -> Result<(), UiError> {
        let geometry = editor.prepare_geometry(viewport)?;
        let model = editor.control.model()?;
        let presentation = model.presentation();
        let focused = model.session().is_some();
        let key = DrawKey {
            text_revision: presentation.text_revision,
            origin: geometry.origin,
            viewport,
        };
        let [_, top, _, bottom] = viewport.edges();
        let visible_y = top - geometry.origin.y()..bottom - geometry.origin.y();
        if self.glyph_key.is_none_or(|old| old.viewport != viewport) {
            scene.replace(self.clip, NodeKind::Clip(Clip::Rect(viewport)))?;
        }
        if self.glyph_key != Some(key) {
            let glyphs = editor
                .system
                .try_borrow_mut()
                .map_err(|_| UiError::HandlerBusy)?
                .editable_glyph_draws(
                    &geometry.layout,
                    geometry.origin,
                    Affine::new([scale, 0.0, 0.0, scale, 0.0, 0.0]),
                    visible_y.clone(),
                )
                .map_err(ui_layout_error)?;
            self.glyphs.update(
                scene,
                glyphs.into_iter().map(|draw| match draw {
                    GlyphDraw::Image(glyph) => Primitive::Image(glyph.rect, glyph.image),
                    GlyphDraw::Vector(glyph) => Primitive::Glyph(glyph),
                    GlyphDraw::Path(path, color) => {
                        Primitive::FillPath(path, FillRule::NonZero, Brush::Solid(color))
                    }
                }),
            )?;
            self.glyph_key = Some(key);
        }
        let selection_key = (key, presentation.selection.range(), focused);
        if self.selection_key.as_ref() != Some(&selection_key) {
            let selection_color = if focused {
                Color::from_rgba8(76, 136, 252, 120)
            } else {
                Color::from_rgba8(132, 140, 156, 86)
            };
            let selection = geometry
                .layout
                .visible_selection_rects(selection_key.1.clone(), visible_y.clone());
            self.selection.update(
                scene,
                selection
                    .into_iter()
                    .map(|rect| fill(translate_rect(rect, geometry.origin), selection_color)),
            )?;
            self.selection_key = Some(selection_key);
        }
        let underline_key = (key, presentation.marked);
        if self.underline_key.as_ref() != Some(&underline_key) {
            let underline = underline_key.1.as_ref().map_or_else(Vec::new, |range| {
                geometry
                    .layout
                    .visible_selection_rects(range.clone(), visible_y)
            });
            self.underline.update(
                scene,
                underline.into_iter().map(|rect| {
                    let [x0, y0, x1, y1] = translate_rect(rect, geometry.origin).edges();
                    fill(
                        Rect::new(x0, (y1 - 2.0).max(y0), x1, (y1 - 1.0).max(y0)),
                        editor.style.color,
                    )
                }),
            )?;
            self.underline_key = Some(underline_key);
        }
        let caret = if focused && presentation.selection.is_empty() && editor.blink.get().visible {
            geometry
                .layout
                .caret_rect(presentation.selection.active)
                .map(|rect| translate_rect(rect, geometry.origin))
        } else {
            None
        };
        if self.caret_rect != caret {
            scene.replace(
                self.caret,
                NodeKind::Draw(
                    caret.map_or_else(hidden_caret, |rect| fill(rect, editor.style.color)),
                ),
            )?;
            self.caret_rect = caret;
        }
        Ok(())
    }
}

struct DrawGroup {
    root: NodeId,
    draws: Vec<NodeId>,
}

impl DrawGroup {
    fn mount(scene: &mut Scene, parent: NodeId) -> Result<Self, UiError> {
        Ok(Self {
            root: append(scene, parent, NodeKind::Transform(Affine::IDENTITY))?,
            draws: Vec::new(),
        })
    }

    fn update(
        &mut self,
        scene: &mut Scene,
        primitives: impl Iterator<Item = Primitive>,
    ) -> Result<(), UiError> {
        let mut count = 0;
        for (index, primitive) in primitives.enumerate() {
            let kind = NodeKind::Draw(primitive);
            if let Some(&draw) = self.draws.get(index) {
                scene.replace(draw, kind)?;
            } else {
                self.draws.push(scene.insert(Some(self.root), index, kind)?);
            }
            count += 1;
        }
        for draw in self.draws.drain(count..).rev() {
            scene.remove(draw)?;
        }
        Ok(())
    }
}

fn fill(rect: Rect, color: Color) -> Primitive {
    Primitive::Fill(rect, CornerRadii::default(), Brush::Solid(color))
}

fn hidden_caret() -> Primitive {
    fill(Rect::default(), Color::from_rgba8(0, 0, 0, 0))
}

fn append(scene: &mut Scene, parent: NodeId, kind: NodeKind) -> Result<NodeId, UiError> {
    Ok(scene.insert(Some(parent), scene.children(Some(parent))?.len(), kind)?)
}
