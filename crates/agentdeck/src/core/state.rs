//! Application state management

use crate::hid::protocol::DeviceMode;

/// Global application state
#[derive(Debug, Default)]
pub struct AppState {
    /// Whether daemon WebSocket is connected
    pub daemon_connected: bool,
    /// Whether HID device is connected
    pub hid_connected: bool,
    /// Whether Claude is currently running
    pub claude_running: bool,
    /// Error message if any
    pub error: Option<String>,
    /// Current device mode (default/plan/accept)
    pub device_mode: DeviceMode,
    /// Whether device YOLO mode is active
    pub device_yolo: bool,
    /// Connected device name (e.g. "Agent Deck")
    pub device_name: Option<String>,
    /// Connected device firmware version
    pub firmware_version: Option<String>,
}
