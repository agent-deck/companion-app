//! Claude Code session discovery
//!
//! Discovers sessions by scanning `.jsonl` conversation history files directly,
//! enriching with metadata from `sessions-index.json` when available (for summaries).
//! This approach is resilient to the index file being stale or missing.

use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::Instant;
use tracing::warn;

/// Cache TTL in seconds — disk scans happen at most this often per directory.
const CACHE_TTL_SECS: u64 = 30;

struct CacheEntry {
    sessions: Vec<ClaudeSession>,
    fetched_at: Instant,
}

thread_local! {
    static SESSION_CACHE: RefCell<HashMap<PathBuf, CacheEntry>> = RefCell::new(HashMap::new());
}

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

// --- sessions-index.json structures (for enrichment) ---

#[derive(Debug, Deserialize)]
struct SessionsIndex {
    #[allow(dead_code)]
    version: u32,
    entries: Vec<SessionIndexEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionIndexEntry {
    session_id: String,
    summary: Option<String>,
    first_prompt: Option<String>,
    #[allow(dead_code)]
    message_count: Option<u32>,
    #[allow(dead_code)]
    created: Option<String>,
    #[allow(dead_code)]
    modified: Option<String>,
}

// --- .jsonl parsing structures ---

#[derive(Deserialize)]
struct JsonlLine {
    #[serde(rename = "type")]
    msg_type: String,
    timestamp: Option<String>,
    message: Option<JsonlMessage>,
}

#[derive(Deserialize)]
struct JsonlMessage {
    content: Option<serde_json::Value>,
}

/// Extract the text content from a user message .jsonl line.
///
/// Handles both `content: "string"` and `content: [{type: "text", text: "..."}]` variants.
/// Returns None for non-user messages or if no text content is found.
fn extract_first_prompt(line: &str) -> Option<String> {
    let parsed: JsonlLine = serde_json::from_str(line).ok()?;
    if parsed.msg_type != "user" {
        return None;
    }
    let content = parsed.message?.content?;
    match content {
        serde_json::Value::String(s) => Some(s),
        serde_json::Value::Array(arr) => {
            for item in arr {
                if let Some(obj) = item.as_object() {
                    if obj.get("type").and_then(|t| t.as_str()) == Some("text") {
                        if let Some(text) = obj.get("text").and_then(|t| t.as_str()) {
                            return Some(text.to_string());
                        }
                    }
                }
            }
            None
        }
        _ => None,
    }
}

/// Check if a prompt looks like a real user message (not a system/internal one).
fn is_real_user_prompt(prompt: &str) -> bool {
    let trimmed = prompt.trim();
    if trimmed.is_empty() {
        return false;
    }
    // Skip system messages like "[Request interrupted by user for tool use]"
    if trimmed.starts_with('[') && trimmed.ends_with(']') {
        return false;
    }
    true
}

/// Scan .jsonl files in a project directory and build session metadata.
fn scan_jsonl_files(project_path: &Path) -> Vec<ClaudeSession> {
    let entries = match std::fs::read_dir(project_path) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut sessions = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();

        // Only .jsonl files directly in the directory (skip subdirs like <uuid>/subagents/)
        if !path.is_file() {
            continue;
        }
        let ext = path.extension().and_then(|e| e.to_str());
        if ext != Some("jsonl") {
            continue;
        }

        let session_id = match path.file_stem().and_then(|s| s.to_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };

        // Read first 2 lines
        let file = match File::open(&path) {
            Ok(f) => f,
            Err(_) => continue,
        };
        let reader = BufReader::new(file);
        let mut lines_iter = reader.lines();

        // Line 1: always file-history-snapshot, skip
        let _line1 = match lines_iter.next() {
            Some(Ok(l)) => l,
            _ => continue,
        };

        // Line 2: should be the first user message
        let line2 = match lines_iter.next() {
            Some(Ok(l)) => l,
            _ => continue, // stub session (only 1 line)
        };

        // Quick check: skip if line 2 is not a user message (stub session)
        if !line2.contains("\"type\":\"user\"") && !line2.contains("\"type\": \"user\"") {
            continue;
        }

        // Extract first prompt — skip system messages like "[Request interrupted...]"
        let mut first_prompt = extract_first_prompt(&line2)
            .filter(|p| is_real_user_prompt(p));

        // Extract created timestamp from line 2
        let created = serde_json::from_str::<JsonlLine>(&line2)
            .ok()
            .and_then(|l| l.timestamp)
            .and_then(|ts| DateTime::parse_from_rfc3339(&ts).ok())
            .map(|dt| dt.with_timezone(&Utc))
            .or_else(|| {
                // Fallback: file birthtime / mtime
                std::fs::metadata(&path)
                    .ok()
                    .and_then(|m| m.created().ok())
                    .map(|t| DateTime::<Utc>::from(t))
            })
            .unwrap_or(DateTime::UNIX_EPOCH);

        // If first prompt was a system message, scan a few more lines for a real one
        if first_prompt.is_none() {
            for line_result in lines_iter.by_ref().take(30) {
                let Ok(line) = line_result else { continue };
                if line.contains("\"type\":\"user\"") || line.contains("\"type\": \"user\"") {
                    if let Some(prompt) = extract_first_prompt(&line) {
                        if is_real_user_prompt(&prompt) {
                            first_prompt = Some(prompt);
                            break;
                        }
                    }
                }
            }
        }

        // Modified from file metadata (fast — no file content reading)
        let modified = std::fs::metadata(&path)
            .ok()
            .and_then(|m| m.modified().ok())
            .map(|t| DateTime::<Utc>::from(t))
            .unwrap_or(DateTime::UNIX_EPOCH);

        // Message count: 0 here, enriched from index in merge step
        sessions.push(ClaudeSession {
            session_id,
            summary: None,
            first_prompt,
            message_count: 0,
            created,
            modified,
        });
    }

    sessions
}

/// Read sessions-index.json and return entries keyed by session_id for merge lookups.
fn read_sessions_index(project_path: &Path) -> HashMap<String, SessionIndexEntry> {
    let index_path = project_path.join("sessions-index.json");
    let content = match std::fs::read_to_string(&index_path) {
        Ok(c) => c,
        Err(_) => return HashMap::new(),
    };
    let index: SessionsIndex = match serde_json::from_str(&content) {
        Ok(i) => i,
        Err(e) => {
            warn!("Failed to parse sessions-index.json: {}", e);
            return HashMap::new();
        }
    };
    index
        .entries
        .into_iter()
        .map(|e| (e.session_id.clone(), e))
        .collect()
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
/// Checks if the .jsonl file exists — this is the ground truth for session existence.
pub fn session_exists(dir: &Path, session_id: &str) -> bool {
    if let Some(project_path) = get_project_storage_path(dir) {
        let session_file = project_path.join(format!("{}.jsonl", session_id));
        session_file.exists()
    } else {
        false
    }
}

/// Get sessions for a directory from Claude Code's storage.
///
/// Results are cached for 30 seconds to avoid repeated disk scans on every frame.
/// Discovers sessions by scanning .jsonl files (primary source), then enriches
/// with summary/first_prompt from sessions-index.json when available.
/// Returns sessions sorted by modified date (most recent first).
pub fn get_sessions_for_directory(dir: &Path) -> Vec<ClaudeSession> {
    let Some(project_path) = get_project_storage_path(dir) else {
        return Vec::new();
    };

    SESSION_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();

        // Return cached result if fresh
        if let Some(entry) = cache.get(&project_path) {
            if entry.fetched_at.elapsed().as_secs() < CACHE_TTL_SECS {
                return entry.sessions.clone();
            }
        }

        // Cache miss or stale — do full scan + merge
        let sessions = fetch_sessions_uncached(&project_path);
        cache.insert(
            project_path,
            CacheEntry {
                sessions: sessions.clone(),
                fetched_at: Instant::now(),
            },
        );
        sessions
    })
}

/// Perform the actual disk scan and index merge (expensive, called on cache miss).
fn fetch_sessions_uncached(project_path: &Path) -> Vec<ClaudeSession> {
    // Primary source: scan .jsonl files
    let mut sessions = scan_jsonl_files(project_path);

    // Enrichment: merge data from sessions-index.json
    let index = read_sessions_index(project_path);
    for session in &mut sessions {
        if let Some(entry) = index.get(&session.session_id) {
            // Take summary from index (not available in .jsonl)
            if entry.summary.is_some() {
                session.summary = entry.summary.clone();
            }
            // Take first_prompt from index if .jsonl extraction failed
            if session.first_prompt.is_none() {
                session.first_prompt = entry.first_prompt.clone();
            }
            // Take message_count from index (avoids expensive full-file scan)
            if let Some(count) = entry.message_count {
                session.message_count = count;
            }
        }
    }

    // Filter out plan-mode sub-sessions (auto-generated by Claude Code)
    sessions.retain(|s| {
        !s.first_prompt
            .as_deref()
            .is_some_and(|p| p.starts_with("Implement the following plan:"))
    });

    // Sort by modified date, most recent first
    sessions.sort_by(|a, b| b.modified.cmp(&a.modified));

    sessions
}

/// Get session count for a directory (uses cached data from get_sessions_for_directory).
///
/// Returns 0 if no sessions found or on error.
pub fn get_session_count(dir: &Path) -> usize {
    get_sessions_for_directory(dir).len()
}

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
                // Truncate to first line or 80 chars
                let first_line = prompt.lines().next().unwrap_or(prompt);
                if first_line.len() > 80 {
                    return format!("{}...", &first_line[..77]);
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
            first_prompt: Some("This is a very long prompt that should be truncated because it exceeds the maximum allowed character limit for session display titles in the list".to_string()),
            message_count: 5,
            created: Utc::now(),
            modified: Utc::now(),
        };
        assert!(session_long_prompt.display_title().ends_with("..."));
        assert!(session_long_prompt.display_title().len() <= 83);
    }

    #[test]
    fn test_extract_first_prompt_string_content() {
        let line = r#"{"type":"user","timestamp":"2026-01-15T10:00:00.000Z","message":{"role":"user","content":"Hello world"}}"#;
        assert_eq!(extract_first_prompt(line), Some("Hello world".to_string()));
    }

    #[test]
    fn test_extract_first_prompt_array_content() {
        let line = r#"{"type":"user","timestamp":"2026-01-15T10:00:00.000Z","message":{"role":"user","content":[{"type":"text","text":"Array hello"}]}}"#;
        assert_eq!(
            extract_first_prompt(line),
            Some("Array hello".to_string())
        );
    }

    #[test]
    fn test_extract_first_prompt_tool_result_only() {
        let line = r#"{"type":"user","timestamp":"2026-01-15T10:00:00.000Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"abc","content":"result"}]}}"#;
        assert_eq!(extract_first_prompt(line), None);
    }

    #[test]
    fn test_extract_first_prompt_non_user_type() {
        let line = r#"{"type":"assistant","timestamp":"2026-01-15T10:00:00.000Z","message":{"role":"assistant","content":"I can help"}}"#;
        assert_eq!(extract_first_prompt(line), None);
    }

    #[test]
    fn test_extract_first_prompt_file_history_snapshot() {
        let line = r#"{"type":"file-history-snapshot","messageId":"abc"}"#;
        assert_eq!(extract_first_prompt(line), None);
    }
}
