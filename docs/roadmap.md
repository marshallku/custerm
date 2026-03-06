# Roadmap

## Implementation Phases

### Phase 1: MVP Terminal ✅
- [x] Cargo workspace with custerm-core, custerm-linux, custerm-cli
- [x] GTK4 + VTE4 native terminal
- [x] Shell spawn (from config)
- [x] Font configuration
- [x] Dynamic font scaling (Ctrl+=/−/0)
- [x] Catppuccin Mocha theme
- [x] TOML config loading
- [x] `--init-config` and `--config-path` CLI flags
- [x] Dark theme forced
- [x] Desktop entry + install script

### Phase 2: Background Images ✅
- [x] Background image cache (scan directory, cache to file)
- [x] GtkOverlay compositing (image → tint → terminal)
- [x] VTE transparent background (`set_clear_background(false)`)
- [x] Random image selection
- [x] Tint overlay with configurable opacity
- [x] D-Bus interface for dynamic control
  - [x] SetBackground, NextBackground, ClearBackground
  - [x] SetTint, GetCurrentBackground

### Phase 3: Tabs + Splits (Not Started)
- [ ] Tab model (multiple TerminalTabs)
- [ ] TabBar component (new/close/switch)
- [ ] Split pane layout (horizontal/vertical)
- [ ] Pane resize
- [ ] Focus tracking
- [ ] Keyboard shortcuts: Ctrl+Shift+T/W/E/O/[1-9]

### Phase 4: Socket API + CLI (Partial)
- [x] CLI tool (custermctl) with clap subcommands
- [x] cmux V2 JSON protocol types
- [x] Unix socket client
- [ ] Socket server in custerm-linux
- [ ] Command dispatch (wire CLI commands to actual actions)
- [ ] Env var injection per session (CUSTERM_SOCKET, CUSTERM_SESSION_ID)

### Phase 5: macOS App (Stub Only)
- [x] Swift Package with basic NSWindow
- [ ] Terminal view (SwiftTerm or Ghostty embedding)
- [ ] PTY integration via custerm-core
- [ ] Background images
- [ ] Socket/IPC control

### Phase 6: Integrations + Polish (Not Started)
- [ ] Config hot-reload
- [ ] Background auto-rotation daemon (timer based on config interval)
- [ ] Git sidebar (branch, PR status)
- [ ] AI agent notifications (OSC 9/99/777 parsing)
- [ ] Clipboard integration
- [ ] URL detection
- [ ] Session persistence/restore
- [ ] Browser panel integration

## Pending Cleanup
- [ ] Verify background images render correctly after `set_clear_background(false)` fix
- [ ] Test D-Bus interface end-to-end
- [ ] Consider whether custerm-core/pty.rs and state.rs are needed for Linux (VTE handles PTY)

## Reference Projects
- `~/dev/cmux/` — Socket protocol, CLI structure, window/workspace model
- `~/dotfiles/zsh/kitty-random-bg.sh` — Background rotation logic (ported to `background.rs`)
