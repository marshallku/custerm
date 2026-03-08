# Plugin Development Guide

## Overview

turm plugins extend the terminal with custom panels (HTML/JS UIs) and commands (shell scripts). Plugins live in `~/.config/turm/plugins/` and are discovered automatically at startup.

## Plugin Structure

```
~/.config/turm/plugins/my-plugin/
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

Open a specific panel: `turmctl plugin open my-plugin --panel settings`

## JS Bridge API

Every plugin panel gets a `window.turm` object injected automatically.

### `turm.panel`

Info about the current panel:

```javascript
turm.panel.id       // UUID of this panel instance
turm.panel.name     // Panel name from manifest (e.g., "main")
turm.panel.plugin   // Plugin name (e.g., "my-plugin")
```

### `turm.call(method, params?)`

Call any turm socket API method. Returns a Promise.

```javascript
// Get terminal state
const state = await turm.call("terminal.state");
console.log(state.cwd, state.cols, state.rows);

// Read terminal screen
const screen = await turm.call("terminal.read");
console.log(screen.text);

// Execute a command in the terminal
await turm.call("terminal.exec", { command: "ls -la" });

// List all panels
const panels = await turm.call("session.list");

// Open a webview
const { panel_id } = await turm.call("webview.open", { url: "https://example.com" });

// Create a new terminal tab
await turm.call("tab.new");

// List themes
const { themes, current } = await turm.call("theme.list");

// Run a plugin command
const result = await turm.call("plugin.my-plugin.greet", { name: "world" });
```

All [socket API methods](./architecture.md#socket-server-ipc) are available.

### `turm.on(type, callback)` / `turm.off(type, callback)`

Listen for turm events:

```javascript
// Listen for focus changes
turm.on("panel.focused", (data) => {
    console.log("Panel focused:", data.panel_id);
});

// Listen for terminal output
turm.on("terminal.output", (data) => {
    console.log("Output:", data.text);
});

// Wildcard: listen for all events
turm.on("*", (type, data) => {
    console.log(`Event: ${type}`, data);
});

// Remove a listener
const handler = (data) => { ... };
turm.on("panel.focused", handler);
turm.off("panel.focused", handler);
```

All [event types](./architecture.md#event-stream) are available.

## Theme CSS Variables

Plugin panels automatically receive CSS variables matching the active turm theme:

```css
:root {
    --turm-bg: #1e1e2e;
    --turm-fg: #cdd6f4;
    --turm-surface0: #313244;
    --turm-surface1: #45475a;
    --turm-surface2: #585b70;
    --turm-overlay0: #6c7086;
    --turm-text: #cdd6f4;
    --turm-subtext0: #a6adc8;
    --turm-subtext1: #bac2de;
    --turm-accent: #89b4fa;
    --turm-red: #f38ba8;
}
```

Use these in your CSS to match the terminal theme:

```css
.card {
    background: var(--turm-surface0);
    border: 1px solid var(--turm-overlay0);
    color: var(--turm-text);
    border-radius: 8px;
    padding: 16px;
}

button {
    background: var(--turm-accent);
    color: var(--turm-bg);
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
| `TURM_SOCKET` | Path to turm's Unix socket |
| `TURM_PLUGIN_DIR` | Absolute path to the plugin directory |

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
turmctl plugin run my-plugin.greet --params '{"name": "turm"}'
```

From a plugin panel's JS:
```javascript
const result = await turm.call("plugin.my-plugin.greet", { name: "turm" });
console.log(result.message); // "Hello, turm!"
```

### Calling turm from Command Scripts

Commands can call back into turm via the socket:

```bash
#!/bin/bash
# Use turmctl with the injected socket path
export TURM_SOCKET="$TURM_SOCKET"
turmctl terminal exec "echo 'hello from plugin'"
```

## CLI

```bash
# List installed plugins
turmctl plugin list

# Open a plugin panel (default panel: "main")
turmctl plugin open my-plugin
turmctl plugin open my-plugin --panel settings

# Run a plugin command
turmctl plugin run my-plugin.greet --params '{"name": "world"}'
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

## Tips

- Plugin panels have `allow_file_access_from_file_urls` enabled, so you can load local CSS/JS/images with relative paths in your HTML
- DevTools are enabled — right-click and inspect to debug your plugin panel
- Use `turm.on("*", console.log)` during development to see all events
- Plugin discovery happens at startup — restart turm after adding a new plugin
- Commands run in a thread, so they won't block the UI even if slow
