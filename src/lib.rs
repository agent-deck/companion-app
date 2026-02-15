//! Agent Deck Companion App
//!
//! A Rust application that connects Claude Code CLI to the Agent Deck macropad display.
//!
//! # Features
//! - Wraps Claude Code CLI in a PTY to monitor its state
//! - Sends status updates to the macropad display via Raw HID
//! - Watches for F20 (Claude key) to launch/focus terminal
//! - Runs as a system tray application
//! - Allows customization of soft keys (F15-F18)
//! - Supports multiple tabs with browser-style tab bar
//! - Bookmark and recent directories management
//! - Settings modal for font and color scheme

pub mod core;
pub mod hid;
pub mod hotkey;
#[cfg(target_os = "macos")]
pub mod macos;
pub mod pty;
pub mod terminal;
pub mod tray;
pub mod window;

pub use core::bookmarks::BookmarkManager;
pub use core::config::Config;
pub use core::events::AppEvent;
pub use core::sessions::{ClaudeActivity, SessionId, SessionInfo, SessionManager};
pub use core::settings::{ColorScheme, Settings};
pub use core::state::AppState;

// Global counter for working Claude sessions (used by macOS quit handler)
#[cfg(target_os = "macos")]
static WORKING_SESSION_COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Update the global working session count (called when claude_activity changes)
#[cfg(target_os = "macos")]
pub fn update_working_session_count(count: usize) {
    WORKING_SESSION_COUNT.store(count, std::sync::atomic::Ordering::SeqCst);
}

/// Get the current working session count
#[cfg(target_os = "macos")]
pub fn get_working_session_count() -> usize {
    WORKING_SESSION_COUNT.load(std::sync::atomic::Ordering::SeqCst)
}

#[cfg(not(target_os = "macos"))]
pub fn update_working_session_count(_count: usize) {
    // No-op on other platforms
}

#[cfg(not(target_os = "macos"))]
pub fn get_working_session_count() -> usize {
    0
}
