# Core Library (turm-core)

Shared Rust library used by all platform targets.

## Modules

### config.rs

TOML config at `~/.config/turm/config.toml`.

```rust
TurmConfig {
    terminal: TerminalConfig { shell, font_family, font_size },
    background: BackgroundConfig { directory, interval, tint, opacity },
    socket: SocketConfig { path },
    theme: ThemeConfig { name },
}
```

Key methods:

- `TurmConfig::load()` — reads config file, returns defaults if missing
- `TurmConfig::write_default()` — creates default config file
- `TurmConfig::config_path()` — returns `~/.config/turm/config.toml`

Defaults:

- shell: `$SHELL` or `/bin/sh`
- font: JetBrainsMono Nerd Font Mono, size 14
- tint: 0.9, opacity: 0.95
- socket: `/tmp/turm.sock`
- theme: `catppuccin-mocha`

### background.rs

Background image cache manager.

```rust
BackgroundManager {
    directory: Option<PathBuf>,
    cache_file: PathBuf,        // ~/.cache/turm/wallpapers.txt
    current: Option<PathBuf>,
    cached_images: Vec<PathBuf>,
}
```

Key methods:

- `load_cache()` — reads cache file, rebuilds if empty or missing
- `rebuild_cache()` — scans directory for image files (jpg, jpeg, png, webp, bmp)
- `next()` — picks random image, avoids current. Uses `rand::seq::IndexedRandom` (rand 0.9 API)
- `delete_current()` — removes current from cache, updates cache file

### protocol.rs

cmux V2 compatible newline-delimited JSON protocol.

```rust
Request { id: String, method: String, params: serde_json::Value }
Response { id: String, ok: bool, result: Option<Value>, error: Option<ResponseError> }
ResponseError { code: String, message: String }
```

Used by turm-cli for socket communication.

### error.rs

```rust
enum TurmError { Io, Config, Protocol }
type Result<T> = std::result::Result<T, TurmError>;
```

### action_registry.rs

Name → handler map for all invocable actions (see [workflow-runtime.md](./workflow-runtime.md)). v1 is synchronous; async registration will be added when the first service provider (Calendar, Slack) needs non-blocking I/O.

```rust
ActionRegistry::new()
ActionRegistry::register(name, |params| -> Result<Value, ResponseError>)
ActionRegistry::invoke(name, params) -> Result<Value, ResponseError>
ActionRegistry::has(name) -> bool
ActionRegistry::names() -> Vec<String>   // sorted
ActionRegistry::len() / is_empty()
```

**Errors:** returns `turm_core::protocol::ResponseError` so socket dispatch can wrap it directly in a `Response::error(...)`. Error helpers:

- `action_registry::invalid_params(msg)` — `code: "invalid_params"`
- `action_registry::internal_error(msg)` — `code: "internal_error"`
- Unknown action — `code: "action_not_found"` (produced by `invoke()`)

**Thread safety:** backed by `RwLock<HashMap>`. Multiple threads can invoke concurrently; registration takes a short write lock. Handlers must be `Fn(Value) -> ActionResult + Send + Sync + 'static` — state capture via `Arc<Mutex<T>>` or `Arc<AtomicX>`.

**Not wired yet:** this is a pure primitive. `turm-linux/socket.rs`'s `dispatch()` still uses its hard-coded match. Migration is incremental — new commands register through the registry; legacy commands move over one at a time.

### event_bus.rs

In-process pub/sub hub for the workflow runtime (see [workflow-runtime.md](./workflow-runtime.md)). All internal event sources (shell signals, VTE output, service providers) publish through this bus; the socket `event.subscribe` stream becomes a projection of it.

```rust
Event { kind: String, source: String, timestamp_ms: u64, payload: Value }
EventBus::new() / with_default_buffer(n)
EventBus::publish(event)
EventBus::subscribe(pattern) -> EventReceiver               // bounded (default)
EventBus::subscribe_with_buffer(pattern, n) -> EventReceiver // bounded, custom size
EventBus::subscribe_unbounded(pattern) -> EventReceiver     // lossless, for wire streams
EventReceiver::try_recv() -> Option<Event>
EventReceiver::recv() -> Option<Event>
```

**Pattern matching:** `*` matches all, `foo.*` matches any kind starting with `foo.` (deep — `foo.bar.baz` matches), otherwise exact string match.

**Delivery modes:**
- **Bounded** (`subscribe` / `subscribe_with_buffer`, default 256): `sync_channel` + `try_send`. On full buffer, the new event is dropped for that subscriber with a warn log — publisher never blocks. Right choice for in-process consumers that poll (plugin panels, UI bridges).
- **Unbounded** (`subscribe_unbounded`): plain `mpsc::channel`. Never drops; memory grows if consumer stalls. Required for external wire contracts (e.g. socket `event.subscribe`) where event loss would violate the client API. The caller must drain promptly or risk unbounded memory.

Disconnected subscribers are pruned lazily on the next publish (both modes).

**Thread safety:** `EventBus` is `Sync`; any thread can publish. Receivers are single-consumer (not `Clone`) — platform UIs drain via `try_recv` on their main thread (GTK: `glib::timeout_add_local`; AppKit: DispatchSource).
