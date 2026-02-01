//! Global hotkey registration and handling

use crate::core::config::HotkeyConfig;
use crate::core::events::AppEvent;
use anyhow::{Context, Result};
use global_hotkey::{
    hotkey::{Code, HotKey, Modifiers},
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
};
use std::collections::HashMap;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

/// Type of hotkey pressed
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HotkeyType {
    /// Claude key (F20) - launches/focuses terminal
    ClaudeKey,
    /// Soft key (F15-F18) - customizable action
    SoftKey(u8),
}

/// Global hotkey manager
pub struct HotkeyManager {
    /// Global hotkey manager from the crate (kept alive for hotkey registration)
    #[allow(dead_code)]
    manager: GlobalHotKeyManager,
    /// Mapping from hotkey ID to type
    hotkey_map: HashMap<u32, HotkeyType>,
    /// Event sender
    event_tx: mpsc::UnboundedSender<AppEvent>,
}

impl HotkeyManager {
    /// Create a new hotkey manager
    pub fn new(event_tx: mpsc::UnboundedSender<AppEvent>, config: &HotkeyConfig) -> Result<Self> {
        let manager = GlobalHotKeyManager::new().context("Failed to create hotkey manager")?;
        let mut hotkey_map = HashMap::new();

        // Register Claude key
        if let Some(hotkey) = parse_hotkey(&config.claude_key) {
            match manager.register(hotkey) {
                Ok(()) => {
                    hotkey_map.insert(hotkey.id(), HotkeyType::ClaudeKey);
                    info!("Registered Claude key: {}", config.claude_key);
                }
                Err(e) => {
                    warn!("Failed to register Claude key {}: {}", config.claude_key, e);
                }
            }
        }

        // Register soft keys
        let soft_keys = [
            (&config.soft_key_1, 1u8),
            (&config.soft_key_2, 2u8),
            (&config.soft_key_3, 3u8),
            (&config.soft_key_4, 4u8),
        ];

        for (key_str, num) in soft_keys {
            if let Some(hotkey) = parse_hotkey(key_str) {
                match manager.register(hotkey) {
                    Ok(()) => {
                        hotkey_map.insert(hotkey.id(), HotkeyType::SoftKey(num));
                        info!("Registered soft key {}: {}", num, key_str);
                    }
                    Err(e) => {
                        warn!("Failed to register soft key {} ({}): {}", num, key_str, e);
                    }
                }
            }
        }

        Ok(Self {
            manager,
            hotkey_map,
            event_tx,
        })
    }

    /// Process hotkey events (call from event loop)
    pub fn process_events(&self) {
        if let Ok(event) = GlobalHotKeyEvent::receiver().try_recv() {
            debug!("Hotkey event: {:?}", event);

            // Only respond to key press, not release
            if event.state != HotKeyState::Pressed {
                return;
            }

            if let Some(hotkey_type) = self.hotkey_map.get(&event.id) {
                debug!("Hotkey pressed: {:?}", hotkey_type);
                if let Err(e) = self.event_tx.send(AppEvent::HotkeyPressed(*hotkey_type)) {
                    error!("Failed to send hotkey event: {}", e);
                }
            }
        }
    }

    /// Unregister all hotkeys
    pub fn unregister_all(&self) {
        for id in self.hotkey_map.keys() {
            // We can't easily unregister by ID, but the drop will handle cleanup
            debug!("Would unregister hotkey with id {}", id);
        }
    }
}

/// Parse a hotkey string into a HotKey
fn parse_hotkey(key_str: &str) -> Option<HotKey> {
    let key_str = key_str.trim().to_uppercase();

    // Parse key code
    let code = match key_str.as_str() {
        "F1" => Code::F1,
        "F2" => Code::F2,
        "F3" => Code::F3,
        "F4" => Code::F4,
        "F5" => Code::F5,
        "F6" => Code::F6,
        "F7" => Code::F7,
        "F8" => Code::F8,
        "F9" => Code::F9,
        "F10" => Code::F10,
        "F11" => Code::F11,
        "F12" => Code::F12,
        "F13" => Code::F13,
        "F14" => Code::F14,
        "F15" => Code::F15,
        "F16" => Code::F16,
        "F17" => Code::F17,
        "F18" => Code::F18,
        "F19" => Code::F19,
        "F20" => Code::F20,
        "F21" => Code::F21,
        "F22" => Code::F22,
        "F23" => Code::F23,
        "F24" => Code::F24,
        _ => {
            warn!("Unknown hotkey: {}", key_str);
            return None;
        }
    };

    // No modifiers for function keys from the macropad
    Some(HotKey::new(Some(Modifiers::empty()), code))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hotkey_f20() {
        let hotkey = parse_hotkey("F20");
        assert!(hotkey.is_some());
    }

    #[test]
    fn test_parse_hotkey_f15() {
        let hotkey = parse_hotkey("F15");
        assert!(hotkey.is_some());
    }

    #[test]
    fn test_parse_hotkey_unknown() {
        let hotkey = parse_hotkey("UNKNOWN");
        assert!(hotkey.is_none());
    }

    #[test]
    fn test_hotkey_type_equality() {
        assert_eq!(HotkeyType::ClaudeKey, HotkeyType::ClaudeKey);
        assert_eq!(HotkeyType::SoftKey(1), HotkeyType::SoftKey(1));
        assert_ne!(HotkeyType::SoftKey(1), HotkeyType::SoftKey(2));
        assert_ne!(HotkeyType::ClaudeKey, HotkeyType::SoftKey(1));
    }
}
