//! Registered GUI clients and the GUI-owned method routing table.
//!
//! See `docs/gui-daemon-protocol.md` § `gui.register` schema + Routing rules.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nestty_core::protocol::{Invoke, Response, ResponseError};
use serde_json::Value;

/// Maps a GUI-owned method to its capability. `None` = daemon-owned.
pub fn method_capability(method: &str) -> Option<&'static str> {
    match method {
        "tab.new" | "tab.close" | "tab.list" | "tab.info" | "tab.rename" | "tabs.toggle_bar"
        | "claude.start" => Some("tab"),
        "split.horizontal" | "split.vertical" => Some("split"),
        "terminal.read" | "terminal.state" | "terminal.exec" | "terminal.feed"
        | "terminal.history" | "terminal.context" => Some("terminal"),
        m if m.starts_with("webview.") => Some("webview"),
        m if m.starts_with("background.") => Some("background"),
        "statusbar.show" | "statusbar.hide" | "statusbar.toggle" => Some("statusbar"),
        "agent.approve" => Some("agent.ui"),
        "plugin.open" => Some("plugin.open"),
        "session.list" | "session.info" => Some("session"),
        // `plugin.<name>.<cmd>` shell commands from a plugin manifest's
        // `[[commands]]`. GUI-routed because nestty-linux's existing
        // dispatch resolves them via `TabManager::plugins()`.
        m if m.starts_with("plugin.") && m.matches('.').count() == 2 => Some("plugin.open"),
        _ => None,
    }
}

/// Some GUI methods legitimately take more than the default 5s — slow
/// WebView ops in particular. The supervisor's action_timeout is 120s
/// upstream, so we match that for any method that can transitively trigger
/// a plugin RPC or a heavy WebView call.
pub fn method_invoke_timeout(method: &str) -> Duration {
    if method.starts_with("webview.")
        || method == "claude.start"
        || (method.starts_with("plugin.") && method.matches('.').count() == 2)
    {
        Duration::from_secs(120)
    } else {
        Duration::from_secs(5)
    }
}

pub struct GuiClient {
    pub client_id: String,
    pub capabilities: HashSet<String>,
    pub want_primary: bool,
    writer_tx: Sender<String>,
    /// `None` after `fail_all_pending` (= unregister) so a stale Arc held
    /// across a disconnect can't insert a new pending entry that nobody
    /// will ever resolve.
    pending: Mutex<Option<HashMap<String, Sender<Response>>>>,
}

impl GuiClient {
    /// Sends an Invoke and blocks until the GUI replies with a matching
    /// Response, or `timeout` elapses. Returns `gui_disconnected`
    /// immediately if the client has already been unregistered.
    pub fn invoke(&self, method: &str, params: Value, timeout: Duration) -> Response {
        let invoke_id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = channel::<Response>();
        {
            let mut guard = self.pending.lock().unwrap();
            match guard.as_mut() {
                Some(map) => {
                    map.insert(invoke_id.clone(), tx);
                }
                None => {
                    return Response::error(
                        String::new(),
                        "gui_disconnected",
                        "GUI disconnected before invoke",
                    );
                }
            }
        }
        let line = match serde_json::to_string(&Invoke::new(&invoke_id, method, params)) {
            Ok(s) => s,
            Err(e) => {
                self.remove_pending(&invoke_id);
                return Response::error(
                    String::new(),
                    "internal_error",
                    &format!("serialize invoke: {e}"),
                );
            }
        };
        if self.writer_tx.send(line).is_err() {
            self.remove_pending(&invoke_id);
            return Response::error(String::new(), "gui_disconnected", "GUI writer closed");
        }
        match rx.recv_timeout(timeout) {
            Ok(resp) => resp,
            Err(_) => {
                self.remove_pending(&invoke_id);
                Response::error(
                    String::new(),
                    "gui_timeout",
                    &format!("no GUI response within {:?}", timeout),
                )
            }
        }
    }

    fn remove_pending(&self, invoke_id: &str) {
        if let Some(map) = self.pending.lock().unwrap().as_mut() {
            map.remove(invoke_id);
        }
    }

    pub fn resolve(&self, response: Response) {
        let tx = self
            .pending
            .lock()
            .unwrap()
            .as_mut()
            .and_then(|m| m.remove(&response.id));
        if let Some(tx) = tx {
            let _ = tx.send(response);
        }
    }

    /// Marks the client disconnected (pending becomes `None`) and fails
    /// every currently-pending invoke with the given error. Subsequent
    /// `invoke` calls also fail fast with `gui_disconnected`.
    pub fn fail_all_pending(&self, err: ResponseError) {
        let drained = self.pending.lock().unwrap().take();
        if let Some(map) = drained {
            for (id, tx) in map {
                let _ = tx.send(Response {
                    id,
                    ok: false,
                    result: None,
                    error: Some(err.clone()),
                });
            }
        }
    }
}

#[derive(Default)]
pub struct GuiRegistry {
    clients: Mutex<HashMap<String, Arc<GuiClient>>>,
    /// Registration order, newest last. Primary promotion picks the most
    /// recent `want_primary=true` entry per spec.
    order: Mutex<Vec<String>>,
    primary: Mutex<Option<String>>,
}

impl GuiRegistry {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Returns `(client_id, is_primary)`. Always acquires locks in order
    /// `clients → order → primary` (see `unregister`/`route` for the same
    /// ordering — diverging would deadlock).
    pub fn register(
        self: &Arc<Self>,
        capabilities: HashSet<String>,
        want_primary: bool,
        writer_tx: Sender<String>,
    ) -> (String, bool) {
        let client_id = uuid::Uuid::new_v4().to_string();
        let client = Arc::new(GuiClient {
            client_id: client_id.clone(),
            capabilities,
            want_primary,
            writer_tx,
            pending: Mutex::new(Some(HashMap::new())),
        });
        let mut clients = self.clients.lock().unwrap();
        let mut order = self.order.lock().unwrap();
        let mut primary = self.primary.lock().unwrap();
        clients.insert(client_id.clone(), client);
        order.push(client_id.clone());
        let is_primary = if primary.is_none() && want_primary {
            *primary = Some(client_id.clone());
            true
        } else {
            false
        };
        (client_id, is_primary)
    }

    pub fn unregister(&self, client_id: &str) {
        // Lock order: clients → order → primary, same as `register`.
        let mut clients = self.clients.lock().unwrap();
        let mut order = self.order.lock().unwrap();
        let mut primary = self.primary.lock().unwrap();
        let removed = clients.remove(client_id);
        order.retain(|id| id != client_id);
        if primary.as_deref() == Some(client_id) {
            *primary = order
                .iter()
                .rev()
                .find(|id| clients.get(*id).map(|c| c.want_primary).unwrap_or(false))
                .cloned();
        }
        drop(primary);
        drop(order);
        drop(clients);
        if let Some(client) = removed {
            client.fail_all_pending(ResponseError {
                code: "gui_disconnected".into(),
                message: "GUI client unregistered".into(),
            });
        }
    }

    /// Lock order: clients → primary (same direction as `register`).
    pub fn route(
        &self,
        method: &str,
        target: Option<&str>,
    ) -> Result<Arc<GuiClient>, &'static str> {
        let Some(cap) = method_capability(method) else {
            return Err("not_gui_owned");
        };
        let clients = self.clients.lock().unwrap();
        let primary = self.primary.lock().unwrap().clone();
        let candidate = match target {
            Some(target_id) => clients.get(target_id).cloned().ok_or("unknown_client")?,
            None => {
                let primary_id = primary.ok_or("no_gui")?;
                clients.get(&primary_id).cloned().ok_or("no_gui")?
            }
        };
        if candidate.capabilities.contains(cap) {
            Ok(candidate)
        } else {
            Err("no_gui")
        }
    }

    pub fn get(&self, client_id: &str) -> Option<Arc<GuiClient>> {
        self.clients.lock().unwrap().get(client_id).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn mk_caps(names: &[&str]) -> HashSet<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn method_capability_maps_known_legacy_methods() {
        assert_eq!(method_capability("tab.list"), Some("tab"));
        assert_eq!(method_capability("webview.click"), Some("webview"));
        assert_eq!(method_capability("terminal.exec"), Some("terminal"));
        assert_eq!(method_capability("claude.start"), Some("tab"));
        assert_eq!(method_capability("system.ping"), None);
        assert_eq!(method_capability("kb.search"), None);
    }

    #[test]
    fn first_want_primary_becomes_primary() {
        let reg = GuiRegistry::new();
        let (tx, _rx) = channel::<String>();
        let (_, is_primary) = reg.register(mk_caps(&["tab"]), true, tx);
        assert!(is_primary);
    }

    #[test]
    fn want_primary_false_stays_secondary() {
        let reg = GuiRegistry::new();
        let (tx, _rx) = channel::<String>();
        let (_, is_primary) = reg.register(mk_caps(&["tab"]), false, tx);
        assert!(!is_primary);
        let (tx2, _rx2) = channel::<String>();
        let (_, is_primary2) = reg.register(mk_caps(&["tab"]), true, tx2);
        assert!(is_primary2);
    }

    #[test]
    fn second_register_with_want_primary_stays_secondary() {
        let reg = GuiRegistry::new();
        let (tx1, _rx1) = channel::<String>();
        let (_, p1) = reg.register(mk_caps(&["tab"]), true, tx1);
        let (tx2, _rx2) = channel::<String>();
        let (_, p2) = reg.register(mk_caps(&["tab"]), true, tx2);
        assert!(p1);
        assert!(!p2);
    }

    #[test]
    fn route_returns_primary_for_matching_capability() {
        let reg = GuiRegistry::new();
        let (tx, _rx) = channel::<String>();
        let (cid, _) = reg.register(mk_caps(&["tab", "split"]), true, tx);
        let client = reg.route("tab.list", None).expect("routed");
        assert_eq!(client.client_id, cid);
    }

    #[test]
    fn route_no_gui_when_no_primary() {
        let reg = GuiRegistry::new();
        assert_eq!(reg.route("tab.list", None).err(), Some("no_gui"));
    }

    #[test]
    fn route_no_gui_when_capability_missing() {
        let reg = GuiRegistry::new();
        let (tx, _rx) = channel::<String>();
        reg.register(mk_caps(&["split"]), true, tx); // no "tab" cap
        assert_eq!(reg.route("tab.list", None).err(), Some("no_gui"));
    }

    #[test]
    fn route_target_client_id_picks_specific() {
        let reg = GuiRegistry::new();
        let (tx_primary, _rx_p) = channel::<String>();
        let (_, _) = reg.register(mk_caps(&["tab"]), true, tx_primary);
        let (tx_secondary, _rx_s) = channel::<String>();
        let (secondary_id, _) = reg.register(mk_caps(&["tab"]), false, tx_secondary);
        let client = reg.route("tab.list", Some(&secondary_id)).expect("routed");
        assert_eq!(client.client_id, secondary_id);
    }

    #[test]
    fn route_target_unknown_returns_unknown_client() {
        let reg = GuiRegistry::new();
        assert_eq!(
            reg.route("tab.list", Some("nope")).err(),
            Some("unknown_client")
        );
    }

    #[test]
    fn route_non_gui_owned_method_returns_not_gui_owned() {
        let reg = GuiRegistry::new();
        let (tx, _rx) = channel::<String>();
        reg.register(mk_caps(&["tab"]), true, tx);
        assert_eq!(reg.route("system.ping", None).err(), Some("not_gui_owned"));
    }

    #[test]
    fn unregister_primary_promotes_most_recent_want_primary() {
        let reg = GuiRegistry::new();
        let (tx1, _rx1) = channel::<String>();
        let (id1, _) = reg.register(mk_caps(&["tab"]), true, tx1);
        let (tx2, _rx2) = channel::<String>();
        let (id2, _) = reg.register(mk_caps(&["tab"]), true, tx2);
        let (tx3, _rx3) = channel::<String>();
        let (id3, _) = reg.register(mk_caps(&["tab"]), true, tx3);
        // Drop the original primary (id1). Most-recent (id3) should win,
        // not id2 (the second-oldest).
        reg.unregister(&id1);
        let routed = reg.route("tab.list", None).expect("primary transferred");
        assert_eq!(routed.client_id, id3);
        // Drop the new primary too — id2 should become primary now.
        reg.unregister(&id3);
        let routed = reg
            .route("tab.list", None)
            .expect("primary transferred again");
        assert_eq!(routed.client_id, id2);
    }

    #[test]
    fn invoke_timeout_returns_gui_timeout_error() {
        let reg = GuiRegistry::new();
        let (writer_tx, _writer_rx) = channel();
        let (_, _) = reg.register(mk_caps(&["tab"]), true, writer_tx);
        let client = reg.route("tab.list", None).unwrap();
        let resp = client.invoke("tab.list", json!({}), Duration::from_millis(50));
        assert!(!resp.ok);
        assert_eq!(resp.error.unwrap().code, "gui_timeout");
    }

    #[test]
    fn invoke_after_unregister_returns_disconnect_fast() {
        // Race we're closing: route() hands out an Arc<GuiClient>, then
        // unregister fires, then the caller still tries to invoke. Must
        // surface gui_disconnected, not wait for the full timeout.
        let reg = GuiRegistry::new();
        let (writer_tx, _writer_rx) = channel::<String>();
        let (cid, _) = reg.register(mk_caps(&["tab"]), true, writer_tx);
        let client = reg.get(&cid).unwrap();
        reg.unregister(&cid);
        let start = std::time::Instant::now();
        let resp = client.invoke("tab.list", json!({}), Duration::from_secs(5));
        assert!(!resp.ok);
        assert_eq!(resp.error.unwrap().code, "gui_disconnected");
        assert!(
            start.elapsed() < Duration::from_millis(50),
            "invoke after disconnect must return immediately, took {:?}",
            start.elapsed()
        );
    }

    #[test]
    fn unregister_fails_pending_invokes_with_disconnect() {
        let reg = GuiRegistry::new();
        let (writer_tx, _writer_rx) = channel();
        let (cid, _) = reg.register(mk_caps(&["tab"]), true, writer_tx);
        let client = reg.get(&cid).unwrap();
        // Issue a pending Invoke from a worker, unregister, expect it to
        // surface gui_disconnected.
        let client_clone = client.clone();
        let handle = std::thread::spawn(move || {
            client_clone.invoke("tab.list", json!({}), Duration::from_secs(5))
        });
        // Brief wait so the pending entry exists before we unregister.
        std::thread::sleep(Duration::from_millis(30));
        reg.unregister(&cid);
        let resp = handle.join().unwrap();
        assert!(!resp.ok);
        assert_eq!(resp.error.unwrap().code, "gui_disconnected");
    }
}
