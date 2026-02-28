//! Soft key UI model, presets, and wire format conversion
//!
//! Bridges between the HID protocol's wire format and the settings UI.

use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use super::keycodes::{compose_keycode, decompose_keycode, KeyModifiers, QmkKeycode};
use super::protocol::{SoftKeyConfig, SoftKeyType};

/// A single key with optional modifiers
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeycodeEntry {
    pub base_key: QmkKeycode,
    pub modifiers: KeyModifiers,
}

impl KeycodeEntry {
    pub fn new(base_key: QmkKeycode) -> Self {
        Self {
            base_key,
            modifiers: KeyModifiers::default(),
        }
    }

    pub fn with_mods(base_key: QmkKeycode, modifiers: KeyModifiers) -> Self {
        Self { base_key, modifiers }
    }

    /// Human-readable display (e.g. "Ctrl+C")
    pub fn display(&self) -> String {
        format!("{}{}", self.modifiers.display_prefix(), self.base_key.display_name())
    }
}

/// Editable soft key state for the UI
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SoftKeyEditState {
    /// Use firmware default — carries resolved keycode from device (display-only)
    Default(Option<KeycodeEntry>),
    /// Single key with optional modifiers
    Keycode(KeycodeEntry),
    /// Typed string (max 126 chars) with optional Enter after
    Text(String, bool),
    /// Tap sequence of keys (max 63 steps)
    Sequence(Vec<KeycodeEntry>),
}

impl SoftKeyEditState {
    /// Decode wire bytes from a SoftKeyConfig into UI model
    pub fn from_config(config: &SoftKeyConfig) -> Self {
        match config.key_type {
            SoftKeyType::Default => {
                // 0x06 ResetSoftKeys response includes resolved keycodes; 0x05 ReadSoftKeys does not
                let resolved = if config.data.len() >= 2 {
                    let keycode = u16::from_be_bytes([config.data[0], config.data[1]]);
                    let (base, mods) = decompose_keycode(keycode);
                    base.map(|key| KeycodeEntry::with_mods(key, mods))
                } else {
                    None
                };
                SoftKeyEditState::Default(resolved)
            }
            SoftKeyType::Keycode => {
                if config.data.len() >= 2 {
                    let keycode = u16::from_be_bytes([config.data[0], config.data[1]]);
                    let (base, mods) = decompose_keycode(keycode);
                    if let Some(key) = base {
                        SoftKeyEditState::Keycode(KeycodeEntry::with_mods(key, mods))
                    } else {
                        SoftKeyEditState::Default(None)
                    }
                } else {
                    SoftKeyEditState::Default(None)
                }
            }
            SoftKeyType::String => {
                // Wire format: [flags, string_bytes...]
                // flags bit 0: send Enter after string
                if config.data.is_empty() {
                    return SoftKeyEditState::Text(String::new(), false);
                }
                let flags = config.data[0];
                let send_enter = flags & 0x01 != 0;
                let str_bytes = &config.data[1..];
                let end = str_bytes.iter().position(|&b| b == 0).unwrap_or(str_bytes.len());
                let s = String::from_utf8_lossy(&str_bytes[..end]).to_string();
                SoftKeyEditState::Text(s, send_enter)
            }
            SoftKeyType::Sequence => {
                if config.data.is_empty() {
                    return SoftKeyEditState::Sequence(vec![]);
                }
                let count = config.data[0] as usize;
                let mut entries = Vec::with_capacity(count);
                let pairs = &config.data[1..];
                for i in 0..count {
                    let offset = i * 2;
                    if offset + 1 < pairs.len() {
                        let keycode = u16::from_be_bytes([pairs[offset], pairs[offset + 1]]);
                        let (base, mods) = decompose_keycode(keycode);
                        if let Some(key) = base {
                            entries.push(KeycodeEntry::with_mods(key, mods));
                        }
                    }
                }
                SoftKeyEditState::Sequence(entries)
            }
        }
    }

    /// Encode back to wire format: (type, data bytes)
    pub fn to_wire_data(&self) -> (SoftKeyType, Vec<u8>) {
        match self {
            SoftKeyEditState::Default(_) => (SoftKeyType::Default, vec![]),
            SoftKeyEditState::Keycode(entry) => {
                let keycode = compose_keycode(entry.base_key, entry.modifiers);
                let bytes = keycode.to_be_bytes();
                (SoftKeyType::Keycode, bytes.to_vec())
            }
            SoftKeyEditState::Text(s, send_enter) => {
                let flags: u8 = if *send_enter { 0x01 } else { 0x00 };
                let mut data = vec![flags];
                let mut str_bytes = s.as_bytes().to_vec();
                str_bytes.truncate(126);
                data.extend_from_slice(&str_bytes);
                data.push(0); // null terminator
                (SoftKeyType::String, data)
            }
            SoftKeyEditState::Sequence(entries) => {
                let count = entries.len().min(63) as u8;
                let mut data = vec![count];
                for entry in entries.iter().take(63) {
                    let keycode = compose_keycode(entry.base_key, entry.modifiers);
                    let bytes = keycode.to_be_bytes();
                    data.push(bytes[0]);
                    data.push(bytes[1]);
                }
                (SoftKeyType::Sequence, data)
            }
        }
    }

    /// Human-readable summary for display
    pub fn summary(&self) -> String {
        match self {
            SoftKeyEditState::Default(Some(entry)) => format!("Default ({})", entry.display()),
            SoftKeyEditState::Default(None) => "Default".to_string(),
            SoftKeyEditState::Keycode(entry) => entry.display(),
            SoftKeyEditState::Text(s, _) => {
                if s.len() > 20 {
                    format!("\"{}...\"", &s[..17])
                } else {
                    format!("\"{}\"", s)
                }
            }
            SoftKeyEditState::Sequence(entries) => {
                if entries.is_empty() {
                    "Empty sequence".to_string()
                } else {
                    let parts: Vec<String> = entries.iter().take(3).map(|e| e.display()).collect();
                    if entries.len() > 3 {
                        format!("{} +{} more", parts.join(", "), entries.len() - 3)
                    } else {
                        parts.join(", ")
                    }
                }
            }
        }
    }

    /// Type name for the combo box
    pub fn type_name(&self) -> &'static str {
        match self {
            SoftKeyEditState::Default(_) => "Default",
            SoftKeyEditState::Keycode(_) => "Key",
            SoftKeyEditState::Text(..) => "Text",
            SoftKeyEditState::Sequence(_) => "Sequence",
        }
    }

    /// Type index for combo box selection
    pub fn type_index(&self) -> usize {
        match self {
            SoftKeyEditState::Default(_) => 0,
            SoftKeyEditState::Keycode(_) => 1,
            SoftKeyEditState::Text(..) => 2,
            SoftKeyEditState::Sequence(_) => 3,
        }
    }

    /// Create a default value for a given type index
    pub fn default_for_type(type_index: usize) -> Self {
        match type_index {
            1 => SoftKeyEditState::Keycode(KeycodeEntry::new(QmkKeycode::A)),
            2 => SoftKeyEditState::Text(String::new(), true),
            3 => SoftKeyEditState::Sequence(vec![]),
            _ => SoftKeyEditState::Default(None),
        }
    }
}

/// A named preset of 3 soft key configurations
pub struct SoftKeyPreset {
    pub name: &'static str,
    pub description: &'static str,
    pub keys: [SoftKeyEditState; 3],
}

/// Built-in presets
pub fn presets() -> Vec<SoftKeyPreset> {
    vec![
        SoftKeyPreset {
            name: "Default",
            description: "Esc+Esc (rewind), Ctrl+O (verbose), Ctrl+B (background)",
            keys: [
                SoftKeyEditState::Sequence(vec![
                    KeycodeEntry::new(QmkKeycode::Escape),
                    KeycodeEntry::new(QmkKeycode::Escape),
                ]),
                SoftKeyEditState::Keycode(KeycodeEntry::with_mods(
                    QmkKeycode::O,
                    KeyModifiers { ctrl: true, ..Default::default() },
                )),
                SoftKeyEditState::Keycode(KeycodeEntry::with_mods(
                    QmkKeycode::B,
                    KeyModifiers { ctrl: true, ..Default::default() },
                )),
            ],
        },
        SoftKeyPreset {
            name: "Vim",
            description: "Ctrl+G (vim mode), :w (save), :q (quit)",
            keys: [
                SoftKeyEditState::Keycode(KeycodeEntry::with_mods(
                    QmkKeycode::G,
                    KeyModifiers { ctrl: true, ..Default::default() },
                )),
                SoftKeyEditState::Text(":w".to_string(), true),
                SoftKeyEditState::Text(":q".to_string(), true),
            ],
        },
        SoftKeyPreset {
            name: "Git",
            description: "/commit, /pr, Esc+Esc (rewind)",
            keys: [
                SoftKeyEditState::Text("/commit".to_string(), true),
                SoftKeyEditState::Text("/pr".to_string(), true),
                SoftKeyEditState::Sequence(vec![
                    KeycodeEntry::new(QmkKeycode::Escape),
                    KeycodeEntry::new(QmkKeycode::Escape),
                ]),
            ],
        },
        SoftKeyPreset {
            name: "Context",
            description: "/compact, /clear, /memory",
            keys: [
                SoftKeyEditState::Text("/compact".to_string(), true),
                SoftKeyEditState::Text("/clear".to_string(), true),
                SoftKeyEditState::Text("/memory".to_string(), true),
            ],
        },
        SoftKeyPreset {
            name: "GSD",
            description: "discuss-phase, plan-phase, execute-phase",
            keys: [
                SoftKeyEditState::Text("/gsd:discuss-phase ".to_string(), false),
                SoftKeyEditState::Text("/gsd:plan-phase ".to_string(), false),
                SoftKeyEditState::Text("/gsd:execute-phase ".to_string(), false),
            ],
        },
    ]
}

/// Built-in preset names (case-insensitive protection)
pub const BUILTIN_PRESET_NAMES: &[&str] = &["Default", "Vim", "Git", "Context", "GSD"];

/// Check if a name matches a built-in preset (case-insensitive)
pub fn is_builtin_preset_name(name: &str) -> bool {
    BUILTIN_PRESET_NAMES
        .iter()
        .any(|b| b.eq_ignore_ascii_case(name))
}

// --- TOML-friendly serialization types ---

/// Flat representation of a soft key for TOML serialization
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SoftKeySerde {
    key_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    base_key: Option<QmkKeycode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    modifiers: Option<KeyModifiers>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    send_enter: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sequence: Option<Vec<KeycodeEntry>>,
}

impl SoftKeySerde {
    fn from_edit_state(state: &SoftKeyEditState) -> Self {
        match state {
            SoftKeyEditState::Default(_) => SoftKeySerde {
                key_type: "Default".to_string(),
                base_key: None,
                modifiers: None,
                text: None,
                send_enter: None,
                sequence: None,
            },
            SoftKeyEditState::Keycode(entry) => SoftKeySerde {
                key_type: "Keycode".to_string(),
                base_key: Some(entry.base_key),
                modifiers: Some(entry.modifiers),
                text: None,
                send_enter: None,
                sequence: None,
            },
            SoftKeyEditState::Text(s, enter) => SoftKeySerde {
                key_type: "Text".to_string(),
                base_key: None,
                modifiers: None,
                text: Some(s.clone()),
                send_enter: Some(*enter),
                sequence: None,
            },
            SoftKeyEditState::Sequence(entries) => SoftKeySerde {
                key_type: "Sequence".to_string(),
                base_key: None,
                modifiers: None,
                text: None,
                send_enter: None,
                sequence: Some(entries.clone()),
            },
        }
    }

    fn to_edit_state(&self) -> SoftKeyEditState {
        match self.key_type.as_str() {
            "Keycode" => {
                if let Some(base_key) = self.base_key {
                    let modifiers = self.modifiers.unwrap_or_default();
                    SoftKeyEditState::Keycode(KeycodeEntry::with_mods(base_key, modifiers))
                } else {
                    SoftKeyEditState::Default(None)
                }
            }
            "Text" => SoftKeyEditState::Text(
                self.text.clone().unwrap_or_default(),
                self.send_enter.unwrap_or(false),
            ),
            "Sequence" => {
                SoftKeyEditState::Sequence(self.sequence.clone().unwrap_or_default())
            }
            _ => SoftKeyEditState::Default(None),
        }
    }
}

/// TOML-serializable preset
#[derive(Debug, Clone, Serialize, Deserialize)]
struct UserPresetSerde {
    name: String,
    keys: Vec<SoftKeySerde>,
}

/// A user-defined preset of 3 soft key configurations
#[derive(Debug, Clone)]
pub struct UserPreset {
    pub name: String,
    pub keys: [SoftKeyEditState; 3],
}

/// TOML file wrapper
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct PresetFile {
    #[serde(default)]
    presets: Vec<UserPresetSerde>,
}

/// Manages user-defined soft key presets with TOML persistence
#[derive(Debug, Clone, Default)]
pub struct PresetManager {
    presets: Vec<UserPreset>,
}

impl PresetManager {
    /// Load presets from disk (returns Default if file missing or invalid)
    pub fn load() -> Result<Self> {
        let path = Self::data_path()?;
        if path.exists() {
            let content = std::fs::read_to_string(&path)
                .with_context(|| format!("Failed to read preset file: {:?}", path))?;
            let file: PresetFile = toml::from_str(&content)
                .with_context(|| format!("Failed to parse preset file: {:?}", path))?;
            let presets = file
                .presets
                .into_iter()
                .filter_map(|p| {
                    if p.keys.len() == 3 {
                        Some(UserPreset {
                            name: p.name,
                            keys: [
                                p.keys[0].to_edit_state(),
                                p.keys[1].to_edit_state(),
                                p.keys[2].to_edit_state(),
                            ],
                        })
                    } else {
                        None
                    }
                })
                .collect();
            Ok(Self { presets })
        } else {
            Ok(Self::default())
        }
    }

    /// Save presets to disk
    pub fn save(&self) -> Result<()> {
        let path = Self::data_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create preset directory: {:?}", parent))?;
        }
        let file = PresetFile {
            presets: self
                .presets
                .iter()
                .map(|p| UserPresetSerde {
                    name: p.name.clone(),
                    keys: p.keys.iter().map(SoftKeySerde::from_edit_state).collect(),
                })
                .collect(),
        };
        let content = toml::to_string_pretty(&file).context("Failed to serialize presets")?;
        std::fs::write(&path, content)
            .with_context(|| format!("Failed to write preset file: {:?}", path))?;
        Ok(())
    }

    fn data_path() -> Result<PathBuf> {
        let proj_dirs = ProjectDirs::from("com", "agentdeck", "AgentDeck")
            .context("Failed to determine data directory")?;
        Ok(proj_dirs.data_dir().join("soft_key_presets.toml"))
    }

    /// All user presets
    pub fn all(&self) -> &[UserPreset] {
        &self.presets
    }

    /// Add or upsert a preset (overwrites if name exists)
    pub fn add(&mut self, name: String, keys: [SoftKeyEditState; 3]) {
        if let Some(existing) = self.presets.iter_mut().find(|p| p.name == name) {
            existing.keys = keys;
        } else {
            self.presets.push(UserPreset { name, keys });
        }
    }

    /// Remove a preset by name, returns true if found
    pub fn remove(&mut self, name: &str) -> bool {
        let len_before = self.presets.len();
        self.presets.retain(|p| p.name != name);
        self.presets.len() < len_before
    }

    /// Rename a preset, returns true if successful
    pub fn rename(&mut self, old_name: &str, new_name: String) -> bool {
        if let Some(preset) = self.presets.iter_mut().find(|p| p.name == old_name) {
            preset.name = new_name;
            true
        } else {
            false
        }
    }

    /// Find a preset by name
    pub fn find(&self, name: &str) -> Option<&UserPreset> {
        self.presets.iter().find(|p| p.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_roundtrip() {
        let config = SoftKeyConfig {
            index: 0,
            key_type: SoftKeyType::Default,
            data: vec![],
        };
        let state = SoftKeyEditState::from_config(&config);
        assert_eq!(state, SoftKeyEditState::Default(None));

        let (ty, data) = state.to_wire_data();
        assert_eq!(ty, SoftKeyType::Default);
        assert!(data.is_empty());
    }

    #[test]
    fn test_keycode_roundtrip() {
        let entry = KeycodeEntry::with_mods(
            QmkKeycode::C,
            KeyModifiers { ctrl: true, ..Default::default() },
        );
        let state = SoftKeyEditState::Keycode(entry.clone());
        let (ty, data) = state.to_wire_data();
        assert_eq!(ty, SoftKeyType::Keycode);
        assert_eq!(data.len(), 2);

        // Reconstruct as if read from device
        let config = SoftKeyConfig {
            index: 0,
            key_type: SoftKeyType::Keycode,
            data,
        };
        let parsed = SoftKeyEditState::from_config(&config);
        assert_eq!(parsed, SoftKeyEditState::Keycode(entry));
    }

    #[test]
    fn test_string_roundtrip() {
        let state = SoftKeyEditState::Text("hello".to_string(), true);
        let (ty, data) = state.to_wire_data();
        assert_eq!(ty, SoftKeyType::String);
        // First byte is flags (0x01 = send_enter), last byte is null terminator
        assert_eq!(data[0], 0x01);
        assert_eq!(*data.last().unwrap(), 0);

        let config = SoftKeyConfig {
            index: 0,
            key_type: SoftKeyType::String,
            data,
        };
        let parsed = SoftKeyEditState::from_config(&config);
        assert_eq!(parsed, SoftKeyEditState::Text("hello".to_string(), true));
    }

    #[test]
    fn test_string_no_enter_roundtrip() {
        let state = SoftKeyEditState::Text("test".to_string(), false);
        let (ty, data) = state.to_wire_data();
        assert_eq!(data[0], 0x00); // flags: no enter

        let config = SoftKeyConfig {
            index: 0,
            key_type: SoftKeyType::String,
            data,
        };
        let parsed = SoftKeyEditState::from_config(&config);
        assert_eq!(parsed, SoftKeyEditState::Text("test".to_string(), false));
    }

    #[test]
    fn test_sequence_roundtrip() {
        let entries = vec![
            KeycodeEntry::new(QmkKeycode::Escape),
            KeycodeEntry::new(QmkKeycode::Escape),
        ];
        let state = SoftKeyEditState::Sequence(entries.clone());
        let (ty, data) = state.to_wire_data();
        assert_eq!(ty, SoftKeyType::Sequence);
        assert_eq!(data[0], 2); // count

        let config = SoftKeyConfig {
            index: 0,
            key_type: SoftKeyType::Sequence,
            data,
        };
        let parsed = SoftKeyEditState::from_config(&config);
        assert_eq!(parsed, SoftKeyEditState::Sequence(entries));
    }

    #[test]
    fn test_summary() {
        assert_eq!(SoftKeyEditState::Default(None).summary(), "Default");

        let entry = KeycodeEntry::with_mods(
            QmkKeycode::C,
            KeyModifiers { ctrl: true, ..Default::default() },
        );
        assert_eq!(SoftKeyEditState::Keycode(entry).summary(), "Ctrl+C");

        assert_eq!(SoftKeyEditState::Text("hi".to_string(), false).summary(), "\"hi\"");

        let seq = SoftKeyEditState::Sequence(vec![
            KeycodeEntry::new(QmkKeycode::Escape),
        ]);
        assert_eq!(seq.summary(), "Escape");
    }

    #[test]
    fn test_presets_valid() {
        let p = presets();
        assert!(p.len() >= 4);
        // All presets should have valid wire roundtrips
        for preset in &p {
            for key in &preset.keys {
                let (ty, data) = key.to_wire_data();
                let config = SoftKeyConfig { index: 0, key_type: ty, data };
                let parsed = SoftKeyEditState::from_config(&config);
                assert_eq!(&parsed, key, "Preset '{}' failed roundtrip", preset.name);
            }
        }
    }

    #[test]
    fn test_type_index() {
        assert_eq!(SoftKeyEditState::Default(None).type_index(), 0);
        assert_eq!(SoftKeyEditState::Keycode(KeycodeEntry::new(QmkKeycode::A)).type_index(), 1);
        assert_eq!(SoftKeyEditState::Text(String::new(), false).type_index(), 2);
        assert_eq!(SoftKeyEditState::Sequence(vec![]).type_index(), 3);
    }

    #[test]
    fn test_string_truncation() {
        let long = "a".repeat(200);
        let state = SoftKeyEditState::Text(long, false);
        let (_, data) = state.to_wire_data();
        // flags byte + 126 chars + null terminator
        assert_eq!(data.len(), 128);
    }

    #[test]
    fn test_is_builtin_preset_name() {
        assert!(is_builtin_preset_name("Default"));
        assert!(is_builtin_preset_name("default"));
        assert!(is_builtin_preset_name("VIM"));
        assert!(is_builtin_preset_name("Git"));
        assert!(is_builtin_preset_name("context"));
        assert!(!is_builtin_preset_name("My Custom"));
        assert!(!is_builtin_preset_name(""));
    }

    #[test]
    fn test_serde_roundtrip_all_types() {
        let keys = [
            SoftKeyEditState::Default(None),
            SoftKeyEditState::Keycode(KeycodeEntry::with_mods(
                QmkKeycode::C,
                KeyModifiers { ctrl: true, ..Default::default() },
            )),
            SoftKeyEditState::Text("hello".to_string(), true),
        ];

        // Serialize via SoftKeySerde
        let serde_keys: Vec<SoftKeySerde> = keys.iter().map(SoftKeySerde::from_edit_state).collect();
        let file = PresetFile {
            presets: vec![UserPresetSerde {
                name: "Test".to_string(),
                keys: serde_keys,
            }],
        };
        let toml_str = toml::to_string_pretty(&file).unwrap();

        // Deserialize back
        let parsed: PresetFile = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.presets.len(), 1);
        assert_eq!(parsed.presets[0].name, "Test");
        let parsed_keys: Vec<SoftKeyEditState> =
            parsed.presets[0].keys.iter().map(|s| s.to_edit_state()).collect();
        assert_eq!(parsed_keys[0], SoftKeyEditState::Default(None));
        assert_eq!(parsed_keys[1], keys[1]);
        assert_eq!(parsed_keys[2], keys[2]);
    }

    #[test]
    fn test_serde_roundtrip_sequence() {
        let keys = [
            SoftKeyEditState::Sequence(vec![
                KeycodeEntry::new(QmkKeycode::Escape),
                KeycodeEntry::new(QmkKeycode::Escape),
            ]),
            SoftKeyEditState::Default(None),
            SoftKeyEditState::Default(None),
        ];

        let serde_keys: Vec<SoftKeySerde> = keys.iter().map(SoftKeySerde::from_edit_state).collect();
        let file = PresetFile {
            presets: vec![UserPresetSerde {
                name: "Seq".to_string(),
                keys: serde_keys,
            }],
        };
        let toml_str = toml::to_string_pretty(&file).unwrap();
        let parsed: PresetFile = toml::from_str(&toml_str).unwrap();
        let parsed_keys: Vec<SoftKeyEditState> =
            parsed.presets[0].keys.iter().map(|s| s.to_edit_state()).collect();
        assert_eq!(parsed_keys[0], keys[0]);
    }

    #[test]
    fn test_preset_manager_crud() {
        let mut mgr = PresetManager::default();
        assert!(mgr.all().is_empty());

        let keys = [
            SoftKeyEditState::Text("test".to_string(), true),
            SoftKeyEditState::Default(None),
            SoftKeyEditState::Default(None),
        ];

        // Add
        mgr.add("My Preset".to_string(), keys.clone());
        assert_eq!(mgr.all().len(), 1);
        assert_eq!(mgr.find("My Preset").unwrap().name, "My Preset");

        // Upsert
        let keys2 = [
            SoftKeyEditState::Default(None),
            SoftKeyEditState::Default(None),
            SoftKeyEditState::Default(None),
        ];
        mgr.add("My Preset".to_string(), keys2.clone());
        assert_eq!(mgr.all().len(), 1);
        assert_eq!(mgr.find("My Preset").unwrap().keys, keys2);

        // Rename
        assert!(mgr.rename("My Preset", "Renamed".to_string()));
        assert!(mgr.find("My Preset").is_none());
        assert!(mgr.find("Renamed").is_some());

        // Remove
        assert!(mgr.remove("Renamed"));
        assert!(mgr.all().is_empty());
        assert!(!mgr.remove("Nonexistent"));
    }
}
