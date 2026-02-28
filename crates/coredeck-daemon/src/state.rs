//! Daemon shared state

use coredeck_protocol::DeviceMode;
use tokio::sync::mpsc;

/// Events emitted by the HID subsystem to the daemon core
#[derive(Debug, Clone)]
pub enum DaemonEvent {
    /// HID device connected (interface opened, communicating)
    HidConnected {
        device_name: String,
        firmware_version: String,
    },
    /// HID device disconnected (interface closed or lost)
    HidDisconnected,
    /// USB device physically available (enumerated, not opened)
    DeviceAvailable { device_name: String },
    /// USB device physically removed
    DeviceUnavailable,
    /// Device state changed (mode button / YOLO switch)
    DeviceStateChanged { mode: DeviceMode, yolo: bool },
    /// Single key event from device
    HidKeyEvent { keycode: u16 },
    /// Type string from device
    HidTypeString { text: String, send_enter: bool },
}

/// Sender for daemon events — wraps a tokio unbounded channel.
/// Unlike the app's EventSender, no winit proxy is needed.
#[derive(Clone)]
pub struct DaemonEventSender {
    tx: mpsc::UnboundedSender<DaemonEvent>,
}

impl DaemonEventSender {
    pub fn new(tx: mpsc::UnboundedSender<DaemonEvent>) -> Self {
        Self { tx }
    }

    pub fn send(&self, event: DaemonEvent) -> Result<(), mpsc::error::SendError<DaemonEvent>> {
        self.tx.send(event)
    }
}

/// Current device status (shared across daemon subsystems)
#[derive(Debug, Clone, Default)]
pub struct DeviceStatus {
    /// Device physically present (USB enumerated)
    pub available: bool,
    /// HID interface open and communicating
    pub connected: bool,
    pub device_name: Option<String>,
    pub firmware_version: Option<String>,
    pub mode: DeviceMode,
    pub yolo: bool,
}

/// Tray updates sent from async code to the main thread
#[derive(Debug, Clone)]
pub enum TrayUpdate {
    /// HID interface opened and communicating
    DeviceConnected(String),
    /// HID interface closed or lost
    DeviceDisconnected,
    /// USB device physically available (plugged in, not opened)
    DeviceAvailable(String),
    /// USB device physically removed
    DeviceUnavailable,
    AppConnected,
    AppDisconnected,
}
