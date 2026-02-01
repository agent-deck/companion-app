//! Application settings management
//!
//! Manages user preferences like font, colors, and other UI settings.

use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Default font family
pub const DEFAULT_FONT_FAMILY: &str = "JetBrainsMono";

/// Default font size
pub const DEFAULT_FONT_SIZE: f32 = 17.0;

/// Minimum font size
pub const MIN_FONT_SIZE: f32 = 10.0;

/// Maximum font size
pub const MAX_FONT_SIZE: f32 = 24.0;

/// Color scheme options
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ColorScheme {
    /// Dark theme (default)
    #[default]
    Dark,
    /// Light theme
    Light,
}

impl ColorScheme {
    /// Get the display name for this color scheme
    pub fn display_name(&self) -> &'static str {
        match self {
            ColorScheme::Dark => "Dark",
            ColorScheme::Light => "Light",
        }
    }

    /// Get all available color schemes
    pub fn all() -> &'static [ColorScheme] {
        &[ColorScheme::Dark, ColorScheme::Light]
    }

    /// Get the background color for this scheme
    pub fn background(&self) -> egui::Color32 {
        match self {
            ColorScheme::Dark => egui::Color32::from_rgb(30, 30, 30),
            ColorScheme::Light => egui::Color32::from_rgb(250, 250, 250),
        }
    }

    /// Get the foreground color for this scheme
    pub fn foreground(&self) -> egui::Color32 {
        match self {
            ColorScheme::Dark => egui::Color32::from_rgb(220, 220, 220),
            ColorScheme::Light => egui::Color32::from_rgb(30, 30, 30),
        }
    }

    /// Get the selection background color
    pub fn selection_background(&self) -> egui::Color32 {
        match self {
            ColorScheme::Dark => egui::Color32::from_rgb(70, 130, 180),
            ColorScheme::Light => egui::Color32::from_rgb(173, 214, 255),
        }
    }

    /// Get the tab bar background color
    pub fn tab_bar_background(&self) -> egui::Color32 {
        match self {
            ColorScheme::Dark => egui::Color32::from_rgb(40, 40, 40),
            ColorScheme::Light => egui::Color32::from_rgb(235, 235, 235),
        }
    }

    /// Get the active tab background color
    pub fn active_tab_background(&self) -> egui::Color32 {
        match self {
            ColorScheme::Dark => egui::Color32::from_rgb(50, 50, 50),
            ColorScheme::Light => egui::Color32::from_rgb(255, 255, 255),
        }
    }

    /// Get the inactive tab background color
    pub fn inactive_tab_background(&self) -> egui::Color32 {
        match self {
            ColorScheme::Dark => egui::Color32::from_rgb(35, 35, 35),
            ColorScheme::Light => egui::Color32::from_rgb(220, 220, 220),
        }
    }
}

/// Application settings
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    /// Font family name
    #[serde(default = "default_font_family")]
    pub font_family: String,

    /// Font size in points
    #[serde(default = "default_font_size")]
    pub font_size: f32,

    /// Color scheme
    #[serde(default)]
    pub color_scheme: ColorScheme,
}

fn default_font_family() -> String {
    DEFAULT_FONT_FAMILY.to_string()
}

fn default_font_size() -> f32 {
    DEFAULT_FONT_SIZE
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            font_family: default_font_family(),
            font_size: default_font_size(),
            color_scheme: ColorScheme::default(),
        }
    }
}

impl Settings {
    /// Create new settings with defaults
    pub fn new() -> Self {
        Self::default()
    }

    /// Load settings from file
    pub fn load() -> Result<Self> {
        let path = Self::settings_path()?;
        if path.exists() {
            let content = std::fs::read_to_string(&path)
                .with_context(|| format!("Failed to read settings file: {:?}", path))?;
            let settings: Settings = toml::from_str(&content)
                .with_context(|| format!("Failed to parse settings file: {:?}", path))?;
            Ok(settings)
        } else {
            Ok(Self::default())
        }
    }

    /// Save settings to file
    pub fn save(&self) -> Result<()> {
        let path = Self::settings_path()?;

        // Create parent directories if needed
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create settings directory: {:?}", parent))?;
        }

        let content = toml::to_string_pretty(self).context("Failed to serialize settings")?;
        std::fs::write(&path, content)
            .with_context(|| format!("Failed to write settings file: {:?}", path))?;

        Ok(())
    }

    /// Get the settings file path
    fn settings_path() -> Result<PathBuf> {
        let proj_dirs = ProjectDirs::from("com", "agentdeck", "AgentDeck")
            .context("Failed to determine settings directory")?;
        Ok(proj_dirs.config_dir().join("settings.toml"))
    }

    /// Set font size with clamping to valid range
    pub fn set_font_size(&mut self, size: f32) {
        self.font_size = size.clamp(MIN_FONT_SIZE, MAX_FONT_SIZE);
    }

    /// Get available font families (preset list)
    pub fn available_fonts() -> &'static [&'static str] {
        &[
            "JetBrainsMono",
            "Menlo",
            "Monaco",
            "Consolas",
            "SF Mono",
            "Fira Code",
            "Source Code Pro",
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_settings() {
        let settings = Settings::default();
        assert_eq!(settings.font_family, DEFAULT_FONT_FAMILY);
        assert_eq!(settings.font_size, DEFAULT_FONT_SIZE);
        assert_eq!(settings.color_scheme, ColorScheme::Dark);
    }

    #[test]
    fn test_font_size_clamping() {
        let mut settings = Settings::default();

        settings.set_font_size(5.0);
        assert_eq!(settings.font_size, MIN_FONT_SIZE);

        settings.set_font_size(50.0);
        assert_eq!(settings.font_size, MAX_FONT_SIZE);

        settings.set_font_size(15.0);
        assert_eq!(settings.font_size, 15.0);
    }

    #[test]
    fn test_color_scheme_colors() {
        let dark = ColorScheme::Dark;
        let light = ColorScheme::Light;

        // Dark should have darker background
        assert!(dark.background().r() < light.background().r());

        // Foreground should be opposite
        assert!(dark.foreground().r() > light.foreground().r());
    }

    #[test]
    fn test_serialization() {
        let settings = Settings {
            font_family: "Menlo".to_string(),
            font_size: 14.0,
            color_scheme: ColorScheme::Light,
        };

        let toml_str = toml::to_string(&settings).unwrap();
        let parsed: Settings = toml::from_str(&toml_str).unwrap();

        assert_eq!(parsed.font_family, "Menlo");
        assert_eq!(parsed.font_size, 14.0);
        assert_eq!(parsed.color_scheme, ColorScheme::Light);
    }
}
