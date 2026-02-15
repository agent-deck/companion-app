//! HID protocol definitions for Agent Deck communication
//!
//! Protocol uses a chunked format with a 2-byte header:
//! - Byte 0: flags (START=0x80, END=0x40)
//! - Byte 1: command ID
//! - Bytes 2-31: payload (30 bytes per chunk)

use serde::{Deserialize, Serialize};

/// HID packet size in bytes
pub const PACKET_SIZE: usize = 32;

/// Header size (flags + command)
pub const HEADER_SIZE: usize = 2;

/// Maximum payload per chunk (packet size - header)
pub const MAX_PAYLOAD_SIZE: usize = PACKET_SIZE - HEADER_SIZE;

/// Flag: this is the first packet of a message
pub const FLAG_START: u8 = 0x80;

/// Flag: this is the last packet of a message
pub const FLAG_END: u8 = 0x40;

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
    /// Set a soft key assignment
    SetSoftKey = 0x04,
    /// Get a soft key assignment
    GetSoftKey = 0x05,
    /// Reset all soft keys to defaults
    ResetSoftKeys = 0x06,
    /// Set device LED mode
    SetMode = 0x07,
    /// Device state report (unsolicited from device)
    StateReport = 0x10,
    /// Error response from device
    Error = 0xFF,
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
            0x04 => Some(HidCommand::SetSoftKey),
            0x05 => Some(HidCommand::GetSoftKey),
            0x06 => Some(HidCommand::ResetSoftKeys),
            0x07 => Some(HidCommand::SetMode),
            0x10 => Some(HidCommand::StateReport),
            0xFF => Some(HidCommand::Error),
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

    /// Create a packet with flags and command set
    pub fn with_command(flags: u8, command: HidCommand) -> Self {
        let mut packet = Self::new();
        packet.data[0] = flags;
        packet.data[1] = command.as_byte();
        packet
    }

    /// Get the flags byte
    pub fn flags(&self) -> u8 {
        self.data[0]
    }

    /// Check if this is the start of a message
    pub fn is_start(&self) -> bool {
        self.data[0] & FLAG_START != 0
    }

    /// Check if this is the end of a message
    pub fn is_end(&self) -> bool {
        self.data[0] & FLAG_END != 0
    }

    /// Get the command byte
    pub fn command_byte(&self) -> u8 {
        self.data[1]
    }

    /// Get the command as enum
    pub fn command(&self) -> Option<HidCommand> {
        HidCommand::from_byte(self.data[1])
    }

    /// Get the payload slice (bytes 2-31)
    pub fn payload(&self) -> &[u8] {
        &self.data[HEADER_SIZE..]
    }

    /// Get mutable payload slice
    pub fn payload_mut(&mut self) -> &mut [u8] {
        &mut self.data[HEADER_SIZE..]
    }

    /// Set payload from bytes, truncating if necessary
    pub fn set_payload(&mut self, payload: &[u8]) {
        let len = payload.len().min(MAX_PAYLOAD_SIZE);
        self.data[HEADER_SIZE..HEADER_SIZE + len].copy_from_slice(&payload[..len]);
        // Zero remaining bytes
        for i in HEADER_SIZE + len..PACKET_SIZE {
            self.data[i] = 0;
        }
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

/// Build chunked packets for a command with a payload.
///
/// Splits payload into 30-byte chunks with correct START/END flags.
/// Single-packet messages get flags START|END (0xC0).
pub fn build_chunked_packets(command: HidCommand, payload: &[u8]) -> Vec<HidPacket> {
    if payload.is_empty() {
        // Single packet, no payload
        let packet = HidPacket::with_command(FLAG_START | FLAG_END, command);
        return vec![packet];
    }

    let chunks: Vec<&[u8]> = payload.chunks(MAX_PAYLOAD_SIZE).collect();
    let last_idx = chunks.len() - 1;

    chunks
        .iter()
        .enumerate()
        .map(|(i, chunk)| {
            let mut flags = 0u8;
            if i == 0 {
                flags |= FLAG_START;
            }
            if i == last_idx {
                flags |= FLAG_END;
            }
            let mut packet = HidPacket::with_command(flags, command);
            packet.set_payload(chunk);
            packet
        })
        .collect()
}

/// Parsed response from the device
#[derive(Debug, Clone)]
pub struct ResponsePacket {
    /// Command this is a response to
    pub command: u8,
    /// Status byte (first byte of reassembled payload)
    pub status: u8,
    /// Remaining data after status byte
    pub data: Vec<u8>,
}

/// Protocol error codes from firmware
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ProtoError {
    /// Payload exceeded device buffer
    Overflow = 0x01,
    /// Received continuation without start
    BadSequence = 0x02,
    /// Unknown command ID
    UnknownCommand = 0x03,
}

impl ProtoError {
    pub fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0x01 => Some(ProtoError::Overflow),
            0x02 => Some(ProtoError::BadSequence),
            0x03 => Some(ProtoError::UnknownCommand),
            _ => None,
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            ProtoError::Overflow => "payload overflow",
            ProtoError::BadSequence => "bad packet sequence",
            ProtoError::UnknownCommand => "unknown command",
        }
    }
}

/// Device operating mode (LED indicator)
///
/// Cycle order on the device: Default -> Accept -> Plan -> Default
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum DeviceMode {
    #[default]
    Default = 0,
    Accept = 1,
    Plan = 2,
}

impl DeviceMode {
    pub fn from_byte(byte: u8) -> Self {
        match byte {
            1 => DeviceMode::Accept,
            2 => DeviceMode::Plan,
            _ => DeviceMode::Default,
        }
    }
}

impl std::fmt::Display for DeviceMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeviceMode::Default => write!(f, "default"),
            DeviceMode::Accept => write!(f, "accept"),
            DeviceMode::Plan => write!(f, "plan"),
        }
    }
}

/// Device state parsed from state report
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DeviceState {
    pub mode: DeviceMode,
    pub yolo: bool,
}

impl DeviceState {
    /// Parse from a single state byte.
    /// Bit layout: bits[1:0] = mode, bit[2] = yolo
    pub fn from_byte(byte: u8) -> Self {
        Self {
            mode: DeviceMode::from_byte(byte & 0x03),
            yolo: byte & 0x04 != 0,
        }
    }
}

/// Soft key type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SoftKeyType {
    Default = 0,
    Keycode = 1,
    String = 2,
    Sequence = 3,
}

impl SoftKeyType {
    pub fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0 => Some(SoftKeyType::Default),
            1 => Some(SoftKeyType::Keycode),
            2 => Some(SoftKeyType::String),
            3 => Some(SoftKeyType::Sequence),
            _ => None,
        }
    }
}

/// Soft key configuration
#[derive(Debug, Clone)]
pub struct SoftKeyConfig {
    pub index: u8,
    pub key_type: SoftKeyType,
    pub data: Vec<u8>,
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
        let packet = HidPacket::with_command(FLAG_START | FLAG_END, HidCommand::Ping);
        assert_eq!(packet.flags(), 0xC0);
        assert_eq!(packet.command(), Some(HidCommand::Ping));
        assert_eq!(packet.as_bytes()[0], 0xC0);
        assert_eq!(packet.as_bytes()[1], 0x02);
    }

    #[test]
    fn test_packet_flags() {
        let packet = HidPacket::with_command(FLAG_START, HidCommand::UpdateDisplay);
        assert!(packet.is_start());
        assert!(!packet.is_end());

        let packet = HidPacket::with_command(FLAG_END, HidCommand::UpdateDisplay);
        assert!(!packet.is_start());
        assert!(packet.is_end());

        let packet = HidPacket::with_command(FLAG_START | FLAG_END, HidCommand::UpdateDisplay);
        assert!(packet.is_start());
        assert!(packet.is_end());
    }

    #[test]
    fn test_packet_payload() {
        let mut packet = HidPacket::with_command(FLAG_START | FLAG_END, HidCommand::UpdateDisplay);
        let data = b"hello";
        packet.set_payload(data);
        assert_eq!(&packet.payload()[..5], b"hello");
        // Rest should be zero
        assert!(packet.payload()[5..].iter().all(|&b| b == 0));
    }

    #[test]
    fn test_build_chunked_single() {
        let packets = build_chunked_packets(HidCommand::Ping, &[]);
        assert_eq!(packets.len(), 1);
        assert_eq!(packets[0].flags(), FLAG_START | FLAG_END);
        assert_eq!(packets[0].command(), Some(HidCommand::Ping));
    }

    #[test]
    fn test_build_chunked_small_payload() {
        let payload = b"small";
        let packets = build_chunked_packets(HidCommand::UpdateDisplay, payload);
        assert_eq!(packets.len(), 1);
        assert_eq!(packets[0].flags(), FLAG_START | FLAG_END);
        assert_eq!(&packets[0].payload()[..5], b"small");
    }

    #[test]
    fn test_build_chunked_multi_packet() {
        // Create a payload that requires multiple chunks (>30 bytes)
        let payload = vec![0xAA; 70]; // 70 bytes = 3 chunks (30+30+10)
        let packets = build_chunked_packets(HidCommand::UpdateDisplay, &payload);
        assert_eq!(packets.len(), 3);

        // First: START only
        assert!(packets[0].is_start());
        assert!(!packets[0].is_end());

        // Middle: neither
        assert!(!packets[1].is_start());
        assert!(!packets[1].is_end());

        // Last: END only
        assert!(!packets[2].is_start());
        assert!(packets[2].is_end());

        // All should have the same command
        for p in &packets {
            assert_eq!(p.command(), Some(HidCommand::UpdateDisplay));
        }
    }

    #[test]
    fn test_build_chunked_exact_boundary() {
        // Exactly 30 bytes = 1 chunk
        let payload = vec![0xBB; 30];
        let packets = build_chunked_packets(HidCommand::UpdateDisplay, &payload);
        assert_eq!(packets.len(), 1);
        assert_eq!(packets[0].flags(), FLAG_START | FLAG_END);

        // Exactly 60 bytes = 2 chunks
        let payload = vec![0xBB; 60];
        let packets = build_chunked_packets(HidCommand::UpdateDisplay, &payload);
        assert_eq!(packets.len(), 2);
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
        let commands = [
            HidCommand::UpdateDisplay,
            HidCommand::Ping,
            HidCommand::SetBrightness,
            HidCommand::SetSoftKey,
            HidCommand::GetSoftKey,
            HidCommand::ResetSoftKeys,
            HidCommand::SetMode,
            HidCommand::StateReport,
            HidCommand::Error,
        ];
        for cmd in commands {
            assert_eq!(HidCommand::from_byte(cmd.as_byte()), Some(cmd));
        }
        assert_eq!(HidCommand::from_byte(0xFE), None);
    }

    #[test]
    fn test_device_state_from_byte() {
        let state = DeviceState::from_byte(0x00);
        assert_eq!(state.mode, DeviceMode::Default);
        assert!(!state.yolo);

        let state = DeviceState::from_byte(0x01); // mode=1 (Accept)
        assert_eq!(state.mode, DeviceMode::Accept);
        assert!(!state.yolo);

        let state = DeviceState::from_byte(0x02); // mode=2 (Plan)
        assert_eq!(state.mode, DeviceMode::Plan);
        assert!(!state.yolo);

        let state = DeviceState::from_byte(0x06); // mode=2 (Plan), yolo=1
        assert_eq!(state.mode, DeviceMode::Plan);
        assert!(state.yolo);

        let state = DeviceState::from_byte(0x05); // mode=1 (Accept), yolo=1
        assert_eq!(state.mode, DeviceMode::Accept);
        assert!(state.yolo);

        let state = DeviceState::from_byte(0x04); // mode=0 (Default), yolo=1
        assert_eq!(state.mode, DeviceMode::Default);
        assert!(state.yolo);
    }

    #[test]
    fn test_proto_error() {
        assert_eq!(ProtoError::from_byte(0x01), Some(ProtoError::Overflow));
        assert_eq!(ProtoError::from_byte(0x02), Some(ProtoError::BadSequence));
        assert_eq!(ProtoError::from_byte(0x03), Some(ProtoError::UnknownCommand));
        assert_eq!(ProtoError::from_byte(0x99), None);
    }

    #[test]
    fn test_soft_key_type() {
        assert_eq!(SoftKeyType::from_byte(0), Some(SoftKeyType::Default));
        assert_eq!(SoftKeyType::from_byte(1), Some(SoftKeyType::Keycode));
        assert_eq!(SoftKeyType::from_byte(2), Some(SoftKeyType::String));
        assert_eq!(SoftKeyType::from_byte(3), Some(SoftKeyType::Sequence));
        assert_eq!(SoftKeyType::from_byte(4), None);
    }
}
