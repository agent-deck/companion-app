//! Input handling methods for TerminalWindowState

use super::input::{
    build_arrow_seq, build_f1_f4_seq, build_home_end_seq, build_tilde_seq, encode_modifiers,
    open_url,
};
use super::terminal::{TerminalAction, TerminalWindowState};
use arboard::Clipboard;
use tracing::debug;
use winit::event::{ElementState, Ime, MouseButton, MouseScrollDelta, WindowEvent};
use winit::keyboard::{Key, KeyCode, NamedKey, PhysicalKey};

#[cfg(target_os = "macos")]
use crate::macos::update_edit_menu_state;

impl TerminalWindowState {
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
                                    let idx =
                                        c.chars().next().unwrap().to_digit(10).unwrap() as usize
                                            - 1;
                                    let sessions = self.session_manager.sessions();
                                    if idx < sessions.len() {
                                        let id = sessions[idx].id;
                                        self.pending_actions
                                            .push(TerminalAction::SwitchTab(id));
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

                    // Ctrl+Tab / Ctrl+Shift+Tab → cycle tabs
                    if let Key::Named(NamedKey::Tab) = &event.logical_key {
                        let state = self.modifiers.state();
                        if state.control_key() && !state.super_key() && !state.alt_key() {
                            let id = if state.shift_key() {
                                self.session_manager.prev_session_id()
                            } else {
                                self.session_manager.next_session_id()
                            };
                            if let Some(id) = id {
                                self.pending_actions.push(TerminalAction::SwitchTab(id));
                            }
                            return true;
                        }
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
                let scale_factor =
                    self.window.as_ref().map(|w| w.scale_factor()).unwrap_or(1.0);
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
                self.window_focused = *focused;
                if *focused {
                    // Clear HID alert for active session when window gains focus
                    if let Some(session_id) = self.session_manager.active_session_id() {
                        let tab_idx = self.session_manager.session_hid_tab_index(session_id);
                        if let Some(session) = self.session_manager.get_session_mut(session_id) {
                            if session.hid_alert_active {
                                session.hid_alert_active = false;
                                session.hid_alert_text = None;
                                session.hid_alert_details = None;
                                if let Some(idx) = tab_idx {
                                    self.pending_actions
                                        .push(TerminalAction::HidClearAlert(idx));
                                }
                            }
                        }
                    }

                    #[cfg(target_os = "macos")]
                    {
                        // Ensure the NSView is the first responder so that
                        // keyboard events (including from USB devices like the
                        // encoder) reach winit after focus changes.
                        if let Some(ref window) = self.window {
                            use raw_window_handle::HasWindowHandle;
                            if let Ok(handle) = window.window_handle() {
                                use raw_window_handle::RawWindowHandle;
                                if let RawWindowHandle::AppKit(appkit_handle) = handle.as_raw() {
                                    let view = appkit_handle.ns_view.as_ptr();
                                    #[allow(deprecated)]
                                    unsafe {
                                        use objc::{msg_send, sel, sel_impl};
                                        let ns_window: cocoa::base::id =
                                            msg_send![view as cocoa::base::id, window];
                                        if ns_window != cocoa::base::nil {
                                            let _: () = msg_send![ns_window,
                                                makeFirstResponder: view as cocoa::base::id];
                                        }
                                    }
                                }
                            }
                        }

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
            WindowEvent::Ime(Ime::Commit(text)) => {
                if !text.is_empty() && !self.settings_modal.is_open {
                    if let Some(session) = self.session_manager.active_session() {
                        if session.is_running {
                            self.scroll_to_bottom();
                            self.clear_selection();
                            self.send_to_pty(text.as_bytes());
                        } else if session.is_new_tab() {
                            // Encoder push sends Enter via IME on macOS;
                            // route it to the new-tab UI as a nav key.
                            for byte in text.bytes() {
                                let key = match byte {
                                    b'\r' | b'\n' => Some(egui::Key::Enter),
                                    0x1b => Some(egui::Key::Escape),
                                    _ => None,
                                };
                                if let Some(k) = key {
                                    self.pending_hid_nav_keys.push(k);
                                }
                            }
                            if !self.pending_hid_nav_keys.is_empty() {
                                if let Some(ref window) = self.window {
                                    window.request_redraw();
                                }
                            }
                        }
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
                    KeyCode::KeyA if ctrl => {
                        Some(if alt { vec![0x1b, 0x01] } else { vec![0x01] })
                    }
                    KeyCode::KeyB if ctrl => {
                        Some(if alt { vec![0x1b, 0x02] } else { vec![0x02] })
                    }
                    KeyCode::KeyC if ctrl => {
                        Some(if alt { vec![0x1b, 0x03] } else { vec![0x03] })
                    }
                    KeyCode::KeyD if ctrl => {
                        Some(if alt { vec![0x1b, 0x04] } else { vec![0x04] })
                    }
                    KeyCode::KeyE if ctrl => {
                        Some(if alt { vec![0x1b, 0x05] } else { vec![0x05] })
                    }
                    KeyCode::KeyF if ctrl => {
                        Some(if alt { vec![0x1b, 0x06] } else { vec![0x06] })
                    }
                    KeyCode::KeyG if ctrl => {
                        Some(if alt { vec![0x1b, 0x07] } else { vec![0x07] })
                    }
                    KeyCode::KeyH if ctrl => {
                        Some(if alt { vec![0x1b, 0x08] } else { vec![0x08] })
                    }
                    KeyCode::KeyI if ctrl => {
                        Some(if alt { vec![0x1b, 0x09] } else { vec![0x09] })
                    }
                    KeyCode::KeyJ if ctrl => {
                        Some(if alt { vec![0x1b, 0x0a] } else { vec![0x0a] })
                    }
                    KeyCode::KeyK if ctrl => {
                        Some(if alt { vec![0x1b, 0x0b] } else { vec![0x0b] })
                    }
                    KeyCode::KeyL if ctrl => {
                        Some(if alt { vec![0x1b, 0x0c] } else { vec![0x0c] })
                    }
                    KeyCode::KeyM if ctrl => {
                        Some(if alt { vec![0x1b, 0x0d] } else { vec![0x0d] })
                    }
                    KeyCode::KeyN if ctrl => {
                        Some(if alt { vec![0x1b, 0x0e] } else { vec![0x0e] })
                    }
                    KeyCode::KeyO if ctrl => {
                        Some(if alt { vec![0x1b, 0x0f] } else { vec![0x0f] })
                    }
                    KeyCode::KeyP if ctrl => {
                        Some(if alt { vec![0x1b, 0x10] } else { vec![0x10] })
                    }
                    KeyCode::KeyQ if ctrl => {
                        Some(if alt { vec![0x1b, 0x11] } else { vec![0x11] })
                    }
                    KeyCode::KeyR if ctrl => {
                        Some(if alt { vec![0x1b, 0x12] } else { vec![0x12] })
                    }
                    KeyCode::KeyS if ctrl => {
                        Some(if alt { vec![0x1b, 0x13] } else { vec![0x13] })
                    }
                    KeyCode::KeyT if ctrl => {
                        Some(if alt { vec![0x1b, 0x14] } else { vec![0x14] })
                    }
                    KeyCode::KeyU if ctrl => {
                        Some(if alt { vec![0x1b, 0x15] } else { vec![0x15] })
                    }
                    KeyCode::KeyV if ctrl => {
                        Some(if alt { vec![0x1b, 0x16] } else { vec![0x16] })
                    }
                    KeyCode::KeyW if ctrl => {
                        Some(if alt { vec![0x1b, 0x17] } else { vec![0x17] })
                    }
                    KeyCode::KeyX if ctrl => {
                        Some(if alt { vec![0x1b, 0x18] } else { vec![0x18] })
                    }
                    KeyCode::KeyY if ctrl => {
                        Some(if alt { vec![0x1b, 0x19] } else { vec![0x19] })
                    }
                    KeyCode::KeyZ if ctrl => {
                        Some(if alt { vec![0x1b, 0x1a] } else { vec![0x1a] })
                    }
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
}
