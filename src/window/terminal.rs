//! Terminal window using egui with WezTerm's terminal emulation
//!
//! Uses wezterm-term for full terminal emulation including escape sequence parsing,
//! cursor handling, scrollback, and all terminal features.
//! Supports multiple tabs with browser-style tab bar.

use super::glyph_cache::{GlyphCache, StyleKey};
use super::new_tab::{render_new_tab_page, NewTabAction};
use super::settings_modal::{render_settings_modal, SettingsModal, SettingsModalResult};
use arboard::Clipboard;

/// Claude icon SVG (white, for tinting)
const CLAUDE_ICON_SVG: &[u8] = include_bytes!("../../assets/icons/claude.svg");

/// Claude orange color
const CLAUDE_ORANGE: egui::Color32 = egui::Color32::from_rgb(0xD9, 0x77, 0x57);
use crate::core::bookmarks::BookmarkManager;
use crate::core::sessions::{SessionId, SessionManager};
use crate::core::settings::Settings;
use crate::core::state::ClaudeState;
use crate::terminal::Session;
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
use tokio::sync::mpsc;
use tracing::{debug, error, info};
use wezterm_cell::color::ColorAttribute;
use wezterm_cell::Hyperlink;
use wezterm_surface::CursorShape;
use wezterm_term::color::ColorPalette;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, Modifiers, MouseScrollDelta, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{Key, KeyCode, NamedKey, PhysicalKey};
use winit::window::{Window, WindowAttributes, WindowId};

/// Tab bar height in logical pixels
const TAB_BAR_HEIGHT: f32 = 32.0;

/// Maximum tab title length
const MAX_TAB_TITLE_LEN: usize = 20;

/// Convert ColorAttribute to egui Color32 using the provided palette
fn color_attr_to_egui(
    attr: ColorAttribute,
    palette: &ColorPalette,
    is_foreground: bool,
) -> egui::Color32 {
    let srgba = if is_foreground {
        palette.resolve_fg(attr)
    } else {
        palette.resolve_bg(attr)
    };
    egui::Color32::from_rgb(
        (srgba.0 * 255.0) as u8,
        (srgba.1 * 255.0) as u8,
        (srgba.2 * 255.0) as u8,
    )
}

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
    /// Open a directory in a new or current tab (resume = true to continue conversation)
    OpenDirectory { path: PathBuf, resume: bool },
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
    font_size: f32,
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
}

impl TerminalWindowState {
    pub fn new(font_size: f32) -> Self {
        // Estimate initial metrics based on font size (will be calibrated on first render)
        let estimated_char_width = font_size * 0.6;
        let estimated_line_height = font_size * 1.3;

        let settings = Settings::load().unwrap_or_default();
        let bookmark_manager = BookmarkManager::load().unwrap_or_default();

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
            selection_start: None,
            selection_end: None,
            is_selecting: false,
            cursor_position: None,
            glyph_cache: RefCell::new(None),
            hovered_hyperlink: None,
            pending_actions: Vec::new(),
            image_loaders_installed: false,
        }
    }

    /// Get pending actions and clear the queue
    pub fn take_pending_actions(&mut self) -> Vec<TerminalAction> {
        std::mem::take(&mut self.pending_actions)
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
                    "🤖".to_string()
                } else {
                    session.working_directory.display().to_string()
                }
            } else {
                "🤖".to_string()
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

    /// Check if there is an active selection
    pub fn has_selection(&self) -> bool {
        self.selection_start.is_some() && self.selection_end.is_some()
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

    /// Get selected text from terminal
    fn get_selection_text(&self) -> Option<String> {
        let (start, end) = match (self.selection_start, self.selection_end) {
            (Some(s), Some(e)) => (s, e),
            _ => return None,
        };

        // Normalize selection (start should be before end)
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

        // Account for tab bar and padding
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

        // Check for hyperlink
        if let Some(hyperlink) = self.get_hyperlink_at(row as usize, col) {
            let state = self.modifiers.state();
            #[cfg(target_os = "macos")]
            let should_open = state.super_key();
            #[cfg(not(target_os = "macos"))]
            let should_open = state.control_key();

            if should_open {
                self.open_url(hyperlink.uri());
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

    fn open_url(&self, url: &str) {
        info!("Opening URL: {}", url);
        #[cfg(target_os = "macos")]
        {
            let _ = std::process::Command::new("open").arg(url).spawn();
        }
        #[cfg(target_os = "linux")]
        {
            let _ = std::process::Command::new("xdg-open").arg(url).spawn();
        }
        #[cfg(target_os = "windows")]
        {
            let _ = std::process::Command::new("cmd")
                .args(["/C", "start", "", url])
                .spawn();
        }
    }

    /// Create the window (call from resumed handler)
    pub fn create_window(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        info!("Creating terminal window");

        let window_attrs = WindowAttributes::default()
            .with_title("Agent Deck")
            .with_inner_size(LogicalSize::new(1000.0, 700.0))
            .with_visible(false);

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

        match GlyphCache::new(scale_factor) {
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
                error!("Failed to initialize glyph cache: {}. Falling back to egui text rendering.", e);
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

        // Update title based on active session
        self.update_window_title();

        info!("Terminal window created");
    }

    /// Handle window event - returns true if event was consumed
    pub fn handle_window_event(&mut self, event: &WindowEvent) -> bool {
        if let Some(ref mut egui_glow) = self.egui_glow {
            let response = egui_glow.on_window_event(self.window.as_ref().unwrap(), event);
            if response.repaint {
                if let Some(ref window) = self.window {
                    window.request_redraw();
                }
            }
        }

        match event {
            WindowEvent::ModifiersChanged(new_modifiers) => {
                self.modifiers = new_modifiers.clone();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                // Don't handle key input if settings modal is open
                if self.settings_modal.is_open {
                    return false;
                }

                if event.state == ElementState::Pressed {
                    // Handle Cmd+T for new tab, Cmd+W for close tab
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
                                    // Cmd+, for settings
                                    self.settings_modal.open(&self.settings);
                                    return true;
                                }
                                // Cmd+1-9 for tab switching
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

                    // F20 (Claude button) opens new tab
                    if let Key::Named(NamedKey::F20) = &event.logical_key {
                        self.pending_actions.push(TerminalAction::NewTab);
                        return true;
                    }

                    // Only forward to PTY if we have an active running session
                    if let Some(session) = self.session_manager.active_session() {
                        if session.is_running {
                            self.handle_key_input(event);
                            return true;
                        }
                    }
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
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
                        if let Some((x, y)) = self.cursor_position {
                            self.handle_mouse_press(x, y);
                            if let Some(ref window) = self.window {
                                window.request_redraw();
                            }
                        }
                    } else {
                        self.handle_mouse_release();
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
            _ => {}
        }

        false
    }

    fn encode_modifiers(&self) -> Option<u8> {
        let state = self.modifiers.state();
        let mut code = 0u8;
        if state.shift_key() {
            code |= 1;
        }
        if state.alt_key() {
            code |= 2;
        }
        if state.control_key() {
            code |= 4;
        }
        if code == 0 {
            None
        } else {
            Some(code + 1)
        }
    }

    fn build_arrow_seq(&self, key_char: u8) -> Vec<u8> {
        match self.encode_modifiers() {
            Some(m) => vec![0x1b, b'[', b'1', b';', b'0' + m, key_char],
            None => vec![0x1b, b'[', key_char],
        }
    }

    fn build_home_end_seq(&self, key_char: u8) -> Vec<u8> {
        match self.encode_modifiers() {
            Some(m) => vec![0x1b, b'[', b'1', b';', b'0' + m, key_char],
            None => vec![0x1b, b'[', key_char],
        }
    }

    fn build_tilde_seq(&self, code: &[u8]) -> Vec<u8> {
        match self.encode_modifiers() {
            Some(m) => {
                let mut seq = vec![0x1b, b'['];
                seq.extend_from_slice(code);
                seq.push(b';');
                seq.push(b'0' + m);
                seq.push(b'~');
                seq
            }
            None => {
                let mut seq = vec![0x1b, b'['];
                seq.extend_from_slice(code);
                seq.push(b'~');
                seq
            }
        }
    }

    fn build_f1_f4_seq(&self, key_char: u8) -> Vec<u8> {
        match self.encode_modifiers() {
            Some(m) => vec![0x1b, b'[', b'1', b';', b'0' + m, key_char],
            None => vec![0x1b, b'O', key_char],
        }
    }

    fn handle_key_input(&mut self, event: &winit::event::KeyEvent) {
        self.scroll_to_bottom();

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
                NamedKey::ArrowUp => Some(self.build_arrow_seq(b'A')),
                NamedKey::ArrowDown => Some(self.build_arrow_seq(b'B')),
                NamedKey::ArrowRight => Some(self.build_arrow_seq(b'C')),
                NamedKey::ArrowLeft => Some(self.build_arrow_seq(b'D')),
                NamedKey::Home => Some(self.build_home_end_seq(b'H')),
                NamedKey::End => Some(self.build_home_end_seq(b'F')),
                NamedKey::PageUp => Some(self.build_tilde_seq(b"5")),
                NamedKey::PageDown => Some(self.build_tilde_seq(b"6")),
                NamedKey::Delete => Some(self.build_tilde_seq(b"3")),
                NamedKey::Insert => Some(self.build_tilde_seq(b"2")),
                NamedKey::Space => {
                    if ctrl {
                        Some(vec![0x00])
                    } else if alt {
                        Some(vec![0x1b, b' '])
                    } else {
                        Some(vec![b' '])
                    }
                }
                NamedKey::F1 => Some(self.build_f1_f4_seq(b'P')),
                NamedKey::F2 => Some(self.build_f1_f4_seq(b'Q')),
                NamedKey::F3 => Some(self.build_f1_f4_seq(b'R')),
                NamedKey::F4 => Some(self.build_f1_f4_seq(b'S')),
                NamedKey::F5 => Some(self.build_tilde_seq(b"15")),
                NamedKey::F6 => Some(self.build_tilde_seq(b"17")),
                NamedKey::F7 => Some(self.build_tilde_seq(b"18")),
                NamedKey::F8 => Some(self.build_tilde_seq(b"19")),
                NamedKey::F9 => Some(self.build_tilde_seq(b"20")),
                NamedKey::F10 => Some(self.build_tilde_seq(b"21")),
                NamedKey::F11 => Some(self.build_tilde_seq(b"23")),
                NamedKey::F12 => Some(self.build_tilde_seq(b"24")),
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

        // Resize all sessions
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
        // Process notifications for active session
        self.process_notifications();

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
        let Some(ref mut egui_glow) = self.egui_glow else {
            return;
        };

        let scroll_offset = self.scroll_offset.load(Ordering::Relaxed) as usize;
        let cached_char_width = &self.cached_char_width;
        let cached_line_height = &self.cached_line_height;
        let font_size = self.font_size;
        let glyph_cache = &self.glyph_cache;
        let hovered_hyperlink = &self.hovered_hyperlink;
        let color_scheme = self.settings.color_scheme;

        // Capture selection state for rendering
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

        // Collect data needed for rendering
        // Tuple: (id, title, is_new_tab, is_running, working_dir, is_loading)
        let sessions_data: Vec<_> = self
            .session_manager
            .sessions()
            .iter()
            .map(|s| (
                s.id,
                s.display_title(MAX_TAB_TITLE_LEN),
                s.is_new_tab(),
                s.is_running,
                s.working_directory.display().to_string(),
                s.is_loading,
            ))
            .collect();
        let active_session_idx = self.session_manager.active_session_index();
        let hid_connected = self.hid_connected;

        // Get active session data
        let active_session_data = self.session_manager.active_session().map(|s| {
            (Arc::clone(&s.session), Arc::clone(&s.claude_state), s.is_new_tab())
        });

        // Clone bookmark manager for new tab rendering
        let bookmark_manager = self.bookmark_manager.clone();

        // Track pending actions
        let mut new_actions = Vec::new();

        // Track if we need to install image loaders
        let need_install_loaders = !self.image_loaders_installed;

        egui_glow.run(window, |ctx| {
            // Install image loaders once (for SVG support)
            if need_install_loaders {
                egui_extras::install_image_loaders(ctx);
            }

            // Tab bar at top
            egui::TopBottomPanel::top("tab_bar")
                .frame(egui::Frame::default().fill(color_scheme.tab_bar_background()))
                .exact_height(TAB_BAR_HEIGHT)
                .show(ctx, |ui| {
                    // Override button visual style for tabs
                    ui.style_mut().visuals.widgets.inactive.bg_fill = color_scheme.inactive_tab_background();
                    ui.style_mut().visuals.widgets.hovered.bg_fill = color_scheme.inactive_tab_background();
                    ui.style_mut().visuals.widgets.active.bg_fill = color_scheme.active_tab_background();
                    ui.style_mut().visuals.widgets.inactive.weak_bg_fill = color_scheme.inactive_tab_background();
                    ui.style_mut().visuals.widgets.hovered.weak_bg_fill = color_scheme.inactive_tab_background();
                    ui.style_mut().visuals.widgets.active.weak_bg_fill = color_scheme.active_tab_background();
                    // Remove yellow/olive stroke
                    ui.style_mut().visuals.widgets.inactive.bg_stroke = egui::Stroke::NONE;
                    ui.style_mut().visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(80, 80, 80));
                    ui.style_mut().visuals.widgets.active.bg_stroke = egui::Stroke::NONE;

                    // Calculate available width for tabs
                    let total_width = ui.available_width();
                    let right_icons_width = 60.0; // Settings + indicator + padding
                    let new_tab_btn_width = 32.0;
                    let tabs_area_width = total_width - right_icons_width - new_tab_btn_width - 4.0;

                    // Calculate per-tab width
                    let num_tabs = sessions_data.len().max(1) as f32;
                    let min_tab_width = 100.0_f32;
                    let max_tab_width = 200.0_f32;
                    let tab_width = (tabs_area_width / num_tabs).clamp(min_tab_width, max_tab_width);

                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 1.0; // Minimal gap between tabs

                        // Render tabs
                        for (idx, (id, title, _is_new, is_running, working_dir, is_loading)) in sessions_data.iter().enumerate() {
                            let is_active = idx == active_session_idx;
                            let tab_bg = if is_active {
                                color_scheme.active_tab_background()
                            } else {
                                color_scheme.inactive_tab_background()
                            };
                            // Dim text for placeholder tabs (not yet started)
                            let text_color = if *is_running || *_is_new || *is_loading {
                                color_scheme.foreground()
                            } else {
                                // Dimmed for placeholder tabs
                                let fg = color_scheme.foreground();
                                egui::Color32::from_rgba_unmultiplied(fg.r(), fg.g(), fg.b(), 150)
                            };

                            // Render tab as a frame
                            let tab_frame = egui::Frame::none()
                                .fill(tab_bg)
                                .rounding(egui::Rounding {
                                    nw: 4.0,
                                    ne: 4.0,
                                    sw: 0.0,
                                    se: 0.0,
                                })
                                .inner_margin(egui::Margin::symmetric(4.0, 0.0));

                            let mut close_clicked = false;
                            let tab_response = tab_frame
                                .show(ui, |ui| {
                                    ui.set_width(tab_width);
                                    ui.set_height(TAB_BAR_HEIGHT - 4.0);

                                    // Use a horizontal layout with title taking remaining space
                                    ui.horizontal_centered(|ui| {
                                        ui.spacing_mut().item_spacing.x = 4.0;

                                        // Left padding
                                        ui.add_space(6.0);

                                        // Claude icon or loading spinner
                                        let icon_size = 14.0;
                                        if *is_loading {
                                            // Show spinning loader when loading
                                            let time = ui.input(|i| i.time);
                                            let angle = time * 3.0; // Rotate 3 radians per second
                                            let spinner_color = CLAUDE_ORANGE;

                                            // Draw a simple spinning arc
                                            let (rect, _) = ui.allocate_exact_size(
                                                egui::vec2(icon_size, icon_size),
                                                egui::Sense::hover(),
                                            );
                                            let center = rect.center();
                                            let radius = icon_size / 2.0 - 1.0;
                                            let painter = ui.painter();

                                            // Draw arc segments to create spinner effect
                                            let segments = 8;
                                            for i in 0..segments {
                                                let start_angle = angle as f32 + (i as f32 * std::f32::consts::TAU / segments as f32);
                                                let alpha = ((i as f32 / segments as f32) * 200.0) as u8 + 55;
                                                let color = egui::Color32::from_rgba_unmultiplied(
                                                    spinner_color.r(),
                                                    spinner_color.g(),
                                                    spinner_color.b(),
                                                    alpha,
                                                );
                                                let x = center.x + radius * start_angle.cos();
                                                let y = center.y + radius * start_angle.sin();
                                                painter.circle_filled(egui::pos2(x, y), 1.5, color);
                                            }
                                        } else {
                                            // Claude icon (orange when running, gray when not)
                                            let icon_tint = if *is_running {
                                                CLAUDE_ORANGE // Orange for running tabs
                                            } else {
                                                egui::Color32::from_gray(100) // Gray for inactive/new tabs
                                            };

                                            ui.add(
                                                egui::Image::from_bytes(
                                                    "bytes://claude-icon.svg",
                                                    CLAUDE_ICON_SVG,
                                                )
                                                .fit_to_exact_size(egui::vec2(icon_size, icon_size))
                                                .tint(icon_tint),
                                            );
                                        }

                                        // Tab title (centered, takes remaining space)
                                        ui.with_layout(
                                            egui::Layout::left_to_right(egui::Align::Center)
                                                .with_main_justify(true),
                                            |ui| {
                                                // Display loading text when loading
                                                let display_title = if *is_loading {
                                                    "Starting..."
                                                } else {
                                                    title.as_str()
                                                };
                                                ui.add(
                                                    egui::Label::new(
                                                        egui::RichText::new(display_title)
                                                            .size(13.0)
                                                            .color(text_color),
                                                    )
                                                    .selectable(false),
                                                );
                                            },
                                        );

                                        // Close button on the right
                                        let close_btn = ui.add(
                                            egui::Button::new(
                                                egui::RichText::new("×")
                                                    .size(14.0)
                                                    .color(egui::Color32::GRAY),
                                            )
                                            .frame(false)
                                            .fill(egui::Color32::TRANSPARENT)
                                            .min_size(egui::vec2(18.0, 18.0)),
                                        );
                                        if close_btn.clicked() {
                                            close_clicked = true;
                                            new_actions.push(TerminalAction::CloseTab(*id));
                                        }
                                    });
                                })
                                .response;

                            // Set cursor to pointer for tab (not text cursor)
                            // Check hovered OR contains_pointer (for when mouse is pressed)
                            if tab_response.hovered() || tab_response.contains_pointer() {
                                ctx.output_mut(|o| o.cursor_icon = egui::CursorIcon::PointingHand);
                            }

                            // Add tooltip with full directory path
                            let tab_response = tab_response.on_hover_text(working_dir);

                            // Click on tab background to switch (only if close wasn't clicked)
                            if !close_clicked && tab_response.interact(egui::Sense::click()).clicked() {
                                new_actions.push(TerminalAction::SwitchTab(*id));
                            }

                            // Middle-click to close
                            if tab_response.interact(egui::Sense::click()).middle_clicked() {
                                new_actions.push(TerminalAction::CloseTab(*id));
                            }
                        }

                        // New tab button
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new("+")
                                        .size(16.0)
                                        .color(color_scheme.foreground()),
                                )
                                .fill(color_scheme.inactive_tab_background())
                                .stroke(egui::Stroke::NONE)
                                .rounding(egui::Rounding {
                                    nw: 4.0,
                                    ne: 4.0,
                                    sw: 0.0,
                                    se: 0.0,
                                })
                                .min_size(egui::vec2(new_tab_btn_width, TAB_BAR_HEIGHT - 4.0)),
                            )
                            .clicked()
                        {
                            new_actions.push(TerminalAction::NewTab);
                        }

                        // Right side: settings and connection indicator
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.add_space(8.0);

                            // HID connection indicator
                            let (indicator_color, indicator_text) = if hid_connected {
                                (egui::Color32::from_rgb(50, 205, 50), "Connected")
                            } else {
                                (egui::Color32::from_rgb(220, 20, 60), "Disconnected")
                            };

                            ui.add(
                                egui::Button::new(
                                    egui::RichText::new("●").size(12.0).color(indicator_color),
                                )
                                .frame(false),
                            )
                            .on_hover_text(indicator_text);

                            ui.add_space(8.0);

                            // Settings button
                            if ui
                                .add(
                                    egui::Button::new(
                                        egui::RichText::new("⚙")
                                            .size(16.0)
                                            .color(color_scheme.foreground()),
                                    )
                                    .frame(false),
                                )
                                .on_hover_text("Settings (Cmd+,)")
                                .clicked()
                            {
                                new_actions.push(TerminalAction::OpenSettings);
                            }
                        });
                    });
                });

            // Main content area
            egui::CentralPanel::default()
                .frame(
                    egui::Frame::default()
                        .fill(color_scheme.background())
                        .inner_margin(8.0),
                )
                .show(ctx, |ui| {
                    if let Some((session, _claude_state, is_new_tab)) = &active_session_data {
                        if *is_new_tab {
                            // Render new tab page
                            if let Some(action) =
                                render_new_tab_page(ui, &bookmark_manager, color_scheme)
                            {
                                match action {
                                    NewTabAction::OpenDirectory { path, resume } => {
                                        new_actions.push(TerminalAction::OpenDirectory { path, resume });
                                    }
                                    NewTabAction::BrowseDirectory => {
                                        new_actions.push(TerminalAction::BrowseDirectory);
                                    }
                                    NewTabAction::AddBookmark(path) => {
                                        new_actions.push(TerminalAction::AddBookmark(path));
                                    }
                                    NewTabAction::RemoveBookmark(path) => {
                                        new_actions.push(TerminalAction::RemoveBookmark(path));
                                    }
                                    NewTabAction::RemoveRecent(path) => {
                                        new_actions.push(TerminalAction::RemoveRecent(path));
                                    }
                                    NewTabAction::ClearRecent => {
                                        new_actions.push(TerminalAction::ClearRecent);
                                    }
                                }
                            }
                        } else {
                            // Render terminal
                            ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
                            ui.style_mut().spacing.interact_size = egui::vec2(0.0, 0.0);

                            let sess = session.lock();
                            let palette = sess.palette().clone();

                            sess.with_terminal_mut(|term| {
                                let cursor = term.cursor_pos();
                                let screen = term.screen_mut();
                                let physical_rows = screen.physical_rows;

                                let content_min = ui.cursor().min;

                                let (char_width, line_height) = {
                                    let cache_ref = glyph_cache.borrow();
                                    if let Some(ref cache) = *cache_ref {
                                        (cache.cell_width() as f32, cache.cell_height() as f32)
                                    } else {
                                        let font_id = egui::FontId::monospace(font_size);
                                        (
                                            ctx.fonts(|f| f.glyph_width(&font_id, 'M')),
                                            ctx.fonts(|f| f.row_height(&font_id)),
                                        )
                                    }
                                };

                                cached_char_width.set(char_width);
                                cached_line_height.set(line_height);

                                let total_lines = screen.scrollback_rows();
                                let visible_start =
                                    total_lines.saturating_sub(physical_rows + scroll_offset);

                                let painter = ui.painter();
                                let default_bg = color_scheme.background();

                                let mut cells_to_render: Vec<(
                                    usize,
                                    usize,
                                    usize,
                                    String,
                                    egui::Color32,
                                    Option<egui::Color32>,
                                    bool,
                                    bool,
                                    bool,
                                    bool,
                                    bool,
                                )> = Vec::new();

                                for row_idx in 0..physical_rows {
                                    let phys_idx = visible_start + row_idx;
                                    if phys_idx >= total_lines {
                                        continue;
                                    }

                                    let line = screen.line_mut(phys_idx);
                                    let current_row = phys_idx as i64;

                                    for cell in line.visible_cells() {
                                        let col_idx = cell.cell_index();
                                        let attrs = cell.attrs();
                                        let mut fg =
                                            color_attr_to_egui(attrs.foreground(), &palette, true);
                                        let bg_attr = attrs.background();
                                        let mut bg = if bg_attr == ColorAttribute::Default {
                                            None
                                        } else {
                                            Some(color_attr_to_egui(bg_attr, &palette, false))
                                        };

                                        if let Some((sel_start, sel_end)) = selection {
                                            let in_selection = if sel_start.0 == sel_end.0 {
                                                current_row == sel_start.0
                                                    && col_idx >= sel_start.1
                                                    && col_idx < sel_end.1
                                            } else if current_row == sel_start.0 {
                                                col_idx >= sel_start.1
                                            } else if current_row == sel_end.0 {
                                                col_idx < sel_end.1
                                            } else {
                                                current_row > sel_start.0
                                                    && current_row < sel_end.0
                                            };

                                            if in_selection {
                                                fg = egui::Color32::WHITE;
                                                bg = Some(color_scheme.selection_background());
                                            }
                                        }

                                        use wezterm_cell::Intensity;
                                        let is_bold = matches!(attrs.intensity(), Intensity::Bold);
                                        match attrs.intensity() {
                                            Intensity::Bold => {
                                                fg = egui::Color32::from_rgb(
                                                    (fg.r() as u16 * 5 / 4).min(255) as u8,
                                                    (fg.g() as u16 * 5 / 4).min(255) as u8,
                                                    (fg.b() as u16 * 5 / 4).min(255) as u8,
                                                );
                                            }
                                            Intensity::Half => {
                                                fg = egui::Color32::from_rgb(
                                                    fg.r() / 2,
                                                    fg.g() / 2,
                                                    fg.b() / 2,
                                                );
                                            }
                                            Intensity::Normal => {}
                                        }

                                        if attrs.reverse() {
                                            let temp_fg = fg;
                                            fg = bg.unwrap_or(default_bg);
                                            bg = Some(temp_fg);
                                        }

                                        if attrs.invisible() {
                                            fg = bg.unwrap_or(default_bg);
                                        }

                                        let is_italic = attrs.italic();
                                        use wezterm_cell::Underline;

                                        let cell_hyperlink = attrs.hyperlink();
                                        let has_hyperlink = cell_hyperlink.is_some();

                                        let is_hovered_hyperlink =
                                            match (cell_hyperlink, hovered_hyperlink) {
                                                (Some(cell_link), Some(hovered_link)) => {
                                                    Arc::ptr_eq(cell_link, hovered_link)
                                                }
                                                _ => false,
                                            };

                                        let has_underline =
                                            attrs.underline() != Underline::None || has_hyperlink;
                                        if is_hovered_hyperlink {
                                            fg = egui::Color32::from_rgb(100, 149, 237);
                                        } else if has_hyperlink {
                                            fg = egui::Color32::from_rgb(80, 120, 200);
                                        }

                                        let has_strikethrough = attrs.strikethrough();

                                        let text = cell.str();
                                        let display_text = if text.is_empty() {
                                            " ".to_string()
                                        } else {
                                            text.to_string()
                                        };

                                        let cell_width = cell.width();

                                        cells_to_render.push((
                                            row_idx,
                                            col_idx,
                                            cell_width,
                                            display_text,
                                            fg,
                                            bg,
                                            is_bold,
                                            is_italic,
                                            has_underline,
                                            has_strikethrough,
                                            is_hovered_hyperlink,
                                        ));
                                    }
                                }

                                let mut cache_ref = glyph_cache.borrow_mut();
                                let use_glyph_cache = cache_ref.is_some();

                                for (
                                    row_idx,
                                    col_idx,
                                    cell_width,
                                    text,
                                    fg,
                                    bg,
                                    is_bold,
                                    is_italic,
                                    has_underline,
                                    has_strikethrough,
                                    _is_hovered_hyperlink,
                                ) in cells_to_render
                                {
                                    let cell_x = content_min.x + col_idx as f32 * char_width;
                                    let cell_y = content_min.y + row_idx as f32 * line_height;
                                    let total_cell_width = cell_width as f32 * char_width;
                                    let cell_rect = egui::Rect::from_min_size(
                                        egui::pos2(cell_x, cell_y),
                                        egui::vec2(total_cell_width, line_height),
                                    );

                                    if let Some(bg_color) = bg {
                                        painter.rect_filled(cell_rect, 0.0, bg_color);
                                    }

                                    let style_key = StyleKey::from_attrs(is_bold, is_italic);

                                    if use_glyph_cache {
                                        if let Some(ref mut cache) = *cache_ref {
                                            if let Some(glyph) =
                                                cache.get_glyph(ctx, &text, style_key)
                                            {
                                                let (glyph_rect, tint) = if glyph.has_color {
                                                    let glyph_w = glyph.width as f32;
                                                    let glyph_h = glyph.height as f32;
                                                    let scale_x = total_cell_width / glyph_w;
                                                    let scale_y = line_height / glyph_h;
                                                    let scale = scale_x.min(scale_y).min(1.0);

                                                    let scaled_w = glyph_w * scale;
                                                    let scaled_h = glyph_h * scale;

                                                    let offset_x =
                                                        (total_cell_width - scaled_w) / 2.0;
                                                    let offset_y =
                                                        (line_height - scaled_h) / 2.0;

                                                    let rect = egui::Rect::from_min_size(
                                                        egui::pos2(
                                                            cell_x + offset_x,
                                                            cell_y + offset_y,
                                                        ),
                                                        egui::vec2(scaled_w, scaled_h),
                                                    );
                                                    (rect, egui::Color32::WHITE)
                                                } else {
                                                    let glyph_x = cell_x + glyph.bearing_x as f32;
                                                    let baseline_y = cell_y + line_height * 0.8;
                                                    let glyph_y =
                                                        baseline_y - glyph.bearing_y as f32;

                                                    let rect = egui::Rect::from_min_size(
                                                        egui::pos2(glyph_x, glyph_y),
                                                        egui::vec2(
                                                            glyph.width as f32,
                                                            glyph.height as f32,
                                                        ),
                                                    );
                                                    (rect, fg)
                                                };

                                                painter.image(
                                                    glyph.texture.id(),
                                                    glyph_rect,
                                                    egui::Rect::from_min_max(
                                                        egui::pos2(0.0, 0.0),
                                                        egui::pos2(1.0, 1.0),
                                                    ),
                                                    tint,
                                                );
                                            }
                                        }
                                    } else {
                                        let font_id = egui::FontId::monospace(font_size);
                                        painter.text(
                                            egui::pos2(cell_x, cell_y),
                                            egui::Align2::LEFT_TOP,
                                            &text,
                                            font_id,
                                            fg,
                                        );
                                    }

                                    if has_underline {
                                        let underline_y = cell_y + line_height - 2.0;
                                        painter.line_segment(
                                            [
                                                egui::pos2(cell_x, underline_y),
                                                egui::pos2(cell_x + total_cell_width, underline_y),
                                            ],
                                            egui::Stroke::new(1.0, fg),
                                        );
                                    }

                                    if has_strikethrough {
                                        let strike_y = cell_y + line_height / 2.0;
                                        painter.line_segment(
                                            [
                                                egui::pos2(cell_x, strike_y),
                                                egui::pos2(cell_x + total_cell_width, strike_y),
                                            ],
                                            egui::Stroke::new(1.0, fg),
                                        );
                                    }
                                }

                                drop(cache_ref);

                                use wezterm_surface::CursorVisibility;
                                let cursor_in_bounds =
                                    cursor.y >= 0 && (cursor.y as usize) < physical_rows;
                                let cursor_visible =
                                    cursor.visibility == CursorVisibility::Visible;
                                let should_draw_cursor =
                                    scroll_offset == 0 && cursor_in_bounds && cursor_visible;

                                if should_draw_cursor {
                                    let cursor_pixel_x =
                                        content_min.x + cursor.x as f32 * char_width;
                                    let cursor_pixel_y =
                                        content_min.y + cursor.y as f32 * line_height;

                                    let cursor_color =
                                        egui::Color32::from_rgba_unmultiplied(200, 200, 200, 220);

                                    let cursor_rect = match cursor.shape {
                                        CursorShape::BlinkingBlock | CursorShape::SteadyBlock => {
                                            egui::Rect::from_min_size(
                                                egui::pos2(cursor_pixel_x, cursor_pixel_y),
                                                egui::vec2(char_width, line_height),
                                            )
                                        }
                                        CursorShape::BlinkingUnderline
                                        | CursorShape::SteadyUnderline => {
                                            egui::Rect::from_min_size(
                                                egui::pos2(
                                                    cursor_pixel_x,
                                                    cursor_pixel_y + line_height - 2.0,
                                                ),
                                                egui::vec2(char_width, 2.0),
                                            )
                                        }
                                        CursorShape::BlinkingBar | CursorShape::SteadyBar => {
                                            egui::Rect::from_min_size(
                                                egui::pos2(cursor_pixel_x, cursor_pixel_y),
                                                egui::vec2(2.0, line_height),
                                            )
                                        }
                                        _ => egui::Rect::from_min_size(
                                            egui::pos2(cursor_pixel_x, cursor_pixel_y),
                                            egui::vec2(char_width, line_height),
                                        ),
                                    };

                                    painter.rect_filled(cursor_rect, 0.0, cursor_color);
                                }
                            });
                        }
                    } else {
                        // No active session - show welcome message
                        ui.centered_and_justified(|ui| {
                            ui.label(
                                egui::RichText::new("No tabs open. Press + to create a new tab.")
                                    .size(16.0)
                                    .color(egui::Color32::GRAY),
                            );
                        });
                    }
                });

            // Hyperlink tooltip
            if let Some(ref hyperlink) = hovered_hyperlink {
                ctx.output_mut(|o| o.cursor_icon = egui::CursorIcon::PointingHand);

                egui::show_tooltip_at_pointer(
                    ctx,
                    egui::LayerId::background(),
                    egui::Id::new("hyperlink_tooltip"),
                    |ui: &mut egui::Ui| {
                        ui.set_min_width(400.0);
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                        ui.label(hyperlink.uri());
                        ui.add_space(4.0);
                        ui.weak(
                            #[cfg(target_os = "macos")]
                            "Cmd+click to open",
                            #[cfg(not(target_os = "macos"))]
                            "Ctrl+click to open",
                        );
                    },
                );
            }

            // Render settings modal (must be inside egui run block to receive input)
            let settings_result = render_settings_modal(ctx, &mut self.settings_modal);
            match settings_result {
                SettingsModalResult::Apply(settings) => {
                    new_actions.push(TerminalAction::ApplySettings(settings));
                }
                SettingsModalResult::Cancel => {}
                SettingsModalResult::None => {}
            }

            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        });

        // Handle OpenSettings action
        for action in &new_actions {
            if matches!(action, TerminalAction::OpenSettings) {
                self.settings_modal.open(&self.settings);
            }
        }

        // Store pending actions
        self.pending_actions.extend(new_actions);

        // Mark image loaders as installed
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

    pub fn process_notifications(&self) {
        use wezterm_term::Alert;

        if let Some(session_info) = self.session_manager.active_session() {
            let session = session_info.session.lock();
            for alert in session.poll_notifications() {
                match alert {
                    Alert::ToastNotification { title, body, focus } => {
                        info!(
                            "Terminal notification: title={:?}, body={}, focus={}",
                            title, body, focus
                        );
                    }
                    Alert::Bell => {
                        debug!("Terminal bell");
                    }
                    Alert::CurrentWorkingDirectoryChanged => {
                        debug!("Working directory changed");
                    }
                    Alert::WindowTitleChanged(title) => {
                        debug!("Window title changed: {}", title);
                    }
                    Alert::IconTitleChanged(title) => {
                        debug!("Icon title changed: {:?}", title);
                    }
                    Alert::TabTitleChanged(title) => {
                        debug!("Tab title changed: {:?}", title);
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
