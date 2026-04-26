use crate::protocol::ResponseError;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

pub type ActionResult = Result<Value, ResponseError>;
pub type ActionFn = Arc<dyn Fn(Value) -> ActionResult + Send + Sync + 'static>;
/// Continuation passed to `try_dispatch`. Called exactly once per
/// successful dispatch. For sync handlers it fires *inline* on the
/// caller's thread before `try_dispatch` returns; for blocking
/// handlers it fires on a worker thread spawned by the registry.
pub type Responder = Box<dyn FnOnce(ActionResult) + Send + 'static>;

struct Entry {
    handler: ActionFn,
    /// True if the handler may block for a non-trivial duration
    /// (network I/O, subprocess RPC, etc.). `try_dispatch` runs
    /// blocking handlers on a worker thread so the caller's thread —
    /// typically the GTK timer in `turm-linux` — never stalls.
    blocking: bool,
}

pub struct ActionRegistry {
    entries: RwLock<HashMap<String, Entry>>,
}

impl ActionRegistry {
    pub fn new() -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
        }
    }

    /// Register a synchronous handler. `try_dispatch` will run it
    /// inline on the caller's thread; use this only for handlers
    /// that return in microseconds (in-memory lookups, registry
    /// queries, etc.).
    pub fn register<F>(&self, name: impl Into<String>, handler: F)
    where
        F: Fn(Value) -> ActionResult + Send + Sync + 'static,
    {
        self.entries.write().unwrap().insert(
            name.into(),
            Entry {
                handler: Arc::new(handler),
                blocking: false,
            },
        );
    }

    /// Register a handler that may block (network I/O, plugin RPC,
    /// LLM completion, etc.). `try_dispatch` will spawn a worker
    /// thread to run it so callers on time-sensitive threads (the
    /// GTK main loop, the trigger pump, the socket dispatcher) keep
    /// flowing. Same handler shape as `register` — the only
    /// difference is the dispatch-time treatment.
    pub fn register_blocking<F>(&self, name: impl Into<String>, handler: F)
    where
        F: Fn(Value) -> ActionResult + Send + Sync + 'static,
    {
        self.entries.write().unwrap().insert(
            name.into(),
            Entry {
                handler: Arc::new(handler),
                blocking: true,
            },
        );
    }

    pub fn invoke(&self, name: &str, params: Value) -> ActionResult {
        // Clone out the handler Arc under the read lock, then drop the guard
        // before running the handler. This keeps handler execution off the
        // lock entirely so a handler may freely call `register`, `invoke`,
        // or other registry methods without risking deadlock.
        let handler = {
            let entries = self.entries.read().unwrap();
            entries
                .get(name)
                .map(|e| e.handler.clone())
                .ok_or_else(|| not_found_error(name))?
        };
        handler(params)
    }

    /// Like `invoke`, but returns `None` if the action is not registered
    /// (rather than synthesizing an `action_not_found` error). Useful for
    /// dispatchers that want to fall through to a different handler when
    /// the registry has no entry for the method.
    ///
    /// Runs the handler INLINE regardless of `blocking` flag. Callers on
    /// time-sensitive threads (GTK, socket dispatch) should use
    /// `try_dispatch` instead so blocking handlers spawn a worker.
    pub fn try_invoke(&self, name: &str, params: Value) -> Option<ActionResult> {
        let handler = self.entries.read().unwrap().get(name).map(|e| e.handler.clone())?;
        Some(handler(params))
    }

    /// Dispatch an action with a continuation. Returns `true` if the
    /// action was found (and `on_done` will be — or already has been
    /// — called exactly once); `false` if not registered (caller
    /// should fall through; `on_done` is dropped uncalled).
    ///
    /// Behavior split:
    /// - **Sync handler** (registered via `register`): handler runs
    ///   inline on the caller's thread; `on_done` fires synchronously
    ///   before this method returns.
    /// - **Blocking handler** (registered via `register_blocking`): a
    ///   worker thread is spawned; `on_done` fires from the worker
    ///   after the handler completes.
    ///
    /// The unified callback API means callers don't need to branch
    /// on sync vs blocking — register a single completion closure,
    /// trust it'll fire once. For sync handlers this carries no
    /// extra cost (no thread spawn). For blocking handlers it
    /// keeps the caller's thread alive while the work proceeds in
    /// the background.
    pub fn try_dispatch(
        self: &Arc<Self>,
        name: &str,
        params: Value,
        on_done: Responder,
    ) -> bool {
        let (handler, blocking) = {
            let entries = self.entries.read().unwrap();
            match entries.get(name) {
                Some(e) => (e.handler.clone(), e.blocking),
                None => return false,
            }
        };
        if blocking {
            // Spawn a worker. The handler's Arc keeps it alive
            // independent of the registry's HashMap, so a concurrent
            // `register` overwrite can't pull the handler out from
            // under the worker.
            std::thread::spawn(move || {
                let result = handler(params);
                on_done(result);
            });
        } else {
            let result = handler(params);
            on_done(result);
        }
        true
    }

    pub fn has(&self, name: &str) -> bool {
        self.entries.read().unwrap().contains_key(name)
    }

    /// True if the named action is registered AND marked blocking.
    /// Useful for diagnostics; not load-bearing for dispatch
    /// (`try_dispatch` already routes correctly).
    pub fn is_blocking(&self, name: &str) -> bool {
        self.entries
            .read()
            .unwrap()
            .get(name)
            .map(|e| e.blocking)
            .unwrap_or(false)
    }

    pub fn names(&self) -> Vec<String> {
        let mut out: Vec<String> = self.entries.read().unwrap().keys().cloned().collect();
        out.sort();
        out
    }

    pub fn len(&self) -> usize {
        self.entries.read().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.read().unwrap().is_empty()
    }
}

impl Default for ActionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

fn not_found_error(name: &str) -> ResponseError {
    ResponseError {
        code: "action_not_found".into(),
        message: format!("no action registered: {name}"),
    }
}

pub fn invalid_params(message: impl Into<String>) -> ResponseError {
    ResponseError {
        code: "invalid_params".into(),
        message: message.into(),
    }
}

pub fn internal_error(message: impl Into<String>) -> ResponseError {
    ResponseError {
        code: "internal_error".into(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    #[test]
    fn register_and_invoke() {
        let reg = ActionRegistry::new();
        reg.register("ping", |_| Ok(json!({"pong": true})));
        let out = reg.invoke("ping", json!({})).unwrap();
        assert_eq!(out, json!({"pong": true}));
    }

    #[test]
    fn try_invoke_returns_none_for_unknown() {
        let reg = ActionRegistry::new();
        reg.register("known", |_| Ok(json!("ok")));
        assert!(reg.try_invoke("missing", json!({})).is_none());
        let some = reg.try_invoke("known", json!({})).expect("registered");
        assert_eq!(some.unwrap(), json!("ok"));
    }

    #[test]
    fn unknown_action_returns_not_found() {
        let reg = ActionRegistry::new();
        let err = reg.invoke("missing", json!({})).unwrap_err();
        assert_eq!(err.code, "action_not_found");
        assert!(err.message.contains("missing"));
    }

    #[test]
    fn handler_error_propagates() {
        let reg = ActionRegistry::new();
        reg.register("fail", |_| Err(invalid_params("bad input")));
        let err = reg.invoke("fail", json!({})).unwrap_err();
        assert_eq!(err.code, "invalid_params");
        assert_eq!(err.message, "bad input");
    }

    #[test]
    fn params_are_passed_through() {
        let reg = ActionRegistry::new();
        reg.register("echo", Ok);
        let out = reg.invoke("echo", json!({"a": 1, "b": [2, 3]})).unwrap();
        assert_eq!(out, json!({"a": 1, "b": [2, 3]}));
    }

    #[test]
    fn has_and_names_reflect_registrations() {
        let reg = ActionRegistry::new();
        assert!(reg.is_empty());
        reg.register("b.thing", |_| Ok(json!(null)));
        reg.register("a.thing", |_| Ok(json!(null)));
        assert!(reg.has("a.thing"));
        assert!(reg.has("b.thing"));
        assert!(!reg.has("c.thing"));
        assert_eq!(reg.len(), 2);
        assert_eq!(reg.names(), vec!["a.thing".to_string(), "b.thing".to_string()]);
    }

    #[test]
    fn handler_can_capture_state() {
        let reg = ActionRegistry::new();
        let counter = Arc::new(AtomicUsize::new(0));
        let c = counter.clone();
        reg.register("bump", move |_| {
            let prev = c.fetch_add(1, Ordering::SeqCst);
            Ok(json!({"prev": prev}))
        });
        reg.invoke("bump", json!({})).unwrap();
        reg.invoke("bump", json!({})).unwrap();
        let out = reg.invoke("bump", json!({})).unwrap();
        assert_eq!(out, json!({"prev": 2}));
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn register_overwrites_existing() {
        let reg = ActionRegistry::new();
        reg.register("x", |_| Ok(json!("old")));
        reg.register("x", |_| Ok(json!("new")));
        assert_eq!(reg.invoke("x", json!({})).unwrap(), json!("new"));
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn concurrent_invoke_from_multiple_threads() {
        let reg = Arc::new(ActionRegistry::new());
        let shared = Arc::new(Mutex::new(0u64));
        {
            let s = shared.clone();
            reg.register("add", move |params| {
                let n = params.as_u64().ok_or_else(|| invalid_params("expected u64"))?;
                let mut g = s.lock().unwrap();
                *g += n;
                Ok(json!(*g))
            });
        }
        let mut handles = vec![];
        for _ in 0..8 {
            let r = reg.clone();
            handles.push(std::thread::spawn(move || {
                for _ in 0..100 {
                    r.invoke("add", json!(1u64)).unwrap();
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(*shared.lock().unwrap(), 8 * 100);
    }

    #[test]
    fn handler_can_register_more_actions_without_deadlock() {
        let reg = Arc::new(ActionRegistry::new());
        let r = reg.clone();
        reg.register("self_extend", move |_| {
            r.register("added_later", |_| Ok(json!("ok")));
            Ok(json!("extended"))
        });
        assert_eq!(
            reg.invoke("self_extend", json!({})).unwrap(),
            json!("extended")
        );
        assert!(reg.has("added_later"));
        assert_eq!(
            reg.invoke("added_later", json!({})).unwrap(),
            json!("ok")
        );
    }

    #[test]
    fn handler_can_invoke_another_action_without_deadlock() {
        let reg = Arc::new(ActionRegistry::new());
        reg.register("inner", |params| Ok(json!({ "echoed": params })));
        let r = reg.clone();
        reg.register("outer", move |_| r.invoke("inner", json!(42)));
        assert_eq!(
            reg.invoke("outer", json!({})).unwrap(),
            json!({ "echoed": 42 })
        );
    }

    #[test]
    fn error_helpers_have_stable_codes() {
        assert_eq!(invalid_params("x").code, "invalid_params");
        assert_eq!(internal_error("x").code, "internal_error");
    }

    // -- try_dispatch behavior --

    #[test]
    fn try_dispatch_returns_false_for_unknown() {
        let reg = Arc::new(ActionRegistry::new());
        let cb_fired = Arc::new(AtomicUsize::new(0));
        let c = cb_fired.clone();
        let dispatched = reg.try_dispatch(
            "missing",
            json!({}),
            Box::new(move |_| {
                c.fetch_add(1, Ordering::SeqCst);
            }),
        );
        assert!(!dispatched);
        // Callback must NOT fire when action isn't registered.
        assert_eq!(cb_fired.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn try_dispatch_sync_runs_inline_on_caller_thread() {
        let reg = Arc::new(ActionRegistry::new());
        reg.register("ping", |_| Ok(json!("pong")));
        let caller = std::thread::current().id();
        let observed = Arc::new(Mutex::new(None::<std::thread::ThreadId>));
        let captured: Arc<Mutex<Option<ActionResult>>> = Arc::new(Mutex::new(None));
        {
            let obs = observed.clone();
            let cap = captured.clone();
            let dispatched = reg.try_dispatch(
                "ping",
                json!({}),
                Box::new(move |result| {
                    *obs.lock().unwrap() = Some(std::thread::current().id());
                    *cap.lock().unwrap() = Some(result);
                }),
            );
            assert!(dispatched);
        }
        // Sync handler: callback already fired before try_dispatch returned.
        assert_eq!(*observed.lock().unwrap(), Some(caller));
        assert_eq!(
            captured.lock().unwrap().as_ref().unwrap().as_ref().unwrap(),
            &json!("pong")
        );
    }

    #[test]
    fn try_dispatch_blocking_runs_on_worker_thread() {
        let reg = Arc::new(ActionRegistry::new());
        reg.register_blocking("slow", |_| {
            std::thread::sleep(Duration::from_millis(50));
            Ok(json!("done"))
        });
        let caller = std::thread::current().id();
        let observed = Arc::new(Mutex::new(None::<std::thread::ThreadId>));
        let done = Arc::new(Mutex::new(false));
        let captured: Arc<Mutex<Option<ActionResult>>> = Arc::new(Mutex::new(None));
        let start = Instant::now();
        {
            let obs = observed.clone();
            let cap = captured.clone();
            let d = done.clone();
            let dispatched = reg.try_dispatch(
                "slow",
                json!({}),
                Box::new(move |result| {
                    *obs.lock().unwrap() = Some(std::thread::current().id());
                    *cap.lock().unwrap() = Some(result);
                    *d.lock().unwrap() = true;
                }),
            );
            assert!(dispatched);
        }
        // try_dispatch returned in well under the handler's sleep —
        // proving the caller wasn't blocked.
        assert!(
            start.elapsed() < Duration::from_millis(40),
            "try_dispatch returned in {:?}, expected < 40ms",
            start.elapsed()
        );
        // Wait for the worker to finish.
        let deadline = Instant::now() + Duration::from_secs(2);
        while !*done.lock().unwrap() {
            assert!(Instant::now() < deadline, "worker never completed");
            std::thread::sleep(Duration::from_millis(5));
        }
        let observed_tid = observed.lock().unwrap().unwrap();
        assert_ne!(observed_tid, caller, "blocking callback should fire on worker thread, not caller");
        assert_eq!(
            captured.lock().unwrap().as_ref().unwrap().as_ref().unwrap(),
            &json!("done")
        );
    }

    #[test]
    fn try_dispatch_sync_propagates_handler_error_via_callback() {
        let reg = Arc::new(ActionRegistry::new());
        reg.register("fail", |_| Err(invalid_params("nope")));
        let captured: Arc<Mutex<Option<ActionResult>>> = Arc::new(Mutex::new(None));
        let cap = captured.clone();
        reg.try_dispatch(
            "fail",
            json!({}),
            Box::new(move |result| {
                *cap.lock().unwrap() = Some(result);
            }),
        );
        let err = captured
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .as_ref()
            .unwrap_err()
            .clone();
        assert_eq!(err.code, "invalid_params");
        assert_eq!(err.message, "nope");
    }

    #[test]
    fn try_dispatch_blocking_propagates_handler_error_via_callback() {
        let reg = Arc::new(ActionRegistry::new());
        reg.register_blocking("fail-slow", |_| {
            std::thread::sleep(Duration::from_millis(20));
            Err(invalid_params("blocked nope"))
        });
        let captured: Arc<Mutex<Option<ActionResult>>> = Arc::new(Mutex::new(None));
        let done = Arc::new(Mutex::new(false));
        {
            let cap = captured.clone();
            let d = done.clone();
            reg.try_dispatch(
                "fail-slow",
                json!({}),
                Box::new(move |result| {
                    *cap.lock().unwrap() = Some(result);
                    *d.lock().unwrap() = true;
                }),
            );
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        while !*done.lock().unwrap() {
            assert!(Instant::now() < deadline, "blocking worker never completed");
            std::thread::sleep(Duration::from_millis(5));
        }
        let err = captured
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .as_ref()
            .unwrap_err()
            .clone();
        assert_eq!(err.code, "invalid_params");
        assert_eq!(err.message, "blocked nope");
    }

    #[test]
    fn is_blocking_reflects_registration_kind() {
        let reg = ActionRegistry::new();
        reg.register("sync", |_| Ok(json!(null)));
        reg.register_blocking("slow", |_| Ok(json!(null)));
        assert!(!reg.is_blocking("sync"));
        assert!(reg.is_blocking("slow"));
        assert!(!reg.is_blocking("missing"));
    }

    #[test]
    fn register_blocking_overwrites_register_and_vice_versa() {
        // A blocking-registered name can be replaced by a sync
        // registration (and vice versa). The flag is part of the
        // entry, not a separate index.
        let reg = ActionRegistry::new();
        reg.register("x", |_| Ok(json!("sync1")));
        assert!(!reg.is_blocking("x"));
        reg.register_blocking("x", |_| Ok(json!("blocking")));
        assert!(reg.is_blocking("x"));
        reg.register("x", |_| Ok(json!("sync2")));
        assert!(!reg.is_blocking("x"));
    }
}
