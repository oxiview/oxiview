use super::{NodeId, UiError, UiTree};
use crate::PaintedScene;
use oxiview_base::{Affine, CornerRadii, Point, Rect, Size};
use oxiview_scene::{Brush, Clip, NodeId as SceneNodeId, NodeKind, Primitive, Scene};
use oxiview_style::{Display, Overflow, Position};

struct PaintItem {
    id: NodeId,
    context: bool,
    phase: u8,
    z: i32,
    ordinal: usize,
}

impl UiTree {
    /// Prepares a complete scene with one logical-to-physical root transform.
    /// Clean geometry and painting content are reused across system redraws.
    pub fn paint(&mut self, viewport: Size, scale: f64) -> Result<PaintedScene, UiError> {
        if !scale.is_finite() || scale <= 0.0 {
            return Err(UiError::InvalidViewport);
        }
        self.layout(viewport)?;
        if !self.paint_dirty
            && let Some((scene, root, old_scale)) = &self.paint_cache
            && *old_scale == scale
        {
            let painted = PaintedScene {
                scene: scene.clone(),
                root: *root,
            };
            self.diagnostics.scene_cache_hits = self.diagnostics.scene_cache_hits.saturating_add(1);
            return Ok(painted);
        }
        let _span = tracing::info_span!("ui_paint", nodes = self.len()).entered();
        let mut scene = Scene::new();
        let root = scene.insert(
            None,
            0,
            NodeKind::Transform(Affine::new([scale, 0.0, 0.0, scale, 0.0, 0.0])),
        )?;
        if !self.suppressed(self.root)? {
            self.paint_context(&mut scene, root, self.root, None, scale)?;
        }
        self.paint_cache = Some((scene.clone(), root, scale));
        self.paint_dirty = false;
        self.diagnostics.scene_builds = self.diagnostics.scene_builds.saturating_add(1);
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

    fn z_eligible(&self, id: NodeId) -> Result<bool, UiError> {
        let node = self.node(id)?;
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
    fn context(&self, id: NodeId) -> Result<bool, UiError> {
        let style = &self.node(id)?.style;
        Ok(id == self.root
            || style.opacity < 1.0
            || (style.z_index.is_some() && self.z_eligible(id)?))
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
            let context = self.context(id)?;
            let positioned = positioned_ancestor || node.style.layout.position != Position::Static;
            let z = if self.z_eligible(id)? {
                node.style.z_index.unwrap_or(0)
            } else {
                0
            };
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
                z: if context { z } else { 0 },
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
        scene: &mut Scene,
        parent: SceneNodeId,
        id: NodeId,
        clip_from: Option<NodeId>,
        scale: f64,
    ) -> Result<(), UiError> {
        let mut scope = self.ancestor_clips(scene, parent, id, clip_from)?;
        if self.node(id)?.style.opacity < 1.0 {
            scope = append(
                scene,
                scope,
                NodeKind::Opacity(self.node(id)?.style.opacity),
            )?;
        }
        self.paint_node(scene, scope, id, scale)?;
        for item in self.items(id)? {
            if item.context {
                self.paint_context(scene, scope, item.id, Some(id), scale)?;
            } else {
                let clipped = self.ancestor_clips(scene, scope, item.id, Some(id))?;
                self.paint_node(scene, clipped, item.id, scale)?;
            }
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
        scene: &mut Scene,
        parent: SceneNodeId,
        id: NodeId,
        scale: f64,
    ) -> Result<(), UiError> {
        let node = self.node(id)?;
        let rect = self.bounds(id)?;
        let radii = normalized_radii(rect, node.style.border_radii);
        if rect.size().width() > 0.0 && rect.size().height() > 0.0 {
            append(
                scene,
                parent,
                NodeKind::Draw(Primitive::Fill(
                    rect,
                    radii,
                    Brush::Solid(node.style.background),
                )),
            )?;
            let width = node
                .style
                .layout
                .border_width
                .min(rect.size().width())
                .min(rect.size().height());
            if width > 0.0 {
                let clip = append(
                    scene,
                    parent,
                    NodeKind::Clip(Clip::RoundedRect(rect, radii)),
                )?;
                let stroke_rect = inset(rect, width / 2.0);
                let center_radii = reduce_radii(radii, width / 2.0);
                append(
                    scene,
                    clip,
                    NodeKind::Draw(Primitive::Stroke(
                        stroke_rect,
                        center_radii,
                        width,
                        Brush::Solid(node.style.border_color),
                    )),
                )?;
            }
        }
        if !node.content.is_empty() || node.text.is_some() {
            let mut content_parent = parent;
            if node.style.overflow == Overflow::Clip {
                let (clip, radii) = self.padding_clip(id)?;
                content_parent = append(
                    scene,
                    content_parent,
                    NodeKind::Clip(Clip::RoundedRect(clip, radii)),
                )?;
            }
            let origin = rect.origin();
            let content_parent = append(
                scene,
                content_parent,
                NodeKind::Transform(Affine::new([1.0, 0.0, 0.0, 1.0, origin.x(), origin.y()])),
            )?;
            if let Some(text) = &node.text {
                for primitive in text.primitives(node.layout, origin, scale)? {
                    append(scene, content_parent, NodeKind::Draw(primitive))?;
                }
            }
            for primitive in &node.content {
                append(scene, content_parent, NodeKind::Draw(primitive.clone()))?;
            }
        }
        Ok(())
    }
    fn padding_clip(&self, id: NodeId) -> Result<(Rect, CornerRadii), UiError> {
        let node = self.node(id)?;
        let rect = self.bounds(id)?;
        let radius = normalized_radii(rect, node.style.border_radii);
        let inner = inset(rect, node.style.layout.border_width);
        Ok((
            inner,
            normalized_radii(inner, reduce_radii(radius, node.style.layout.border_width)),
        ))
    }
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
