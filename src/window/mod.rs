//! Terminal window module
//!
//! Provides a GUI window for displaying Claude CLI output and capturing input.

mod glyph_cache;
pub mod new_tab;
pub mod settings_modal;
mod terminal;

pub use glyph_cache::{GlyphCache, StyleKey};
pub use new_tab::{render_new_tab_page, NewTabAction};
pub use settings_modal::{render_settings_modal, SettingsModal, SettingsModalResult};
pub use terminal::{InputSender, TerminalAction, TerminalWindowState};
