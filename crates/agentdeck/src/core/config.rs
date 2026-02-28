//! Configuration management

use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// HID device configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HidConfig {
    /// USB Vendor ID
    #[serde(default = "default_vendor_id")]
    pub vendor_id: u16,
    /// USB Product ID
    #[serde(default = "default_product_id")]
    pub product_id: u16,
    /// HID Usage Page
    #[serde(default = "default_usage_page")]
    pub usage_page: u16,
    /// HID Usage ID
    #[serde(default = "default_usage_id")]
    pub usage_id: u16,
    /// Keep-alive ping interval in milliseconds
    #[serde(default = "default_ping_interval")]
    pub ping_interval_ms: u64,
    /// Reconnect attempt interval in milliseconds
    #[serde(default = "default_reconnect_interval")]
    pub reconnect_interval_ms: u64,
}

fn default_vendor_id() -> u16 {
    0xFEED
}
fn default_product_id() -> u16 {
    0x0803
}
fn default_usage_page() -> u16 {
    0xFF60
}
fn default_usage_id() -> u16 {
    0x61
}
fn default_ping_interval() -> u64 {
    2000
}
fn default_reconnect_interval() -> u64 {
    1000
}

impl Default for HidConfig {
    fn default() -> Self {
        Self {
            vendor_id: default_vendor_id(),
            product_id: default_product_id(),
            usage_page: default_usage_page(),
            usage_id: default_usage_id(),
            ping_interval_ms: default_ping_interval(),
            reconnect_interval_ms: default_reconnect_interval(),
        }
    }
}

/// Display configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisplayConfig {
    /// Default brightness (0-255)
    #[serde(default = "default_brightness")]
    pub default_brightness: u8,
}

fn default_brightness() -> u8 {
    200
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self {
            default_brightness: default_brightness(),
        }
    }
}

/// Terminal configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalConfig {
    /// Preferred terminal emulator
    #[serde(default = "default_terminal")]
    pub preferred: String,
    /// Working directory for new terminals
    #[serde(default)]
    pub working_directory: String,
    /// Font size in points
    #[serde(default = "default_font_size")]
    pub font_size: f32,
}

fn default_terminal() -> String {
    "auto".to_string()
}

fn default_font_size() -> f32 {
    17.0
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            preferred: default_terminal(),
            working_directory: String::new(),
            font_size: default_font_size(),
        }
    }
}

/// Claude CLI configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClaudeConfig {
    /// Path to claude CLI
    #[serde(default)]
    pub cli_path: String,
    /// Default arguments for claude CLI
    #[serde(default)]
    pub default_args: Vec<String>,
}


/// Main application configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    /// HID device configuration
    #[serde(default)]
    pub hid: HidConfig,
    /// Display configuration
    #[serde(default)]
    pub display: DisplayConfig,
    /// Terminal configuration
    #[serde(default)]
    pub terminal: TerminalConfig,
    /// Claude CLI configuration
    #[serde(default)]
    pub claude: ClaudeConfig,
}

impl Config {
    /// Load configuration from file
    pub fn load() -> Result<Self> {
        let config_path = Self::config_path()?;

        if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)
                .with_context(|| format!("Failed to read config file: {:?}", config_path))?;
            let config: Config = toml::from_str(&content)
                .with_context(|| format!("Failed to parse config file: {:?}", config_path))?;
            Ok(config)
        } else {
            // Return default config if file doesn't exist
            Ok(Config::default())
        }
    }

    /// Save configuration to file
    pub fn save(&self) -> Result<()> {
        let config_path = Self::config_path()?;

        // Create parent directories if needed
        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create config directory: {:?}", parent))?;
        }

        let content = toml::to_string_pretty(self).context("Failed to serialize config")?;
        std::fs::write(&config_path, content)
            .with_context(|| format!("Failed to write config file: {:?}", config_path))?;

        Ok(())
    }

    /// Get the configuration file path
    pub fn config_path() -> Result<PathBuf> {
        let proj_dirs = ProjectDirs::from("com", "agentdeck", "AgentDeck")
            .context("Failed to determine config directory")?;
        Ok(proj_dirs.config_dir().join("config.toml"))
    }

    /// Get the default configuration embedded in the binary
    pub fn default_config_str() -> &'static str {
        include_str!("../../config/default.toml")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.hid.vendor_id, 0xFEED);
        assert_eq!(config.hid.product_id, 0x0803);
        assert_eq!(config.hid.usage_page, 0xFF60);
        assert_eq!(config.hid.usage_id, 0x61);
        assert_eq!(config.display.default_brightness, 200);
    }

    #[test]
    fn test_config_serialization() {
        let config = Config::default();
        let toml_str = toml::to_string(&config).unwrap();
        let parsed: Config = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.hid.vendor_id, config.hid.vendor_id);
    }
}
