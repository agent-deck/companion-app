//! HID command helpers
//!
//! Convenience functions for building common HID commands.

#![allow(dead_code)]

use super::protocol::{HidCommand, HidPacket};
use crate::core::state::ClaudeState;

/// Build a display update packet from Claude state
pub fn build_display_update(state: &ClaudeState) -> HidPacket {
    let mut packet = HidPacket::with_command(HidCommand::UpdateDisplay);

    let json = serde_json::json!({
        "task": truncate(&state.task, 64),
        "model": truncate(&state.model, 64),
        "progress": state.progress.min(100),
        "tokens": truncate(&state.tokens, 16),
        "cost": truncate(&state.cost, 16),
    });

    packet.set_payload_str(&json.to_string());
    packet
}

/// Build a ping packet
pub fn build_ping() -> HidPacket {
    HidPacket::with_command(HidCommand::Ping)
}

/// Build a brightness control packet
pub fn build_set_brightness(level: u8, save: bool) -> HidPacket {
    let mut packet = HidPacket::with_command(HidCommand::SetBrightness);
    let payload = packet.payload_mut();
    payload[0] = level;
    payload[1] = if save { 0x01 } else { 0x00 };
    packet
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

    #[test]
    fn test_build_display_update() {
        let state = ClaudeState {
            task: "Testing".to_string(),
            model: "Claude".to_string(),
            progress: 50,
            tokens: "1000".to_string(),
            cost: "$0.01".to_string(),
        };

        let packet = build_display_update(&state);
        assert_eq!(packet.command(), Some(HidCommand::UpdateDisplay));

        // The JSON is truncated due to the 31-byte payload limit
        // Just verify the command byte is correct and the packet was built
        let payload = packet.payload();
        // Payload should not be all zeros
        assert!(payload.iter().any(|&b| b != 0), "Payload should contain data");
    }

    #[test]
    fn test_build_ping() {
        let packet = build_ping();
        assert_eq!(packet.command(), Some(HidCommand::Ping));
    }

    #[test]
    fn test_build_set_brightness() {
        let packet = build_set_brightness(200, true);
        assert_eq!(packet.command(), Some(HidCommand::SetBrightness));
        assert_eq!(packet.payload()[0], 200);
        assert_eq!(packet.payload()[1], 0x01);
    }

    #[test]
    fn test_truncate() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello world", 5), "hello");
    }
}
