//! Bookmark and recent directory management
//!
//! Tracks user-starred directories (bookmarks) and recently used directories.

use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::PathBuf;

/// Maximum number of recent entries to keep
const MAX_RECENT_ENTRIES: usize = 20;

/// A bookmarked directory
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Bookmark {
    /// Path to the bookmarked directory
    pub path: PathBuf,
    /// Optional custom display name
    pub name: Option<String>,
}

impl Bookmark {
    /// Create a new bookmark
    pub fn new(path: PathBuf) -> Self {
        Self { path, name: None }
    }

    /// Create a new bookmark with a custom name
    pub fn with_name(path: PathBuf, name: String) -> Self {
        Self {
            path,
            name: Some(name),
        }
    }

    /// Get the display name (custom name or directory name)
    pub fn display_name(&self) -> String {
        if let Some(ref name) = self.name {
            name.clone()
        } else {
            self.path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| self.path.display().to_string())
        }
    }
}

/// A recently used directory
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecentEntry {
    /// Path to the directory
    pub path: PathBuf,
    /// Unix timestamp of last access
    pub last_accessed: u64,
}

impl RecentEntry {
    /// Create a new recent entry with current timestamp
    pub fn new(path: PathBuf) -> Self {
        let last_accessed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        Self {
            path,
            last_accessed,
        }
    }

    /// Get the display name (directory name)
    pub fn display_name(&self) -> String {
        self.path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| self.path.display().to_string())
    }
}

/// Manages bookmarks and recent directories
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BookmarkManager {
    /// User-starred directories
    pub bookmarks: Vec<Bookmark>,
    /// Recently used directories (most recent first)
    pub recent: VecDeque<RecentEntry>,
}

impl BookmarkManager {
    /// Create a new empty BookmarkManager
    pub fn new() -> Self {
        Self {
            bookmarks: Vec::new(),
            recent: VecDeque::new(),
        }
    }

    /// Load bookmarks and recent from file
    pub fn load() -> Result<Self> {
        let path = Self::data_path()?;
        if path.exists() {
            let content = std::fs::read_to_string(&path)
                .with_context(|| format!("Failed to read bookmarks file: {:?}", path))?;
            let manager: BookmarkManager = toml::from_str(&content)
                .with_context(|| format!("Failed to parse bookmarks file: {:?}", path))?;
            Ok(manager)
        } else {
            Ok(Self::new())
        }
    }

    /// Save bookmarks and recent to file
    pub fn save(&self) -> Result<()> {
        let path = Self::data_path()?;

        // Create parent directories if needed
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create bookmarks directory: {:?}", parent))?;
        }

        let content = toml::to_string_pretty(self).context("Failed to serialize bookmarks")?;
        std::fs::write(&path, content)
            .with_context(|| format!("Failed to write bookmarks file: {:?}", path))?;

        Ok(())
    }

    /// Get the data file path
    fn data_path() -> Result<PathBuf> {
        let proj_dirs = ProjectDirs::from("com", "coredeck", "CoreDeck")
            .context("Failed to determine data directory")?;
        Ok(proj_dirs.data_dir().join("bookmarks.toml"))
    }

    // === Bookmark operations ===

    /// Add a bookmark
    pub fn add_bookmark(&mut self, path: PathBuf) {
        // Don't add duplicates
        if !self.bookmarks.iter().any(|b| b.path == path) {
            self.bookmarks.push(Bookmark::new(path));
        }
    }

    /// Add a bookmark with a custom name
    pub fn add_bookmark_with_name(&mut self, path: PathBuf, name: String) {
        // Remove existing bookmark with same path
        self.bookmarks.retain(|b| b.path != path);
        self.bookmarks.push(Bookmark::with_name(path, name));
    }

    /// Remove a bookmark by path
    pub fn remove_bookmark(&mut self, path: &PathBuf) -> bool {
        let len_before = self.bookmarks.len();
        self.bookmarks.retain(|b| &b.path != path);
        self.bookmarks.len() < len_before
    }

    /// Check if a path is bookmarked
    pub fn is_bookmarked(&self, path: &PathBuf) -> bool {
        self.bookmarks.iter().any(|b| &b.path == path)
    }

    /// Get all bookmarks
    pub fn get_bookmarks(&self) -> &[Bookmark] {
        &self.bookmarks
    }

    // === Recent operations ===

    /// Add or update a recent entry
    pub fn add_recent(&mut self, path: PathBuf) {
        // Remove existing entry with same path
        self.recent.retain(|r| r.path != path);

        // Add to front
        self.recent.push_front(RecentEntry::new(path));

        // Trim to max size
        while self.recent.len() > MAX_RECENT_ENTRIES {
            self.recent.pop_back();
        }
    }

    /// Remove a recent entry by path
    pub fn remove_recent(&mut self, path: &PathBuf) -> bool {
        let len_before = self.recent.len();
        self.recent.retain(|r| &r.path != path);
        self.recent.len() < len_before
    }

    /// Clear all recent entries
    pub fn clear_recent(&mut self) {
        self.recent.clear();
    }

    /// Get all recent entries (most recent first)
    pub fn get_recent(&self) -> &VecDeque<RecentEntry> {
        &self.recent
    }

    /// Get recent entries as a slice (for easier iteration)
    pub fn recent_as_slice(&self) -> Vec<&RecentEntry> {
        self.recent.iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bookmark_display_name() {
        let bookmark = Bookmark::new(PathBuf::from("/home/user/project"));
        assert_eq!(bookmark.display_name(), "project");

        let bookmark = Bookmark::with_name(PathBuf::from("/home/user/project"), "My Project".into());
        assert_eq!(bookmark.display_name(), "My Project");
    }

    #[test]
    fn test_add_bookmark() {
        let mut manager = BookmarkManager::new();
        manager.add_bookmark(PathBuf::from("/project1"));
        manager.add_bookmark(PathBuf::from("/project2"));
        manager.add_bookmark(PathBuf::from("/project1")); // Duplicate

        assert_eq!(manager.bookmarks.len(), 2);
    }

    #[test]
    fn test_remove_bookmark() {
        let mut manager = BookmarkManager::new();
        manager.add_bookmark(PathBuf::from("/project1"));
        manager.add_bookmark(PathBuf::from("/project2"));

        assert!(manager.remove_bookmark(&PathBuf::from("/project1")));
        assert!(!manager.remove_bookmark(&PathBuf::from("/nonexistent")));
        assert_eq!(manager.bookmarks.len(), 1);
    }

    #[test]
    fn test_add_recent() {
        let mut manager = BookmarkManager::new();
        manager.add_recent(PathBuf::from("/project1"));
        manager.add_recent(PathBuf::from("/project2"));
        manager.add_recent(PathBuf::from("/project1")); // Move to front

        assert_eq!(manager.recent.len(), 2);
        assert_eq!(manager.recent[0].path, PathBuf::from("/project1"));
    }

    #[test]
    fn test_recent_max_size() {
        let mut manager = BookmarkManager::new();
        for i in 0..30 {
            manager.add_recent(PathBuf::from(format!("/project{}", i)));
        }

        assert_eq!(manager.recent.len(), MAX_RECENT_ENTRIES);
    }

    #[test]
    fn test_is_bookmarked() {
        let mut manager = BookmarkManager::new();
        manager.add_bookmark(PathBuf::from("/project1"));

        assert!(manager.is_bookmarked(&PathBuf::from("/project1")));
        assert!(!manager.is_bookmarked(&PathBuf::from("/project2")));
    }
}
