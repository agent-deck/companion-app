//! Terminal module - Terminal emulation for Claude Code
//!
//! This module provides:
//! - `AgentDeckTermConfig`: Configuration for wezterm-based terminal emulation
//! - `Session`: Terminal session wrapping wezterm Terminal and PTY

mod config;
mod notifications;
mod session;

pub use config::AgentDeckTermConfig;
pub use notifications::NotificationHandler;
pub use session::Session;
