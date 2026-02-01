//! Claude state extraction from terminal output
//!
//! Parses Claude Code CLI output to extract current state information
//! for display on the Agent Deck.

use crate::core::state::ClaudeState;
use once_cell::sync::Lazy;
use regex::Regex;
use super::parser::{AnsiParser, ParsedElement};

/// Regular expressions for parsing Claude output
static MODEL_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)(claude[- ]?(?:3\.5|4|opus|sonnet|haiku)[^\s]*)").unwrap()
});

static TOKEN_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(\d{1,3}(?:,\d{3})*|\d+)\s*(?:tokens?|tok)").unwrap()
});

static COST_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\$(\d+(?:\.\d{2})?)").unwrap()
});

static PROGRESS_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(\d{1,3})%").unwrap()
});

static TASK_PATTERNS: Lazy<Vec<Regex>> = Lazy::new(|| {
    vec![
        // Common task indicators
        Regex::new(r"(?i)(?:working on|processing|analyzing|reading|writing|editing|creating|updating|fixing|refactoring|implementing|reviewing)\s+(.+)").unwrap(),
        // File operations
        Regex::new(r"(?i)(?:file|path):\s*(.+)").unwrap(),
        // Tool use indicators
        Regex::new(r"(?i)(?:using|running|executing)\s+(.+)").unwrap(),
    ]
});

/// Spinner patterns to detect active processing
static SPINNER_CHARS: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏', '◐', '◓', '◑', '◒', '⣾', '⣽', '⣻', '⢿', '⡿', '⣟', '⣯', '⣷'];

/// Extracts Claude state from terminal output
pub struct ClaudeStateExtractor {
    /// ANSI parser
    parser: AnsiParser,
    /// Current accumulated state
    current_state: ClaudeState,
    /// Lines buffer for multi-line parsing
    lines: Vec<String>,
    /// Current line being built
    current_line: String,
    /// Whether we're in an active session
    in_session: bool,
}

impl Default for ClaudeStateExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl ClaudeStateExtractor {
    /// Create a new state extractor
    pub fn new() -> Self {
        Self {
            parser: AnsiParser::new(),
            current_state: ClaudeState::default(),
            lines: Vec::new(),
            current_line: String::new(),
            in_session: false,
        }
    }

    /// Process raw PTY output and extract state
    pub fn process(&mut self, data: &[u8]) -> Option<ClaudeState> {
        let elements = self.parser.parse(data);
        let mut state_changed = false;

        for element in elements {
            match element {
                ParsedElement::Text(text) => {
                    self.current_line.push_str(&text);
                    state_changed |= self.extract_from_text(&text);
                }
                ParsedElement::Newline => {
                    if !self.current_line.is_empty() {
                        let line = std::mem::take(&mut self.current_line);
                        state_changed |= self.process_line(&line);
                        self.lines.push(line);
                        // Keep only last 100 lines
                        if self.lines.len() > 100 {
                            self.lines.remove(0);
                        }
                    }
                }
                ParsedElement::CarriageReturn => {
                    // Status line update - process current line
                    if !self.current_line.is_empty() {
                        let line = self.current_line.clone();
                        state_changed |= self.process_line(&line);
                    }
                    self.current_line.clear();
                }
                ParsedElement::ClearScreen => {
                    // Screen cleared - might indicate new session or major state change
                    self.lines.clear();
                    self.current_line.clear();
                }
                ParsedElement::ClearLine => {
                    self.current_line.clear();
                }
                _ => {}
            }
        }

        // Process current line for status updates
        if !self.current_line.is_empty() {
            let line = self.current_line.clone();
            state_changed |= self.process_line(&line);
        }

        if state_changed {
            Some(self.current_state.clone())
        } else {
            None
        }
    }

    /// Extract state information from a text fragment
    fn extract_from_text(&mut self, text: &str) -> bool {
        // Check for spinner (indicates active processing)
        for c in text.chars() {
            if SPINNER_CHARS.contains(&c) {
                self.in_session = true;
                break;
            }
        }

        false
    }

    /// Process a complete line of output
    fn process_line(&mut self, line: &str) -> bool {
        let mut changed = false;

        // Extract model name
        if let Some(captures) = MODEL_REGEX.captures(line) {
            if let Some(model) = captures.get(1) {
                let model_str = model.as_str().to_string();
                if self.current_state.model != model_str {
                    self.current_state.model = model_str;
                    changed = true;
                }
            }
        }

        // Extract token count
        if let Some(captures) = TOKEN_REGEX.captures(line) {
            if let Some(tokens) = captures.get(1) {
                let tokens_str = tokens.as_str().to_string();
                if self.current_state.tokens != tokens_str {
                    self.current_state.tokens = tokens_str;
                    changed = true;
                }
            }
        }

        // Extract cost
        if let Some(captures) = COST_REGEX.captures(line) {
            if let Some(cost) = captures.get(0) {
                let cost_str = cost.as_str().to_string();
                if self.current_state.cost != cost_str {
                    self.current_state.cost = cost_str;
                    changed = true;
                }
            }
        }

        // Extract progress
        if let Some(captures) = PROGRESS_REGEX.captures(line) {
            if let Some(progress) = captures.get(1) {
                if let Ok(pct) = progress.as_str().parse::<u8>() {
                    if self.current_state.progress != pct {
                        self.current_state.progress = pct.min(100);
                        changed = true;
                    }
                }
            }
        }

        // Extract task description
        for pattern in TASK_PATTERNS.iter() {
            if let Some(captures) = pattern.captures(line) {
                if let Some(task) = captures.get(1) {
                    let task_str = task.as_str().trim().to_string();
                    if !task_str.is_empty() && self.current_state.task != task_str {
                        self.current_state.task = task_str;
                        changed = true;
                        break;
                    }
                }
            }
        }

        // Detect session end
        if line.contains("Goodbye") || line.contains("Session ended") {
            self.in_session = false;
            self.current_state = ClaudeState::default();
            changed = true;
        }

        changed
    }

    /// Get current extracted state
    pub fn state(&self) -> &ClaudeState {
        &self.current_state
    }

    /// Check if we're in an active session
    pub fn is_active(&self) -> bool {
        self.in_session
    }

    /// Reset extractor state
    pub fn reset(&mut self) {
        self.parser.reset();
        self.current_state = ClaudeState::default();
        self.lines.clear();
        self.current_line.clear();
        self.in_session = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_model() {
        let mut extractor = ClaudeStateExtractor::new();
        let state = extractor.process(b"Using Claude 3.5 Sonnet\n");

        assert!(state.is_some());
        let state = state.unwrap();
        assert!(state.model.contains("Claude"));
    }

    #[test]
    fn test_extract_tokens() {
        let mut extractor = ClaudeStateExtractor::new();
        let state = extractor.process(b"Used 1,234 tokens so far\n");

        assert!(state.is_some());
        let state = state.unwrap();
        assert_eq!(state.tokens, "1,234");
    }

    #[test]
    fn test_extract_cost() {
        let mut extractor = ClaudeStateExtractor::new();
        let state = extractor.process(b"Cost: $0.05\n");

        assert!(state.is_some());
        let state = state.unwrap();
        assert_eq!(state.cost, "$0.05");
    }

    #[test]
    fn test_extract_progress() {
        let mut extractor = ClaudeStateExtractor::new();
        let state = extractor.process(b"Progress: 75%\n");

        assert!(state.is_some());
        let state = state.unwrap();
        assert_eq!(state.progress, 75);
    }

    #[test]
    fn test_extract_task() {
        let mut extractor = ClaudeStateExtractor::new();
        let state = extractor.process(b"Working on auth.rs\n");

        assert!(state.is_some());
        let state = state.unwrap();
        assert!(state.task.contains("auth.rs"));
    }

    #[test]
    fn test_carriage_return_update() {
        let mut extractor = ClaudeStateExtractor::new();

        // Simulate a progress update with carriage return
        extractor.process(b"Progress: 25%");
        let state = extractor.process(b"\rProgress: 50%");

        assert!(state.is_some());
        let state = state.unwrap();
        assert_eq!(state.progress, 50);
    }
}
