# Terminal Emulator - Future Improvements

This document tracks potential improvements for the Agent Deck terminal emulator.

## Current Implementation

Based on WezTerm's `term` crate with custom egui rendering.

### Supported Features
- Full escape sequence parsing (via wezterm-term)
- 256 color and true color support
- Text attributes: bold, dim, italic, underline, strikethrough, invisible, reverse video
- Scrollback buffer (10,000 lines)
- Cursor rendering (respects visibility for TUI apps)
- PTY integration with resize support

## Completed

- [x] **Selection & Copy** - Mouse selection with clipboard support
- [x] **Mouse Events** - Pass mouse clicks/scroll to PTY for TUI apps (SGR mode)
- [x] **Hyperlinks (OSC 8)** - Clickable URLs in terminal output with tooltip
- [x] **Bold Fonts** - Semibold font variant + color brightening

## Future Improvements

### Medium Priority
- [ ] **URL Detection** - Auto-detect and highlight URLs (without OSC 8)
- [ ] **Different Underline Styles** - Single, double, curly, dotted

### Low Priority / Nice to Have
- [ ] **Search** - Find text in scrollback buffer (Claude Code has Ctrl-R for history)
- [ ] **Sixel/Kitty/iTerm2 Images** - Terminal image protocols (waiting on Claude Code support - [#2266](https://github.com/anthropics/claude-code/issues/2266))
- [ ] **Ligatures** - Programming font ligatures
- [ ] **Blink Animation** - Blinking text/cursor animation
- [ ] **Bell** - Visual/audio bell notification

## Architecture Notes

- `terminal/session.rs` - Wraps wezterm-term's Terminal
- `terminal/config.rs` - Implements TerminalConfiguration trait
- `window/terminal.rs` - egui-based rendering and input handling
- Cursor for TUI apps (like Claude Code) uses reverse video, not terminal cursor
