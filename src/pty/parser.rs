//! ANSI terminal output parser using VTE
//!
//! Parses ANSI escape sequences from Claude CLI output to extract
//! plain text and control sequences.

use std::collections::VecDeque;
use vte::{Params, Parser, Perform};

/// Parsed output element from ANSI stream
#[derive(Debug, Clone, PartialEq)]
pub enum ParsedElement {
    /// Plain text content
    Text(String),
    /// Newline
    Newline,
    /// Carriage return (line update)
    CarriageReturn,
    /// Clear line
    ClearLine,
    /// Clear screen
    ClearScreen,
    /// Cursor movement
    CursorMove { row: u16, col: u16 },
    /// Color/style change (SGR)
    Style(Vec<u16>),
    /// Bell
    Bell,
    /// Backspace
    Backspace,
    /// Tab
    Tab,
}

/// ANSI parser state
pub struct AnsiParser {
    /// VTE parser
    parser: Parser,
    /// Parsed elements queue
    elements: VecDeque<ParsedElement>,
    /// Current text buffer
    text_buffer: String,
}

impl Default for AnsiParser {
    fn default() -> Self {
        Self::new()
    }
}

impl AnsiParser {
    /// Create a new ANSI parser
    pub fn new() -> Self {
        Self {
            parser: Parser::new(),
            elements: VecDeque::new(),
            text_buffer: String::new(),
        }
    }

    /// Parse input bytes and return parsed elements
    pub fn parse(&mut self, data: &[u8]) -> Vec<ParsedElement> {
        // Create a performer to collect elements
        let mut performer = ParserPerformer {
            elements: &mut self.elements,
            text_buffer: &mut self.text_buffer,
        };

        // Parse each byte
        for byte in data {
            self.parser.advance(&mut performer, *byte);
        }

        // Flush any remaining text
        performer.flush_text();

        // Drain and return all elements
        self.elements.drain(..).collect()
    }

    /// Get the current line content (for status line parsing)
    pub fn current_line(&self) -> &str {
        &self.text_buffer
    }

    /// Reset parser state
    pub fn reset(&mut self) {
        self.parser = Parser::new();
        self.elements.clear();
        self.text_buffer.clear();
    }
}

/// VTE Perform implementation for parsing
struct ParserPerformer<'a> {
    elements: &'a mut VecDeque<ParsedElement>,
    text_buffer: &'a mut String,
}

impl ParserPerformer<'_> {
    fn flush_text(&mut self) {
        if !self.text_buffer.is_empty() {
            let text = std::mem::take(self.text_buffer);
            self.elements.push_back(ParsedElement::Text(text));
        }
    }
}

impl Perform for ParserPerformer<'_> {
    fn print(&mut self, c: char) {
        self.text_buffer.push(c);
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            0x07 => {
                // Bell
                self.flush_text();
                self.elements.push_back(ParsedElement::Bell);
            }
            0x08 => {
                // Backspace
                self.flush_text();
                self.elements.push_back(ParsedElement::Backspace);
            }
            0x09 => {
                // Tab
                self.flush_text();
                self.elements.push_back(ParsedElement::Tab);
            }
            0x0A => {
                // Line feed
                self.flush_text();
                self.elements.push_back(ParsedElement::Newline);
            }
            0x0D => {
                // Carriage return
                self.flush_text();
                self.elements.push_back(ParsedElement::CarriageReturn);
            }
            _ => {}
        }
    }

    fn hook(&mut self, _params: &Params, _intermediates: &[u8], _ignore: bool, _action: char) {
        // DCS sequences - not needed for basic parsing
    }

    fn put(&mut self, _byte: u8) {
        // DCS data - not needed for basic parsing
    }

    fn unhook(&mut self) {
        // End of DCS - not needed for basic parsing
    }

    fn osc_dispatch(&mut self, _params: &[&[u8]], _bell_terminated: bool) {
        // OSC sequences (like window title) - not needed for state extraction
    }

    fn csi_dispatch(&mut self, params: &Params, _intermediates: &[u8], _ignore: bool, action: char) {
        self.flush_text();

        match action {
            // Cursor movement
            'H' | 'f' => {
                // Cursor position
                let mut iter = params.iter();
                let row = iter.next().and_then(|p| p.first().copied()).unwrap_or(1);
                let col = iter.next().and_then(|p| p.first().copied()).unwrap_or(1);
                self.elements.push_back(ParsedElement::CursorMove {
                    row: row.max(1),
                    col: col.max(1),
                });
            }
            'J' => {
                // Erase in display
                let mode = params.iter().next().and_then(|p| p.first().copied()).unwrap_or(0);
                if mode == 2 || mode == 3 {
                    self.elements.push_back(ParsedElement::ClearScreen);
                }
            }
            'K' => {
                // Erase in line
                self.elements.push_back(ParsedElement::ClearLine);
            }
            'm' => {
                // SGR (Select Graphic Rendition)
                let codes: Vec<u16> = params
                    .iter()
                    .filter_map(|p| p.first().copied())
                    .collect();
                if !codes.is_empty() {
                    self.elements.push_back(ParsedElement::Style(codes));
                }
            }
            _ => {}
        }
    }

    fn esc_dispatch(&mut self, _intermediates: &[u8], _ignore: bool, _byte: u8) {
        // ESC sequences - not needed for basic parsing
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_plain_text() {
        let mut parser = AnsiParser::new();
        let elements = parser.parse(b"Hello, World!");

        assert_eq!(elements.len(), 1);
        assert_eq!(elements[0], ParsedElement::Text("Hello, World!".to_string()));
    }

    #[test]
    fn test_parse_newline() {
        let mut parser = AnsiParser::new();
        let elements = parser.parse(b"Line 1\nLine 2");

        assert_eq!(elements.len(), 3);
        assert_eq!(elements[0], ParsedElement::Text("Line 1".to_string()));
        assert_eq!(elements[1], ParsedElement::Newline);
        assert_eq!(elements[2], ParsedElement::Text("Line 2".to_string()));
    }

    #[test]
    fn test_parse_carriage_return() {
        let mut parser = AnsiParser::new();
        let elements = parser.parse(b"Progress: 50%\rProgress: 75%");

        assert_eq!(elements.len(), 3);
        assert_eq!(elements[0], ParsedElement::Text("Progress: 50%".to_string()));
        assert_eq!(elements[1], ParsedElement::CarriageReturn);
        assert_eq!(elements[2], ParsedElement::Text("Progress: 75%".to_string()));
    }

    #[test]
    fn test_parse_ansi_color() {
        let mut parser = AnsiParser::new();
        let elements = parser.parse(b"\x1b[32mGreen\x1b[0m");

        assert_eq!(elements.len(), 3);
        assert_eq!(elements[0], ParsedElement::Style(vec![32]));
        assert_eq!(elements[1], ParsedElement::Text("Green".to_string()));
        assert_eq!(elements[2], ParsedElement::Style(vec![0]));
    }

    #[test]
    fn test_parse_cursor_move() {
        let mut parser = AnsiParser::new();
        let elements = parser.parse(b"\x1b[10;20H");

        assert_eq!(elements.len(), 1);
        assert_eq!(
            elements[0],
            ParsedElement::CursorMove { row: 10, col: 20 }
        );
    }

    #[test]
    fn test_parse_clear_screen() {
        let mut parser = AnsiParser::new();
        let elements = parser.parse(b"\x1b[2J");

        assert_eq!(elements.len(), 1);
        assert_eq!(elements[0], ParsedElement::ClearScreen);
    }
}
