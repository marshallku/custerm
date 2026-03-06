# Core Library (custerm-core)

Shared Rust library used by all platform targets.

## Modules

### config.rs

TOML config at `~/.config/custerm/config.toml`.

```rust
CustermConfig {
    terminal: TerminalConfig { shell, font_family, font_size },
    background: BackgroundConfig { directory, interval, tint, opacity },
    socket: SocketConfig { path },
    theme: ThemeConfig { name },
}
```

Key methods:
- `CustermConfig::load()` — reads config file, returns defaults if missing
- `CustermConfig::write_default()` — creates default config file
- `CustermConfig::config_path()` — returns `~/.config/custerm/config.toml`

Defaults:
- shell: `$SHELL` or `/bin/sh`
- font: JetBrainsMono Nerd Font Mono, size 14
- tint: 0.9, opacity: 0.95
- socket: `/tmp/custerm.sock`
- theme: `catppuccin-mocha`

### background.rs

Background image cache manager.

```rust
BackgroundManager {
    directory: Option<PathBuf>,
    cache_file: PathBuf,        // ~/.cache/custerm/wallpapers.txt
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

Used by custerm-cli for socket communication.

### state.rs

Application state model.

```rust
AppState {
    config: CustermConfig,
    sessions: Mutex<HashMap<String, PtySession>>,
    workspaces: Mutex<Vec<Workspace>>,
    active_workspace: Mutex<Option<String>>,
}

Workspace { id, name, sessions: Vec<String>, focused_session: Option<String> }
```

**Note:** On Linux, VTE handles PTY internally. This state model is used for socket server features and is not yet wired into custerm-linux.

### pty.rs

Cross-platform PTY session using `portable-pty`.

```rust
PtySession { master, child, input_tx: mpsc::Sender<Vec<u8>> }
```

- Input: mpsc channel → dedicated writer thread (no Mutex on hot path)
- Output: reader thread → callback function
- Buffer: 64KB reads

**Note:** Not used by custerm-linux (VTE handles PTY). Intended for macOS and future socket server.

### error.rs

```rust
enum CustermError { Pty, Io, Config, SessionNotFound, Protocol }
type Result<T> = std::result::Result<T, CustermError>;
```
