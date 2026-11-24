//! Themed retained controls built from the same Actions, focus and pointer routing as a View.
//! Install [`CONTROL_THEME`] or supply the widget rules in your application theme.

mod scrollbar;

pub use scrollbar::{Scrollbar, ScrollbarAxis, ScrollbarMetrics};

use oxiview_base::{BezierPath, Color, CornerRadii, PathCommand, Point, Rect};
use oxiview_core::{
    Action, Editable, FocusPolicy, KeyBinding, KeyChord, Keymap, NodeId, NodeSemantics, TextStyle,
    TextSystem, UiError, UiTree, View, editable_text, node, text,
};
use oxiview_platform::{KeyCode, KeyModifiers, LogicalKey};
use oxiview_scene::{Brush, FillRule, Primitive};
use oxiview_style::{
    Alignment, ContentAlignment, Edges, LayoutPatch, LayoutStyle, Length, Overflow, Style,
    StylePatch,
};
use oxiview_text::{EditMode, Editor};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

/// Theme rules for the widget types supplied by this module.
pub const CONTROL_THEME: &str = include_str!("controls/theme.toml");

const ACTIVATE: &str = "oxiview.control.activate";

fn px(value: f64) -> Length {
    Length::Points(value)
}

fn rgb(value: u32) -> Color {
    let [_, r, g, b] = value.to_be_bytes();
    Color::from_rgba8(r, g, b, 255)
}

fn label(system: Rc<RefCell<TextSystem>>, value: &str, typography: TextStyle) -> View {
    text(system, value, typography).style(Style {
        hit_test: false,
        ..Style::default()
    })
}

#[derive(Clone, Default)]
struct Handle(Rc<Cell<Option<NodeId>>>);

impl Handle {
    fn mount(&self, view: View) -> View {
        let handle = self.clone();
        view.on_mount(move |id| handle.0.set(Some(id)))
    }

    fn node(&self) -> Option<NodeId> {
        self.0.get()
    }
}

fn semantic(role: &str, name: &str, value: &str) -> NodeSemantics {
    NodeSemantics {
        role: role.into(),
        label: name.into(),
        value: value.into(),
        ..Default::default()
    }
}

fn value(tree: &mut UiTree, id: NodeId, value: &str) -> Result<(), UiError> {
    if let Some(old) = tree.semantics(id)?
        && old.value != value
    {
        let mut semantics = old.clone();
        semantics.value = value.into();
        tree.set_semantics(id, Some(semantics))?;
    }
    Ok(())
}

fn face(widget: &str, focus: FocusPolicy) -> View {
    node()
        .theme_type(widget)
        .style_patch(StylePatch {
            layout: LayoutPatch {
                height: Some(px(30.)),
                border_width: (widget == "button").then_some(1.),
                padding: Some(Edges {
                    left: px(9.),
                    right: px(9.),
                    ..Edges::all(px(0.))
                }),
                flex_shrink: Some(0.),
                column_gap: Some(px(6.)),
                align_items: Some(Alignment::Center),
                justify_content: Some(ContentAlignment::Center),
                ..Default::default()
            },
            ..Default::default()
        })
        .focusable(focus)
}

fn activating(view: View, name: &str, mut callback: impl FnMut() + 'static) -> View {
    view.on_action(Action::new(ACTIVATE, name), move |_, _, _| callback())
        .activate_with(ACTIVATE)
}

/// Window-thread handle for a push button.
#[derive(Clone, Default)]
pub struct Button(Handle);

impl Button {
    /// Returns the identity recorded during mount.
    pub fn node(&self) -> Option<NodeId> {
        self.0.node()
    }

    /// Builds a button activated by a primary click, Space or Enter.
    pub fn view(
        &self,
        system: Rc<RefCell<TextSystem>>,
        name: &str,
        typography: TextStyle,
        callback: impl FnMut() + 'static,
    ) -> View {
        self.0.mount(
            activating(face("button", FocusPolicy::Tab), name, callback)
                .semantics(semantic("button", name, ""))
                .child(label(system, name, typography)),
        )
    }

    /// Updates availability and lets the tree recover focus if needed.
    pub fn set_enabled(&self, tree: &mut UiTree, enabled: bool) -> Result<(), UiError> {
        if let Some(id) = self.node() {
            tree.set_disabled(id, !enabled)?;
        }
        Ok(())
    }
}

/// Shape and semantic role of a two-state control.
#[derive(Clone, Copy)]
pub enum ToggleKind {
    /// A discrete checkbox.
    Checkbox,
    /// A switch with an on and off position.
    Switch,
}

/// Window-thread handle for a checkbox or switch.
#[derive(Clone, Default)]
pub struct Toggle {
    face: Handle,
    mark: Handle,
    checked: Rc<Cell<bool>>,
}

impl Toggle {
    /// Returns the identity recorded during mount.
    pub fn node(&self) -> Option<NodeId> {
        self.face.node()
    }

    /// Builds a controlled toggle. The callback receives the proposed new value.
    pub fn view(
        &self,
        system: Rc<RefCell<TextSystem>>,
        name: &str,
        typography: TextStyle,
        kind: ToggleKind,
        checked: bool,
        mut callback: impl FnMut(bool) + 'static,
    ) -> View {
        self.checked.set(checked);
        let current = self.checked.clone();
        let (role, width) = match kind {
            ToggleKind::Checkbox => ("checkbox", 13.),
            ToggleKind::Switch => ("switch", 24.),
        };
        self.face.mount(
            activating(face(role, FocusPolicy::Tab), name, move || {
                callback(!current.get());
            })
            .semantics(semantic(role, name, toggle_value(checked)))
            .child(
                self.mark.mount(
                    node()
                        .style(Style {
                            layout: LayoutStyle {
                                width: px(width),
                                height: px(14.),
                                flex_shrink: 0.,
                                ..Default::default()
                            },
                            hit_test: false,
                            ..Default::default()
                        })
                        .content(toggle_mark(kind, checked)),
                ),
            )
            .child(label(system, name, typography)),
        )
    }

    /// Updates the value, mark and semantic value without replacing the View.
    pub fn set_checked(
        &self,
        tree: &mut UiTree,
        kind: ToggleKind,
        checked: bool,
    ) -> Result<(), UiError> {
        if self.checked.replace(checked) != checked {
            if let Some(id) = self.mark.node() {
                tree.set_content(id, toggle_mark(kind, checked))?;
            }
            if let Some(id) = self.node() {
                value(tree, id, toggle_value(checked))?;
            }
        }
        Ok(())
    }

    /// Updates availability and lets the tree recover focus if needed.
    pub fn set_enabled(&self, tree: &mut UiTree, enabled: bool) -> Result<(), UiError> {
        if let Some(id) = self.node() {
            tree.set_disabled(id, !enabled)?;
        }
        Ok(())
    }
}

fn toggle_value(checked: bool) -> &'static str {
    if checked { "true" } else { "false" }
}

fn fill(x: f64, y: f64, w: f64, h: f64, radius: f64, color: u32) -> Primitive {
    Primitive::Fill(
        Rect::new(x, y, x + w, y + h),
        CornerRadii::uniform(radius),
        Brush::Solid(rgb(color)),
    )
}

fn toggle_mark(kind: ToggleKind, checked: bool) -> Vec<Primitive> {
    let accent = 0x286b64;
    match (kind, checked) {
        (ToggleKind::Checkbox, true) => {
            let mut mark = vec![fill(0., 1., 13., 13., 3., accent)];
            if let Ok(check) = BezierPath::new(vec![
                PathCommand::MoveTo(Point::new(2.8, 7.3)),
                PathCommand::LineTo(Point::new(4.0, 6.2)),
                PathCommand::LineTo(Point::new(5.7, 8.0)),
                PathCommand::LineTo(Point::new(9.0, 4.5)),
                PathCommand::LineTo(Point::new(10.2, 5.7)),
                PathCommand::LineTo(Point::new(5.7, 10.3)),
                PathCommand::Close,
            ]) {
                mark.push(Primitive::FillPath(
                    check,
                    FillRule::NonZero,
                    Brush::Solid(rgb(0xffffff)),
                ));
            }
            mark
        }
        (ToggleKind::Checkbox, false) => vec![
            fill(0., 1., 13., 13., 3., 0x81928e),
            fill(1., 2., 11., 11., 2., 0xf6faf8),
        ],
        (ToggleKind::Switch, _) => vec![
            fill(
                0.,
                1.,
                24.,
                13.,
                6.5,
                if checked { accent } else { 0x879995 },
            ),
            fill(if checked { 13. } else { 2. }, 3., 9., 9., 4.5, 0xffffff),
        ],
    }
}

/// A two-option choice with one tab stop and arrow-key traversal.
#[derive(Clone, Default)]
pub struct Segmented {
    group: Handle,
    segments: [Handle; 2],
    selected: Rc<Cell<usize>>,
}

impl Segmented {
    /// Returns the mounted identity for option zero or one.
    pub fn node(&self, index: usize) -> Option<NodeId> {
        self.segments.get(index)?.node()
    }

    /// Builds a choice whose application updates the selected option.
    pub fn view(
        &self,
        system: Rc<RefCell<TextSystem>>,
        name: &str,
        options: [&str; 2],
        typography: TextStyle,
        callback: impl Fn(usize) + 'static,
    ) -> Result<View, UiError> {
        const INACTIVE: &str = "oxiview.segment.other";
        let choose = Rc::new(callback);
        let keymap = Keymap::new([KeyCode::ArrowLeft, KeyCode::ArrowRight].map(|key| {
            KeyBinding::new(
                INACTIVE,
                [KeyChord::logical(
                    LogicalKey::Named(key),
                    KeyModifiers::default(),
                )],
            )
        }))
        .map_err(|error| UiError::View(error.to_string()))?;
        let selected = self.selected.get();
        let mut group = node()
            .theme_type("segments")
            .style_patch(StylePatch {
                layout: LayoutPatch {
                    border_width: Some(1.),
                    padding: Some(Edges::all(px(2.))),
                    column_gap: Some(px(2.)),
                    flex_shrink: Some(0.),
                    ..Default::default()
                },
                ..Default::default()
            })
            .semantics(semantic("radiogroup", name, ""));
        for (index, option) in options.into_iter().enumerate() {
            let press = choose.clone();
            let arrow = choose.clone();

            let segment = activating(
                face(
                    "segment",
                    if index == selected {
                        FocusPolicy::Tab
                    } else {
                        FocusPolicy::Programmatic
                    },
                ),
                option,
                move || press(index),
            )
            .content(segment_mark(index == selected))
            .semantics(semantic("radio", option, toggle_value(index == selected)))
            .on_action(
                Action::new(INACTIVE, "Select the other option"),
                move |_, _, _| arrow(1 - index),
            )
            .keymap(keymap.clone())
            .child(label(system.clone(), option, typography.clone()));
            group = group.child(self.segments[index].mount(segment));
        }
        Ok(self.group.mount(group))
    }

    /// Updates availability of both options.
    pub fn set_enabled(&self, tree: &mut UiTree, enabled: bool) -> Result<(), UiError> {
        if let Some(id) = self.group.node() {
            tree.set_disabled(id, !enabled)?;
        }
        Ok(())
    }

    /// Applies an option change and moves focus within the group if it owns focus.
    pub fn set_selected(&self, tree: &mut UiTree, index: usize) -> Result<(), UiError> {
        let old = self.selected.get();
        if old == index || index >= self.segments.len() {
            return Ok(());
        }
        let transfer = self
            .node(old)
            .is_some_and(|old| tree.focused_node() == Some(old));
        for option in 0..self.segments.len() {
            if let Some(id) = self.node(option) {
                tree.set_content(id, segment_mark(option == index))?;
                tree.set_focus_policy(
                    id,
                    if option == index {
                        FocusPolicy::Tab
                    } else {
                        FocusPolicy::Programmatic
                    },
                )?;
                value(tree, id, toggle_value(option == index))?;
            }
        }
        if transfer && let Some(id) = self.node(index) {
            tree.focus(id)?;
        }
        self.selected.set(index);
        Ok(())
    }
}

fn segment_mark(active: bool) -> Vec<Primitive> {
    if active {
        vec![fill(8., 24., 14., 2., 1., 0x328379)]
    } else {
        Vec::new()
    }
}

/// Single-line M6 editing surface with a stable handle for the native input bridge.
#[derive(Clone)]
pub struct TextField(Editable);

impl TextField {
    /// Creates a field with its initial committed text.
    pub fn new(value: impl Into<String>) -> Self {
        Self(Editable::new(value, EditMode::SingleLine))
    }

    /// Returns the mounted editing node.
    pub fn node(&self) -> Option<NodeId> {
        self.0.node()
    }

    /// Returns the window-thread model used for committed text and preedit.
    pub fn editor(&self) -> Rc<RefCell<Editor>> {
        self.0.editor()
    }

    /// Builds the field. Supply an associated visible label where appropriate.
    pub fn view(&self, system: Rc<RefCell<TextSystem>>, name: &str, typography: TextStyle) -> View {
        editable_text(system, self.0.clone(), typography)
            .theme_type("text-field")
            .semantics(semantic("textbox", name, self.editor().borrow().source()))
            .style_patch(StylePatch {
                overflow: Some(Overflow::Clip),
                layout: LayoutPatch {
                    height: Some(px(32.)),
                    min_width: Some(px(50.)),
                    flex_grow: Some(1.),
                    flex_basis: Some(px(0.)),
                    border_width: Some(1.),
                    padding: Some(Edges {
                        left: px(9.),
                        right: px(9.),
                        top: px(5.),
                        bottom: px(4.),
                    }),
                    ..Default::default()
                },
                ..Default::default()
            })
    }

    /// Publishes committed text as the semantic value.
    pub fn sync_value(&self, tree: &mut UiTree) -> Result<(), UiError> {
        if let Some(id) = self.node() {
            let model = self.editor();
            let model = model.try_borrow().map_err(|_| UiError::HandlerBusy)?;
            value(tree, id, model.source())?;
        }
        Ok(())
    }

    /// Updates availability and lets the tree end its input session if needed.
    pub fn set_enabled(&self, tree: &mut UiTree, enabled: bool) -> Result<(), UiError> {
        if let Some(id) = self.node() {
            tree.set_disabled(id, !enabled)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
