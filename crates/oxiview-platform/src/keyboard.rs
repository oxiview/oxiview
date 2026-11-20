// Keep the persisted spelling beside each variant so Rust renames can leave it intact.
macro_rules! define_key_codes {
    ($($(#[$variant_doc:meta])* $variant:ident => $name:literal),+ $(,)?) => {
        /// Layout-independent key positions. The letter, digit, and punctuation names
        /// refer to their labels on a US keyboard.
        #[non_exhaustive]
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum KeyCode {
            $($(#[$variant_doc])* $variant),+
        }

        impl KeyCode {
            /// Returns the stable, case-sensitive name for persistence.
            pub const fn name(self) -> &'static str {
                match self {
                    $(Self::$variant => $name),+
                }
            }

            /// Parses a stable name returned by `KeyCode::name`.
            pub fn from_name(name: &str) -> Option<Self> {
                match name {
                    $($name => Some(Self::$variant),)+
                    _ => None,
                }
            }
        }

        #[cfg(test)]
        const ALL_KEY_CODES: &[KeyCode] = &[$(KeyCode::$variant),+];
    };
}

define_key_codes! {
    /// Escape key.
    Escape => "Escape",
    /// Enter or Return, including keypad Enter.
    Enter => "Enter",
    /// Space bar.
    Space => "Space",
    /// Tab key.
    Tab => "Tab",
    /// Backspace, also labeled Delete on Mac keyboards.
    Backspace => "Backspace",
    /// Forward Delete key.
    Delete => "Delete",
    /// Insert key.
    Insert => "Insert",
    /// Home key.
    Home => "Home",
    /// End key.
    End => "End",
    /// Page Up key.
    PageUp => "PageUp",
    /// Page Down key.
    PageDown => "PageDown",
    /// Help key.
    Help => "Help",
    /// Keypad Clear key.
    Clear => "Clear",
    /// Left arrow.
    ArrowLeft => "ArrowLeft",
    /// Right arrow.
    ArrowRight => "ArrowRight",
    /// Up arrow.
    ArrowUp => "ArrowUp",
    /// Down arrow.
    ArrowDown => "ArrowDown",
    /// A position.
    KeyA => "KeyA",
    /// B position.
    KeyB => "KeyB",
    /// C position.
    KeyC => "KeyC",
    /// D position.
    KeyD => "KeyD",
    /// E position.
    KeyE => "KeyE",
    /// F position.
    KeyF => "KeyF",
    /// G position.
    KeyG => "KeyG",
    /// H position.
    KeyH => "KeyH",
    /// I position.
    KeyI => "KeyI",
    /// J position.
    KeyJ => "KeyJ",
    /// K position.
    KeyK => "KeyK",
    /// L position.
    KeyL => "KeyL",
    /// M position.
    KeyM => "KeyM",
    /// N position.
    KeyN => "KeyN",
    /// O position.
    KeyO => "KeyO",
    /// P position.
    KeyP => "KeyP",
    /// Q position.
    KeyQ => "KeyQ",
    /// R position.
    KeyR => "KeyR",
    /// S position.
    KeyS => "KeyS",
    /// T position.
    KeyT => "KeyT",
    /// U position.
    KeyU => "KeyU",
    /// V position.
    KeyV => "KeyV",
    /// W position.
    KeyW => "KeyW",
    /// X position.
    KeyX => "KeyX",
    /// Y position.
    KeyY => "KeyY",
    /// Z position.
    KeyZ => "KeyZ",
    /// 0 in the number row.
    Digit0 => "Digit0",
    /// 1 in the number row.
    Digit1 => "Digit1",
    /// 2 in the number row.
    Digit2 => "Digit2",
    /// 3 in the number row.
    Digit3 => "Digit3",
    /// 4 in the number row.
    Digit4 => "Digit4",
    /// 5 in the number row.
    Digit5 => "Digit5",
    /// 6 in the number row.
    Digit6 => "Digit6",
    /// 7 in the number row.
    Digit7 => "Digit7",
    /// 8 in the number row.
    Digit8 => "Digit8",
    /// 9 in the number row.
    Digit9 => "Digit9",
    /// Backquote position.
    Backquote => "Backquote",
    /// Backslash position.
    Backslash => "Backslash",
    /// Left bracket position.
    BracketLeft => "BracketLeft",
    /// Right bracket position.
    BracketRight => "BracketRight",
    /// Comma position.
    Comma => "Comma",
    /// Equals position.
    Equal => "Equal",
    /// Minus position.
    Minus => "Minus",
    /// Period position.
    Period => "Period",
    /// Quote position.
    Quote => "Quote",
    /// Semicolon position.
    Semicolon => "Semicolon",
    /// Slash position.
    Slash => "Slash",
    /// Extra ISO keyboard position next to the left Shift key.
    IntlBackslash => "IntlBackslash",
    /// Yen position on Japanese keyboards.
    IntlYen => "IntlYen",
    /// Ro position on Japanese keyboards.
    IntlRo => "IntlRo",
    /// Keypad 0.
    Numpad0 => "Numpad0",
    /// Keypad 1.
    Numpad1 => "Numpad1",
    /// Keypad 2.
    Numpad2 => "Numpad2",
    /// Keypad 3.
    Numpad3 => "Numpad3",
    /// Keypad 4.
    Numpad4 => "Numpad4",
    /// Keypad 5.
    Numpad5 => "Numpad5",
    /// Keypad 6.
    Numpad6 => "Numpad6",
    /// Keypad 7.
    Numpad7 => "Numpad7",
    /// Keypad 8.
    Numpad8 => "Numpad8",
    /// Keypad 9.
    Numpad9 => "Numpad9",
    /// Keypad addition.
    NumpadAdd => "NumpadAdd",
    /// Keypad subtraction.
    NumpadSubtract => "NumpadSubtract",
    /// Keypad multiplication.
    NumpadMultiply => "NumpadMultiply",
    /// Keypad division.
    NumpadDivide => "NumpadDivide",
    /// Keypad decimal separator.
    NumpadDecimal => "NumpadDecimal",
    /// Keypad comma.
    NumpadComma => "NumpadComma",
    /// Keypad equals.
    NumpadEqual => "NumpadEqual",
    /// First function key.
    F1 => "F1",
    /// Second function key.
    F2 => "F2",
    /// Third function key.
    F3 => "F3",
    /// Fourth function key.
    F4 => "F4",
    /// Fifth function key.
    F5 => "F5",
    /// Sixth function key.
    F6 => "F6",
    /// Seventh function key.
    F7 => "F7",
    /// Eighth function key.
    F8 => "F8",
    /// Ninth function key.
    F9 => "F9",
    /// Tenth function key.
    F10 => "F10",
    /// Eleventh function key.
    F11 => "F11",
    /// Twelfth function key.
    F12 => "F12",
    /// Thirteenth function key.
    F13 => "F13",
    /// Fourteenth function key.
    F14 => "F14",
    /// Fifteenth function key.
    F15 => "F15",
    /// Sixteenth function key.
    F16 => "F16",
    /// Seventeenth function key.
    F17 => "F17",
    /// Eighteenth function key.
    F18 => "F18",
    /// Nineteenth function key.
    F19 => "F19",
    /// Twentieth function key.
    F20 => "F20",
    /// No portable physical position is available.
    Unidentified => "Unidentified",
}

impl KeyCode {
    /// Supplies an unshifted logical identity for a synthetic US keyboard.
    /// Layout-specific positions without a US equivalent are unidentified.
    pub fn us_logical_key(self) -> LogicalKey {
        let character = match self {
            Self::KeyA => "a",
            Self::KeyB => "b",
            Self::KeyC => "c",
            Self::KeyD => "d",
            Self::KeyE => "e",
            Self::KeyF => "f",
            Self::KeyG => "g",
            Self::KeyH => "h",
            Self::KeyI => "i",
            Self::KeyJ => "j",
            Self::KeyK => "k",
            Self::KeyL => "l",
            Self::KeyM => "m",
            Self::KeyN => "n",
            Self::KeyO => "o",
            Self::KeyP => "p",
            Self::KeyQ => "q",
            Self::KeyR => "r",
            Self::KeyS => "s",
            Self::KeyT => "t",
            Self::KeyU => "u",
            Self::KeyV => "v",
            Self::KeyW => "w",
            Self::KeyX => "x",
            Self::KeyY => "y",
            Self::KeyZ => "z",
            Self::Digit0 | Self::Numpad0 => "0",
            Self::Digit1 | Self::Numpad1 => "1",
            Self::Digit2 | Self::Numpad2 => "2",
            Self::Digit3 | Self::Numpad3 => "3",
            Self::Digit4 | Self::Numpad4 => "4",
            Self::Digit5 | Self::Numpad5 => "5",
            Self::Digit6 | Self::Numpad6 => "6",
            Self::Digit7 | Self::Numpad7 => "7",
            Self::Digit8 | Self::Numpad8 => "8",
            Self::Digit9 | Self::Numpad9 => "9",
            Self::Backquote => "`",
            Self::Backslash => "\\",
            Self::BracketLeft => "[",
            Self::BracketRight => "]",
            Self::Comma | Self::NumpadComma => ",",
            Self::Equal | Self::NumpadEqual => "=",
            Self::Minus | Self::NumpadSubtract => "-",
            Self::Period | Self::NumpadDecimal => ".",
            Self::Quote => "'",
            Self::Semicolon => ";",
            Self::Slash | Self::NumpadDivide => "/",
            Self::NumpadAdd => "+",
            Self::NumpadMultiply => "*",
            Self::IntlBackslash | Self::IntlYen | Self::IntlRo | Self::Unidentified => {
                return LogicalKey::Unidentified;
            }
            _ => return LogicalKey::Named(self),
        };
        LogicalKey::Character(character.into())
    }
}

/// Logical identity for shortcut resolution in the current keyboard layout.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum LogicalKey {
    /// A named control key, including Space.
    Named(KeyCode),
    /// Layout-produced shortcut characters with letter case normalized to lowercase.
    Character(String),
    /// No logical identity is available.
    Unidentified,
}

/// State of the four shortcut modifiers when a key event was captured.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct KeyModifiers {
    /// Shift.
    pub shift: bool,
    /// Control.
    pub control: bool,
    /// Alt, called Option on macOS.
    pub alt: bool,
    /// Meta, called Command on macOS.
    pub meta: bool,
}

/// Desktop conventions used to construct default shortcuts, independently of the event source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShortcutPlatform {
    /// Command-based application shortcuts.
    MacOs,
    /// Control-based shortcuts and Alt mnemonics.
    Windows,
    /// Control-based application shortcuts.
    Linux,
}

impl ShortcutPlatform {
    /// Selects defaults for the build target. Headless callers can choose any profile.
    pub const fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::MacOs
        } else if cfg!(target_os = "windows") {
            Self::Windows
        } else {
            Self::Linux
        }
    }

    /// Modifier for the platform's primary application shortcuts.
    pub const fn primary_modifier(self) -> KeyModifiers {
        KeyModifiers {
            shift: false,
            alt: false,
            control: matches!(self, Self::Windows | Self::Linux),
            meta: matches!(self, Self::MacOs),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ALL_KEY_CODES, KeyCode};

    #[test]
    fn names_round_trip_for_every_key_code() {
        for &code in ALL_KEY_CODES {
            assert_eq!(KeyCode::from_name(code.name()), Some(code));
        }
    }

    #[test]
    fn persisted_names_are_exact() {
        for (code, name) in [
            (KeyCode::ArrowUp, "ArrowUp"),
            (KeyCode::KeyW, "KeyW"),
            (KeyCode::F1, "F1"),
            (KeyCode::Unidentified, "Unidentified"),
        ] {
            assert_eq!(code.name(), name);
            assert_eq!(KeyCode::from_name(name), Some(code));
        }
        for invalid in ["", "arrowup", "KeyW ", "F21", "VK_0D"] {
            assert_eq!(KeyCode::from_name(invalid), None);
        }
    }
}
