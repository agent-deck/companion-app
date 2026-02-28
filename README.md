# AgentDeck

Companion software for the Agent Deck macropad — a hardware control surface for Claude Code. Connects the USB macropad's OLED display, soft keys, mode LEDs, and rotary encoder to your terminal sessions.

## Architecture

The system consists of two binaries:

- **`agentdeck-daemon`** — Background service that owns the HID device. Exposes HTTP REST and WebSocket APIs on `127.0.0.1:19384`. Runs as a tray icon with no dock presence.
- **`agent-deck`** — GUI app with embedded terminal emulator (wezterm-term + egui). Connects to the daemon via WebSocket for real-time device control.

Third-party tools can also integrate with the daemon via its REST API — no GUI app required.

## Building

Requires Rust 1.75+ (stable).

```bash
# macOS: ensure Xcode CLI tools are installed
xcode-select --install

# Build everything
cargo build --workspace

# Release build (LTO, stripped)
cargo build --workspace --release
```

Output binaries: `target/release/agent-deck` and `target/release/agentdeck-daemon`.

See [docs/Building.md](docs/Building.md) for Linux dependencies, individual crate builds, and detailed notes.

## Running

```bash
# Start the daemon (must be running first)
agentdeck-daemon

# Install as launchd service for auto-start
agentdeck-daemon install

# Start the GUI app
agent-deck
```

## Quick API Test

With the daemon running and no GUI app connected:

```bash
# Check device status
curl -s http://127.0.0.1:19384/api/status | jq

# Update the display
curl -X POST http://127.0.0.1:19384/api/display \
  -H 'Content-Type: application/json' \
  -d '{"session": "my-project", "task": "Building...", "tabs": [0, 2, 1], "active": 1}'

# Show an alert
curl -X POST http://127.0.0.1:19384/api/alert \
  -H 'Content-Type: application/json' \
  -d '{"tab": 0, "session": "my-project", "text": "Done!", "details": "All tests passed"}'
```

## Workspace Structure

```
crates/
  agentdeck-protocol/   # Shared types & wire format (serde only)
  agentdeck-daemon/     # Background daemon (HID, tray, axum server)
  agentdeck/            # GUI app (egui, wezterm-term, PTY)
docs/                   # API documentation
```

## Documentation

- [Building from Source](docs/Building.md) — Prerequisites, build commands, workspace layout
- [Daemon API Overview](docs/API.md) — How the daemon works, access modes, quick examples
- [REST API Reference](docs/REST-API.md) — All HTTP endpoints with request/response schemas
- [WebSocket Protocol](docs/WebSocket-Protocol.md) — Binary WS protocol for real-time control
- [Protocol Limits](docs/Protocol-Limits.md) — Hard limits on text, tabs, brightness, payloads
- [Shared Types](docs/Types.md) — JSON schemas for all API types

## License

GPL-3.0-or-later
