//! PTY wrapper for spawning and managing Claude CLI

use crate::core::claude_sessions::get_session_count;
use crate::core::config::ClaudeConfig;
use crate::core::events::AppEvent;
use crate::core::sessions::SessionId;
use crate::core::state::ClaudeState;
use super::claude_state::ClaudeStateExtractor;
use anyhow::{Context, Result};
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;
use parking_lot::Mutex;
use tracing::{debug, error, info, warn};

/// PTY wrapper for Claude CLI
pub struct PtyWrapper {
    /// PTY master handle
    master: Arc<Mutex<Option<Box<dyn MasterPty + Send>>>>,
    /// Writer to PTY
    writer: Arc<Mutex<Option<Box<dyn Write + Send>>>>,
    /// State extractor
    extractor: Arc<Mutex<ClaudeStateExtractor>>,
    /// Event sender
    event_tx: mpsc::UnboundedSender<AppEvent>,
    /// Configuration
    config: ClaudeConfig,
    /// Whether process is running
    running: Arc<Mutex<bool>>,
    /// Working directory (optional, for per-session PTYs)
    working_directory: Option<PathBuf>,
    /// Session ID (optional, for per-session events)
    session_id: Option<SessionId>,
    /// Session to resume:
    /// - None = --continue (auto-continue most recent session)
    /// - Some("") = fresh start (no flags, explicit "New Session")
    /// - Some(id) = --resume {id} or --session-id {id} depending on is_new_session
    resume_session: Option<String>,
    /// Whether this is a brand new session (use --session-id) vs resuming (use --resume)
    is_new_session: bool,
    /// COLORFGBG value to set in environment (for background color detection)
    colorfgbg: Option<String>,
}

impl PtyWrapper {
    /// Create a new PTY wrapper
    pub fn new(config: ClaudeConfig, event_tx: mpsc::UnboundedSender<AppEvent>) -> Self {
        Self {
            master: Arc::new(Mutex::new(None)),
            writer: Arc::new(Mutex::new(None)),
            extractor: Arc::new(Mutex::new(ClaudeStateExtractor::new())),
            event_tx,
            config,
            running: Arc::new(Mutex::new(false)),
            working_directory: None,
            session_id: None,
            resume_session: None,
            is_new_session: false,
            colorfgbg: None,
        }
    }

    /// Create a new PTY wrapper with a specific working directory and session ID
    ///
    /// # Arguments
    /// * `resume_session` - None = --continue, Some("") = fresh, Some(id) = session ID
    /// * `is_new_session` - true = use --session-id (creating new), false = use --resume (resuming existing)
    /// * `colorfgbg` - COLORFGBG env var value for background color detection
    pub fn new_with_cwd(
        config: ClaudeConfig,
        event_tx: mpsc::UnboundedSender<AppEvent>,
        working_directory: PathBuf,
        session_id: SessionId,
        resume_session: Option<String>,
        is_new_session: bool,
        colorfgbg: Option<String>,
    ) -> Self {
        Self {
            master: Arc::new(Mutex::new(None)),
            writer: Arc::new(Mutex::new(None)),
            extractor: Arc::new(Mutex::new(ClaudeStateExtractor::new())),
            event_tx,
            config,
            running: Arc::new(Mutex::new(false)),
            working_directory: Some(working_directory),
            session_id: Some(session_id),
            resume_session,
            is_new_session,
            colorfgbg,
        }
    }

    /// Start Claude CLI in a PTY
    pub fn start(&self) -> Result<()> {
        let pty_system = native_pty_system();

        // Create PTY with size matching terminal surface (will be resized when window opens)
        let pair = pty_system
            .openpty(PtySize {
                rows: 50,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("Failed to create PTY")?;

        // Build command
        let cli_path = if self.config.cli_path.is_empty() {
            "claude".to_string()
        } else {
            self.config.cli_path.clone()
        };

        let mut cmd = CommandBuilder::new(&cli_path);
        for arg in &self.config.default_args {
            cmd.arg(arg);
        }

        // Handle session:
        // - None = --continue (auto-continue most recent, if sessions exist)
        // - Some(id) with non-empty id = --resume {id} or --session-id {id} based on is_new_session
        // - Some("") = fresh start, no flags
        match &self.resume_session {
            None => {
                // Auto-continue most recent session, but only if sessions exist
                // (claude --continue fails if no conversations exist)
                let has_sessions = self.working_directory
                    .as_ref()
                    .map(|dir| get_session_count(dir) > 0)
                    .unwrap_or(false);
                if has_sessions {
                    cmd.arg("--continue");
                }
            }
            Some(id) if !id.is_empty() => {
                if self.is_new_session {
                    // Brand new session - use --session-id to create with specific ID
                    cmd.arg("--session-id");
                    cmd.arg(id);
                    info!("Creating new Claude session with ID: {}", id);
                } else {
                    // Resuming existing session
                    cmd.arg("--resume");
                    cmd.arg(id);
                    info!("Resuming Claude session: {}", id);
                }
            }
            Some(_) => {
                // Empty string = fresh start, no flags
            }
        }

        // Use provided working directory or fall back to current
        if let Some(ref cwd) = self.working_directory {
            cmd.cwd(cwd);
        } else if let Ok(cwd) = std::env::current_dir() {
            cmd.cwd(cwd);
        }

        // Set TERM for color support
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");

        // Set COLORFGBG for background color detection (used by programs like Claude Code)
        if let Some(ref colorfgbg) = self.colorfgbg {
            cmd.env("COLORFGBG", colorfgbg);
        }

        // On macOS, apps launched from Finder don't inherit the shell's PATH.
        // Add common installation directories to ensure we can find claude and other tools.
        #[cfg(target_os = "macos")]
        {
            let current_path = std::env::var("PATH").unwrap_or_default();
            let home = std::env::var("HOME").unwrap_or_default();
            let mut paths = vec![
                "/opt/homebrew/bin".to_string(),
                "/opt/homebrew/sbin".to_string(),
                "/usr/local/bin".to_string(),
                "/usr/local/sbin".to_string(),
                // Common npm global paths
                format!("{}/.npm-global/bin", home),
                // Claude Code local install
                format!("{}/.claude/local", home),
            ];
            if !current_path.is_empty() {
                paths.push(current_path);
            }
            cmd.env("PATH", paths.join(":"));
        }

        info!("Starting Claude CLI: {} {:?}", cli_path, self.config.default_args);

        // Spawn the child process
        let child = pair
            .slave
            .spawn_command(cmd)
            .context("Failed to spawn Claude CLI")?;

        // Get writer for sending input
        let writer = pair
            .master
            .take_writer()
            .context("Failed to get PTY writer")?;

        // Store handles
        *self.master.lock() = Some(pair.master);
        *self.writer.lock() = Some(writer);
        *self.running.lock() = true;

        // Start reader task
        self.start_reader_task(child);

        Ok(())
    }

    /// Start background task to read PTY output
    fn start_reader_task(&self, mut child: Box<dyn portable_pty::Child + Send + Sync>) {
        let master = Arc::clone(&self.master);
        let extractor = Arc::clone(&self.extractor);
        let event_tx = self.event_tx.clone();
        let running = Arc::clone(&self.running);
        let session_id = self.session_id;

        std::thread::spawn(move || {
            let mut buffer = [0u8; 4096];
            // Keep recent output for error logging (last 4KB)
            let mut recent_output: Vec<u8> = Vec::with_capacity(4096);
            const MAX_RECENT_OUTPUT: usize = 4096;

            // Get reader from master
            let reader_result = {
                let master_guard = master.lock();
                master_guard.as_ref().map(|m| m.try_clone_reader())
            };

            let mut reader = match reader_result {
                Some(Ok(r)) => r,
                Some(Err(e)) => {
                    error!("Failed to get PTY reader: {}", e);
                    return;
                }
                None => {
                    error!("No PTY master available");
                    return;
                }
            };

            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => {
                        // EOF - process exited
                        debug!("PTY EOF");
                        break;
                    }
                    Ok(n) => {
                        let data = &buffer[..n];

                        // Keep recent output for error logging
                        recent_output.extend_from_slice(data);
                        if recent_output.len() > MAX_RECENT_OUTPUT {
                            let excess = recent_output.len() - MAX_RECENT_OUTPUT;
                            recent_output.drain(..excess);
                        }

                        // Send raw output (use session-specific event if we have a session ID)
                        if let Some(sid) = session_id {
                            let _ = event_tx.send(AppEvent::PtyOutputForSession {
                                session_id: sid,
                                data: data.to_vec(),
                            });
                        } else {
                            let _ = event_tx.send(AppEvent::PtyOutput(data.to_vec()));
                        }

                        // Extract state
                        let mut ext = extractor.lock();
                        if let Some(state) = ext.process(data) {
                            let _ = event_tx.send(AppEvent::ClaudeStateChanged(state));
                        }
                    }
                    Err(e) => {
                        if e.kind() != std::io::ErrorKind::Interrupted {
                            warn!("PTY read error: {}", e);
                            break;
                        }
                    }
                }
            }

            // Wait for child to exit
            let exit_code: Option<i32> = match child.wait() {
                Ok(status) => {
                    info!("Claude CLI exited with status: {:?}", status);
                    // Log recent output if exit code indicates an error
                    if status.exit_code() != 0 && !recent_output.is_empty() {
                        // Strip ANSI escape codes for cleaner logging
                        let output_str = String::from_utf8_lossy(&recent_output);
                        let clean_output = strip_ansi_codes(&output_str);
                        error!(
                            "Claude CLI failed (exit code {}). Last output:\n{}",
                            status.exit_code(),
                            clean_output.trim()
                        );
                    }
                    Some(status.exit_code() as i32)
                }
                Err(e) => {
                    error!("Failed to wait for Claude CLI: {}", e);
                    None
                }
            };

            *running.lock() = false;

            // Send appropriate exit event
            if let Some(sid) = session_id {
                let _ = event_tx.send(AppEvent::PtyExitedForSession {
                    session_id: sid,
                    code: exit_code,
                });
            } else {
                let _ = event_tx.send(AppEvent::PtyExited(exit_code));
            }
        });
    }

    /// Send input to the PTY
    pub fn send_input(&self, data: &[u8]) -> Result<()> {
        let mut writer_guard = self.writer.lock();
        let writer = writer_guard
            .as_mut()
            .context("PTY not running")?;

        writer.write_all(data)?;
        writer.flush()?;

        Ok(())
    }

    /// Send a key press (e.g., Enter, Ctrl+C)
    pub fn send_key(&self, key: &str) -> Result<()> {
        let bytes = match key {
            "enter" => b"\r".as_slice(),
            "ctrl-c" => b"\x03".as_slice(),
            "ctrl-d" => b"\x04".as_slice(),
            "escape" => b"\x1b".as_slice(),
            "tab" => b"\t".as_slice(),
            _ => key.as_bytes(),
        };

        self.send_input(bytes)
    }

    /// Check if process is running
    pub fn is_running(&self) -> bool {
        *self.running.lock()
    }

    /// Get current Claude state
    pub fn state(&self) -> ClaudeState {
        self.extractor.lock().state().clone()
    }

    /// Stop the PTY process
    pub fn stop(&self) -> Result<()> {
        if !self.is_running() {
            return Ok(());
        }

        // Send Ctrl+C to gracefully stop
        if let Err(e) = self.send_key("ctrl-c") {
            warn!("Failed to send Ctrl+C: {}", e);
        }

        // Give it a moment to exit
        std::thread::sleep(std::time::Duration::from_millis(500));

        // If still running, send Ctrl+D
        if self.is_running() {
            if let Err(e) = self.send_key("ctrl-d") {
                warn!("Failed to send Ctrl+D: {}", e);
            }
        }

        Ok(())
    }

    /// Resize the PTY
    pub fn resize(&self, rows: u16, cols: u16) -> Result<()> {
        let master_guard = self.master.lock();
        if let Some(ref master) = *master_guard {
            master
                .resize(PtySize {
                    rows,
                    cols,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .context("Failed to resize PTY")?;
        }
        Ok(())
    }
}

impl Drop for PtyWrapper {
    fn drop(&mut self) {
        if self.is_running() {
            let _ = self.stop();
        }
    }
}

/// Strip ANSI escape codes from a string for cleaner logging
fn strip_ansi_codes(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Skip escape sequence
            if chars.peek() == Some(&'[') {
                chars.next(); // consume '['
                // Skip until we hit a letter (end of CSI sequence)
                while let Some(&next) = chars.peek() {
                    chars.next();
                    if next.is_ascii_alphabetic() {
                        break;
                    }
                }
            } else if chars.peek() == Some(&']') {
                // OSC sequence - skip until BEL or ST
                chars.next(); // consume ']'
                while let Some(&next) = chars.peek() {
                    if next == '\x07' {
                        chars.next();
                        break;
                    }
                    if next == '\x1b' {
                        chars.next();
                        if chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                    chars.next();
                }
            }
        } else {
            result.push(c);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pty_wrapper_creation() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let config = ClaudeConfig::default();
        let wrapper = PtyWrapper::new(config, tx);

        assert!(!wrapper.is_running());
    }
}
