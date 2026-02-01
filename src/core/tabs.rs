//! Tab state persistence
//!
//! Saves and loads open tabs between app sessions.
//! Tabs are restored with their working directories but PTY is only
//! started when the tab becomes active (lazy loading).

use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A persisted tab entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabEntry {
    /// Working directory for this tab
    pub working_directory: PathBuf,
    /// Tab title
    pub title: String,
}

/// Persisted tab state
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TabState {
    /// Open tabs
    pub tabs: Vec<TabEntry>,
    /// Index of the active tab
    pub active_tab: usize,
}

impl TabState {
    /// Load tab state from file
    pub fn load() -> Result<Self> {
        let state_path = Self::state_path()?;

        if state_path.exists() {
            let content = std::fs::read_to_string(&state_path)
                .with_context(|| format!("Failed to read tab state file: {:?}", state_path))?;
            let state: TabState = serde_json::from_str(&content)
                .with_context(|| format!("Failed to parse tab state file: {:?}", state_path))?;
            Ok(state)
        } else {
            Ok(TabState::default())
        }
    }

    /// Save tab state to file
    pub fn save(&self) -> Result<()> {
        let state_path = Self::state_path()?;

        // Create parent directories if needed
        if let Some(parent) = state_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create state directory: {:?}", parent))?;
        }

        let content = serde_json::to_string_pretty(self).context("Failed to serialize tab state")?;
        std::fs::write(&state_path, content)
            .with_context(|| format!("Failed to write tab state file: {:?}", state_path))?;

        Ok(())
    }

    /// Get the tab state file path
    pub fn state_path() -> Result<PathBuf> {
        let proj_dirs = ProjectDirs::from("com", "agentdeck", "AgentDeck")
            .context("Failed to determine state directory")?;
        Ok(proj_dirs.data_dir().join("tabs.json"))
    }

    /// Check if there are any saved tabs
    pub fn has_tabs(&self) -> bool {
        !self.tabs.is_empty()
    }
}
