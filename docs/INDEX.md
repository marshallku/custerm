# custerm Documentation Index

## File Structure

| File | Purpose | When to Read |
|------|---------|--------------|
| [architecture.md](./architecture.md) | Project structure, crate layout, tech stack | Starting work, understanding the codebase |
| [linux-app.md](./linux-app.md) | GTK4 + VTE4 Linux app internals | Working on custerm-linux |
| [core-lib.md](./core-lib.md) | Shared Rust core library modules | Working on custerm-core |
| [cli.md](./cli.md) | CLI tool (custermctl) and D-Bus interface | Working on remote control features |
| [config.md](./config.md) | Configuration format and defaults | Adding config options |
| [decisions.md](./decisions.md) | Key technical decisions and rationale | Understanding "why" behind choices |
| [troubleshooting.md](./troubleshooting.md) | Known issues, fixes, gotchas | Debugging problems |
| [roadmap.md](./roadmap.md) | Implementation phases, pending work | Planning next steps |

## Quick Reference

- **Binary names**: `custerm` (terminal app), `custermctl` (CLI control tool)
- **Config path**: `~/.config/custerm/config.toml`
- **Cache path**: `~/.cache/custerm/wallpapers.txt`
- **D-Bus bus name**: `com.marshall.custerm`
- **GTK app ID**: `com.marshall.custerm`
- **Theme**: Catppuccin Mocha
- **Rust edition**: 2024
