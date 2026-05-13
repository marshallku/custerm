//! Daemon-client mode for nestty-linux: connects to `nesttyd`, advertises
//! GUI capabilities via `gui.register`, and forwards inbound `Invoke`
//! requests through the existing dispatch pump.
//!
//! Off by default. Enable with `NESTTY_DAEMON_CLIENT=1`.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Sender, channel};
use std::thread;
use std::time::Duration;

use nestty_core::protocol::{Invoke, Request, Response};
use serde_json::Value;

use crate::socket::SocketCommand;

const PROTOCOL_VERSION: u32 = nestty_core::protocol::PROTOCOL_VERSION;

const CAPABILITIES: &[&str] = &[
    "tab",
    "split",
    "terminal",
    "webview",
    "background",
    "statusbar",
    "agent.ui",
    "plugin.open",
    "session",
    "search",
];

/// Connection-and-loop thread. Returns on disconnect; caller can re-spawn
/// to reconnect (step 4a does not auto-reconnect).
pub fn spawn(dispatch_tx: Sender<SocketCommand>) {
    thread::Builder::new()
        .name("nestty-gui-client".into())
        .spawn(move || {
            // Honors NESTTY_SOCKET — matches what nestctl + nesttyd use,
            // so an override aligns the whole stack.
            let socket_path = nestty_core::paths::socket_path();
            match run(&socket_path.to_string_lossy(), dispatch_tx) {
                Ok(()) => eprintln!("[nestty] gui_client disconnected from daemon"),
                Err(e) => eprintln!("[nestty] gui_client error: {e}"),
            }
        })
        .expect("spawn nestty-gui-client");
}

fn run(socket_path: &str, dispatch_tx: Sender<SocketCommand>) -> Result<(), String> {
    let stream = UnixStream::connect(socket_path)
        .map_err(|e| format!("connect to nesttyd at {socket_path}: {e}"))?;
    let write_stream = stream
        .try_clone()
        .map_err(|e| format!("clone stream: {e}"))?;

    let (writer_tx, writer_rx) = channel::<String>();
    thread::spawn(move || {
        let mut writer = write_stream;
        while let Ok(line) = writer_rx.recv() {
            if writeln!(writer, "{line}").is_err() {
                return;
            }
        }
    });

    let mut reader = BufReader::new(stream);
    let register_id = register(&writer_tx)?;
    await_register_ack(&mut reader, &register_id)?;

    for line in reader.lines() {
        let line = line.map_err(|e| format!("read: {e}"))?;
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                log::debug!("[nestty] gui_client malformed line: {e}");
                continue;
            }
        };

        if value.get("invoke").is_some() {
            handle_invoke(value, &dispatch_tx, &writer_tx)?;
        } else if value.get("ok").is_some() {
            // Response to our gui.register or a heartbeat reply later.
            log::debug!("[nestty] gui_client response: {value}");
        } else if value.get("type").is_some() {
            // Auto-subscribed Event stream — not consumed yet.
            log::trace!("[nestty] gui_client event: {value}");
        } else {
            log::debug!("[nestty] gui_client ignoring: {line:.200}");
        }
    }
    Ok(())
}

fn register(writer_tx: &Sender<String>) -> Result<String, String> {
    let window_id = uuid::Uuid::new_v4().to_string();
    let req_id = uuid::Uuid::new_v4().to_string();
    let req = Request::new(
        &req_id,
        "gui.register",
        serde_json::json!({
            "window_id": window_id,
            "capabilities": CAPABILITIES,
            "want_primary": true,
            "version": env!("CARGO_PKG_VERSION"),
            "protocol_version": PROTOCOL_VERSION,
        }),
    );
    let line = serde_json::to_string(&req).map_err(|e| format!("serialize register: {e}"))?;
    writer_tx
        .send(line)
        .map_err(|_| "writer thread exited before register".to_string())?;
    Ok(req_id)
}

fn await_register_ack(reader: &mut BufReader<UnixStream>, register_id: &str) -> Result<(), String> {
    let mut line = String::new();
    if reader
        .read_line(&mut line)
        .map_err(|e| format!("read register ack: {e}"))?
        == 0
    {
        return Err("daemon closed connection before register ack".into());
    }
    let resp: Response = serde_json::from_str(line.trim())
        .map_err(|e| format!("parse register ack: {e} (line={line:.200})"))?;
    if resp.id != register_id {
        return Err(format!(
            "register ack id mismatch: expected {register_id}, got {}",
            resp.id
        ));
    }
    if !resp.ok {
        let err = resp.error.unwrap_or(nestty_core::protocol::ResponseError {
            code: "unknown".into(),
            message: String::new(),
        });
        return Err(format!("register rejected: {} {}", err.code, err.message));
    }
    log::info!(
        "[nestty] gui_client registered with nesttyd: {}",
        resp.result.unwrap_or_default()
    );
    Ok(())
}

fn handle_invoke(
    value: Value,
    dispatch_tx: &Sender<SocketCommand>,
    writer_tx: &Sender<String>,
) -> Result<(), String> {
    let inv: Invoke = serde_json::from_value(value).map_err(|e| format!("parse Invoke: {e}"))?;
    let (reply_tx, reply_rx) = channel::<Response>();
    let cmd = SocketCommand {
        request: Request::new(inv.id.clone(), &inv.invoke, inv.params),
        reply: reply_tx,
    };
    if dispatch_tx.send(cmd).is_err() {
        return Err("GTK dispatch channel closed".into());
    }
    // Keep this strictly larger than the daemon's max per-method timeout
    // (currently 120s) so the daemon's outer timeout fires first — this
    // is just the safety net for a wedged GTK pump.
    let resp = match reply_rx.recv_timeout(Duration::from_secs(125)) {
        Ok(r) => r,
        Err(_) => Response::error(
            inv.id.clone(),
            "gui_internal_timeout",
            "GTK pump did not reply",
        ),
    };
    let encoded = serde_json::to_string(&resp).map_err(|e| format!("serialize response: {e}"))?;
    writer_tx
        .send(encoded)
        .map_err(|_| "writer thread closed".to_string())
}
