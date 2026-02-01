//! Parser integration tests

use agent_deck::pty::{AnsiParser, ClaudeStateExtractor, ParsedElement};

#[test]
fn test_parse_claude_status_fixture() {
    let fixture = include_str!("../fixtures/ansi_samples/claude_status.txt");

    let mut extractor = ClaudeStateExtractor::new();
    let state = extractor.process(fixture.as_bytes());

    assert!(state.is_some());
    let state = state.unwrap();

    assert!(state.model.contains("Claude"));
    assert!(state.task.contains("auth.rs"));
    assert_eq!(state.progress, 50);
    assert_eq!(state.tokens, "1,234");
    assert!(state.cost.contains("$0.05"));
}

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

#[test]
fn test_carriage_return_handling() {
    // Simulate a progress bar update
    let input = b"Progress: 25%\rProgress: 50%\rProgress: 75%";

    let mut extractor = ClaudeStateExtractor::new();

    // Process all at once
    let state = extractor.process(input);

    // Should end up with 75%
    assert!(state.is_some());
    assert_eq!(state.unwrap().progress, 75);
}
