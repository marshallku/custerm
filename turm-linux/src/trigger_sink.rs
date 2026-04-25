use std::sync::Arc;
use std::sync::mpsc;

use serde_json::{Value, json};
use turm_core::action_registry::{ActionRegistry, ActionResult, internal_error};
use turm_core::protocol::{Request, Response};
use turm_core::trigger::TriggerSink;

use crate::socket::SocketCommand;

/// `TriggerSink` impl that tries the in-process `ActionRegistry` first,
/// then falls through to `socket::dispatch` (via the same channel that
/// plugins use) for actions still living in the legacy match arm. This is
/// what makes legacy commands like `tab.new`, `terminal.exec`, `webview.*`
/// reachable from triggers without migrating each one into the registry.
///
/// **Async error visibility:** fallthrough cannot block for the dispatch
/// reply (this method runs on the GTK main thread, which is also the
/// thread that processes the queued command — blocking would deadlock).
/// Instead, every fallthrough call hands `socket::dispatch` a clone of a
/// shared reply channel that's drained by a dedicated consumer thread,
/// which logs `ok=false` responses (typos, unknown methods, runtime
/// errors). The trigger engine's per-event `fired` count still
/// over-counts on fallthrough — it counts queueing as success — but
/// misconfigured trigger actions are no longer silently invisible.
/// Registry actions retain full SYNCHRONOUS error semantics; only
/// fallthrough is async-logged.
pub struct LiveTriggerSink {
    registry: Arc<ActionRegistry>,
    dispatch_tx: mpsc::Sender<SocketCommand>,
    reply_tx: mpsc::Sender<Response>,
}

impl LiveTriggerSink {
    pub fn new(registry: Arc<ActionRegistry>, dispatch_tx: mpsc::Sender<SocketCommand>) -> Self {
        let (reply_tx, reply_rx) = mpsc::channel::<Response>();
        // Consumer thread: logs any fallthrough reply that came back with
        // ok=false. Lives until all `reply_tx` clones drop (i.e. the sink is
        // gone AND every queued SocketCommand has been processed).
        std::thread::spawn(move || {
            while let Ok(resp) = reply_rx.recv() {
                if resp.ok {
                    continue;
                }
                let (code, msg) = resp
                    .error
                    .map(|e| (e.code, e.message))
                    .unwrap_or_else(|| ("unknown".into(), String::new()));
                eprintln!(
                    "[turm] trigger fallthrough id={} failed: {}: {}",
                    resp.id, code, msg
                );
            }
        });
        Self {
            registry,
            dispatch_tx,
            reply_tx,
        }
    }
}

impl TriggerSink for LiveTriggerSink {
    fn dispatch_action(&self, action: &str, params: Value) -> ActionResult {
        // Registry first: full sync error semantics for migrated actions.
        if let Some(result) = self.registry.try_invoke(action, params.clone()) {
            return result;
        }
        // Fall through to legacy `socket::dispatch`. The reply channel is
        // shared with the consumer thread spawned in `new()` — that thread
        // surfaces any non-ok response to logs.
        let cmd = SocketCommand {
            request: Request::new(
                format!("trg-{}", uuid::Uuid::new_v4()),
                action.to_string(),
                params,
            ),
            reply: self.reply_tx.clone(),
        };
        self.dispatch_tx
            .send(cmd)
            .map_err(|e| internal_error(format!("trigger redispatch failed: {e}")))?;
        Ok(json!({ "queued": true }))
    }
}
