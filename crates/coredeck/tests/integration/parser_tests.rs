//! Parser integration tests

use core_deck::pty::{AnsiParser, ParsedElement};

#[test]
fn test_parse_spinner_fixture() {
    let fixture = include_str!("../fixtures/ansi_samples/spinner_output.txt");

    let mut parser = AnsiParser::new();
    let elements = parser.parse(fixture.as_bytes());

    // Should have text elements
    assert!(!elements.is_empty());
}

#[test]
fn test_ansi_color_stripping() {
    // ANSI escape sequence for green text
    let input = b"\x1b[32mGreen text\x1b[0m";

    let mut parser = AnsiParser::new();
    let elements = parser.parse(input);

    // Should have style, text, style elements
    assert!(elements.len() >= 1);

    // Find the text element
    let has_text = elements.iter().any(|e| {
        matches!(e, ParsedElement::Text(t) if t == "Green text")
    });
    assert!(has_text);
}
