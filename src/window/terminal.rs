//! Terminal window using egui with WezTerm's terminal emulation
//!
//! Uses wezterm-term for full terminal emulation including escape sequence parsing,
//! cursor handling, scrollback, and all terminal features.
//! Supports multiple tabs with browser-style tab bar.

use super::context_menu::{render_context_menu, ContextMenuState};
use super::glyph_cache::GlyphCache;
use super::input::{build_arrow_seq, build_f1_f4_seq, build_home_end_seq, build_tilde_seq, encode_modifiers, open_url};
use super::render::{
    handle_settings_modal_result, render_hyperlink_tooltip, render_tab_bar,
    render_terminal_content, RenderParams, MAX_TAB_TITLE_LEN, TAB_BAR_HEIGHT,
};
use super::settings_modal::{render_settings_modal, SettingsModal};
use arboard::Clipboard;
use crate::core::bookmarks::BookmarkManager;
use crate::core::sessions::{SessionId, SessionManager};
use crate::core::settings::{ColorScheme, Settings};
use crate::core::state::ClaudeState;
use crate::core::themes::{Theme, ThemeRegistry, claude_json_mtime, read_claude_theme_is_light};
use crate::terminal::Session;
use wezterm_term::color::ColorPalette;
use egui_glow::EguiGlow;
use glutin::config::ConfigTemplateBuilder;
use glutin::context::{ContextAttributesBuilder, PossiblyCurrentContext};
use glutin::display::GetGlDisplay;
use glutin::prelude::*;
use glutin::surface::{Surface, SurfaceAttributesBuilder, WindowSurface};
use glutin_winit::DisplayBuilder;
use parking_lot::Mutex;
use raw_window_handle::HasWindowHandle;
use std::cell::{Cell, RefCell};
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Arc;
use std::time::SystemTime;
use tokio::sync::mpsc;
use tracing::{debug, error, info};
use wezterm_cell::Hyperlink;
use winit::dpi::{LogicalPosition, LogicalSize};
use winit::event::{ElementState, MouseButton, Modifiers, MouseScrollDelta, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{Key, KeyCode, NamedKey, PhysicalKey};
use winit::window::{Window, WindowAttributes, WindowId};

use crate::core::claude_sessions::{find_most_recent_session, get_sessions_for_directory};
#[cfg(target_os = "macos")]
use crate::macos::{update_edit_menu_state, update_recent_sessions_menu, ContextMenuAction, ContextMenuSession, show_context_menu};

/// Channel for sending input to PTY
pub type InputSender = mpsc::UnboundedSender<Vec<u8>>;

/// Callback type for PTY resize notifications
pub type ResizeCallback = Box<dyn Fn(u16, u16) + Send + Sync>;

/// Actions that can be triggered from the terminal UI
#[derive(Debug, Clone)]
pub enum TerminalAction {
    /// Create a new tab
    NewTab,
    /// Close a tab by session ID
    CloseTab(SessionId),
    /// Switch to a tab by session ID
    SwitchTab(SessionId),
    /// Open a directory in a new or current tab
    /// resume_session: None = --continue, Some("") = fresh start, Some(id) = --resume {session-id}
    OpenDirectory { path: PathBuf, resume_session: Option<String> },
    /// Browse for a directory using native dialog
    BrowseDirectory,
    /// Add a bookmark
    AddBookmark(PathBuf),
    /// Remove a bookmark
    RemoveBookmark(PathBuf),
    /// Remove a recent entry
    RemoveRecent(PathBuf),
    /// Clear all recent entries
    ClearRecent,
    /// Open settings modal
    OpenSettings,
    /// Apply settings
    ApplySettings(Settings),
    /// Copy selected text to clipboard
    Copy,
    /// Paste from clipboard
    Paste,
    /// Open fresh session from current directory
    FreshSessionCurrentDir,
    /// Load a specific Claude session by ID
    LoadSession { session_id: String },
    /// Save tabs to persistent storage
    SaveTabs,
}

/// Terminal window state managed within the main app
pub struct TerminalWindowState {
    /// Window handle
    pub window: Option<Arc<Window>>,
    /// OpenGL context
    gl_context: Option<PossiblyCurrentContext>,
    /// OpenGL surface
    gl_surface: Option<Surface<WindowSurface>>,
    /// Glow context
    glow_context: Option<Arc<glow::Context>>,
    /// Egui integration
    egui_glow: Option<EguiGlow>,
    /// Session manager for multiple tabs
    pub session_manager: SessionManager,
    /// Bookmark manager
    pub bookmark_manager: BookmarkManager,
    /// App settings
    pub settings: Settings,
    /// Settings modal state
    settings_modal: SettingsModal,
    /// HID connection state
    pub hid_connected: bool,
    /// Whether window should be visible
    visible: Arc<AtomicBool>,
    /// Window ID (when created)
    window_id: Option<WindowId>,
    /// Callback to notify PTY of resize (for active session)
    resize_callback: Option<ResizeCallback>,
    /// Current scroll offset (0 = bottom, positive = viewing history)
    scroll_offset: Arc<AtomicI32>,
    /// Current keyboard modifiers state
    modifiers: Modifiers,
    /// Cached character width for resize calculations (Cell for interior mutability)
    cached_char_width: Cell<f32>,
    /// Cached line height for resize calculations (Cell for interior mutability)
    cached_line_height: Cell<f32>,
    /// Font size in points
    pub font_size: f32,
    /// Initial font size at app start (for reset)
    initial_font_size: f32,
    /// Selection start position (row, col) in terminal coordinates
    selection_start: Option<(i64, usize)>,
    /// Selection end position (row, col) in terminal coordinates
    selection_end: Option<(i64, usize)>,
    /// Whether mouse is currently dragging for selection
    is_selecting: bool,
    /// Current cursor position in logical pixels
    cursor_position: Option<(f64, f64)>,
    /// WezTerm-based glyph cache for proper Unicode rendering
    glyph_cache: RefCell<Option<GlyphCache>>,
    /// Currently hovered hyperlink (for visual feedback)
    hovered_hyperlink: Option<Arc<Hyperlink>>,
    /// Pending actions to be processed by the main app
    pending_actions: Vec<TerminalAction>,
    /// Whether egui image loaders have been installed
    image_loaders_installed: bool,
    /// Context menu state for right-click popup
    context_menu: ContextMenuState,
    /// Theme registry with all available themes
    pub theme_registry: ThemeRegistry,
    /// Currently active theme
    pub current_theme: Theme,
    /// Current color palette (derived from theme)
    pub current_palette: ColorPalette,
    /// Current color scheme (derived from theme, for UI chrome)
    pub color_scheme: ColorScheme,
    /// Last known mtime of ~/.claude.json (for detecting theme changes)
    claude_json_mtime: Option<SystemTime>,
}

impl TerminalWindowState {
    pub fn new(font_size: f32) -> Self {
        // Estimate initial metrics based on font size (will be calibrated on first render)
        let estimated_char_width = font_size * 0.6;
        let estimated_line_height = font_size * 1.3;

        let mut settings = Settings::load().unwrap_or_default();
        // Sync settings font_size with the actual startup value from config
        settings.font_size = font_size;
        let bookmark_manager = BookmarkManager::load().unwrap_or_default();

        // Initialize theme system — auto-detect from ~/.claude.json
        let theme_registry = ThemeRegistry::new();
        let is_light = read_claude_theme_is_light();
        let theme_name = if is_light { "Light" } else { "Dark" };
        let current_theme = theme_registry
            .find(theme_name)
            .cloned()
            .unwrap_or_else(|| theme_registry.find("Dark").unwrap().clone());
        let current_palette = current_theme.to_color_palette();
        let color_scheme = ColorScheme::from_is_light(current_theme.is_light);
        let cj_mtime = claude_json_mtime();

        Self {
            window: None,
            gl_context: None,
            gl_surface: None,
            glow_context: None,
            egui_glow: None,
            session_manager: SessionManager::new(),
            bookmark_manager,
            settings: settings.clone(),
            settings_modal: SettingsModal::new(settings),
            hid_connected: false,
            visible: Arc::new(AtomicBool::new(false)),
            window_id: None,
            resize_callback: None,
            scroll_offset: Arc::new(AtomicI32::new(0)),
            modifiers: Modifiers::default(),
            cached_char_width: Cell::new(estimated_char_width),
            cached_line_height: Cell::new(estimated_line_height),
            font_size,
            initial_font_size: font_size,
            selection_start: None,
            selection_end: None,
            is_selecting: false,
            cursor_position: None,
            glyph_cache: RefCell::new(None),
            hovered_hyperlink: None,
            pending_actions: Vec::new(),
            image_loaders_installed: false,
            context_menu: ContextMenuState::default(),
            theme_registry,
            current_theme,
            current_palette,
            color_scheme,
            claude_json_mtime: cj_mtime,
        }
    }

    /// Get pending actions and clear the queue
    pub fn take_pending_actions(&mut self) -> Vec<TerminalAction> {
        std::mem::take(&mut self.pending_actions)
    }

    /// Destroy OpenGL resources (must be called before dropping)
    pub fn destroy(&mut self) {
        if let Some(ref mut egui_glow) = self.egui_glow {
            egui_glow.destroy();
        }
    }

    /// Get the active session's terminal session (for legacy compatibility)
    pub fn session(&self) -> Option<Arc<Mutex<Session>>> {
        self.session_manager
            .active_session()
            .map(|s| Arc::clone(&s.session))
    }

    /// Get the active session's claude state (for legacy compatibility)
    pub fn claude_state(&self) -> Option<Arc<Mutex<ClaudeState>>> {
        self.session_manager
            .active_session()
            .map(|s| Arc::clone(&s.claude_state))
    }

    /// Set callback for PTY resize notifications
    pub fn set_resize_callback<F>(&mut self, callback: F)
    where
        F: Fn(u16, u16) + Send + Sync + 'static,
    {
        self.resize_callback = Some(Box::new(callback));
    }

    /// Trigger resize based on current window size (call after setting resize_callback)
    pub fn sync_size(&mut self) {
        if let Some(ref window) = self.window {
            let size = window.inner_size();
            self.handle_resize(size.width, size.height);
        }
    }

    /// Set input sender for the active session
    pub fn set_input_sender(&mut self, tx: InputSender) {
        if let Some(session) = self.session_manager.active_session_mut() {
            session.pty_input_tx = Some(tx);
        }
    }

    /// Set input sender for a specific session
    pub fn set_session_input_sender(&mut self, session_id: SessionId, tx: InputSender) {
        if let Some(session) = self.session_manager.get_session_mut(session_id) {
            session.pty_input_tx = Some(tx);
            session.is_running = true;
        }
    }

    /// Update the window title based on active session
    pub fn update_window_title(&self) {
        if let Some(ref window) = self.window {
            let title = if let Some(session) = self.session_manager.active_session() {
                if session.is_new_tab() {
                    "\u{1F916}".to_string()
                } else {
                    session.working_directory.display().to_string()
                }
            } else {
                "\u{1F916}".to_string()
            };
            window.set_title(&title);
        }
    }

    /// Mark a session as started
    pub fn mark_session_started(&mut self, session_id: SessionId) {
        if let Some(session) = self.session_manager.get_session_mut(session_id) {
            session.is_running = true;
            session.is_loading = false;
        }
    }

    /// Mark a session as loading (PTY starting)
    pub fn mark_session_loading(&mut self, session_id: SessionId) {
        if let Some(session) = self.session_manager.get_session_mut(session_id) {
            session.is_loading = true;
        }
    }

    /// Try to resolve the actual Claude session ID for a fresh session
    fn try_resolve_session_id(&mut self, session_id: SessionId) {
        let working_dir = {
            let session = match self.session_manager.get_session(session_id) {
                Some(s) => s,
                None => return,
            };

            let start = match session.session_start_time {
                Some(t) => t,
                None => return,
            };

            if start.elapsed().as_secs() < 1 {
                return;
            }

            session.working_directory.clone()
        };

        if let Some(found_id) = find_most_recent_session(&working_dir) {
            if let Some(session) = self.session_manager.get_session_mut(session_id) {
                info!(
                    "Resolved session {} to Claude session ID: {}",
                    session_id, found_id
                );
                session.claude_session_id = Some(found_id);
                session.needs_session_resolution = false;
            }

            self.pending_actions.push(TerminalAction::SaveTabs);
        }
    }

    pub fn is_visible(&self) -> bool {
        self.visible.load(Ordering::Relaxed)
    }

    pub fn show(&self) {
        self.visible.store(true, Ordering::Relaxed);
        if let Some(ref window) = self.window {
            window.set_visible(true);
            window.focus_window();
        }
    }

    pub fn hide(&self) {
        self.visible.store(false, Ordering::Relaxed);
        if let Some(ref window) = self.window {
            window.set_visible(false);
        }
    }

    pub fn toggle(&self) {
        if self.is_visible() {
            self.hide();
        } else {
            self.show();
        }
    }

    pub fn window_id(&self) -> Option<WindowId> {
        self.window_id
    }

    pub fn is_our_window(&self, id: WindowId) -> bool {
        self.window_id == Some(id)
    }

    /// Send input bytes to the active session's PTY
    fn send_to_pty(&self, data: &[u8]) {
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

    /// Scroll the view (positive = scroll up into history, negative = scroll down)
    pub fn scroll_view(&self, delta: i32) {
        if let Some(session_info) = self.session_manager.active_session() {
            let session = session_info.session.lock();
            let max_offset = session.with_terminal(|term| {
                let screen = term.screen();
                screen.scrollback_rows().saturating_sub(screen.physical_rows) as i32
            });
            drop(session);

            let current = self.scroll_offset.load(Ordering::Relaxed);
            let new_offset = (current + delta).clamp(0, max_offset);
            self.scroll_offset.store(new_offset, Ordering::Relaxed);
        }
    }

    /// Reset scroll to bottom (viewing current content)
    pub fn scroll_to_bottom(&self) {
        self.scroll_offset.store(0, Ordering::Relaxed);
    }

    /// Clear current text selection
    pub fn clear_selection(&mut self) {
        self.selection_start = None;
        self.selection_end = None;
        self.is_selecting = false;
    }

    /// Check if there is an active selection with actual content
    pub fn has_selection(&self) -> bool {
        match (self.selection_start, self.selection_end) {
            (Some(start), Some(end)) => start != end,
            _ => false,
        }
    }

    /// Open context menu at the specified position
    fn open_context_menu(&mut self, x: f32, y: f32) {
        let sessions = if let Some(session) = self.session_manager.active_session() {
            let mut sessions = get_sessions_for_directory(&session.working_directory);
            sessions.truncate(5);
            sessions
        } else {
            Vec::new()
        };

        #[cfg(target_os = "macos")]
        {
            let menu_sessions: Vec<(String, String)> = sessions
                .iter()
                .map(|s| (s.session_id.clone(), s.display_title()))
                .collect();
            update_recent_sessions_menu(&menu_sessions);

            if let Some(ref window) = self.window {
                if let Ok(handle) = window.window_handle() {
                    use raw_window_handle::RawWindowHandle;
                    if let RawWindowHandle::AppKit(appkit_handle) = handle.as_raw() {
                        let view = appkit_handle.ns_view.as_ptr();

                        let has_selection = self.has_selection();
                        let has_clipboard = Clipboard::new()
                            .ok()
                            .and_then(|mut c| c.get_text().ok())
                            .map(|t| !t.is_empty())
                            .unwrap_or(false);

                        let menu_sessions: Vec<ContextMenuSession> = sessions
                            .iter()
                            .map(|s| ContextMenuSession {
                                session_id: s.session_id.clone(),
                                title: s.display_title(),
                                time_ago: s.relative_modified_time(),
                            })
                            .collect();

                        let action = show_context_menu(
                            view,
                            x as f64,
                            y as f64,
                            has_selection,
                            has_clipboard,
                            &menu_sessions,
                        );

                        if let Some(action) = action {
                            match action {
                                ContextMenuAction::NewSession => {
                                    self.pending_actions.push(TerminalAction::NewTab);
                                }
                                ContextMenuAction::FreshSessionHere => {
                                    self.pending_actions.push(TerminalAction::FreshSessionCurrentDir);
                                }
                                ContextMenuAction::LoadSession { session_id } => {
                                    self.pending_actions.push(TerminalAction::LoadSession { session_id });
                                }
                                ContextMenuAction::Copy => {
                                    self.pending_actions.push(TerminalAction::Copy);
                                }
                                ContextMenuAction::Paste => {
                                    self.pending_actions.push(TerminalAction::Paste);
                                }
                            }
                        }
                        return;
                    }
                }
            }
        }

        // Fallback to egui context menu
        self.context_menu.available_sessions = sessions;
        self.context_menu.position = egui::Pos2::new(x, y);
        self.context_menu.is_open = true;
        self.context_menu.submenu_open = false;
    }

    /// Close context menu
    fn close_context_menu(&mut self) {
        self.context_menu.is_open = false;
        self.context_menu.submenu_open = false;
        self.context_menu.opened_time = 0.0;
    }

    /// Select all text in the terminal
    pub fn select_all(&mut self) {
        if let Some(session_info) = self.session_manager.active_session() {
            let session = session_info.session.lock();
            let (first_row, last_row, cols) = session.with_terminal(|term| {
                let screen = term.screen();
                let scrollback = screen.scrollback_rows() as i64;
                let physical = screen.physical_rows as i64;
                let first_row = -(scrollback as i64);
                let last_row = physical - 1;
                let cols = screen.physical_cols;
                (first_row, last_row, cols)
            });
            drop(session);

            self.selection_start = Some((first_row, 0));
            self.selection_end = Some((last_row, cols));
        }
    }

    /// Invalidate the glyph cache (e.g., after font size change)
    pub fn invalidate_glyph_cache(&mut self) {
        *self.glyph_cache.borrow_mut() = None;
    }

    /// Open the settings modal
    pub fn open_settings(&mut self) {
        self.settings_modal.open(&self.settings);
    }

    /// Check ~/.claude.json for theme changes and update if needed.
    /// Returns true if the theme changed.
    pub fn check_claude_theme(&mut self) -> bool {
        let new_mtime = claude_json_mtime();
        if new_mtime == self.claude_json_mtime {
            return false;
        }
        self.claude_json_mtime = new_mtime;

        let is_light = read_claude_theme_is_light();
        let theme_name = if is_light { "Light" } else { "Dark" };

        if self.current_theme.name == theme_name {
            return false;
        }

        if let Some(theme) = self.theme_registry.find(theme_name).cloned() {
            info!("Claude theme changed to '{}', switching terminal theme", theme_name);
            self.current_palette = theme.to_color_palette();
            self.color_scheme = ColorScheme::from_is_light(theme.is_light);
            self.current_theme = theme;
            true
        } else {
            false
        }
    }

    /// Apply a new font size temporarily (for View menu), without updating settings
    pub fn apply_font_size_temporary(&mut self, new_size: f32) {
        self.font_size = new_size;
        self.apply_font_size_internal(new_size);
    }

    /// Apply a new font size permanently, updating settings
    pub fn apply_font_size(&mut self, new_size: f32) {
        self.font_size = new_size;
        self.settings.font_size = new_size;
        self.apply_font_size_internal(new_size);
    }

    /// Get the initial font size the app started with
    pub fn initial_font_size(&self) -> f32 {
        self.initial_font_size
    }

    /// Internal font size application (shared logic)
    fn apply_font_size_internal(&mut self, new_size: f32) {
        if let Some(ref window) = self.window {
            let scale_factor = window.scale_factor();

            match GlyphCache::new(scale_factor, new_size) {
                Ok(cache) => {
                    let cell_width = cache.cell_width() as f32;
                    let cell_height = cache.cell_height() as f32;
                    self.cached_char_width.set(cell_width);
                    self.cached_line_height.set(cell_height);
                    info!(
                        "Glyph cache recreated for font_size={}: cell_width={:.2}, cell_height={:.2}",
                        new_size, cell_width, cell_height
                    );
                    *self.glyph_cache.borrow_mut() = Some(cache);

                    let size = window.inner_size();
                    let width = (size.width as f64 / scale_factor) as f32;
                    let height = (size.height as f64 / scale_factor) as f32;
                    let inner_margin = 8.0_f32 * 2.0;

                    let cols = ((width - inner_margin) / cell_width).max(10.0) as u16;
                    let rows = ((height - inner_margin - TAB_BAR_HEIGHT) / cell_height).max(5.0) as u16;

                    debug!(
                        "Font size change resize: {:.0}x{:.0} logical -> {}cols x {}rows",
                        width, height, cols, rows
                    );

                    for session_info in self.session_manager.iter() {
                        let sess = session_info.session.lock();
                        sess.resize(cols as usize, rows as usize);
                    }

                    if let Some(callback) = &self.resize_callback {
                        callback(rows, cols);
                    }
                }
                Err(e) => {
                    error!("Failed to recreate glyph cache: {}. Using estimated metrics.", e);
                    let estimated_char_width = new_size * 0.6;
                    let estimated_line_height = new_size * 1.3;
                    self.cached_char_width.set(estimated_char_width);
                    self.cached_line_height.set(estimated_line_height);
                    self.invalidate_glyph_cache();
                }
            }
        }
    }

    /// Get selected text from terminal
    pub fn get_selection_text(&self) -> Option<String> {
        let (start, end) = match (self.selection_start, self.selection_end) {
            (Some(s), Some(e)) => (s, e),
            _ => return None,
        };

        let (start, end) = if start.0 < end.0 || (start.0 == end.0 && start.1 <= end.1) {
            (start, end)
        } else {
            (end, start)
        };

        let session_info = self.session_manager.active_session()?;
        let session = session_info.session.lock();
        let text = session.with_terminal_mut(|term| {
            let screen = term.screen_mut();
            let mut result = String::new();
            let total_lines = screen.scrollback_rows();
            let cols = screen.physical_cols;

            for phys_idx in start.0..=end.0 {
                if phys_idx < 0 || phys_idx as usize >= total_lines {
                    continue;
                }
                let start_col = if phys_idx == start.0 { start.1 } else { 0 };
                let end_col = if phys_idx == end.0 { end.1 } else { cols };

                let line = screen.line_mut(phys_idx as usize);
                for cell in line.visible_cells() {
                    let col = cell.cell_index();
                    if col >= start_col && col < end_col {
                        result.push_str(cell.str());
                    }
                }

                if phys_idx < end.0 {
                    let trimmed = result.trim_end_matches(' ');
                    result.truncate(trimmed.len());
                    result.push('\n');
                }
            }

            result.trim_end().to_string()
        });
        drop(session);

        if text.is_empty() {
            None
        } else {
            Some(text)
        }
    }

    /// Convert screen position (in logical pixels) to terminal cell coordinates
    fn screen_to_terminal_coords(&self, x: f64, y: f64) -> (i64, usize) {
        let char_width = self.cached_char_width.get();
        let line_height = self.cached_line_height.get();

        let tab_bar_height = TAB_BAR_HEIGHT as f64;
        let padding = 8.0;
        let x = (x - padding).max(0.0);
        let y = (y - tab_bar_height - padding).max(0.0);

        let col = (x / char_width as f64) as usize;
        let visible_row = (y / line_height as f64) as usize;

        let scroll_offset = self.scroll_offset.load(Ordering::Relaxed) as usize;

        if let Some(session_info) = self.session_manager.active_session() {
            let session = session_info.session.lock();
            let phys_row = session.with_terminal(|term| {
                let screen = term.screen();
                let total_lines = screen.scrollback_rows();
                let physical_rows = screen.physical_rows;
                let visible_start = total_lines.saturating_sub(physical_rows + scroll_offset);
                (visible_start + visible_row) as i64
            });
            drop(session);
            (phys_row, col)
        } else {
            (0, col)
        }
    }

    /// Handle mouse button press for selection or hyperlink click
    fn handle_mouse_press(&mut self, x: f64, y: f64) -> bool {
        let (row, col) = self.screen_to_terminal_coords(x, y);

        if let Some(hyperlink) = self.get_hyperlink_at(row as usize, col) {
            let state = self.modifiers.state();
            #[cfg(target_os = "macos")]
            let should_open = state.super_key();
            #[cfg(not(target_os = "macos"))]
            let should_open = state.control_key();

            if should_open {
                open_url(hyperlink.uri());
                return true;
            }
        }

        self.selection_start = Some((row, col));
        self.selection_end = Some((row, col));
        self.is_selecting = true;
        false
    }

    fn handle_mouse_release(&mut self) {
        self.is_selecting = false;

        #[cfg(target_os = "macos")]
        {
            let has_selection = self.has_selection();
            let has_clipboard = arboard::Clipboard::new()
                .ok()
                .and_then(|mut c| c.get_text().ok())
                .map(|t| !t.is_empty())
                .unwrap_or(false);
            update_edit_menu_state(has_selection, has_clipboard);
        }
    }

    fn handle_mouse_move(&mut self, x: f64, y: f64) {
        self.cursor_position = Some((x, y));

        let (row, col) = self.screen_to_terminal_coords(x, y);
        self.hovered_hyperlink = self.get_hyperlink_at(row as usize, col);

        if self.is_selecting {
            self.selection_end = Some((row, col));
        }
    }

    fn get_hyperlink_at(&self, row: usize, col: usize) -> Option<Arc<Hyperlink>> {
        let session_info = self.session_manager.active_session()?;
        session_info.session.lock().with_terminal_mut(|term| {
            let screen = term.screen_mut();
            let total_lines = screen.scrollback_rows();
            if row >= total_lines {
                return None;
            }
            let line = screen.line_mut(row);
            for cell in line.visible_cells() {
                if cell.cell_index() == col {
                    return cell.attrs().hyperlink().cloned();
                }
            }
            None
        })
    }

    /// Create the window (call from resumed handler)
    pub fn create_window(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        info!("Creating terminal window");

        let geometry = &self.settings.window_geometry;
        let mut window_attrs = WindowAttributes::default()
            .with_title("Agent Deck")
            .with_inner_size(LogicalSize::new(geometry.width, geometry.height))
            .with_visible(false);

        if let (Some(x), Some(y)) = (geometry.x, geometry.y) {
            window_attrs = window_attrs.with_position(LogicalPosition::new(x, y));
        }

        let template = ConfigTemplateBuilder::new()
            .with_alpha_size(8)
            .with_transparency(false);

        let display_builder = DisplayBuilder::new().with_window_attributes(Some(window_attrs));

        let (window, gl_config) = match display_builder.build(event_loop, template, |configs| {
            configs
                .reduce(|accum, config| {
                    if config.num_samples() > accum.num_samples() {
                        config
                    } else {
                        accum
                    }
                })
                .unwrap()
        }) {
            Ok((Some(window), config)) => (window, config),
            Ok((None, _)) => {
                error!("Failed to create window");
                return;
            }
            Err(e) => {
                error!("Failed to create window: {}", e);
                return;
            }
        };

        let window = Arc::new(window);
        self.window_id = Some(window.id());

        let context_attrs =
            ContextAttributesBuilder::new().build(window.window_handle().ok().map(|h| h.as_raw()));

        let gl_display = gl_config.display();

        let gl_context = unsafe {
            gl_display
                .create_context(&gl_config, &context_attrs)
                .expect("Failed to create OpenGL context")
        };

        let size = window.inner_size();
        let surface_attrs = SurfaceAttributesBuilder::<WindowSurface>::new().build(
            window.window_handle().unwrap().as_raw(),
            NonZeroU32::new(size.width.max(1)).unwrap(),
            NonZeroU32::new(size.height.max(1)).unwrap(),
        );

        let gl_surface = unsafe {
            gl_display
                .create_window_surface(&gl_config, &surface_attrs)
                .expect("Failed to create OpenGL surface")
        };

        let gl_context = gl_context
            .make_current(&gl_surface)
            .expect("Failed to make context current");

        let glow_context = unsafe {
            glow::Context::from_loader_function_cstr(|s| gl_display.get_proc_address(s) as *const _)
        };
        let glow_context = Arc::new(glow_context);

        let egui_glow = EguiGlow::new(event_loop, glow_context.clone(), None, None, false);

        let scale_factor = window.scale_factor();
        debug!("Initializing glyph cache with scale_factor {}", scale_factor);

        // Configure egui fonts
        {
            let mut fonts = egui::FontDefinitions::default();
            let jetbrains_mono = include_bytes!("../../assets/fonts/JetBrainsMono-Regular.ttf");
            fonts.font_data.insert(
                "JetBrainsMono".to_owned(),
                egui::FontData::from_static(jetbrains_mono).into(),
            );
            fonts
                .families
                .entry(egui::FontFamily::Monospace)
                .or_default()
                .insert(0, "JetBrainsMono".to_owned());
            egui_glow.egui_ctx.set_fonts(fonts);
        }

        match GlyphCache::new(scale_factor, self.font_size) {
            Ok(cache) => {
                let cell_width = cache.cell_width() as f32;
                let cell_height = cache.cell_height() as f32;
                self.cached_char_width.set(cell_width);
                self.cached_line_height.set(cell_height);
                info!(
                    "WezTerm glyph cache initialized: cell_width={:.2}, cell_height={:.2}",
                    cell_width, cell_height
                );
                *self.glyph_cache.borrow_mut() = Some(cache);
            }
            Err(e) => {
                error!("Failed to initialize glyph cache: {}. Will use egui text rendering.", e);
            }
        }

        self.window = Some(window);
        self.gl_context = Some(gl_context);
        self.gl_surface = Some(gl_surface);
        self.glow_context = Some(glow_context);
        self.egui_glow = Some(egui_glow);

        if let Some(ref window) = self.window {
            let initial_size = window.inner_size();
            self.handle_resize(initial_size.width, initial_size.height);
        }

        self.update_window_title();

        info!("Terminal window created");
    }

    /// Get the current window geometry (size and position)
    pub fn get_window_geometry(&self) -> Option<crate::core::settings::WindowGeometry> {
        let window = self.window.as_ref()?;
        let size = window.inner_size();
        let scale_factor = window.scale_factor();

        let logical_width = size.width as f64 / scale_factor;
        let logical_height = size.height as f64 / scale_factor;

        let position = window.outer_position().ok().map(|pos| {
            let logical_x = (pos.x as f64 / scale_factor) as i32;
            let logical_y = (pos.y as f64 / scale_factor) as i32;
            (logical_x, logical_y)
        });

        Some(crate::core::settings::WindowGeometry {
            width: logical_width,
            height: logical_height,
            x: position.map(|(x, _)| x),
            y: position.map(|(_, y)| y),
        })
    }

    /// Handle window event - returns true if event was consumed
    pub fn handle_window_event(&mut self, event: &WindowEvent) -> bool {
        let egui_should_handle_keyboard = self.settings_modal.is_open
            || self.session_manager.active_session().map_or(true, |s| !s.is_running);

        let should_pass_to_egui = match event {
            WindowEvent::KeyboardInput { .. } => egui_should_handle_keyboard,
            _ => true,
        };

        if should_pass_to_egui {
            if let Some(ref mut egui_glow) = self.egui_glow {
                let response = egui_glow.on_window_event(self.window.as_ref().unwrap(), event);
                if response.repaint {
                    if let Some(ref window) = self.window {
                        window.request_redraw();
                    }
                }
            }
        }

        match event {
            WindowEvent::ModifiersChanged(new_modifiers) => {
                self.modifiers = new_modifiers.clone();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if self.settings_modal.is_open {
                    return false;
                }

                if event.state == ElementState::Pressed {
                    if let Key::Named(NamedKey::Escape) = &event.logical_key {
                        if self.context_menu.is_open {
                            self.close_context_menu();
                            if let Some(ref window) = self.window {
                                window.request_redraw();
                            }
                            return true;
                        }
                    }

                    if self.context_menu.is_open {
                        self.close_context_menu();
                        if let Some(ref window) = self.window {
                            window.request_redraw();
                        }
                    }

                    let state = self.modifiers.state();
                    if state.super_key() && !state.control_key() && !state.alt_key() {
                        if let Key::Character(c) = &event.logical_key {
                            match c.as_str() {
                                "t" | "T" => {
                                    self.pending_actions.push(TerminalAction::NewTab);
                                    return true;
                                }
                                "w" | "W" => {
                                    if let Some(id) = self.session_manager.active_session_id() {
                                        self.pending_actions.push(TerminalAction::CloseTab(id));
                                    }
                                    return true;
                                }
                                "," => {
                                    self.settings_modal.open(&self.settings);
                                    return true;
                                }
                                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" => {
                                    let idx = c.chars().next().unwrap().to_digit(10).unwrap() as usize - 1;
                                    let sessions = self.session_manager.sessions();
                                    if idx < sessions.len() {
                                        let id = sessions[idx].id;
                                        self.pending_actions.push(TerminalAction::SwitchTab(id));
                                    }
                                    return true;
                                }
                                _ => {}
                            }
                        }
                    }

                    if let Key::Named(NamedKey::F20) = &event.logical_key {
                        self.pending_actions.push(TerminalAction::NewTab);
                        return true;
                    }

                    if let Some(session) = self.session_manager.active_session() {
                        if session.is_running {
                            self.handle_key_input(event);
                            return true;
                        }
                    }
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                if self.context_menu.is_open {
                    return true;
                }
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => *y as i32 * 3,
                    MouseScrollDelta::PixelDelta(pos) => (pos.y / 20.0) as i32,
                };
                if lines != 0 {
                    self.scroll_view(lines);
                    if let Some(ref window) = self.window {
                        window.request_redraw();
                    }
                }
                return true;
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if *button == MouseButton::Left {
                    if *state == ElementState::Pressed {
                        if !self.context_menu.is_open {
                            if let Some((x, y)) = self.cursor_position {
                                self.handle_mouse_press(x, y);
                                if let Some(ref window) = self.window {
                                    window.request_redraw();
                                }
                            }
                        }
                    } else {
                        if !self.context_menu.is_open {
                            self.handle_mouse_release();
                        }
                    }
                    return true;
                } else if *button == MouseButton::Right && *state == ElementState::Pressed {
                    if let Some(session) = self.session_manager.active_session() {
                        if !session.is_new_tab() {
                            if let Some((x, y)) = self.cursor_position {
                                self.open_context_menu(x as f32, y as f32);
                                if let Some(ref window) = self.window {
                                    window.request_redraw();
                                }
                            }
                        }
                    }
                    return true;
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let scale_factor = self.window.as_ref().map(|w| w.scale_factor()).unwrap_or(1.0);
                let logical_x = position.x / scale_factor;
                let logical_y = position.y / scale_factor;
                self.handle_mouse_move(logical_x, logical_y);
                if self.is_selecting {
                    if let Some(ref window) = self.window {
                        window.request_redraw();
                    }
                }
            }
            WindowEvent::Focused(focused) => {
                if *focused {
                    #[cfg(target_os = "macos")]
                    {
                        let has_selection = self.has_selection();
                        let has_clipboard = arboard::Clipboard::new()
                            .ok()
                            .and_then(|mut c| c.get_text().ok())
                            .map(|t| !t.is_empty())
                            .unwrap_or(false);
                        update_edit_menu_state(has_selection, has_clipboard);
                    }
                }
            }
            _ => {}
        }

        false
    }

    fn handle_key_input(&mut self, event: &winit::event::KeyEvent) {
        let state = self.modifiers.state();
        let ctrl = state.control_key();
        let alt = state.alt_key();
        let shift = state.shift_key();
        let super_key = state.super_key();

        if super_key && !ctrl && !alt {
            if let Key::Character(c) = &event.logical_key {
                match c.as_str() {
                    "v" | "V" => {
                        if let Ok(mut clipboard) = Clipboard::new() {
                            if let Ok(text) = clipboard.get_text() {
                                self.send_to_pty(text.as_bytes());
                            }
                        }
                        return;
                    }
                    "c" | "C" => {
                        if let Some(text) = self.get_selection_text() {
                            if let Ok(mut clipboard) = Clipboard::new() {
                                let _ = clipboard.set_text(&text);
                            }
                            self.clear_selection();
                        }
                        return;
                    }
                    "a" | "A" => {
                        self.select_all();
                        return;
                    }
                    _ => {}
                }
            }
        }

        let modifiers = encode_modifiers(shift, alt, ctrl);

        let bytes: Option<Vec<u8>> = match &event.logical_key {
            Key::Named(named) => match named {
                NamedKey::Enter => {
                    if shift {
                        Some(vec![b'\n'])
                    } else if alt {
                        Some(vec![0x1b, b'\r'])
                    } else {
                        Some(vec![b'\r'])
                    }
                }
                NamedKey::Backspace => {
                    if alt {
                        Some(vec![0x1b, 0x7f])
                    } else if ctrl {
                        Some(vec![0x17])
                    } else {
                        Some(vec![0x7f])
                    }
                }
                NamedKey::Tab => {
                    if shift {
                        Some(vec![0x1b, b'[', b'Z'])
                    } else {
                        Some(vec![b'\t'])
                    }
                }
                NamedKey::Escape => Some(vec![0x1b]),
                NamedKey::ArrowUp => Some(build_arrow_seq(modifiers, b'A')),
                NamedKey::ArrowDown => Some(build_arrow_seq(modifiers, b'B')),
                NamedKey::ArrowRight => Some(build_arrow_seq(modifiers, b'C')),
                NamedKey::ArrowLeft => Some(build_arrow_seq(modifiers, b'D')),
                NamedKey::Home => Some(build_home_end_seq(modifiers, b'H')),
                NamedKey::End => Some(build_home_end_seq(modifiers, b'F')),
                NamedKey::PageUp => Some(build_tilde_seq(modifiers, b"5")),
                NamedKey::PageDown => Some(build_tilde_seq(modifiers, b"6")),
                NamedKey::Delete => Some(build_tilde_seq(modifiers, b"3")),
                NamedKey::Insert => Some(build_tilde_seq(modifiers, b"2")),
                NamedKey::Space => {
                    if ctrl {
                        Some(vec![0x00])
                    } else if alt {
                        Some(vec![0x1b, b' '])
                    } else {
                        Some(vec![b' '])
                    }
                }
                NamedKey::F1 => Some(build_f1_f4_seq(modifiers, b'P')),
                NamedKey::F2 => Some(build_f1_f4_seq(modifiers, b'Q')),
                NamedKey::F3 => Some(build_f1_f4_seq(modifiers, b'R')),
                NamedKey::F4 => Some(build_f1_f4_seq(modifiers, b'S')),
                NamedKey::F5 => Some(build_tilde_seq(modifiers, b"15")),
                NamedKey::F6 => Some(build_tilde_seq(modifiers, b"17")),
                NamedKey::F7 => Some(build_tilde_seq(modifiers, b"18")),
                NamedKey::F8 => Some(build_tilde_seq(modifiers, b"19")),
                NamedKey::F9 => Some(build_tilde_seq(modifiers, b"20")),
                NamedKey::F10 => Some(build_tilde_seq(modifiers, b"21")),
                NamedKey::F11 => Some(build_tilde_seq(modifiers, b"23")),
                NamedKey::F12 => Some(build_tilde_seq(modifiers, b"24")),
                _ => None,
            },
            Key::Character(c) => {
                let s = c.as_str();
                if ctrl && s.len() == 1 {
                    let ch = s.chars().next().unwrap();
                    match ch.to_ascii_lowercase() {
                        'a'..='z' => {
                            let ctrl_char = (ch.to_ascii_lowercase() as u8) - b'a' + 1;
                            if alt {
                                Some(vec![0x1b, ctrl_char])
                            } else {
                                Some(vec![ctrl_char])
                            }
                        }
                        '[' => Some(vec![0x1b]),
                        '\\' => Some(vec![0x1c]),
                        ']' => Some(vec![0x1d]),
                        '^' | '6' => Some(vec![0x1e]),
                        '_' | '-' => Some(vec![0x1f]),
                        '@' | '2' => Some(vec![0x00]),
                        '/' => {
                            if alt {
                                Some(vec![0x1b, 0x1f])
                            } else {
                                Some(vec![0x1f])
                            }
                        }
                        _ => Some(s.as_bytes().to_vec()),
                    }
                } else if alt && !ctrl && !s.is_empty() {
                    let mut bytes = vec![0x1b];
                    bytes.extend_from_slice(s.as_bytes());
                    Some(bytes)
                } else if s.len() == 1 {
                    let ch = s.chars().next().unwrap();
                    if (ch as u32) < 0x20 {
                        Some(vec![ch as u8])
                    } else {
                        Some(s.as_bytes().to_vec())
                    }
                } else {
                    Some(s.as_bytes().to_vec())
                }
            }
            _ => None,
        };

        let bytes = bytes.or_else(|| {
            if let PhysicalKey::Code(key_code) = event.physical_key {
                match key_code {
                    KeyCode::KeyA if ctrl => Some(if alt { vec![0x1b, 0x01] } else { vec![0x01] }),
                    KeyCode::KeyB if ctrl => Some(if alt { vec![0x1b, 0x02] } else { vec![0x02] }),
                    KeyCode::KeyC if ctrl => Some(if alt { vec![0x1b, 0x03] } else { vec![0x03] }),
                    KeyCode::KeyD if ctrl => Some(if alt { vec![0x1b, 0x04] } else { vec![0x04] }),
                    KeyCode::KeyE if ctrl => Some(if alt { vec![0x1b, 0x05] } else { vec![0x05] }),
                    KeyCode::KeyF if ctrl => Some(if alt { vec![0x1b, 0x06] } else { vec![0x06] }),
                    KeyCode::KeyG if ctrl => Some(if alt { vec![0x1b, 0x07] } else { vec![0x07] }),
                    KeyCode::KeyH if ctrl => Some(if alt { vec![0x1b, 0x08] } else { vec![0x08] }),
                    KeyCode::KeyI if ctrl => Some(if alt { vec![0x1b, 0x09] } else { vec![0x09] }),
                    KeyCode::KeyJ if ctrl => Some(if alt { vec![0x1b, 0x0a] } else { vec![0x0a] }),
                    KeyCode::KeyK if ctrl => Some(if alt { vec![0x1b, 0x0b] } else { vec![0x0b] }),
                    KeyCode::KeyL if ctrl => Some(if alt { vec![0x1b, 0x0c] } else { vec![0x0c] }),
                    KeyCode::KeyM if ctrl => Some(if alt { vec![0x1b, 0x0d] } else { vec![0x0d] }),
                    KeyCode::KeyN if ctrl => Some(if alt { vec![0x1b, 0x0e] } else { vec![0x0e] }),
                    KeyCode::KeyO if ctrl => Some(if alt { vec![0x1b, 0x0f] } else { vec![0x0f] }),
                    KeyCode::KeyP if ctrl => Some(if alt { vec![0x1b, 0x10] } else { vec![0x10] }),
                    KeyCode::KeyQ if ctrl => Some(if alt { vec![0x1b, 0x11] } else { vec![0x11] }),
                    KeyCode::KeyR if ctrl => Some(if alt { vec![0x1b, 0x12] } else { vec![0x12] }),
                    KeyCode::KeyS if ctrl => Some(if alt { vec![0x1b, 0x13] } else { vec![0x13] }),
                    KeyCode::KeyT if ctrl => Some(if alt { vec![0x1b, 0x14] } else { vec![0x14] }),
                    KeyCode::KeyU if ctrl => Some(if alt { vec![0x1b, 0x15] } else { vec![0x15] }),
                    KeyCode::KeyV if ctrl => Some(if alt { vec![0x1b, 0x16] } else { vec![0x16] }),
                    KeyCode::KeyW if ctrl => Some(if alt { vec![0x1b, 0x17] } else { vec![0x17] }),
                    KeyCode::KeyX if ctrl => Some(if alt { vec![0x1b, 0x18] } else { vec![0x18] }),
                    KeyCode::KeyY if ctrl => Some(if alt { vec![0x1b, 0x19] } else { vec![0x19] }),
                    KeyCode::KeyZ if ctrl => Some(if alt { vec![0x1b, 0x1a] } else { vec![0x1a] }),
                    _ => None,
                }
            } else {
                None
            }
        });

        if let Some(ref data) = bytes {
            debug!("Sending to PTY: {:?}", data);
            self.scroll_to_bottom();
            self.clear_selection();
            self.send_to_pty(data);
        }
    }

    pub fn handle_resize(&mut self, physical_width: u32, physical_height: u32) {
        if let (Some(ref gl_context), Some(ref gl_surface)) = (&self.gl_context, &self.gl_surface) {
            gl_surface.resize(
                gl_context,
                NonZeroU32::new(physical_width.max(1)).unwrap(),
                NonZeroU32::new(physical_height.max(1)).unwrap(),
            );
        }

        let scale_factor = self.window.as_ref().map(|w| w.scale_factor()).unwrap_or(1.0);
        let width = (physical_width as f64 / scale_factor) as f32;
        let height = (physical_height as f64 / scale_factor) as f32;

        let char_width = self.cached_char_width.get();
        let line_height = self.cached_line_height.get();
        let inner_margin = 8.0_f32 * 2.0;
        let tab_bar_height = TAB_BAR_HEIGHT;

        let cols = ((width - inner_margin) / char_width).max(10.0) as u16;
        let rows = ((height - inner_margin - tab_bar_height) / line_height).max(5.0) as u16;

        debug!("Window resize: {:.0}x{:.0} logical -> {}cols x {}rows", width, height, cols, rows);

        for session_info in self.session_manager.iter() {
            let sess = session_info.session.lock();
            sess.resize(cols as usize, rows as usize);
        }

        if let Some(ref callback) = self.resize_callback {
            callback(rows, cols);
        }
    }

    pub fn cache_font_metrics(&self, char_width: f32, line_height: f32) {
        self.cached_char_width.set(char_width);
        self.cached_line_height.set(line_height);
    }

    /// Render the window
    pub fn render(&mut self) {
        self.process_notifications();
        self.process_terminal_responses();

        if !self.is_visible() {
            return;
        }

        let Some(ref window) = self.window else {
            return;
        };
        let Some(ref gl_context) = self.gl_context else {
            return;
        };
        let Some(ref gl_surface) = self.gl_surface else {
            return;
        };
        let Some(ref glow_context) = self.glow_context else {
            return;
        };

        let scroll_offset = self.scroll_offset.load(Ordering::Relaxed) as usize;
        let font_size = self.font_size;
        let color_scheme = self.color_scheme;
        let has_selection_for_menu = self.has_selection();

        let Some(ref mut egui_glow) = self.egui_glow else {
            return;
        };

        let selection = match (self.selection_start, self.selection_end) {
            (Some(start), Some(end)) => {
                let (start, end) = if start.0 < end.0 || (start.0 == end.0 && start.1 <= end.1) {
                    (start, end)
                } else {
                    (end, start)
                };
                Some((start, end))
            }
            _ => None,
        };

        let display_titles = self.session_manager.compute_display_titles(MAX_TAB_TITLE_LEN);

        let sessions_data: Vec<_> = self
            .session_manager
            .sessions()
            .iter()
            .map(|s| (
                s.id,
                display_titles.get(&s.id).cloned().unwrap_or_else(|| s.title.clone()),
                s.is_new_tab(),
                s.is_running,
                s.working_directory.display().to_string(),
                s.is_loading,
                s.terminal_title.clone(),
                s.bell_active,
                s.claude_activity,
                s.finished_in_background,
            ))
            .collect();
        let active_session_idx = self.session_manager.active_session_index();
        let hid_connected = self.hid_connected;

        let active_session_data = self.session_manager.active_session().map(|s| {
            (Arc::clone(&s.session), Arc::clone(&s.claude_state), s.is_new_tab(), s.id)
        });

        let bookmark_manager = self.bookmark_manager.clone();

        let mut new_actions = Vec::new();

        let need_install_loaders = !self.image_loaders_installed;

        let render_params = RenderParams {
            scroll_offset,
            font_size,
            color_scheme,
            current_theme: &self.current_theme,
            has_selection_for_menu,
            sessions_data,
            active_session_idx,
            hid_connected,
            active_session_data,
            bookmark_manager,
            selection,
            cached_char_width: &self.cached_char_width,
            cached_line_height: &self.cached_line_height,
            glyph_cache: &self.glyph_cache,
            hovered_hyperlink: &self.hovered_hyperlink,
        };

        egui_glow.run(window, |ctx| {
            // Tab bar at top
            egui::TopBottomPanel::top("tab_bar")
                .frame(egui::Frame::default().fill(color_scheme.tab_bar_background()))
                .exact_height(TAB_BAR_HEIGHT)
                .show(ctx, |ui| {
                    render_tab_bar(ui, ctx, &render_params, &mut new_actions, need_install_loaders);
                });

            // Main content area - use theme background for terminal area
            let terminal_bg = self.current_theme.background_color32();
            egui::CentralPanel::default()
                .frame(
                    egui::Frame::default()
                        .fill(terminal_bg)
                        .inner_margin(8.0),
                )
                .show(ctx, |ui| {
                    render_terminal_content(ui, ctx, &render_params, &mut new_actions);
                });

            // Hyperlink tooltip
            render_hyperlink_tooltip(ctx, &self.hovered_hyperlink);

            // Render settings modal
            let settings_result = render_settings_modal(ctx, &mut self.settings_modal);
            handle_settings_modal_result(settings_result, &mut new_actions);

            // Render context menu (if open)
            if self.context_menu.is_open {
                let context_actions = render_context_menu(
                    ctx,
                    &mut self.context_menu,
                    color_scheme,
                    has_selection_for_menu,
                );
                new_actions.extend(context_actions);
            }

            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        });

        // Handle OpenSettings action
        for action in &new_actions {
            if matches!(action, TerminalAction::OpenSettings) {
                self.settings_modal.open(&self.settings);
            }
        }

        self.pending_actions.extend(new_actions);

        if need_install_loaders {
            self.image_loaders_installed = true;
        }

        {
            use glow::HasContext as _;
            unsafe {
                glow_context.clear_color(0.1, 0.1, 0.1, 1.0);
                glow_context.clear(glow::COLOR_BUFFER_BIT);
            }
        }

        egui_glow.paint(window);
        gl_surface.swap_buffers(gl_context).unwrap();
    }

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
        use crate::core::sessions::ClaudeActivity;
        use wezterm_term::Alert;

        let mut title_changes: Vec<(SessionId, String)> = Vec::new();
        let mut activity_changes: Vec<(SessionId, ClaudeActivity)> = Vec::new();
        let mut bell_sessions: Vec<SessionId> = Vec::new();
        let active_session_id = self.session_manager.active_session_id();

        for session_info in self.session_manager.iter() {
            let session = session_info.session.lock();
            let alerts: Vec<Alert> = session.poll_notifications();
            for alert in alerts {
                match alert {
                    Alert::ToastNotification { title, body, focus } => {
                        info!(
                            "Terminal notification: title={:?}, body={}, focus={}",
                            title, body, focus
                        );
                    }
                    Alert::Bell => {
                        debug!("Terminal bell for session {} (active={:?})", session_info.id, active_session_id);
                        if Some(session_info.id) != active_session_id {
                            debug!("Adding bell indicator for background session {}", session_info.id);
                            bell_sessions.push(session_info.id);
                        }
                    }
                    Alert::CurrentWorkingDirectoryChanged => {
                        debug!("Working directory changed");
                    }
                    Alert::WindowTitleChanged(title) => {
                        debug!("Window title changed for session {}: {}", session_info.id, title);
                        let activity = crate::core::sessions::ClaudeActivity::from_title(&title);
                        activity_changes.push((session_info.id, activity));
                        let clean = clean_terminal_title(&title);
                        if !clean.is_empty() && clean != "Claude Code" {
                            title_changes.push((session_info.id, clean));
                        }
                    }
                    Alert::IconTitleChanged(title) => {
                        debug!("Icon title changed: {:?}", title);
                    }
                    Alert::TabTitleChanged(title) => {
                        debug!("Tab title changed for session {}: {:?}", session_info.id, title);
                        if let Some(t) = title {
                            let clean = clean_terminal_title(&t);
                            if !clean.is_empty() && clean != "Claude Code" {
                                title_changes.push((session_info.id, clean));
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

        for (session_id, clean_title) in title_changes {
            if let Some(session_info) = self.session_manager.get_session_mut(session_id) {
                session_info.terminal_title = Some(clean_title);

                if session_info.needs_session_resolution {
                    sessions_needing_resolution.push(session_id);
                }
            }
        }

        for session_id in sessions_needing_resolution {
            self.try_resolve_session_id(session_id);
        }

        let active_session_id = self.session_manager.active_session_id();
        for (session_id, activity) in activity_changes {
            if let Some(session_info) = self.session_manager.get_session_mut(session_id) {
                let was_working = session_info.claude_activity.is_working();
                let is_background = Some(session_id) != active_session_id;
                let stopped_working = !activity.is_working();

                if was_working && stopped_working && is_background {
                    session_info.finished_in_background = true;
                }

                session_info.claude_activity = activity;
            }
        }

        crate::update_working_session_count(self.session_manager.working_session_count());

        for session_id in bell_sessions {
            if let Some(session_info) = self.session_manager.get_session_mut(session_id) {
                session_info.bell_active = true;
            }
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

impl Default for TerminalWindowState {
    fn default() -> Self {
        Self::new(17.0)
    }
}

/// Clean terminal title by removing leading symbols/emojis
fn clean_terminal_title(title: &str) -> String {
    title
        .trim_start_matches(|c: char| !c.is_alphanumeric())
        .trim()
        .to_string()
}
