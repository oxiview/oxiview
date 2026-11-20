//! Versioned player settings. Files contain portable key identities and modifiers.

use crate::shortcuts::{self, EDITABLE};
use oxiview_core::{KeyBinding, KeyChord, Keymap, ShortcutKey};
use oxiview_platform::{KeyCode, KeyModifiers, LogicalKey};
use oxiview_style::AppearanceMode;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum StoredKey {
    Character(String),
    Named(String),
    Physical(String),
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Modifiers {
    shift: bool,
    control: bool,
    alt: bool,
    meta: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stroke {
    key: StoredKey,
    modifiers: Modifiers,
}

impl Stroke {
    fn from_chord(chord: &KeyChord) -> Result<Self, String> {
        let key = match &chord.key {
            ShortcutKey::Logical(LogicalKey::Character(s)) if !s.is_empty() => {
                StoredKey::Character(s.clone())
            }
            ShortcutKey::Logical(LogicalKey::Named(code)) if *code != KeyCode::Unidentified => {
                StoredKey::Named(code.name().into())
            }
            ShortcutKey::Physical(code) if *code != KeyCode::Unidentified => {
                StoredKey::Physical(code.name().into())
            }
            _ => return Err("This key has no shortcut identity.".into()),
        };
        let m = chord.modifiers;
        Ok(Self {
            key,
            modifiers: Modifiers {
                shift: m.shift,
                control: m.control,
                alt: m.alt,
                meta: m.meta,
            },
        })
    }

    fn chord(&self) -> Result<KeyChord, String> {
        let m = &self.modifiers;
        let modifiers = KeyModifiers {
            shift: m.shift,
            control: m.control,
            alt: m.alt,
            meta: m.meta,
        };
        let named = |value: &str| {
            KeyCode::from_name(value)
                .filter(|key| *key != KeyCode::Unidentified)
                .ok_or_else(|| format!("Unknown shortcut key {value}"))
        };
        let chord = match &self.key {
            StoredKey::Character(s) if !s.is_empty() => {
                KeyChord::logical(LogicalKey::Character(s.clone()), modifiers)
            }
            StoredKey::Character(_) => return Err("Empty shortcut character".into()),
            StoredKey::Named(s) => KeyChord::logical(LogicalKey::Named(named(s)?), modifiers),
            StoredKey::Physical(s) => KeyChord::physical(named(s)?, modifiers),
        };
        if matches!(
            &chord.key,
            ShortcutKey::Logical(LogicalKey::Named(KeyCode::Tab))
                | ShortcutKey::Physical(KeyCode::Tab)
        ) {
            return Err("Tab is reserved for moving keyboard focus.".into());
        }
        Ok(chord)
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Palette {
    #[default]
    System,
    Light,
    Dark,
}

impl Palette {
    pub(crate) fn mode(self) -> AppearanceMode {
        match self {
            Self::System => AppearanceMode::System,
            Self::Light => AppearanceMode::Light,
            Self::Dark => AppearanceMode::Dark,
        }
    }
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }
    pub(crate) fn step(self, forward: bool) -> Self {
        match (self, forward) {
            (Self::System, true) | (Self::Dark, false) => Self::Light,
            (Self::Light, true) | (Self::System, false) => Self::Dark,
            _ => Self::System,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Preferences {
    version: u32,
    pub(crate) appearance: Palette,
    shortcuts: BTreeMap<String, Stroke>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            version: 1,
            appearance: Palette::System,
            shortcuts: BTreeMap::new(),
        }
    }
}

impl Preferences {
    pub(crate) fn read(path: &Path) -> Result<Self, String> {
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(e.to_string()),
        };
        let prefs: Self = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        prefs.keymap()?;
        Ok(prefs)
    }

    pub(crate) fn keymap(&self) -> Result<Keymap, String> {
        if self.version != 1 {
            return Err(format!("Unknown settings version {}", self.version));
        }
        let mut map = shortcuts::board_keymap().map_err(|e| e.to_string())?;
        let mut overrides = Vec::new();
        for (action, value) in &self.shortcuts {
            if !EDITABLE.iter().any(|(id, _)| id == action) {
                return Err(format!("Unknown Action {action}"));
            }
            overrides.push(KeyBinding::new(action, [value.chord()?]));
        }
        map.set_overrides(overrides).map_err(|e| e.to_string())?;
        Ok(map)
    }

    pub(crate) fn rebind(&self, action: &str, chord: &KeyChord) -> Result<(Self, Keymap), String> {
        let mut next = self.clone();
        next.shortcuts
            .insert(action.into(), Stroke::from_chord(chord)?);
        let map = next.keymap()?;
        Ok((next, map))
    }

    pub(crate) fn save(&self, path: &Path) -> io::Result<()> {
        static WRITES: AtomicU64 = AtomicU64::new(0);
        let directory = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::create_dir_all(directory)?;
        let bytes = serde_json::to_vec_pretty(self)?;
        let (staging, mut file) = loop {
            let revision = WRITES.fetch_add(1, Ordering::Relaxed);
            let staging = directory.join(format!(
                ".snake-settings-{}-{revision}.tmp",
                std::process::id()
            ));
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&staging)
            {
                Ok(file) => break (staging, file),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        };
        let written = file.write_all(&bytes).and_then(|()| file.sync_all());
        drop(file);
        let result = written.and_then(|()| fs::rename(&staging, path));
        if result.is_err() {
            let _ = fs::remove_file(staging);
        }
        result
    }
}

pub(crate) fn user_path() -> Option<PathBuf> {
    let directory = if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
    } else if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    };
    directory.map(|d| d.join("oxiview/snake-m5-oxi/settings.json"))
}
