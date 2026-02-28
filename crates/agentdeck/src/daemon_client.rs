//! WebSocket client for communicating with the agentdeck-daemon.
//!
//! `DaemonClient` mirrors the public API of the former `HidManager` so that
//! call-sites in `main.rs` can switch with minimal changes.

use agentdeck_protocol::{
    AlertRequest, AppControlAction, DeviceInfo, DeviceMode, DeviceState, DisplayUpdate,
    SoftKeyConfig, SoftKeyType, WsCommandTag, WsEventTag, WsResponseTag, decode_ws_frame,
    encode_ws_frame,
};
use anyhow::{anyhow, Result};
use futures_util::{SinkExt, StreamExt};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::Arc;
use tokio::sync::{Mutex, oneshot};
use tracing::{debug, error, info, warn};

/// Name of the daemon binary (same directory as the app binary)
const DAEMON_BIN_NAME: &str = "agentdeck-daemon";

use crate::core::events::{AppEvent, EventSender, TrayAction};

/// WebSocket client that talks to the agentdeck-daemon.
pub struct DaemonClient {
    /// Send binary WS frames to the daemon
    ws_tx: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
    /// Sequence counter for request-response correlation (wraps, skips 0)
    seq: AtomicU16,
    /// Pending responses keyed by sequence number
    pending: Arc<Mutex<HashMap<u16, oneshot::Sender<(u8, Vec<u8>)>>>>,
    /// Whether the WS connection is alive
    connected: Arc<AtomicBool>,
    /// Last display payload sent (for deduplication, same as old HidManager)
    last_display_payload: parking_lot::Mutex<String>,
}

impl DaemonClient {
    /// Connect to the daemon at the given address.
    ///
    /// Spawns a background reader thread that forwards daemon events to the
    /// app's `EventSender` (same pattern as the old `HidManager`).
    pub fn connect(addr: &str, event_tx: EventSender) -> Result<Self> {
        let url = format!("ws://{}/ws", addr);

        let (ws_tx, ws_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
        let connected = Arc::new(AtomicBool::new(false));
        let pending: Arc<Mutex<HashMap<u16, oneshot::Sender<(u8, Vec<u8>)>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        let connected_clone = Arc::clone(&connected);
        let pending_clone = Arc::clone(&pending);

        // Spawn background thread with its own tokio runtime for the WS connection
        std::thread::Builder::new()
            .name("daemon-ws-client".into())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("Failed to create tokio runtime for daemon client");

                rt.block_on(async move {
                    run_ws_loop(url, ws_rx, connected_clone, pending_clone, event_tx).await;
                });
            })?;

        Ok(Self {
            ws_tx,
            seq: AtomicU16::new(1),
            pending,
            connected,
            last_display_payload: parking_lot::Mutex::new(String::new()),
        })
    }

    /// Whether the daemon connection is alive.
    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    /// Allocate the next non-zero sequence number.
    fn next_seq(&self) -> u16 {
        loop {
            let s = self.seq.fetch_add(1, Ordering::Relaxed);
            if s != 0 {
                return s;
            }
        }
    }

    /// Send a fire-and-forget command (no response expected).
    fn fire_and_forget(&self, tag: WsCommandTag, payload: &[u8]) -> Result<()> {
        let seq = self.next_seq();
        let frame = encode_ws_frame(tag as u8, seq, payload);
        self.ws_tx
            .send(frame)
            .map_err(|_| anyhow!("Daemon connection closed"))
    }

    // ── Public API (mirrors HidManager) ──────────────────────────────

    /// Send a display update. Skips if payload is identical to the last one.
    pub fn send_display_update(
        &self,
        session: &str,
        task: Option<&str>,
        task2: Option<&str>,
        tabs: &[u8],
        active: usize,
    ) -> Result<()> {
        // Dedup
        let payload_key = format!("{}|{}|{}|{:?}|{}", session, task.unwrap_or(""), task2.unwrap_or(""), tabs, active);
        {
            let mut last = self.last_display_payload.lock();
            if *last == payload_key {
                return Ok(());
            }
            *last = payload_key;
        }

        let update = DisplayUpdate {
            session: session.to_string(),
            task: task.unwrap_or("").to_string(),
            task2: task2.unwrap_or("").to_string(),
            tabs: tabs.to_vec(),
            active,
        };
        let json = serde_json::to_vec(&update)?;
        self.fire_and_forget(WsCommandTag::UpdateDisplay, &json)
    }

    /// Set the device LED mode.
    pub fn set_mode(&self, mode: DeviceMode) -> Result<()> {
        self.fire_and_forget(WsCommandTag::SetMode, &[mode as u8])
    }

    /// Send an alert overlay to the device display.
    pub fn send_alert(
        &self,
        tab: usize,
        session: &str,
        text: &str,
        details: Option<&str>,
    ) -> Result<()> {
        let req = AlertRequest {
            tab,
            session: session.to_string(),
            text: text.to_string(),
            details: details.map(|s| s.to_string()),
        };
        let json = serde_json::to_vec(&req)?;
        self.fire_and_forget(WsCommandTag::Alert, &json)
    }

    /// Clear the alert overlay for a specific tab.
    pub fn clear_alert(&self, tab: usize) -> Result<()> {
        self.fire_and_forget(WsCommandTag::ClearAlert, &[tab as u8])
    }

    /// Set display brightness.
    pub fn set_brightness(&self, level: u8, save: bool) -> Result<()> {
        self.fire_and_forget(
            WsCommandTag::SetBrightness,
            &[level, if save { 1 } else { 0 }],
        )
    }

    /// Get a soft key configuration from the device (blocking).
    pub fn get_soft_key(&self, index: u8) -> Result<SoftKeyConfig> {
        // We need to run the async request on a thread that has a tokio runtime.
        // Since this is called from the main winit thread, we spawn a blocking task.
        let pending = Arc::clone(&self.pending);
        let seq = self.next_seq();

        let (tx, rx) = oneshot::channel();
        {
            // Use try_lock to avoid async in sync context
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            rt.block_on(async {
                let mut map = pending.lock().await;
                map.insert(seq, tx);
            });
        }

        let frame = encode_ws_frame(WsCommandTag::GetSoftKey as u8, seq, &[index]);
        self.ws_tx
            .send(frame)
            .map_err(|_| anyhow!("Daemon connection closed"))?;

        // Block waiting for response (with timeout)
        match rx.blocking_recv() {
            Ok((tag, data)) => {
                if tag == WsResponseTag::CommandError as u8 {
                    let msg = String::from_utf8_lossy(&data);
                    return Err(anyhow!("Device error: {}", msg));
                }
                // Parse SoftKeyResponse: [index, type, ...data]
                if data.len() >= 2 {
                    let key_type = SoftKeyType::from_byte(data[1]).unwrap_or(SoftKeyType::Default);
                    Ok(SoftKeyConfig {
                        index: data[0],
                        key_type,
                        data: data[2..].to_vec(),
                    })
                } else {
                    Err(anyhow!("Invalid soft key response"))
                }
            }
            Err(_) => Err(anyhow!("Response channel dropped")),
        }
    }

    /// Set a soft key configuration on the device.
    pub fn set_soft_key(
        &self,
        index: u8,
        key_type: SoftKeyType,
        data: &[u8],
        save: bool,
    ) -> Result<()> {
        let mut payload = vec![index, key_type as u8, if save { 1 } else { 0 }];
        payload.extend_from_slice(data);
        self.fire_and_forget(WsCommandTag::SetSoftKey, &payload)
    }

    /// Reset all soft keys to defaults and return the new configurations (blocking).
    pub fn reset_soft_keys(&self) -> Result<[SoftKeyConfig; 3]> {
        let pending = Arc::clone(&self.pending);
        let seq = self.next_seq();

        let (tx, rx) = oneshot::channel();
        {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            rt.block_on(async {
                let mut map = pending.lock().await;
                map.insert(seq, tx);
            });
        }

        let frame = encode_ws_frame(WsCommandTag::ResetSoftKeys as u8, seq, &[]);
        self.ws_tx
            .send(frame)
            .map_err(|_| anyhow!("Daemon connection closed"))?;

        match rx.blocking_recv() {
            Ok((tag, data)) => {
                if tag == WsResponseTag::CommandError as u8 {
                    let msg = String::from_utf8_lossy(&data);
                    return Err(anyhow!("Device error: {}", msg));
                }
                // Parse 3 soft key configs: [index, type, data_len, ...data] × 3
                let mut configs = [
                    SoftKeyConfig { index: 0, key_type: SoftKeyType::Default, data: vec![] },
                    SoftKeyConfig { index: 1, key_type: SoftKeyType::Default, data: vec![] },
                    SoftKeyConfig { index: 2, key_type: SoftKeyType::Default, data: vec![] },
                ];
                let mut offset = 0;
                for config in &mut configs {
                    if offset + 3 > data.len() {
                        break;
                    }
                    config.index = data[offset];
                    config.key_type =
                        SoftKeyType::from_byte(data[offset + 1]).unwrap_or(SoftKeyType::Default);
                    let data_len = data[offset + 2] as usize;
                    offset += 3;
                    if offset + data_len <= data.len() {
                        config.data = data[offset..offset + data_len].to_vec();
                        offset += data_len;
                    }
                }
                Ok(configs)
            }
            Err(_) => Err(anyhow!("Response channel dropped")),
        }
    }

    /// Query the firmware version string (blocking).
    pub fn query_version(&self) -> String {
        let pending = Arc::clone(&self.pending);
        let seq = self.next_seq();

        let (tx, rx) = oneshot::channel();
        {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(_) => return "unknown".to_string(),
            };
            rt.block_on(async {
                let mut map = pending.lock().await;
                map.insert(seq, tx);
            });
        }

        let frame = encode_ws_frame(WsCommandTag::GetVersion as u8, seq, &[]);
        if self.ws_tx.send(frame).is_err() {
            return "unknown".to_string();
        }

        match rx.blocking_recv() {
            Ok((_tag, data)) => String::from_utf8(data).unwrap_or_else(|_| "unknown".to_string()),
            Err(_) => "unknown".to_string(),
        }
    }
}

// ── Background WebSocket loop ────────────────────────────────────────

/// Try to find and spawn the daemon binary as a detached process.
///
/// Looks for `agentdeck-daemon` in:
/// 1. Same directory as the current executable
/// 2. PATH
fn try_spawn_daemon(listen_addr: &str) -> bool {
    // Try same directory as the current executable
    let daemon_path = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|p| p.join(DAEMON_BIN_NAME)))
        .filter(|p| p.exists());

    let daemon_cmd = match daemon_path {
        Some(ref p) => p.as_os_str(),
        None => std::ffi::OsStr::new(DAEMON_BIN_NAME), // Fall back to PATH
    };

    info!("Attempting to start daemon: {:?}", daemon_cmd);

    match std::process::Command::new(daemon_cmd)
        .arg("--listen")
        .arg(listen_addr)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(child) => {
            // Detach: drop the Child handle so the daemon keeps running
            std::mem::forget(child);
            info!("Daemon spawned successfully");
            true
        }
        Err(e) => {
            warn!("Failed to spawn daemon: {}", e);
            false
        }
    }
}

/// Extract the `host:port` listen address from a `ws://host:port/ws` URL.
fn listen_addr_from_url(url: &str) -> String {
    url.strip_prefix("ws://")
        .and_then(|rest| rest.strip_suffix("/ws"))
        .unwrap_or(agentdeck_protocol::DEFAULT_DAEMON_ADDR)
        .to_string()
}

async fn run_ws_loop(
    url: String,
    mut outgoing_rx: tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
    connected: Arc<AtomicBool>,
    pending: Arc<Mutex<HashMap<u16, oneshot::Sender<(u8, Vec<u8>)>>>>,
    event_tx: EventSender,
) {
    let mut backoff_ms: u64 = 500;
    const MAX_BACKOFF_MS: u64 = 5000;
    let mut daemon_spawn_attempted = false;

    loop {
        info!("Connecting to daemon at {}...", url);

        match tokio_tungstenite::connect_async(&url).await {
            Ok((ws_stream, _)) => {
                info!("Connected to daemon");
                connected.store(true, Ordering::Relaxed);
                backoff_ms = 500; // Reset backoff on success
                daemon_spawn_attempted = false; // Allow re-spawn after future disconnect

                let _ = event_tx.send(AppEvent::DaemonConnected);

                let (mut ws_sink, mut ws_stream_rx) = ws_stream.split();

                // Forward outgoing frames to the WebSocket
                let writer = tokio::spawn(async move {
                    while let Some(frame) = outgoing_rx.recv().await {
                        use tokio_tungstenite::tungstenite::Message;
                        if ws_sink.send(Message::Binary(frame.into())).await.is_err() {
                            break;
                        }
                    }
                    // Return the receiver so we can reuse it after reconnect
                    outgoing_rx
                });

                // Read incoming frames from the daemon
                loop {
                    match ws_stream_rx.next().await {
                        Some(Ok(msg)) => {
                            use tokio_tungstenite::tungstenite::Message;
                            match msg {
                                Message::Binary(data) => {
                                    handle_daemon_frame(
                                        &data,
                                        &pending,
                                        &event_tx,
                                    )
                                    .await;
                                }
                                Message::Close(_) => {
                                    info!("Daemon closed WS connection");
                                    break;
                                }
                                _ => {} // Ignore text/ping/pong
                            }
                        }
                        Some(Err(e)) => {
                            warn!("WS read error: {}", e);
                            break;
                        }
                        None => {
                            info!("WS stream ended");
                            break;
                        }
                    }
                }

                // Connection lost
                connected.store(false, Ordering::Relaxed);
                let _ = event_tx.send(AppEvent::DaemonDisconnected);

                // Cancel writer and recover the outgoing_rx
                writer.abort();
                match writer.await {
                    Ok(rx) => outgoing_rx = rx,
                    Err(_) => {
                        // Writer was aborted, we can't recover outgoing_rx.
                        // The DaemonClient is effectively dead — stop reconnecting.
                        error!("Lost outgoing channel after disconnect, stopping reconnect");
                        return;
                    }
                }
            }
            Err(e) => {
                debug!("Failed to connect to daemon: {} (retry in {}ms)", e, backoff_ms);

                // Auto-start: try spawning the daemon on the first connection failure
                if !daemon_spawn_attempted {
                    daemon_spawn_attempted = true;
                    let addr = listen_addr_from_url(&url);
                    if try_spawn_daemon(&addr) {
                        // Give daemon time to start listening
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                        backoff_ms = 500; // Reset backoff for the fresh attempt
                        continue;
                    }
                }
            }
        }

        // Backoff before retry
        tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
        backoff_ms = (backoff_ms * 3 / 2).min(MAX_BACKOFF_MS);
    }
}

/// Process a single binary frame from the daemon.
async fn handle_daemon_frame(
    data: &[u8],
    pending: &Arc<Mutex<HashMap<u16, oneshot::Sender<(u8, Vec<u8>)>>>>,
    event_tx: &EventSender,
) {
    let (tag, seq, payload) = match decode_ws_frame(data) {
        Some(v) => v,
        None => return,
    };

    // Events (seq == 0): map to AppEvent
    if seq == 0 {
        if let Some(event_tag) = WsEventTag::from_byte(tag) {
            match event_tag {
                WsEventTag::DeviceConnected => {
                    let info: DeviceInfo = serde_json::from_slice(payload).unwrap_or(DeviceInfo {
                        name: "Agent Deck".to_string(),
                        firmware: "unknown".to_string(),
                    });
                    let _ = event_tx.send(AppEvent::HidConnected {
                        device_name: info.name,
                        firmware_version: info.firmware,
                    });
                }
                WsEventTag::DeviceDisconnected => {
                    let _ = event_tx.send(AppEvent::HidDisconnected);
                }
                WsEventTag::StateChanged => {
                    if !payload.is_empty() {
                        let state = DeviceState::from_byte(payload[0]);
                        let _ = event_tx.send(AppEvent::DeviceStateChanged {
                            mode: state.mode,
                            yolo: state.yolo,
                        });
                    }
                }
                WsEventTag::KeyEvent => {
                    if payload.len() >= 2 {
                        let keycode = ((payload[0] as u16) << 8) | (payload[1] as u16);
                        let _ = event_tx.send(AppEvent::HidKeyEvent { keycode });
                    }
                }
                WsEventTag::TypeString => {
                    if !payload.is_empty() {
                        let send_enter = payload[0] != 0;
                        let text = String::from_utf8_lossy(&payload[1..]).to_string();
                        let _ = event_tx.send(AppEvent::HidTypeString { text, send_enter });
                    }
                }
                WsEventTag::AppControl => {
                    if !payload.is_empty() {
                        if let Some(action) = AppControlAction::from_byte(payload[0]) {
                            match action {
                                AppControlAction::ShowWindow | AppControlAction::HideWindow => {
                                    let _ =
                                        event_tx.send(AppEvent::TrayAction(TrayAction::ToggleWindow));
                                }
                            }
                        }
                    }
                }
            }
        }
        return;
    }

    // Responses (seq > 0): route to pending request
    if let Some(response_tag) = WsResponseTag::from_byte(tag) {
        let mut map = pending.lock().await;
        if let Some(sender) = map.remove(&seq) {
            let _ = sender.send((tag, payload.to_vec()));
        } else {
            debug!("No pending request for seq={} (tag={:?})", seq, response_tag);
        }
    } else {
        // Could be a CommandAck for fire-and-forget — just ignore
        let mut map = pending.lock().await;
        if let Some(sender) = map.remove(&seq) {
            let _ = sender.send((tag, payload.to_vec()));
        }
    }
}
