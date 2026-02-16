//! Session management for multiple Claude sessions
//!
//! Manages multiple terminal sessions, each with its own PTY, working directory,
//! and Claude state.

use crate::core::claude_sessions::get_sessions_for_directory;
use crate::terminal::Session;
use crate::window::InputSender;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use wezterm_term::color::ColorPalette;

/// Unique identifier for a session
pub type SessionId = usize;

/// Claude's activity state derived from terminal title prefix
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ClaudeActivity {
    /// Not running or unknown state
    #[default]
    Unknown,
    /// Claude is idle (✳ prefix)
    Idle,
    /// Claude is working/thinking (spinner prefix like ⠐, ⠂, etc.)
    Working,
}

impl ClaudeActivity {
    /// Parse activity state from terminal title
    pub fn from_title(title: &str) -> Self {
        let first_char = title.chars().next();
        match first_char {
            Some('✳') => ClaudeActivity::Idle,
            // Braille spinner characters (U+2800-U+28FF range)
            Some(c) if ('\u{2800}'..='\u{28FF}').contains(&c) => ClaudeActivity::Working,
            // Other potential working indicators
            Some('⠋') | Some('⠙') | Some('⠹') | Some('⠸') | Some('⠼') | Some('⠴') | Some('⠦') | Some('⠧') | Some('⠇') | Some('⠏') => ClaudeActivity::Working,
            _ => ClaudeActivity::Unknown,
        }
    }

    /// Check if Claude is currently working
    pub fn is_working(&self) -> bool {
        matches!(self, ClaudeActivity::Working)
    }
}

/// Information about a single Claude session
pub struct SessionInfo {
    /// Unique session ID
    pub id: SessionId,
    /// Working directory for this session
    pub working_directory: PathBuf,
    /// Display title (auto-derived from CWD or user-set)
    pub title: String,
    /// Terminal-set title (from OSC escape sequence, e.g., Claude Code's status)
    /// Used as suffix in tab title for disambiguation
    pub terminal_title: Option<String>,
    /// Terminal session (WezTerm-based)
    pub session: Arc<Mutex<Session>>,
    /// Input sender to PTY (None if session not started)
    pub pty_input_tx: Option<InputSender>,
    /// Whether this session has an active PTY
    pub is_running: bool,
    /// Whether PTY is currently being started (for loading indicator)
    pub is_loading: bool,
    /// Claude session ID to resume (None = fresh start, Some(id) = --resume {id})
    pub claude_session_id: Option<String>,
    /// Whether a bell occurred in this session (for visual bell indicator)
    pub bell_active: bool,
    /// Timestamp when fresh session was started (for session ID resolution)
    pub session_start_time: Option<std::time::Instant>,
    /// Whether we're waiting to resolve the session ID
    pub needs_session_resolution: bool,
    /// Claude's current activity state (derived from terminal title prefix)
    pub claude_activity: ClaudeActivity,
    /// Current task text when Claude is working (from OSC title with spinner prefix)
    pub current_task: Option<String>,
    /// Whether Claude finished working while tab was in background (for notification indicator)
    pub finished_in_background: bool,
    /// Whether an HID alert is currently active for this session
    pub hid_alert_active: bool,
    /// Monotonic order value for FIFO alert resolution (0 = no alert)
    pub alert_order: u64,
}

impl SessionInfo {
    /// Create a new session with the given working directory and color palette
    pub fn new(id: SessionId, working_directory: PathBuf, palette: &ColorPalette) -> Self {
        let title = working_directory
            .file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "New Session".to_string());

        Self {
            id,
            working_directory,
            title,
            terminal_title: None,
            session: Arc::new(Mutex::new(Session::new(0, 120, 50, palette.clone()))),
            pty_input_tx: None,
            is_running: false,
            is_loading: false,
            claude_session_id: None,
            bell_active: false,
            session_start_time: None,
            needs_session_resolution: false,
            claude_activity: ClaudeActivity::default(),
            current_task: None,
            finished_in_background: false,
            hid_alert_active: false,
            alert_order: 0,
        }
    }

    /// Create a new tab session (not yet started, shows new tab page)
    pub fn new_tab(id: SessionId, palette: &ColorPalette) -> Self {
        Self {
            id,
            working_directory: PathBuf::new(),
            title: "New Session".to_string(),
            terminal_title: None,
            session: Arc::new(Mutex::new(Session::new(0, 120, 50, palette.clone()))),
            pty_input_tx: None,
            is_running: false,
            is_loading: false,
            claude_session_id: None,
            bell_active: false,
            session_start_time: None,
            needs_session_resolution: false,
            claude_activity: ClaudeActivity::default(),
            current_task: None,
            finished_in_background: false,
            hid_alert_active: false,
            alert_order: 0,
        }
    }

    /// Check if this is a "new tab" (not yet started)
    pub fn is_new_tab(&self) -> bool {
        self.working_directory.as_os_str().is_empty() && !self.is_running
    }

    /// Set a custom title for this session
    pub fn set_title(&mut self, title: String) {
        self.title = title;
    }

    /// Get the session name for HID display.
    /// Prefers the terminal-set title (Claude session name); falls back to directory name.
    pub fn hid_session_name(&self) -> &str {
        self.terminal_title
            .as_deref()
            .filter(|t| !t.is_empty())
            .unwrap_or(&self.title)
    }

    /// Get the display title (truncated if necessary)
    pub fn display_title(&self, max_len: usize) -> String {
        if self.title.len() <= max_len {
            self.title.clone()
        } else {
            format!("{}...", &self.title[..max_len.saturating_sub(3)])
        }
    }
}

/// Manages multiple Claude sessions
pub struct SessionManager {
    /// All sessions
    sessions: Vec<SessionInfo>,
    /// Index of the currently active session
    active_session: usize,
    /// Next session ID to assign
    next_id: SessionId,
}

impl SessionManager {
    /// Create a new SessionManager with no sessions
    pub fn new() -> Self {
        Self {
            sessions: Vec::new(),
            active_session: 0,
            next_id: 0,
        }
    }

    /// Create a new session and return its ID
    pub fn create_session(&mut self, working_directory: PathBuf, palette: &ColorPalette) -> SessionId {
        let id = self.next_id;
        self.next_id += 1;
        self.sessions.push(SessionInfo::new(id, working_directory, palette));
        id
    }

    /// Create a new tab (empty session for new tab page)
    pub fn create_new_tab(&mut self, palette: &ColorPalette) -> SessionId {
        let id = self.next_id;
        self.next_id += 1;
        self.sessions.push(SessionInfo::new_tab(id, palette));
        id
    }

    /// Create a placeholder session (has working directory but PTY not started)
    /// Used for restoring tabs from saved state with lazy loading
    pub fn create_placeholder(
        &mut self,
        working_directory: PathBuf,
        title: String,
        claude_session_id: Option<String>,
        terminal_title: Option<String>,
        palette: &ColorPalette,
    ) -> SessionId {
        let id = self.next_id;
        self.next_id += 1;
        let mut session = SessionInfo::new(id, working_directory, palette);
        session.title = title;
        session.claude_session_id = claude_session_id;
        session.terminal_title = terminal_title;
        // is_running remains false - PTY will be started on demand
        self.sessions.push(session);
        id
    }

    /// Get the number of sessions
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// Get a reference to a session by ID
    pub fn get_session(&self, id: SessionId) -> Option<&SessionInfo> {
        self.sessions.iter().find(|s| s.id == id)
    }

    /// Get a mutable reference to a session by ID
    pub fn get_session_mut(&mut self, id: SessionId) -> Option<&mut SessionInfo> {
        self.sessions.iter_mut().find(|s| s.id == id)
    }

    /// Get the active session
    pub fn active_session(&self) -> Option<&SessionInfo> {
        self.sessions.get(self.active_session)
    }

    /// Get a mutable reference to the active session
    pub fn active_session_mut(&mut self) -> Option<&mut SessionInfo> {
        self.sessions.get_mut(self.active_session)
    }

    /// Get the active session ID
    pub fn active_session_id(&self) -> Option<SessionId> {
        self.sessions.get(self.active_session).map(|s| s.id)
    }

    /// Set the active session by ID
    pub fn set_active_session(&mut self, id: SessionId) -> bool {
        if let Some(idx) = self.sessions.iter().position(|s| s.id == id) {
            self.active_session = idx;
            true
        } else {
            false
        }
    }

    /// Set the active session by index
    pub fn set_active_session_index(&mut self, index: usize) -> bool {
        if index < self.sessions.len() {
            self.active_session = index;
            true
        } else {
            false
        }
    }

    /// Get the active session index
    pub fn active_session_index(&self) -> usize {
        self.active_session
    }

    /// Close a session by ID, returns true if the session was found and closed
    pub fn close_session(&mut self, id: SessionId) -> bool {
        if let Some(idx) = self.sessions.iter().position(|s| s.id == id) {
            self.sessions.remove(idx);

            // Adjust active session index if needed
            if self.sessions.is_empty() {
                self.active_session = 0;
            } else if self.active_session >= self.sessions.len() {
                self.active_session = self.sessions.len() - 1;
            } else if idx < self.active_session {
                self.active_session = self.active_session.saturating_sub(1);
            }

            true
        } else {
            false
        }
    }

    /// Iterate over all sessions
    pub fn iter(&self) -> impl Iterator<Item = &SessionInfo> {
        self.sessions.iter()
    }

    /// Iterate mutably over all sessions
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut SessionInfo> {
        self.sessions.iter_mut()
    }

    /// Check if there are any sessions
    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    /// Check if any sessions have Claude actively working
    pub fn has_working_sessions(&self) -> bool {
        self.sessions.iter().any(|s| s.claude_activity.is_working())
    }

    /// Count sessions where Claude is actively working
    pub fn working_session_count(&self) -> usize {
        self.sessions.iter().filter(|s| s.claude_activity.is_working()).count()
    }

    /// Get the session ID of the oldest alerting session (lowest alert_order).
    /// Returns None if no sessions have an active HID alert.
    pub fn oldest_alerting_session_id(&self) -> Option<SessionId> {
        self.sessions
            .iter()
            .filter(|s| s.hid_alert_active)
            .min_by_key(|s| s.alert_order)
            .map(|s| s.id)
    }

    /// Get sessions as a slice for rendering
    pub fn sessions(&self) -> &[SessionInfo] {
        &self.sessions
    }

    /// Compute the HID-space tab index for a given session ID.
    ///
    /// HID tab indices skip new-tab placeholders, matching `collect_tab_states()` ordering.
    /// Returns None if the session is a new-tab or not found.
    pub fn session_hid_tab_index(&self, session_id: SessionId) -> Option<usize> {
        let mut idx = 0;
        for session in &self.sessions {
            if session.is_new_tab() {
                continue;
            }
            if session.id == session_id {
                return Some(idx);
            }
            idx += 1;
        }
        None
    }

    /// Collect tab states for HID display update.
    ///
    /// Returns `(tab_states, active_index)` where:
    /// - `tab_states` is a Vec of u8 state constants for all non-new-tab sessions
    /// - `active_index` is the index into that Vec for the currently active session
    ///   (defaults to 0 if active session is a new tab or not found)
    pub fn collect_tab_states(&self) -> (Vec<u8>, usize) {
        use crate::hid::protocol::{TAB_STATE_INACTIVE, TAB_STATE_STARTED, TAB_STATE_WORKING};

        let active_id = self.active_session_id();
        let mut tab_states = Vec::new();
        let mut active_index = 0;

        for session in &self.sessions {
            if session.is_new_tab() {
                continue;
            }

            let state = if !session.is_running {
                TAB_STATE_INACTIVE
            } else {
                match session.claude_activity {
                    ClaudeActivity::Unknown => TAB_STATE_INACTIVE,
                    ClaudeActivity::Idle => TAB_STATE_STARTED,
                    ClaudeActivity::Working => TAB_STATE_WORKING,
                }
            };

            if Some(session.id) == active_id {
                active_index = tab_states.len();
            }

            tab_states.push(state);
        }

        (tab_states, active_index)
    }

    /// Compute disambiguated display titles for all sessions
    ///
    /// This handles:
    /// 1. Same-named directories under different paths (e.g., project1/app vs project2/app)
    /// 2. Same directory with different Claude sessions
    ///
    /// Returns a map of session ID to display title
    pub fn compute_display_titles(&self, max_len: usize) -> HashMap<SessionId, String> {
        let mut result = HashMap::new();

        // Skip if no sessions
        if self.sessions.is_empty() {
            return result;
        }

        // Step 1: Group sessions by their base directory name
        let mut groups: HashMap<String, Vec<&SessionInfo>> = HashMap::new();
        for session in &self.sessions {
            if session.is_new_tab() {
                // New tabs just use their title directly
                result.insert(session.id, session.title.clone());
                continue;
            }

            let base_name = session
                .working_directory
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("Unknown")
                .to_string();

            groups.entry(base_name).or_default().push(session);
        }

        // Step 2: Process each group
        for (_base_name, sessions_in_group) in groups {
            if sessions_in_group.len() == 1 {
                // No conflict - but still show parent/dir format for context
                let session = sessions_in_group[0];
                let title = build_title_with_parent(&session.working_directory, session, max_len);
                result.insert(session.id, title);
            } else {
                // Conflict - need disambiguation
                disambiguate_sessions(&sessions_in_group, max_len, &mut result);
            }
        }

        result
    }
}

/// Build a title showing parent/directory format (always includes parent for context)
fn build_title_with_parent(path: &PathBuf, session: &SessionInfo, max_len: usize) -> String {
    let base_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("Unknown");

    // Get the immediate parent directory name
    let parent_name = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str());

    let path_title = if let Some(parent) = parent_name {
        format!("{}/{}", parent, base_name)
    } else {
        base_name.to_string()
    };

    // Add terminal title context if available
    if let Some(ref term_title) = session.terminal_title {
        if !term_title.is_empty() {
            let suffix_max = max_len.saturating_sub(path_title.len() + 2);
            let short_suffix = if term_title.len() > suffix_max {
                format!("{}...", &term_title[..suffix_max.saturating_sub(3)])
            } else {
                term_title.clone()
            };
            return truncate_title(&format!("{}: {}", path_title, short_suffix), max_len);
        }
    }

    truncate_title(&path_title, max_len)
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Truncate a title to max length, adding ellipsis if needed
fn truncate_title(title: &str, max_len: usize) -> String {
    if title.len() <= max_len {
        title.to_string()
    } else {
        format!("{}...", &title[..max_len.saturating_sub(3)])
    }
}

/// Get parent directories as path components, from innermost to outermost
fn get_parent_components(path: &Path) -> Vec<&str> {
    path.ancestors()
        .skip(1) // Skip the path itself
        .filter_map(|p| p.file_name().and_then(|n| n.to_str()))
        .collect()
}

/// Shorten a path for display, using ~ for home directory
fn shorten_path(path: &Path) -> String {
    let path_str = path.to_string_lossy();

    if let Some(home) = dirs::home_dir() {
        let home_str = home.to_string_lossy();
        if path_str.starts_with(home_str.as_ref()) {
            return format!("~{}", &path_str[home_str.len()..]);
        }
    }

    path_str.to_string()
}

/// Disambiguate sessions with the same base directory name
fn disambiguate_sessions(
    sessions: &[&SessionInfo],
    max_len: usize,
    result: &mut HashMap<SessionId, String>,
) {
    // First, group by working directory path to find same-directory conflicts
    let mut by_path: HashMap<&PathBuf, Vec<&SessionInfo>> = HashMap::new();
    for session in sessions {
        by_path
            .entry(&session.working_directory)
            .or_default()
            .push(session);
    }

    // Check if all sessions are in the same directory
    if by_path.len() == 1 {
        // All sessions are for the same directory - disambiguate by Claude session
        disambiguate_by_session(sessions, max_len, result);
    } else if by_path.values().all(|v| v.len() == 1) {
        // All different directories - disambiguate by path
        disambiguate_by_path(sessions, max_len, result);
    } else {
        // Mix: some same directory, some different
        // First, add path context for different directories
        // Then, for same-directory groups, add session context
        for (_path, group) in &by_path {
            if group.len() == 1 {
                // Single session for this path - use path disambiguation
                let session = group[0];
                let title = build_path_title(&session.working_directory, sessions, max_len);
                result.insert(session.id, title);
            } else {
                // Multiple sessions for same path - need session disambiguation
                disambiguate_same_path_sessions(group, sessions, max_len, result);
            }
        }
    }
}

/// Disambiguate sessions that are all in different directories (same base name)
fn disambiguate_by_path(
    sessions: &[&SessionInfo],
    max_len: usize,
    result: &mut HashMap<SessionId, String>,
) {
    for session in sessions {
        let path_title = build_path_title(&session.working_directory, sessions, max_len);

        // Add terminal title context if available
        let title = if let Some(ref term_title) = session.terminal_title {
            if !term_title.is_empty() {
                build_title_with_suffix(&path_title, term_title, max_len)
            } else {
                path_title
            }
        } else {
            path_title
        };

        result.insert(session.id, title);
    }
}

/// Build a title with enough path context to be unique
fn build_path_title(path: &PathBuf, all_sessions: &[&SessionInfo], max_len: usize) -> String {
    let base_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("Unknown");

    let parent_components = get_parent_components(path);

    // Try adding parent directories one by one until unique
    for depth in 1..=parent_components.len() {
        let mut parts: Vec<&str> = parent_components[..depth].iter().copied().collect();
        parts.reverse();
        parts.push(base_name);
        let candidate = parts.join("/");

        // Check if this is unique among all sessions
        let is_unique = all_sessions
            .iter()
            .filter(|s| s.working_directory != *path)
            .all(|other| {
                let other_components = get_parent_components(&other.working_directory);
                let other_base = other
                    .working_directory
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("");

                if depth > other_components.len() {
                    return true; // Different depth means unique
                }

                let mut other_parts: Vec<&str> =
                    other_components[..depth].iter().copied().collect();
                other_parts.reverse();
                other_parts.push(other_base);
                let other_candidate = other_parts.join("/");

                candidate != other_candidate
            });

        if is_unique {
            return truncate_title(&candidate, max_len);
        }
    }

    // Fallback: use shortened full path
    truncate_title(&shorten_path(path), max_len)
}

/// Disambiguate sessions that are all for the same directory (different Claude sessions)
fn disambiguate_by_session(
    sessions: &[&SessionInfo],
    max_len: usize,
    result: &mut HashMap<SessionId, String>,
) {
    let base_name = sessions
        .first()
        .and_then(|s| s.working_directory.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("Unknown");

    // Get Claude session info for each (fallback if no terminal_title)
    let claude_sessions = sessions
        .first()
        .map(|s| get_sessions_for_directory(&s.working_directory))
        .unwrap_or_default();

    // Build a lookup from session ID to summary
    let session_summaries: HashMap<&str, String> = claude_sessions
        .iter()
        .map(|s| (s.session_id.as_str(), s.display_title()))
        .collect();

    for session in sessions {
        // Prefer terminal_title (from OSC) over Claude session metadata
        let title = if let Some(ref term_title) = session.terminal_title {
            if !term_title.is_empty() {
                build_title_with_suffix(base_name, term_title, max_len)
            } else {
                build_session_title(session, base_name, &session_summaries, max_len)
            }
        } else {
            build_session_title(session, base_name, &session_summaries, max_len)
        };

        result.insert(session.id, title);
    }
}

/// Build a title with base name and suffix
fn build_title_with_suffix(base: &str, suffix: &str, max_len: usize) -> String {
    let suffix_max = max_len.saturating_sub(base.len() + 2);
    let short_suffix = if suffix.len() > suffix_max {
        format!("{}...", &suffix[..suffix_max.saturating_sub(3)])
    } else {
        suffix.to_string()
    };
    truncate_title(&format!("{}: {}", base, short_suffix), max_len)
}

/// Build a title using Claude session metadata as fallback
fn build_session_title(
    session: &SessionInfo,
    base_name: &str,
    session_summaries: &HashMap<&str, String>,
    max_len: usize,
) -> String {
    let title = if let Some(ref claude_id) = session.claude_session_id {
        if claude_id.is_empty() {
            // Explicit new session
            format!("{}: New", base_name)
        } else if let Some(summary) = session_summaries.get(claude_id.as_str()) {
            // Has a summary - append truncated version
            build_title_with_suffix(base_name, summary, max_len)
        } else {
            // Has session ID but no summary found
            format!("{}: Session", base_name)
        }
    } else {
        // No explicit session - will auto-continue
        format!("{}: Continue", base_name)
    };

    truncate_title(&title, max_len)
}

/// Disambiguate sessions for the same path when there are also other paths
fn disambiguate_same_path_sessions(
    same_path_sessions: &[&SessionInfo],
    all_sessions: &[&SessionInfo],
    max_len: usize,
    result: &mut HashMap<SessionId, String>,
) {
    // First, get the path-disambiguated prefix
    let path = &same_path_sessions[0].working_directory;
    let path_prefix = build_path_title(path, all_sessions, max_len);

    // Remove trailing components to make room for session context
    let base = path_prefix
        .split('/')
        .last()
        .unwrap_or(&path_prefix);

    // Get Claude session info (fallback if no terminal_title)
    let claude_sessions = get_sessions_for_directory(path);
    let session_summaries: HashMap<&str, String> = claude_sessions
        .iter()
        .map(|s| (s.session_id.as_str(), s.display_title()))
        .collect();

    for session in same_path_sessions {
        // Prefer terminal_title (from OSC) over Claude session metadata
        let title = if let Some(ref term_title) = session.terminal_title {
            if !term_title.is_empty() {
                build_title_with_suffix(base, term_title, max_len)
            } else {
                build_session_title(session, base, &session_summaries, max_len)
            }
        } else {
            build_session_title(session, base, &session_summaries, max_len)
        };

        result.insert(session.id, title);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_palette() -> ColorPalette {
        ColorPalette::default()
    }

    #[test]
    fn test_session_manager_create() {
        let mut manager = SessionManager::new();
        assert!(manager.is_empty());

        let id1 = manager.create_session(PathBuf::from("/home/user/project1"), &default_palette());
        assert_eq!(manager.session_count(), 1);
        assert_eq!(manager.active_session_id(), Some(id1));

        let id2 = manager.create_session(PathBuf::from("/home/user/project2"), &default_palette());
        assert_eq!(manager.session_count(), 2);
        // Active session should still be the first one
        assert_eq!(manager.active_session_id(), Some(id1));
    }

    #[test]
    fn test_session_manager_switch() {
        let mut manager = SessionManager::new();
        let id1 = manager.create_session(PathBuf::from("/project1"), &default_palette());
        let id2 = manager.create_session(PathBuf::from("/project2"), &default_palette());

        assert!(manager.set_active_session(id2));
        assert_eq!(manager.active_session_id(), Some(id2));

        assert!(manager.set_active_session(id1));
        assert_eq!(manager.active_session_id(), Some(id1));
    }

    #[test]
    fn test_session_manager_close() {
        let mut manager = SessionManager::new();
        let id1 = manager.create_session(PathBuf::from("/project1"), &default_palette());
        let id2 = manager.create_session(PathBuf::from("/project2"), &default_palette());
        let id3 = manager.create_session(PathBuf::from("/project3"), &default_palette());

        manager.set_active_session(id2);
        assert_eq!(manager.active_session_index(), 1);

        // Close the active session
        assert!(manager.close_session(id2));
        assert_eq!(manager.session_count(), 2);
        // Active should now be id3 (at index 1)
        assert_eq!(manager.active_session_index(), 1);

        // Close a session before the active one
        manager.set_active_session(id3);
        assert!(manager.close_session(id1));
        assert_eq!(manager.active_session_index(), 0);
    }

    #[test]
    fn test_new_tab_session() {
        let mut manager = SessionManager::new();
        let id = manager.create_new_tab(&default_palette());

        let session = manager.get_session(id).unwrap();
        assert!(session.is_new_tab());
        assert_eq!(session.title, "New Session");
    }

    #[test]
    fn test_display_title_truncation() {
        // Use a path with a long final component to test truncation
        let session = SessionInfo::new(0, PathBuf::from("/path/to/very_long_directory_name_here"), &default_palette());
        let title = session.display_title(10);
        assert!(title.len() <= 10);
        assert!(title.ends_with("..."));
    }

    #[test]
    fn test_compute_display_titles_unique() {
        // When all directories have different names, show parent/dir format for context
        let mut manager = SessionManager::new();
        manager.create_session(PathBuf::from("/home/user/project1"), &default_palette());
        manager.create_session(PathBuf::from("/home/user/project2"), &default_palette());
        manager.create_session(PathBuf::from("/work/different"), &default_palette());

        let titles = manager.compute_display_titles(50);
        assert_eq!(titles.len(), 3);

        // All should have parent/dir format for context
        let values: Vec<_> = titles.values().collect();
        assert!(values.iter().any(|v| *v == "user/project1"));
        assert!(values.iter().any(|v| *v == "user/project2"));
        assert!(values.iter().any(|v| *v == "work/different"));
    }

    #[test]
    fn test_compute_display_titles_same_name_different_path() {
        // When directories have the same name but different paths, disambiguate
        let mut manager = SessionManager::new();
        let id1 = manager.create_session(PathBuf::from("/work/project1/app"), &default_palette());
        let id2 = manager.create_session(PathBuf::from("/work/project2/app"), &default_palette());

        let titles = manager.compute_display_titles(50);
        assert_eq!(titles.len(), 2);

        // Both should have parent context to disambiguate
        let title1 = titles.get(&id1).unwrap();
        let title2 = titles.get(&id2).unwrap();

        assert!(title1.contains("project1") || title1.contains("app"));
        assert!(title2.contains("project2") || title2.contains("app"));
        assert_ne!(title1, title2); // They should be different
    }

    #[test]
    fn test_compute_display_titles_new_tab() {
        // New tabs should just show "New Session"
        let mut manager = SessionManager::new();
        let id = manager.create_new_tab(&default_palette());

        let titles = manager.compute_display_titles(50);
        assert_eq!(titles.get(&id), Some(&"New Session".to_string()));
    }

    #[test]
    fn test_truncate_title() {
        assert_eq!(truncate_title("short", 10), "short");
        assert_eq!(truncate_title("very long title here", 10), "very lo...");
        assert_eq!(truncate_title("exactly10!", 10), "exactly10!");
    }

    #[test]
    fn test_get_parent_components() {
        let path = PathBuf::from("/home/user/work/project");
        let components = get_parent_components(&path);
        // Should be [work, user, home] - innermost to outermost
        assert_eq!(components, vec!["work", "user", "home"]);
    }

    #[test]
    fn test_compute_display_titles_with_terminal_title() {
        // When a session has a terminal_title set, it should be included in the display
        let mut manager = SessionManager::new();
        let id = manager.create_session(PathBuf::from("/home/user/project"), &default_palette());

        // Set the terminal title (simulating OSC title change from Claude)
        if let Some(session) = manager.get_session_mut(id) {
            session.terminal_title = Some("Building feature X".to_string());
        }

        let titles = manager.compute_display_titles(50);
        let title = titles.get(&id).unwrap();

        // Should include both the directory name and terminal title
        assert!(title.contains("project"));
        assert!(title.contains("Building feature X"));
    }

    #[test]
    fn test_build_title_with_suffix() {
        assert_eq!(build_title_with_suffix("app", "Testing", 20), "app: Testing");
        assert_eq!(build_title_with_suffix("app", "Very long title here", 15), "app: Very lo...");
    }
}
