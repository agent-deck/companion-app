//! Application event definitions

use super::sessions::SessionId;
use crate::hid::protocol::DeviceMode;
#[cfg(target_os = "macos")]
use crate::macos::MenuAction;
use crate::tray::TrayAction;
use tokio::sync::mpsc;
use winit::event_loop::EventLoopProxy;

/// Wrapper around `mpsc::UnboundedSender<AppEvent>` that also wakes the winit
/// event loop via `EventLoopProxy::wake_up()` after every send.  This allows
/// switching from `ControlFlow::Poll` to `ControlFlow::Wait` without losing
/// responsiveness to background events (PTY output, HID, tray).
#[derive(Clone)]
pub struct EventSender {
    tx: mpsc::UnboundedSender<AppEvent>,
    proxy: EventLoopProxy<()>,
}

impl EventSender {
    pub fn new(tx: mpsc::UnboundedSender<AppEvent>, proxy: EventLoopProxy<()>) -> Self {
        Self { tx, proxy }
    }

    pub fn send(&self, event: AppEvent) -> Result<(), mpsc::error::SendError<AppEvent>> {
        let result = self.tx.send(event);
        let _ = self.proxy.send_event(());
        result
    }
}

/// Application-wide events for inter-module communication
#[derive(Debug, Clone)]
pub enum AppEvent {
    /// HID device connected
    HidConnected,

    /// HID device disconnected
    HidDisconnected,

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

    /// HID key event: single key press with QMK 16-bit keycode
    HidKeyEvent { keycode: u16 },

    /// HID type string: string injection from device
    HidTypeString { text: String, send_enter: bool },

    /// Menu bar action triggered (macOS only)
    #[cfg(target_os = "macos")]
    MenuAction(MenuAction),
}
