# Technical Decisions

## 1. Tauri v2 Abandoned → Native Platform UIs

**Problem:** Tauri IPC introduced noticeable input latency in the terminal. Every keypress went through JS → Tauri invoke → Rust → PTY, and output went PTY → Rust → Tauri event → JS → xterm.js. The round-trip was perceptible.

**Decision:** Switched to platform-native UIs with a shared Rust core:

- Linux: GTK4 + VTE4 (VTE handles PTY internally, zero IPC overhead)
- macOS: Swift/AppKit (SwiftTerm or Ghostty embedding, TBD)

**Tradeoff:** More code per platform, but terminal responsiveness is non-negotiable.

## 2. VTE Handles PTY on Linux

**Rationale:** VTE has its own optimized PTY management. Using `portable-pty` alongside VTE would mean double PTY handling. Let VTE do what it does best.

**Consequence:** `turm-core/pty.rs` is not used by turm-linux. It exists for macOS and potential future socket server needs.

## 3. D-Bus for Linux IPC (Not Unix Socket)

**Rationale:** D-Bus is the standard Linux IPC mechanism. Using it means:

- No custom socket server needed
- System integration (other tools can control turm)
- Session bus handles lifecycle automatically

**GTK thread safety issue:** GTK widgets are not `Send+Sync`. D-Bus callbacks can't directly modify widgets.

**Solution:** `mpsc::channel` + `glib::timeout_add_local(50ms)` polling on the GTK main thread. D-Bus handler sends commands through the channel, GTK main loop polls and applies them.

**Note:** `glib::MainContext::channel` was removed in newer glib versions, so we use `std::sync::mpsc` with manual polling instead.

## 4. GtkOverlay for Background Compositing

**Stack:** `bg_picture` (child) → `tint_overlay` (overlay) → `terminal` (overlay)

**Critical detail:** VTE paints its own opaque background by default. To see the image layers beneath, you must:

1. Call `terminal.set_clear_background(false)`
2. Set VTE background color to transparent `RGBA(0,0,0,0)`

Without step 1, VTE covers the entire overlay with its own background color.

## 5. Binary Names: turm + turmctl

**Problem:** Both turm-linux and turm-cli had `[[bin]] name = "turm"`, causing Cargo output filename collision.

**Decision:** CLI binary renamed to `turmctl` (follows kubectl, sysctl naming convention).

## 6. Theme System

**Design:** Themes are defined as `Theme` structs in `turm-core/theme.rs` with semantic color slots (foreground, background, 16-color palette, surface/overlay/accent UI colors). 10 built-in themes are embedded. All UI components (terminal, tab bar, search bar, webview URL bar, window background) use theme colors via CSS generation functions.

**Config:** `[theme] name = "catppuccin-mocha"` selects the active theme. Hot-reloads on config change.

**Built-in themes:** catppuccin-mocha (default), catppuccin-latte, catppuccin-frappe, catppuccin-macchiato, dracula, nord, tokyo-night, gruvbox-dark, one-dark, solarized-dark.

## 7. cmux V2 Protocol for Socket Communication

**Format:** Newline-delimited JSON with UUID request IDs.
**Reference:** ~/dev/cmux/ (Marshall's macOS terminal multiplexer)

This protocol is used by both turmctl and the turm-linux socket server. D-Bus remains for system integration (background control), while the socket API handles all rich control (tabs, splits, webview, terminal agent, approval workflow).

## 8. Forced Dark Theme

**Problem:** When VTE background is transparent (for bg images) and no image is loaded yet, the system GTK theme shows through. On light themes this makes the terminal white.

**Fix:** Force dark theme in `app.rs` via `set_gtk_application_prefer_dark_theme(true)` + CSS `window { background-color: #1e1e2e; }`.

## 9. Rust Edition 2024

Using the latest Rust edition. No compatibility concerns since the project is new.

## 10. In-Terminal Search via VTE Regex

**Problem:** Popular terminals (Ghostty, Kitty) lack built-in Ctrl+F search, requiring piping through external tools.

**Decision:** Implemented search using VTE4's built-in `search_set_regex` / `search_find_next` / `search_find_previous` with PCRE2 regex. Search bar is a `gtk4::Box` overlay at the bottom of each terminal panel.

**UX details:**

- Search text is preserved when closing, but fully selected on reopen (type to replace, Enter to reuse)
- `glib::idle_add_local_once` is needed for `select_region` — GTK4 Entry ignores selection before focus is fully settled

## 11. Configurable Tab Position

**Decision:** Tab bar position (`top`, `bottom`, `left`, `right`) is configurable via `[tabs] position` in config. Uses `gtk4::Notebook::set_tab_pos()`. Hot-reloads on config change.

**Rationale:** Vertical tabs (left/right) make better use of widescreen displays and are preferred by some users.

## 12. CEF (Chromium) for WebView Panels

**Decision:** Use CEF (Chromium Embedded Framework) via `cef-rs` crate for embedded browser panels.

**Previous approach:** WebKitGTK 6.0 — abandoned due to an upstream WebKitGTK 2.50.x bug where complex Vite/React dev server pages crash the WebKitWebProcess (SIGABRT in JSC). Confirmed reproducible in MiniBrowser.

**Architecture:**
- **Off-Screen Rendering (OSR):** CEF renders to BGRA pixel buffer → `gdk4::MemoryTexture` → `gtk4::Picture`. Required on Wayland (no X11 window reparenting available).
- **External message pump:** `cef::do_message_loop_work()` called every 10ms from GTK4's main loop via `glib::timeout_add_local`.
- **Multi-process:** CEF spawns renderer/GPU sub-processes by re-launching the same binary with `--type=renderer`. Detected and handled at the top of `main()`.
- **JS execution:** DevTools protocol (`Runtime.evaluate` via `BrowserHost::send_dev_tools_message`) for results, `Frame::execute_java_script` for fire-and-forget.
- **Input forwarding:** GTK4 EventControllers → CEF's `send_key_event`, `send_mouse_click_event`, `send_mouse_move_event`, `send_mouse_wheel_event`.
- **Plugin JS bridge:** Uses CEF's `cefQuery` mechanism instead of WebKit's `register_script_message_handler_with_reply`.

**Tradeoff:** More complex embedding code (OSR, input forwarding, size polling) but far more robust rendering — no crashes on complex web pages.

## 13. Socket Auto-Discovery in turmctl

**Problem:** `TURM_SOCKET` env var often points to a dead turm process socket, making turmctl fail.

**Decision:** turmctl auto-discovers sockets by scanning `/tmp/turm-*.sock`, sorting by modification time (newest first), and trying to connect. Falls back to `/tmp/turm.sock` if none found.

**Priority order:**
1. `--socket` CLI flag (explicit)
2. `TURM_SOCKET` env var (if connectable)
3. Auto-discovered newest live socket from `/tmp/turm-*.sock`
4. `/tmp/turm.sock` (fallback)
