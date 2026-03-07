# CLI Tool (custerm-cli)

Binary name: `custermctl`

## Usage

```bash
custermctl [--socket <path>] [--json] <command>
```

- `--socket` — override socket path (default: `$CUSTERM_SOCKET` or `/tmp/custerm-{PID}.sock`)
- `--json` — output in JSON format

## Commands

### System
- `custermctl ping` — ping running instance

### Session
- `custermctl session list` — list all panels across all tabs
- `custermctl session info <id>` — detailed info for a panel

### Background
- `custermctl background set <path>` — set background image (path is canonicalized)
- `custermctl background clear` — clear background image
- `custermctl background set-tint <opacity>` — set tint opacity (0.0–1.0)
- `custermctl background next` — switch to next random background
- `custermctl background toggle` — toggle background visibility

### Tab
- `custermctl tab new` — create a new tab
- `custermctl tab close` — close the focused tab/panel
- `custermctl tab list` — list tabs
- `custermctl tab info` — extended tab info with panel counts

### Split
- `custermctl split horizontal` — split focused pane horizontally
- `custermctl split vertical` — split focused pane vertically

### Event Stream
- `custermctl event subscribe` — subscribe to terminal events (streams JSON lines to stdout)

### WebView
- `custermctl webview open <url> [--mode tab|split_h|split_v]` — open URL in new webview panel
- `custermctl webview navigate --id <id> <url>` — navigate existing webview
- `custermctl webview back --id <id>` — go back in history
- `custermctl webview forward --id <id>` — go forward in history
- `custermctl webview reload --id <id>` — reload page
- `custermctl webview exec-js --id <id> <code>` — execute JavaScript, return result
- `custermctl webview get-content --id <id> [--format text|html]` — get page content
- `custermctl webview screenshot --id <id> [--path <file>]` — screenshot (base64 PNG or save to file)
- `custermctl webview query --id <id> <selector>` — query single DOM element
- `custermctl webview query-all --id <id> <selector> [--limit 50]` — query all matching elements
- `custermctl webview get-styles --id <id> <selector> <properties>` — get computed CSS styles
- `custermctl webview click --id <id> <selector>` — click a DOM element
- `custermctl webview fill --id <id> <selector> <value>` — type text into an input
- `custermctl webview scroll --id <id> [--selector <sel>] [--x 0] [--y 0]` — scroll to position or element
- `custermctl webview page-info --id <id>` — get page metadata (title, dimensions, element counts)

## Protocol

Uses cmux V2 newline-delimited JSON over Unix domain socket.

Request:
```json
{"id": "<uuid>", "method": "background.next", "params": {}}
```

Response:
```json
{"id": "<uuid>", "ok": true, "result": {...}}
```

## Socket Client (`client.rs`)

- Connects to Unix socket
- 15s read timeout, 5s write timeout
- Sends JSON request, reads matching response by ID
- Matches request/response by UUID
