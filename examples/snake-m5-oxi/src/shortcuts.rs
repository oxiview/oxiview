//! Snake's Action catalog and its board, window and settings Keymaps.

use oxiview_core::{KeyBinding, KeyChord, Keymap, KeymapError, ShortcutKey};
use oxiview_platform::{KeyCode, KeyModifiers, LogicalKey, ShortcutPlatform};
use snake::{
    application::Action,
    game::{Difficulty, Direction},
};

pub(crate) const OPEN: &str = "snake.settings.open";
pub(crate) const CLOSE: &str = "snake.settings.close";
pub(crate) const PRIMARY: &str = "snake.primary";
pub(crate) const RESTART: &str = "snake.restart";
pub(crate) const ENTER: &str = "snake.start-or-restart";
pub(crate) const PAUSE: &str = "snake.pause";

pub(crate) fn action_id(action: Action) -> &'static str {
    match action {
        Action::Primary => PRIMARY,
        Action::NewGame => RESTART,
        Action::Pause => PAUSE,
        Action::Turn(Direction::Up) => "snake.turn.up",
        Action::Turn(Direction::Right) => "snake.turn.right",
        Action::Turn(Direction::Down) => "snake.turn.down",
        Action::Turn(Direction::Left) => "snake.turn.left",
        Action::Difficulty(Difficulty::Chill) => "snake.pace.chill",
        Action::Difficulty(Difficulty::Classic) => "snake.pace.classic",
        Action::Difficulty(Difficulty::Swift) => "snake.pace.swift",
    }
}

/// The directions have both a layout-independent game position and a named arrow.
pub(crate) const DIRECTIONS: [(Direction, KeyCode, KeyCode, &str); 4] = [
    (Direction::Up, KeyCode::ArrowUp, KeyCode::KeyW, "Turn up"),
    (
        Direction::Right,
        KeyCode::ArrowRight,
        KeyCode::KeyD,
        "Turn right",
    ),
    (
        Direction::Down,
        KeyCode::ArrowDown,
        KeyCode::KeyS,
        "Turn down",
    ),
    (
        Direction::Left,
        KeyCode::ArrowLeft,
        KeyCode::KeyA,
        "Turn left",
    ),
];

pub(crate) const EDITABLE: [(&str, &str); 7] = [
    ("snake.turn.up", "Turn up"),
    ("snake.turn.right", "Turn right"),
    ("snake.turn.down", "Turn down"),
    ("snake.turn.left", "Turn left"),
    (PRIMARY, "Start / pause / resume"),
    (ENTER, "Start / new game"),
    (PAUSE, "Pause"),
];

pub(crate) fn named(key: KeyCode) -> KeyChord {
    KeyChord::logical(LogicalKey::Named(key), KeyModifiers::default())
}

pub(crate) fn board_keymap() -> Result<Keymap, KeymapError> {
    let mut bindings = Vec::new();
    for (direction, arrow, position, _) in DIRECTIONS {
        let action = action_id(Action::Turn(direction));
        bindings.push(KeyBinding::new(action, [named(arrow)]));
        bindings.push(KeyBinding::new(
            action,
            [KeyChord::physical(position, KeyModifiers::default())],
        ));
    }
    bindings.extend([
        KeyBinding::new(PRIMARY, [named(KeyCode::Space)]),
        KeyBinding::new(ENTER, [named(KeyCode::Enter)]),
        KeyBinding::new(PAUSE, [named(KeyCode::Escape)]),
        KeyBinding::new(
            PAUSE,
            [
                KeyChord::logical(
                    LogicalKey::Character("k".into()),
                    ShortcutPlatform::current().primary_modifier(),
                ),
                KeyChord::logical(LogicalKey::Character("p".into()), KeyModifiers::default()),
            ],
        ),
    ]);
    Keymap::new(bindings)
}

pub(crate) fn window_keymap() -> Result<Keymap, KeymapError> {
    window_keymap_for(ShortcutPlatform::current())
}

fn window_keymap_for(profile: ShortcutPlatform) -> Result<Keymap, KeymapError> {
    let mut bindings = vec![KeyBinding::new(
        OPEN,
        [KeyChord::logical(
            LogicalKey::Character(",".into()),
            profile.primary_modifier(),
        )],
    )];
    if profile == ShortcutPlatform::Windows {
        bindings.push(KeyBinding::new(
            OPEN,
            [KeyChord::logical(
                LogicalKey::Character("s".into()),
                KeyModifiers {
                    alt: true,
                    ..Default::default()
                },
            )],
        ));
    }
    Keymap::new(bindings)
}

pub(crate) fn shortcut_label(map: &Keymap, action: &str) -> String {
    map.effective_bindings()
        .filter(|binding| binding.action == action)
        .map(|binding| {
            binding
                .sequence
                .iter()
                .map(chord_label)
                .collect::<Vec<_>>()
                .join(" then ")
        })
        .collect::<Vec<_>>()
        .join(" / ")
}

fn chord_label(chord: &KeyChord) -> String {
    let mut parts = Vec::new();
    let modifiers = chord.modifiers;
    if modifiers.control {
        parts.push("Ctrl".to_string());
    }
    if modifiers.alt {
        parts.push(
            if cfg!(target_os = "macos") {
                "Option"
            } else {
                "Alt"
            }
            .to_string(),
        );
    }
    if modifiers.shift {
        parts.push("Shift".to_string());
    }
    if modifiers.meta {
        parts.push(
            if cfg!(target_os = "macos") {
                "Cmd"
            } else {
                "Meta"
            }
            .to_string(),
        );
    }
    let key = match &chord.key {
        ShortcutKey::Logical(LogicalKey::Character(character)) => character.to_uppercase(),
        ShortcutKey::Logical(LogicalKey::Named(code)) | ShortcutKey::Physical(code) => match code {
            KeyCode::ArrowUp => "↑".into(),
            KeyCode::ArrowRight => "→".into(),
            KeyCode::ArrowDown => "↓".into(),
            KeyCode::ArrowLeft => "←".into(),
            code => code
                .name()
                .strip_prefix("Key")
                .unwrap_or(code.name())
                .to_owned(),
        },
        _ => "?".into(),
    };
    parts.push(key);
    parts.join("+")
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxiview_base::Size;
    use oxiview_core::{Action as UiAction, UiTree};
    use oxiview_platform::WindowEvent;
    use std::{cell::Cell, error::Error, rc::Rc, time::Duration};

    #[test]
    fn desktop_defaults_resolve_through_the_same_dispatch_on_any_host() -> Result<(), Box<dyn Error>>
    {
        let ctrl = KeyModifiers {
            control: true,
            ..Default::default()
        };
        let cmd = KeyModifiers {
            meta: true,
            ..Default::default()
        };
        let alt = KeyModifiers {
            alt: true,
            ..Default::default()
        };
        for (profile, strokes) in [
            (
                ShortcutPlatform::MacOs,
                [
                    (cmd, KeyCode::Comma, true),
                    (ctrl, KeyCode::Comma, false),
                    (alt, KeyCode::KeyS, false),
                ],
            ),
            (
                ShortcutPlatform::Windows,
                [
                    (cmd, KeyCode::Comma, false),
                    (ctrl, KeyCode::Comma, true),
                    (alt, KeyCode::KeyS, true),
                ],
            ),
            (
                ShortcutPlatform::Linux,
                [
                    (cmd, KeyCode::Comma, false),
                    (ctrl, KeyCode::Comma, true),
                    (alt, KeyCode::KeyS, false),
                ],
            ),
        ] {
            let mut tree = UiTree::new()?;
            let calls = Rc::new(Cell::new(0));
            let observed = Rc::clone(&calls);
            let root = tree.root();
            tree.set_keymap(root, window_keymap_for(profile)?)?;
            tree.register_action(root, UiAction::new(OPEN, "Settings"), move |_, _, _| {
                observed.set(observed.get() + 1)
            })?;
            for (modifiers, code, expected) in strokes {
                let before = calls.get();
                for pressed in [true, false] {
                    tree.dispatch(
                        &WindowEvent::Key {
                            code,
                            logical: code.us_logical_key(),
                            modifiers,
                            pressed,
                            repeat: false,
                            timestamp: Duration::ZERO,
                        },
                        Size::new(64.0, 64.0),
                        |_, _, _| {},
                    )?;
                }
                assert_eq!(calls.get() - before, i32::from(expected));
            }
        }
        Ok(())
    }
}
