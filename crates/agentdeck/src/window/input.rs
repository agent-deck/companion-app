//! Input handling for terminal window
//!
//! Contains keyboard sequence building functions and input event processing helpers.

use tracing::info;

/// Build arrow key escape sequence with optional modifiers
pub fn build_arrow_seq(modifiers: Option<u8>, key_char: u8) -> Vec<u8> {
    match modifiers {
        Some(m) => vec![0x1b, b'[', b'1', b';', b'0' + m, key_char],
        None => vec![0x1b, b'[', key_char],
    }
}

/// Build Home/End escape sequence with optional modifiers
pub fn build_home_end_seq(modifiers: Option<u8>, key_char: u8) -> Vec<u8> {
    match modifiers {
        Some(m) => vec![0x1b, b'[', b'1', b';', b'0' + m, key_char],
        None => vec![0x1b, b'[', key_char],
    }
}

/// Build tilde-terminated escape sequence (PageUp, PageDown, Delete, Insert, F5-F12)
pub fn build_tilde_seq(modifiers: Option<u8>, code: &[u8]) -> Vec<u8> {
    match modifiers {
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

/// Build F1-F4 escape sequence with optional modifiers
pub fn build_f1_f4_seq(modifiers: Option<u8>, key_char: u8) -> Vec<u8> {
    match modifiers {
        Some(m) => vec![0x1b, b'[', b'1', b';', b'0' + m, key_char],
        None => vec![0x1b, b'O', key_char],
    }
}

/// Encode keyboard modifiers as a modifier code for escape sequences
/// Returns None if no modifiers are pressed, otherwise returns 1 + modifier bits
pub fn encode_modifiers(shift: bool, alt: bool, ctrl: bool) -> Option<u8> {
    let mut code = 0u8;
    if shift {
        code |= 1;
    }
    if alt {
        code |= 2;
    }
    if ctrl {
        code |= 4;
    }
    if code == 0 {
        None
    } else {
        Some(code + 1)
    }
}

/// Open a URL using the platform-specific default handler
pub fn open_url(url: &str) {
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
