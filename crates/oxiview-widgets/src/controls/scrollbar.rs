//! Scrollbar interaction, thumb geometry, and viewport synchronization.

use super::{Handle, px, semantic, value};
use oxiview_base::{Point, Size};
use oxiview_core::{
    Action, FocusPolicy, KeyBinding, KeyChord, Keymap, NodeId, PointerEventKind, UiError, UiTree,
    View, node,
};
use oxiview_platform::{KeyCode, KeyModifiers, LogicalKey, PointerButton};
use oxiview_style::{Edges, LayoutPatch, LayoutStyle, Length, Position, Style, StylePatch};
use std::{cell::Cell, rc::Rc};

/// The axis of a scrollbar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollbarAxis {
    /// Left to right.
    Horizontal,
    /// Top to bottom.
    Vertical,
}

impl ScrollbarAxis {
    fn coordinate(self, point: Point) -> f64 {
        match self {
            Self::Horizontal => point.x(),
            Self::Vertical => point.y(),
        }
    }

    fn length(self, size: Size) -> f64 {
        match self {
            Self::Horizontal => size.width(),
            Self::Vertical => size.height(),
        }
    }
}

/// Current geometry of the viewport represented by a scrollbar, in logical points.
#[derive(Clone, Copy, Default, PartialEq)]
pub struct ScrollbarMetrics {
    /// Visible extent on the selected axis.
    pub viewport: f64,
    /// Maximum legal offset.
    pub maximum: f64,
    /// Current offset.
    pub offset: f64,
}

impl ScrollbarMetrics {
    fn thumb(self, track: f64) -> (f64, f64) {
        if self.maximum <= 0. || track <= 0. {
            return (0., 0.);
        }
        let length =
            (track * self.viewport / (self.viewport + self.maximum)).clamp(track.min(22.), track);
        let start = (track - length) * (self.offset / self.maximum).clamp(0., 1.);
        (start, length)
    }
}

/// Window-thread scrollbar handle. It proposes offsets and the viewport remains authoritative.
#[derive(Clone, Default)]
pub struct Scrollbar {
    track: Handle,
    thumb: Handle,
    metrics: Rc<Cell<ScrollbarMetrics>>,
    rendered: Rc<Cell<Option<(ScrollbarMetrics, f64)>>>,
}

impl Scrollbar {
    /// Returns the retained track and capture target.
    pub fn node(&self) -> Option<NodeId> {
        self.track.node()
    }

    /// Returns the retained thumb.
    pub fn thumb_node(&self) -> Option<NodeId> {
        self.thumb.node()
    }

    /// Builds a scrollbar with thumb capture, track paging and keyboard Actions.
    pub fn view(
        &self,
        axis: ScrollbarAxis,
        name: &str,
        callback: impl Fn(f64) + 'static,
    ) -> Result<View, UiError> {
        let callback = Rc::new(callback);
        let pointer = callback.clone();
        let metrics = self.metrics.clone();
        let mut drag = None;
        let (width, height) = match axis {
            ScrollbarAxis::Horizontal => (Length::Auto, px(12.)),
            ScrollbarAxis::Vertical => (px(12.), Length::Auto),
        };
        let mut track = node()
            .theme_type("scrollbar")
            .focusable(FocusPolicy::Tab)
            .semantics(semantic("scrollbar", name, "0%"))
            .style_patch(StylePatch {
                layout: LayoutPatch {
                    width: Some(width),
                    height: Some(height),
                    position: Some(Position::Relative),
                    flex_shrink: Some(0.),
                    ..Default::default()
                },
                ..Default::default()
            })
            .child(
                self.thumb.mount(
                    node().style(thumb_geometry(axis, 0., 0.)).child(
                        node()
                            .theme_type("scrollbar-thumb")
                            .style_patch(StylePatch {
                                hit_test: Some(false),
                                layout: LayoutPatch {
                                    width: Some(Length::Percent(1.)),
                                    height: Some(Length::Percent(1.)),
                                    ..Default::default()
                                },
                                ..Default::default()
                            }),
                    ),
                ),
            )
            .on_pointer(move |event, tree, commands| {
                let metrics = metrics.get();
                if metrics.maximum <= 0. {
                    drag = None;
                    return;
                }
                let Ok(bounds) = tree.bounds(event.current_target) else {
                    return;
                };
                let length = axis.length(bounds.size());
                let (start, thumb) = metrics.thumb(length);
                let coordinate = axis.coordinate(event.local_position);
                let travel = length - thumb;
                match event.kind {
                    PointerEventKind::Down if event.button == Some(PointerButton::Primary) => {
                        if coordinate >= start && coordinate <= start + thumb {
                            drag = Some(coordinate - start);
                            commands.capture(event.current_target);
                        } else {
                            let direction = if coordinate < start { -1. } else { 1. };
                            pointer(
                                (metrics.offset + direction * metrics.viewport)
                                    .clamp(0., metrics.maximum),
                            );
                        }
                    }
                    PointerEventKind::Move if travel > 0. => {
                        if let Some(grab) = drag {
                            pointer(((coordinate - grab) / travel).clamp(0., 1.) * metrics.maximum);
                        }
                    }
                    PointerEventKind::Up if event.button == Some(PointerButton::Primary) => {
                        if drag.take().is_some() {
                            commands.release_capture(event.current_target);
                        }
                    }
                    PointerEventKind::Cancel => drag = None,
                    _ => {}
                }
            });
        let arrows = match axis {
            ScrollbarAxis::Horizontal => [KeyCode::ArrowLeft, KeyCode::ArrowRight],
            ScrollbarAxis::Vertical => [KeyCode::ArrowUp, KeyCode::ArrowDown],
        };
        let keys = [
            (arrows[0], "oxiview.scrollbar.back", "Scroll back"),
            (arrows[1], "oxiview.scrollbar.forward", "Scroll forward"),
            (
                KeyCode::PageUp,
                "oxiview.scrollbar.page-back",
                "Scroll back one page",
            ),
            (
                KeyCode::PageDown,
                "oxiview.scrollbar.page-forward",
                "Scroll forward one page",
            ),
            (
                KeyCode::Home,
                "oxiview.scrollbar.start",
                "Scroll to the start",
            ),
            (KeyCode::End, "oxiview.scrollbar.end", "Scroll to the end"),
        ];
        let bindings = keys.map(|(key, id, _)| {
            KeyBinding::new(
                id,
                [KeyChord::logical(
                    LogicalKey::Named(key),
                    KeyModifiers::default(),
                )],
            )
            .repeatable()
        });
        for (index, (_, id, description)) in keys.into_iter().enumerate() {
            let metrics = self.metrics.clone();
            let callback = callback.clone();
            track = track.on_action(Action::new(id, description), move |_, _, _| {
                let metrics = metrics.get();
                let requested = match index {
                    0 => metrics.offset - 32.,
                    1 => metrics.offset + 32.,
                    2 => metrics.offset - metrics.viewport,
                    3 => metrics.offset + metrics.viewport,
                    4 => 0.,
                    _ => metrics.maximum,
                };
                callback(requested.clamp(0., metrics.maximum));
            });
        }
        let keys = Keymap::new(bindings).map_err(|error| UiError::View(error.to_string()))?;
        let rendered = self.rendered.clone();
        Ok(self.track.mount(
            track
                .keymap(keys)
                .disabled(true)
                .on_mount(move |_| rendered.set(None)),
        ))
    }

    /// Synchronizes the thumb and semantic value with a laid-out viewport.
    pub fn update(
        &self,
        tree: &mut UiTree,
        axis: ScrollbarAxis,
        metrics: ScrollbarMetrics,
    ) -> Result<bool, UiError> {
        let (Some(track), Some(thumb)) = (self.node(), self.thumb_node()) else {
            return Ok(false);
        };
        let metrics = ScrollbarMetrics {
            viewport: nonnegative(metrics.viewport),
            maximum: nonnegative(metrics.maximum),
            offset: nonnegative(metrics.offset).min(nonnegative(metrics.maximum)),
        };
        let length = axis.length(tree.bounds(track)?.size());
        if self.rendered.get() == Some((metrics, length)) {
            return Ok(false);
        }
        let (start, extent) = metrics.thumb(length);
        let changed = tree.set_style(thumb, thumb_geometry(axis, start, extent))?;
        tree.set_disabled(track, metrics.maximum == 0. || length == 0.)?;
        let percent = if metrics.maximum > 0. {
            (metrics.offset / metrics.maximum) * 100.
        } else {
            0.
        };
        value(tree, track, &format!("{percent:.0}%"))?;
        self.metrics.set(metrics);
        self.rendered.set(Some((metrics, length)));
        Ok(changed)
    }
}

fn nonnegative(value: f64) -> f64 {
    if value.is_finite() { value.max(0.) } else { 0. }
}

fn thumb_geometry(axis: ScrollbarAxis, start: f64, length: f64) -> Style {
    let mut inset = Edges::all(Length::Auto);
    let (width, height) = match axis {
        ScrollbarAxis::Horizontal => {
            inset.left = px(start);
            inset.top = px(2.);
            (length, 8.)
        }
        ScrollbarAxis::Vertical => {
            inset.left = px(2.);
            inset.top = px(start);
            (8., length)
        }
    };
    Style {
        hit_test: false,
        layout: LayoutStyle {
            position: Position::Absolute,
            width: px(width),
            height: px(height),
            inset,
            ..Default::default()
        },
        ..Default::default()
    }
}
