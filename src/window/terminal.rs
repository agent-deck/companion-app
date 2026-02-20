//! Terminal window using egui with WezTerm's terminal emulation
//!
//! Uses wezterm-term for full terminal emulation including escape sequence parsing,
//! cursor handling, scrollback, and all terminal features.
//! Supports multiple tabs with browser-style tab bar.

use super::context_menu::{render_context_menu, ContextMenuState};
use super::glyph_cache::GlyphCache;
use super::render::{
    handle_settings_modal_result, render_hyperlink_tooltip, render_tab_bar,
    render_terminal_content, RenderParams, MAX_TAB_TITLE_LEN, TAB_BAR_HEIGHT,
};
use super::settings_modal::{render_settings_modal, SettingsModal};
use crate::hid::{DeviceMode, SoftKeyEditState};
use crate::core::bookmarks::BookmarkManager;
use crate::core::sessions::{SessionId, SessionManager};
use crate::core::settings::{ColorScheme, Settings};
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
use winit::event::Modifiers;
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowAttributes, WindowId};

use crate::core::claude_sessions::find_most_recent_session;

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
    /// Read soft key configs from device
    ReadSoftKeys,
    /// Apply soft key configs to device
    ApplySoftKeys([SoftKeyEditState; 3]),
    /// Reset soft keys to firmware defaults
    ResetSoftKeys,
    /// Send HID display update with session name, current task, tab states, and active index
    HidDisplayUpdate { session: String, task: Option<String>, tabs: Vec<u8>, active: usize },
    /// Send HID alert overlay for a background session
    HidAlert { tab: usize, session: String, text: String, details: Option<String> },
    /// Clear HID alert overlay for a tab
    HidClearAlert(usize),
    /// Set HID device LED mode
    HidSetMode(DeviceMode),
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
    pub(super) egui_glow: Option<EguiGlow>,
    /// Session manager for multiple tabs
    pub session_manager: SessionManager,
    /// Bookmark manager
    pub bookmark_manager: BookmarkManager,
    /// App settings
    pub settings: Settings,
    /// Settings modal state
    pub(super) settings_modal: SettingsModal,
    /// HID connection state
    pub hid_connected: bool,
    /// Device YOLO mode state
    pub device_yolo: bool,
    /// Last detected Claude Code mode (to send updates only on change)
    pub detected_mode: DeviceMode,
    /// Last mode reported by the device (to detect actual button presses vs confirmations)
    pub last_device_reported_mode: Option<DeviceMode>,
    /// When the app last sent SetMode to the device (suppresses stale StateReports)
    pub mode_set_from_app_at: Option<std::time::Instant>,
    /// Pending HID navigation keys for new-tab UI (encoder knob, etc.)
    pub pending_hid_nav_keys: Vec<egui::Key>,
    /// Whether window should be visible
    visible: Arc<AtomicBool>,
    /// Whether the window currently has focus
    pub(super) window_focused: bool,
    /// Window ID (when created)
    window_id: Option<WindowId>,
    /// Callback to notify PTY of resize (for active session)
    resize_callback: Option<ResizeCallback>,
    /// Current scroll offset (0 = bottom, positive = viewing history)
    pub(super) scroll_offset: Arc<AtomicI32>,
    /// Current keyboard modifiers state
    pub(super) modifiers: Modifiers,
    /// Cached character width for resize calculations (Cell for interior mutability)
    pub(super) cached_char_width: Cell<f32>,
    /// Cached line height for resize calculations (Cell for interior mutability)
    pub(super) cached_line_height: Cell<f32>,
    /// Font size in points
    pub font_size: f32,
    /// Initial font size at app start (for reset)
    initial_font_size: f32,
    /// Selection start position (row, col) in terminal coordinates
    pub(super) selection_start: Option<(i64, usize)>,
    /// Selection end position (row, col) in terminal coordinates
    pub(super) selection_end: Option<(i64, usize)>,
    /// Whether mouse is currently dragging for selection
    pub(super) is_selecting: bool,
    /// Current cursor position in logical pixels
    pub(super) cursor_position: Option<(f64, f64)>,
    /// WezTerm-based glyph cache for proper Unicode rendering
    glyph_cache: RefCell<Option<GlyphCache>>,
    /// Currently hovered hyperlink (for visual feedback)
    pub(super) hovered_hyperlink: Option<Arc<Hyperlink>>,
    /// Pending actions to be processed by the main app
    pub(super) pending_actions: Vec<TerminalAction>,
    /// Whether egui image loaders have been installed
    image_loaders_installed: bool,
    /// Context menu state for right-click popup
    pub(super) context_menu: ContextMenuState,
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
    /// Monotonic counter for HID alert insertion order (mirrors firmware FIFO)
    pub(super) alert_order_counter: u64,
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
            device_yolo: false,
            detected_mode: DeviceMode::Default,
            last_device_reported_mode: None,
            mode_set_from_app_at: None,
            pending_hid_nav_keys: Vec::new(),
            visible: Arc::new(AtomicBool::new(false)),
            window_focused: false,
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
            alert_order_counter: 0,
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
            let base = if let Some(session) = self.session_manager.active_session() {
                if session.is_new_tab() {
                    "\u{1F916}".to_string()
                } else {
                    session.working_directory.display().to_string()
                }
            } else {
                "\u{1F916}".to_string()
            };
            let title = if self.device_yolo {
                format!("\u{1F525} {}", base)
            } else {
                base
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
    pub(super) fn try_resolve_session_id(&mut self, session_id: SessionId) {
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

    pub fn is_focused(&self) -> bool {
        self.window_focused
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

    /// Invalidate the glyph cache (e.g., after font size change)
    pub fn invalidate_glyph_cache(&mut self) {
        *self.glyph_cache.borrow_mut() = None;
    }

    /// Open the settings modal
    pub fn open_settings(&mut self) {
        self.settings_modal.open(&self.settings);
    }

    /// Set soft key configs on the settings modal (called after device read)
    pub fn set_soft_key_configs(&mut self, keys: [SoftKeyEditState; 3]) {
        self.settings_modal.set_soft_keys(keys);
    }

    /// Set soft key error on the settings modal
    pub fn set_soft_key_error(&mut self, err: String) {
        self.settings_modal.set_soft_keys_error(err);
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
            (Arc::clone(&s.session), s.is_new_tab(), s.id)
        });

        let bookmark_manager = self.bookmark_manager.clone();

        let mut new_actions = Vec::new();

        let need_install_loaders = !self.image_loaders_installed;

        let mut render_params = RenderParams {
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
            hid_nav_keys: &mut self.pending_hid_nav_keys,
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
                    render_terminal_content(ui, ctx, &mut render_params, &mut new_actions);
                });

            // Hyperlink tooltip
            render_hyperlink_tooltip(ctx, &self.hovered_hyperlink);

            // Render settings modal
            let settings_result = render_settings_modal(ctx, &mut self.settings_modal, hid_connected);
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

            // Only schedule periodic repaints when there's active content that needs animation.
            // With ControlFlow::Wait, the event loop sleeps until woken by an event,
            // so we only need repaints for cursor blink, loading spinners, or working indicators.
            let has_active_content = self.session_manager.has_running_sessions();
            if has_active_content {
                ctx.request_repaint_after(std::time::Duration::from_millis(500));
            }
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
}

impl Default for TerminalWindowState {
    fn default() -> Self {
        Self::new(17.0)
    }
}
