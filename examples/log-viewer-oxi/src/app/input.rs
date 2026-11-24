//! Routes application Actions and input intents to the viewer.

mod collection;
mod desktop;

pub(crate) use collection::Navigation;
pub(super) use collection::{HeadingDrag, RecordDrag};
pub(super) use desktop::Desktop;

use super::{Intent, Intents, Mode, ViewerApp, error, presentation::column};
use crate::{model::Column, view::Ui};
use oxiview_base::Point;
use oxiview_core::{Action, Error, KeyBinding, KeyChord, Keymap, NodeId, UiTree};
use oxiview_platform::{KeyCode, KeyModifiers, LogicalKey, ShortcutPlatform};
use oxiview_widgets::{collection::CellKey, controls::ScrollbarAxis};

pub(super) fn register(tree: &mut UiTree, ui: &Ui, intents: &Intents) -> Result<(), Error> {
    let root = tree.root();
    let body = ui.collection.get()?;
    let primary = ShortcutPlatform::current().primary_modifier();
    let desktop = [
        (Intent::Open, "log.open", "Open a log file", "o"),
        (Intent::CopyShortcut, "log.copy", "Copy selection", "c"),
        (Intent::CutSearch, "log.cut-search", "Cut search text", "x"),
        (
            Intent::PasteSearch,
            "log.paste-search",
            "Paste search text",
            "v",
        ),
        (Intent::FocusSearch, "log.search", "Focus search", "f"),
    ];
    let root_keys = desktop.map(|(intent, id, name, key)| {
        (
            intent,
            id,
            name,
            KeyBinding::new(
                id,
                [KeyChord::logical(
                    LogicalKey::Character(key.into()),
                    primary,
                )],
            ),
        )
    });
    install(tree, root, intents, root_keys)?;
    let directions = [
        (Navigation::Up, KeyCode::ArrowUp, "up"),
        (Navigation::Down, KeyCode::ArrowDown, "down"),
        (Navigation::Left, KeyCode::ArrowLeft, "left"),
        (Navigation::Right, KeyCode::ArrowRight, "right"),
        (Navigation::PageUp, KeyCode::PageUp, "page-up"),
        (Navigation::PageDown, KeyCode::PageDown, "page-down"),
        (Navigation::First, KeyCode::Home, "first"),
        (Navigation::Last, KeyCode::End, "last"),
    ];
    let mut keymap = Vec::new();
    for (navigation, code, direction) in directions {
        for extend in [false, true] {
            let id = format!(
                "log.{}.{}",
                if extend { "extend" } else { "navigate" },
                direction
            );
            let name = if extend {
                "Extend selection"
            } else {
                "Navigate records"
            };
            let queue = intents.clone();
            tree.register_action(body, Action::new(&id, name), move |_, _, _| {
                queue.push(Intent::Navigate(navigation, extend));
            })?;
            keymap.push(
                KeyBinding::new(
                    id,
                    [KeyChord::logical(
                        LogicalKey::Named(code),
                        KeyModifiers {
                            shift: extend,
                            ..Default::default()
                        },
                    )],
                )
                .repeatable(),
            );
        }
    }
    let queue = intents.clone();
    tree.register_action(
        body,
        Action::new(
            "log.activate-record",
            "Expand a list record or sort the active table column",
        ),
        move |_, _, _| {
            queue.push(Intent::ActivateRecord);
        },
    )?;
    keymap.push(KeyBinding::new(
        "log.activate-record",
        [KeyChord::logical(
            LogicalKey::Named(KeyCode::Enter),
            KeyModifiers::default(),
        )],
    ));
    tree.set_keymap(body, Keymap::new(keymap).map_err(error)?)?;
    Ok(())
}

fn install(
    tree: &mut UiTree,
    node: NodeId,
    intents: &Intents,
    entries: impl IntoIterator<Item = (Intent, &'static str, &'static str, KeyBinding)>,
) -> Result<(), Error> {
    let mut bindings = Vec::new();
    for (intent, id, name, binding) in entries {
        let queue = intents.clone();
        tree.register_action(node, Action::new(id, name), move |_, _, _| {
            queue.push(intent)
        })?;
        bindings.push(binding);
    }
    tree.set_keymap(node, Keymap::new(bindings).map_err(error)?)?;
    Ok(())
}

impl ViewerApp {
    pub(super) fn perform(&mut self, intent: Intent) -> Result<bool, Error> {
        match intent {
            Intent::Open => self.request_panel()?,
            Intent::Copy => self.copy_records()?,
            Intent::CopyShortcut if self.tree.focused_node() == self.ui.search.node() => {
                self.clip_search(false, false)?;
            }
            Intent::CopyShortcut => self.copy_records()?,
            Intent::CutSearch => self.clip_search(false, true)?,
            Intent::PasteSearch => self.clip_search(true, false)?,
            Intent::FocusSearch => {
                if let Some(node) = self.ui.search.node() {
                    self.tree.focus(node)?;
                    return Ok(true);
                }
            }
            Intent::Severity(severity, enabled) => {
                self.display.sort_or_filter |= self.book.set_severity(severity, enabled);
            }
            Intent::Follow(true) => {
                self.display.sort_or_filter |= self.book.sort().is_some();
                self.book.resume_follow();
                self.display.reveal_tail = true;
            }
            Intent::Follow(false) => self.book.pause_follow(),
            Intent::Mode(mode) => {
                if mode != self.mode {
                    match mode {
                        Mode::List => self.list_key = self.active_key(),
                        Mode::Table => {
                            if let Some(row) = self.list_key {
                                let previous = self
                                    .table
                                    .selection()
                                    .filter(|selection| selection.active.row == row);
                                if previous.is_none() {
                                    self.table.select(
                                        CellKey {
                                            row,
                                            column: Column::Number as u64,
                                        },
                                        false,
                                    );
                                }
                            } else {
                                self.table.clear_selection();
                            }
                        }
                    }
                    self.mode = mode;
                    self.record_drag = None;
                    self.heading_drag = None;
                }
            }
            Intent::Scrollbar(axis, value) => {
                let collection = self.ui.collection.get()?;
                let old = self.tree.scroll_offset(collection)?;
                let new = match axis {
                    ScrollbarAxis::Horizontal => Point::new(value, old.y()),
                    ScrollbarAxis::Vertical => Point::new(old.x(), value),
                };
                let changed = self.tree.set_scroll_offset(collection, new)?;
                self.pause_if_scrolled_back(old)?;
                return Ok(changed);
            }
            Intent::Heading(event) => return self.heading(event),
            Intent::RecordPointer(event) => return self.record_pointer(event),
            Intent::Navigate(navigation, extend) => return self.navigate(navigation, extend),
            Intent::Sort(column) => self.sort(column),
            Intent::ActivateRecord => match self.mode {
                Mode::List => {
                    if let Some(key) = self.list_key {
                        self.toggle_expanded(key);
                    }
                }
                Mode::Table => {
                    if let Some(column) = self
                        .table
                        .selection()
                        .and_then(|range| column(range.active.column))
                    {
                        self.sort(column);
                    }
                }
            },
        }
        Ok(false)
    }
}
