//! Document Actions shared by the toolbar, search and keyboard.

use oxiview_core::{Action, Editable, KeyBinding, KeyChord, Keymap, NodeId, UiError, UiTree};
use oxiview_platform::{KeyCode, KeyModifiers, LogicalKey, ShortcutPlatform};
use std::{cell::RefCell, collections::VecDeque, rc::Rc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Command {
    New,
    Open,
    Save,
    SaveAs,
    Close,
    Find,
    Dismiss,
    Next,
    Previous,
    Replace,
    ReplaceAll,
    Bold,
    Italic,
    Copy,
    Cut,
    Paste,
    KeepWriting,
    Discard,
}

impl Command {
    pub(crate) fn id(self) -> &'static str {
        match self {
            Self::New => "document.new",
            Self::Open => "document.open",
            Self::Save => "document.save",
            Self::SaveAs => "document.save-as",
            Self::Close => "document.close",
            Self::Find => "document.find",
            Self::Next => "document.next-match",
            Self::Previous => "document.previous-match",
            Self::Replace => "document.replace",
            Self::ReplaceAll => "document.replace-all",
            Self::Dismiss => "document.dismiss",
            Self::Bold => "document.bold",
            Self::Italic => "document.italic",
            Self::Copy => "document.copy",
            Self::Cut => "document.cut",
            Self::Paste => "document.paste",
            Self::KeepWriting => "document.keep-writing",
            Self::Discard => "document.discard",
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::New => "New",
            Self::Open => "Open",
            Self::Save => "Save",
            Self::SaveAs => "Save As",
            Self::Close => "Close document",
            Self::Find => "Find",
            Self::Dismiss => "Done",
            Self::Next => "Next",
            Self::Previous => "Previous",
            Self::Replace => "Replace",
            Self::ReplaceAll => "Replace All",
            Self::Bold => "Bold",
            Self::Italic => "Italic",
            Self::Copy => "Copy",
            Self::Cut => "Cut",
            Self::Paste => "Paste",
            Self::KeepWriting => "Cancel",
            Self::Discard => "Discard",
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct Commands(Rc<RefCell<VecDeque<Command>>>);

impl Commands {
    pub(crate) fn push(&self, command: Command) {
        self.0.borrow_mut().push_back(command)
    }
    pub(crate) fn pop(&self) -> Option<Command> {
        self.0.borrow_mut().pop_front()
    }

    pub(crate) fn register(
        &self,
        tree: &mut UiTree,
        source: &Editable,
        search_fields: [&Editable; 2],
    ) -> Result<(), UiError> {
        use Command::*;
        let root = tree.root();
        let primary = ShortcutPlatform::current().primary_modifier();
        let shifted = KeyModifiers {
            shift: true,
            ..primary
        };
        let character = |command: Command, letter: &str, modifiers| {
            KeyBinding::new(
                command.id(),
                [KeyChord::logical(
                    LogicalKey::Character(letter.into()),
                    modifiers,
                )],
            )
        };
        let named = |command: Command, key| {
            KeyBinding::new(
                command.id(),
                [KeyChord::logical(
                    LogicalKey::Named(key),
                    KeyModifiers::default(),
                )],
            )
        };
        extend_keymap(
            tree,
            root,
            [
                character(New, "n", primary),
                character(Open, "o", primary),
                character(Save, "s", primary),
                character(SaveAs, "s", shifted),
                character(Close, "w", primary),
                character(Find, "f", primary),
                character(Next, "g", primary).repeatable(),
                character(Previous, "g", shifted).repeatable(),
                named(Dismiss, KeyCode::Escape),
            ],
        )?;
        let clipboard = || {
            [
                character(Copy, "c", primary),
                character(Cut, "x", primary),
                character(Paste, "v", primary),
            ]
        };
        extend_keymap(
            tree,
            mounted_field(source)?,
            clipboard().into_iter().chain([
                character(Bold, "b", primary),
                character(Italic, "i", primary),
            ]),
        )?;
        for field in search_fields {
            let id = mounted_field(field)?;
            extend_keymap(
                tree,
                id,
                clipboard()
                    .into_iter()
                    .chain([named(Next, KeyCode::Enter).repeatable()]),
            )?;
            let model = field.editor();
            let commands = self.clone();
            tree.register_action(id, Action::new(Next.id(), Next.label()), move |_, _, _| {
                if model.borrow().presentation().marked.is_none() {
                    commands.push(Next);
                }
            })?;
        }

        for command in [
            New,
            Open,
            Save,
            SaveAs,
            Close,
            Find,
            Dismiss,
            Next,
            Previous,
            Replace,
            ReplaceAll,
            Bold,
            Italic,
            Copy,
            Cut,
            Paste,
            KeepWriting,
            Discard,
        ] {
            let commands = self.clone();
            tree.register_action(
                root,
                Action::new(command.id(), command.label()),
                move |_, _, _| commands.push(command),
            )?;
        }
        Ok(())
    }
}

fn mounted_field(field: &Editable) -> Result<NodeId, UiError> {
    field
        .node()
        .ok_or_else(|| UiError::View("Editor has no live view".into()))
}

fn extend_keymap(
    tree: &mut UiTree,
    node: NodeId,
    bindings: impl IntoIterator<Item = KeyBinding>,
) -> Result<(), UiError> {
    let defaults = tree
        .keymap(node)?
        .into_iter()
        .flat_map(Keymap::effective_bindings);
    let keymap = Keymap::new(defaults.cloned().chain(bindings))
        .map_err(|reason| UiError::View(reason.to_string()))?;
    tree.set_keymap(node, keymap)
}
