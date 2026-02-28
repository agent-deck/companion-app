//! Notification processing and terminal response forwarding for TerminalWindowState

use super::terminal::{TerminalAction, TerminalWindowState};
use crate::core::sessions::{ClaudeActivity, SessionId};
use tracing::{debug, info};
use wezterm_term::Alert;

impl TerminalWindowState {
    /// Forward terminal responses (e.g., OSC 11 bg color replies) to PTY input.
    /// Call this periodically to ensure programs get responses to their queries.
    pub fn process_terminal_responses(&self) {
        for session_info in self.session_manager.iter() {
            let session = session_info.session.lock();
            let responses = session.poll_responses();
            if !responses.is_empty() {
                if let Some(ref tx) = session_info.pty_input_tx {
                    for response in responses {
                        let _ = tx.send(response);
                    }
                }
            }
        }
    }

    pub fn process_notifications(&mut self) {
        // Collect (session_id, activity, cleaned_title) tuples from title changes.
        // Title is None when the cleaned title is "Claude Code" (activity still tracked).
        let mut title_activity_changes: Vec<(SessionId, ClaudeActivity, Option<String>)> =
            Vec::new();
        let mut bell_sessions: Vec<SessionId> = Vec::new();
        let mut attention_sessions: Vec<(SessionId, String)> = Vec::new(); // (id, body) for toast notifications
        let active_session_id = self.session_manager.active_session_id();

        for session_info in self.session_manager.iter() {
            let session = session_info.session.lock();
            let alerts: Vec<Alert> = session.poll_notifications();
            for alert in alerts {
                match alert {
                    Alert::ToastNotification {
                        title,
                        body,
                        focus,
                    } => {
                        info!(
                            "Terminal notification: title={:?}, body={}, focus={}",
                            title, body, focus
                        );
                        // Treat toast notifications as attention requests when user can't see them
                        // (background tab or window not focused)
                        if Some(session_info.id) != active_session_id || !self.window_focused {
                            attention_sessions.push((session_info.id, body.clone()));
                        }
                    }
                    Alert::Bell => {
                        debug!(
                            "Terminal bell for session {} (active={:?})",
                            session_info.id, active_session_id
                        );
                        if Some(session_info.id) != active_session_id {
                            debug!(
                                "Adding bell indicator for background session {}",
                                session_info.id
                            );
                            bell_sessions.push(session_info.id);
                        }
                    }
                    Alert::CurrentWorkingDirectoryChanged => {
                        debug!("Working directory changed");
                    }
                    Alert::WindowTitleChanged(title) => {
                        debug!(
                            "Window title changed for session {}: {}",
                            session_info.id, title
                        );
                        let activity = ClaudeActivity::from_title(&title);
                        let clean = clean_terminal_title(&title);
                        if !clean.is_empty() {
                            // Pass None for title when it's just "Claude Code" (still track activity)
                            let display_title = if clean == "Claude Code" {
                                None
                            } else {
                                Some(clean)
                            };
                            title_activity_changes.push((
                                session_info.id,
                                activity,
                                display_title,
                            ));
                        }
                    }
                    Alert::IconTitleChanged(title) => {
                        debug!("Icon title changed: {:?}", title);
                    }
                    Alert::TabTitleChanged(title) => {
                        debug!(
                            "Tab title changed for session {}: {:?}",
                            session_info.id, title
                        );
                        if let Some(t) = title {
                            let activity = ClaudeActivity::from_title(&t);
                            let clean = clean_terminal_title(&t);
                            if !clean.is_empty() {
                                let display_title = if clean == "Claude Code" {
                                    None
                                } else {
                                    Some(clean)
                                };
                                title_activity_changes.push((
                                    session_info.id,
                                    activity,
                                    display_title,
                                ));
                            }
                        }
                    }
                    Alert::SetUserVar { name, value } => {
                        debug!("User var set: {}={}", name, value);
                    }
                    Alert::OutputSinceFocusLost => {
                        debug!("Output since focus lost");
                    }
                    _ => {
                        debug!("Other alert received");
                    }
                }
            }
        }

        let mut sessions_needing_resolution: Vec<SessionId> = Vec::new();
        let mut hid_needs_update = false;
        let active_session_id = self.session_manager.active_session_id();
        for (session_id, activity, clean_title) in title_activity_changes {
            if let Some(session_info) = self.session_manager.get_session_mut(session_id) {
                // Track activity transitions for background notification
                let was_working = session_info.claude_activity.is_working();
                let is_background = Some(session_id) != active_session_id;

                // Check what actually changed before updating state
                let activity_changed = session_info.claude_activity != activity;
                let title_changed = match &clean_title {
                    Some(t) => session_info.terminal_title.as_ref() != Some(t),
                    None => false,
                };

                // Route title based on activity:
                // - Title text is always the session name (both working and idle)
                // - Task comes from the terminal screen content (spinner status line)
                // - None means "Claude Code" default -- don't overwrite existing title
                if let Some(title) = clean_title {
                    session_info.terminal_title = Some(title);
                }

                match activity {
                    ClaudeActivity::Working => {
                        // Scan terminal content for the spinner task line.
                        // If not found (mid-redraw), keep previous task.
                        let task = {
                            let session = session_info.session.lock();
                            session.find_spinner_task()
                        };
                        debug!("Session {}: working, task = {:?}", session_id, task);
                        if task.is_some() {
                            session_info.current_task = task;
                        }
                    }
                    _ => {
                        if session_info.current_task.is_some() {
                            debug!("Session {}: idle, clearing task", session_id);
                        }
                        session_info.current_task = None;
                    }
                }

                // Detect finished-in-background
                if was_working && !activity.is_working() && is_background {
                    session_info.finished_in_background = true;
                }

                session_info.claude_activity = activity;

                // Only trigger HID update when something actually changed
                if activity_changed || title_changed {
                    hid_needs_update = true;
                }

                if session_info.needs_session_resolution {
                    sessions_needing_resolution.push(session_id);
                }
            }
        }

        for session_id in sessions_needing_resolution {
            self.try_resolve_session_id(session_id);
        }

        crate::update_working_session_count(self.session_manager.working_session_count());

        for session_id in bell_sessions {
            // Compute HID tab index before mutable borrow
            let tab_idx = self.session_manager.session_hid_tab_index(session_id);
            if let Some(session_info) = self.session_manager.get_session_mut(session_id) {
                if !session_info.hid_alert_active {
                    if let Some(idx) = tab_idx {
                        let session_name = session_info.hid_session_name().to_string();
                        let details = {
                            let session = session_info.session.lock();
                            session.extract_prompt_context()
                        };
                        self.alert_order_counter += 1;
                        session_info.alert_order = self.alert_order_counter;
                        session_info.hid_alert_active = true;
                        session_info.hid_alert_text = Some("Bell".to_string());
                        session_info.hid_alert_details = details.clone();
                        self.pending_actions.push(TerminalAction::HidAlert {
                            tab: idx,
                            session: session_name,
                            text: "Bell".to_string(),
                            details,
                        });
                    }
                }
            }
        }

        // Toast notifications (OSC 9) -- Claude asking for permission
        for (session_id, body) in attention_sessions {
            let tab_idx = self.session_manager.session_hid_tab_index(session_id);
            if let Some(session_info) = self.session_manager.get_session_mut(session_id) {
                if !session_info.hid_alert_active {
                    if let Some(idx) = tab_idx {
                        let session_name = session_info.hid_session_name().to_string();
                        let details = {
                            let session = session_info.session.lock();
                            session.extract_prompt_context()
                        };
                        self.alert_order_counter += 1;
                        session_info.alert_order = self.alert_order_counter;
                        session_info.hid_alert_active = true;
                        session_info.hid_alert_text = Some(body.clone());
                        session_info.hid_alert_details = details.clone();
                        self.pending_actions.push(TerminalAction::HidAlert {
                            tab: idx,
                            session: session_name,
                            text: body,
                            details,
                        });
                    }
                }
            }
        }

        // Periodically rescan task for active working session, even without title changes.
        // The spinner task line on screen can change independently of OSC title updates.
        if !hid_needs_update {
            if let Some(session_info) = self.session_manager.active_session_mut() {
                if session_info.claude_activity.is_working() {
                    let task = {
                        let session = session_info.session.lock();
                        session.find_spinner_task()
                    };
                    if task.is_some() && task != session_info.current_task {
                        session_info.current_task = task;
                        hid_needs_update = true;
                    }
                }
            }
        }

        // Detect Claude Code mode changes for active session
        if let Some(session_info) = self.session_manager.active_session() {
            let mode = {
                let session = session_info.session.lock();
                session.detect_claude_mode()
            };
            if mode != self.detected_mode {
                self.detected_mode = mode;
                self.pending_actions
                    .push(TerminalAction::HidSetMode(mode));
            }
        }
        // Clear stale mode suppression (safety net in case confirmation never arrives)
        if let Some(sent_at) = self.mode_set_from_app_at {
            if sent_at.elapsed() > std::time::Duration::from_secs(2) {
                self.mode_set_from_app_at = None;
            }
        }

        // Push HID display update if any tab's state changed
        if hid_needs_update {
            if let Some(session_info) = self.session_manager.active_session() {
                let session_name = session_info.hid_session_name().to_string();
                let (task, task2) = match &session_info.current_task {
                    Some(t) => {
                        let (line1, line2) = crate::core::text_compact::split_task_lines(t);
                        (Some(line1), line2)
                    }
                    None => (None, None),
                };
                let (tabs, active) = self.session_manager.collect_tab_states();
                self.pending_actions.push(TerminalAction::HidDisplayUpdate {
                    session: session_name,
                    task,
                    task2,
                    tabs,
                    active,
                });
            }
        }
    }
}

/// Clean terminal title by removing leading symbols/emojis
fn clean_terminal_title(title: &str) -> String {
    title
        .trim_start_matches(|c: char| !c.is_alphanumeric())
        .trim()
        .to_string()
}
