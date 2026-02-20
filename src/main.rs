// Hide console window on Windows release builds
#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

//! Agent Deck Companion App - Entry Point
//!
//! This is the main entry point for the Agent Deck companion application.
//! It initializes all modules and runs the main event loop.

use agent_deck::{
    core::{
        config::Config,
        events::{AppEvent, EventSender},
        sessions::SessionId,
        state::AppState,
        tabs::{TabEntry, TabState},
    },
    hid::{keycodes::{qmk_keycode_to_egui_key, qmk_keycode_to_terminal_bytes}, HidManager, SoftKeyEditState},
    pty::{resolve_login_env, PtyWrapper},
    tray::TrayManager,
    window::{TerminalAction, TerminalWindowState},
};
#[cfg(target_os = "macos")]
use agent_deck::macos::{create_menu_bar, init_menu_sender, update_recent_sessions_menu, MenuAction};
use anyhow::Result;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    window::WindowId,
};

use agent_deck::tray;

/// Per-session PTY state
struct SessionPty {
    pty: Arc<PtyWrapper>,
    _input_thread: std::thread::JoinHandle<()>,
}

/// Main application handler for winit event loop
struct App {
    /// Application state
    state: Arc<RwLock<AppState>>,
    /// Event sender for inter-module communication (wakes event loop)
    event_tx: EventSender,
    /// Event receiver for inter-module communication
    event_rx: Option<mpsc::UnboundedReceiver<AppEvent>>,
    /// HID manager for device communication
    hid_manager: Option<HidManager>,
    /// Tray manager for system tray
    tray_manager: Option<TrayManager>,
    /// PTY wrappers per session
    session_ptys: HashMap<SessionId, SessionPty>,
    /// Terminal window state
    terminal_window: TerminalWindowState,
    /// Configuration
    config: Config,
    /// Login shell environment captured at startup
    login_env: Arc<HashMap<String, String>>,
    /// Whether the macOS menu bar has been created
    #[cfg(target_os = "macos")]
    menu_created: bool,
    /// Last time check_claude_theme was called (throttle to once per 2s)
    last_theme_check: std::time::Instant,
}

impl App {
    fn new(
        state: Arc<RwLock<AppState>>,
        event_tx: EventSender,
        event_rx: mpsc::UnboundedReceiver<AppEvent>,
        config: Config,
        login_env: Arc<HashMap<String, String>>,
    ) -> Self {
        let font_size = config.terminal.font_size;
        Self {
            state,
            event_tx,
            event_rx: Some(event_rx),
            hid_manager: None,
            tray_manager: None,
            session_ptys: HashMap::new(),
            terminal_window: TerminalWindowState::new(font_size),
            config,
            login_env,
            #[cfg(target_os = "macos")]
            menu_created: false,
            last_theme_check: std::time::Instant::now(),
        }
    }

    /// Start Claude in PTY for a specific session
    ///
    /// # Arguments
    /// * `resume_session` - None = --continue, Some("") = fresh start, Some(id) = --resume {id}
    fn start_claude_for_session(
        &mut self,
        session_id: SessionId,
        working_directory: PathBuf,
        resume_session: Option<String>,
        event_loop: &ActiveEventLoop,
    ) {
        if self.session_ptys.contains_key(&session_id) {
            // Already running for this session
            return;
        }

        info!(
            "Starting Claude in PTY for session {} at {:?} (resume_session={:?})",
            session_id, working_directory, resume_session
        );

        // Mark session as loading
        self.terminal_window.mark_session_loading(session_id);

        // Create channel for PTY input
        let (input_tx, mut input_rx) = mpsc::unbounded_channel::<Vec<u8>>();

        // Set input sender on the session
        self.terminal_window
            .set_session_input_sender(session_id, input_tx);

        // Create terminal window if not exists
        self.terminal_window.create_window(event_loop);

        // For fresh sessions (Some("")), generate a UUID upfront.
        // This ensures we know the session ID immediately and can persist it.
        // The PTY wrapper will use --session-id <uuid> for new sessions.
        let (actual_resume_session, is_new_session, needs_session_id_resolution) = match &resume_session {
            Some(id) if id.is_empty() => {
                // Fresh session: generate UUID and use it
                let uuid = uuid::Uuid::new_v4().to_string();
                info!("Generated UUID for fresh session {}: {}", session_id, uuid);

                // Update SessionInfo with the generated UUID
                if let Some(session) = self.terminal_window.session_manager.get_session_mut(session_id) {
                    session.claude_session_id = Some(uuid.clone());
                }

                (Some(uuid), true, false) // is_new_session = true
            }
            Some(id) => {
                // Trying to resume a specific session - check if it exists
                use agent_deck::core::claude_sessions::session_exists;
                let exists = session_exists(&working_directory, id);

                if exists {
                    // Session exists - resume it
                    (Some(id.clone()), false, false) // is_new_session = false
                } else {
                    // Session was saved but never actually used (no conversation happened)
                    // Generate a new UUID so we can track this session
                    let uuid = uuid::Uuid::new_v4().to_string();
                    info!(
                        "Session {} not found on disk (never used?), generating new UUID: {}",
                        id, uuid
                    );

                    // Update SessionInfo with the new UUID
                    if let Some(session) = self.terminal_window.session_manager.get_session_mut(session_id) {
                        session.claude_session_id = Some(uuid.clone());
                    }

                    (Some(uuid), true, false) // is_new_session = true (creating new)
                }
            }
            None => {
                // Auto-continue - still need resolution to find which session was continued
                (None, false, true)
            }
        };

        // Create PTY wrapper with custom working directory
        let claude_config = self.config.claude.clone();
        let colorfgbg = Some(self.terminal_window.current_theme.colorfgbg());
        let pty = Arc::new(PtyWrapper::new_with_cwd(
            claude_config,
            self.event_tx.clone(),
            working_directory.clone(),
            session_id,
            actual_resume_session,
            is_new_session,
            colorfgbg,
            Arc::clone(&self.login_env),
        ));

        match pty.start() {
            Ok(()) => {
                info!("Claude PTY started successfully for session {}", session_id);

                // Set up resize callback
                let pty_for_resize = Arc::clone(&pty);
                self.terminal_window.set_resize_callback(move |rows, cols| {
                    if let Err(e) = pty_for_resize.resize(rows, cols) {
                        warn!("Failed to resize PTY: {}", e);
                    }
                });

                // Sync terminal size with PTY
                self.terminal_window.sync_size();

                // Show terminal window
                self.terminal_window.show();

                // Mark session as running
                self.terminal_window.mark_session_started(session_id);

                // Set up resolution tracking for auto-continue sessions (None).
                // Fresh sessions already have UUIDs assigned; explicit resumes have their IDs.
                if needs_session_id_resolution {
                    if let Some(session) = self.terminal_window.session_manager.get_session_mut(session_id) {
                        session.session_start_time = Some(std::time::Instant::now());
                        session.needs_session_resolution = true;
                    }
                }

                // Update state
                {
                    let mut state = self.state.write();
                    state.claude_running = true;
                }

                // Add to recent
                self.terminal_window
                    .bookmark_manager
                    .add_recent(working_directory);
                let _ = self.terminal_window.bookmark_manager.save();

                // Start a thread to forward PTY input
                let pty_clone = Arc::clone(&pty);
                let input_thread = std::thread::spawn(move || {
                    while let Some(data) = input_rx.blocking_recv() {
                        if let Err(e) = pty_clone.send_input(&data) {
                            warn!("Failed to send input to PTY: {}", e);
                            break;
                        }
                    }
                });

                // Store PTY wrapper
                self.session_ptys.insert(
                    session_id,
                    SessionPty {
                        pty,
                        _input_thread: input_thread,
                    },
                );
            }
            Err(e) => {
                error!("Failed to start Claude PTY for session {}: {}", session_id, e);
                // Clear loading state on failure
                if let Some(session) = self.terminal_window.session_manager.get_session_mut(session_id) {
                    session.is_loading = false;
                }
            }
        }
    }

    /// Start Claude in PTY and show terminal window (legacy, for first tab)
    fn start_claude(&mut self, event_loop: &ActiveEventLoop) {
        // Create a new tab with the default working directory
        let working_dir = if self.config.terminal.working_directory.is_empty() {
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
        } else {
            PathBuf::from(&self.config.terminal.working_directory)
        };

        // Create or get active session - collect info first to avoid borrow issues
        let session_info = self
            .terminal_window
            .session_manager
            .active_session()
            .map(|s| (s.id, s.is_new_tab()));

        let session_id = match session_info {
            Some((id, is_new_tab)) => {
                if is_new_tab {
                    // Convert new tab to running session
                    if let Some(s) = self.terminal_window.session_manager.active_session_mut() {
                        s.working_directory = working_dir.clone();
                        s.title = working_dir
                            .file_name()
                            .and_then(|n| n.to_str())
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| "New Session".to_string());
                    }
                    id
                } else if self.session_ptys.contains_key(&id) {
                    // Already running, just show window
                    self.terminal_window.show();
                    return;
                } else {
                    id
                }
            }
            None => {
                // No sessions, create one
                self.terminal_window
                    .session_manager
                    .create_session(working_dir.clone(), &self.terminal_window.current_palette)
            }
        };

        // Start fresh when started from tray/HID (user can use new tab page for session selection)
        self.start_claude_for_session(session_id, working_dir, None, event_loop);
    }

    /// Save current tabs to persistent storage
    fn save_tabs(&self) {
        // Get the active session ID to find its position after filtering
        let active_id = self.terminal_window.session_manager.active_session_id();
        let active_idx = self.terminal_window.session_manager.active_session_index();

        info!("save_tabs: active_id={:?}, active_index={}", active_id, active_idx);

        // Collect non-empty tabs with their IDs
        let tabs_with_ids: Vec<_> = self
            .terminal_window
            .session_manager
            .iter()
            .filter(|s| !s.is_new_tab()) // Don't save empty "new tab" placeholders
            .map(|s| (s.id, TabEntry {
                working_directory: s.working_directory.clone(),
                title: s.title.clone(),
                // Preserve session intent:
                // - None = auto-continue most recent session
                // - Some("") = explicit fresh start
                // - Some(id) = resume specific session
                claude_session_id: s.claude_session_id.clone(),
                terminal_title: s.terminal_title.clone(),
            }))
            .collect();

        info!("save_tabs: tabs_with_ids={:?}", tabs_with_ids.iter().map(|(id, _)| id).collect::<Vec<_>>());

        // Find the active tab index in the filtered list
        let active_tab = active_id
            .and_then(|id| tabs_with_ids.iter().position(|(tab_id, _)| *tab_id == id))
            .unwrap_or(0);

        info!("save_tabs: computed active_tab={}", active_tab);

        let tabs: Vec<TabEntry> = tabs_with_ids.into_iter().map(|(_, entry)| entry).collect();

        let tab_state = TabState {
            tabs,
            active_tab,
        };

        if let Err(e) = tab_state.save() {
            warn!("Failed to save tab state: {}", e);
        } else {
            info!("Saved {} tabs", tab_state.tabs.len());
        }
    }

    /// Save window geometry to settings
    fn save_window_geometry(&mut self) {
        if let Some(geometry) = self.terminal_window.get_window_geometry() {
            self.terminal_window.settings.window_geometry = geometry;
            if let Err(e) = self.terminal_window.settings.save() {
                error!("Failed to save window geometry: {}", e);
            }
        }
    }

    /// Load saved tabs from persistent storage
    fn load_saved_tabs(&mut self) {
        match TabState::load() {
            Ok(tab_state) if tab_state.has_tabs() => {
                info!("Loading {} saved tabs, active_tab={}", tab_state.tabs.len(), tab_state.active_tab);
                for tab in &tab_state.tabs {
                    info!("  - tab: {:?}", tab.title);
                }
                for tab in tab_state.tabs {
                    self.terminal_window
                        .session_manager
                        .create_placeholder(tab.working_directory, tab.title, tab.claude_session_id, tab.terminal_title, &self.terminal_window.current_palette);
                }
                // Set active tab (clamped to valid range)
                let active = tab_state.active_tab.min(
                    self.terminal_window.session_manager.session_count().saturating_sub(1)
                );
                info!("Setting active_session_index to {}", active);
                self.terminal_window.session_manager.set_active_session_index(active);
            }
            Ok(_) => {
                info!("No saved tabs to restore");
            }
            Err(e) => {
                warn!("Failed to load saved tabs: {}", e);
            }
        }
    }

    /// Stop Claude PTY for a specific session
    fn stop_claude_for_session(&mut self, session_id: SessionId) {
        if let Some(session_pty) = self.session_ptys.remove(&session_id) {
            if let Err(e) = session_pty.pty.stop() {
                warn!("Error stopping PTY for session {}: {}", session_id, e);
            }
        }

        // Close the session in session manager
        self.terminal_window.session_manager.close_session(session_id);

        // If no more sessions, hide window and update state
        if self.terminal_window.session_manager.is_empty() {
            self.terminal_window.hide();
            {
                let mut state = self.state.write();
                state.claude_running = false;
            }
        }

        info!("Claude stopped for session {}", session_id);
    }

    /// Send HID display update and mode for the currently active session.
    /// Called whenever the active tab changes (switch, close, new tab, etc.)
    fn send_hid_for_active_session(&mut self) {
        if let Some(session) = self.terminal_window.session_manager.active_session() {
            let session_name = session.hid_session_name().to_string();
            let current_task = session.current_task.clone();
            let mode = {
                let s = session.session.lock();
                s.detect_claude_mode()
            };
            let (tabs, active) = self.terminal_window.session_manager.collect_tab_states();
            if let Some(ref hid) = self.hid_manager {
                if let Err(e) = hid.send_display_update(&session_name, current_task.as_deref(), &tabs, active) {
                    debug!("Failed to send HID display update: {}", e);
                }
                self.terminal_window.detected_mode = mode;
                self.terminal_window.last_device_reported_mode = Some(mode);
                self.terminal_window.mode_set_from_app_at = Some(std::time::Instant::now());
                if let Err(e) = hid.set_mode(mode) {
                    debug!("Failed to set HID mode: {}", e);
                }
            }
        }
    }

    /// Clear HID alert for a specific session (called on HID input events)
    fn clear_hid_alert_for_session(&mut self, session_id: SessionId) {
        let hid_tab_idx = self.terminal_window.session_manager.session_hid_tab_index(session_id);
        if let Some(session) = self.terminal_window.session_manager.get_session_mut(session_id) {
            if session.hid_alert_active {
                session.hid_alert_active = false;
                session.hid_alert_text = None;
                session.hid_alert_details = None;
                session.bell_active = false;
                session.finished_in_background = false;
                if let Some(idx) = hid_tab_idx {
                    if let Some(ref hid) = self.hid_manager {
                        if let Err(e) = hid.clear_alert(idx) {
                            debug!("Failed to clear HID alert on input: {}", e);
                        }
                    }
                }
            }
        }
    }

    /// Handle terminal UI actions
    fn handle_terminal_action(&mut self, action: TerminalAction, event_loop: &ActiveEventLoop) {
        match action {
            TerminalAction::NewTab => {
                // Create a new tab (will show new tab page)
                let session_id = self.terminal_window.session_manager.create_new_tab(&self.terminal_window.current_palette);
                self.terminal_window
                    .session_manager
                    .set_active_session(session_id);
                self.terminal_window.update_window_title();
                self.send_hid_for_active_session();
                info!("Created new tab with session ID {}", session_id);
            }
            TerminalAction::CloseTab(session_id) => {
                // Check if Claude is actively working in this session
                if let Some(session) = self.terminal_window.session_manager.get_session(session_id) {
                    if session.claude_activity.is_working() {
                        let confirmed = rfd::MessageDialog::new()
                            .set_title("Close Tab")
                            .set_description("Claude is still working in this session. Close anyway?")
                            .set_buttons(rfd::MessageButtons::YesNo)
                            .set_level(rfd::MessageLevel::Warning)
                            .show() == rfd::MessageDialogResult::Yes;
                        if !confirmed {
                            return;
                        }
                    }
                }

                // Before closing: record the closed tab's HID index and collect
                // active alerts that will need re-indexing afterwards.
                let closed_hid_idx = self.terminal_window.session_manager.session_hid_tab_index(session_id);

                // Clear HID alert for the tab being closed
                if let Some(session) = self.terminal_window.session_manager.get_session(session_id) {
                    if session.hid_alert_active {
                        if let Some(idx) = closed_hid_idx {
                            if let Some(ref hid) = self.hid_manager {
                                let _ = hid.clear_alert(idx);
                            }
                        }
                    }
                }

                // Collect alerts whose HID index will shift down after the close.
                // We need: (session_id, old_hid_index, session_name, text, details)
                let shifted_alerts: Vec<_> = if let Some(closed_idx) = closed_hid_idx {
                    self.terminal_window.session_manager.iter()
                        .filter(|s| s.id != session_id && s.hid_alert_active)
                        .filter_map(|s| {
                            let old_idx = self.terminal_window.session_manager.session_hid_tab_index(s.id)?;
                            if old_idx > closed_idx {
                                Some((s.id, old_idx, s.hid_session_name().to_string(),
                                      s.hid_alert_text.clone(), s.hid_alert_details.clone()))
                            } else {
                                None
                            }
                        })
                        .collect()
                } else {
                    vec![]
                };

                self.stop_claude_for_session(session_id);

                // Re-send shifted alerts at their new (decremented) indices
                if let Some(ref hid) = self.hid_manager {
                    for (_, old_idx, session_name, text, details) in &shifted_alerts {
                        // Clear the alert at the old index on the device
                        let _ = hid.clear_alert(*old_idx);
                        // Re-send at the new index (old - 1)
                        let new_idx = old_idx - 1;
                        if let Some(ref text) = text {
                            let _ = hid.send_alert(new_idx, session_name, text, details.as_deref());
                        }
                    }
                }

                // Update window title after closing (active tab may have changed)
                self.terminal_window.update_window_title();
                // Save tabs after closing
                self.save_tabs();
                // Send HID update and mode for newly active tab
                self.send_hid_for_active_session();

                // Start PTY for newly active session if needed (lazy loading)
                if let Some(new_active) = self.terminal_window.session_manager.active_session() {
                    let needs_start = !new_active.is_running && !new_active.is_loading && !new_active.is_new_tab();
                    if needs_start {
                        let new_session_id = new_active.id;
                        let working_dir = new_active.working_directory.clone();
                        let claude_session_id = new_active.claude_session_id.clone();
                        self.start_claude_for_session(new_session_id, working_dir, claude_session_id, event_loop);
                    }
                }
            }
            TerminalAction::SwitchTab(session_id) => {
                // Compute HID tab index before mutable borrow
                let hid_tab_idx = self.terminal_window.session_manager.session_hid_tab_index(session_id);

                self.terminal_window
                    .session_manager
                    .set_active_session(session_id);

                // Clear bell indicator, finished-in-background, and HID alert when tab becomes active
                if let Some(session) = self.terminal_window.session_manager.get_session_mut(session_id) {
                    session.bell_active = false;
                    session.finished_in_background = false;
                    if session.hid_alert_active {
                        session.hid_alert_active = false;
                        session.hid_alert_text = None;
                        session.hid_alert_details = None;
                        if let Some(idx) = hid_tab_idx {
                            if let Some(ref hid) = self.hid_manager {
                                if let Err(e) = hid.clear_alert(idx) {
                                    debug!("Failed to clear HID alert on tab switch: {}", e);
                                }
                            }
                        }
                    }
                }

                // Update window title to reflect active tab
                self.terminal_window.update_window_title();

                // Send HID display update and mode for the newly active session
                self.send_hid_for_active_session();

                // Update recent sessions menu for the new active tab's directory
                #[cfg(target_os = "macos")]
                if let Some(session) = self.terminal_window.session_manager.get_session(session_id) {
                    use agent_deck::core::claude_sessions::get_sessions_for_directory;
                    let mut sessions = get_sessions_for_directory(&session.working_directory);
                    sessions.truncate(5);
                    let menu_sessions: Vec<(String, String)> = sessions
                        .iter()
                        .map(|s| (s.session_id.clone(), s.display_title()))
                        .collect();
                    update_recent_sessions_menu(&menu_sessions);
                }

                // Save tabs immediately when switching (to persist active tab)
                self.save_tabs();

                // Check if this session needs PTY to be started (lazy loading)
                // Don't start if already running or loading
                let needs_start = self
                    .terminal_window
                    .session_manager
                    .get_session(session_id)
                    .map(|s| !s.is_running && !s.is_loading && !s.is_new_tab())
                    .unwrap_or(false);

                if needs_start {
                    if let Some(session) = self.terminal_window.session_manager.get_session(session_id) {
                        let working_dir = session.working_directory.clone();
                        let claude_session_id = session.claude_session_id.clone();
                        // Resume the stored session if available
                        self.start_claude_for_session(session_id, working_dir, claude_session_id, event_loop);
                    }
                }
            }
            TerminalAction::OpenDirectory { path, resume_session } => {
                // Get the active session info first to avoid borrow conflicts
                let session_info = self
                    .terminal_window
                    .session_manager
                    .active_session()
                    .map(|s| (s.id, s.is_new_tab()));

                let session_id = match session_info {
                    Some((id, true)) => {
                        // Update the existing new tab
                        if let Some(s) =
                            self.terminal_window.session_manager.active_session_mut()
                        {
                            s.working_directory = path.clone();
                            s.title = path
                                .file_name()
                                .and_then(|n| n.to_str())
                                .map(|s| s.to_string())
                                .unwrap_or_else(|| "New Session".to_string());
                            // Store the claude session ID for persistence
                            s.claude_session_id = resume_session.clone();
                        }
                        id
                    }
                    Some((_, false)) => {
                        // Create a new session for this directory
                        let id = self.terminal_window.session_manager.create_session(path.clone(), &self.terminal_window.current_palette);
                        // Store the claude session ID for persistence
                        if let Some(s) = self.terminal_window.session_manager.get_session_mut(id) {
                            s.claude_session_id = resume_session.clone();
                        }
                        id
                    }
                    None => {
                        let id = self.terminal_window.session_manager.create_session(path.clone(), &self.terminal_window.current_palette);
                        // Store the claude session ID for persistence
                        if let Some(s) = self.terminal_window.session_manager.get_session_mut(id) {
                            s.claude_session_id = resume_session.clone();
                        }
                        id
                    }
                };

                self.terminal_window
                    .session_manager
                    .set_active_session(session_id);
                self.terminal_window.update_window_title();
                self.send_hid_for_active_session();
                self.start_claude_for_session(session_id, path, resume_session, event_loop);
                // Save tabs after opening a directory
                self.save_tabs();
            }
            TerminalAction::BrowseDirectory => {
                // Use rfd for native file dialog (default to resume)
                if let Some(path) = rfd::FileDialog::new().pick_folder() {
                    // Recursively handle the open directory action
                    // Start fresh when browsing for a directory (user can use session picker for existing sessions)
                    self.handle_terminal_action(TerminalAction::OpenDirectory { path, resume_session: None }, event_loop);
                }
            }
            TerminalAction::AddBookmark(path) => {
                self.terminal_window.bookmark_manager.add_bookmark(path);
                let _ = self.terminal_window.bookmark_manager.save();
            }
            TerminalAction::RemoveBookmark(path) => {
                self.terminal_window.bookmark_manager.remove_bookmark(&path);
                let _ = self.terminal_window.bookmark_manager.save();
            }
            TerminalAction::RemoveRecent(path) => {
                self.terminal_window.bookmark_manager.remove_recent(&path);
                let _ = self.terminal_window.bookmark_manager.save();
            }
            TerminalAction::ClearRecent => {
                self.terminal_window.bookmark_manager.clear_recent();
                let _ = self.terminal_window.bookmark_manager.save();
            }
            TerminalAction::OpenSettings => {
                self.terminal_window.open_settings();
                if let Some(ref window) = self.terminal_window.window {
                    window.request_redraw();
                }
            }
            TerminalAction::ApplySettings(settings) => {
                // Check if font size changed
                let font_size_changed = self.terminal_window.font_size != settings.font_size;

                self.terminal_window.settings = settings.clone();
                let _ = settings.save();

                // Also persist font size to config for startup
                if font_size_changed {
                    self.config.terminal.font_size = settings.font_size;
                    let _ = self.config.save();
                    self.terminal_window.apply_font_size(settings.font_size);

                    if let Some(ref window) = self.terminal_window.window {
                        window.request_redraw();
                    }
                }
            }
            TerminalAction::Copy => {
                // Copy selected text to clipboard
                if let Some(text) = self.terminal_window.get_selection_text() {
                    if let Ok(mut clipboard) = arboard::Clipboard::new() {
                        let _ = clipboard.set_text(text);
                    }
                }
            }
            TerminalAction::Paste => {
                // Paste from clipboard to PTY
                if let Ok(mut clipboard) = arboard::Clipboard::new() {
                    if let Ok(text) = clipboard.get_text() {
                        self.terminal_window.send_to_active_pty(text.as_bytes());
                    }
                }
            }
            TerminalAction::FreshSessionCurrentDir => {
                // Get current directory and open fresh session
                if let Some(session) = self.terminal_window.session_manager.active_session() {
                    let path = session.working_directory.clone();
                    // Some("") means fresh start (no --continue, no --resume)
                    self.handle_terminal_action(
                        TerminalAction::OpenDirectory {
                            path,
                            resume_session: Some(String::new()),
                        },
                        event_loop,
                    );
                }
            }
            TerminalAction::LoadSession { session_id } => {
                // Get current directory and load specific session
                if let Some(session) = self.terminal_window.session_manager.active_session() {
                    let path = session.working_directory.clone();
                    // Some(id) means --resume {id}
                    self.handle_terminal_action(
                        TerminalAction::OpenDirectory {
                            path,
                            resume_session: Some(session_id),
                        },
                        event_loop,
                    );
                }
            }
            TerminalAction::SaveTabs => {
                self.save_tabs();
            }
            TerminalAction::ReadSoftKeys => {
                if let Some(ref hid) = self.hid_manager {
                    if hid.is_connected() {
                        let mut keys = [
                            SoftKeyEditState::Default(None),
                            SoftKeyEditState::Default(None),
                            SoftKeyEditState::Default(None),
                        ];
                        let mut had_error = false;
                        for i in 0u8..3 {
                            match hid.get_soft_key(i) {
                                Ok(config) => {
                                    keys[i as usize] = SoftKeyEditState::from_config(&config);
                                }
                                Err(e) => {
                                    error!("Failed to read soft key {}: {}", i, e);
                                    self.terminal_window
                                        .set_soft_key_error(format!("Failed to read key {}: {}", i, e));
                                    had_error = true;
                                    break;
                                }
                            }
                        }
                        if !had_error {
                            self.terminal_window.set_soft_key_configs(keys);
                        }
                    } else {
                        self.terminal_window
                            .set_soft_key_error("Device not connected".to_string());
                    }
                } else {
                    self.terminal_window
                        .set_soft_key_error("HID manager not initialized".to_string());
                }
            }
            TerminalAction::ApplySoftKeys(keys) => {
                if let Some(ref hid) = self.hid_manager {
                    for (i, key) in keys.iter().enumerate() {
                        let (key_type, data) = key.to_wire_data();
                        if let Err(e) = hid.set_soft_key(i as u8, key_type, &data, true) {
                            error!("Failed to set soft key {}: {}", i, e);
                            self.terminal_window
                                .set_soft_key_error(format!("Failed to set key {}: {}", i, e));
                            break;
                        }
                    }
                    info!("Soft keys applied to device");
                }
            }
            TerminalAction::ResetSoftKeys => {
                if let Some(ref hid) = self.hid_manager {
                    match hid.reset_soft_keys() {
                        Ok(configs) => {
                            info!("Soft keys reset to defaults");
                            let keys = [
                                SoftKeyEditState::from_config(&configs[0]),
                                SoftKeyEditState::from_config(&configs[1]),
                                SoftKeyEditState::from_config(&configs[2]),
                            ];
                            self.terminal_window.set_soft_key_configs(keys);
                        }
                        Err(e) => {
                            error!("Failed to reset soft keys: {}", e);
                            self.terminal_window
                                .set_soft_key_error(format!("Failed to reset: {}", e));
                        }
                    }
                }
            }
            TerminalAction::HidDisplayUpdate { session, task, tabs, active } => {
                if let Some(ref hid) = self.hid_manager {
                    if let Err(e) = hid.send_display_update(&session, task.as_deref(), &tabs, active) {
                        debug!("Failed to send HID display update: {}", e);
                    }
                }
            }
            TerminalAction::HidAlert { tab, session, text, details } => {
                if let Some(ref hid) = self.hid_manager {
                    if let Err(e) = hid.send_alert(tab, &session, &text, details.as_deref()) {
                        debug!("Failed to send HID alert: {}", e);
                    }
                }
            }
            TerminalAction::HidClearAlert(tab) => {
                if let Some(ref hid) = self.hid_manager {
                    if let Err(e) = hid.clear_alert(tab) {
                        debug!("Failed to clear HID alert: {}", e);
                    }
                }
            }
            TerminalAction::HidSetMode(mode) => {
                if let Some(ref hid) = self.hid_manager {
                    // Update last known device mode so the confirmation StateReport
                    // is recognized as a repeat (not a button press)
                    self.terminal_window.last_device_reported_mode = Some(mode);
                    // Suppress device StateReports briefly — the device may echo back
                    // stale state before processing our SetMode command
                    self.terminal_window.mode_set_from_app_at = Some(std::time::Instant::now());
                    if let Err(e) = hid.set_mode(mode) {
                        debug!("Failed to set HID mode: {}", e);
                    }
                }
            }
        }
    }

    /// Process an application event
    fn handle_event(&mut self, event: AppEvent, event_loop: &ActiveEventLoop) {
        match event {
            AppEvent::HidConnected => {
                info!("HID device connected");
                {
                    let mut state = self.state.write();
                    state.hid_connected = true;
                }
                self.terminal_window.hid_connected = true;
                if let Some(ref mut tray) = self.tray_manager {
                    tray.set_connected(true);
                }
                // Send initial display state to the newly connected device (only if window is shown)
                if self.terminal_window.is_visible() {
                    self.send_hid_for_active_session();
                }
            }
            AppEvent::HidDisconnected => {
                info!("HID device disconnected");
                {
                    let mut state = self.state.write();
                    state.hid_connected = false;
                }
                self.terminal_window.hid_connected = false;
                if let Some(ref mut tray) = self.tray_manager {
                    tray.set_connected(false);
                }
                // Clear all HID alert flags since device is gone
                for session in self.terminal_window.session_manager.iter_mut() {
                    session.hid_alert_active = false;
                    session.hid_alert_text = None;
                    session.hid_alert_details = None;
                }
            }
            AppEvent::TrayAction(action) => {
                info!("Tray action: {:?}", action);
                match action {
                    tray::TrayAction::ToggleWindow => {
                        if self.terminal_window.is_visible() {
                            // Hide window
                            self.save_tabs();
                            self.save_window_geometry();
                            self.terminal_window.hide();
                        } else {
                            // Show window
                            self.terminal_window.create_window(event_loop);
                            self.terminal_window.show();
                            self.send_hid_for_active_session();
                            // Start PTY for active session if needed
                            if let Some(session) = self.terminal_window.session_manager.active_session() {
                                if !session.is_new_tab() && !session.is_running && !session.is_loading {
                                    let session_id = session.id;
                                    let working_dir = session.working_directory.clone();
                                    let claude_session_id = session.claude_session_id.clone();
                                    self.start_claude_for_session(session_id, working_dir, claude_session_id, event_loop);
                                }
                            }
                        }
                        // Update tray menu text
                        if let Some(ref mut tray) = self.tray_manager {
                            tray.set_window_visible(self.terminal_window.is_visible());
                        }
                    }
                    tray::TrayAction::OpenSettings => {
                        info!("Opening settings...");
                        // Create window if needed and show
                        self.terminal_window.create_window(event_loop);
                        self.terminal_window.show();
                        // Open settings via action
                        self.handle_terminal_action(TerminalAction::OpenSettings, event_loop);
                    }
                    tray::TrayAction::Quit => {
                        // Check if any sessions have Claude actively working
                        let working_count = self.terminal_window.session_manager.working_session_count();
                        let should_quit = if working_count > 0 {
                            let message = if working_count == 1 {
                                "Claude is still working in 1 session. Quit anyway?".to_string()
                            } else {
                                format!("Claude is still working in {} sessions. Quit anyway?", working_count)
                            };

                            rfd::MessageDialog::new()
                                .set_title("Quit AgentDeck")
                                .set_description(&message)
                                .set_buttons(rfd::MessageButtons::YesNo)
                                .set_level(rfd::MessageLevel::Warning)
                                .show() == rfd::MessageDialogResult::Yes
                        } else {
                            true
                        };

                        if should_quit {
                            info!("Quitting application...");
                            // Use event_loop.exit() to trigger the exiting() callback
                            // which handles graceful shutdown of all Claude sessions
                            event_loop.exit();
                        }
                    }
                }
            }
            AppEvent::PtyOutput(output) => {
                // Process PTY output for active session (legacy)
                self.terminal_window.process_output(&output);
                if let Some(ref window) = self.terminal_window.window {
                    window.request_redraw();
                }
            }
            AppEvent::PtyOutputForSession { session_id, data } => {
                // Process PTY output for specific session
                self.terminal_window.process_output_for_session(session_id, &data);
                if let Some(ref window) = self.terminal_window.window {
                    window.request_redraw();
                }
            }
            AppEvent::DeviceStateChanged { mode, yolo } => {
                debug!("Device state changed: mode={}, yolo={}", mode, yolo);
                {
                    let mut state = self.state.write();
                    state.device_mode = mode;
                    state.device_yolo = yolo;
                }
                self.terminal_window.device_yolo = yolo;
                self.terminal_window.update_window_title();

                // Check if the app recently sent a SetMode command. If so, this
                // StateReport is a confirmation (or stale echo) — not a user button
                // press. Suppress Shift+Tab to prevent feedback loops when Claude
                // auto-switches modes (e.g., entering plan mode on its own).
                let suppressed = if let Some(sent_at) = self.terminal_window.mode_set_from_app_at {
                    if sent_at.elapsed() < std::time::Duration::from_millis(500) {
                        // Within suppression window — treat as confirmation
                        if mode == self.terminal_window.detected_mode {
                            // Device confirmed the mode we set — clear suppression
                            self.terminal_window.mode_set_from_app_at = None;
                        }
                        true
                    } else {
                        // Suppression expired
                        self.terminal_window.mode_set_from_app_at = None;
                        false
                    }
                } else {
                    false
                };

                // Send Shift+Tab only when the device mode actually changed since
                // the last report (i.e., user pressed the mode button on the device).
                // Ignore confirmations from our SetMode commands and repeated reports.
                let prev_device_mode = self.terminal_window.last_device_reported_mode;
                self.terminal_window.last_device_reported_mode = Some(mode);
                if !suppressed && prev_device_mode.is_some() && prev_device_mode != Some(mode) {
                    if let Some(session) = self.terminal_window.session_manager.active_session() {
                        if session.is_running {
                            debug!("Device mode button press: {} -> {}, sending Shift+Tab",
                                   prev_device_mode.unwrap(), mode);
                            self.terminal_window.send_to_session_pty(session.id, b"\x1b[Z");
                        }
                    }
                }
            }
            AppEvent::HidKeyEvent { keycode } => {
                // Resolve target: oldest alerting session (if any), and fallback to active
                let alert_id = self.terminal_window.session_manager.oldest_alerting_session_id();
                let target_id = alert_id
                    .or_else(|| self.terminal_window.session_manager.active_session_id());

                if let Some(sid) = target_id {
                    self.clear_hid_alert_for_session(sid);
                }

                // F20 (0x006F) → Claude button: show/focus window, or new tab if already focused
                if keycode == agent_deck::hid::keycodes::QmkKeycode::F20 as u16 {
                    if let Some(alert_sid) = alert_id {
                        // Active alert → switch to the alerting tab, bring window to front
                        self.terminal_window.show();
                        if let Some(ref mut tray) = self.tray_manager {
                            tray.set_window_visible(true);
                        }
                        self.handle_terminal_action(TerminalAction::SwitchTab(alert_sid), event_loop);
                    } else if self.terminal_window.is_visible() && self.terminal_window.is_focused() {
                        // Window is visible and focused → new tab
                        self.handle_terminal_action(TerminalAction::NewTab, event_loop);
                    } else if self.terminal_window.is_visible() {
                        // Window is visible but not focused → just bring to front
                        self.terminal_window.show();
                    } else if !self.session_ptys.is_empty() {
                        // Has running sessions → show window
                        self.terminal_window.show();
                        self.send_hid_for_active_session();
                        if let Some(ref mut tray) = self.tray_manager {
                            tray.set_window_visible(true);
                        }
                    } else if !self.terminal_window.session_manager.is_empty() {
                        // Has saved tabs but no PTY → create window and start active tab
                        self.terminal_window.create_window(event_loop);
                        self.terminal_window.show();
                        self.send_hid_for_active_session();
                        if let Some(ref mut tray) = self.tray_manager {
                            tray.set_window_visible(true);
                        }
                        if let Some(session) = self.terminal_window.session_manager.active_session() {
                            if !session.is_new_tab() && !session.is_running {
                                let session_id = session.id;
                                let working_dir = session.working_directory.clone();
                                let claude_session_id = session.claude_session_id.clone();
                                self.start_claude_for_session(session_id, working_dir, claude_session_id, event_loop);
                            }
                        }
                    } else {
                        // No sessions at all → start fresh
                        self.start_claude(event_loop);
                        self.send_hid_for_active_session();
                        if let Some(ref mut tray) = self.tray_manager {
                            tray.set_window_visible(true);
                        }
                    }
                } else {
                    // Check if active session is a new-tab page (no PTY)
                    let active_is_new_tab = self.terminal_window.session_manager
                        .active_session()
                        .map_or(false, |s| s.is_new_tab());

                    if active_is_new_tab {
                        // Route navigation keys to the new-tab UI
                        if let Some(egui_key) = qmk_keycode_to_egui_key(keycode) {
                            self.terminal_window.pending_hid_nav_keys.push(egui_key);
                            if let Some(ref window) = self.terminal_window.window {
                                window.request_redraw();
                            }
                        }
                    } else if let Some(bytes) = qmk_keycode_to_terminal_bytes(keycode) {
                        // Route keypress to alerting session (or active if no alerts)
                        if let Some(sid) = target_id {
                            self.terminal_window.send_to_session_pty(sid, &bytes);
                        }
                        if let Some(ref window) = self.terminal_window.window {
                            window.request_redraw();
                        }
                    }
                }
            }
            AppEvent::HidTypeString { text, send_enter } => {
                // Resolve target: oldest alerting session, or active session as fallback
                let target_id = self.terminal_window.session_manager
                    .oldest_alerting_session_id()
                    .or_else(|| self.terminal_window.session_manager.active_session_id());

                if let Some(sid) = target_id {
                    self.clear_hid_alert_for_session(sid);
                    self.terminal_window.send_to_session_pty(sid, text.as_bytes());
                    if send_enter {
                        self.terminal_window.send_to_session_pty(sid, b"\r");
                    }
                }
                if let Some(ref window) = self.terminal_window.window {
                    window.request_redraw();
                }
            }
            AppEvent::PtyExited(code) => {
                info!("PTY exited with code: {:?}", code);
                // Legacy handling - find which session exited
                // This would need session_id to be more precise
            }
            AppEvent::PtyExitedForSession { session_id, code } => {
                info!("PTY exited for session {} with code: {:?}", session_id, code);
                // Remove PTY (it already exited, no need to stop it)
                self.session_ptys.remove(&session_id);

                // Close the session/tab
                self.terminal_window.session_manager.close_session(session_id);

                // Save tabs after session closes
                self.save_tabs();

                // If no more sessions, hide window and update state
                if self.terminal_window.session_manager.is_empty() {
                    self.terminal_window.hide();
                    {
                        let mut state = self.state.write();
                        state.claude_running = false;
                    }
                } else {
                    // Send HID update for the newly active tab
                    self.send_hid_for_active_session();
                }
            }
            #[cfg(target_os = "macos")]
            AppEvent::MenuAction(action) => {
                self.handle_menu_action(action, event_loop);
            }
        }
    }

    /// Handle a menu action (macOS only)
    #[cfg(target_os = "macos")]
    fn handle_menu_action(&mut self, action: MenuAction, event_loop: &ActiveEventLoop) {
        use MenuAction::*;

        info!("Menu action: {:?}", action);

        match action {
            // App menu
            About => {
                // Show about dialog
                rfd::MessageDialog::new()
                    .set_title("About Agent Deck")
                    .set_description("Agent Deck v0.1.0\n\nA companion app for Claude Code CLI.")
                    .set_buttons(rfd::MessageButtons::Ok)
                    .show();
            }
            Settings => {
                // Create window if needed and show settings
                self.terminal_window.create_window(event_loop);
                self.terminal_window.show();
                self.handle_terminal_action(TerminalAction::OpenSettings, event_loop);
            }
            HideApp | HideOthers | ShowAll => {
                // These are handled by standard NSApp actions
            }
            HideWindow => {
                // Cmd+Q now hides window to tray instead of quitting
                self.save_tabs();
                self.save_window_geometry();
                self.terminal_window.hide();
                if let Some(ref mut tray) = self.tray_manager {
                    tray.set_window_visible(false);
                }
            }
            Quit => {
                // Check if any sessions have Claude actively working
                let working_count = self.terminal_window.session_manager.working_session_count();
                let should_quit = if working_count > 0 {
                    let message = if working_count == 1 {
                        "Claude is still working in 1 session. Quit anyway?".to_string()
                    } else {
                        format!("Claude is still working in {} sessions. Quit anyway?", working_count)
                    };

                    rfd::MessageDialog::new()
                        .set_title("Quit Agent Deck")
                        .set_description(&message)
                        .set_buttons(rfd::MessageButtons::YesNo)
                        .set_level(rfd::MessageLevel::Warning)
                        .show() == rfd::MessageDialogResult::Yes
                } else {
                    true
                };

                if should_quit {
                    info!("Quitting application via menu...");
                    // Use event_loop.exit() to trigger the exiting() callback
                    // which handles graceful shutdown of all Claude sessions
                    event_loop.exit();
                }
            }

            // File menu
            NewSession => {
                // Create new tab (show window if needed)
                self.terminal_window.create_window(event_loop);
                self.terminal_window.show();
                self.handle_terminal_action(TerminalAction::NewTab, event_loop);
            }
            FreshSession => {
                // Fresh session in current directory
                self.terminal_window.create_window(event_loop);
                self.terminal_window.show();
                self.handle_terminal_action(TerminalAction::FreshSessionCurrentDir, event_loop);
            }
            CloseTab => {
                // Close current tab
                if let Some(session) = self.terminal_window.session_manager.active_session() {
                    let session_id = session.id;
                    self.handle_terminal_action(TerminalAction::CloseTab(session_id), event_loop);
                }
            }

            // Edit menu
            Copy => {
                self.handle_terminal_action(TerminalAction::Copy, event_loop);
            }
            Paste => {
                self.handle_terminal_action(TerminalAction::Paste, event_loop);
            }
            SelectAll => {
                // Select all text in terminal
                self.terminal_window.select_all();
                if let Some(ref window) = self.terminal_window.window {
                    window.request_redraw();
                }
            }
            // View menu - temporary font size changes (not persisted)
            IncreaseFontSize => {
                let current = self.terminal_window.font_size;
                let new_size = (current + 1.0).min(72.0);
                self.terminal_window.apply_font_size_temporary(new_size);
                if let Some(ref window) = self.terminal_window.window {
                    window.request_redraw();
                }
            }
            DecreaseFontSize => {
                let current = self.terminal_window.font_size;
                let new_size = (current - 1.0).max(8.0);
                self.terminal_window.apply_font_size_temporary(new_size);
                if let Some(ref window) = self.terminal_window.window {
                    window.request_redraw();
                }
            }
            ResetFontSize => {
                // Reset to the font size at app startup
                let initial_size = self.terminal_window.initial_font_size();
                self.terminal_window.apply_font_size_temporary(initial_size);
                if let Some(ref window) = self.terminal_window.window {
                    window.request_redraw();
                }
            }
            ToggleFullscreen => {
                // Handled by standard action
            }

            // Window menu
            Minimize | Zoom => {
                // Handled by standard actions
            }

            // Help menu
            Help => {
                // Open help documentation (could be a web page or local file)
                if let Err(e) = open::that("https://github.com/anthropics/claude-code") {
                    error!("Failed to open help URL: {}", e);
                }
            }
            ReportIssue => {
                // Open GitHub issues page
                if let Err(e) = open::that("https://github.com/vden/agentdeck/issues/new") {
                    error!("Failed to open issues URL: {}", e);
                }
            }

            // Recent session submenu
            LoadRecentSession(idx) => {
                // This would load a specific recent session
                // For now, we'll just log it - full implementation would need session list
                info!("Load recent session at index: {}", idx);
            }
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        event_loop.set_control_flow(ControlFlow::Wait);

        // Create macOS native menu bar (must happen after winit starts the event loop)
        #[cfg(target_os = "macos")]
        if !self.menu_created {
            create_menu_bar();
            self.menu_created = true;
            info!("macOS native menu bar created");
        }

        // Load saved tabs (will be started lazily when activated)
        self.load_saved_tabs();

        // Initialize tray manager
        match TrayManager::new(self.event_tx.clone()) {
            Ok(tray) => {
                self.tray_manager = Some(tray);
                info!("Tray manager initialized");
            }
            Err(e) => {
                error!("Failed to initialize tray manager: {}", e);
            }
        }

        // Initialize HID manager
        let hid_config = self.config.hid.clone();
        let event_tx = self.event_tx.clone();
        match HidManager::new(hid_config, event_tx) {
            Ok(hid) => {
                self.hid_manager = Some(hid);
                // Note: HidConnected event is sent by HidManager::try_connect() if device is found
                info!("HID manager initialized");
            }
            Err(e) => {
                error!("Failed to initialize HID manager: {}", e);
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if self.terminal_window.is_our_window(window_id) {
            match &event {
                WindowEvent::CloseRequested => {
                    // Check if any sessions have Claude actively working
                    let working_count = self.terminal_window.session_manager.working_session_count();
                    if working_count > 0 {
                        // Show confirmation dialog
                        let message = if working_count == 1 {
                            "Claude is still working in 1 session. Close anyway?".to_string()
                        } else {
                            format!("Claude is still working in {} sessions. Close anyway?", working_count)
                        };

                        let confirmed = rfd::MessageDialog::new()
                            .set_title("Close AgentDeck")
                            .set_description(&message)
                            .set_buttons(rfd::MessageButtons::YesNo)
                            .set_level(rfd::MessageLevel::Warning)
                            .show() == rfd::MessageDialogResult::Yes;

                        if !confirmed {
                            return;
                        }
                    }

                    // Save window geometry before closing
                    self.save_window_geometry();
                    // Save tabs when window is closed
                    self.save_tabs();
                    self.terminal_window.hide();
                    // Sync tray menu
                    if let Some(ref mut tray) = self.tray_manager {
                        tray.set_window_visible(false);
                    }
                    return;
                }
                WindowEvent::Resized(size) => {
                    self.terminal_window.handle_resize(size.width, size.height);
                }
                WindowEvent::RedrawRequested => {
                    self.terminal_window.render();

                    // Process any terminal actions
                    let actions = self.terminal_window.take_pending_actions();
                    for action in actions {
                        self.handle_terminal_action(action, event_loop);
                    }
                    return;
                }
                _ => {}
            }
            self.terminal_window.handle_window_event(&event);
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let mut needs_redraw = false;

        // Collect events first, then process them
        let events: Vec<AppEvent> = if let Some(ref mut rx) = self.event_rx {
            let mut events = Vec::new();
            while let Ok(event) = rx.try_recv() {
                events.push(event);
            }
            events
        } else {
            Vec::new()
        };

        if !events.is_empty() {
            needs_redraw = true;
        }
        for event in events {
            self.handle_event(event, event_loop);
        }

        // Process any pending terminal actions
        let actions = self.terminal_window.take_pending_actions();
        if !actions.is_empty() {
            needs_redraw = true;
        }
        for action in actions {
            self.handle_terminal_action(action, event_loop);
        }

        // Check for Claude theme changes (throttled to once per 2 seconds)
        if self.last_theme_check.elapsed() >= std::time::Duration::from_secs(2) {
            self.last_theme_check = std::time::Instant::now();
            if self.terminal_window.check_claude_theme() {
                needs_redraw = true;
            }
        }

        // Request redraw if any events or actions were processed
        if needs_redraw {
            if let Some(ref window) = self.terminal_window.window {
                window.request_redraw();
            }
        }
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        info!("Application exiting, saving state...");

        // Destroy OpenGL resources properly to avoid "Resources will leak!" warning
        self.terminal_window.destroy();

        // Save window geometry before exiting
        if let Some(geometry) = self.terminal_window.get_window_geometry() {
            self.terminal_window.settings.window_geometry = geometry;
            if let Err(e) = self.terminal_window.settings.save() {
                error!("Failed to save window geometry on exit: {}", e);
            }
        }

        // Save tabs before closing sessions (preserves resolved session IDs)
        self.save_tabs();

        // Gracefully stop all Claude sessions by sending Ctrl-D twice
        // This ensures Claude saves the conversation before exiting
        for (session_id, session_pty) in &self.session_ptys {
            if session_pty.pty.is_running() {
                info!("Sending Ctrl-D to session {} to save conversation", session_id);
                // First Ctrl-D signals EOF/exit intent
                if let Err(e) = session_pty.pty.send_key("ctrl-d") {
                    warn!("Failed to send first Ctrl-D to session {}: {}", session_id, e);
                }
                // Brief pause between signals
                std::thread::sleep(std::time::Duration::from_millis(50));
                // Second Ctrl-D confirms exit
                if let Err(e) = session_pty.pty.send_key("ctrl-d") {
                    warn!("Failed to send second Ctrl-D to session {}: {}", session_id, e);
                }
            }
        }

        // Give Claude time to process and save the conversation
        if !self.session_ptys.is_empty() {
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
    }
}

/// Set up macOS application: process name and full native menu bar
#[cfg(target_os = "macos")]
#[allow(deprecated, unused_imports)]
fn setup_macos_app(event_tx: EventSender) {
    use cocoa::appkit::NSApp;
    use cocoa::base::nil;
    use cocoa::foundation::NSString;
    use objc::runtime::Object;
    use objc::{sel, sel_impl};

    unsafe {
        let app = NSApp();

        // Set activation policy to Regular (shows in dock, has menu bar)
        // NSApplicationActivationPolicyRegular = 0
        let _: () = objc::msg_send![app, setActivationPolicy: 0_isize];

        // Set the process name
        let process_info: *mut Object = cocoa::foundation::NSProcessInfo::processInfo(nil);
        let app_name = NSString::alloc(nil).init_str("Agent Deck");
        let _: () = objc::msg_send![process_info, setProcessName: app_name];
    }

    // Create channel for menu actions
    let (menu_tx, mut menu_rx) = mpsc::unbounded_channel::<MenuAction>();

    // Initialize the menu sender (menu bar will be created in resumed() callback)
    init_menu_sender(menu_tx);

    // Spawn a thread to forward menu events to the main event channel
    let event_tx_clone = event_tx.clone();
    std::thread::spawn(move || {
        while let Some(action) = menu_rx.blocking_recv() {
            if let Err(e) = event_tx_clone.send(AppEvent::MenuAction(action)) {
                error!("Failed to forward menu action: {}", e);
                break;
            }
        }
    });

    info!("macOS app setup complete with native menu bar");
}

#[cfg(not(target_os = "macos"))]
fn setup_macos_app(_event_tx: EventSender) {
    // No-op on other platforms
}

/// Set up macOS quit confirmation handler
/// This intercepts Cmd-Q to show confirmation when Claude is working
/// Note: Since we now have a proper menu bar with Quit handled via MenuAction,
/// this handler is disabled. The menu's Quit action handles confirmation.
#[cfg(target_os = "macos")]
fn setup_macos_quit_handler() {
    // Quit confirmation is now handled by the menu bar's Quit action
    // in handle_menu_action(). This avoids the complexity of isa-swizzling
    // the app delegate.
}

#[cfg(not(target_os = "macos"))]
fn setup_macos_quit_handler() {
    // No-op on other platforms
}

fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(tracing_subscriber::fmt::layer())
        .init();

    info!("Starting Agent Deck companion app");

    // Capture login shell environment (PATH, LANG, etc.) before anything else.
    // When launched from Finder/.app bundle, the process inherits a bare-bones
    // environment — this ensures PTY processes get the user's full setup.
    let login_env = Arc::new(resolve_login_env());

    // Load configuration
    let config = Config::load()?;
    info!("Configuration loaded");

    // Create shared state
    let state = Arc::new(RwLock::new(AppState::default()));

    // Create event channel
    let (event_tx, event_rx) = mpsc::unbounded_channel();

    // Create event loop
    let event_loop = EventLoop::new()?;

    // Create EventSender that wraps the channel + event loop proxy for wake-up
    let proxy = event_loop.create_proxy();
    let event_sender = EventSender::new(event_tx, proxy);

    // Set up macOS app (process name and native menu bar, must be after event loop creation)
    setup_macos_app(event_sender.clone());

    // Set up macOS quit confirmation handler (must be after event loop creation)
    setup_macos_quit_handler();

    // Create application
    let mut app = App::new(state, event_sender, event_rx, config, login_env);

    // Run event loop
    event_loop.run_app(&mut app)?;

    Ok(())
}
