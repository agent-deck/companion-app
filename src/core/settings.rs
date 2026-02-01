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

/// Default window width
pub const DEFAULT_WINDOW_WIDTH: f64 = 1000.0;

/// Default window height
pub const DEFAULT_WINDOW_HEIGHT: f64 = 700.0;

/// Window geometry (size and position)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowGeometry {
    /// Window width in logical pixels
    pub width: f64,
    /// Window height in logical pixels
    pub height: f64,
    /// Window X position (optional, None means let OS decide)
    pub x: Option<i32>,
    /// Window Y position (optional, None means let OS decide)
    pub y: Option<i32>,
}

impl Default for WindowGeometry {
    fn default() -> Self {
        Self {
            width: DEFAULT_WINDOW_WIDTH,
            height: DEFAULT_WINDOW_HEIGHT,
            x: None,
            y: None,
        }
    }
}

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

    /// Get the bell indicator tab background color (visual bell)
    pub fn bell_tab_background(&self) -> egui::Color32 {
        match self {
            ColorScheme::Dark => egui::Color32::from_rgb(100, 60, 40), // Orange-brown tint
            ColorScheme::Light => egui::Color32::from_rgb(255, 220, 180), // Light orange tint
        }
    }

    /// Get the accent color (for links, session counts, etc.)
    pub fn accent_color(&self) -> egui::Color32 {
        match self {
            ColorScheme::Dark => egui::Color32::from_rgb(100, 149, 237), // Cornflower blue
            ColorScheme::Light => egui::Color32::from_rgb(65, 105, 225), // Royal blue
        }
    }

    /// Get the popup/context menu background color
    pub fn popup_background(&self) -> egui::Color32 {
        match self {
            ColorScheme::Dark => egui::Color32::from_rgb(45, 45, 45),
            ColorScheme::Light => egui::Color32::from_rgb(255, 255, 255),
        }
    }

    /// Get the popup/context menu border color
    pub fn popup_border(&self) -> egui::Color32 {
        match self {
            ColorScheme::Dark => egui::Color32::from_rgb(70, 70, 70),
            ColorScheme::Light => egui::Color32::from_rgb(200, 200, 200),
        }
    }

    /// Get the disabled/grayed out text color
    pub fn disabled_foreground(&self) -> egui::Color32 {
        match self {
            ColorScheme::Dark => egui::Color32::from_rgb(100, 100, 100),
            ColorScheme::Light => egui::Color32::from_rgb(160, 160, 160),
        }
    }

    /// Get the secondary/dimmed text color
    pub fn secondary_foreground(&self) -> egui::Color32 {
        match self {
            ColorScheme::Dark => egui::Color32::from_rgb(150, 150, 150),
            ColorScheme::Light => egui::Color32::from_rgb(120, 120, 120),
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

    /// Window geometry (size and position)
    #[serde(default)]
    pub window_geometry: WindowGeometry,
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
            window_geometry: WindowGeometry::default(),
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
        assert_eq!(settings.window_geometry.width, DEFAULT_WINDOW_WIDTH);
        assert_eq!(settings.window_geometry.height, DEFAULT_WINDOW_HEIGHT);
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
            window_geometry: WindowGeometry {
                width: 1200.0,
                height: 800.0,
                x: Some(100),
                y: Some(50),
            },
        };

        let toml_str = toml::to_string(&settings).unwrap();
        let parsed: Settings = toml::from_str(&toml_str).unwrap();

        assert_eq!(parsed.font_family, "Menlo");
        assert_eq!(parsed.font_size, 14.0);
        assert_eq!(parsed.color_scheme, ColorScheme::Light);
        assert_eq!(parsed.window_geometry.width, 1200.0);
        assert_eq!(parsed.window_geometry.height, 800.0);
        assert_eq!(parsed.window_geometry.x, Some(100));
        assert_eq!(parsed.window_geometry.y, Some(50));
    }

    #[test]
    fn test_window_geometry_default() {
        let geometry = WindowGeometry::default();
        assert_eq!(geometry.width, DEFAULT_WINDOW_WIDTH);
        assert_eq!(geometry.height, DEFAULT_WINDOW_HEIGHT);
        assert_eq!(geometry.x, None);
        assert_eq!(geometry.y, None);
    }

    #[test]
    fn test_settings_backward_compatible() {
        // Test that settings without window_geometry can still be parsed
        let old_toml = r#"
font_family = "Monaco"
font_size = 16.0
color_scheme = "Dark"
"#;
        let parsed: Settings = toml::from_str(old_toml).unwrap();
        assert_eq!(parsed.font_family, "Monaco");
        assert_eq!(parsed.window_geometry, WindowGeometry::default());
    }
}
