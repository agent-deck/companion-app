//! Terminal window module
//!
//! Provides a GUI window for displaying Claude CLI output and capturing input.

mod context_menu;
mod glyph_cache;
mod input;
pub mod new_tab;
mod render;
pub mod settings_modal;
mod terminal;
mod terminal_input;
mod terminal_notifications;
mod terminal_pty;
mod terminal_selection;

pub use context_menu::{render_context_menu, ContextMenuState};
pub use glyph_cache::{GlyphCache, StyleKey, BASE_DPI};
pub use input::{build_arrow_seq, build_f1_f4_seq, build_home_end_seq, build_tilde_seq, encode_modifiers, open_url};
pub use new_tab::{render_new_tab_page, NewTabAction};
pub use render::{
    color_attr_to_egui, handle_settings_modal_result, render_hyperlink_tooltip,
    render_tab_bar, render_terminal_content, RenderParams, SessionRenderData,
    CLAUDE_ICON_SVG, CLAUDE_ORANGE, DECK_CONNECTED_SVG, DECK_DISCONNECTED_SVG,
    MAX_TAB_TITLE_LEN, TAB_BAR_HEIGHT,
};
pub use settings_modal::{render_settings_modal, SettingsModal, SettingsModalResult, SettingsTab};
pub use terminal::{InputSender, TerminalAction, TerminalWindowState};
