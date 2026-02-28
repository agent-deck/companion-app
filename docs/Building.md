# Building from Source

## Prerequisites

### Rust Toolchain

Rust 1.75 or later (stable). Install via [rustup](https://rustup.rs/):

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

### System Dependencies

#### macOS

```bash
# Xcode command line tools (provides system frameworks: AppKit, CoreFoundation, IOKit)
xcode-select --install
```

No additional packages required. The macOS system frameworks (`cocoa`, `core-foundation`, `IOKit`) are accessed via Rust crate bindings.

The `hidapi` crate links against IOKit for USB HID access. No Homebrew packages needed.

#### Linux

```bash
# Debian/Ubuntu
sudo apt install build-essential pkg-config libudev-dev libhidapi-dev \
  libx11-dev libxcb1-dev libxkbcommon-dev libgl1-mesa-dev libfontconfig1-dev

# Fedora
sudo dnf install gcc pkg-config systemd-devel hidapi-devel \
  libX11-devel libxcb-devel libxkbcommon-devel mesa-libGL-devel fontconfig-devel
```

## Workspace Structure

The project is a Cargo workspace with 3 crates:

```
crates/
  coredeck-protocol/   # Shared types & wire format (serde only, no system deps)
  coredeck-daemon/     # Background daemon (HID, tray icon, axum server)
  coredeck/            # GUI app (egui, wezterm-term, PTY)
```

The default member is `coredeck` (the GUI app), so a bare `cargo build` only builds the app and its dependencies.

## Build Commands

### Build everything

```bash
cargo build --workspace
```

### Build individual crates

```bash
# Daemon only
cargo build -p coredeck-daemon

# GUI app only (default)
cargo build -p core-deck
# or just:
cargo build

# Protocol crate only
cargo build -p coredeck-protocol
```

### Release build

```bash
cargo build --workspace --release
```

Release profile uses `opt-level = 3`, LTO, single codegen unit, and symbol stripping for minimal binary size.

Output binaries:

| Binary | Path |
|--------|------|
| `core-deck` | `target/release/core-deck` |
| `coredeck-daemon` | `target/release/coredeck-daemon` |

### Run

```bash
# Run the GUI app
cargo run -p core-deck

# Run the daemon
cargo run -p coredeck-daemon

# Run the daemon on a custom port
cargo run -p coredeck-daemon -- --listen 127.0.0.1:9000
```

### Tests

```bash
cargo test --workspace
```

## Notes

### Patched Dependencies

The workspace patches `zune-jpeg` (vendored in `patches/zune-jpeg/`) to fix an unsafe neon SIMD issue on aarch64. This is applied automatically via `[patch.crates-io]` in the root `Cargo.toml`.

### Build Scripts

Both the root workspace and the `coredeck` crate have `build.rs` scripts that generate placeholder tray icon PNGs (16x16 solid color) if they don't already exist in `assets/icons/`. Real icons are checked into the repo, so the build scripts are effectively no-ops on a normal clone.

### WezTerm Git Dependencies

The GUI app depends on several crates from the WezTerm repository, pinned to a specific commit (`05343b3`). The first build will clone and compile these, which takes a few minutes. Subsequent builds use the cached checkout.

### Optional: Ghostty Themes

The GUI app loads terminal color themes from Ghostty.app if installed at `/Applications/Ghostty.app`. This is entirely optional — the app ships with built-in Dark and Light themes.

### Logging

Both binaries use `tracing` with `RUST_LOG` env filter:

```bash
RUST_LOG=debug cargo run -p coredeck-daemon
RUST_LOG=core_deck=trace cargo run -p core-deck
```
