//! HID device discovery and connection management

use super::protocol::{DisplayUpdate, HidCommand, HidPacket, PACKET_SIZE};
use crate::core::config::HidConfig;
use crate::core::events::AppEvent;
use crate::core::state::ClaudeState;
use anyhow::{anyhow, Context, Result};
use hidapi::{HidApi, HidDevice};
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

/// Manager for HID device communication with Agent Deck
pub struct HidManager {
    /// HID API instance
    api: Arc<Mutex<HidApi>>,
    /// Connected device (if any)
    device: Arc<Mutex<Option<HidDevice>>>,
    /// Configuration
    config: HidConfig,
    /// Event sender for status updates
    event_tx: mpsc::UnboundedSender<AppEvent>,
    /// Whether currently connected
    connected: Arc<AtomicBool>,
    /// Whether the ping thread should stop
    stop_ping: Arc<AtomicBool>,
}

impl HidManager {
    /// Create a new HID manager
    pub fn new(config: HidConfig, event_tx: mpsc::UnboundedSender<AppEvent>) -> Result<Self> {
        let api = HidApi::new().context("Failed to initialize HID API")?;

        let manager = Self {
            api: Arc::new(Mutex::new(api)),
            device: Arc::new(Mutex::new(None)),
            config,
            event_tx,
            connected: Arc::new(AtomicBool::new(false)),
            stop_ping: Arc::new(AtomicBool::new(false)),
        };

        // Try initial connection
        manager.try_connect()?;

        // Start ping thread
        manager.start_ping_thread();

        Ok(manager)
    }

    /// Start background ping thread
    fn start_ping_thread(&self) {
        let device = Arc::clone(&self.device);
        let connected = Arc::clone(&self.connected);
        let stop_ping = Arc::clone(&self.stop_ping);
        let ping_interval = self.config.ping_interval_ms;

        thread::spawn(move || {
            info!("Ping thread started");
            while !stop_ping.load(Ordering::Relaxed) {
                if connected.load(Ordering::Relaxed) {
                    let device_guard = device.lock();
                    if let Some(ref dev) = *device_guard {
                        // Send ping
                        let packet = HidPacket::with_command(HidCommand::Ping);
                        match send_packet_to_device(dev, &packet) {
                            Ok(()) => {
                                debug!("Ping sent");
                                // Try to read pong response
                                let mut buffer = [0u8; PACKET_SIZE];
                                match dev.read_timeout(&mut buffer, 100) {
                                    Ok(n) if n > 0 => {
                                        if buffer[0] == HidCommand::Ping as u8 {
                                            debug!("Pong received");
                                        }
                                    }
                                    Ok(_) => {
                                        debug!("No pong response");
                                    }
                                    Err(e) => {
                                        warn!("Error reading pong: {}", e);
                                    }
                                }
                            }
                            Err(e) => {
                                warn!("Failed to send ping: {}", e);
                            }
                        }
                    }
                    drop(device_guard);
                }
                thread::sleep(Duration::from_millis(ping_interval));
            }
            info!("Ping thread stopped");
        });
    }

    /// Try to connect to the Agent Deck device
    pub fn try_connect(&self) -> Result<()> {
        let api = self.api.lock();

        // Find device by VID/PID and usage page
        let device_info = api
            .device_list()
            .find(|d| {
                d.vendor_id() == self.config.vendor_id
                    && d.product_id() == self.config.product_id
                    && d.usage_page() == self.config.usage_page
                    && d.usage() == self.config.usage_id
            })
            .ok_or_else(|| {
                anyhow!(
                    "Agent Deck not found (VID: 0x{:04X}, PID: 0x{:04X}, Usage: 0x{:04X}/0x{:02X})",
                    self.config.vendor_id,
                    self.config.product_id,
                    self.config.usage_page,
                    self.config.usage_id
                )
            })?;

        info!(
            "Found Agent Deck: {} {}",
            device_info
                .manufacturer_string()
                .unwrap_or("Unknown"),
            device_info.product_string().unwrap_or("Unknown")
        );

        // Open the device
        let device = device_info
            .open_device(&api)
            .context("Failed to open HID device")?;

        // Set non-blocking mode
        device
            .set_blocking_mode(false)
            .context("Failed to set non-blocking mode")?;

        // Store device
        *self.device.lock() = Some(device);
        self.connected.store(true, Ordering::Relaxed);

        // Notify connection
        let _ = self.event_tx.send(AppEvent::HidConnected);

        info!("Connected to Agent Deck");
        Ok(())
    }

    /// Check if device is connected
    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    /// Send a display update to the device
    pub fn send_display_update(&self, state: &ClaudeState) -> Result<()> {
        let device_guard = self.device.lock();
        let device = device_guard
            .as_ref()
            .ok_or_else(|| anyhow!("Device not connected"))?;

        let truncated = state.truncated();
        let update = DisplayUpdate::from_claude_state(&truncated);
        let json = update.to_json();

        debug!("Sending display update: {}", json);

        // Check if JSON fits in payload
        if json.len() > super::protocol::MAX_PAYLOAD_SIZE {
            warn!(
                "JSON too long ({} bytes), truncating",
                json.len()
            );
        }

        let mut packet = HidPacket::with_command(HidCommand::UpdateDisplay);
        packet.set_payload_str(&json);

        send_packet_to_device(device, &packet)?;

        // Try to read ACK but don't fail if not received
        let mut buffer = [0u8; PACKET_SIZE];
        let _ = device.read_timeout(&mut buffer, 50);

        Ok(())
    }

    /// Send a task update to the device (simplified display update with just task)
    pub fn send_task_update(&self, task: &str) -> Result<()> {
        let device_guard = self.device.lock();
        let device = device_guard
            .as_ref()
            .ok_or_else(|| anyhow!("Device not connected"))?;

        // Clean up the task string - remove leading symbols/emojis that Claude uses for status
        let clean_task = task
            .trim_start_matches(|c: char| !c.is_alphanumeric())
            .trim();

        // Create JSON payload with just the task
        let json = format!(r#"{{"task":"{}"}}"#, clean_task.replace('"', "\\\""));

        debug!("Sending task update: {}", json);

        // Check if JSON fits in payload
        if json.len() > super::protocol::MAX_PAYLOAD_SIZE {
            warn!(
                "Task JSON too long ({} bytes), truncating",
                json.len()
            );
        }

        let mut packet = HidPacket::with_command(HidCommand::UpdateDisplay);
        packet.set_payload_str(&json);

        send_packet_to_device(device, &packet)?;

        // Try to read ACK but don't fail if not received
        let mut buffer = [0u8; PACKET_SIZE];
        let _ = device.read_timeout(&mut buffer, 50);

        Ok(())
    }

    /// Set display brightness
    pub fn set_brightness(&self, level: u8, save: bool) -> Result<()> {
        let device_guard = self.device.lock();
        let device = device_guard
            .as_ref()
            .ok_or_else(|| anyhow!("Device not connected"))?;

        let mut packet = HidPacket::with_command(HidCommand::SetBrightness);
        let payload = packet.payload_mut();
        payload[0] = level;
        payload[1] = if save { 0x01 } else { 0x00 };

        send_packet_to_device(device, &packet)?;

        // Try to read ACK
        let mut buffer = [0u8; PACKET_SIZE];
        let _ = device.read_timeout(&mut buffer, 50);

        info!("Brightness set to {}", level);
        Ok(())
    }

    /// Disconnect from the device
    pub fn disconnect(&self) {
        let mut device_guard = self.device.lock();
        if device_guard.take().is_some() {
            self.connected.store(false, Ordering::Relaxed);
            let _ = self.event_tx.send(AppEvent::HidDisconnected);
            info!("Disconnected from Agent Deck");
        }
    }
}

impl Drop for HidManager {
    fn drop(&mut self) {
        self.stop_ping.store(true, Ordering::Relaxed);
        self.disconnect();
    }
}

/// Send a packet to the HID device
fn send_packet_to_device(device: &HidDevice, packet: &HidPacket) -> Result<()> {
    let bytes = packet.as_bytes();

    // On macOS/Windows, hidapi requires prepending a 0x00 report ID for devices
    // that don't use numbered reports (like QMK Raw HID)
    let mut data = Vec::with_capacity(PACKET_SIZE + 1);
    data.push(0x00); // Report ID
    data.extend_from_slice(bytes);

    let written = device
        .write(&data)
        .context("Failed to write to HID device")?;

    debug!("Wrote {} bytes to HID device", written);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hid_config_default() {
        let config = HidConfig::default();
        assert_eq!(config.vendor_id, 0xFEED);
        assert_eq!(config.product_id, 0x0803);
    }
}
