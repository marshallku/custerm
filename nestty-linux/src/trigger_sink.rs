use std::sync::Arc;
use std::sync::mpsc;

use nestty_core::action_registry::{ActionRegistry, ActionResult, internal_error};
use nestty_core::protocol::{Request, Response};
use nestty_core::trigger::TriggerSink;
use serde_json::{Value, json};

use crate::socket::SocketCommand;

/// `TriggerSink` impl that tries the in-process `ActionRegistry` first,
/// then falls through to `socket::dispatch` (via the same channel that
/// plugins use) for actions still living in the legacy match arm. This is
/// what makes legacy commands like `tab.new`, `terminal.exec`, `webview.*`
/// reachable from triggers without migrating each one into the registry.
///
/// **Async error visibility:**
/// - Sync registered handlers: errors come back through the
///   `try_dispatch` callback synchronously (the callback fires inline
///   before `try_dispatch` returns), so they're logged the same tick
///   the trigger fires — no observable latency vs the old
///   sync-return-value flow.
/// - Blocking registered handlers (every service-plugin action): the
///   registry spawns a worker thread, the callback fires from the
///   worker after the handler returns, and any error is logged from
///   that thread. The trigger pump reports the action as queued
///   immediately (`fired += 1`); failures surface in the log shortly
///   after.
/// - Legacy fallthrough (`socket::dispatch`): same model as before —
///   queued via `dispatch_tx`, replies drained by a dedicated
///   consumer thread that logs `ok=false` responses.
///
/// All three paths log via `eprintln!` with a `[nestty] trigger ...`
/// prefix so a misconfigured trigger is visible regardless of which
/// path handled it.
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
                    "[nestty] trigger fallthrough id={} failed: {}: {}",
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
        if self.registry.has(action) {
            // Branch on blocking flag so we preserve the prior
            // synchronous error semantics for sync registry actions.
            // The TriggerEngine increments `fired` only on `Ok` and
            // log::warn's on `Err`; collapsing every registry call to
            // `Ok(queued)` would silently re-classify sync failures
            // as successful queueing.
            if self.registry.is_blocking(action) {
                // Worker-thread path: callback fires from worker after
                // we've returned `Ok(queued)` to the engine, so the
                // engine never sees the underlying error — log here
                // directly so misconfigured blocking actions stay
                // visible.
                let action_owned = action.to_string();
                self.registry.try_dispatch(
                    action,
                    params,
                    Box::new(move |result| {
                        if let Err(err) = result {
                            eprintln!(
                                "[nestty] trigger registry id={} (blocking) failed: {}: {}",
                                action_owned, err.code, err.message
                            );
                        }
                    }),
                );
                return Ok(json!({ "queued": true }));
            }
            // Sync path: invoke inline and propagate the actual
            // ActionResult so the engine can log Err / increment
            // `fired` only on Ok, matching the pre-Phase-9.4 contract.
            // `try_invoke` runs inline regardless of flag, but we've
            // already guarded against `is_blocking == true` above.
            return self
                .registry
                .try_invoke(action, params)
                .expect("registry.has() just returned true");
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

#[cfg(test)]
mod tests {
    use super::*;
    use nestty_core::action_registry::invalid_params;

    fn mk_sink_with_registry() -> (
        Arc<ActionRegistry>,
        LiveTriggerSink,
        mpsc::Receiver<SocketCommand>,
    ) {
        let registry = Arc::new(ActionRegistry::new());
        let (tx, rx) = mpsc::channel::<SocketCommand>();
        let sink = LiveTriggerSink::new(registry.clone(), tx);
        (registry, sink, rx)
    }

    #[test]
    fn sync_registry_action_returns_actual_result_not_queued() {
        let (registry, sink, _rx) = mk_sink_with_registry();
        registry.register("sync.ok", |_| Ok(json!("real-value")));
        let r = sink.dispatch_action("sync.ok", json!({})).unwrap();
        assert_eq!(r, json!("real-value"));
    }

    #[test]
    fn sync_registry_action_propagates_err_so_engine_logs_it() {
        let (registry, sink, _rx) = mk_sink_with_registry();
        registry.register("sync.fail", |_| Err(invalid_params("bad")));
        let err = sink.dispatch_action("sync.fail", json!({})).unwrap_err();
        assert_eq!(err.code, "invalid_params");
        assert_eq!(err.message, "bad");
    }

    #[test]
    fn blocking_registry_action_returns_queued_immediately() {
        let (registry, sink, _rx) = mk_sink_with_registry();
        registry.register_blocking("slow.ok", |_| {
            std::thread::sleep(std::time::Duration::from_millis(50));
            Ok(json!("eventual"))
        });
        let start = std::time::Instant::now();
        let r = sink.dispatch_action("slow.ok", json!({})).unwrap();
        assert!(
            start.elapsed() < std::time::Duration::from_millis(40),
            "dispatch_action returned in {:?}, expected <40ms",
            start.elapsed()
        );
        assert_eq!(r, json!({"queued": true}));
    }

    #[test]
    fn unknown_action_falls_through_to_socket_dispatch() {
        let (_registry, sink, rx) = mk_sink_with_registry();
        let r = sink
            .dispatch_action("legacy.thing", json!({"x": 1}))
            .unwrap();
        assert_eq!(r, json!({"queued": true}));
        // The fallthrough must have queued one SocketCommand on the
        // dispatch channel.
        let cmd = rx.try_recv().expect("expected one queued legacy command");
        assert_eq!(cmd.request.method, "legacy.thing");
        assert_eq!(cmd.request.params, json!({"x": 1}));
    }
}
