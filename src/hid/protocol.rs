//! HID protocol definitions for Agent Deck communication
//!
//! Protocol based on firmware implementation in display.c:
//! - Packet size: 32 bytes
//! - Command byte at position 0
//! - Payload in bytes 1-31

use serde::{Deserialize, Serialize};

/// HID packet size in bytes
pub const PACKET_SIZE: usize = 32;

/// Maximum JSON payload size (packet size - command byte)
pub const MAX_PAYLOAD_SIZE: usize = PACKET_SIZE - 1;

/// HID commands supported by the Agent Deck firmware
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum HidCommand {
    /// Update display with JSON data
    UpdateDisplay = 0x01,
    /// Ping/Pong keep-alive
    Ping = 0x02,
    /// Set display brightness
    SetBrightness = 0x03,
}

impl HidCommand {
    /// Convert command to byte value
    pub fn as_byte(&self) -> u8 {
        *self as u8
    }

    /// Parse command from byte
    pub fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0x01 => Some(HidCommand::UpdateDisplay),
            0x02 => Some(HidCommand::Ping),
            0x03 => Some(HidCommand::SetBrightness),
            _ => None,
        }
    }
}

/// A 32-byte HID packet for communication
#[derive(Debug, Clone)]
pub struct HidPacket {
    /// Raw packet data
    data: [u8; PACKET_SIZE],
}

impl Default for HidPacket {
    fn default() -> Self {
        Self::new()
    }
}

impl HidPacket {
    /// Create a new empty packet
    pub fn new() -> Self {
        Self {
            data: [0u8; PACKET_SIZE],
        }
    }

    /// Create a packet with the given command
    pub fn with_command(command: HidCommand) -> Self {
        let mut packet = Self::new();
        packet.data[0] = command.as_byte();
        packet
    }

    /// Get the command byte
    pub fn command(&self) -> Option<HidCommand> {
        HidCommand::from_byte(self.data[0])
    }

    /// Set the command byte
    pub fn set_command(&mut self, command: HidCommand) {
        self.data[0] = command.as_byte();
    }

    /// Get the payload slice (bytes 1-31)
    pub fn payload(&self) -> &[u8] {
        &self.data[1..]
    }

    /// Get mutable payload slice
    pub fn payload_mut(&mut self) -> &mut [u8] {
        &mut self.data[1..]
    }

    /// Set payload from bytes, truncating if necessary
    pub fn set_payload(&mut self, payload: &[u8]) {
        let len = payload.len().min(MAX_PAYLOAD_SIZE);
        self.data[1..1 + len].copy_from_slice(&payload[..len]);
        // Zero remaining bytes
        for i in 1 + len..PACKET_SIZE {
            self.data[i] = 0;
        }
    }

    /// Set payload from string, truncating if necessary
    pub fn set_payload_str(&mut self, payload: &str) {
        self.set_payload(payload.as_bytes());
    }

    /// Get raw packet data for sending
    pub fn as_bytes(&self) -> &[u8; PACKET_SIZE] {
        &self.data
    }

    /// Create packet from raw bytes
    pub fn from_bytes(bytes: &[u8]) -> Self {
        let mut packet = Self::new();
        let len = bytes.len().min(PACKET_SIZE);
        packet.data[..len].copy_from_slice(&bytes[..len]);
        packet
    }
}

/// Display update data structure matching firmware JSON format
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisplayUpdate {
    /// Current task description
    pub task: String,
    /// Model name
    pub model: String,
    /// Progress percentage (0-100)
    pub progress: u8,
    /// Token count (formatted)
    pub tokens: String,
    /// Cost (formatted)
    pub cost: String,
}

impl DisplayUpdate {
    /// Convert to JSON string for HID transmission
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// Create from ClaudeState
    pub fn from_claude_state(state: &crate::core::state::ClaudeState) -> Self {
        Self {
            task: state.task.clone(),
            model: state.model.clone(),
            progress: state.progress,
            tokens: state.tokens.clone(),
            cost: state.cost.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_packet_creation() {
        let packet = HidPacket::new();
        assert_eq!(packet.as_bytes().len(), PACKET_SIZE);
        assert!(packet.as_bytes().iter().all(|&b| b == 0));
    }

    #[test]
    fn test_packet_with_command() {
        let packet = HidPacket::with_command(HidCommand::Ping);
        assert_eq!(packet.command(), Some(HidCommand::Ping));
        assert_eq!(packet.as_bytes()[0], 0x02);
    }

    #[test]
    fn test_packet_payload() {
        let mut packet = HidPacket::with_command(HidCommand::UpdateDisplay);
        packet.set_payload_str(r#"{"task":"test"}"#);
        assert_eq!(&packet.payload()[..15], br#"{"task":"test"}"#);
    }

    #[test]
    fn test_display_update_json() {
        let update = DisplayUpdate {
            task: "Testing".to_string(),
            model: "Claude".to_string(),
            progress: 50,
            tokens: "1000".to_string(),
            cost: "$0.01".to_string(),
        };
        let json = update.to_json();
        assert!(json.contains("\"task\":\"Testing\""));
        assert!(json.contains("\"progress\":50"));
    }

    #[test]
    fn test_command_roundtrip() {
        assert_eq!(
            HidCommand::from_byte(HidCommand::UpdateDisplay.as_byte()),
            Some(HidCommand::UpdateDisplay)
        );
        assert_eq!(
            HidCommand::from_byte(HidCommand::Ping.as_byte()),
            Some(HidCommand::Ping)
        );
        assert_eq!(
            HidCommand::from_byte(HidCommand::SetBrightness.as_byte()),
            Some(HidCommand::SetBrightness)
        );
        assert_eq!(HidCommand::from_byte(0xFF), None);
    }
}
