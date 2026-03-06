# custerm

Cross-platform custom terminal emulator with shared Rust core and platform-native UIs.

## Documentation

**Always read `docs/INDEX.md` first** when starting a session. Read only the specific doc files relevant to your current task.

**Always update docs** when making changes:
- New features or modules → update `docs/architecture.md` and relevant doc
- Bug fixes or gotchas → add to `docs/troubleshooting.md`
- Design decisions → add to `docs/decisions.md`
- Completed/new tasks → update `docs/roadmap.md`

## Project Structure

- `custerm-core/` — Shared Rust library (config, background, protocol, state, pty, error)
- `custerm-linux/` — GTK4 + VTE4 native terminal app (binary: `custerm`)
- `custerm-cli/` — CLI control tool (binary: `custermctl`)
- `custerm-macos/` — Swift/AppKit app (stub)
- `docs/` — Project documentation (architecture, decisions, troubleshooting, roadmap)

## Build & Run

```bash
# Build all
cargo build

# Run terminal
cargo run -p custerm-linux

# Run CLI
cargo run -p custerm-cli -- <command>
```

## Key Conventions

- Rust edition 2024, Cargo workspace with `resolver = "2"`
- GTK4 with `gnome_46` feature flag
- VTE handles PTY on Linux (no custom PTY management)
- D-Bus (`com.marshall.custerm`) for Linux IPC
- Config: `~/.config/custerm/config.toml` (TOML)
- Cache: `~/.cache/custerm/wallpapers.txt`
- Theme: Catppuccin Mocha (hardcoded)
- Dark theme forced via GTK settings

## Critical Implementation Details

- **Background images**: Must call `terminal.set_clear_background(false)` for VTE transparency
- **GTK thread safety**: D-Bus → mpsc channel → glib::timeout_add_local polling
- **Binary names**: `custerm` (app) and `custermctl` (CLI) — do not rename to collide
