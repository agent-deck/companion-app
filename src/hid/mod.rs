//! HID module - USB HID communication with Agent Deck macropad

mod commands;
mod device;
pub mod keycodes;
pub mod protocol;
pub mod soft_keys;

#[cfg(target_os = "macos")]
mod hotplug_macos;

pub use device::HidManager;
pub use protocol::{DeviceMode, DeviceState, HidCommand, HidPacket, SoftKeyConfig, SoftKeyType};
pub use soft_keys::{PresetManager, SoftKeyEditState, UserPreset};
