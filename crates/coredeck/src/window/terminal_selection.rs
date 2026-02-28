//! Selection, scrolling, coordinate mapping, and context menu methods for TerminalWindowState

use super::render::TAB_BAR_HEIGHT;
use super::terminal::{TerminalAction, TerminalWindowState};
use crate::core::claude_sessions::get_sessions_for_directory;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use wezterm_cell::Hyperlink;

#[cfg(target_os = "macos")]
use crate::macos::{
    show_context_menu, update_recent_sessions_menu, ContextMenuAction, ContextMenuSession,
};
#[cfg(target_os = "macos")]
use raw_window_handle::HasWindowHandle;

impl TerminalWindowState {
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
    pub(super) fn open_context_menu(&mut self, x: f32, y: f32) {
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
                        let has_clipboard = arboard::Clipboard::new()
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
                                    self.pending_actions
                                        .push(TerminalAction::FreshSessionCurrentDir);
                                }
                                ContextMenuAction::LoadSession { session_id } => {
                                    self.pending_actions
                                        .push(TerminalAction::LoadSession { session_id });
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
    pub(super) fn close_context_menu(&mut self) {
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
    pub(super) fn screen_to_terminal_coords(&self, x: f64, y: f64) -> (i64, usize) {
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

    pub(super) fn get_hyperlink_at(&self, row: usize, col: usize) -> Option<Arc<Hyperlink>> {
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
}
