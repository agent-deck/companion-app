//! HID module - USB HID communication with Agent Deck macropad

mod commands;
mod device;
mod protocol;

pub use device::HidManager;
pub use protocol::{HidCommand, HidPacket};
