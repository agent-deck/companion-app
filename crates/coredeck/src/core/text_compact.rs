//! Display text compacting — ported from firmware `compact_text()` and parenthetical split.
//!
//! The firmware used to do content-aware compacting on-device.  Now the app
//! pre-processes task strings before sending them over the wire, and the
//! firmware only does font-safety sanitization.

/// Thin space (U+2009) encoded as UTF-8.
const THIN_SPACE: &str = "\u{2009}";

/// Apply display-specific text compaction (port of firmware `compact_text`):
///
/// - Remove space between `↑`/`↓` and a following digit (`↑ 12` → `↑12`)
/// - Strip ` tokens` / ` token` from the *middle* of the string (not at the end)
/// - Replace ASCII spaces with Unicode thin space (U+2009)
pub fn compact_text(s: &str) -> String {
    let bytes = s.as_bytes();
    let len = bytes.len();
    let mut out = String::with_capacity(len * 3);
    let mut i = 0;

    while i < len {
        // ↑ (E2 86 91) or ↓ (E2 86 93) followed by space + digit
        if i + 4 < len
            && bytes[i] == 0xE2
            && bytes[i + 1] == 0x86
            && (bytes[i + 2] == 0x91 || bytes[i + 2] == 0x93)
            && bytes[i + 3] == b' '
            && bytes[i + 4].is_ascii_digit()
        {
            // Copy arrow, skip the space
            out.push_str(&s[i..i + 3]);
            i += 4; // skip arrow (3 bytes) + space (1 byte)
            continue;
        }

        // " tokens" removed unless at end of string
        if bytes[i] == b' ' && i + 7 <= len && &s[i..i + 7] == " tokens" && i + 7 < len {
            i += 7;
            continue;
        }
        // " token" (not followed by 's') removed unless at end
        if bytes[i] == b' '
            && i + 6 <= len
            && &s[i..i + 6] == " token"
            && (i + 6 >= len || bytes[i + 6] != b's')
            && i + 6 < len
        {
            i += 6;
            continue;
        }
        // "tokens" at start or after non-space, removed unless at end
        if i + 6 <= len && &s[i..i + 6] == "tokens" && i + 6 < len {
            i += 6;
            continue;
        }
        // "token" (not followed by 's'), removed unless at end
        if i + 5 <= len
            && &s[i..i + 5] == "token"
            && (i + 5 >= len || bytes[i + 5] != b's')
            && i + 5 < len
        {
            i += 5;
            continue;
        }

        // Replace ASCII space with thin space
        if bytes[i] == b' ' {
            out.push_str(THIN_SPACE);
            i += 1;
            continue;
        }

        // Copy one UTF-8 character
        let ch_len = utf8_char_len(bytes[i]);
        out.push_str(&s[i..i + ch_len]);
        i += ch_len;
    }

    out
}

/// Split a task string into two display lines (port of firmware parenthetical split).
///
/// If the task contains `(` not at position 0:
/// - Line 1: text before `(`, trimmed, compacted
/// - Line 2: content inside `(…)`, compacted
///
/// Otherwise: single compacted line.
pub fn split_task_lines(task: &str) -> (String, Option<String>) {
    if let Some(paren_pos) = task.find('(') {
        if paren_pos > 0 {
            let prefix = task[..paren_pos].trim_end();
            let rest = &task[paren_pos + 1..];
            let stats = rest.trim_end_matches(')');
            return (compact_text(prefix), Some(compact_text(stats)));
        }
    }
    (compact_text(task), None)
}

/// Length of a UTF-8 character from its first byte.
fn utf8_char_len(b: u8) -> usize {
    if b < 0x80 {
        1
    } else if b & 0xE0 == 0xC0 {
        2
    } else if b & 0xF0 == 0xE0 {
        3
    } else if b & 0xF8 == 0xF0 {
        4
    } else {
        1 // Invalid — advance by 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compact_arrow_digit_spacing() {
        assert_eq!(compact_text("↑ 12k"), "↑12k");
        assert_eq!(compact_text("↓ 5"), "↓5");
    }

    #[test]
    fn test_compact_thin_spaces() {
        assert_eq!(compact_text("Reading files"), "Reading\u{2009}files");
    }

    #[test]
    fn test_compact_strip_tokens_middle() {
        // " tokens" in the middle should be stripped
        assert_eq!(compact_text("↑ 12k tokens, $0.04"), "↑12k,\u{2009}$0.04");
    }

    #[test]
    fn test_compact_keep_tokens_at_end() {
        // " tokens" at end of string should be kept
        assert_eq!(compact_text("12k tokens"), "12k\u{2009}tokens");
    }

    #[test]
    fn test_compact_strip_token_singular_middle() {
        assert_eq!(compact_text("1 token, $0.01"), "1,\u{2009}$0.01");
    }

    #[test]
    fn test_compact_empty() {
        assert_eq!(compact_text(""), "");
    }

    #[test]
    fn test_split_with_parens() {
        let (line1, line2) = split_task_lines("Reading files (↑ 12k tokens, $0.04)");
        assert_eq!(line1, "Reading\u{2009}files");
        assert_eq!(line2.unwrap(), "↑12k,\u{2009}$0.04");
    }

    #[test]
    fn test_split_no_parens() {
        let (line1, line2) = split_task_lines("Reading files");
        assert_eq!(line1, "Reading\u{2009}files");
        assert!(line2.is_none());
    }

    #[test]
    fn test_split_paren_at_start() {
        // Paren at position 0 — treat as single line
        let (line1, line2) = split_task_lines("(something)");
        assert_eq!(line1, "(something)");
        assert!(line2.is_none());
    }

    #[test]
    fn test_compact_no_double_strip() {
        // Make sure we don't eat "tokens" when it's an actual word at the end
        assert_eq!(compact_text("count tokens"), "count\u{2009}tokens");
    }
}
