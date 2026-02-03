//! Claude Code session discovery
//!
//! Reads session metadata from Claude Code's local storage to enable
//! session-aware directory selection in the new tab UI.

use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use tracing::warn;

/// A Claude Code session with metadata
#[derive(Debug, Clone)]
pub struct ClaudeSession {
    pub session_id: String,
    pub summary: Option<String>,
    pub first_prompt: Option<String>,
    pub message_count: u32,
    pub created: DateTime<Utc>,
    pub modified: DateTime<Utc>,
}

/// The sessions-index.json file structure
#[derive(Debug, Deserialize)]
struct SessionsIndex {
    #[allow(dead_code)]
    version: u32,
    entries: Vec<SessionIndexEntry>,
}

/// Raw session entry as stored in sessions-index.json
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionIndexEntry {
    session_id: String,
    summary: Option<String>,
    first_prompt: Option<String>,
    message_count: Option<u32>,
    created: Option<String>,
    modified: Option<String>,
}

impl SessionIndexEntry {
    fn into_claude_session(self) -> Option<ClaudeSession> {
        // Parse dates, defaulting to epoch if missing/invalid
        let created = self
            .created
            .as_ref()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|| DateTime::UNIX_EPOCH.into());

        let modified = self
            .modified
            .as_ref()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|| DateTime::UNIX_EPOCH.into());

        Some(ClaudeSession {
            session_id: self.session_id,
            summary: self.summary,
            first_prompt: self.first_prompt,
            message_count: self.message_count.unwrap_or(0),
            created,
            modified,
        })
    }
}

/// Encode a path the way Claude Code does: / → -
///
/// Example: `/Users/vden/work/foo` → `-Users-vden-work-foo`
fn encode_project_path(path: &Path) -> String {
    let path_str = path.to_string_lossy();
    path_str.replace('/', "-").replace('\\', "-").replace('_', "-")
}

/// Get the Claude Code storage directory for a project
fn get_project_storage_path(dir: &Path) -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    let encoded = encode_project_path(dir);
    Some(home.join(".claude").join("projects").join(encoded))
}

/// Find the most recently created session in a directory
///
/// Returns the session ID if found. Sessions are sorted by modified date,
/// so the first one is the most recent.
pub fn find_most_recent_session(dir: &Path) -> Option<String> {
    let sessions = get_sessions_for_directory(dir);
    sessions.first().map(|s| s.session_id.clone())
}

/// Check if a session with the given ID exists in a directory
///
/// Checks both the sessions-index.json and the actual session file (<id>.jsonl)
/// since the index might not be updated immediately after session creation.
pub fn session_exists(dir: &Path, session_id: &str) -> bool {
    // First check if the actual session file exists (more reliable)
    if let Some(project_path) = get_project_storage_path(dir) {
        let session_file = project_path.join(format!("{}.jsonl", session_id));
        if session_file.exists() {
            return true;
        }
    }

    // Fall back to checking the index
    let sessions = get_sessions_for_directory(dir);
    sessions.iter().any(|s| s.session_id == session_id)
}

/// Get sessions for a directory from Claude Code's storage
///
/// Returns sessions sorted by modified date (most recent first).
/// Returns empty Vec if no sessions found or on error.
pub fn get_sessions_for_directory(dir: &Path) -> Vec<ClaudeSession> {
    let Some(project_path) = get_project_storage_path(dir) else {
        return Vec::new();
    };

    let index_path = project_path.join("sessions-index.json");

    // Read and parse the sessions index
    let content = match std::fs::read_to_string(&index_path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let index: SessionsIndex = match serde_json::from_str(&content) {
        Ok(i) => i,
        Err(e) => {
            warn!("Failed to parse sessions-index.json for {:?}: {}", dir, e);
            return Vec::new();
        }
    };

    let mut sessions: Vec<ClaudeSession> = index.entries
        .into_iter()
        .filter_map(|e| e.into_claude_session())
        .collect();

    // Sort by modified date, most recent first
    sessions.sort_by(|a, b| b.modified.cmp(&a.modified));

    sessions
}

/// Get session count for a directory (fast, for display in lists)
///
/// Returns 0 if no sessions found or on error.
pub fn get_session_count(dir: &Path) -> usize {
    let Some(project_path) = get_project_storage_path(dir) else {
        return 0;
    };

    let index_path = project_path.join("sessions-index.json");

    // Read and parse the sessions index
    let content = match std::fs::read_to_string(&index_path) {
        Ok(c) => c,
        Err(_) => return 0,
    };

    // Parse and count entries
    let index: SessionsIndex = match serde_json::from_str(&content) {
        Ok(i) => i,
        Err(_) => return 0,
    };

    index.entries.len()
}

/// Get a display string for a session
impl ClaudeSession {
    /// Get the display title (summary or truncated first prompt)
    pub fn display_title(&self) -> String {
        if let Some(ref summary) = self.summary {
            if !summary.is_empty() {
                return summary.clone();
            }
        }

        if let Some(ref prompt) = self.first_prompt {
            if !prompt.is_empty() {
                // Truncate to first line or 60 chars
                let first_line = prompt.lines().next().unwrap_or(prompt);
                if first_line.len() > 60 {
                    return format!("{}...", &first_line[..57]);
                }
                return first_line.to_string();
            }
        }

        "Untitled session".to_string()
    }

    /// Get a relative time string for the modified date
    pub fn relative_modified_time(&self) -> String {
        let now = Utc::now();
        let duration = now.signed_duration_since(self.modified);

        if duration.num_minutes() < 1 {
            "just now".to_string()
        } else if duration.num_minutes() < 60 {
            let mins = duration.num_minutes();
            if mins == 1 {
                "1 minute ago".to_string()
            } else {
                format!("{} minutes ago", mins)
            }
        } else if duration.num_hours() < 24 {
            let hours = duration.num_hours();
            if hours == 1 {
                "1 hour ago".to_string()
            } else {
                format!("{} hours ago", hours)
            }
        } else if duration.num_days() < 7 {
            let days = duration.num_days();
            if days == 1 {
                "yesterday".to_string()
            } else {
                format!("{} days ago", days)
            }
        } else if duration.num_weeks() < 4 {
            let weeks = duration.num_weeks();
            if weeks == 1 {
                "1 week ago".to_string()
            } else {
                format!("{} weeks ago", weeks)
            }
        } else {
            self.modified.format("%b %d, %Y").to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_project_path() {
        assert_eq!(
            encode_project_path(Path::new("/Users/vden/work/foo")),
            "-Users-vden-work-foo"
        );
        assert_eq!(
            encode_project_path(Path::new("/home/user/project")),
            "-home-user-project"
        );
    }

    #[test]
    fn test_session_display_title() {
        let session = ClaudeSession {
            session_id: "test".to_string(),
            summary: Some("Test Summary".to_string()),
            first_prompt: Some("Test prompt".to_string()),
            message_count: 5,
            created: Utc::now(),
            modified: Utc::now(),
        };
        assert_eq!(session.display_title(), "Test Summary");

        let session_no_summary = ClaudeSession {
            session_id: "test".to_string(),
            summary: None,
            first_prompt: Some("Test prompt text".to_string()),
            message_count: 5,
            created: Utc::now(),
            modified: Utc::now(),
        };
        assert_eq!(session_no_summary.display_title(), "Test prompt text");

        let session_long_prompt = ClaudeSession {
            session_id: "test".to_string(),
            summary: None,
            first_prompt: Some("This is a very long prompt that should be truncated because it exceeds sixty characters".to_string()),
            message_count: 5,
            created: Utc::now(),
            modified: Utc::now(),
        };
        assert!(session_long_prompt.display_title().ends_with("..."));
        assert!(session_long_prompt.display_title().len() <= 63);
    }
}
