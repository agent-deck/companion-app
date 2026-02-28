//! CoreDeck Daemon - Background process that owns the HID device
//!
//! Provides WebSocket (exclusive) and HTTP REST (shared) APIs for
//! controlling the CoreDeck macropad.

mod hid;
mod rpc;
mod state;
mod tray;
mod ws;

use coredeck_protocol::{AppControlAction, DEFAULT_DAEMON_ADDR};
use clap::{Parser, Subcommand};
use hid::HidManager;
use state::{DaemonEvent, DaemonEventSender, DeviceStatus, TrayUpdate};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex, Notify, RwLock};
use tracing::{error, info};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

/// HID device configuration (matches the app's HidConfig)
#[derive(Debug, Clone)]
pub struct HidConfig {
    pub vendor_id: u16,
    pub product_id: u16,
    pub usage_page: u16,
    pub usage_id: u16,
    pub ping_interval_ms: u64,
    pub reconnect_interval_ms: u64,
}

impl Default for HidConfig {
    fn default() -> Self {
        Self {
            vendor_id: 0xFEED,
            product_id: 0x0803,
            usage_page: 0xFF60,
            usage_id: 0x61,
            ping_interval_ms: 5000,
            reconnect_interval_ms: 2000,
        }
    }
}

/// Shared state across the daemon (must be Send + Sync for axum)
pub struct DaemonState {
    /// HID device manager
    pub hid: Mutex<HidManager>,
    /// Current device status
    pub device_status: RwLock<DeviceStatus>,
    /// Connected WS client (the lock)
    pub ws_client: Mutex<Option<ws::WsClientHandle>>,
    /// Notified when WS lock changes
    pub notify_lock_change: Notify,
    /// Channel to send tray updates to the main thread (tray is !Send, lives on main thread)
    pub tray_tx: std::sync::mpsc::Sender<TrayUpdate>,
}

impl DaemonState {
    pub fn send_tray_update(&self, update: TrayUpdate) {
        let _ = self.tray_tx.send(update);
    }
}

#[derive(Parser)]
#[command(name = "coredeck-daemon", about = "CoreDeck background daemon")]
struct Cli {
    /// Listen address
    #[arg(long, default_value = DEFAULT_DAEMON_ADDR)]
    listen: String,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Install launchd plist for auto-start
    Install,
    /// Uninstall launchd plist
    Uninstall,
}

fn main() {
    // Initialize logging
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let cli = Cli::parse();

    // Handle install/uninstall subcommands
    match cli.command {
        Some(Commands::Install) => {
            install_launchd(&cli.listen);
            return;
        }
        Some(Commands::Uninstall) => {
            uninstall_launchd();
            return;
        }
        None => {}
    }

    info!("Starting CoreDeck daemon on {}", cli.listen);

    // macOS: set activation policy to Accessory (no dock icon, just tray)
    #[cfg(target_os = "macos")]
    setup_macos_accessory();

    // Create event channel for HID events
    let (event_tx, event_rx) = mpsc::unbounded_channel::<DaemonEvent>();
    let event_sender = DaemonEventSender::new(event_tx);

    // Initialize HID manager
    let hid_config = HidConfig::default();
    let hid_manager = match HidManager::new(hid_config, event_sender) {
        Ok(hid) => {
            info!("HID manager initialized");
            hid
        }
        Err(e) => {
            error!("Failed to initialize HID manager: {}", e);
            std::process::exit(1);
        }
    };

    // Create tray (must happen on main thread on macOS)
    let (tray_manager, tray_action_rx) = match tray::DaemonTrayManager::new() {
        Ok((tray, rx)) => (Some(tray), Some(rx)),
        Err(e) => {
            error!("Failed to create tray: {}", e);
            (None, None)
        }
    };

    // Channel for async code to send tray updates to main thread
    let (tray_update_tx, tray_update_rx) = std::sync::mpsc::channel::<TrayUpdate>();

    // Initialize device status from HID manager's enumeration
    let initial_status = DeviceStatus {
        available: hid_manager.is_device_available(),
        device_name: hid_manager.cached_device_name(),
        ..DeviceStatus::default()
    };

    // Build shared state (Send + Sync — no tray handle here)
    let state = Arc::new(DaemonState {
        hid: Mutex::new(hid_manager),
        device_status: RwLock::new(initial_status),
        ws_client: Mutex::new(None),
        notify_lock_change: Notify::new(),
        tray_tx: tray_update_tx,
    });

    // Run the tokio runtime + axum server on a spawned thread.
    // The winit event loop must run on the main thread (required for tray on macOS).
    let state_clone = Arc::clone(&state);
    let listen_addr = cli.listen.clone();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("Failed to create tokio runtime");

        rt.block_on(async move {
            run_async(state_clone, event_rx, listen_addr).await;
        });
    });

    // Handle tray events on main thread (via winit event loop)
    run_main_event_loop(state, tray_manager, tray_action_rx, tray_update_rx);
}

/// Run the async daemon (axum server + event processing)
async fn run_async(
    state: Arc<DaemonState>,
    mut event_rx: mpsc::UnboundedReceiver<DaemonEvent>,
    listen_addr: String,
) {
    // Build axum router
    let app = axum::Router::new()
        .route("/ws", axum::routing::get(ws::ws_handler))
        .route("/api/status", axum::routing::get(rpc::get_status))
        .route("/api/display", axum::routing::post(rpc::post_display))
        .route("/api/alert", axum::routing::post(rpc::post_alert))
        .route("/api/alert/clear", axum::routing::post(rpc::post_alert_clear))
        .route("/api/brightness", axum::routing::post(rpc::post_brightness))
        .route("/api/mode", axum::routing::post(rpc::post_mode))
        .route("/api/version", axum::routing::get(rpc::get_version))
        .layer(
            tower_http::cors::CorsLayer::new()
                .allow_origin(tower_http::cors::Any)
                .allow_methods(tower_http::cors::Any)
                .allow_headers(tower_http::cors::Any),
        )
        .with_state(Arc::clone(&state));

    // Start HTTP/WS server
    let listener = match tokio::net::TcpListener::bind(&listen_addr).await {
        Ok(l) => {
            info!("Listening on {}", listen_addr);
            l
        }
        Err(e) => {
            error!("Failed to bind to {}: {}", listen_addr, e);
            std::process::exit(1);
        }
    };

    let server = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            error!("Server error: {}", e);
        }
    });

    // Process HID events and forward to WS client
    let state_for_events = Arc::clone(&state);
    let event_handler = tokio::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            // Update shared device status and notify tray
            match &event {
                DaemonEvent::HidConnected { device_name, firmware_version } => {
                    let mut status = state_for_events.device_status.write().await;
                    status.available = true;
                    status.connected = true;
                    status.device_name = Some(device_name.clone());
                    status.firmware_version = Some(firmware_version.clone());

                    state_for_events.send_tray_update(TrayUpdate::DeviceConnected(device_name.clone()));
                }
                DaemonEvent::HidDisconnected => {
                    let mut status = state_for_events.device_status.write().await;
                    status.connected = false;
                    status.device_name = None;
                    status.firmware_version = None;

                    state_for_events.send_tray_update(TrayUpdate::DeviceDisconnected);
                }
                DaemonEvent::DeviceAvailable { device_name } => {
                    let mut status = state_for_events.device_status.write().await;
                    status.available = true;
                    status.device_name = Some(device_name.clone());
                    state_for_events.send_tray_update(TrayUpdate::DeviceAvailable(device_name.clone()));
                }
                DaemonEvent::DeviceUnavailable => {
                    let mut status = state_for_events.device_status.write().await;
                    status.available = false;
                    status.device_name = None;
                    status.firmware_version = None;
                    state_for_events.send_tray_update(TrayUpdate::DeviceUnavailable);
                }
                DaemonEvent::DeviceStateChanged { mode, yolo } => {
                    let mut status = state_for_events.device_status.write().await;
                    status.mode = *mode;
                    status.yolo = *yolo;
                }
                _ => {}
            }

            // Forward to WS client
            ws::forward_event_to_ws(&state_for_events, &event).await;
        }
    });

    // Wait for shutdown
    tokio::select! {
        _ = server => {}
        _ = event_handler => {}
        _ = tokio::signal::ctrl_c() => {
            info!("Received Ctrl+C, shutting down...");
        }
    }
}

/// Run the winit event loop on the main thread (for tray icon support on macOS)
fn run_main_event_loop(
    state: Arc<DaemonState>,
    mut tray_manager: Option<tray::DaemonTrayManager>,
    tray_action_rx: Option<std::sync::mpsc::Receiver<tray::DaemonTrayAction>>,
    tray_update_rx: std::sync::mpsc::Receiver<TrayUpdate>,
) {
    use winit::application::ApplicationHandler;
    use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};

    struct TrayApp {
        state: Arc<DaemonState>,
        tray_manager: Option<tray::DaemonTrayManager>,
        tray_action_rx: Option<std::sync::mpsc::Receiver<tray::DaemonTrayAction>>,
        tray_update_rx: std::sync::mpsc::Receiver<TrayUpdate>,
    }

    impl ApplicationHandler for TrayApp {
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            event_loop.set_control_flow(ControlFlow::Wait);
        }

        fn window_event(
            &mut self,
            _event_loop: &ActiveEventLoop,
            _window_id: winit::window::WindowId,
            _event: winit::event::WindowEvent,
        ) {}

        fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
            // Process tray updates from async code (non-blocking)
            while let Ok(update) = self.tray_update_rx.try_recv() {
                if let Some(ref mut tray) = self.tray_manager {
                    match update {
                        TrayUpdate::DeviceConnected(name) => {
                            tray.set_device_status(tray::DevicePresence::Active, Some(&name));
                        }
                        TrayUpdate::DeviceDisconnected => {
                            // Device interface closed but may still be physically available
                            tray.set_device_status(tray::DevicePresence::Available, None);
                        }
                        TrayUpdate::DeviceAvailable(name) => {
                            tray.set_device_status(tray::DevicePresence::Available, Some(&name));
                        }
                        TrayUpdate::DeviceUnavailable => {
                            tray.set_device_status(tray::DevicePresence::None, None);
                        }
                        TrayUpdate::AppConnected => {
                            tray.set_app_connected(true);
                        }
                        TrayUpdate::AppDisconnected => {
                            tray.set_app_connected(false);
                        }
                    }
                }
            }

            // Poll tray menu actions (non-blocking)
            if let Some(ref rx) = self.tray_action_rx {
                while let Ok(action) = rx.try_recv() {
                    match action {
                        tray::DaemonTrayAction::ToggleApp => {
                            // Send show/hide to app via WS
                            let state = Arc::clone(&self.state);
                            std::thread::spawn(move || {
                                let rt = tokio::runtime::Builder::new_current_thread()
                                    .enable_all()
                                    .build()
                                    .unwrap();
                                rt.block_on(async {
                                    ws::send_app_control(&state, AppControlAction::ShowWindow).await;
                                });
                            });
                        }
                        tray::DaemonTrayAction::Quit => {
                            info!("Quit requested from tray");
                            event_loop.exit();
                        }
                    }
                }
            }
        }
    }

    let event_loop = EventLoop::new().expect("Failed to create event loop");
    let mut app = TrayApp {
        state,
        tray_manager,
        tray_action_rx,
        tray_update_rx,
    };
    let _ = event_loop.run_app(&mut app);

    info!("Daemon exiting");
    std::process::exit(0);
}

#[cfg(target_os = "macos")]
#[allow(deprecated)]
fn setup_macos_accessory() {
    use cocoa::appkit::NSApp;
    use objc::{sel, sel_impl};

    unsafe {
        let app = NSApp();
        // NSApplicationActivationPolicyAccessory = 1 (no dock icon)
        let _: () = objc::msg_send![app, setActivationPolicy: 1_isize];
    }
}

#[cfg(not(target_os = "macos"))]
fn setup_macos_accessory() {}

// ── launchd install/uninstall ──────────────────────────────────────

fn install_launchd(listen: &str) {
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var("HOME").expect("HOME not set");
        let plist_dir = format!("{}/Library/LaunchAgents", home);
        let plist_path = format!("{}/com.coredeck.daemon.plist", plist_dir);

        let exe = std::env::current_exe()
            .expect("Failed to get current exe path")
            .to_string_lossy()
            .to_string();

        let plist = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.coredeck.daemon</string>
    <key>ProgramArguments</key>
    <array>
        <string>{exe}</string>
        <string>--listen</string>
        <string>{listen}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>{home}/Library/Logs/coredeck-daemon.log</string>
    <key>StandardErrorPath</key>
    <string>{home}/Library/Logs/coredeck-daemon.log</string>
</dict>
</plist>"#
        );

        std::fs::create_dir_all(&plist_dir).expect("Failed to create LaunchAgents dir");
        std::fs::write(&plist_path, plist).expect("Failed to write plist");

        // Load the plist
        let status = std::process::Command::new("launchctl")
            .args(["load", &plist_path])
            .status()
            .expect("Failed to run launchctl");

        if status.success() {
            println!("Installed and loaded: {}", plist_path);
        } else {
            eprintln!("launchctl load failed (exit {})", status.code().unwrap_or(-1));
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = listen;
        eprintln!("launchd is only available on macOS");
    }
}

fn uninstall_launchd() {
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var("HOME").expect("HOME not set");
        let plist_path = format!("{}/Library/LaunchAgents/com.coredeck.daemon.plist", home);

        if std::path::Path::new(&plist_path).exists() {
            let _ = std::process::Command::new("launchctl")
                .args(["unload", &plist_path])
                .status();
            std::fs::remove_file(&plist_path).expect("Failed to remove plist");
            println!("Uninstalled: {}", plist_path);
        } else {
            println!("Plist not found: {}", plist_path);
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        eprintln!("launchd is only available on macOS");
    }
}
