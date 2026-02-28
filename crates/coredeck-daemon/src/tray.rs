//! Daemon tray icon and menu
//!
//! Simplified tray for the daemon: device status, Show/Hide app, Quit.

use anyhow::{Context, Result};
use tray_icon::{
    menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem},
    TrayIcon as TrayIconHandle, TrayIconBuilder,
};
use tracing::{debug, error, info};

/// Device presence for tray display
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DevicePresence {
    /// No device plugged in
    None,
    /// Device plugged in but HID interface not open
    Available,
    /// Device plugged in AND HID interface open (app connected)
    Active,
}

/// Tray menu actions
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonTrayAction {
    /// Show/Hide the app window (sent via WS to the connected app)
    ToggleApp,
    /// Quit the daemon
    Quit,
}

/// Daemon tray manager
pub struct DaemonTrayManager {
    tray: TrayIconHandle,
    icons: TrayIcons,
    toggle_item: MenuItem,
    toggle_id: MenuId,
    quit_id: MenuId,
}

impl DaemonTrayManager {
    pub fn new() -> Result<(Self, std::sync::mpsc::Receiver<DaemonTrayAction>)> {
        let icons = TrayIcons::new().context("Failed to load tray icons")?;

        let menu = Menu::new();

        let toggle_item = MenuItem::new("Show Core Deck", false, None);
        let toggle_id = toggle_item.id().clone();

        let quit_item = MenuItem::new("Quit Daemon", true, None);
        let quit_id = quit_item.id().clone();

        menu.append(&toggle_item)?;
        menu.append(&PredefinedMenuItem::separator())?;
        menu.append(&quit_item)?;

        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("Core Deck Daemon - Disconnected")
            .with_icon(icons.disconnected().clone())
            .build()
            .context("Failed to create tray icon")?;

        info!("Daemon tray icon created");

        let (action_tx, action_rx) = std::sync::mpsc::channel();

        // Menu event handler thread
        let toggle_id_clone = toggle_id.clone();
        let quit_id_clone = quit_id.clone();
        std::thread::spawn(move || {
            let receiver = MenuEvent::receiver();
            loop {
                if let Ok(event) = receiver.recv() {
                    debug!("Daemon menu event: {:?}", event);
                    let action = if event.id == toggle_id_clone {
                        Some(DaemonTrayAction::ToggleApp)
                    } else if event.id == quit_id_clone {
                        Some(DaemonTrayAction::Quit)
                    } else {
                        None
                    };
                    if let Some(action) = action {
                        if action_tx.send(action).is_err() {
                            break;
                        }
                    }
                }
            }
        });

        let manager = Self {
            tray,
            icons,
            toggle_item,
            toggle_id,
            quit_id,
        };

        Ok((manager, action_rx))
    }

    /// Update tray to reflect device presence state
    pub fn set_device_status(&mut self, presence: DevicePresence, device_name: Option<&str>) {
        let icon = match presence {
            DevicePresence::Active => self.icons.connected(),
            DevicePresence::Available => self.icons.connected(),
            DevicePresence::None => self.icons.disconnected(),
        };

        let tooltip = match presence {
            DevicePresence::Active => {
                format!("Core Deck Daemon - {}", device_name.unwrap_or("Active"))
            }
            DevicePresence::Available => {
                format!("Core Deck Daemon - {} (idle)", device_name.unwrap_or("Available"))
            }
            DevicePresence::None => "Core Deck Daemon - No device".to_string(),
        };

        if let Err(e) = self.tray.set_icon(Some(icon.clone())) {
            error!("Failed to set tray icon: {}", e);
        }
        if let Err(e) = self.tray.set_tooltip(Some(&tooltip)) {
            error!("Failed to set tray tooltip: {}", e);
        }
    }

    /// Enable/disable the "Show/Hide" toggle based on WS client connection
    pub fn set_app_connected(&mut self, connected: bool) {
        self.toggle_item.set_enabled(connected);
        if connected {
            self.toggle_item.set_text("Show Core Deck");
        } else {
            self.toggle_item.set_text("Show Core Deck");
        }
    }

    /// Update toggle text based on app visibility
    pub fn set_app_visible(&mut self, visible: bool) {
        let text = if visible { "Hide Core Deck" } else { "Show Core Deck" };
        self.toggle_item.set_text(text);
    }
}

// ── Tray icons ─────────────────────────────────────────────────────

const CONNECTED_DARK_DATA: &[u8] = include_bytes!("../assets/icons/tray_connected.png");
const DISCONNECTED_DARK_DATA: &[u8] = include_bytes!("../assets/icons/tray_disconnected.png");
const CONNECTED_LIGHT_DATA: &[u8] = include_bytes!("../assets/icons/tray_connected_light.png");
const DISCONNECTED_LIGHT_DATA: &[u8] = include_bytes!("../assets/icons/tray_disconnected_light.png");

struct TrayIcons {
    connected_dark: tray_icon::Icon,
    disconnected_dark: tray_icon::Icon,
    connected_light: tray_icon::Icon,
    disconnected_light: tray_icon::Icon,
}

impl TrayIcons {
    fn new() -> Result<Self> {
        Ok(Self {
            connected_dark: load_icon_from_png(CONNECTED_DARK_DATA)?,
            disconnected_dark: load_icon_from_png(DISCONNECTED_DARK_DATA)?,
            connected_light: load_icon_from_png(CONNECTED_LIGHT_DATA)?,
            disconnected_light: load_icon_from_png(DISCONNECTED_LIGHT_DATA)?,
        })
    }

    fn connected(&self) -> &tray_icon::Icon {
        if is_dark_mode() { &self.connected_dark } else { &self.connected_light }
    }

    fn disconnected(&self) -> &tray_icon::Icon {
        if is_dark_mode() { &self.disconnected_dark } else { &self.disconnected_light }
    }
}

#[cfg(target_os = "macos")]
fn is_dark_mode() -> bool {
    use cocoa::base::{id, nil};
    use cocoa::foundation::NSString;
    use objc::{msg_send, sel, sel_impl};

    unsafe {
        let user_defaults: id = msg_send![objc::class!(NSUserDefaults), standardUserDefaults];
        let key = NSString::alloc(nil).init_str("AppleInterfaceStyle");
        let value: id = msg_send![user_defaults, stringForKey: key];
        if value == nil {
            false
        } else {
            let utf8: *const i8 = msg_send![value, UTF8String];
            if utf8.is_null() {
                false
            } else {
                let style = std::ffi::CStr::from_ptr(utf8).to_string_lossy();
                style == "Dark"
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn is_dark_mode() -> bool {
    true
}

fn load_icon_from_png(data: &[u8]) -> Result<tray_icon::Icon> {
    let decoder = png::Decoder::new(std::io::Cursor::new(data));
    let mut reader = decoder.read_info()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf)?;
    buf.truncate(info.buffer_size());

    let rgba_data = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => {
            let mut rgba = Vec::with_capacity(buf.len() * 4 / 3);
            for chunk in buf.chunks(3) {
                rgba.extend_from_slice(chunk);
                rgba.push(255);
            }
            rgba
        }
        png::ColorType::GrayscaleAlpha => {
            let mut rgba = Vec::with_capacity(buf.len() * 2);
            for chunk in buf.chunks(2) {
                rgba.extend_from_slice(&[chunk[0], chunk[0], chunk[0], chunk[1]]);
            }
            rgba
        }
        png::ColorType::Grayscale => {
            let mut rgba = Vec::with_capacity(buf.len() * 4);
            for &gray in &buf {
                rgba.extend_from_slice(&[gray, gray, gray, 255]);
            }
            rgba
        }
        png::ColorType::Indexed => {
            anyhow::bail!("Indexed color not supported");
        }
    };

    tray_icon::Icon::from_rgba(rgba_data, info.width, info.height)
        .map_err(|e| anyhow::anyhow!("Failed to create icon: {}", e))
}
