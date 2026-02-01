//! Tray module - System tray icon and menu

mod icon;
mod menu;

pub use icon::TrayIcon;
pub use menu::{TrayAction, TrayManager};
