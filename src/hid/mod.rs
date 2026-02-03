//! HID module - USB HID communication with Agent Deck macropad

mod commands;
mod device;
mod protocol;

#[cfg(target_os = "macos")]
mod hotplug_macos;

pub use device::HidManager;
pub use protocol::{HidCommand, HidPacket};
