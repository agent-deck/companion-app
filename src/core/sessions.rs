//! Session management for multiple Claude sessions
//!
//! Manages multiple terminal sessions, each with its own PTY, working directory,
//! and Claude state.

use crate::core::state::ClaudeState;
use crate::terminal::Session;
use crate::window::InputSender;
use parking_lot::Mutex;
use std::path::PathBuf;
use std::sync::Arc;

/// Unique identifier for a session
pub type SessionId = usize;

/// Information about a single Claude session
pub struct SessionInfo {
    /// Unique session ID
    pub id: SessionId,
    /// Working directory for this session
    pub working_directory: PathBuf,
    /// Display title (auto-derived from CWD or user-set)
    pub title: String,
    /// Terminal session (WezTerm-based)
    pub session: Arc<Mutex<Session>>,
    /// Input sender to PTY (None if session not started)
    pub pty_input_tx: Option<InputSender>,
    /// Claude state for this session
    pub claude_state: Arc<Mutex<ClaudeState>>,
    /// Whether this session has an active PTY
    pub is_running: bool,
    /// Whether PTY is currently being started (for loading indicator)
    pub is_loading: bool,
}

impl SessionInfo {
    /// Create a new session with the given working directory
    pub fn new(id: SessionId, working_directory: PathBuf) -> Self {
        let title = working_directory
            .file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "New Tab".to_string());

        Self {
            id,
            working_directory,
            title,
            session: Arc::new(Mutex::new(Session::new(0, 120, 50))),
            pty_input_tx: None,
            claude_state: Arc::new(Mutex::new(ClaudeState::default())),
            is_running: false,
            is_loading: false,
        }
    }

    /// Create a new tab session (not yet started, shows new tab page)
    pub fn new_tab(id: SessionId) -> Self {
        Self {
            id,
            working_directory: PathBuf::new(),
            title: "New Tab".to_string(),
            session: Arc::new(Mutex::new(Session::new(0, 120, 50))),
            pty_input_tx: None,
            claude_state: Arc::new(Mutex::new(ClaudeState::default())),
            is_running: false,
            is_loading: false,
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
    pub fn create_session(&mut self, working_directory: PathBuf) -> SessionId {
        let id = self.next_id;
        self.next_id += 1;
        self.sessions.push(SessionInfo::new(id, working_directory));
        id
    }

    /// Create a new tab (empty session for new tab page)
    pub fn create_new_tab(&mut self) -> SessionId {
        let id = self.next_id;
        self.next_id += 1;
        self.sessions.push(SessionInfo::new_tab(id));
        id
    }

    /// Create a placeholder session (has working directory but PTY not started)
    /// Used for restoring tabs from saved state with lazy loading
    pub fn create_placeholder(&mut self, working_directory: PathBuf, title: String) -> SessionId {
        let id = self.next_id;
        self.next_id += 1;
        let mut session = SessionInfo::new(id, working_directory);
        session.title = title;
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

    /// Get sessions as a slice for rendering
    pub fn sessions(&self) -> &[SessionInfo] {
        &self.sessions
    }
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_manager_create() {
        let mut manager = SessionManager::new();
        assert!(manager.is_empty());

        let id1 = manager.create_session(PathBuf::from("/home/user/project1"));
        assert_eq!(manager.session_count(), 1);
        assert_eq!(manager.active_session_id(), Some(id1));

        let id2 = manager.create_session(PathBuf::from("/home/user/project2"));
        assert_eq!(manager.session_count(), 2);
        // Active session should still be the first one
        assert_eq!(manager.active_session_id(), Some(id1));
    }

    #[test]
    fn test_session_manager_switch() {
        let mut manager = SessionManager::new();
        let id1 = manager.create_session(PathBuf::from("/project1"));
        let id2 = manager.create_session(PathBuf::from("/project2"));

        assert!(manager.set_active_session(id2));
        assert_eq!(manager.active_session_id(), Some(id2));

        assert!(manager.set_active_session(id1));
        assert_eq!(manager.active_session_id(), Some(id1));
    }

    #[test]
    fn test_session_manager_close() {
        let mut manager = SessionManager::new();
        let id1 = manager.create_session(PathBuf::from("/project1"));
        let id2 = manager.create_session(PathBuf::from("/project2"));
        let id3 = manager.create_session(PathBuf::from("/project3"));

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
        let id = manager.create_new_tab();

        let session = manager.get_session(id).unwrap();
        assert!(session.is_new_tab());
        assert_eq!(session.title, "New Tab");
    }

    #[test]
    fn test_display_title_truncation() {
        let session = SessionInfo::new(0, PathBuf::from("/very/long/directory/name/here"));
        let title = session.display_title(10);
        assert!(title.len() <= 10);
        assert!(title.ends_with("..."));
    }
}
