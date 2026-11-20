use super::{Node, NodeId, UiError, UiTree, editable::EditorDraws};
use crate::PaintedScene;
use oxiview_base::{Affine, Color, CornerRadii, Point, Rect, Size};
use oxiview_layout::LayoutBox;
use oxiview_scene::{Brush, Clip, NodeId as SceneNodeId, NodeKind, Primitive, Scene};
use oxiview_style::{Display, FocusRing, Overflow, Position};
use std::collections::HashMap;

struct PaintItem {
    id: NodeId,
    context: bool,
    phase: u8,
    z: i32,
    ordinal: usize,
}

// The node owns its prepared drawing. Stacking and ancestor scopes are resolved
// for each new Scene, since a descendant may paint outside its parent's context.
pub(super) struct NodePainting {
    rect: Rect,
    text_key: Option<(LayoutBox, f64)>,
    background_radii: Option<CornerRadii>,
    border: Option<BorderPainting>,
    content: Option<ContentPainting>,
}

struct BorderPainting {
    clip: Clip,
    rect: Rect,
    radii: CornerRadii,
    width: f64,
}

struct ContentPainting {
    clip: Option<Clip>,
    transform: Affine,
    glyphs: Vec<Primitive>,
}

#[derive(Default)]
struct PaintFrame {
    scene: Scene,
    prepared: Vec<(NodeId, Box<NodePainting>)>,
    draws: HashMap<NodeId, DrawHandles>,
    editors: HashMap<NodeId, EditorDraws>,
}

#[derive(Default)]
struct DrawHandles {
    background: Option<SceneNodeId>,
    border: Option<SceneNodeId>,
    focus_ring: Option<SceneNodeId>,
    custom: Vec<SceneNodeId>,
}

#[derive(Default)]
pub(super) struct DrawChanges {
    pub(super) background: bool,
    pub(super) border: bool,
    pub(super) focus_ring: bool,
    pub(super) custom: bool,
    pub(super) editor: bool,
}

pub(super) struct PaintCache {
    scene: Scene,
    root: SceneNodeId,
    scale: f64,
    draws: HashMap<NodeId, DrawHandles>,
    editors: HashMap<NodeId, EditorDraws>,
    changes: HashMap<NodeId, DrawChanges>,
}

impl PaintCache {
    fn snapshot(&self) -> PaintedScene {
        PaintedScene {
            scene: self.scene.clone(),
            root: self.root,
        }
    }

    fn update(&mut self, tree: &UiTree) -> Result<(), UiError> {
        for (id, changes) in std::mem::take(&mut self.changes) {
            if changes.editor
                && let Some(draws) = self.editors.get_mut(&id)
                && let Some(editor) = tree.editors.get(&id)
            {
                draws.update(
                    &mut self.scene,
                    editor,
                    tree.editor_viewport(id)?,
                    self.scale,
                )?;
            }
            // A suppressed node has no draws in the retained Scene.
            if let Some(draws) = self.draws.get(&id) {
                draws.replace(&mut self.scene, tree.node(id)?, changes)?;
            }
        }
        Ok(())
    }
}

impl DrawHandles {
    fn replace(&self, scene: &mut Scene, node: &Node, changes: DrawChanges) -> Result<(), UiError> {
        if let Some(painting) = &node.painting {
            if changes.background
                && let Some((id, primitive)) = self
                    .background
                    .zip(painting.background(node.style.background))
            {
                scene.replace(id, NodeKind::Draw(primitive))?;
            }
            if changes.border
                && let Some((id, border)) = self.border.zip(painting.border.as_ref())
            {
                scene.replace(
                    id,
                    NodeKind::Draw(border.primitive(node.style.border_color)),
                )?;
            }
            if changes.focus_ring
                && let Some((id, primitive)) = self.focus_ring.zip(painting.focus_ring(node))
            {
                scene.replace(id, NodeKind::Draw(primitive))?;
            }
        }
        if changes.custom {
            // A length change invalidates the Scene and its handles before paint.
            for (&id, primitive) in self.custom.iter().zip(&node.content) {
                scene.replace(id, NodeKind::Draw(primitive.clone()))?;
            }
        }
        Ok(())
    }
}

impl UiTree {
    /// Prepares a complete scene with one logical-to-physical root transform.
    /// Paint-only draw edits reuse the existing Scene structure.
    pub fn paint(&mut self, viewport: Size, scale: f64) -> Result<PaintedScene, UiError> {
        if !scale.is_finite() || scale <= 0.0 {
            return Err(UiError::InvalidViewport);
        }
        self.layout(viewport)?;
        if let Some(cache) = &self.paint_cache
            && cache.scale == scale
            && cache.changes.is_empty()
        {
            self.diagnostics.scene_cache_hits = self.diagnostics.scene_cache_hits.saturating_add(1);
            self.paint_dirty = false;
            return Ok(cache.snapshot());
        }
        self.repaint(scale)
    }

    fn repaint(&mut self, scale: f64) -> Result<PaintedScene, UiError> {
        if let Some(mut cache) = self.paint_cache.take()
            && cache.scale == scale
        {
            let changed_nodes = cache.changes.len();
            let _span = tracing::info_span!("ui_paint_update", nodes = changed_nodes).entered();
            // A failed replacement drops this cache. Earlier snapshots are
            // independent, and the next paint rebuilds from the live tree.
            cache.update(self)?;
            self.diagnostics.record_scene_update(changed_nodes);
            let painted = cache.snapshot();
            self.paint_cache = Some(cache);
            self.paint_dirty = false;
            return Ok(painted);
        }
        self.paint_dirty = true;
        self.build_scene(scale)
    }

    pub(super) fn dirty_draws(&mut self, id: NodeId) -> Option<&mut DrawChanges> {
        self.paint_dirty = true;
        self.paint_cache
            .as_mut()
            .map(|cache| cache.changes.entry(id).or_default())
    }

    fn build_scene(&mut self, scale: f64) -> Result<PaintedScene, UiError> {
        let span = tracing::info_span!(
            "ui_paint",
            nodes = self.len(),
            painted_nodes = tracing::field::Empty
        );
        let _entered = span.enter();
        let mut frame = PaintFrame::default();
        let root = frame.scene.insert(
            None,
            0,
            NodeKind::Transform(Affine::new([scale, 0.0, 0.0, scale, 0.0, 0.0])),
        )?;
        if !self.suppressed(self.root)? {
            self.paint_context(&mut frame, root, self.root, None, scale)?;
        }
        let PaintFrame {
            scene,
            prepared,
            draws,
            editors,
        } = frame;
        let painted_nodes = prepared.len();
        for (id, painting) in prepared {
            self.node_mut(id)?.painting = Some(painting);
        }
        span.record("painted_nodes", painted_nodes);
        self.diagnostics
            .record_scene_build(draws.len(), painted_nodes);
        self.paint_cache = Some(PaintCache {
            scene: scene.clone(),
            root,
            scale,
            draws,
            editors,
            changes: HashMap::new(),
        });
        self.paint_dirty = false;
        Ok(PaintedScene { scene, root })
    }

    /// Performs geometric hit testing against committed layout and final paint order.
    /// Opacity and absence of handlers do not affect hit participation.
    pub fn hit_test(&self, position: Point) -> Result<Option<NodeId>, UiError> {
        if !position.x().is_finite() || !position.y().is_finite() {
            return Ok(None);
        }
        let _span = tracing::trace_span!("ui_hit_test").entered();
        if self.suppressed(self.root)? {
            return Ok(None);
        }
        let mut order = Vec::new();
        self.flatten_context(self.root, &mut order)?;
        for id in order.into_iter().rev() {
            let node = self.node(id)?;
            if !node.style.hit_test {
                continue;
            }
            let rect = self.bounds(id)?;
            if !rounded_contains(
                rect,
                normalized_radii(rect, node.style.border_radii),
                position,
            ) {
                continue;
            }
            let mut clipped = false;
            for ancestor in self
                .path(id)?
                .into_iter()
                .filter(|ancestor| *ancestor != id)
            {
                if self.node(ancestor)?.style.overflow == Overflow::Clip {
                    let (clip, radii) = self.padding_clip(ancestor)?;
                    if !rounded_contains(clip, radii, position) {
                        clipped = true;
                        break;
                    }
                }
            }
            if !clipped {
                return Ok(Some(id));
            }
        }
        Ok(None)
    }

    fn z_eligible(&self, node: &Node) -> Result<bool, UiError> {
        if node.style.layout.position != Position::Static {
            return Ok(true);
        }
        if let Some(parent) = node.parent {
            return Ok(matches!(
                self.node(parent)?.style.layout.display,
                Display::Flex | Display::Grid
            ));
        }
        Ok(false)
    }
    fn items(&self, root: NodeId) -> Result<Vec<PaintItem>, UiError> {
        let mut items = Vec::new();
        let mut stack = Vec::new();
        for child in self.node(root)?.children.iter().rev() {
            stack.push((*child, false));
        }
        while let Some((id, positioned_ancestor)) = stack.pop() {
            let node = self.node(id)?;
            if node.hidden || node.style.layout.display == Display::None {
                continue;
            }
            let z_index = match node.style.z_index {
                Some(z) if self.z_eligible(node)? => Some(z),
                _ => None,
            };
            let context = node.style.opacity < 1.0 || z_index.is_some();
            let positioned = positioned_ancestor || node.style.layout.position != Position::Static;
            let z = z_index.unwrap_or(0);
            let phase = if context && z < 0 {
                0
            } else if context && z > 0 {
                3
            } else if context || positioned {
                2
            } else {
                1
            };
            items.push(PaintItem {
                id,
                context,
                phase,
                z,
                ordinal: items.len(),
            });
            if !context {
                for child in node.children.iter().rev() {
                    stack.push((*child, positioned));
                }
            }
        }
        items.sort_by_key(|item| (item.phase, item.z, item.ordinal));
        Ok(items)
    }
    fn flatten_context(&self, root: NodeId, order: &mut Vec<NodeId>) -> Result<(), UiError> {
        order.push(root);
        for item in self.items(root)? {
            if item.context {
                self.flatten_context(item.id, order)?;
            } else {
                order.push(item.id);
            }
        }
        Ok(())
    }
    fn paint_context(
        &self,
        frame: &mut PaintFrame,
        parent: SceneNodeId,
        id: NodeId,
        clip_from: Option<NodeId>,
        scale: f64,
    ) -> Result<(), UiError> {
        let mut scope = self.ancestor_clips(&mut frame.scene, parent, id, clip_from)?;
        if self.node(id)?.style.opacity < 1.0 {
            scope = append(
                &mut frame.scene,
                scope,
                NodeKind::Opacity(self.node(id)?.style.opacity),
            )?;
        }
        let mut focus_rings = Vec::new();
        self.paint_node(frame, scope, id, scale, &mut focus_rings)?;
        for item in self.items(id)? {
            if item.context {
                self.paint_context(frame, scope, item.id, Some(id), scale)?;
            } else {
                let clipped = self.ancestor_clips(&mut frame.scene, scope, item.id, Some(id))?;
                self.paint_node(frame, clipped, item.id, scale, &mut focus_rings)?;
            }
        }
        for node in focus_rings {
            let clipped = self.ancestor_clips(&mut frame.scene, scope, node, Some(id))?;
            self.paint_focus_ring(frame, clipped, node)?;
        }
        Ok(())
    }
    fn ancestor_clips(
        &self,
        scene: &mut Scene,
        mut parent: SceneNodeId,
        id: NodeId,
        clip_from: Option<NodeId>,
    ) -> Result<SceneNodeId, UiError> {
        let mut active = clip_from.is_none();
        for ancestor in self
            .path(id)?
            .into_iter()
            .filter(|ancestor| *ancestor != id)
        {
            active |= clip_from == Some(ancestor);
            if !active {
                continue;
            }
            if self.node(ancestor)?.style.overflow == Overflow::Clip {
                let (rect, radii) = self.padding_clip(ancestor)?;
                parent = append(
                    scene,
                    parent,
                    NodeKind::Clip(Clip::RoundedRect(rect, radii)),
                )?;
            }
        }
        Ok(parent)
    }
    fn paint_node(
        &self,
        frame: &mut PaintFrame,
        parent: SceneNodeId,
        id: NodeId,
        scale: f64,
        focus_rings: &mut Vec<NodeId>,
    ) -> Result<(), UiError> {
        let node = self.node(id)?;
        let rect = self.bounds(id)?;
        // Absolute logical origin is part of rect. A fractional ancestor
        // translation can change device glyph phase even with a clean local box.
        let text_key = node.text.as_ref().map(|_| (node.layout, scale));
        let draws = if let Some(painting) = &node.painting
            && painting.rect == rect
            && painting.text_key == text_key
        {
            painting.emit(&mut frame.scene, parent, node)?
        } else {
            let painting = Box::new(NodePainting::prepare(node, rect, text_key)?);
            let draws = painting.emit(&mut frame.scene, parent, node)?;
            frame.prepared.push((id, painting));
            draws
        };
        frame.draws.insert(id, draws);
        if !self.editors.is_empty()
            && let Some(editor) = self.editors.get(&id)
        {
            let viewport = self.editor_viewport(id)?;
            frame.editors.insert(
                id,
                EditorDraws::mount(&mut frame.scene, parent, editor, viewport, scale)?,
            );
        }
        if node_focus_ring(node).is_some() {
            focus_rings.push(id);
        }
        Ok(())
    }

    fn paint_focus_ring(
        &self,
        frame: &mut PaintFrame,
        parent: SceneNodeId,
        id: NodeId,
    ) -> Result<(), UiError> {
        let node = self.node(id)?;
        let rect = self.bounds(id)?;
        let radii = normalized_radii(rect, node.style.border_radii);
        if let Some(ring) =
            node_focus_ring(node).and_then(|ring| focus_ring_shape(rect, radii, ring))
            && let Some(draws) = frame.draws.get_mut(&id)
        {
            draws.focus_ring = Some(append(&mut frame.scene, parent, NodeKind::Draw(ring))?);
        }
        Ok(())
    }
    pub(super) fn padding_clip(&self, id: NodeId) -> Result<(Rect, CornerRadii), UiError> {
        let node = self.node(id)?;
        let rect = self.bounds(id)?;
        let radii = normalized_radii(rect, node.style.border_radii);
        Ok(padding_box(rect, radii, node.style.layout.border_width))
    }
}

impl NodePainting {
    fn prepare(
        node: &Node,
        rect: Rect,
        text_key: Option<(LayoutBox, f64)>,
    ) -> Result<Self, UiError> {
        let radii = normalized_radii(rect, node.style.border_radii);
        let border_width = node.style.layout.border_width;
        let size = rect.size();
        let background_radii = (size.width() > 0.0 && size.height() > 0.0).then_some(radii);
        let width = border_width.min(size.width()).min(size.height());
        let border = (background_radii.is_some() && width > 0.0).then(|| BorderPainting {
            clip: Clip::RoundedRect(rect, radii),
            rect: inset(rect, width / 2.0),
            radii: reduce_radii(radii, width / 2.0),
            width,
        });
        let content = if node.content.is_empty() && node.text.is_none() {
            None
        } else {
            let origin = rect.origin();
            let clip = (node.style.overflow == Overflow::Clip).then(|| {
                let (inner, radii) = padding_box(rect, radii, border_width);
                Clip::RoundedRect(inner, radii)
            });
            let glyphs = match (&node.text, text_key) {
                (Some(text), Some((layout, scale))) => text.primitives(layout, origin, scale)?,
                _ => Vec::new(),
            };
            Some(ContentPainting {
                clip,
                transform: Affine::new([1.0, 0.0, 0.0, 1.0, origin.x(), origin.y()]),
                glyphs,
            })
        };
        Ok(Self {
            rect,
            text_key,
            background_radii,
            border,
            content,
        })
    }

    fn background(&self, color: Color) -> Option<Primitive> {
        self.background_radii
            .map(|radii| Primitive::Fill(self.rect, radii, Brush::Solid(color)))
    }

    fn focus_ring(&self, node: &Node) -> Option<Primitive> {
        focus_ring_shape(self.rect, self.background_radii?, node_focus_ring(node)?)
    }

    fn emit(
        &self,
        scene: &mut Scene,
        parent: SceneNodeId,
        node: &Node,
    ) -> Result<DrawHandles, UiError> {
        let mut draws = DrawHandles::default();
        if let Some(background) = self.background(node.style.background) {
            draws.background = Some(append(scene, parent, NodeKind::Draw(background))?);
        }
        if let Some(border) = &self.border {
            let clip = append(scene, parent, NodeKind::Clip(border.clip.clone()))?;
            let stroke = border.primitive(node.style.border_color);
            draws.border = Some(append(scene, clip, NodeKind::Draw(stroke))?);
        }
        if let Some(content) = &self.content {
            let scope = if let Some(clip) = &content.clip {
                append(scene, parent, NodeKind::Clip(clip.clone()))?
            } else {
                parent
            };
            let scope = append(scene, scope, NodeKind::Transform(content.transform))?;
            for primitive in &content.glyphs {
                append(scene, scope, NodeKind::Draw(primitive.clone()))?;
            }
            for primitive in &node.content {
                draws
                    .custom
                    .push(append(scene, scope, NodeKind::Draw(primitive.clone()))?);
            }
        }
        Ok(draws)
    }
}

impl BorderPainting {
    fn primitive(&self, color: Color) -> Primitive {
        Primitive::Stroke(self.rect, self.radii, self.width, Brush::Solid(color))
    }
}

fn node_focus_ring(node: &Node) -> Option<FocusRing> {
    node.theme.as_ref()?.focus_ring
}

fn focus_ring_shape(rect: Rect, radii: CornerRadii, ring: FocusRing) -> Option<Primitive> {
    let size = rect.size();
    let width = ring.width.min(size.width() / 2.0).min(size.height() / 2.0);
    (width > 0.0).then(|| {
        let inset_width = width / 2.0;
        Primitive::Stroke(
            inset(rect, inset_width),
            reduce_radii(radii, inset_width),
            width,
            Brush::Solid(ring.color),
        )
    })
}

fn padding_box(rect: Rect, radii: CornerRadii, width: f64) -> (Rect, CornerRadii) {
    let inner = inset(rect, width);
    (inner, normalized_radii(inner, reduce_radii(radii, width)))
}

fn append(scene: &mut Scene, parent: SceneNodeId, kind: NodeKind) -> Result<SceneNodeId, UiError> {
    let index = scene.children(Some(parent))?.len();
    Ok(scene.insert(Some(parent), index, kind)?)
}
fn inset(rect: Rect, width: f64) -> Rect {
    let [left, top, right, bottom] = rect.edges();
    let horizontal = width.min((right - left).max(0.0) / 2.0);
    let vertical = width.min((bottom - top).max(0.0) / 2.0);
    Rect::new(
        left + horizontal,
        top + vertical,
        right - horizontal,
        bottom - vertical,
    )
}
fn reduce_radii(radii: CornerRadii, value: f64) -> CornerRadii {
    let [a, b, c, d] = radii.values().map(|radius| (radius - value).max(0.0));
    CornerRadii::new(a, b, c, d)
}
fn normalized_radii(rect: Rect, radii: CornerRadii) -> CornerRadii {
    let [a, b, c, d] = radii.values();
    let width = rect.size().width().max(0.0);
    let height = rect.size().height().max(0.0);
    let mut factor: f64 = 1.0;
    for (length, sum) in [
        (width, a + b),
        (height, b + c),
        (width, c + d),
        (height, d + a),
    ] {
        if sum > 0.0 {
            factor = factor.min(length / sum);
        }
    }
    CornerRadii::new(a * factor, b * factor, c * factor, d * factor)
}
fn rounded_contains(rect: Rect, radii: CornerRadii, point: Point) -> bool {
    let [left, top, right, bottom] = rect.edges();
    let x = point.x();
    let y = point.y();
    if x < left || x >= right || y < top || y >= bottom {
        return false;
    }
    let [tl, tr, br, bl] = radii.values();
    for (cx, cy, radius, in_corner) in [
        (left + tl, top + tl, tl, x < left + tl && y < top + tl),
        (right - tr, top + tr, tr, x > right - tr && y < top + tr),
        (
            right - br,
            bottom - br,
            br,
            x > right - br && y > bottom - br,
        ),
        (left + bl, bottom - bl, bl, x < left + bl && y > bottom - bl),
    ] {
        if in_corner && radius > 0.0 && (x - cx).powi(2) + (y - cy).powi(2) > radius.powi(2) {
            return false;
        }
    }
    true
}
