//! HID module — keycode translation and soft-key UI model
//!
//! The actual HID device communication is handled by coredeck-daemon.
//! This module retains the UI-facing pieces:
//! - `keycodes`: QMK keycode → terminal byte / egui key mapping
//! - `protocol`: re-exports shared types from `coredeck_protocol`
//! - `soft_keys`: soft key editing state and preset management

pub mod keycodes;
pub mod protocol;
pub mod soft_keys;

pub use protocol::{DeviceMode, DeviceState, SoftKeyConfig, SoftKeyType};
pub use soft_keys::{PresetManager, SoftKeyEditState, UserPreset};
