use super::{EditorNode, NodeId, Rect, Selection, UiError, UiTree, ui_edit_error};
use crate::{Action, KeyBinding, KeyChord, Keymap};
use oxiview_platform::{
    KeyCode, KeyModifiers, LogicalKey, ShortcutPlatform, TextEditingCommand as Command,
};
use oxiview_text::{Caret, EditCommand, EditMode};
use std::time::Duration;

#[derive(Clone, Copy)]
enum EditorAction {
    Command(Command),
    SelectAll,
    Undo,
    Redo,
}

#[derive(Clone, Copy)]
enum Motion {
    Horizontal(bool),
    Word(bool),
    Vertical(bool),
    LineEdge(bool),
    DocumentEdge(bool),
}

pub(super) fn install(tree: &mut UiTree, id: NodeId, mode: EditMode) -> Result<(), UiError> {
    let platform = ShortcutPlatform::current();
    let primary = platform.primary_modifier();
    let word = if platform == ShortcutPlatform::MacOs {
        KeyModifiers {
            alt: true,
            ..Default::default()
        }
    } else {
        primary
    };
    let normal = KeyModifiers::default();
    let mut bindings = Vec::new();

    let mut register = |name: &str,
                        description: &'static str,
                        action,
                        keys: Vec<(LogicalKey, KeyModifiers)>,
                        repeat| {
        let name = format!("editor.{name}");
        for (key, modifiers) in keys {
            let binding = KeyBinding::new(&name, [KeyChord::logical(key, modifiers)]);
            bindings.push(if repeat {
                binding.repeatable()
            } else {
                binding
            });
        }
        tree.register_action(
            id,
            Action::new(name, description),
            move |event, _, commands| {
                let now = event.timestamp.unwrap_or_default();
                commands.push(super::Command::Mutation(
                    id,
                    Box::new(move |tree, _, _| tree.run_editor_action(id, action, now)),
                ));
            },
        )
    };

    let mut movement = |name: &str,
                        description: &'static str,
                        extend_name: &str,
                        command: fn(bool) -> Command,
                        keys: &[(KeyCode, KeyModifiers)]| {
        register(
            name,
            description,
            EditorAction::Command(command(false)),
            keys.iter()
                .map(|&(key, modifiers)| (LogicalKey::Named(key), modifiers))
                .collect(),
            true,
        )?;
        register(
            extend_name,
            description,
            EditorAction::Command(command(true)),
            keys.iter()
                .map(|&(key, modifiers)| (LogicalKey::Named(key), shifted(modifiers)))
                .collect(),
            true,
        )
    };

    movement(
        "left",
        "Move caret left",
        "select-left",
        |select| Command::MoveLeft { select },
        &[(KeyCode::ArrowLeft, normal)],
    )?;
    movement(
        "right",
        "Move caret right",
        "select-right",
        |select| Command::MoveRight { select },
        &[(KeyCode::ArrowRight, normal)],
    )?;
    movement(
        "up",
        "Move caret up",
        "select-up",
        |select| Command::MoveUp { select },
        &[(KeyCode::ArrowUp, normal)],
    )?;
    movement(
        "down",
        "Move caret down",
        "select-down",
        |select| Command::MoveDown { select },
        &[(KeyCode::ArrowDown, normal)],
    )?;
    movement(
        "word-left",
        "Move caret left by word",
        "select-word-left",
        |select| Command::MoveWordLeft { select },
        &[(KeyCode::ArrowLeft, word)],
    )?;
    movement(
        "word-right",
        "Move caret right by word",
        "select-word-right",
        |select| Command::MoveWordRight { select },
        &[(KeyCode::ArrowRight, word)],
    )?;

    let mut line_start = vec![(KeyCode::Home, normal)];
    let mut line_end = vec![(KeyCode::End, normal)];
    let mut document_start = vec![(KeyCode::Home, primary)];
    let mut document_end = vec![(KeyCode::End, primary)];
    if platform == ShortcutPlatform::MacOs {
        line_start.push((KeyCode::ArrowLeft, primary));
        line_end.push((KeyCode::ArrowRight, primary));
        document_start.push((KeyCode::ArrowUp, primary));
        document_end.push((KeyCode::ArrowDown, primary));
    }
    movement(
        "line-start",
        "Move caret to line start",
        "select-to-line-start",
        |select| Command::MoveLineStart { select },
        &line_start,
    )?;
    movement(
        "line-end",
        "Move caret to line end",
        "select-to-line-end",
        |select| Command::MoveLineEnd { select },
        &line_end,
    )?;
    movement(
        "document-start",
        "Move caret to document start",
        "select-to-document-start",
        |select| Command::MoveDocumentStart { select },
        &document_start,
    )?;
    movement(
        "document-end",
        "Move caret to document end",
        "select-to-document-end",
        |select| Command::MoveDocumentEnd { select },
        &document_end,
    )?;

    for (name, description, command, key, modifiers) in [
        (
            "delete-backward",
            "Delete backward",
            Command::DeleteBackward,
            KeyCode::Backspace,
            normal,
        ),
        (
            "delete-forward",
            "Delete forward",
            Command::DeleteForward,
            KeyCode::Delete,
            normal,
        ),
        (
            "delete-word-backward",
            "Delete previous word",
            Command::DeleteWordBackward,
            KeyCode::Backspace,
            word,
        ),
        (
            "delete-word-forward",
            "Delete next word",
            Command::DeleteWordForward,
            KeyCode::Delete,
            word,
        ),
    ] {
        register(
            name,
            description,
            EditorAction::Command(command),
            vec![(LogicalKey::Named(key), modifiers)],
            true,
        )?;
    }
    if platform == ShortcutPlatform::MacOs {
        for (name, description, command, key) in [
            (
                "delete-to-line-start",
                "Delete to line start",
                Command::DeleteToLineStart,
                KeyCode::Backspace,
            ),
            (
                "delete-to-line-end",
                "Delete to line end",
                Command::DeleteToLineEnd,
                KeyCode::Delete,
            ),
        ] {
            register(
                name,
                description,
                EditorAction::Command(command),
                vec![(LogicalKey::Named(key), primary)],
                true,
            )?;
        }
    }
    if mode == EditMode::MultiLine {
        register(
            "newline",
            "Insert newline",
            EditorAction::Command(Command::InsertNewline),
            vec![(LogicalKey::Named(KeyCode::Enter), normal)],
            true,
        )?;
    }

    let character = |letter: &str| LogicalKey::Character(letter.into());
    register(
        "select-all",
        "Select all text",
        EditorAction::SelectAll,
        vec![(character("a"), primary)],
        false,
    )?;
    register(
        "undo",
        "Undo text edit",
        EditorAction::Undo,
        vec![(character("z"), primary)],
        false,
    )?;
    let mut redo = vec![(character("z"), shifted(primary))];
    if platform != ShortcutPlatform::MacOs {
        redo.push((character("y"), primary));
    }
    register("redo", "Redo text edit", EditorAction::Redo, redo, false)?;
    let keymap = Keymap::new(bindings).map_err(|error| UiError::View(error.to_string()))?;
    tree.set_keymap(id, keymap)
}

fn shifted(modifiers: KeyModifiers) -> KeyModifiers {
    KeyModifiers {
        shift: true,
        ..modifiers
    }
}

impl UiTree {
    fn run_editor_action(
        &mut self,
        id: NodeId,
        action: EditorAction,
        now: Duration,
    ) -> Result<(), UiError> {
        if !self.interaction_available(id)? {
            return Ok(());
        }
        let editor = self.editor(id)?;
        let (changed, desired) = match action {
            EditorAction::SelectAll => {
                let mut model = editor.control.model_mut()?;
                let selection = Selection::new(0, model.source().len());
                (model.set_selection(selection).map_err(ui_edit_error)?, None)
            }
            EditorAction::Undo => (
                editor.control.model_mut()?.undo().map_err(ui_edit_error)?,
                None,
            ),
            EditorAction::Redo => (
                editor.control.model_mut()?.redo().map_err(ui_edit_error)?,
                None,
            ),
            EditorAction::Command(command) => {
                let Some((command, column)) =
                    editor.resolve_command(self.editor_viewport(id)?, command)?
                else {
                    return Ok(());
                };
                (
                    editor
                        .control
                        .model_mut()?
                        .apply_command(command, now)
                        .map_err(ui_edit_error)?,
                    column,
                )
            }
        };
        editor.desired_x.set(desired);
        if changed {
            editor.note_edit(false)?;
            self.dirty_editor(id);
        }
        Ok(())
    }
}

impl EditorNode {
    pub(super) fn resolve_command(
        &self,
        viewport: Rect,
        command: Command,
    ) -> Result<Option<(EditCommand, Option<f64>)>, UiError> {
        let (motion, extend) = match command {
            Command::MoveLeft { select } => (Motion::Horizontal(false), select),
            Command::MoveRight { select } => (Motion::Horizontal(true), select),
            Command::MoveUp { select } => (Motion::Vertical(false), select),
            Command::MoveDown { select } => (Motion::Vertical(true), select),
            Command::MoveWordLeft { select } => (Motion::Word(false), select),
            Command::MoveWordRight { select } => (Motion::Word(true), select),
            Command::MoveLineStart { select } => (Motion::LineEdge(false), select),
            Command::MoveLineEnd { select } => (Motion::LineEdge(true), select),
            Command::MoveDocumentStart { select } => (Motion::DocumentEdge(false), select),
            Command::MoveDocumentEnd { select } => (Motion::DocumentEdge(true), select),
            Command::DeleteBackward => {
                return resolved(EditCommand::DeleteBackward { word: false });
            }
            Command::DeleteForward => return resolved(EditCommand::DeleteForward { word: false }),
            Command::DeleteWordBackward => {
                return resolved(EditCommand::DeleteBackward { word: true });
            }
            Command::DeleteWordForward => {
                return resolved(EditCommand::DeleteForward { word: true });
            }
            Command::InsertNewline => return resolved(EditCommand::InsertNewline),
            Command::DeleteToLineStart | Command::DeleteToLineEnd => {
                let active = self.control.model()?.selection().active;
                let geometry = self.prepare_geometry(viewport)?;
                let layout = self.committed_layout(&geometry)?;
                let caret = layout.line_edge(active, command == Command::DeleteToLineEnd);
                return resolved(EditCommand::DeleteTo(caret.offset));
            }
            _ => return Ok(None),
        };

        let (selection, length) = {
            let model = self.control.model()?;
            (model.selection(), model.source().len())
        };
        let active = selection.active;
        let (caret, column) = if let Motion::DocumentEdge(end) = motion {
            (Caret::new(if end { length } else { 0 }), None)
        } else {
            let geometry = self.prepare_geometry(viewport)?;
            let layout = self.committed_layout(&geometry)?;
            match motion {
                Motion::Horizontal(right) if !extend && !selection.is_empty() => {
                    (layout.selection_edge(selection, right), None)
                }
                Motion::Horizontal(right) => (layout.move_visual(active, right), None),
                Motion::Word(right) => (layout.move_word(active, right), None),
                Motion::LineEdge(end) => (layout.line_edge(active, end), None),
                Motion::Vertical(down) => {
                    let x = self
                        .desired_x
                        .get()
                        .or_else(|| layout.caret_rect(active).map(|rect| rect.origin().x()));
                    let caret = x.map_or(active, |x| layout.move_vertical(active, down, x));
                    (caret, x)
                }
                Motion::DocumentEdge(_) => (active, None),
            }
        };
        let anchor = if extend { selection.anchor } else { caret };
        Ok(Some((
            EditCommand::Select(Selection {
                anchor,
                active: caret,
            }),
            column,
        )))
    }
}

fn resolved(command: EditCommand) -> Result<Option<(EditCommand, Option<f64>)>, UiError> {
    Ok(Some((command, None)))
}
