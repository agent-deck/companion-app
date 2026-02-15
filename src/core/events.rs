//! Application event definitions

use super::sessions::SessionId;
use crate::hid::protocol::{DeviceMode};
use crate::hotkey::HotkeyType;
#[cfg(target_os = "macos")]
use crate::macos::MenuAction;
use crate::tray::TrayAction;

/// Application-wide events for inter-module communication
#[derive(Debug, Clone)]
pub enum AppEvent {
    /// HID device connected
    HidConnected,

    /// HID device disconnected
    HidDisconnected,

    /// Global hotkey was pressed
    HotkeyPressed(HotkeyType),

    /// Tray menu action triggered
    TrayAction(TrayAction),

    /// PTY output received (raw bytes) - legacy, for default session
    PtyOutput(Vec<u8>),

    /// PTY output received for a specific session
    PtyOutputForSession { session_id: SessionId, data: Vec<u8> },

    /// PTY process exited - legacy
    PtyExited(Option<i32>),

    /// PTY process exited for a specific session
    PtyExitedForSession { session_id: SessionId, code: Option<i32> },

    /// HID device state changed (mode button or YOLO switch)
    DeviceStateChanged { mode: DeviceMode, yolo: bool },

    /// Menu bar action triggered (macOS only)
    #[cfg(target_os = "macos")]
    MenuAction(MenuAction),
}
