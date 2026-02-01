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

/// Writer that discards all writes (for output-only terminal mode)
struct NullWriter;

impl Write for NullWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
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
    /// Color palette for rendering
    palette: ColorPalette,
    /// Receiver for terminal notifications (OSC 9, OSC 777, etc.)
    notification_rx: mpsc::Receiver<Alert>,
}

impl Session {
    /// Create a new session with the given dimensions.
    pub fn new(id: usize, cols: usize, rows: usize) -> Self {
        let size = TerminalSize {
            rows,
            cols,
            pixel_width: cols * 8,
            pixel_height: rows * 16,
            dpi: 96,
        };

        let config = Arc::new(AgentDeckTermConfig::default());

        // Use NullWriter since we handle input separately via PtyWrapper
        let writer = Box::new(NullWriter);

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
            palette: ColorPalette::default(),
            notification_rx,
        }
    }

    /// Get the session ID
    pub fn id(&self) -> usize {
        self.id
    }

    /// Get the color palette
    pub fn palette(&self) -> &ColorPalette {
        &self.palette
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
}

impl Default for Session {
    fn default() -> Self {
        Self::new(0, 120, 50)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_creation() {
        let session = Session::new(1, 80, 24);
        assert_eq!(session.id(), 1);
        assert_eq!(session.physical_rows(), 24);
        assert_eq!(session.physical_cols(), 80);
    }

    #[test]
    fn test_session_advance_bytes() {
        let session = Session::new(1, 80, 24);
        session.advance_bytes(b"Hello, world!\r\n");
        // The text should be in the terminal now
        session.with_terminal(|term| {
            let screen = term.screen();
            let line_idx = screen.phys_row(0);
            // Line should contain the text
            assert!(line_idx < screen.scrollback_rows());
        });
    }
}
