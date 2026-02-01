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
pub mod pty;
pub mod terminal;
pub mod tray;
pub mod window;

pub use core::bookmarks::BookmarkManager;
pub use core::config::Config;
pub use core::events::AppEvent;
pub use core::sessions::{SessionId, SessionInfo, SessionManager};
pub use core::settings::{ColorScheme, Settings};
pub use core::state::{AppState, ClaudeState};
