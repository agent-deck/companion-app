//! HID command helpers
//!
//! Convenience functions for building common HID commands.
//! All builders return `Vec<HidPacket>` using the chunked protocol.

#![allow(dead_code)]

use super::protocol::{build_chunked_packets, DeviceMode, HidCommand, HidPacket, SoftKeyType};
use crate::core::state::ClaudeState;

/// Build a display update from Claude state (chunked across packets)
pub fn build_display_update(state: &ClaudeState) -> Vec<HidPacket> {
    let json = serde_json::json!({
        "task": truncate(&state.task, 64),
        "model": truncate(&state.model, 64),
        "progress": state.progress.min(100),
        "tokens": truncate(&state.tokens, 16),
        "cost": truncate(&state.cost, 16),
    });

    build_chunked_packets(HidCommand::UpdateDisplay, json.to_string().as_bytes())
}

/// Build a ping packet (single packet)
pub fn build_ping() -> Vec<HidPacket> {
    build_chunked_packets(HidCommand::Ping, &[])
}

/// Build a brightness control packet (single packet)
pub fn build_set_brightness(level: u8, save: bool) -> Vec<HidPacket> {
    let payload = [level, if save { 0x01 } else { 0x00 }];
    build_chunked_packets(HidCommand::SetBrightness, &payload)
}

/// Build a set soft key command (may be multi-packet for long string data)
pub fn build_set_soft_key(index: u8, key_type: SoftKeyType, data: &[u8], save: bool) -> Vec<HidPacket> {
    let mut payload = vec![index, key_type as u8, if save { 0x01 } else { 0x00 }];
    payload.extend_from_slice(data);
    build_chunked_packets(HidCommand::SetSoftKey, &payload)
}

/// Build a get soft key query (single packet)
pub fn build_get_soft_key(index: u8) -> Vec<HidPacket> {
    build_chunked_packets(HidCommand::GetSoftKey, &[index])
}

/// Build a reset soft keys command (single packet)
pub fn build_reset_soft_keys() -> Vec<HidPacket> {
    build_chunked_packets(HidCommand::ResetSoftKeys, &[])
}

/// Build a set mode command (single packet)
pub fn build_set_mode(mode: DeviceMode) -> Vec<HidPacket> {
    build_chunked_packets(HidCommand::SetMode, &[mode as u8])
}

/// Truncate a string to a maximum length
fn truncate(s: &str, max_len: usize) -> &str {
    if s.len() <= max_len {
        return s;
    }

    // Find a valid UTF-8 boundary
    let mut end = max_len;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }

    &s[..end]
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::protocol::{FLAG_END, FLAG_START};

    #[test]
    fn test_build_display_update() {
        let state = ClaudeState {
            task: "Testing".to_string(),
            model: "Claude".to_string(),
            progress: 50,
            tokens: "1000".to_string(),
            cost: "$0.01".to_string(),
        };

        let packets = build_display_update(&state);
        assert!(!packets.is_empty());
        // First packet should be START
        assert!(packets[0].is_start());
        // Last packet should be END
        assert!(packets.last().unwrap().is_end());
        // All packets should have UpdateDisplay command
        for p in &packets {
            assert_eq!(p.command(), Some(HidCommand::UpdateDisplay));
        }
    }

    #[test]
    fn test_build_ping() {
        let packets = build_ping();
        assert_eq!(packets.len(), 1);
        assert_eq!(packets[0].flags(), FLAG_START | FLAG_END);
        assert_eq!(packets[0].command(), Some(HidCommand::Ping));
    }

    #[test]
    fn test_build_set_brightness() {
        let packets = build_set_brightness(200, true);
        assert_eq!(packets.len(), 1);
        assert_eq!(packets[0].command(), Some(HidCommand::SetBrightness));
        assert_eq!(packets[0].payload()[0], 200);
        assert_eq!(packets[0].payload()[1], 0x01);
    }

    #[test]
    fn test_build_set_soft_key() {
        let packets = build_set_soft_key(0, SoftKeyType::String, b"hello", true);
        assert!(!packets.is_empty());
        assert_eq!(packets[0].command(), Some(HidCommand::SetSoftKey));
        // Payload: [index=0, type=2, save=1, 'h', 'e', 'l', 'l', 'o']
        let payload = packets[0].payload();
        assert_eq!(payload[0], 0); // index
        assert_eq!(payload[1], 2); // SoftKeyType::String
        assert_eq!(payload[2], 1); // save
        assert_eq!(&payload[3..8], b"hello");
    }

    #[test]
    fn test_build_get_soft_key() {
        let packets = build_get_soft_key(2);
        assert_eq!(packets.len(), 1);
        assert_eq!(packets[0].command(), Some(HidCommand::GetSoftKey));
        assert_eq!(packets[0].payload()[0], 2);
    }

    #[test]
    fn test_build_reset_soft_keys() {
        let packets = build_reset_soft_keys();
        assert_eq!(packets.len(), 1);
        assert_eq!(packets[0].command(), Some(HidCommand::ResetSoftKeys));
    }

    #[test]
    fn test_build_set_mode() {
        let packets = build_set_mode(DeviceMode::Plan);
        assert_eq!(packets.len(), 1);
        assert_eq!(packets[0].command(), Some(HidCommand::SetMode));
        assert_eq!(packets[0].payload()[0], 2); // Plan = 2
    }

    #[test]
    fn test_truncate() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello world", 5), "hello");
    }
}
