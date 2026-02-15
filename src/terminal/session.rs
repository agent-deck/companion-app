//! Terminal session wrapping WezTerm's Terminal
//!
//! Provides a thin wrapper around wezterm-term's Terminal for use with egui rendering.

use crate::terminal::config::AgentDeckTermConfig;
use crate::terminal::notifications::NotificationHandler;
use parking_lot::Mutex;
use std::io::Write;
use std::sync::{mpsc, Arc};
use wezterm_term::color::ColorPalette;
use wezterm_term::{Alert, CursorPosition, Terminal, TerminalSize};

/// Writer that forwards terminal responses through an mpsc channel to the PTY.
/// This enables OSC 11 background color query responses to reach programs.
struct PtyForwardWriter {
    tx: mpsc::Sender<Vec<u8>>,
}

impl Write for PtyForwardWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let _ = self.tx.send(buf.to_vec());
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// A terminal session wrapping WezTerm's terminal emulator.
///
/// This struct provides the terminal state management using wezterm-term's
/// battle-tested implementation, while allowing our custom egui-based rendering.
pub struct Session {
    /// The WezTerm terminal emulator
    terminal: Arc<Mutex<Terminal>>,
    /// Session ID
    id: usize,
    /// Receiver for terminal notifications (OSC 9, OSC 777, etc.)
    notification_rx: mpsc::Receiver<Alert>,
    /// Receiver for terminal responses (OSC replies that need to go back to PTY)
    response_rx: mpsc::Receiver<Vec<u8>>,
}

impl Session {
    /// Create a new session with the given dimensions and color palette.
    pub fn new(id: usize, cols: usize, rows: usize, palette: ColorPalette) -> Self {
        let size = TerminalSize {
            rows,
            cols,
            pixel_width: cols * 8,
            pixel_height: rows * 16,
            dpi: 96,
        };

        let config = Arc::new(AgentDeckTermConfig::new(palette));

        // Use PtyForwardWriter to capture terminal responses (e.g., OSC 11 replies)
        let (response_tx, response_rx) = mpsc::channel();
        let writer = Box::new(PtyForwardWriter { tx: response_tx });

        let mut terminal = Terminal::new(
            size,
            config,
            "AgentDeck",
            env!("CARGO_PKG_VERSION"),
            writer,
        );

        // Set up notification handler for OSC 9/777 alerts
        let (notification_tx, notification_rx) = mpsc::channel();
        terminal.set_notification_handler(Box::new(NotificationHandler::new(notification_tx)));

        Self {
            terminal: Arc::new(Mutex::new(terminal)),
            id,
            notification_rx,
            response_rx,
        }
    }

    /// Get the session ID
    pub fn id(&self) -> usize {
        self.id
    }

    /// Process bytes from PTY output
    pub fn advance_bytes(&self, data: &[u8]) {
        let mut term = self.terminal.lock();
        term.advance_bytes(data);
    }

    /// Get cursor position
    pub fn cursor_pos(&self) -> CursorPosition {
        let term = self.terminal.lock();
        term.cursor_pos()
    }

    /// Get the number of physical rows
    pub fn physical_rows(&self) -> usize {
        let term = self.terminal.lock();
        term.screen().physical_rows
    }

    /// Get the number of physical columns
    pub fn physical_cols(&self) -> usize {
        let term = self.terminal.lock();
        term.screen().physical_cols
    }

    /// Access the terminal for rendering.
    /// The callback receives a reference to the locked terminal.
    pub fn with_terminal<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&Terminal) -> R,
    {
        let term = self.terminal.lock();
        f(&term)
    }

    /// Access the terminal mutably.
    pub fn with_terminal_mut<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut Terminal) -> R,
    {
        let mut term = self.terminal.lock();
        f(&mut term)
    }

    /// Resize the terminal
    pub fn resize(&self, cols: usize, rows: usize) {
        let size = TerminalSize {
            rows,
            cols,
            pixel_width: cols * 8,
            pixel_height: rows * 16,
            dpi: 96,
        };

        let mut term = self.terminal.lock();
        term.resize(size);
    }

    /// Poll for pending notifications (non-blocking)
    ///
    /// Returns all alerts that have been received since the last poll.
    /// This includes OSC 9 (iTerm2), OSC 777 (rxvt), bell, and progress updates.
    pub fn poll_notifications(&self) -> Vec<Alert> {
        let mut alerts = Vec::new();
        while let Ok(alert) = self.notification_rx.try_recv() {
            alerts.push(alert);
        }
        alerts
    }

    /// Scan the visible terminal content for a spinner task line.
    ///
    /// Claude Code renders a status line with a braille spinner character
    /// followed by the current task text (e.g., "⠋ Reading src/main.rs").
    /// This scans visible rows from the cursor position upward looking for it.
    pub fn find_spinner_task(&self) -> Option<String> {
        let mut term = self.terminal.lock();
        let cursor_row = term.cursor_pos().y as usize;
        let screen = term.screen_mut();
        let total_lines = screen.scrollback_rows();
        let physical_rows = screen.physical_rows;

        let visible_start = total_lines.saturating_sub(physical_rows);
        let start_phys = visible_start + cursor_row;

        // Claude Code renders a thinking/working status line prefixed with a
        // rotating dingbat star character (U+2726–U+2748): ✦✧✱✲✳✴✵✶✷✸✹✺✻✼✽✾✿❀❁❂❃❄❅❆❇❈
        // e.g., "✶ Slithering…", "✻ Pondering…"
        // This is several rows above the cursor (status bar, separators in between).
        for offset in 0..15 {
            let phys_idx = start_phys.saturating_sub(offset);
            if phys_idx >= total_lines {
                continue;
            }

            let line = screen.line_mut(phys_idx);
            let mut line_text = String::new();
            for cell in line.visible_cells() {
                line_text.push_str(cell.str());
            }
            let line_text = line_text.trim_end();

            if let Some(first_char) = line_text.chars().next() {
                // Diagnostic: log non-ASCII first chars to find spinner codepoint
                if !first_char.is_ascii() && (first_char as u32) > 0x2000 {
                    tracing::debug!(
                        "scan: offset={} U+{:04X} text={:.80}",
                        offset, first_char as u32, line_text
                    );
                }
                if ('\u{2726}'..='\u{2748}').contains(&first_char) {
                    let task = line_text
                        .trim_start_matches(|c: char| !c.is_alphanumeric())
                        .trim();
                    if task.is_empty() || is_duration_summary(task) {
                        continue;
                    }
                    return Some(strip_keybinding_hints(task));
                }
            }
        }

        None
    }

    /// Poll for terminal responses that need to be forwarded to the PTY.
    /// These are generated by the terminal emulator in response to queries
    /// (e.g., OSC 11 background color query).
    pub fn poll_responses(&self) -> Vec<Vec<u8>> {
        let mut responses = Vec::new();
        while let Ok(data) = self.response_rx.try_recv() {
            responses.push(data);
        }
        responses
    }
}

impl Default for Session {
    fn default() -> Self {
        Self::new(0, 120, 50, ColorPalette::default())
    }
}

/// Check if text is a duration summary line like "Worked for 40s", "Churned for 2m".
/// Matches pattern: "<word> for <digits><time-suffix>" where suffix is s/m/ms/min/sec.
fn is_duration_summary(s: &str) -> bool {
    let mut parts = s.splitn(3, ' ');
    let _verb = match parts.next() {
        Some(w) if w.chars().all(|c| c.is_alphabetic()) => w,
        _ => return false,
    };
    if parts.next() != Some("for") {
        return false;
    }
    match parts.next() {
        Some(rest) => {
            // Take only the first token (e.g., "40s" from "40s · 531 tokens")
            let token = rest.split_whitespace().next().unwrap_or(rest);
            // Must be digits followed by a time suffix only (not "3 patterns")
            let digit_end = token.find(|c: char| !c.is_ascii_digit()).unwrap_or(token.len());
            digit_end > 0 && matches!(&token[digit_end..], "s" | "m" | "ms" | "min" | "sec")
        }
        None => false,
    }
}

/// Removes things like "(ctrl+o to expand)", "(esc to interrupt)"
/// but keeps other parenthesized content like "(3 files)".
fn strip_keybinding_hints(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '(' {
            // Collect the parenthesized content
            let mut paren_content = String::new();
            let mut depth = 1;
            for inner in chars.by_ref() {
                if inner == '(' {
                    depth += 1;
                } else if inner == ')' {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                paren_content.push(inner);
            }

            let lower = paren_content.to_lowercase();
            let is_keybinding = lower.contains("ctrl+")
                || lower.contains("alt+")
                || lower.contains("cmd+")
                || lower.contains("shift+")
                || lower.starts_with("esc ")
                || lower == "esc";

            if !is_keybinding {
                result.push('(');
                result.push_str(&paren_content);
                result.push(')');
            }
        } else {
            result.push(c);
        }
    }

    result.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_creation() {
        let session = Session::new(1, 80, 24, ColorPalette::default());
        assert_eq!(session.id(), 1);
        assert_eq!(session.physical_rows(), 24);
        assert_eq!(session.physical_cols(), 80);
    }

    #[test]
    fn test_session_advance_bytes() {
        let session = Session::new(1, 80, 24, ColorPalette::default());
        session.advance_bytes(b"Hello, world!\r\n");
        // The text should be in the terminal now
        session.with_terminal(|term| {
            let screen = term.screen();
            let line_idx = screen.phys_row(0);
            // Line should contain the text
            assert!(line_idx < screen.scrollback_rows());
        });
    }

    #[test]
    fn test_bell_notification() {
        let session = Session::new(1, 80, 24, ColorPalette::default());

        // Send a standalone bell character
        session.advance_bytes(b"\x07");

        // Poll for notifications
        let alerts = session.poll_notifications();

        // Should have received a Bell alert
        assert!(!alerts.is_empty(), "Expected bell notification but got none");
        assert!(
            alerts.iter().any(|a| matches!(a, Alert::Bell)),
            "Expected Alert::Bell but got: {:?}",
            alerts
        );
    }

    #[test]
    fn test_strip_keybinding_hints() {
        // Strips ctrl+ hints
        assert_eq!(
            strip_keybinding_hints("Searching for 2 patterns, reading 3 files… (ctrl+o to expand)"),
            "Searching for 2 patterns, reading 3 files…"
        );
        // Strips esc hints
        assert_eq!(
            strip_keybinding_hints("Running command (esc to cancel)"),
            "Running command"
        );
        // Keeps useful parenthesized info
        assert_eq!(
            strip_keybinding_hints("Building project (3 files)"),
            "Building project (3 files)"
        );
        // No parens at all
        assert_eq!(
            strip_keybinding_hints("Slithering…"),
            "Slithering…"
        );
    }
}
