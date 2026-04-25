use crate::protocol::ResponseError;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

pub type ActionResult = Result<Value, ResponseError>;
pub type ActionFn = Arc<dyn Fn(Value) -> ActionResult + Send + Sync + 'static>;

pub struct ActionRegistry {
    handlers: RwLock<HashMap<String, ActionFn>>,
}

impl ActionRegistry {
    pub fn new() -> Self {
        Self {
            handlers: RwLock::new(HashMap::new()),
        }
    }

    pub fn register<F>(&self, name: impl Into<String>, handler: F)
    where
        F: Fn(Value) -> ActionResult + Send + Sync + 'static,
    {
        self.handlers
            .write()
            .unwrap()
            .insert(name.into(), Arc::new(handler));
    }

    pub fn invoke(&self, name: &str, params: Value) -> ActionResult {
        // Clone out the handler Arc under the read lock, then drop the guard
        // before running the handler. This keeps handler execution off the
        // lock entirely so a handler may freely call `register`, `invoke`,
        // or other registry methods without risking deadlock.
        let handler = {
            let handlers = self.handlers.read().unwrap();
            handlers
                .get(name)
                .cloned()
                .ok_or_else(|| not_found_error(name))?
        };
        handler(params)
    }

    /// Like `invoke`, but returns `None` if the action is not registered
    /// (rather than synthesizing an `action_not_found` error). Useful for
    /// dispatchers that want to fall through to a different handler when
    /// the registry has no entry for the method.
    pub fn try_invoke(&self, name: &str, params: Value) -> Option<ActionResult> {
        let handler = self.handlers.read().unwrap().get(name).cloned()?;
        Some(handler(params))
    }

    pub fn has(&self, name: &str) -> bool {
        self.handlers.read().unwrap().contains_key(name)
    }

    pub fn names(&self) -> Vec<String> {
        let mut out: Vec<String> = self.handlers.read().unwrap().keys().cloned().collect();
        out.sort();
        out
    }

    pub fn len(&self) -> usize {
        self.handlers.read().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.handlers.read().unwrap().is_empty()
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
}
