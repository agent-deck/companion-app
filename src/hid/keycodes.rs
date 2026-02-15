//! QMK keycode definitions for soft key configuration
//!
//! Maps common USB HID usage codes to human-readable names for the UI.

use serde::{Deserialize, Serialize};

/// A common QMK keycode (USB HID usage code)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum QmkKeycode {
    A = 0x04,
    B = 0x05,
    C = 0x06,
    D = 0x07,
    E = 0x08,
    F = 0x09,
    G = 0x0A,
    H = 0x0B,
    I = 0x0C,
    J = 0x0D,
    K = 0x0E,
    L = 0x0F,
    M = 0x10,
    N = 0x11,
    O = 0x12,
    P = 0x13,
    Q = 0x14,
    R = 0x15,
    S = 0x16,
    T = 0x17,
    U = 0x18,
    V = 0x19,
    W = 0x1A,
    X = 0x1B,
    Y = 0x1C,
    Z = 0x1D,
    Num1 = 0x1E,
    Num2 = 0x1F,
    Num3 = 0x20,
    Num4 = 0x21,
    Num5 = 0x22,
    Num6 = 0x23,
    Num7 = 0x24,
    Num8 = 0x25,
    Num9 = 0x26,
    Num0 = 0x27,
    Enter = 0x28,
    Escape = 0x29,
    Backspace = 0x2A,
    Tab = 0x2B,
    Space = 0x2C,
    Minus = 0x2D,
    Equal = 0x2E,
    LeftBracket = 0x2F,
    RightBracket = 0x30,
    Backslash = 0x31,
    Semicolon = 0x33,
    Quote = 0x34,
    Grave = 0x35,
    Comma = 0x36,
    Dot = 0x37,
    Slash = 0x38,
    CapsLock = 0x39,
    F1 = 0x3A,
    F2 = 0x3B,
    F3 = 0x3C,
    F4 = 0x3D,
    F5 = 0x3E,
    F6 = 0x3F,
    F7 = 0x40,
    F8 = 0x41,
    F9 = 0x42,
    F10 = 0x43,
    F11 = 0x44,
    F12 = 0x45,
    PrintScreen = 0x46,
    ScrollLock = 0x47,
    Pause = 0x48,
    Insert = 0x49,
    Home = 0x4A,
    PageUp = 0x4B,
    Delete = 0x4C,
    End = 0x4D,
    PageDown = 0x4E,
    Right = 0x4F,
    Left = 0x50,
    Down = 0x51,
    Up = 0x52,
    F13 = 0x68,
    F14 = 0x69,
    F15 = 0x6A,
    F16 = 0x6B,
    F17 = 0x6C,
    F18 = 0x6D,
    F19 = 0x6E,
    F20 = 0x6F,
    F21 = 0x70,
    F22 = 0x71,
    F23 = 0x72,
    F24 = 0x73,
}

impl QmkKeycode {
    /// Convert to USB HID usage byte
    pub fn to_byte(self) -> u8 {
        self as u8
    }

    /// Parse from USB HID usage byte
    pub fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0x04 => Some(Self::A),
            0x05 => Some(Self::B),
            0x06 => Some(Self::C),
            0x07 => Some(Self::D),
            0x08 => Some(Self::E),
            0x09 => Some(Self::F),
            0x0A => Some(Self::G),
            0x0B => Some(Self::H),
            0x0C => Some(Self::I),
            0x0D => Some(Self::J),
            0x0E => Some(Self::K),
            0x0F => Some(Self::L),
            0x10 => Some(Self::M),
            0x11 => Some(Self::N),
            0x12 => Some(Self::O),
            0x13 => Some(Self::P),
            0x14 => Some(Self::Q),
            0x15 => Some(Self::R),
            0x16 => Some(Self::S),
            0x17 => Some(Self::T),
            0x18 => Some(Self::U),
            0x19 => Some(Self::V),
            0x1A => Some(Self::W),
            0x1B => Some(Self::X),
            0x1C => Some(Self::Y),
            0x1D => Some(Self::Z),
            0x1E => Some(Self::Num1),
            0x1F => Some(Self::Num2),
            0x20 => Some(Self::Num3),
            0x21 => Some(Self::Num4),
            0x22 => Some(Self::Num5),
            0x23 => Some(Self::Num6),
            0x24 => Some(Self::Num7),
            0x25 => Some(Self::Num8),
            0x26 => Some(Self::Num9),
            0x27 => Some(Self::Num0),
            0x28 => Some(Self::Enter),
            0x29 => Some(Self::Escape),
            0x2A => Some(Self::Backspace),
            0x2B => Some(Self::Tab),
            0x2C => Some(Self::Space),
            0x2D => Some(Self::Minus),
            0x2E => Some(Self::Equal),
            0x2F => Some(Self::LeftBracket),
            0x30 => Some(Self::RightBracket),
            0x31 => Some(Self::Backslash),
            0x33 => Some(Self::Semicolon),
            0x34 => Some(Self::Quote),
            0x35 => Some(Self::Grave),
            0x36 => Some(Self::Comma),
            0x37 => Some(Self::Dot),
            0x38 => Some(Self::Slash),
            0x39 => Some(Self::CapsLock),
            0x3A => Some(Self::F1),
            0x3B => Some(Self::F2),
            0x3C => Some(Self::F3),
            0x3D => Some(Self::F4),
            0x3E => Some(Self::F5),
            0x3F => Some(Self::F6),
            0x40 => Some(Self::F7),
            0x41 => Some(Self::F8),
            0x42 => Some(Self::F9),
            0x43 => Some(Self::F10),
            0x44 => Some(Self::F11),
            0x45 => Some(Self::F12),
            0x46 => Some(Self::PrintScreen),
            0x47 => Some(Self::ScrollLock),
            0x48 => Some(Self::Pause),
            0x49 => Some(Self::Insert),
            0x4A => Some(Self::Home),
            0x4B => Some(Self::PageUp),
            0x4C => Some(Self::Delete),
            0x4D => Some(Self::End),
            0x4E => Some(Self::PageDown),
            0x4F => Some(Self::Right),
            0x50 => Some(Self::Left),
            0x51 => Some(Self::Down),
            0x52 => Some(Self::Up),
            0x68 => Some(Self::F13),
            0x69 => Some(Self::F14),
            0x6A => Some(Self::F15),
            0x6B => Some(Self::F16),
            0x6C => Some(Self::F17),
            0x6D => Some(Self::F18),
            0x6E => Some(Self::F19),
            0x6F => Some(Self::F20),
            0x70 => Some(Self::F21),
            0x71 => Some(Self::F22),
            0x72 => Some(Self::F23),
            0x73 => Some(Self::F24),
            _ => None,
        }
    }

    /// Human-readable display name for the UI
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::A => "A", Self::B => "B", Self::C => "C", Self::D => "D",
            Self::E => "E", Self::F => "F", Self::G => "G", Self::H => "H",
            Self::I => "I", Self::J => "J", Self::K => "K", Self::L => "L",
            Self::M => "M", Self::N => "N", Self::O => "O", Self::P => "P",
            Self::Q => "Q", Self::R => "R", Self::S => "S", Self::T => "T",
            Self::U => "U", Self::V => "V", Self::W => "W", Self::X => "X",
            Self::Y => "Y", Self::Z => "Z",
            Self::Num1 => "1", Self::Num2 => "2", Self::Num3 => "3",
            Self::Num4 => "4", Self::Num5 => "5", Self::Num6 => "6",
            Self::Num7 => "7", Self::Num8 => "8", Self::Num9 => "9",
            Self::Num0 => "0",
            Self::Enter => "Enter", Self::Escape => "Escape",
            Self::Backspace => "Backspace", Self::Tab => "Tab",
            Self::Space => "Space", Self::Minus => "-", Self::Equal => "=",
            Self::LeftBracket => "[", Self::RightBracket => "]",
            Self::Backslash => "\\", Self::Semicolon => ";", Self::Quote => "'",
            Self::Grave => "`", Self::Comma => ",", Self::Dot => ".",
            Self::Slash => "/", Self::CapsLock => "CapsLock",
            Self::F1 => "F1", Self::F2 => "F2", Self::F3 => "F3",
            Self::F4 => "F4", Self::F5 => "F5", Self::F6 => "F6",
            Self::F7 => "F7", Self::F8 => "F8", Self::F9 => "F9",
            Self::F10 => "F10", Self::F11 => "F11", Self::F12 => "F12",
            Self::PrintScreen => "PrtSc", Self::ScrollLock => "ScrLk",
            Self::Pause => "Pause", Self::Insert => "Insert",
            Self::Home => "Home", Self::PageUp => "PgUp",
            Self::Delete => "Delete", Self::End => "End",
            Self::PageDown => "PgDn",
            Self::Right => "Right", Self::Left => "Left",
            Self::Down => "Down", Self::Up => "Up",
            Self::F13 => "F13", Self::F14 => "F14", Self::F15 => "F15",
            Self::F16 => "F16", Self::F17 => "F17", Self::F18 => "F18",
            Self::F19 => "F19", Self::F20 => "F20", Self::F21 => "F21",
            Self::F22 => "F22", Self::F23 => "F23", Self::F24 => "F24",
        }
    }

    /// All standard keycodes for the combo box picker
    pub fn all_standard() -> &'static [QmkKeycode] {
        &[
            // Letters
            Self::A, Self::B, Self::C, Self::D, Self::E, Self::F, Self::G,
            Self::H, Self::I, Self::J, Self::K, Self::L, Self::M, Self::N,
            Self::O, Self::P, Self::Q, Self::R, Self::S, Self::T, Self::U,
            Self::V, Self::W, Self::X, Self::Y, Self::Z,
            // Numbers
            Self::Num0, Self::Num1, Self::Num2, Self::Num3, Self::Num4,
            Self::Num5, Self::Num6, Self::Num7, Self::Num8, Self::Num9,
            // Function keys
            Self::F1, Self::F2, Self::F3, Self::F4, Self::F5, Self::F6,
            Self::F7, Self::F8, Self::F9, Self::F10, Self::F11, Self::F12,
            Self::F13, Self::F14, Self::F15, Self::F16, Self::F17, Self::F18,
            Self::F19, Self::F20, Self::F21, Self::F22, Self::F23, Self::F24,
            // Common keys
            Self::Enter, Self::Escape, Self::Backspace, Self::Tab, Self::Space,
            Self::Delete, Self::Insert,
            // Navigation
            Self::Up, Self::Down, Self::Left, Self::Right,
            Self::Home, Self::End, Self::PageUp, Self::PageDown,
            // Punctuation
            Self::Minus, Self::Equal, Self::LeftBracket, Self::RightBracket,
            Self::Backslash, Self::Semicolon, Self::Quote, Self::Grave,
            Self::Comma, Self::Dot, Self::Slash,
            // Others
            Self::CapsLock, Self::PrintScreen, Self::ScrollLock, Self::Pause,
        ]
    }
}

/// Keyboard modifier flags
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct KeyModifiers {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub gui: bool,
}

// QMK modifier bit positions (left modifiers)
const MOD_LCTL: u16 = 0x0100;
const MOD_LSFT: u16 = 0x0200;
const MOD_LALT: u16 = 0x0400;
const MOD_LGUI: u16 = 0x0800;

impl KeyModifiers {
    /// Encode modifiers to QMK modifier bits (upper byte of 16-bit keycode)
    pub fn to_modifier_bits(self) -> u16 {
        let mut bits = 0u16;
        if self.ctrl { bits |= MOD_LCTL; }
        if self.shift { bits |= MOD_LSFT; }
        if self.alt { bits |= MOD_LALT; }
        if self.gui { bits |= MOD_LGUI; }
        bits
    }

    /// Decode from QMK modifier bits
    pub fn from_modifier_bits(bits: u16) -> Self {
        Self {
            ctrl: bits & MOD_LCTL != 0,
            shift: bits & MOD_LSFT != 0,
            alt: bits & MOD_LALT != 0,
            gui: bits & MOD_LGUI != 0,
        }
    }

    /// Human-readable summary (e.g. "Ctrl+Shift+")
    pub fn display_prefix(&self) -> String {
        let mut parts = Vec::new();
        if self.ctrl { parts.push("Ctrl"); }
        if self.shift { parts.push("Shift"); }
        if self.alt {
            if cfg!(target_os = "macos") {
                parts.push("Opt");
            } else {
                parts.push("Alt");
            }
        }
        if self.gui {
            if cfg!(target_os = "macos") {
                parts.push("Cmd");
            } else {
                parts.push("Win");
            }
        }
        if parts.is_empty() {
            String::new()
        } else {
            format!("{}+", parts.join("+"))
        }
    }
}

/// Compose a 16-bit QMK keycode from base key and modifiers
pub fn compose_keycode(base: QmkKeycode, mods: KeyModifiers) -> u16 {
    (base.to_byte() as u16) | mods.to_modifier_bits()
}

/// Decompose a 16-bit QMK keycode into base key and modifiers
pub fn decompose_keycode(keycode: u16) -> (Option<QmkKeycode>, KeyModifiers) {
    let base_byte = (keycode & 0xFF) as u8;
    let mods = KeyModifiers::from_modifier_bits(keycode & 0xFF00);
    (QmkKeycode::from_byte(base_byte), mods)
}

/// Map an egui::Key to the corresponding QMK keycode
pub fn from_egui_key(key: egui::Key) -> Option<QmkKeycode> {
    match key {
        egui::Key::A => Some(QmkKeycode::A),
        egui::Key::B => Some(QmkKeycode::B),
        egui::Key::C => Some(QmkKeycode::C),
        egui::Key::D => Some(QmkKeycode::D),
        egui::Key::E => Some(QmkKeycode::E),
        egui::Key::F => Some(QmkKeycode::F),
        egui::Key::G => Some(QmkKeycode::G),
        egui::Key::H => Some(QmkKeycode::H),
        egui::Key::I => Some(QmkKeycode::I),
        egui::Key::J => Some(QmkKeycode::J),
        egui::Key::K => Some(QmkKeycode::K),
        egui::Key::L => Some(QmkKeycode::L),
        egui::Key::M => Some(QmkKeycode::M),
        egui::Key::N => Some(QmkKeycode::N),
        egui::Key::O => Some(QmkKeycode::O),
        egui::Key::P => Some(QmkKeycode::P),
        egui::Key::Q => Some(QmkKeycode::Q),
        egui::Key::R => Some(QmkKeycode::R),
        egui::Key::S => Some(QmkKeycode::S),
        egui::Key::T => Some(QmkKeycode::T),
        egui::Key::U => Some(QmkKeycode::U),
        egui::Key::V => Some(QmkKeycode::V),
        egui::Key::W => Some(QmkKeycode::W),
        egui::Key::X => Some(QmkKeycode::X),
        egui::Key::Y => Some(QmkKeycode::Y),
        egui::Key::Z => Some(QmkKeycode::Z),
        egui::Key::Num0 => Some(QmkKeycode::Num0),
        egui::Key::Num1 => Some(QmkKeycode::Num1),
        egui::Key::Num2 => Some(QmkKeycode::Num2),
        egui::Key::Num3 => Some(QmkKeycode::Num3),
        egui::Key::Num4 => Some(QmkKeycode::Num4),
        egui::Key::Num5 => Some(QmkKeycode::Num5),
        egui::Key::Num6 => Some(QmkKeycode::Num6),
        egui::Key::Num7 => Some(QmkKeycode::Num7),
        egui::Key::Num8 => Some(QmkKeycode::Num8),
        egui::Key::Num9 => Some(QmkKeycode::Num9),
        egui::Key::F1 => Some(QmkKeycode::F1),
        egui::Key::F2 => Some(QmkKeycode::F2),
        egui::Key::F3 => Some(QmkKeycode::F3),
        egui::Key::F4 => Some(QmkKeycode::F4),
        egui::Key::F5 => Some(QmkKeycode::F5),
        egui::Key::F6 => Some(QmkKeycode::F6),
        egui::Key::F7 => Some(QmkKeycode::F7),
        egui::Key::F8 => Some(QmkKeycode::F8),
        egui::Key::F9 => Some(QmkKeycode::F9),
        egui::Key::F10 => Some(QmkKeycode::F10),
        egui::Key::F11 => Some(QmkKeycode::F11),
        egui::Key::F12 => Some(QmkKeycode::F12),
        egui::Key::F13 => Some(QmkKeycode::F13),
        egui::Key::F14 => Some(QmkKeycode::F14),
        egui::Key::F15 => Some(QmkKeycode::F15),
        egui::Key::F16 => Some(QmkKeycode::F16),
        egui::Key::F17 => Some(QmkKeycode::F17),
        egui::Key::F18 => Some(QmkKeycode::F18),
        egui::Key::F19 => Some(QmkKeycode::F19),
        egui::Key::F20 => Some(QmkKeycode::F20),
        egui::Key::Enter => Some(QmkKeycode::Enter),
        egui::Key::Escape => Some(QmkKeycode::Escape),
        egui::Key::Backspace => Some(QmkKeycode::Backspace),
        egui::Key::Tab => Some(QmkKeycode::Tab),
        egui::Key::Space => Some(QmkKeycode::Space),
        egui::Key::Delete => Some(QmkKeycode::Delete),
        egui::Key::Insert => Some(QmkKeycode::Insert),
        egui::Key::Home => Some(QmkKeycode::Home),
        egui::Key::End => Some(QmkKeycode::End),
        egui::Key::PageUp => Some(QmkKeycode::PageUp),
        egui::Key::PageDown => Some(QmkKeycode::PageDown),
        egui::Key::ArrowUp => Some(QmkKeycode::Up),
        egui::Key::ArrowDown => Some(QmkKeycode::Down),
        egui::Key::ArrowLeft => Some(QmkKeycode::Left),
        egui::Key::ArrowRight => Some(QmkKeycode::Right),
        egui::Key::Minus => Some(QmkKeycode::Minus),
        egui::Key::Equals => Some(QmkKeycode::Equal),
        egui::Key::OpenBracket => Some(QmkKeycode::LeftBracket),
        egui::Key::CloseBracket => Some(QmkKeycode::RightBracket),
        egui::Key::Backslash => Some(QmkKeycode::Backslash),
        egui::Key::Semicolon => Some(QmkKeycode::Semicolon),
        egui::Key::Quote => Some(QmkKeycode::Quote),
        egui::Key::Backtick => Some(QmkKeycode::Grave),
        egui::Key::Comma => Some(QmkKeycode::Comma),
        egui::Key::Period => Some(QmkKeycode::Dot),
        egui::Key::Slash => Some(QmkKeycode::Slash),
        _ => None,
    }
}

/// Map egui::Modifiers to KeyModifiers
pub fn from_egui_modifiers(mods: &egui::Modifiers) -> KeyModifiers {
    KeyModifiers {
        ctrl: mods.ctrl,
        shift: mods.shift,
        alt: mods.alt,
        gui: mods.mac_cmd,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_keycode_roundtrip() {
        for key in QmkKeycode::all_standard() {
            let byte = key.to_byte();
            let parsed = QmkKeycode::from_byte(byte);
            assert_eq!(parsed, Some(*key), "Roundtrip failed for {:?} (0x{:02X})", key, byte);
        }
    }

    #[test]
    fn test_known_byte_values() {
        assert_eq!(QmkKeycode::A.to_byte(), 0x04);
        assert_eq!(QmkKeycode::Z.to_byte(), 0x1D);
        assert_eq!(QmkKeycode::Num1.to_byte(), 0x1E);
        assert_eq!(QmkKeycode::Num0.to_byte(), 0x27);
        assert_eq!(QmkKeycode::Enter.to_byte(), 0x28);
        assert_eq!(QmkKeycode::Escape.to_byte(), 0x29);
        assert_eq!(QmkKeycode::F1.to_byte(), 0x3A);
        assert_eq!(QmkKeycode::F20.to_byte(), 0x6F);
    }

    #[test]
    fn test_from_byte_unknown() {
        assert_eq!(QmkKeycode::from_byte(0x00), None);
        assert_eq!(QmkKeycode::from_byte(0x03), None);
        assert_eq!(QmkKeycode::from_byte(0xFF), None);
    }

    #[test]
    fn test_modifier_bits_roundtrip() {
        let mods = KeyModifiers { ctrl: true, shift: false, alt: true, gui: false };
        let bits = mods.to_modifier_bits();
        let parsed = KeyModifiers::from_modifier_bits(bits);
        assert_eq!(parsed, mods);
    }

    #[test]
    fn test_modifier_bits_values() {
        let ctrl_only = KeyModifiers { ctrl: true, ..Default::default() };
        assert_eq!(ctrl_only.to_modifier_bits(), 0x0100);

        let shift_only = KeyModifiers { shift: true, ..Default::default() };
        assert_eq!(shift_only.to_modifier_bits(), 0x0200);

        let all = KeyModifiers { ctrl: true, shift: true, alt: true, gui: true };
        assert_eq!(all.to_modifier_bits(), 0x0F00);
    }

    #[test]
    fn test_compose_decompose_roundtrip() {
        let key = QmkKeycode::C;
        let mods = KeyModifiers { ctrl: true, ..Default::default() };
        let composed = compose_keycode(key, mods);
        let (parsed_key, parsed_mods) = decompose_keycode(composed);
        assert_eq!(parsed_key, Some(key));
        assert_eq!(parsed_mods, mods);
    }

    #[test]
    fn test_compose_ctrl_c() {
        let keycode = compose_keycode(
            QmkKeycode::C,
            KeyModifiers { ctrl: true, ..Default::default() },
        );
        // C = 0x06, LCTL = 0x0100 => 0x0106
        assert_eq!(keycode, 0x0106);
    }

    #[test]
    fn test_display_name() {
        assert_eq!(QmkKeycode::A.display_name(), "A");
        assert_eq!(QmkKeycode::Enter.display_name(), "Enter");
        assert_eq!(QmkKeycode::F20.display_name(), "F20");
        assert_eq!(QmkKeycode::Slash.display_name(), "/");
    }

    #[test]
    fn test_modifier_display_prefix() {
        let mods = KeyModifiers { ctrl: true, shift: true, ..Default::default() };
        assert_eq!(mods.display_prefix(), "Ctrl+Shift+");

        let none = KeyModifiers::default();
        assert_eq!(none.display_prefix(), "");
    }
}
