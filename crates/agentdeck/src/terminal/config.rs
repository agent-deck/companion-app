//! Terminal configuration for WezTerm's Terminal
//!
//! Implements the TerminalConfiguration trait required by wezterm-term.

use wezterm_term::color::ColorPalette;
use wezterm_term::config::TerminalConfiguration;

/// Configuration for the AgentDeck terminal emulator.
#[derive(Debug, Clone)]
pub struct AgentDeckTermConfig {
    /// Number of lines to keep in scrollback buffer
    pub scrollback_size: usize,
    /// Color palette for this terminal
    palette: ColorPalette,
}

impl AgentDeckTermConfig {
    /// Create a new config with the given color palette
    pub fn new(palette: ColorPalette) -> Self {
        Self {
            scrollback_size: 10_000,
            palette,
        }
    }
}

impl Default for AgentDeckTermConfig {
    fn default() -> Self {
        Self {
            scrollback_size: 10_000,
            palette: ColorPalette::default(),
        }
    }
}

impl TerminalConfiguration for AgentDeckTermConfig {
    fn scrollback_size(&self) -> usize {
        self.scrollback_size
    }

    fn color_palette(&self) -> ColorPalette {
        self.palette.clone()
    }
}
