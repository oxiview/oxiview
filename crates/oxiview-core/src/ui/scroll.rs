//! Logical viewport geometry and time-based scroll motion.

use super::{Node, NodeId, UiError, UiTree};
use oxiview_base::{Point, Rect, Size};
use oxiview_layout::LayoutBox;
use oxiview_platform::{ScrollMomentum, ScrollPhase};
use oxiview_style::Overflow;
use std::time::Duration;

const DECAY_PER_SECOND: f64 = 8.0;
const MIN_SPEED: f64 = 8.0;
const MAX_SPEED: f64 = 4_000.0;
const RELEASE_TIMEOUT: Duration = Duration::from_millis(80);

pub(super) struct Viewport {
    extent: Size,
    pub(super) offset: Point,
}

impl Viewport {
    fn max_offset(&self, size: Size, border_width: f64) -> Point {
        // Match the padding-box inset used for paint and hit testing.
        let width = (size.width() - 2.0 * border_width).max(0.0);
        let height = (size.height() - 2.0 * border_width).max(0.0);
        Point::new(
            (self.extent.width() - width).max(0.0),
            (self.extent.height() - height).max(0.0),
        )
    }
}

impl Node {
    pub(super) fn clips_descendants(&self) -> bool {
        self.scroll.is_some() || self.style.overflow == Overflow::Clip
    }

    fn scroll_viewport(&self) -> Result<&Viewport, UiError> {
        self.scroll.as_deref().ok_or(UiError::ScrollUnavailable)
    }
}

#[derive(Default)]
pub(super) struct ScrollInput {
    motion: Motion,
    reduced_motion: bool,
    pub(super) viewports: usize,
    pub(super) hover_pending: bool,
}

#[derive(Default)]
enum Motion {
    #[default]
    Idle,
    Direct(Direct),
    AwaitingNative(Vec<NodeId>),
    Native(Vec<NodeId>),
    Inertia(Inertia),
}

struct Direct {
    path: Vec<NodeId>,
    producer: ScrollMomentum,
    started: Duration,
    last_motion: Option<Duration>,
    velocity: Point,
}

impl Direct {
    fn sample(&mut self, delta: Point, timestamp: Duration) {
        if self.producer != ScrollMomentum::Framework || delta == Point::default() {
            return;
        }
        let last = self.last_motion.unwrap_or(self.started);
        let Some(elapsed) = timestamp.checked_sub(last) else {
            return;
        };
        if !elapsed.is_zero() {
            let seconds = elapsed.as_secs_f64();
            self.velocity = Point::new(
                (delta.x() / seconds).clamp(-MAX_SPEED, MAX_SPEED),
                (delta.y() / seconds).clamp(-MAX_SPEED, MAX_SPEED),
            );
        }
        self.last_motion = Some(timestamp);
    }
}

struct Inertia {
    path: Vec<NodeId>,
    timestamp: Duration,
    velocity: Point,
}

impl UiTree {
    /// Configures a retained node as a viewport and sets its logical content
    /// width and height. Descendants scroll within its padding-box clip. The
    /// node's own drawing stays fixed. An extent may describe unmounted content.
    ///
    /// Returns whether the extent changed. Shrinking it clamps the offset.
    pub fn set_scroll_content_extent(&mut self, id: NodeId, extent: Size) -> Result<bool, UiError> {
        if ![extent.width(), extent.height()]
            .into_iter()
            .all(|length| length.is_finite() && length >= 0.0)
        {
            return Err(UiError::InvalidScrollGeometry);
        }
        let node = self.node_mut(id)?;
        if let Some(viewport) = &mut node.scroll {
            if viewport.extent == extent {
                return Ok(false);
            }
            viewport.extent = extent;
            let max = viewport.max_offset(node.layout.size, node.style.layout.border_width);
            let clamped = clamp(viewport.offset, max);
            if viewport.offset != clamped {
                viewport.offset = clamped;
                self.scroll_geometry_changed();
            }
        } else {
            node.scroll = Some(Box::new(Viewport {
                extent,
                offset: Point::default(),
            }));
            node.painting = None;
            self.scroll_input.viewports += 1;
            self.scroll_geometry_changed();
        }
        Ok(true)
    }

    /// Returns the viewport's current logical offset from its content origin.
    pub fn scroll_offset(&self, id: NodeId) -> Result<Point, UiError> {
        Ok(self.node(id)?.scroll_viewport()?.offset)
    }

    /// Returns the maximum logical offset for each axis at the committed size.
    pub fn scroll_max_offset(&self, id: NodeId) -> Result<Point, UiError> {
        let node = self.node(id)?;
        Ok(node
            .scroll_viewport()?
            .max_offset(node.layout.size, node.style.layout.border_width))
    }

    /// Returns the viewport's padding box in translated logical window
    /// coordinates, before intersecting it with enclosing clips.
    pub fn scroll_viewport_bounds(&self, id: NodeId) -> Result<Rect, UiError> {
        self.node(id)?.scroll_viewport()?;
        Ok(self.padding_clip(id)?.0)
    }

    /// Moves a viewport to a logical offset, clamped to the committed extent.
    /// Returns whether its position changed and interrupts an ongoing gesture.
    /// Flush UI updates to deliver any pending stationary hover notifications.
    pub fn set_scroll_offset(&mut self, id: NodeId, offset: Point) -> Result<bool, UiError> {
        if !finite_point(offset) {
            return Err(UiError::InvalidScrollGeometry);
        }
        let offset = clamp(offset, self.scroll_max_offset(id)?);
        self.interrupt_scroll();
        self.apply_scroll_offset(id, offset)
    }

    /// Scrolls enclosing viewports, innermost first, to expose a retained node's
    /// committed border box. Returns whether any offset changed.
    pub fn reveal_node(&mut self, id: NodeId) -> Result<bool, UiError> {
        if self.suppressed(id)? {
            return Ok(false);
        }
        let path = self.path(id)?;
        let mut visible = None;
        let mut changed = false;
        for ancestor in path.into_iter().rev().skip(1) {
            let node = self.node(ancestor)?;
            if !node.clips_descendants() {
                continue;
            }
            let [mut left, mut top, mut right, mut bottom] = match visible {
                Some(rect) => rect,
                None => self.bounds(id)?.edges(),
            };
            let [clip_left, clip_top, clip_right, clip_bottom] =
                self.padding_clip(ancestor)?.0.edges();
            if let Some(viewport) = &node.scroll {
                let before = viewport.offset;
                let delta = Point::new(
                    reveal_axis(left, right, clip_left, clip_right),
                    reveal_axis(top, bottom, clip_top, clip_bottom),
                );
                if self.consume_scroll(&[ancestor], delta)? {
                    let offset = self.scroll_offset(ancestor)?;
                    let x = offset.x() - before.x();
                    let y = offset.y() - before.y();
                    left -= x;
                    right -= x;
                    top -= y;
                    bottom -= y;
                    changed = true;
                }
            }
            visible = Some([
                left.max(clip_left),
                top.max(clip_top),
                right.min(clip_right),
                bottom.min(clip_bottom),
            ]);
            if left >= clip_right || right <= clip_left || top >= clip_bottom || bottom <= clip_top
            {
                break;
            }
        }
        if changed {
            self.interrupt_scroll();
        }
        Ok(changed)
    }

    /// Whether framework momentum needs another frame opportunity. Native
    /// momentum advances when the platform supplies its next scroll event.
    pub fn scroll_animation_active(&self) -> bool {
        matches!(self.scroll_input.motion, Motion::Inertia(_))
    }

    pub(super) fn commit_scroll_layout(
        &mut self,
        id: NodeId,
        layout: LayoutBox,
    ) -> Result<(), UiError> {
        let node = self.node_mut(id)?;
        node.layout = layout;
        if let Some(viewport) = &mut node.scroll {
            let max = viewport.max_offset(layout.size, node.style.layout.border_width);
            let offset = clamp(viewport.offset, max);
            if offset != viewport.offset {
                viewport.offset = offset;
                self.scroll_geometry_changed();
            }
        }
        Ok(())
    }

    pub(super) fn interrupt_scroll(&mut self) {
        self.scroll_input.motion = Motion::Idle;
    }

    pub(super) fn set_scroll_reduced_motion(&mut self, reduced: bool) {
        self.scroll_input.reduced_motion = reduced;
        if reduced && !matches!(self.scroll_input.motion, Motion::Direct(_)) {
            self.interrupt_scroll();
        }
    }

    pub(super) fn wheel_scroll(&mut self, position: Point, delta: Point) -> Result<bool, UiError> {
        if !self.accepts_scroll(position, delta) {
            return Ok(false);
        }
        self.interrupt_scroll();
        if delta == Point::default() {
            return Ok(false);
        }
        let path = self.scroll_path_at(position)?;
        self.consume_scroll(&path, delta)
    }

    pub(super) fn phased_scroll(
        &mut self,
        position: Point,
        delta: Point,
        phase: ScrollPhase,
        timestamp: Duration,
    ) -> Result<bool, UiError> {
        if !self.accepts_scroll(position, delta) {
            return Ok(false);
        }

        let mut direct_pointer = false;
        let motion = std::mem::take(&mut self.scroll_input.motion);
        self.scroll_input.motion = match (phase, motion) {
            (ScrollPhase::Begin(producer), _) => {
                let path = self.scroll_path_at(position)?;
                direct_pointer = !path.is_empty();
                self.consume_scroll(&path, delta)?;
                Motion::Direct(Direct {
                    path,
                    producer,
                    started: timestamp,
                    last_motion: None,
                    velocity: Point::default(),
                })
            }
            (ScrollPhase::Update, Motion::Direct(mut direct)) => {
                direct_pointer = !direct.path.is_empty();
                self.consume_scroll(&direct.path, delta)?;
                direct.sample(delta, timestamp);
                Motion::Direct(direct)
            }
            (ScrollPhase::End, Motion::Direct(mut direct)) => {
                direct_pointer = !direct.path.is_empty();
                self.consume_scroll(&direct.path, delta)?;
                direct.sample(delta, timestamp);
                self.release_scroll(direct, timestamp)?
            }
            (ScrollPhase::MomentumBegin, Motion::AwaitingNative(path)) => {
                self.consume_scroll(&path, delta)?;
                Motion::Native(path)
            }
            (ScrollPhase::MomentumUpdate, Motion::Native(path)) => {
                self.consume_scroll(&path, delta)?;
                Motion::Native(path)
            }
            (ScrollPhase::MomentumEnd, Motion::Native(path)) => {
                self.consume_scroll(&path, delta)?;
                Motion::Idle
            }
            (ScrollPhase::Cancel, _) => Motion::Idle,
            (_, motion) => motion,
        };
        // Direct input updates the pointer used by stationary hover. Native
        // momentum may carry a captured position from earlier in the gesture.
        Ok(direct_pointer)
    }

    fn release_scroll(&self, direct: Direct, timestamp: Duration) -> Result<Motion, UiError> {
        if self.scroll_input.reduced_motion {
            return Ok(Motion::Idle);
        }
        if direct.producer == ScrollMomentum::Native {
            return Ok(Motion::AwaitingNative(direct.path));
        }
        if direct.producer != ScrollMomentum::Framework
            || !direct.last_motion.is_some_and(|last| {
                timestamp
                    .checked_sub(last)
                    .is_some_and(|gap| gap < RELEASE_TIMEOUT)
            })
        {
            return Ok(Motion::Idle);
        }
        let velocity = self.scrollable_velocity(&direct.path, direct.velocity)?;
        if velocity == Point::default() {
            return Ok(Motion::Idle);
        }
        Ok(Motion::Inertia(Inertia {
            path: direct.path,
            timestamp,
            velocity,
        }))
    }

    pub(super) fn advance_scroll_motion(&mut self, timestamp: Duration) -> Result<(), UiError> {
        let Motion::Inertia(motion) = &mut self.scroll_input.motion else {
            return Ok(());
        };
        if timestamp <= motion.timestamp {
            return Ok(());
        }
        let Motion::Inertia(mut motion) = std::mem::take(&mut self.scroll_input.motion) else {
            return Ok(());
        };
        let seconds = timestamp.saturating_sub(motion.timestamp).as_secs_f64();
        let (x, vx) = decay(motion.velocity.x(), seconds);
        let (y, vy) = decay(motion.velocity.y(), seconds);
        if self.consume_scroll(&motion.path, Point::new(x, y))? {
            motion.timestamp = timestamp;
            motion.velocity = self.scrollable_velocity(&motion.path, Point::new(vx, vy))?;
            if motion.velocity != Point::default() {
                self.scroll_input.motion = Motion::Inertia(motion);
            }
        }
        Ok(())
    }

    fn accepts_scroll(&self, position: Point, delta: Point) -> bool {
        self.accepts_input()
            && self.capture_holder().is_none()
            && finite_point(position)
            && finite_point(delta)
    }

    fn scroll_path_at(&self, position: Point) -> Result<Vec<NodeId>, UiError> {
        if position.x() < 0.0
            || position.y() < 0.0
            || position.x() >= self.viewport.width()
            || position.y() >= self.viewport.height()
        {
            return Ok(Vec::new());
        }
        match self.hit_test(position)? {
            Some(target) => self.scroll_ancestors(target),
            None => Ok(Vec::new()),
        }
    }

    fn scroll_ancestors(&self, target: NodeId) -> Result<Vec<NodeId>, UiError> {
        let mut path = Vec::new();
        for node in self.ancestors(target) {
            let node = node?;
            if node.scroll.is_some() {
                path.push(node.id);
            }
        }
        Ok(path)
    }

    fn scroll_available(&self, id: NodeId) -> Result<bool, UiError> {
        // A latched path may outlive a row, a viewport, or the viewport's scope.
        Ok(self.contains(id) && self.interaction_available(id)?)
    }

    fn consume_scroll(&mut self, path: &[NodeId], mut delta: Point) -> Result<bool, UiError> {
        let mut changed = false;
        for &id in path {
            if delta == Point::default() {
                break;
            }
            if !self.scroll_available(id)? {
                continue;
            }
            let offset = self.scroll_offset(id)?;
            let max = self.scroll_max_offset(id)?;
            let (x, remaining_x) = consume_axis(offset.x(), max.x(), delta.x());
            let (y, remaining_y) = consume_axis(offset.y(), max.y(), delta.y());
            changed |= self.apply_scroll_offset(id, Point::new(x, y))?;
            delta = Point::new(remaining_x, remaining_y);
        }
        Ok(changed)
    }

    fn scrollable_velocity(&self, path: &[NodeId], velocity: Point) -> Result<Point, UiError> {
        let mut active_x = false;
        let mut active_y = false;
        for &id in path {
            if !self.scroll_available(id)? {
                continue;
            }
            let offset = self.scroll_offset(id)?;
            let max = self.scroll_max_offset(id)?;
            active_x |= can_move(offset.x(), max.x(), velocity.x());
            active_y |= can_move(offset.y(), max.y(), velocity.y());
            if active_x && active_y {
                break;
            }
        }
        Ok(Point::new(
            if active_x { velocity.x() } else { 0.0 },
            if active_y { velocity.y() } else { 0.0 },
        ))
    }

    fn apply_scroll_offset(&mut self, id: NodeId, offset: Point) -> Result<bool, UiError> {
        let viewport = self
            .node_mut(id)?
            .scroll
            .as_mut()
            .ok_or(UiError::ScrollUnavailable)?;
        if viewport.offset == offset {
            return Ok(false);
        }
        viewport.offset = offset;
        self.scroll_geometry_changed();
        Ok(true)
    }

    fn scroll_geometry_changed(&mut self) {
        self.dirty_paint();
        self.scroll_input.hover_pending = true;
    }
}

fn finite_point(point: Point) -> bool {
    point.x().is_finite() && point.y().is_finite()
}

fn clamp(offset: Point, max: Point) -> Point {
    Point::new(
        offset.x().clamp(0.0, max.x()),
        offset.y().clamp(0.0, max.y()),
    )
}

fn consume_axis(offset: f64, max: f64, delta: f64) -> (f64, f64) {
    let to_end = max - offset;
    if delta >= to_end {
        (max, delta - to_end)
    } else if delta <= -offset {
        (0.0, delta + offset)
    } else {
        // The viewport has room for the entire update. Subtracting absolute
        // offsets here would pass floating-point rounding noise to its parent.
        ((offset + delta).clamp(0.0, max), 0.0)
    }
}

fn can_move(offset: f64, max: f64, velocity: f64) -> bool {
    (velocity > MIN_SPEED && offset < max) || (velocity < -MIN_SPEED && offset > 0.0)
}

fn decay(velocity: f64, seconds: f64) -> (f64, f64) {
    if velocity == 0.0 {
        return (0.0, 0.0);
    }
    let until_stop = (velocity.abs() / MIN_SPEED).ln() / DECAY_PER_SECOND;
    let exponent = -DECAY_PER_SECOND * seconds.min(until_stop);
    let distance = -velocity * exponent.exp_m1() / DECAY_PER_SECOND;
    let next = if seconds < until_stop {
        velocity * exponent.exp()
    } else {
        0.0
    };
    (distance, next)
}

fn reveal_axis(low: f64, high: f64, clip_low: f64, clip_high: f64) -> f64 {
    if clip_low >= clip_high {
        return 0.0;
    }
    if high - low > clip_high - clip_low {
        if low > clip_low {
            low - clip_low
        } else {
            (high - clip_high).min(0.0)
        }
    } else if low < clip_low {
        low - clip_low
    } else {
        (high - clip_high).max(0.0)
    }
}
