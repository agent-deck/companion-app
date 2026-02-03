//! macOS-specific functionality
//!
//! This module contains macOS-specific code for native integrations:
//! - Native menu bar using NSMenu/NSMenuItem
//! - Native context menus

pub mod menu;

pub use menu::{
    create_menu_bar, init_menu_sender, update_edit_menu_state, update_recent_sessions_menu,
    MenuAction, ContextMenuAction, ContextMenuSession, show_context_menu,
};
