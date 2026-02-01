//! Application state management

use serde::{Deserialize, Serialize};

/// State of the Claude Code CLI session
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClaudeState {
    /// Current task being performed
    pub task: String,
    /// AI model being used
    pub model: String,
    /// Progress percentage (0-100)
    pub progress: u8,
    /// Token count (formatted string)
    pub tokens: String,
    /// Cost (formatted string)
    pub cost: String,
}

impl ClaudeState {
    /// Create a new ClaudeState with default values
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if the state has any meaningful data
    pub fn is_empty(&self) -> bool {
        self.task.is_empty() && self.model.is_empty() && self.tokens.is_empty()
    }

    /// Convert to JSON for HID transmission
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// Truncate strings to fit HID protocol limits
    pub fn truncated(&self) -> Self {
        Self {
            task: truncate_string(&self.task, 64),
            model: truncate_string(&self.model, 64),
            progress: self.progress.min(100),
            tokens: truncate_string(&self.tokens, 16),
            cost: truncate_string(&self.cost, 16),
        }
    }
}

/// Global application state
#[derive(Debug, Default)]
pub struct AppState {
    /// Current Claude state
    pub claude_state: ClaudeState,
    /// Whether HID device is connected
    pub hid_connected: bool,
    /// Whether Claude is currently running
    pub claude_running: bool,
    /// Error message if any
    pub error: Option<String>,
}


/// Truncate a string to a maximum length, preserving UTF-8 boundaries
fn truncate_string(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        return s.to_string();
    }

    // Find a valid UTF-8 boundary
    let mut end = max_len;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }

    s[..end].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_claude_state_default() {
        let state = ClaudeState::new();
        assert!(state.is_empty());
        assert_eq!(state.progress, 0);
    }

    #[test]
    fn test_claude_state_to_json() {
        let state = ClaudeState {
            task: "Testing".to_string(),
            model: "Claude 3.5 Sonnet".to_string(),
            progress: 50,
            tokens: "1,000".to_string(),
            cost: "$0.01".to_string(),
        };
        let json = state.to_json();
        assert!(json.contains("\"task\":\"Testing\""));
        assert!(json.contains("\"progress\":50"));
    }

    #[test]
    fn test_truncate_string() {
        assert_eq!(truncate_string("hello", 10), "hello");
        assert_eq!(truncate_string("hello world", 5), "hello");
        // Test UTF-8 handling
        assert_eq!(truncate_string("hello", 3), "hel");
    }

    #[test]
    fn test_claude_state_truncated() {
        let state = ClaudeState {
            task: "A".repeat(100),
            model: "B".repeat(100),
            progress: 150,
            tokens: "C".repeat(50),
            cost: "D".repeat(50),
        };
        let truncated = state.truncated();
        assert_eq!(truncated.task.len(), 64);
        assert_eq!(truncated.model.len(), 64);
        assert_eq!(truncated.progress, 100);
        assert_eq!(truncated.tokens.len(), 16);
        assert_eq!(truncated.cost.len(), 16);
    }
}
