use objc2_app_kit as appkit;
use objc2_app_kit::{NSEvent, NSEventModifierFlags};
use oxiview_platform::{KeyCode, KeyModifiers, LogicalKey, WindowEvent};

use super::seconds;

pub(super) fn transition(event: &NSEvent, pressed: bool) -> WindowEvent {
    let flags = event.modifierFlags();
    let logical =
        event
            .charactersIgnoringModifiers()
            .map_or(LogicalKey::Unidentified, |characters| {
                logical_key(
                    &characters
                        .precomposedStringWithCanonicalMapping()
                        .to_string(),
                )
            });
    WindowEvent::Key {
        code: physical_key(event.keyCode()),
        logical,
        modifiers: KeyModifiers {
            shift: flags.contains(NSEventModifierFlags::Shift),
            control: flags.contains(NSEventModifierFlags::Control),
            alt: flags.contains(NSEventModifierFlags::Option),
            meta: flags.contains(NSEventModifierFlags::Command),
        },
        pressed,
        repeat: pressed && event.isARepeat(),
        timestamp: seconds(event.timestamp()),
    }
}

fn physical_key(code: u16) -> KeyCode {
    // HIToolbox/Events.h defines these virtual positions independently of the
    // active input source. Keep the existing Return and keypad Enter identity.
    match code {
        0x00 => KeyCode::KeyA,
        0x01 => KeyCode::KeyS,
        0x02 => KeyCode::KeyD,
        0x03 => KeyCode::KeyF,
        0x04 => KeyCode::KeyH,
        0x05 => KeyCode::KeyG,
        0x06 => KeyCode::KeyZ,
        0x07 => KeyCode::KeyX,
        0x08 => KeyCode::KeyC,
        0x09 => KeyCode::KeyV,
        0x0a => KeyCode::IntlBackslash,
        0x0b => KeyCode::KeyB,
        0x0c => KeyCode::KeyQ,
        0x0d => KeyCode::KeyW,
        0x0e => KeyCode::KeyE,
        0x0f => KeyCode::KeyR,
        0x10 => KeyCode::KeyY,
        0x11 => KeyCode::KeyT,
        0x12 => KeyCode::Digit1,
        0x13 => KeyCode::Digit2,
        0x14 => KeyCode::Digit3,
        0x15 => KeyCode::Digit4,
        0x16 => KeyCode::Digit6,
        0x17 => KeyCode::Digit5,
        0x18 => KeyCode::Equal,
        0x19 => KeyCode::Digit9,
        0x1a => KeyCode::Digit7,
        0x1b => KeyCode::Minus,
        0x1c => KeyCode::Digit8,
        0x1d => KeyCode::Digit0,
        0x1e => KeyCode::BracketRight,
        0x1f => KeyCode::KeyO,
        0x20 => KeyCode::KeyU,
        0x21 => KeyCode::BracketLeft,
        0x22 => KeyCode::KeyI,
        0x23 => KeyCode::KeyP,
        0x24 | 0x4c => KeyCode::Enter,
        0x25 => KeyCode::KeyL,
        0x26 => KeyCode::KeyJ,
        0x27 => KeyCode::Quote,
        0x28 => KeyCode::KeyK,
        0x29 => KeyCode::Semicolon,
        0x2a => KeyCode::Backslash,
        0x2b => KeyCode::Comma,
        0x2c => KeyCode::Slash,
        0x2d => KeyCode::KeyN,
        0x2e => KeyCode::KeyM,
        0x2f => KeyCode::Period,
        0x30 => KeyCode::Tab,
        0x31 => KeyCode::Space,
        0x32 => KeyCode::Backquote,
        0x33 => KeyCode::Backspace,
        0x35 => KeyCode::Escape,
        0x40 => KeyCode::F17,
        0x41 => KeyCode::NumpadDecimal,
        0x43 => KeyCode::NumpadMultiply,
        0x45 => KeyCode::NumpadAdd,
        0x47 => KeyCode::Clear,
        0x4b => KeyCode::NumpadDivide,
        0x4e => KeyCode::NumpadSubtract,
        0x4f => KeyCode::F18,
        0x50 => KeyCode::F19,
        0x51 => KeyCode::NumpadEqual,
        0x52 => KeyCode::Numpad0,
        0x53 => KeyCode::Numpad1,
        0x54 => KeyCode::Numpad2,
        0x55 => KeyCode::Numpad3,
        0x56 => KeyCode::Numpad4,
        0x57 => KeyCode::Numpad5,
        0x58 => KeyCode::Numpad6,
        0x59 => KeyCode::Numpad7,
        0x5a => KeyCode::F20,
        0x5b => KeyCode::Numpad8,
        0x5c => KeyCode::Numpad9,
        0x5d => KeyCode::IntlYen,
        0x5e => KeyCode::IntlRo,
        0x5f => KeyCode::NumpadComma,
        0x60 => KeyCode::F5,
        0x61 => KeyCode::F6,
        0x62 => KeyCode::F7,
        0x63 => KeyCode::F3,
        0x64 => KeyCode::F8,
        0x65 => KeyCode::F9,
        0x67 => KeyCode::F11,
        0x69 => KeyCode::F13,
        0x6a => KeyCode::F16,
        0x6b => KeyCode::F14,
        0x6d => KeyCode::F10,
        0x6f => KeyCode::F12,
        0x71 => KeyCode::F15,
        0x72 => KeyCode::Help,
        0x73 => KeyCode::Home,
        0x74 => KeyCode::PageUp,
        0x75 => KeyCode::Delete,
        0x76 => KeyCode::F4,
        0x77 => KeyCode::End,
        0x78 => KeyCode::F2,
        0x79 => KeyCode::PageDown,
        0x7a => KeyCode::F1,
        0x7b => KeyCode::ArrowLeft,
        0x7c => KeyCode::ArrowRight,
        0x7d => KeyCode::ArrowDown,
        0x7e => KeyCode::ArrowUp,
        _ => KeyCode::Unidentified,
    }
}

fn logical_key(characters: &str) -> LogicalKey {
    let mut chars = characters.chars();
    let Some(first) = chars.next() else {
        return LogicalKey::Unidentified;
    };
    if chars.as_str().is_empty()
        && let Some(code) = named_key(first)
    {
        return LogicalKey::Named(code);
    }
    // AppKit reserves this private-use range for function key symbols.
    if characters
        .chars()
        .any(|ch| ch.is_control() || ('\u{f700}'..='\u{f8ff}').contains(&ch))
    {
        return LogicalKey::Unidentified;
    }
    LogicalKey::Character(characters.to_lowercase())
}

fn named_key(character: char) -> Option<KeyCode> {
    Some(match u32::from(character) {
        // Cocoa text-system characters include a separate backtab and keypad
        // Enter. Both take the same named identity as their unmodified control.
        0x09 | 0x19 => KeyCode::Tab,
        0x03 | 0x0a | 0x0d => KeyCode::Enter,
        0x08 | 0x7f => KeyCode::Backspace,
        0x1b => KeyCode::Escape,
        0x20 => KeyCode::Space,
        appkit::NSUpArrowFunctionKey => KeyCode::ArrowUp,
        appkit::NSDownArrowFunctionKey => KeyCode::ArrowDown,
        appkit::NSLeftArrowFunctionKey => KeyCode::ArrowLeft,
        appkit::NSRightArrowFunctionKey => KeyCode::ArrowRight,
        appkit::NSF1FunctionKey => KeyCode::F1,
        appkit::NSF2FunctionKey => KeyCode::F2,
        appkit::NSF3FunctionKey => KeyCode::F3,
        appkit::NSF4FunctionKey => KeyCode::F4,
        appkit::NSF5FunctionKey => KeyCode::F5,
        appkit::NSF6FunctionKey => KeyCode::F6,
        appkit::NSF7FunctionKey => KeyCode::F7,
        appkit::NSF8FunctionKey => KeyCode::F8,
        appkit::NSF9FunctionKey => KeyCode::F9,
        appkit::NSF10FunctionKey => KeyCode::F10,
        appkit::NSF11FunctionKey => KeyCode::F11,
        appkit::NSF12FunctionKey => KeyCode::F12,
        appkit::NSF13FunctionKey => KeyCode::F13,
        appkit::NSF14FunctionKey => KeyCode::F14,
        appkit::NSF15FunctionKey => KeyCode::F15,
        appkit::NSF16FunctionKey => KeyCode::F16,
        appkit::NSF17FunctionKey => KeyCode::F17,
        appkit::NSF18FunctionKey => KeyCode::F18,
        appkit::NSF19FunctionKey => KeyCode::F19,
        appkit::NSF20FunctionKey => KeyCode::F20,
        appkit::NSInsertFunctionKey => KeyCode::Insert,
        appkit::NSDeleteFunctionKey => KeyCode::Delete,
        appkit::NSHomeFunctionKey => KeyCode::Home,
        appkit::NSEndFunctionKey => KeyCode::End,
        appkit::NSPageUpFunctionKey => KeyCode::PageUp,
        appkit::NSPageDownFunctionKey => KeyCode::PageDown,
        appkit::NSClearLineFunctionKey | appkit::NSClearDisplayFunctionKey => KeyCode::Clear,
        appkit::NSHelpFunctionKey => KeyCode::Help,
        _ => return None,
    })
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use objc2::rc::Retained;
    use objc2_foundation::{NSPoint, NSString};
    use std::time::Duration;

    fn appkit_event(
        kind: appkit::NSEventType,
        code: u16,
        characters: &str,
        layout_characters: &str,
        flags: NSEventModifierFlags,
        repeat: bool,
    ) -> Retained<NSEvent> {
        NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
            kind,
            NSPoint::new(0.0, 0.0),
            flags,
            12.125,
            0,
            None,
            &NSString::from_str(characters),
            &NSString::from_str(layout_characters),
            repeat,
            code,
        )
        .expect("AppKit key event")
    }

    #[test]
    fn all_us_letters_digits_and_punctuation_have_physical_positions() {
        for (code, label) in [
            (0x00, "a"),
            (0x0b, "b"),
            (0x08, "c"),
            (0x02, "d"),
            (0x0e, "e"),
            (0x03, "f"),
            (0x05, "g"),
            (0x04, "h"),
            (0x22, "i"),
            (0x26, "j"),
            (0x28, "k"),
            (0x25, "l"),
            (0x2e, "m"),
            (0x2d, "n"),
            (0x1f, "o"),
            (0x23, "p"),
            (0x0c, "q"),
            (0x0f, "r"),
            (0x01, "s"),
            (0x11, "t"),
            (0x20, "u"),
            (0x09, "v"),
            (0x0d, "w"),
            (0x07, "x"),
            (0x10, "y"),
            (0x06, "z"),
            (0x1d, "0"),
            (0x12, "1"),
            (0x13, "2"),
            (0x14, "3"),
            (0x15, "4"),
            (0x17, "5"),
            (0x16, "6"),
            (0x1a, "7"),
            (0x1c, "8"),
            (0x19, "9"),
            (0x32, "`"),
            (0x2a, "\\"),
            (0x21, "["),
            (0x1e, "]"),
            (0x2b, ","),
            (0x18, "="),
            (0x1b, "-"),
            (0x2f, "."),
            (0x27, "'"),
            (0x29, ";"),
            (0x2c, "/"),
        ] {
            assert_eq!(
                physical_key(code).us_logical_key(),
                LogicalKey::Character(label.into())
            );
        }
        assert_eq!(physical_key(0x52), KeyCode::Numpad0);
        assert_eq!(physical_key(0x45), KeyCode::NumpadAdd);
        assert_eq!(physical_key(0x5d), KeyCode::IntlYen);
    }

    #[test]
    fn appkit_transitions_keep_layout_modifiers_repeat_and_capture_time() {
        let down = appkit_event(
            appkit::NSEventType::KeyDown,
            0x0c,
            "\u{1}",
            "A\u{300}",
            NSEventModifierFlags::Command
                | NSEventModifierFlags::Option
                | NSEventModifierFlags::Control
                | NSEventModifierFlags::Shift,
            true,
        );
        assert_eq!(
            transition(&down, true),
            WindowEvent::Key {
                code: KeyCode::KeyQ,
                logical: LogicalKey::Character("à".into()),
                modifiers: KeyModifiers {
                    shift: true,
                    control: true,
                    alt: true,
                    meta: true,
                },
                pressed: true,
                repeat: true,
                timestamp: Duration::from_millis(12_125),
            }
        );
        let up = appkit_event(
            appkit::NSEventType::KeyUp,
            0x0c,
            "à",
            "à",
            NSEventModifierFlags::empty(),
            false,
        );
        assert_eq!(
            transition(&up, false),
            WindowEvent::Key {
                code: KeyCode::KeyQ,
                logical: LogicalKey::Character("à".into()),
                modifiers: KeyModifiers::default(),
                pressed: false,
                repeat: false,
                timestamp: Duration::from_millis(12_125),
            }
        );
    }

    #[test]
    fn normalizes_named_controls_including_backtab_and_keypad_enter() {
        for (code, character, flags, expected) in [
            (0x30, "\t", NSEventModifierFlags::empty(), KeyCode::Tab),
            (0x30, "\u{19}", NSEventModifierFlags::Shift, KeyCode::Tab),
            (0x24, "\r", NSEventModifierFlags::empty(), KeyCode::Enter),
            (
                0x4c,
                "\u{3}",
                NSEventModifierFlags::NumericPad,
                KeyCode::Enter,
            ),
            (0x31, " ", NSEventModifierFlags::empty(), KeyCode::Space),
            (
                0x33,
                "\u{7f}",
                NSEventModifierFlags::empty(),
                KeyCode::Backspace,
            ),
            (
                0x35,
                "\u{1b}",
                NSEventModifierFlags::empty(),
                KeyCode::Escape,
            ),
            (
                0x75,
                "\u{f728}",
                NSEventModifierFlags::Function,
                KeyCode::Delete,
            ),
            (
                0x7b,
                "\u{f702}",
                NSEventModifierFlags::NumericPad,
                KeyCode::ArrowLeft,
            ),
            (
                0x7c,
                "\u{f703}",
                NSEventModifierFlags::NumericPad,
                KeyCode::ArrowRight,
            ),
            (
                0x7d,
                "\u{f701}",
                NSEventModifierFlags::NumericPad,
                KeyCode::ArrowDown,
            ),
            (
                0x7e,
                "\u{f700}",
                NSEventModifierFlags::NumericPad,
                KeyCode::ArrowUp,
            ),
            (
                0x7a,
                "\u{f704}",
                NSEventModifierFlags::Function,
                KeyCode::F1,
            ),
        ] {
            let input = appkit_event(
                appkit::NSEventType::KeyDown,
                code,
                character,
                character,
                flags,
                false,
            );
            assert_eq!(
                transition(&input, true),
                WindowEvent::Key {
                    code: expected,
                    logical: LogicalKey::Named(expected),
                    modifiers: KeyModifiers {
                        shift: flags.contains(NSEventModifierFlags::Shift),
                        ..KeyModifiers::default()
                    },
                    pressed: true,
                    repeat: false,
                    timestamp: Duration::from_millis(12_125),
                }
            );
        }
    }

    #[test]
    fn logical_and_physical_keys_can_be_unidentified_independently() {
        let dead_key = appkit_event(
            appkit::NSEventType::KeyDown,
            0x0e,
            "",
            "",
            NSEventModifierFlags::Option,
            false,
        );
        assert!(matches!(
            transition(&dead_key, true),
            WindowEvent::Key {
                code: KeyCode::KeyE,
                logical: LogicalKey::Unidentified,
                ..
            }
        ));
        let unknown_position = appkit_event(
            appkit::NSEventType::KeyDown,
            0xff,
            "?",
            "?",
            NSEventModifierFlags::Shift,
            false,
        );
        assert!(matches!(
            transition(&unknown_position, true),
            WindowEvent::Key {
                code: KeyCode::Unidentified,
                logical: LogicalKey::Character(character),
                ..
            } if character == "?"
        ));
        assert_eq!(logical_key("\u{f747}"), LogicalKey::Unidentified);
    }
}
