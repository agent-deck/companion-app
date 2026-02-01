//! Alert handler for terminal notifications
//!
//! Intercepts OSC 777 (rxvt) and OSC 9 (iTerm2) notifications from the terminal
//! and forwards them via a channel for processing.

use std::sync::mpsc;
use wezterm_term::{Alert, AlertHandler};

/// Channel-based alert handler that forwards notifications
pub struct NotificationHandler {
    sender: mpsc::Sender<Alert>,
}

impl NotificationHandler {
    pub fn new(sender: mpsc::Sender<Alert>) -> Self {
        Self { sender }
    }
}

impl AlertHandler for NotificationHandler {
    fn alert(&mut self, alert: Alert) {
        let _ = self.sender.send(alert);
    }
}
