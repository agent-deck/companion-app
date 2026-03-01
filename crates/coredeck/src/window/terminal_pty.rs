//! PTY I/O methods (input routing and output processing) for TerminalWindowState

use super::terminal::TerminalWindowState;
use crate::core::sessions::SessionId;
use tracing::debug;

impl TerminalWindowState {
    /// Send input bytes to the active session's PTY
    pub(super) fn send_to_pty(&self, data: &[u8]) {
        if let Some(session) = self.session_manager.active_session() {
            if let Some(ref tx) = session.pty_input_tx {
                let _ = tx.send(data.to_vec());
            }
        }
    }

    /// Send input bytes to the active session's PTY (public wrapper)
    pub fn send_to_active_pty(&self, data: &[u8]) {
        self.send_to_pty(data);
    }

    /// Send input bytes to a specific session's PTY
    pub fn send_to_session_pty(&self, session_id: SessionId, data: &[u8]) {
        if let Some(session) = self.session_manager.get_session(session_id) {
            if let Some(ref tx) = session.pty_input_tx {
                match tx.send(data.to_vec()) {
                    Ok(()) => debug!("PTY input sent to session {}: {} bytes", session_id, data.len()),
                    Err(e) => tracing::warn!("PTY input send failed for session {}: {}", session_id, e),
                }
            } else {
                tracing::warn!("No PTY input tx for session {}", session_id);
            }
        } else {
            tracing::warn!("Session {} not found for PTY send", session_id);
        }
    }

    /// Process PTY output for a specific session
    pub fn process_output_for_session(&self, session_id: SessionId, data: &[u8]) {
        if let Some(session_info) = self.session_manager.get_session(session_id) {
            debug!("PTY output for session {}: {} bytes", session_id, data.len());
            let session = session_info.session.lock();
            session.advance_bytes(data);
        }
    }

    /// Process PTY output for active session (legacy compatibility)
    pub fn process_output(&self, data: &[u8]) {
        if let Some(session_info) = self.session_manager.active_session() {
            debug!("PTY output: {} bytes", data.len());
            let session = session_info.session.lock();
            session.advance_bytes(data);
        }
    }
}
