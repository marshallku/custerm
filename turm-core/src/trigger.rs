//! Config-driven `event → action` automation.
//!
//! v1 design (see `docs/workflow-runtime.md`):
//! - Triggers are declared declaratively in TOML / JSON as `[[triggers]]`.
//! - `when` matches an event kind (glob) plus optional payload-field equality.
//! - `params` may contain `{event.X}` / `{context.X}` interpolation tokens.
//! - Action invocations go through `ActionRegistry`; errors are logged but
//!   never propagate, so one bad trigger cannot poison the dispatcher.
//!
//! This module is the pure primitive — no bus subscription, no config
//! loading, no thread management. The platform layer is responsible for
//! pumping events into `dispatch()`.

use crate::action_registry::ActionRegistry;
use crate::context::Context;
use crate::event_bus::{Event, pattern_matches};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Trigger {
    pub name: String,
    pub when: WhenSpec,
    pub action: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WhenSpec {
    /// Glob pattern matched against `event.kind`. Required.
    pub event_kind: String,
    /// Any other keys in the `when` table are treated as payload-field
    /// equality requirements. `{ event_kind = "slack.mention", channel = "alerts" }`
    /// matches `slack.mention` events whose payload has `channel == "alerts"`.
    #[serde(flatten)]
    pub payload_match: Map<String, Value>,
}

impl Trigger {
    pub fn matches(&self, event: &Event) -> bool {
        if !pattern_matches(&self.when.event_kind, &event.kind) {
            return false;
        }
        for (key, expected) in &self.when.payload_match {
            match event.payload.get(key) {
                Some(actual) if actual == expected => continue,
                _ => return false,
            }
        }
        true
    }

    pub fn interpolate(&self, event: &Event, context: Option<&Context>) -> Value {
        interpolate_value(&self.params, event, context)
    }
}

pub struct TriggerEngine {
    triggers: RwLock<Vec<Trigger>>,
    registry: Arc<ActionRegistry>,
}

impl TriggerEngine {
    pub fn new(registry: Arc<ActionRegistry>) -> Self {
        Self {
            triggers: RwLock::new(Vec::new()),
            registry,
        }
    }

    /// Replace the trigger list atomically. Used on startup and on config
    /// hot-reload. Concurrent dispatch sees either the old or the new full
    /// list, never a half-applied state.
    pub fn set_triggers(&self, triggers: Vec<Trigger>) {
        *self.triggers.write().unwrap() = triggers;
    }

    pub fn count(&self) -> usize {
        self.triggers.read().unwrap().len()
    }

    pub fn names(&self) -> Vec<String> {
        self.triggers
            .read()
            .unwrap()
            .iter()
            .map(|t| t.name.clone())
            .collect()
    }

    /// Match every trigger against `event`, interpolate params, invoke
    /// the corresponding action via `ActionRegistry`. Errors are logged.
    /// Returns the number of triggers that fired.
    pub fn dispatch(&self, event: &Event, context: Option<&Context>) -> usize {
        // Snapshot the trigger list under a short read lock so a concurrent
        // `set_triggers` can't observe partial iteration. Triggers are small
        // and infrequent, so cloning is cheap.
        let snapshot: Vec<Trigger> = self.triggers.read().unwrap().clone();
        let mut fired = 0;
        for trigger in &snapshot {
            if !trigger.matches(event) {
                continue;
            }
            let params = trigger.interpolate(event, context);
            match self.registry.invoke(&trigger.action, params) {
                Ok(_) => {
                    fired += 1;
                    log::debug!(
                        "trigger {:?} fired action {:?} for event {:?}",
                        trigger.name,
                        trigger.action,
                        event.kind
                    );
                }
                Err(err) => {
                    log::warn!(
                        "trigger {:?} action {:?} failed for event {:?}: code={} msg={}",
                        trigger.name,
                        trigger.action,
                        event.kind,
                        err.code,
                        err.message
                    );
                }
            }
        }
        fired
    }
}

fn interpolate_value(template: &Value, event: &Event, context: Option<&Context>) -> Value {
    match template {
        Value::String(s) => Value::String(interpolate_string(s, event, context)),
        Value::Array(arr) => Value::Array(
            arr.iter()
                .map(|v| interpolate_value(v, event, context))
                .collect(),
        ),
        Value::Object(obj) => {
            let mut out = Map::new();
            for (k, v) in obj {
                out.insert(k.clone(), interpolate_value(v, event, context));
            }
            Value::Object(out)
        }
        _ => template.clone(),
    }
}

fn interpolate_string(s: &str, event: &Event, context: Option<&Context>) -> String {
    let mut result = String::new();
    let mut rest = s;
    while let Some(open) = rest.find('{') {
        result.push_str(&rest[..open]);
        let after_open = &rest[open + 1..];
        if let Some(close_rel) = after_open.find('}') {
            let token = &after_open[..close_rel];
            if let Some(val) = resolve_token(token, event, context) {
                result.push_str(&val);
            } else {
                // Unresolvable token: keep the literal `{token}` so misconfigured
                // triggers fail loudly in their target action rather than
                // silently substituting empty string.
                result.push('{');
                result.push_str(token);
                result.push('}');
            }
            rest = &after_open[close_rel + 1..];
        } else {
            // Unclosed `{` — append the remainder verbatim.
            result.push_str(&rest[open..]);
            return result;
        }
    }
    result.push_str(rest);
    result
}

fn resolve_token(token: &str, event: &Event, context: Option<&Context>) -> Option<String> {
    if let Some(field) = token.strip_prefix("event.") {
        return event.payload.get(field).map(json_scalar_to_string);
    }
    if let Some(field) = token.strip_prefix("context.") {
        let ctx = context?;
        return match field {
            "active_panel" => ctx.active_panel.clone(),
            "active_cwd" => ctx
                .active_cwd
                .as_ref()
                .map(|p| p.to_string_lossy().to_string()),
            _ => None,
        };
    }
    None
}

fn json_scalar_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "null".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action_registry::invalid_params;
    use serde_json::json;
    use std::path::PathBuf;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn evt(kind: &str, payload: Value) -> Event {
        Event::new(kind, "test", payload)
    }

    fn mk_engine() -> (Arc<ActionRegistry>, TriggerEngine) {
        let reg = Arc::new(ActionRegistry::new());
        let engine = TriggerEngine::new(reg.clone());
        (reg, engine)
    }

    #[test]
    fn matches_exact_kind() {
        let t = Trigger {
            name: "t".into(),
            when: WhenSpec {
                event_kind: "calendar.event_imminent".into(),
                payload_match: Map::new(),
            },
            action: "noop".into(),
            params: Value::Null,
        };
        assert!(t.matches(&evt("calendar.event_imminent", json!({}))));
        assert!(!t.matches(&evt("calendar.event_started", json!({}))));
    }

    #[test]
    fn matches_glob_kind() {
        let t = Trigger {
            name: "t".into(),
            when: WhenSpec {
                event_kind: "calendar.*".into(),
                payload_match: Map::new(),
            },
            action: "noop".into(),
            params: Value::Null,
        };
        assert!(t.matches(&evt("calendar.event_imminent", json!({}))));
        assert!(t.matches(&evt("calendar.event_created", json!({}))));
        assert!(!t.matches(&evt("slack.mention", json!({}))));
    }

    #[test]
    fn payload_match_required() {
        let t = Trigger {
            name: "t".into(),
            when: WhenSpec {
                event_kind: "slack.mention".into(),
                payload_match: {
                    let mut m = Map::new();
                    m.insert("channel".into(), json!("alerts"));
                    m
                },
            },
            action: "noop".into(),
            params: Value::Null,
        };
        assert!(t.matches(&evt("slack.mention", json!({"channel": "alerts", "text": "hi"}))));
        assert!(!t.matches(&evt("slack.mention", json!({"channel": "general"}))));
        assert!(!t.matches(&evt("slack.mention", json!({})))); // missing field
    }

    #[test]
    fn interpolates_event_payload_fields() {
        let t = Trigger {
            name: "t".into(),
            when: WhenSpec {
                event_kind: "*".into(),
                payload_match: Map::new(),
            },
            action: "noop".into(),
            params: json!({
                "id": "{event.id}",
                "msg": "got {event.id} from {event.source}",
            }),
        };
        let result = t.interpolate(
            &evt("calendar.event_imminent", json!({"id": "abc", "source": "x"})),
            None,
        );
        // event.source resolves from payload (we publish "test" as source but
        // tokens look up payload, not the top-level Event::source field).
        assert_eq!(result["id"], json!("abc"));
        assert_eq!(result["msg"], json!("got abc from x"));
    }

    #[test]
    fn interpolates_context_fields() {
        let t = Trigger {
            name: "t".into(),
            when: WhenSpec {
                event_kind: "*".into(),
                payload_match: Map::new(),
            },
            action: "noop".into(),
            params: json!({"cmd": "echo {context.active_cwd} :: {context.active_panel}"}),
        };
        let ctx = Context {
            active_panel: Some("panel-1".into()),
            active_cwd: Some(PathBuf::from("/tmp/work")),
        };
        let result = t.interpolate(&evt("any", json!({})), Some(&ctx));
        assert_eq!(result["cmd"], json!("echo /tmp/work :: panel-1"));
    }

    #[test]
    fn unresolved_tokens_kept_as_literals() {
        let t = Trigger {
            name: "t".into(),
            when: WhenSpec {
                event_kind: "*".into(),
                payload_match: Map::new(),
            },
            action: "noop".into(),
            params: json!({
                "a": "{event.missing}",
                "b": "{unknown}",
                "c": "no braces",
                "d": "unclosed {brace",
            }),
        };
        let result = t.interpolate(&evt("any", json!({})), None);
        assert_eq!(result["a"], json!("{event.missing}"));
        assert_eq!(result["b"], json!("{unknown}"));
        assert_eq!(result["c"], json!("no braces"));
        assert_eq!(result["d"], json!("unclosed {brace"));
    }

    #[test]
    fn interpolation_walks_nested_arrays_and_objects() {
        let t = Trigger {
            name: "t".into(),
            when: WhenSpec {
                event_kind: "*".into(),
                payload_match: Map::new(),
            },
            action: "noop".into(),
            params: json!({
                "list": ["{event.a}", "x", {"deep": "{event.b}"}],
                "n": 42,
                "b": true,
            }),
        };
        let result = t.interpolate(&evt("any", json!({"a": "A", "b": "B"})), None);
        assert_eq!(result["list"][0], json!("A"));
        assert_eq!(result["list"][1], json!("x"));
        assert_eq!(result["list"][2]["deep"], json!("B"));
        assert_eq!(result["n"], json!(42));
        assert_eq!(result["b"], json!(true));
    }

    #[test]
    fn dispatch_invokes_matching_action_with_interpolated_params() {
        let (reg, engine) = mk_engine();
        let captured = Arc::new(Mutex::new(Vec::<Value>::new()));
        {
            let c = captured.clone();
            reg.register("record", move |params| {
                c.lock().unwrap().push(params);
                Ok(json!(null))
            });
        }
        engine.set_triggers(vec![Trigger {
            name: "t".into(),
            when: WhenSpec {
                event_kind: "calendar.event_imminent".into(),
                payload_match: Map::new(),
            },
            action: "record".into(),
            params: json!({"id": "{event.id}"}),
        }]);
        let fired = engine.dispatch(
            &evt("calendar.event_imminent", json!({"id": "evt-9"})),
            None,
        );
        assert_eq!(fired, 1);
        let captured = captured.lock().unwrap();
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0], json!({"id": "evt-9"}));
    }

    #[test]
    fn dispatch_skips_non_matching_triggers() {
        let (reg, engine) = mk_engine();
        let count = Arc::new(AtomicUsize::new(0));
        {
            let c = count.clone();
            reg.register("bump", move |_| {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(json!(null))
            });
        }
        engine.set_triggers(vec![Trigger {
            name: "only_slack".into(),
            when: WhenSpec {
                event_kind: "slack.*".into(),
                payload_match: Map::new(),
            },
            action: "bump".into(),
            params: Value::Null,
        }]);
        engine.dispatch(&evt("calendar.event_imminent", json!({})), None);
        engine.dispatch(&evt("terminal.cwd_changed", json!({})), None);
        assert_eq!(count.load(Ordering::SeqCst), 0);
        engine.dispatch(&evt("slack.mention", json!({})), None);
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn action_error_is_logged_not_propagated() {
        let (reg, engine) = mk_engine();
        reg.register("fail", |_| Err(invalid_params("nope")));
        engine.set_triggers(vec![Trigger {
            name: "t".into(),
            when: WhenSpec {
                event_kind: "any".into(),
                payload_match: Map::new(),
            },
            action: "fail".into(),
            params: Value::Null,
        }]);
        // Should not panic. fired count is 0 because the action returned Err.
        let fired = engine.dispatch(&evt("any", json!({})), None);
        assert_eq!(fired, 0);
    }

    #[test]
    fn unknown_action_is_logged_not_propagated() {
        let (_reg, engine) = mk_engine();
        engine.set_triggers(vec![Trigger {
            name: "t".into(),
            when: WhenSpec {
                event_kind: "any".into(),
                payload_match: Map::new(),
            },
            action: "no_such_action".into(),
            params: Value::Null,
        }]);
        let fired = engine.dispatch(&evt("any", json!({})), None);
        assert_eq!(fired, 0);
    }

    #[test]
    fn set_triggers_replaces_existing_atomically() {
        let (reg, engine) = mk_engine();
        let count = Arc::new(AtomicUsize::new(0));
        {
            let c = count.clone();
            reg.register("bump", move |_| {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(json!(null))
            });
        }
        let make = |kind: &str| Trigger {
            name: kind.into(),
            when: WhenSpec {
                event_kind: kind.into(),
                payload_match: Map::new(),
            },
            action: "bump".into(),
            params: Value::Null,
        };
        engine.set_triggers(vec![make("a"), make("b")]);
        assert_eq!(engine.count(), 2);
        engine.dispatch(&evt("a", json!({})), None);
        engine.dispatch(&evt("b", json!({})), None);
        assert_eq!(count.load(Ordering::SeqCst), 2);

        engine.set_triggers(vec![make("c")]);
        assert_eq!(engine.count(), 1);
        engine.dispatch(&evt("a", json!({})), None);
        engine.dispatch(&evt("b", json!({})), None);
        // No further bumps: a/b triggers are gone.
        assert_eq!(count.load(Ordering::SeqCst), 2);
        engine.dispatch(&evt("c", json!({})), None);
        assert_eq!(count.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn deserializes_from_toml_round_trip() {
        let toml_src = r#"
            name = "meeting-prep"
            action = "plugin.notion.open_event_doc"
            params = { event_id = "{event.id}", lead_minutes = 10 }

            [when]
            event_kind = "calendar.event_imminent"
            minutes = 10
        "#;
        let t: Trigger = toml::from_str(toml_src).unwrap();
        assert_eq!(t.name, "meeting-prep");
        assert_eq!(t.action, "plugin.notion.open_event_doc");
        assert_eq!(t.when.event_kind, "calendar.event_imminent");
        // The non-`event_kind` field under `[when]` becomes a payload match.
        assert_eq!(t.when.payload_match["minutes"], json!(10));
        // `params` interpolates as a normal Value tree.
        assert_eq!(t.params["event_id"], json!("{event.id}"));
        assert_eq!(t.params["lead_minutes"], json!(10));
    }
}
