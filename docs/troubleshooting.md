# Troubleshooting

## Build Issues

### Missing vte4 system library

```
error: could not find system library 'vte-2.91-gtk4'
```

**Fix:** `sudo pacman -S vte4`

### Missing gtk4 system library

**Fix:** `sudo pacman -S gtk4`

### `load_from_string` not found on CssProvider

The method is gated behind a feature flag.
**Fix:** Add `features = ["gnome_46"]` to gtk4 dependency in Cargo.toml.

### Cargo binary name collision

```
warning: output filename collision at target/debug/turm
```

turm-linux and turm-cli both output `turm`.
**Fix:** CLI binary renamed to `turmctl` in turm-cli/Cargo.toml.

## Runtime Issues

### Wayland protocol error (Error 71)

```
Gdk-Message: Error 71 (Protocol error)
```

**Fix:** Set `GDK_BACKEND=x11` in environment or in main.rs.

### GBM buffer error

```
Failed to create GBM buffer of size 841x1352: Invalid argument
```

**Fix:** Set `WEBKIT_DISABLE_DMABUF_RENDERER=1` (only relevant if using WebKit components).

### Terminal shows in light mode

**Cause:** Transparent VTE background with no image loaded shows system theme underneath.
**Fix:**

1. Force dark theme: `settings.set_gtk_application_prefer_dark_theme(true)` in `app.rs`
2. Set opaque VTE bg by default, only go transparent when bg image is applied

### Background images not showing (solid color only)

Multiple possible causes:

1. **Config `directory` is commented out**: Check `~/.config/turm/config.toml`. The `directory` field must be uncommented. A `#` before the key comments it out.

2. **VTE paints opaque background**: Call `terminal.set_clear_background(false)` in `set_background()`. Without this, VTE covers the image layer.

3. **Image loading fails silently**: The original `GtkPicture::set_file()` loads asynchronously and fails silently. Fixed by using `gdk::Texture::from_file()` for synchronous loading with error reporting.

4. **Tint too opaque**: Tint at 0.9 makes images nearly invisible (90% opaque dark overlay). Lower to 0.85 or less.

5. **GTK single-instance**: If an old turm is running, new launches activate the old instance and exit immediately (exit code 0, no output). Kill all instances first: `killall turm`.

### App exits immediately with no error

**Cause:** GTK single-instance behavior. Another turm instance already owns the D-Bus app ID `com.marshall.turm`.
**Fix:** `killall turm` then relaunch.

### env_logger output not visible

**Cause:** GTK may capture/redirect stderr. `RUST_LOG=info` has no visible effect.
**Fix:** Use `eprintln!("[turm] ...")` instead of `log::info!()` for debug output.

### D-Bus: GTK widgets not Send+Sync

**Problem:** D-Bus callbacks need `Send+Sync` closures, but GTK widgets can't be sent across threads.
**Fix:** Use `mpsc::channel` to send commands from D-Bus handler to GTK main thread. Poll with `glib::timeout_add_local(50ms)`.

### D-Bus: `glib::MainContext::channel` not found

**Cause:** Removed in newer glib crate versions.
**Fix:** Use `std::sync::mpsc` + `glib::timeout_add_local` polling instead.

### Terminal shows only one line (collapsed height)

**Cause:** `GtkOverlay` sizes based on its child widget (`bg_picture`). When no background image is set, `bg_picture` is hidden and has zero natural size, collapsing the entire overlay.
**Fix:** Call `overlay.set_measure_overlay(&terminal, true)` so the terminal overlay widget contributes to size measurement even when `bg_picture` is hidden. Also set `overlay.set_hexpand(true)` and `overlay.set_vexpand(true)`.

### D-Bus: `register_object` API mismatch

**Cause:** gio 0.20 uses builder pattern, not positional args.
**Fix:** Use `connection.register_object(path, &interface_info).method_call(closure).build()`.

### VTE 0.82: `shell-precmd` / `shell-preexec` / `notification-received` signal panic

```
signal 'shell-precmd' is invalid for instance '...' of type 'VteTerminal'
```

**Cause:** VTE 0.82 removed `shell-precmd`, `shell-preexec`, and `notification-received` signals. The Rust `vte4` crate still exposes `connect_shell_precmd()` etc., but the underlying GObject signal doesn't exist.

**Fix:** Guard signal connections with `SignalId::lookup()` before connecting:

```rust
use gtk4::glib::object::ObjectExt;
use gtk4::glib::subclass::signal::SignalId;
if SignalId::lookup("shell-precmd", term_obj.type_()).is_some() {
    term.terminal.connect_shell_precmd(move |_term| { ... });
}
```

### CEF: Browser panel shows blank / no rendering

**Possible causes:**
1. **CEF binaries not found:** CEF shared libraries (libcef.so etc.) must be in `LD_LIBRARY_PATH`. They're auto-downloaded to the build output dir during `cargo build`. Set `LD_LIBRARY_PATH` to include the CEF directory.
2. **GPU issues on Wayland:** CEF is configured with `--disable-gpu` and `--disable-gpu-compositing` flags. If rendering issues occur, check CEF subprocess logs.
3. **Size not detected:** CEF OSR uses polling (every 200ms) to detect size changes. The Picture widget must have a non-zero allocation.

### CEF: Input not working in browser panel

**Cause:** GTK4 EventControllers forward input to CEF via `send_key_event`, `send_mouse_click_event`, etc. If the Picture widget doesn't have focus, events won't be captured.
**Fix:** Ensure the Picture widget has `set_focusable(true)` and `set_can_focus(true)`.

### Historical: WebKitGTK crashes (reason for CEF migration)

WebKitGTK 2.50.x had an upstream bug where complex Vite/React dev server pages crash the WebKitWebProcess (SIGABRT in JSC). Confirmed reproducible in MiniBrowser. This was the primary motivation for migrating to CEF/Chromium. See decisions.md #12.
