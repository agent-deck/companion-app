//! HID device discovery and connection management

use super::commands;
use super::protocol::{
    DeviceMode, DeviceState, HidCommand, HidPacket, ResponsePacket, SoftKeyConfig, SoftKeyType,
    PACKET_SIZE,
};
use crate::core::config::HidConfig;
use crate::core::events::AppEvent;
use anyhow::{anyhow, Context, Result};
use hidapi::{HidApi, HidDevice};
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

#[cfg(target_os = "macos")]
use super::hotplug_macos::{HotplugEvent, HotplugWatcher};

/// Number of consecutive ping failures before declaring disconnection
const DISCONNECT_THRESHOLD: u32 = 3;

/// Polling interval when hotplug is not available (non-macOS platforms)
#[cfg(not(target_os = "macos"))]
const RECONNECT_INITIAL_MS: u64 = 500;

#[cfg(not(target_os = "macos"))]
const RECONNECT_MAX_MS: u64 = 5000;

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
    /// Whether the monitor thread should stop
    stop_monitor: Arc<AtomicBool>,
    /// macOS hotplug watcher
    #[cfg(target_os = "macos")]
    hotplug_watcher: Option<HotplugWatcher>,
}

impl HidManager {
    /// Create a new HID manager
    pub fn new(config: HidConfig, event_tx: mpsc::UnboundedSender<AppEvent>) -> Result<Self> {
        let api = HidApi::new().context("Failed to initialize HID API")?;

        let mut manager = Self {
            api: Arc::new(Mutex::new(api)),
            device: Arc::new(Mutex::new(None)),
            config: config.clone(),
            event_tx: event_tx.clone(),
            connected: Arc::new(AtomicBool::new(false)),
            stop_monitor: Arc::new(AtomicBool::new(false)),
            #[cfg(target_os = "macos")]
            hotplug_watcher: None,
        };

        // Try initial connection (don't fail if device not found)
        if let Err(e) = manager.try_connect() {
            info!("Initial connection failed (will retry): {}", e);
        }

        // Start the appropriate monitor mechanism
        #[cfg(target_os = "macos")]
        {
            manager.start_macos_hotplug(config, event_tx);
        }

        #[cfg(not(target_os = "macos"))]
        {
            manager.start_polling_monitor();
        }

        // Start ping thread for connection health monitoring
        manager.start_ping_thread();

        Ok(manager)
    }

    /// Start macOS IOKit hotplug watcher
    #[cfg(target_os = "macos")]
    fn start_macos_hotplug(&mut self, config: HidConfig, _event_tx: mpsc::UnboundedSender<AppEvent>) {
        // Create channel for hotplug events
        let (hotplug_tx, mut hotplug_rx) = mpsc::unbounded_channel();

        // Start the IOKit watcher
        match HotplugWatcher::new(config.vendor_id, config.product_id, hotplug_tx) {
            Ok(watcher) => {
                self.hotplug_watcher = Some(watcher);
                info!("Started native IOKit hotplug watcher");

                // Spawn task to handle hotplug events
                let api = Arc::clone(&self.api);
                let device = Arc::clone(&self.device);
                let connected = Arc::clone(&self.connected);
                let stop_monitor = Arc::clone(&self.stop_monitor);
                let event_tx = self.event_tx.clone();
                let config = self.config.clone();

                thread::spawn(move || {
                    // Use a blocking receiver in a thread
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .expect("Failed to create tokio runtime");

                    rt.block_on(async {
                        while !stop_monitor.load(Ordering::Relaxed) {
                            tokio::select! {
                                Some(event) = hotplug_rx.recv() => {
                                    match event {
                                        HotplugEvent::DeviceArrived => {
                                            if !connected.load(Ordering::Relaxed) {
                                                // Small delay to let the device initialize
                                                tokio::time::sleep(Duration::from_millis(100)).await;

                                                // Refresh device list
                                                {
                                                    let mut api_guard = api.lock();
                                                    let _ = api_guard.refresh_devices();
                                                }

                                                // Try to connect
                                                if let Some(dev) = try_open_device(&api, &config) {
                                                    *device.lock() = Some(dev);
                                                    connected.store(true, Ordering::Relaxed);
                                                    let _ = event_tx.send(AppEvent::HidConnected);
                                                    info!("Device connected via hotplug");
                                                }
                                            }
                                        }
                                        HotplugEvent::DeviceRemoved => {
                                            if connected.load(Ordering::Relaxed) {
                                                *device.lock() = None;
                                                connected.store(false, Ordering::Relaxed);
                                                let _ = event_tx.send(AppEvent::HidDisconnected);
                                                info!("Device disconnected via hotplug");
                                            }
                                        }
                                    }
                                }
                                _ = tokio::time::sleep(Duration::from_millis(100)) => {
                                    // Check stop flag periodically
                                    if stop_monitor.load(Ordering::Relaxed) {
                                        break;
                                    }
                                }
                            }
                        }
                    });
                });
            }
            Err(e) => {
                warn!("Failed to start IOKit hotplug watcher: {}, falling back to polling", e);
                self.start_polling_monitor_internal();
            }
        }
    }

    /// Start polling-based monitor (for non-macOS or fallback)
    #[cfg(not(target_os = "macos"))]
    fn start_polling_monitor(&self) {
        self.start_polling_monitor_internal();
    }

    /// Internal polling monitor implementation
    fn start_polling_monitor_internal(&self) {
        let api = Arc::clone(&self.api);
        let device = Arc::clone(&self.device);
        let connected = Arc::clone(&self.connected);
        let stop_monitor = Arc::clone(&self.stop_monitor);
        let event_tx = self.event_tx.clone();
        let config = self.config.clone();

        thread::spawn(move || {
            info!("HID polling monitor thread started");

            #[cfg(not(target_os = "macos"))]
            let mut reconnect_interval_ms = RECONNECT_INITIAL_MS;
            #[cfg(target_os = "macos")]
            let mut reconnect_interval_ms = 500u64;

            #[cfg(not(target_os = "macos"))]
            let max_interval = RECONNECT_MAX_MS;
            #[cfg(target_os = "macos")]
            let max_interval = 5000u64;

            while !stop_monitor.load(Ordering::Relaxed) {
                if !connected.load(Ordering::Relaxed) {
                    // Refresh device list to see newly connected devices
                    {
                        let mut api_guard = api.lock();
                        if let Err(e) = api_guard.refresh_devices() {
                            debug!("Failed to refresh device list: {}", e);
                        }
                    }

                    // Try to find and connect to device
                    if let Some(dev) = try_open_device(&api, &config) {
                        *device.lock() = Some(dev);
                        connected.store(true, Ordering::Relaxed);
                        let _ = event_tx.send(AppEvent::HidConnected);
                        reconnect_interval_ms = 500; // Reset on success
                    } else {
                        // Exponential backoff
                        reconnect_interval_ms = (reconnect_interval_ms * 3 / 2).min(max_interval);
                        debug!("Device not found, next attempt in {}ms", reconnect_interval_ms);
                    }

                    thread::sleep(Duration::from_millis(reconnect_interval_ms));
                } else {
                    // When connected, just sleep (ping thread handles disconnection)
                    thread::sleep(Duration::from_millis(1000));
                }
            }
            info!("HID polling monitor thread stopped");
        });
    }

    /// Start ping thread for connection health monitoring
    fn start_ping_thread(&self) {
        let device = Arc::clone(&self.device);
        let connected = Arc::clone(&self.connected);
        let stop_monitor = Arc::clone(&self.stop_monitor);
        let event_tx = self.event_tx.clone();
        let ping_interval = self.config.ping_interval_ms;

        thread::spawn(move || {
            info!("HID ping thread started");
            let mut consecutive_failures: u32 = 0;

            while !stop_monitor.load(Ordering::Relaxed) {
                if connected.load(Ordering::Relaxed) {
                    let ping_ok = {
                        let device_guard = device.lock();
                        if let Some(ref dev) = *device_guard {
                            // Send ping using chunked protocol
                            let packets = commands::build_ping();
                            match send_packets_to_device(dev, &packets) {
                                Ok(()) => {
                                    debug!("Ping sent");
                                    // Read pong response
                                    match read_raw_packet(dev, 100) {
                                        Ok(Some(pkt)) => {
                                            if pkt.command() == Some(HidCommand::Ping) {
                                                debug!("Pong received");
                                            }
                                            // After pong, firmware may also send a state report
                                            // Try to read it with a short timeout
                                            if let Ok(Some(state_pkt)) = read_raw_packet(dev, 50) {
                                                if state_pkt.command() == Some(HidCommand::StateReport) {
                                                    let state_byte = state_pkt.payload()[0];
                                                    let ds = DeviceState::from_byte(state_byte);
                                                    debug!("State report after ping: mode={}, yolo={}", ds.mode, ds.yolo);
                                                    let _ = event_tx.send(AppEvent::DeviceStateChanged {
                                                        mode: ds.mode,
                                                        yolo: ds.yolo,
                                                    });
                                                }
                                            }
                                            true
                                        }
                                        Ok(None) => {
                                            debug!("No pong response");
                                            true // Write succeeded, device might be busy
                                        }
                                        Err(e) => {
                                            warn!("Error reading pong: {}", e);
                                            false
                                        }
                                    }
                                }
                                Err(e) => {
                                    warn!("Failed to send ping: {}", e);
                                    false
                                }
                            }
                        } else {
                            false
                        }
                    };

                    if ping_ok {
                        consecutive_failures = 0;
                    } else {
                        consecutive_failures += 1;
                        warn!(
                            "Ping failure {} of {}",
                            consecutive_failures, DISCONNECT_THRESHOLD
                        );

                        if consecutive_failures >= DISCONNECT_THRESHOLD {
                            info!("Device disconnected (consecutive ping failures)");
                            *device.lock() = None;
                            connected.store(false, Ordering::Relaxed);
                            let _ = event_tx.send(AppEvent::HidDisconnected);
                            consecutive_failures = 0;
                        }
                    }
                }
                thread::sleep(Duration::from_millis(ping_interval));
            }
            info!("HID ping thread stopped");
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
            device_info.manufacturer_string().unwrap_or("Unknown"),
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

    /// Send a display update with session name, current task, tab states, and active tab index
    pub fn send_display_update(&self, session: &str, task: Option<&str>, tabs: &[u8], active: usize) -> Result<()> {
        let device_guard = self.device.lock();
        let device = device_guard
            .as_ref()
            .ok_or_else(|| anyhow!("Device not connected"))?;

        let packets = commands::build_display_update(session, task, tabs, active);
        send_packets_to_device(device, &packets)?;

        self.drain_response(device);

        Ok(())
    }

    /// Set display brightness (chunked protocol)
    pub fn set_brightness(&self, level: u8, save: bool) -> Result<()> {
        let device_guard = self.device.lock();
        let device = device_guard
            .as_ref()
            .ok_or_else(|| anyhow!("Device not connected"))?;

        let packets = commands::build_set_brightness(level, save);
        send_packets_to_device(device, &packets)?;

        // Read response
        self.drain_response(device);

        info!("Brightness set to {}", level);
        Ok(())
    }

    /// Set a soft key assignment
    pub fn set_soft_key(&self, index: u8, key_type: SoftKeyType, data: &[u8], save: bool) -> Result<()> {
        let device_guard = self.device.lock();
        let device = device_guard
            .as_ref()
            .ok_or_else(|| anyhow!("Device not connected"))?;

        let packets = commands::build_set_soft_key(index, key_type, data, save);
        send_packets_to_device(device, &packets)?;

        self.drain_response(device);

        info!("Soft key {} set", index);
        Ok(())
    }

    /// Get a soft key configuration
    pub fn get_soft_key(&self, index: u8) -> Result<SoftKeyConfig> {
        let device_guard = self.device.lock();
        let device = device_guard
            .as_ref()
            .ok_or_else(|| anyhow!("Device not connected"))?;

        let packets = commands::build_get_soft_key(index);
        send_packets_to_device(device, &packets)?;

        // Read response — expect chunked response with key config data
        let response = read_response(device, HidCommand::GetSoftKey, &self.event_tx)?;

        // Parse response: [key_index, key_type, ...entry_data]
        // The firmware sends: send_response(cmd, status=0x00, [key_index, type, data...])
        // read_response() strips the status byte, so response.data = [key_index, type, entry_data...]
        if response.data.len() < 2 {
            return Ok(SoftKeyConfig {
                index,
                key_type: SoftKeyType::Default,
                data: vec![],
            });
        }

        let _key_index = response.data[0];
        let key_type = SoftKeyType::from_byte(response.data[1]).unwrap_or(SoftKeyType::Default);
        let data = if response.data.len() > 2 {
            response.data[2..].to_vec()
        } else {
            vec![]
        };

        Ok(SoftKeyConfig {
            index,
            key_type,
            data,
        })
    }

    /// Reset all soft keys to defaults
    ///
    /// Returns the effective assignment for each key post-reset.
    /// Format from firmware: [type, kc_hi, kc_lo] x 3
    pub fn reset_soft_keys(&self) -> Result<[SoftKeyConfig; 3]> {
        let device_guard = self.device.lock();
        let device = device_guard
            .as_ref()
            .ok_or_else(|| anyhow!("Device not connected"))?;

        let packets = commands::build_reset_soft_keys();
        send_packets_to_device(device, &packets)?;

        // Read the response — firmware now returns effective assignments
        let response = read_response(device, HidCommand::ResetSoftKeys, &self.event_tx)?;

        // Parse response data: [type, kc_hi, kc_lo] x 3
        let mut configs = [
            SoftKeyConfig { index: 0, key_type: SoftKeyType::Default, data: vec![] },
            SoftKeyConfig { index: 1, key_type: SoftKeyType::Default, data: vec![] },
            SoftKeyConfig { index: 2, key_type: SoftKeyType::Default, data: vec![] },
        ];

        for i in 0..3usize {
            let offset = i * 3;
            if offset + 2 < response.data.len() {
                let key_type = SoftKeyType::from_byte(response.data[offset])
                    .unwrap_or(SoftKeyType::Default);
                let kc_hi = response.data[offset + 1];
                let kc_lo = response.data[offset + 2];
                configs[i] = SoftKeyConfig {
                    index: i as u8,
                    key_type,
                    data: match key_type {
                        SoftKeyType::Keycode | SoftKeyType::Default => vec![kc_hi, kc_lo],
                        // String/Sequence only have kc=0 in the 0x06 response
                        _ => vec![],
                    },
                };
            }
        }

        info!("Soft keys reset to defaults");
        Ok(configs)
    }


    /// Set device LED mode
    pub fn set_mode(&self, mode: DeviceMode) -> Result<()> {
        let device_guard = self.device.lock();
        let device = device_guard
            .as_ref()
            .ok_or_else(|| anyhow!("Device not connected"))?;

        let packets = commands::build_set_mode(mode);
        send_packets_to_device(device, &packets)?;

        self.drain_response(device);

        info!("Device mode set to {}", mode);
        Ok(())
    }

    /// Read and discard response packets, dispatching any state reports
    fn drain_response(&self, device: &HidDevice) {
        // Read up to a few packets, handling state reports
        for _ in 0..3 {
            match read_raw_packet(device, 50) {
                Ok(Some(pkt)) => {
                    if pkt.command() == Some(HidCommand::StateReport) {
                        let state_byte = pkt.payload()[0];
                        let ds = DeviceState::from_byte(state_byte);
                        debug!("State report: mode={}, yolo={}", ds.mode, ds.yolo);
                        let _ = self.event_tx.send(AppEvent::DeviceStateChanged {
                            mode: ds.mode,
                            yolo: ds.yolo,
                        });
                    }
                    // If this is END packet of a response, we're done
                    if pkt.is_end() {
                        break;
                    }
                }
                Ok(None) => break, // Timeout, no more data
                Err(_) => break,
            }
        }
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
        self.stop_monitor.store(true, Ordering::Relaxed);
        #[cfg(target_os = "macos")]
        {
            if let Some(ref mut watcher) = self.hotplug_watcher {
                watcher.stop();
            }
        }
        self.disconnect();
    }
}

/// Try to open the HID device
fn try_open_device(api: &Arc<Mutex<HidApi>>, config: &HidConfig) -> Option<HidDevice> {
    let api_guard = api.lock();
    let device_info = api_guard.device_list().find(|d| {
        d.vendor_id() == config.vendor_id
            && d.product_id() == config.product_id
            && d.usage_page() == config.usage_page
            && d.usage() == config.usage_id
    })?;

    match device_info.open_device(&api_guard) {
        Ok(dev) => {
            if let Err(e) = dev.set_blocking_mode(false) {
                warn!("Failed to set non-blocking mode: {}", e);
                return None;
            }
            info!(
                "Opened device: {} {}",
                device_info.manufacturer_string().unwrap_or("Unknown"),
                device_info.product_string().unwrap_or("Unknown")
            );
            Some(dev)
        }
        Err(e) => {
            debug!("Failed to open device: {}", e);
            None
        }
    }
}

/// Send multiple packets (chunks) to the HID device sequentially
fn send_packets_to_device(device: &HidDevice, packets: &[HidPacket]) -> Result<()> {
    for packet in packets {
        send_single_packet(device, packet)?;
    }
    Ok(())
}

/// Send a single 32-byte packet to the HID device
fn send_single_packet(device: &HidDevice, packet: &HidPacket) -> Result<()> {
    let bytes = packet.as_bytes();

    #[cfg(any(target_os = "macos", target_os = "windows"))]
    let data = {
        let mut data = Vec::with_capacity(PACKET_SIZE + 1);
        data.push(0x00); // Report ID
        data.extend_from_slice(bytes);
        data
    };

    #[cfg(target_os = "linux")]
    let data = bytes.to_vec();

    let written = device
        .write(&data)
        .context("Failed to write to HID device")?;

    debug!("Wrote {} bytes to HID device", written);

    Ok(())
}

/// Read a single raw HID packet with timeout
fn read_raw_packet(device: &HidDevice, timeout_ms: i32) -> Result<Option<HidPacket>> {
    let mut buffer = [0u8; PACKET_SIZE];
    match device.read_timeout(&mut buffer, timeout_ms) {
        Ok(n) if n > 0 => Ok(Some(HidPacket::from_bytes(&buffer))),
        Ok(_) => Ok(None), // Timeout
        Err(e) => Err(anyhow!("HID read error: {}", e)),
    }
}

/// Read a complete chunked response for a specific command.
/// Transparently handles interleaved state reports by dispatching them as events.
fn read_response(
    device: &HidDevice,
    expected_cmd: HidCommand,
    event_tx: &mpsc::UnboundedSender<AppEvent>,
) -> Result<ResponsePacket> {
    let mut payload = Vec::new();
    let mut got_start = false;
    let mut command_byte = 0u8;

    // Read packets until we get a complete response (up to reasonable limit)
    for _ in 0..20 {
        let pkt = match read_raw_packet(device, 200)? {
            Some(pkt) => pkt,
            None => {
                if got_start {
                    // Timeout mid-response
                    return Err(anyhow!("Timeout waiting for response continuation"));
                } else {
                    return Err(anyhow!("Timeout waiting for response"));
                }
            }
        };

        // Handle interleaved state reports
        if pkt.command() == Some(HidCommand::StateReport) {
            let state_byte = pkt.payload()[0];
            let ds = DeviceState::from_byte(state_byte);
            debug!("State report during response read: mode={}, yolo={}", ds.mode, ds.yolo);
            let _ = event_tx.send(AppEvent::DeviceStateChanged {
                mode: ds.mode,
                yolo: ds.yolo,
            });
            continue; // Read next packet
        }

        // Check command matches
        if pkt.command() != Some(expected_cmd) && pkt.command() != Some(HidCommand::Error) {
            debug!(
                "Unexpected response command: {:?} (expected {:?})",
                pkt.command(),
                expected_cmd
            );
            continue;
        }

        if pkt.is_start() {
            got_start = true;
            command_byte = pkt.command_byte();
            payload.clear();
        }

        if got_start {
            payload.extend_from_slice(pkt.payload());
        }

        if pkt.is_end() && got_start {
            // Complete response assembled
            // Trim trailing zeros from the last chunk
            while payload.last() == Some(&0) {
                payload.pop();
            }

            let status = if payload.is_empty() { 0 } else { payload[0] };
            let data = if payload.len() > 1 {
                payload[1..].to_vec()
            } else {
                vec![]
            };

            return Ok(ResponsePacket {
                command: command_byte,
                status,
                data,
            });
        }
    }

    Err(anyhow!("Response read exceeded maximum packet count"))
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
