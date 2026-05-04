# Plugin Development Guide

## Overview

nestty plugins extend the terminal with custom panels (HTML/JS UIs) and commands (shell scripts). Plugins live in `~/.config/nestty/plugins/` and are discovered automatically at startup.

## Plugin Structure

```
~/.config/nestty/plugins/my-plugin/
├── plugin.toml          # Manifest (required)
├── index.html           # Panel UI
├── styles.css           # Panel styles
├── main.js              # Panel logic
└── scripts/
    └── do-thing.sh      # Shell command
```

## Manifest (`plugin.toml`)

```toml
[plugin]
name = "my-plugin"        # Unique identifier (kebab-case)
title = "My Plugin"        # Display name
version = "0.1.0"
description = "What this plugin does"

# Panels are HTML UIs rendered in WebView tabs
[[panels]]
name = "main"                              # Panel identifier
title = "My Panel"                         # Tab title
file = "index.html"                        # HTML file to load (relative to plugin dir)
icon = "applications-system-symbolic"      # GTK icon name (optional)

# Commands are shell scripts callable via the socket API
[[commands]]
name = "do-thing"                          # Command identifier
exec = "bash scripts/do-thing.sh"          # Shell command to run
description = "Does a thing"               # Optional description

# Modules are widgets rendered in the status bar (data from shell, styled with CSS)
[[modules]]
name = "clock"                             # Module identifier
exec = "date '+%H:%M:%S'"                  # Shell command (stdout → module text)
interval = 1                               # Re-run interval in seconds (default: 10)
position = "right"                         # left, center, right
order = 100                                # Sort order within section (lower = first)
class = "clock"                            # CSS class for styling (optional)
```

### Multiple Panels

A plugin can define multiple panels:

```toml
[[panels]]
name = "main"
title = "Dashboard"
file = "index.html"

[[panels]]
name = "settings"
title = "Settings"
file = "settings.html"
icon = "preferences-system-symbolic"
```

Open a specific panel: `nestctl plugin open my-plugin --panel settings`

## JS Bridge API

Every plugin panel gets a `window.nestty` object injected automatically.

### `nestty.panel`

Info about the current panel:

```javascript
nestty.panel.id       // UUID of this panel instance
nestty.panel.name     // Panel name from manifest (e.g., "main")
nestty.panel.plugin   // Plugin name (e.g., "my-plugin")
```

### `nestty.call(method, params?)`

Call any nestty socket API method. Returns a Promise.

```javascript
// Get terminal state
const state = await nestty.call("terminal.state");
console.log(state.cwd, state.cols, state.rows);

// Read terminal screen
const screen = await nestty.call("terminal.read");
console.log(screen.text);

// Execute a command in the terminal
await nestty.call("terminal.exec", { command: "ls -la" });

// List all panels
const panels = await nestty.call("session.list");

// Open a webview
const { panel_id } = await nestty.call("webview.open", { url: "https://example.com" });

// Create a new terminal tab
await nestty.call("tab.new");

// List themes
const { themes, current } = await nestty.call("theme.list");

// Run a plugin command
const result = await nestty.call("plugin.my-plugin.greet", { name: "world" });
```

All [socket API methods](./architecture.md#socket-server-ipc) are available.

### `nestty.on(type, callback)` / `nestty.off(type, callback)`

Listen for nestty events:

```javascript
// Listen for focus changes
nestty.on("panel.focused", (data) => {
    console.log("Panel focused:", data.panel_id);
});

// Listen for terminal output
nestty.on("terminal.output", (data) => {
    console.log("Output:", data.text);
});

// Wildcard: listen for all events
nestty.on("*", (type, data) => {
    console.log(`Event: ${type}`, data);
});

// Remove a listener
const handler = (data) => { ... };
nestty.on("panel.focused", handler);
nestty.off("panel.focused", handler);
```

All [event types](./architecture.md#event-stream) are available.

## Theme CSS Variables

Plugin panels automatically receive CSS variables matching the active nestty theme:

```css
:root {
    --nestty-bg: #1e1e2e;
    --nestty-fg: #cdd6f4;
    --nestty-surface0: #313244;
    --nestty-surface1: #45475a;
    --nestty-surface2: #585b70;
    --nestty-overlay0: #6c7086;
    --nestty-text: #cdd6f4;
    --nestty-subtext0: #a6adc8;
    --nestty-subtext1: #bac2de;
    --nestty-accent: #89b4fa;
    --nestty-red: #f38ba8;
}
```

Use these in your CSS to match the terminal theme:

```css
.card {
    background: var(--nestty-surface0);
    border: 1px solid var(--nestty-overlay0);
    color: var(--nestty-text);
    border-radius: 8px;
    padding: 16px;
}

button {
    background: var(--nestty-accent);
    color: var(--nestty-bg);
    border: none;
    border-radius: 4px;
    padding: 8px 16px;
    cursor: pointer;
}

button:hover {
    opacity: 0.9;
}
```

The base `body` style is also set automatically (background color, text color, system font, no margin/padding).

## Plugin Commands

Commands are shell scripts that run when called via `plugin.<name>.<command>`.

### Environment Variables

| Variable | Value |
|----------|-------|
| `NESTTY_SOCKET` | Path to nestty's Unix socket |
| `NESTTY_PLUGIN_DIR` | Absolute path to the plugin directory |

### Input/Output

- **stdin**: JSON params from the caller
- **stdout**: JSON response (or plain text, wrapped as `{"output": "..."}`)
- **stderr**: Logged on failure
- **Exit code**: 0 = success, non-zero = error

### Example Command Script

```bash
#!/bin/bash
# scripts/greet.sh — reads params from stdin, writes JSON to stdout

PARAMS=$(cat)
NAME=$(echo "$PARAMS" | jq -r '.name // "world"')

echo "{\"message\": \"Hello, $NAME!\"}"
```

### Calling Commands

From CLI:
```bash
nestctl plugin run my-plugin.greet --params '{"name": "nestty"}'
```

From a plugin panel's JS:
```javascript
const result = await nestty.call("plugin.my-plugin.greet", { name: "nestty" });
console.log(result.message); // "Hello, nestty!"
```

### Calling nestty from Command Scripts

Commands can call back into nestty via the socket:

```bash
#!/bin/bash
# Use nestctl with the injected socket path
export NESTTY_SOCKET="$NESTTY_SOCKET"
nestctl terminal exec "echo 'hello from plugin'"
```

## CLI

```bash
# List installed plugins
nestctl plugin list

# Open a plugin panel (default panel: "main")
nestctl plugin open my-plugin
nestctl plugin open my-plugin --panel settings

# Run a plugin command
nestctl plugin run my-plugin.greet --params '{"name": "world"}'
```

## GTK Icon Names

Common icons for the `icon` field in panel definitions:

| Icon Name | Use For |
|-----------|---------|
| `utilities-terminal-symbolic` | Terminal-related |
| `applications-system-symbolic` | System/settings |
| `preferences-system-symbolic` | Preferences |
| `folder-symbolic` | File management |
| `web-browser-symbolic` | Web content |
| `edit-find-symbolic` | Search |
| `document-open-symbolic` | Documents |
| `view-list-symbolic` | List views |
| `dialog-information-symbolic` | Info/status |
| `application-x-addon-symbolic` | Generic plugin (default) |

Use `gtk4-icon-browser` to explore all available icons on your system.

## Status Bar Modules

Plugins can contribute modules to the Waybar-style status bar. The bar is a WebView rendering CSS-styled modules, with data provided by shell scripts — similar to Waybar's custom modules.

### Module Manifest

```toml
[[modules]]
name = "clock"
exec = "date '+%H:%M:%S'"    # shell command, stdout → module text
interval = 1                   # re-run every N seconds
position = "right"             # left, center, right
order = 100                    # sort order (lower = first)
class = "clock"                # CSS class for styling
```

### Data Format

Module `exec` stdout supports two formats:

**Plain text** — used as-is:
```
23:45:01
```

**JSON** — with `text` and optional `tooltip`:
```json
{"text": "$12.34 | 62kout", "tooltip": "178 messages today\nModel: opus-4-6"}
```

### CSS Styling

Place a `style.css` in the plugin directory. It's injected into the bar alongside theme CSS variables.

```css
/* ~/.config/nestty/plugins/my-plugin/style.css */
.clock {
    font-family: monospace;
    color: var(--nestty-subtext0);
}

.claude-usage {
    color: var(--nestty-accent);
    font-weight: bold;
}
```

Available CSS variables: `--nestty-bg`, `--nestty-fg`, `--nestty-surface0/1/2`, `--nestty-overlay0`, `--nestty-text`, `--nestty-subtext0/1`, `--nestty-accent`, `--nestty-red`.

### Environment Variables

Module scripts receive:

| Variable | Value |
|----------|-------|
| `NESTTY_SOCKET` | Path to nestty's Unix socket |
| `NESTTY_PLUGIN_DIR` | Absolute path to the plugin directory |

Scripts can use `nestctl` for nestty integration (CWD, tab info, etc.).

### Config

```toml
[statusbar]
enabled = true          # Show/hide the status bar (default: true)
position = "bottom"     # "top" or "bottom" (default: "bottom")
height = 28             # Height in pixels (default: 28)
```

### CLI

```bash
nestctl statusbar show
nestctl statusbar hide
nestctl statusbar toggle
```

### Socket API

| Command | Response |
|---------|----------|
| `statusbar.show` | `{visible: true}` |
| `statusbar.hide` | `{visible: false}` |
| `statusbar.toggle` | `{visible: bool}` |

### Architecture

The status bar is a single WebView that renders CSS-styled module containers. Rust periodically runs each module's `exec` command in a thread, then updates the corresponding DOM element via `evaluate_javascript()`. This gives full CSS styling power with lightweight shell-based data collection.

## Tips

- Plugin panels have `allow_file_access_from_file_urls` enabled, so you can load local CSS/JS/images with relative paths in your HTML
- DevTools are enabled — right-click and inspect to debug your plugin panel
- Use `nestty.on("*", console.log)` during development to see all events
- Plugin discovery happens at startup — restart nestty after adding a new plugin
- Commands run in a thread, so they won't block the UI even if slow
