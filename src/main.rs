//! Agent Deck Companion App - Entry Point
//!
//! This is the main entry point for the Agent Deck companion application.
//! It initializes all modules and runs the main event loop.

use agent_deck::{
    core::{
        config::Config,
        events::AppEvent,
        sessions::SessionId,
        state::AppState,
        tabs::{TabEntry, TabState},
    },
    hid::HidManager,
    hotkey::HotkeyManager,
    pty::PtyWrapper,
    tray::TrayManager,
    window::{TerminalAction, TerminalWindowState},
};
use anyhow::Result;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{error, info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    window::WindowId,
};

use agent_deck::hotkey;
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
    /// Event sender for inter-module communication
    event_tx: mpsc::UnboundedSender<AppEvent>,
    /// Event receiver for inter-module communication
    event_rx: Option<mpsc::UnboundedReceiver<AppEvent>>,
    /// HID manager for device communication
    hid_manager: Option<HidManager>,
    /// Hotkey manager for global key handling
    hotkey_manager: Option<HotkeyManager>,
    /// Tray manager for system tray
    tray_manager: Option<TrayManager>,
    /// PTY wrappers per session
    session_ptys: HashMap<SessionId, SessionPty>,
    /// Terminal window state
    terminal_window: TerminalWindowState,
    /// Configuration
    config: Config,
}

impl App {
    fn new(
        state: Arc<RwLock<AppState>>,
        event_tx: mpsc::UnboundedSender<AppEvent>,
        event_rx: mpsc::UnboundedReceiver<AppEvent>,
        config: Config,
    ) -> Self {
        let font_size = config.terminal.font_size;
        Self {
            state,
            event_tx,
            event_rx: Some(event_rx),
            hid_manager: None,
            hotkey_manager: None,
            tray_manager: None,
            session_ptys: HashMap::new(),
            terminal_window: TerminalWindowState::new(font_size),
            config,
        }
    }

    /// Start Claude in PTY for a specific session
    fn start_claude_for_session(
        &mut self,
        session_id: SessionId,
        working_directory: PathBuf,
        resume: bool,
        event_loop: &ActiveEventLoop,
    ) {
        if self.session_ptys.contains_key(&session_id) {
            // Already running for this session
            return;
        }

        info!(
            "Starting Claude in PTY for session {} at {:?} (resume={})",
            session_id, working_directory, resume
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

        // Create PTY wrapper with custom working directory
        let claude_config = self.config.claude.clone();
        // The PTY wrapper will use the working directory from the config
        // We need to modify it to use the session's working directory
        let pty = Arc::new(PtyWrapper::new_with_cwd(
            claude_config,
            self.event_tx.clone(),
            working_directory.clone(),
            session_id,
            resume,
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
                            .unwrap_or_else(|| "Claude".to_string());
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
                    .create_session(working_dir.clone())
            }
        };

        // Default to resuming the conversation when started from tray/HID
        self.start_claude_for_session(session_id, working_dir, true, event_loop);
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
                        .create_placeholder(tab.working_directory, tab.title);
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

    /// Handle terminal UI actions
    fn handle_terminal_action(&mut self, action: TerminalAction, event_loop: &ActiveEventLoop) {
        match action {
            TerminalAction::NewTab => {
                // Create a new tab (will show new tab page)
                let session_id = self.terminal_window.session_manager.create_new_tab();
                self.terminal_window
                    .session_manager
                    .set_active_session(session_id);
                self.terminal_window.update_window_title();
                info!("Created new tab with session ID {}", session_id);
            }
            TerminalAction::CloseTab(session_id) => {
                self.stop_claude_for_session(session_id);
                // Update window title after closing (active tab may have changed)
                self.terminal_window.update_window_title();
                // Save tabs after closing
                self.save_tabs();
            }
            TerminalAction::SwitchTab(session_id) => {
                self.terminal_window
                    .session_manager
                    .set_active_session(session_id);

                // Update window title to reflect active tab
                self.terminal_window.update_window_title();

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
                        self.start_claude_for_session(session_id, working_dir, true, event_loop);
                    }
                }
            }
            TerminalAction::OpenDirectory { path, resume } => {
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
                                .unwrap_or_else(|| "Claude".to_string());
                        }
                        id
                    }
                    Some((_, false)) => {
                        // Create a new session for this directory
                        self.terminal_window.session_manager.create_session(path.clone())
                    }
                    None => {
                        self.terminal_window.session_manager.create_session(path.clone())
                    }
                };

                self.terminal_window
                    .session_manager
                    .set_active_session(session_id);
                self.terminal_window.update_window_title();
                self.start_claude_for_session(session_id, path, resume, event_loop);
                // Save tabs after opening a directory
                self.save_tabs();
            }
            TerminalAction::BrowseDirectory => {
                // Use rfd for native file dialog (default to resume)
                if let Some(path) = rfd::FileDialog::new().pick_folder() {
                    // Recursively handle the open directory action
                    self.handle_terminal_action(TerminalAction::OpenDirectory { path, resume: true }, event_loop);
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
                // Settings modal is handled internally by the terminal window
            }
            TerminalAction::ApplySettings(settings) => {
                self.terminal_window.settings = settings.clone();
                let _ = settings.save();
                // TODO: Apply font size changes to glyph cache
            }
        }
    }

    /// Process an application event
    fn handle_event(&mut self, event: AppEvent, event_loop: &ActiveEventLoop) {
        match event {
            AppEvent::ClaudeStateChanged(ref claude_state) => {
                info!("Claude state changed: {:?}", claude_state);
                {
                    let mut state = self.state.write();
                    state.claude_state = claude_state.clone();
                }
                // Update active session's claude state
                if let Some(session) = self.terminal_window.session_manager.active_session() {
                    *session.claude_state.lock() = claude_state.clone();
                }
                // Send update to HID device
                if let Some(ref hid) = self.hid_manager {
                    if let Err(e) = hid.send_display_update(claude_state) {
                        error!("Failed to send display update: {}", e);
                    }
                }
            }
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
            }
            AppEvent::HotkeyPressed(key) => {
                info!("Hotkey pressed: {:?}", key);
                match key {
                    hotkey::HotkeyType::ClaudeKey => {
                        // Toggle terminal window visibility or start Claude if not running
                        if !self.session_ptys.is_empty() {
                            self.terminal_window.toggle();
                        } else if !self.terminal_window.session_manager.is_empty() {
                            // We have saved tabs but no PTY running - show window and start active tab
                            self.terminal_window.create_window(event_loop);
                            self.terminal_window.show();

                            // Start PTY for active session if it has a working directory
                            if let Some(session) = self.terminal_window.session_manager.active_session() {
                                if !session.is_new_tab() && !session.is_running {
                                    let session_id = session.id;
                                    let working_dir = session.working_directory.clone();
                                    self.start_claude_for_session(session_id, working_dir, true, event_loop);
                                }
                            }
                        } else {
                            self.start_claude(event_loop);
                        }
                    }
                    hotkey::HotkeyType::SoftKey(n) => {
                        info!("Soft key {} pressed", n);
                        // TODO: Handle soft key actions
                    }
                }
            }
            AppEvent::TrayAction(action) => {
                info!("Tray action: {:?}", action);
                match action {
                    tray::TrayAction::StartClaude => {
                        info!("Starting Claude...");
                        // Check if we have saved tabs to restore
                        if !self.terminal_window.session_manager.is_empty() {
                            self.terminal_window.create_window(event_loop);
                            self.terminal_window.show();
                            // Start PTY for active session if needed
                            if let Some(session) = self.terminal_window.session_manager.active_session() {
                                if !session.is_new_tab() && !session.is_running {
                                    let session_id = session.id;
                                    let working_dir = session.working_directory.clone();
                                    self.start_claude_for_session(session_id, working_dir, true, event_loop);
                                }
                            }
                        } else {
                            self.start_claude(event_loop);
                        }
                    }
                    tray::TrayAction::StopClaude => {
                        info!("Stopping Claude...");
                        // Stop all sessions
                        let session_ids: Vec<_> = self.session_ptys.keys().cloned().collect();
                        for session_id in session_ids {
                            self.stop_claude_for_session(session_id);
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
                        info!("Quitting application...");
                        // Save tabs before quitting
                        self.save_tabs();
                        // Stop all sessions
                        let session_ids: Vec<_> = self.session_ptys.keys().cloned().collect();
                        for session_id in session_ids {
                            self.stop_claude_for_session(session_id);
                        }
                        std::process::exit(0);
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
            AppEvent::TerminalTitleChanged(title) => {
                info!("Terminal title changed: {}", title);
                if let Some(ref hid) = self.hid_manager {
                    if let Err(e) = hid.send_task_update(&title) {
                        error!("Failed to send task update to HID: {}", e);
                    }
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
                }
            }
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        event_loop.set_control_flow(ControlFlow::Poll);

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

        // Initialize hotkey manager
        match HotkeyManager::new(self.event_tx.clone(), &self.config.hotkeys) {
            Ok(hotkey) => {
                self.hotkey_manager = Some(hotkey);
                info!("Hotkey manager initialized");
            }
            Err(e) => {
                error!("Failed to initialize hotkey manager: {}", e);
            }
        }

        // Initialize HID manager
        let hid_config = self.config.hid.clone();
        let event_tx = self.event_tx.clone();
        match HidManager::new(hid_config, event_tx) {
            Ok(hid) => {
                self.hid_manager = Some(hid);
                let _ = self.event_tx.send(AppEvent::HidConnected);
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
                    // Save tabs when window is closed
                    self.save_tabs();
                    self.terminal_window.hide();
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
        // Process hotkey events
        if let Some(ref manager) = self.hotkey_manager {
            manager.process_events();
        }

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

        for event in events {
            self.handle_event(event, event_loop);
        }

        // Process any pending terminal actions
        let actions = self.terminal_window.take_pending_actions();
        for action in actions {
            self.handle_terminal_action(action, event_loop);
        }

        // Request redraw if terminal window is visible
        if self.terminal_window.is_visible() {
            if let Some(ref window) = self.terminal_window.window {
                window.request_redraw();
            }
        }
    }
}

/// Set the macOS application name in the menu bar
#[cfg(target_os = "macos")]
#[allow(deprecated)]
fn set_macos_app_name() {
    use cocoa::appkit::NSApp;
    use cocoa::base::nil;
    use cocoa::foundation::NSString;
    use objc::runtime::Object;
    use objc::{sel, sel_impl};

    unsafe {
        // Set the process name
        let process_info: *mut Object = cocoa::foundation::NSProcessInfo::processInfo(nil);
        let app_name = NSString::alloc(nil).init_str("Agent Deck");
        let _: () = objc::msg_send![process_info, setProcessName: app_name];

        // Also set the app's main menu title by getting the app and its main menu
        let app = NSApp();
        if app != nil {
            let main_menu: *mut Object = objc::msg_send![app, mainMenu];
            if main_menu != nil {
                let first_item: *mut Object = objc::msg_send![main_menu, itemAtIndex: 0i64];
                if first_item != nil {
                    let submenu: *mut Object = objc::msg_send![first_item, submenu];
                    if submenu != nil {
                        let _: () = objc::msg_send![submenu, setTitle: app_name];
                    }
                }
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn set_macos_app_name() {
    // No-op on other platforms
}

fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(tracing_subscriber::fmt::layer())
        .init();

    info!("Starting Agent Deck companion app");

    // Load configuration
    let config = Config::load()?;
    info!("Configuration loaded");

    // Create shared state
    let state = Arc::new(RwLock::new(AppState::default()));

    // Create event channel
    let (event_tx, event_rx) = mpsc::unbounded_channel();

    // Create event loop
    let event_loop = EventLoop::new()?;

    // Set macOS app name (must be after event loop creation)
    set_macos_app_name();

    // Create application
    let mut app = App::new(state, event_tx, event_rx, config);

    // Run event loop
    event_loop.run_app(&mut app)?;

    Ok(())
}
