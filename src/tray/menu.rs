//! Tray menu management

use crate::core::events::{AppEvent, EventSender};
use super::icon::TrayIcon;
use anyhow::{Context, Result};
use tray_icon::{
    menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem},
    TrayIcon as TrayIconHandle, TrayIconBuilder,
};
use tracing::{debug, error, info};

/// Tray menu actions
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayAction {
    /// Toggle window visibility (show/hide)
    ToggleWindow,
    /// Open settings
    OpenSettings,
    /// Quit application (really quit)
    Quit,
}

/// Tray manager
pub struct TrayManager {
    /// Tray icon handle
    tray: TrayIconHandle,
    /// Icons for connected/disconnected states
    icons: TrayIcon,
    /// Event sender (wakes event loop)
    event_tx: EventSender,
    /// Toggle window menu item (for dynamic text updates)
    toggle_item: MenuItem,
    /// Toggle menu item ID
    toggle_id: MenuId,
    /// Settings menu item ID
    settings_id: MenuId,
    /// Quit menu item ID
    quit_id: MenuId,
}

impl TrayManager {
    /// Create a new tray manager
    pub fn new(event_tx: EventSender) -> Result<Self> {
        // Load icons
        let icons = TrayIcon::new().context("Failed to load tray icons")?;

        // Create menu
        let menu = Menu::new();

        let toggle_item = MenuItem::new("Show Agent Deck", true, None);
        let toggle_id = toggle_item.id().clone();

        let settings_item = MenuItem::new("Settings...", true, None);
        let settings_id = settings_item.id().clone();

        let quit_item = MenuItem::new("Quit", true, None);
        let quit_id = quit_item.id().clone();

        menu.append(&toggle_item)?;
        menu.append(&PredefinedMenuItem::separator())?;
        menu.append(&settings_item)?;
        menu.append(&PredefinedMenuItem::separator())?;
        menu.append(&quit_item)?;

        // Create tray icon
        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("Agent Deck - Disconnected")
            .with_icon(icons.disconnected().clone())
            .build()
            .context("Failed to create tray icon")?;

        info!("Tray icon created");

        let manager = Self {
            tray,
            icons,
            event_tx,
            toggle_item,
            toggle_id,
            settings_id,
            quit_id,
        };

        // Start menu event handler
        manager.start_menu_handler();

        Ok(manager)
    }

    /// Start menu event handler
    fn start_menu_handler(&self) {
        let event_tx = self.event_tx.clone();
        let toggle_id = self.toggle_id.clone();
        let settings_id = self.settings_id.clone();
        let quit_id = self.quit_id.clone();

        std::thread::spawn(move || {
            let receiver = MenuEvent::receiver();

            loop {
                if let Ok(event) = receiver.recv() {
                    debug!("Menu event: {:?}", event);

                    let action = if event.id == toggle_id {
                        Some(TrayAction::ToggleWindow)
                    } else if event.id == settings_id {
                        Some(TrayAction::OpenSettings)
                    } else if event.id == quit_id {
                        Some(TrayAction::Quit)
                    } else {
                        None
                    };

                    if let Some(action) = action {
                        if let Err(e) = event_tx.send(AppEvent::TrayAction(action)) {
                            error!("Failed to send tray action: {}", e);
                        }
                    }
                }
            }
        });
    }

    /// Update the toggle menu item text based on window visibility
    pub fn set_window_visible(&mut self, visible: bool) {
        let text = if visible { "Hide Agent Deck" } else { "Show Agent Deck" };
        self.toggle_item.set_text(text);
    }

    /// Set connected/disconnected state
    pub fn set_connected(&mut self, connected: bool) {
        let icon = if connected {
            self.icons.connected()
        } else {
            self.icons.disconnected()
        };

        let tooltip = if connected {
            "Agent Deck - Connected"
        } else {
            "Agent Deck - Disconnected"
        };

        if let Err(e) = self.tray.set_icon(Some(icon.clone())) {
            error!("Failed to set tray icon: {}", e);
        }

        if let Err(e) = self.tray.set_tooltip(Some(tooltip)) {
            error!("Failed to set tray tooltip: {}", e);
        }
    }

    /// Set tooltip with status information
    pub fn set_status(&mut self, status: &str) {
        if let Err(e) = self.tray.set_tooltip(Some(status)) {
            error!("Failed to set tray tooltip: {}", e);
        }
    }
}
