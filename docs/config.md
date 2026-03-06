# Configuration

Path: `~/.config/custerm/config.toml`

## Generate Default Config

```bash
custerm --init-config
```

## Print Config Path

```bash
custerm --config-path
```

## Full Example

```toml
[terminal]
shell = "/bin/zsh"
font_family = "JetBrainsMono Nerd Font Mono"
font_size = 14

[background]
directory = "/mnt/disk2/Wallpapers/"
interval = 300      # Auto-rotation interval in seconds
tint = 0.9          # Tint overlay opacity (0.0 = no tint, 1.0 = fully opaque)
opacity = 0.95      # Terminal opacity

[socket]
path = "/tmp/custerm.sock"

[theme]
name = "catppuccin-mocha"
```

## Sections

### [terminal]

| Key | Default | Description |
|-----|---------|-------------|
| `shell` | `$SHELL` or `/bin/sh` | Shell to spawn |
| `font_family` | `JetBrainsMono Nerd Font Mono` | Font family |
| `font_size` | `14` | Font size in points |

### [background]

| Key | Default | Description |
|-----|---------|-------------|
| `directory` | — (optional) | Path to wallpaper directory |
| `interval` | `300` | Auto-rotation interval (seconds) |
| `tint` | `0.9` | Tint overlay opacity |
| `opacity` | `0.95` | Terminal opacity |

### [socket]

| Key | Default | Description |
|-----|---------|-------------|
| `path` | `/tmp/custerm.sock` | Unix socket path |

### [theme]

| Key | Default | Description |
|-----|---------|-------------|
| `name` | `catppuccin-mocha` | Theme name |

## Notes

- All fields have defaults; config file is optional
- Missing sections are filled with defaults via `#[serde(default)]`
- Config is read once at startup (hot-reload not yet implemented)
