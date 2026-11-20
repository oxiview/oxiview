use oxiview_platform::{KeyCode, KeyModifiers, LogicalKey};
use std::collections::HashSet;

/// The logical or physical identity matched by a shortcut chord.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ShortcutKey {
    /// Match the named key or character produced by the current layout.
    Logical(LogicalKey),
    /// Match a keyboard position independently of the active layout.
    Physical(KeyCode),
}

/// One shortcut stroke with an exact modifier set.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct KeyChord {
    /// Key identity selected by the binding author.
    pub key: ShortcutKey,
    /// Modifiers that must all match the captured transition.
    pub modifiers: KeyModifiers,
}

impl KeyChord {
    /// Matches a layout-aware shortcut key.
    pub fn logical(key: LogicalKey, modifiers: KeyModifiers) -> Self {
        Self {
            key: ShortcutKey::Logical(key),
            modifiers,
        }
    }

    /// Matches a physical shortcut position.
    pub fn physical(code: KeyCode, modifiers: KeyModifiers) -> Self {
        Self {
            key: ShortcutKey::Physical(code),
            modifiers,
        }
    }

    fn is_identified(&self) -> bool {
        match &self.key {
            ShortcutKey::Physical(KeyCode::Unidentified)
            | ShortcutKey::Logical(
                LogicalKey::Unidentified | LogicalKey::Named(KeyCode::Unidentified),
            ) => false,
            ShortcutKey::Logical(LogicalKey::Character(character)) => !character.is_empty(),
            _ => true,
        }
    }

    fn reserved_for_tab(&self) -> bool {
        matches!(
            self.key,
            ShortcutKey::Logical(LogicalKey::Named(KeyCode::Tab))
                | ShortcutKey::Physical(KeyCode::Tab)
        ) && !(self.modifiers.control || self.modifiers.alt || self.modifiers.meta)
    }

    fn matches(&self, code: KeyCode, logical: &LogicalKey, modifiers: KeyModifiers) -> bool {
        self.modifiers == modifiers
            && match &self.key {
                ShortcutKey::Physical(physical) => *physical == code,
                ShortcutKey::Logical(key) => key == logical,
            }
    }
}

/// A shortcut sequence for a named Action in one focus context.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyBinding {
    /// Stable identifier of the Action to invoke.
    pub action: String,
    /// Ordered strokes of the shortcut.
    pub sequence: Vec<KeyChord>,
    /// A one-stroke shortcut may invoke its Action on key repeats.
    pub repeat: bool,
}

impl KeyBinding {
    /// Creates a binding that fires on a fresh press.
    pub fn new(action: impl Into<String>, sequence: impl IntoIterator<Item = KeyChord>) -> Self {
        Self {
            action: action.into(),
            sequence: sequence.into_iter().collect(),
            repeat: false,
        }
    }

    /// Allows key repeats for this one-stroke binding.
    pub fn repeatable(mut self) -> Self {
        self.repeat = true;
        self
    }
}

/// A rejected default Keymap or a rejected transaction of user overrides.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum KeymapError {
    /// The binding has an invalid Action identifier, sequence, or repeat policy.
    #[error("invalid shortcut for {action:?}: {reason}")]
    InvalidBinding {
        /// Identifier supplied by the binding.
        action: String,
        /// Why the binding was rejected.
        reason: &'static str,
    },
    /// The two bindings are equal or one is a complete prefix of the other.
    #[error("shortcut for {:?} conflicts with {:?}", .first.action, .second.action)]
    Conflict {
        /// Earlier effective binding.
        first: Box<KeyBinding>,
        /// Later effective binding.
        second: Box<KeyBinding>,
    },
}

/// Validated default and user shortcut layers for one focus context.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Keymap {
    defaults: Vec<KeyBinding>,
    effective: Vec<KeyBinding>,
}

impl Keymap {
    /// Validates and installs default bindings for one context.
    pub fn new(bindings: impl IntoIterator<Item = KeyBinding>) -> Result<Self, KeymapError> {
        let defaults: Vec<_> = bindings.into_iter().collect();
        validate(&defaults)?;
        Ok(Self {
            effective: defaults.clone(),
            defaults,
        })
    }

    /// Replaces the override layer as a transaction. Any overridden Action's
    /// defaults are omitted. An empty layer restores all defaults.
    pub fn set_overrides(
        &mut self,
        bindings: impl IntoIterator<Item = KeyBinding>,
    ) -> Result<(), KeymapError> {
        let mut effective: Vec<_> = bindings.into_iter().collect();
        let overridden: HashSet<_> = effective.iter().map(|binding| &binding.action).collect();
        let remaining = self
            .defaults
            .iter()
            .filter(|binding| !overridden.contains(&binding.action))
            .cloned()
            .collect::<Vec<_>>();
        effective.extend(remaining);
        validate(&effective)?;
        self.effective = effective;
        Ok(())
    }

    /// Reads the active bindings in priority order. An override has priority
    /// over defaults, including when distinct logical and physical chords
    /// match the same input on the current keyboard layout.
    pub fn effective_bindings(&self) -> impl ExactSizeIterator<Item = &KeyBinding> {
        self.effective.iter()
    }

    pub(super) fn matching_binding(
        &self,
        prefix: &[KeyChord],
        code: KeyCode,
        logical: &LogicalKey,
        modifiers: KeyModifiers,
    ) -> Option<(&KeyBinding, &KeyChord)> {
        self.effective.iter().find_map(|binding| {
            let chord = binding.sequence.get(prefix.len())?;
            (binding.sequence.starts_with(prefix) && chord.matches(code, logical, modifiers))
                .then_some((binding, chord))
        })
    }
}

fn validate(bindings: &[KeyBinding]) -> Result<(), KeymapError> {
    for (index, binding) in bindings.iter().enumerate() {
        let invalid = if binding.action.trim().is_empty() {
            Some("Action identifier is empty")
        } else if binding.sequence.is_empty() {
            Some("sequence is empty")
        } else if binding.sequence.iter().any(|chord| !chord.is_identified()) {
            Some("sequence contains an unidentified key")
        } else if binding.sequence.iter().any(KeyChord::reserved_for_tab) {
            Some("Tab traversal reserves this stroke")
        } else if binding.repeat && binding.sequence.len() != 1 {
            Some("repeats require a one-stroke sequence")
        } else {
            None
        };
        if let Some(reason) = invalid {
            return Err(KeymapError::InvalidBinding {
                action: binding.action.clone(),
                reason,
            });
        }
        if let Some(first) = bindings.iter().take(index).find(|first| {
            first.sequence.starts_with(&binding.sequence)
                || binding.sequence.starts_with(&first.sequence)
        }) {
            return Err(KeymapError::Conflict {
                first: Box::new(first.clone()),
                second: Box::new(binding.clone()),
            });
        }
    }
    Ok(())
}
