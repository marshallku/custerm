# CLI Tool (custerm-cli)

Binary name: `custermctl`

## Usage

```bash
custermctl [--socket <path>] [--json] <command>
```

- `--socket` — override socket path (default: `$CUSTERM_SOCKET` or `/tmp/custerm.sock`)
- `--json` — output in JSON format

## Commands

### System
- `custermctl ping` — ping running instance

### Window Management
- `custermctl window list`
- `custermctl window new`
- `custermctl window focus <id>`
- `custermctl window close [id]`

### Workspace Management
- `custermctl workspace list`
- `custermctl workspace new [--name <name>]`
- `custermctl workspace select <id>`
- `custermctl workspace close [id]`
- `custermctl workspace rename <id> <name>`

### Session Management
- `custermctl session list`
- `custermctl session send --id <id> <text>`
- `custermctl session read [--id <id>] [--lines <n>]`
- `custermctl session close <id>`

### Background
- `custermctl background set <path>`
- `custermctl background next`
- `custermctl background toggle`

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

## Current Status

The CLI commands map to methods but the socket server in custerm-linux is **not yet implemented**. Currently, background control uses D-Bus directly (see [linux-app.md](./linux-app.md)). The socket server is planned for Phase 4.
