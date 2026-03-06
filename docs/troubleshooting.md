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
warning: output filename collision at target/debug/custerm
```
custerm-linux and custerm-cli both output `custerm`.
**Fix:** CLI binary renamed to `custermctl` in custerm-cli/Cargo.toml.

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
**Cause:** VTE paints its own opaque background over the GtkOverlay layers.
**Fix:** Call `terminal.set_clear_background(false)` in `set_background()` method. This stops VTE from painting its own background, allowing the image layer to show through.

### D-Bus: GTK widgets not Send+Sync
**Problem:** D-Bus callbacks need `Send+Sync` closures, but GTK widgets can't be sent across threads.
**Fix:** Use `mpsc::channel` to send commands from D-Bus handler to GTK main thread. Poll with `glib::timeout_add_local(50ms)`.

### D-Bus: `glib::MainContext::channel` not found
**Cause:** Removed in newer glib crate versions.
**Fix:** Use `std::sync::mpsc` + `glib::timeout_add_local` polling instead.

### D-Bus: `register_object` API mismatch
**Cause:** gio 0.20 uses builder pattern, not positional args.
**Fix:** Use `connection.register_object(path, &interface_info).method_call(closure).build()`.
