//! Map raw Slack `events_api` payloads to `slack.mention` /
//! `slack.dm` turm events.
//!
//! Slack delivers a wide variety of message types over Socket Mode:
//! channel messages, DMs, edits, deletions, joins, bot messages,
//! thread replies, channel-renames, app_home, etc. The plugin filters
//! aggressively so triggers only fire on signal — actual human
//! mentions and direct messages — without each user having to
//! handle the full diversity in their `[[triggers]]` config.

use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlackEvent {
    Mention(MessageFields),
    Dm(MessageFields),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageFields {
    pub user: String,
    pub channel: String,
    pub text: String,
    pub ts: String,
    pub thread_ts: Option<String>,
    pub team_id: Option<String>,
    pub event_id: Option<String>,
}

impl SlackEvent {
    pub fn kind(&self) -> &'static str {
        match self {
            SlackEvent::Mention(_) => "slack.mention",
            SlackEvent::Dm(_) => "slack.dm",
        }
    }

    pub fn payload_json(&self) -> Value {
        let f = match self {
            SlackEvent::Mention(f) | SlackEvent::Dm(f) => f,
        };
        json!({
            "user": f.user,
            "channel": f.channel,
            "text": f.text,
            "ts": f.ts,
            "thread_ts": f.thread_ts,
            "team_id": f.team_id,
            "event_id": f.event_id,
        })
    }
}

/// Top-level entrypoint: examine an `events_api` envelope payload and
/// either return a turm-shaped event or `None` if the payload should
/// be filtered out.
///
/// `payload` is the value of the outer frame's `payload` key, which
/// itself contains `event_id`, `team_id`, `event`, etc. (Slack's
/// "Events API outer wrapper.")
pub fn from_events_api_payload(payload: &Value) -> Option<SlackEvent> {
    let event = payload.get("event")?;
    let event_id = payload
        .get("event_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    let team_id = payload
        .get("team_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    classify_event(event, event_id, team_id)
}

fn classify_event(
    event: &Value,
    event_id: Option<String>,
    team_id: Option<String>,
) -> Option<SlackEvent> {
    let event_type = event.get("type")?.as_str()?;
    match event_type {
        "app_mention" => {
            // Defensive: skip bot-originated mentions and edits.
            // Slack normally won't send these for app_mention but
            // keeping the filter symmetric with the DM path means a
            // future Slack delivery rule change can't accidentally
            // turn the plugin into a self-loop generator.
            if event.get("subtype").is_some() {
                return None;
            }
            if event.get("bot_id").is_some() {
                return None;
            }
            let f = parse_message_fields(event, event_id, team_id)?;
            Some(SlackEvent::Mention(f))
        }
        "message" => {
            // Filter aggressively. Slack sends edits, deletions, joins,
            // pinned-messages, and bot-broadcasts all under
            // `type=message` — only DMs from a real user without a
            // subtype should reach turm as `slack.dm`.
            //
            // - `subtype` present → edit / delete / join / file_share
            //   etc. Skip; users can layer in handling later via the
            //   raw archive (Phase 11.2) if they want.
            // - `bot_id` present → message was sent by a bot, including
            //   our own self-loops if the bot user happens to chat in
            //   the channel. Skip.
            // - `channel_type != "im"` → not a direct message. Skip.
            if event.get("subtype").is_some() {
                return None;
            }
            if event.get("bot_id").is_some() {
                return None;
            }
            let channel_type = event
                .get("channel_type")
                .and_then(Value::as_str)
                .unwrap_or("");
            if channel_type != "im" {
                return None;
            }
            let f = parse_message_fields(event, event_id, team_id)?;
            Some(SlackEvent::Dm(f))
        }
        _ => None,
    }
}

fn parse_message_fields(
    event: &Value,
    event_id: Option<String>,
    team_id: Option<String>,
) -> Option<MessageFields> {
    Some(MessageFields {
        user: event.get("user").and_then(Value::as_str)?.to_string(),
        channel: event.get("channel").and_then(Value::as_str)?.to_string(),
        text: event
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        ts: event.get("ts").and_then(Value::as_str)?.to_string(),
        thread_ts: event
            .get("thread_ts")
            .and_then(Value::as_str)
            .map(str::to_string),
        team_id,
        event_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload_with(event: Value) -> Value {
        json!({
            "event_id": "Ev0PV52K21",
            "team_id": "T0123",
            "event": event,
        })
    }

    #[test]
    fn parses_app_mention() {
        let p = payload_with(json!({
            "type": "app_mention",
            "user": "U999",
            "channel": "C123",
            "text": "<@U800> ping?",
            "ts": "1700000000.000100",
        }));
        match from_events_api_payload(&p).unwrap() {
            SlackEvent::Mention(f) => {
                assert_eq!(f.user, "U999");
                assert_eq!(f.channel, "C123");
                assert_eq!(f.text, "<@U800> ping?");
                assert_eq!(f.ts, "1700000000.000100");
                assert_eq!(f.team_id.as_deref(), Some("T0123"));
            }
            other => panic!("expected Mention, got {other:?}"),
        }
    }

    #[test]
    fn parses_dm() {
        let p = payload_with(json!({
            "type": "message",
            "channel_type": "im",
            "user": "U999",
            "channel": "D123",
            "text": "hi there",
            "ts": "1700000000.000200",
        }));
        match from_events_api_payload(&p).unwrap() {
            SlackEvent::Dm(f) => {
                assert_eq!(f.user, "U999");
                assert_eq!(f.channel, "D123");
                assert_eq!(f.text, "hi there");
            }
            other => panic!("expected Dm, got {other:?}"),
        }
    }

    #[test]
    fn skips_channel_message() {
        // type=message + channel_type=channel → ordinary channel
        // chatter, not a DM. Must not emit slack.dm.
        let p = payload_with(json!({
            "type": "message",
            "channel_type": "channel",
            "user": "U999",
            "channel": "C123",
            "text": "team standup",
            "ts": "1700000000.000300",
        }));
        assert!(from_events_api_payload(&p).is_none());
    }

    #[test]
    fn skips_message_with_subtype() {
        // Edits, deletions, joins all carry a `subtype` field.
        let p = payload_with(json!({
            "type": "message",
            "channel_type": "im",
            "subtype": "message_changed",
            "user": "U999",
            "channel": "D123",
            "text": "edited",
            "ts": "1700000000.000400",
        }));
        assert!(from_events_api_payload(&p).is_none());
    }

    #[test]
    fn skips_bot_message() {
        let p = payload_with(json!({
            "type": "message",
            "channel_type": "im",
            "bot_id": "B000",
            "channel": "D123",
            "text": "automated",
            "ts": "1700000000.000500",
            "user": "U999",
        }));
        assert!(from_events_api_payload(&p).is_none());
    }

    #[test]
    fn skips_bot_mention() {
        // Defensive: app_mention with bot_id should be filtered too.
        let p = payload_with(json!({
            "type": "app_mention",
            "bot_id": "B000",
            "user": "U999",
            "channel": "C123",
            "text": "<@U800>",
            "ts": "1700000000.000700",
        }));
        assert!(from_events_api_payload(&p).is_none());
    }

    #[test]
    fn skips_mention_with_subtype() {
        let p = payload_with(json!({
            "type": "app_mention",
            "subtype": "message_changed",
            "user": "U999",
            "channel": "C123",
            "text": "<@U800>",
            "ts": "1700000000.000800",
        }));
        assert!(from_events_api_payload(&p).is_none());
    }

    #[test]
    fn skips_unknown_event_type() {
        let p = payload_with(json!({
            "type": "channel_rename",
            "channel": { "id": "C123", "name": "general" },
        }));
        assert!(from_events_api_payload(&p).is_none());
    }

    #[test]
    fn captures_thread_ts() {
        let p = payload_with(json!({
            "type": "app_mention",
            "user": "U999",
            "channel": "C123",
            "text": "in thread",
            "ts": "1700000000.000600",
            "thread_ts": "1700000000.000500",
        }));
        match from_events_api_payload(&p).unwrap() {
            SlackEvent::Mention(f) => {
                assert_eq!(f.thread_ts.as_deref(), Some("1700000000.000500"));
            }
            other => panic!("expected Mention, got {other:?}"),
        }
    }

    #[test]
    fn payload_json_includes_all_fields() {
        let f = MessageFields {
            user: "U999".into(),
            channel: "C123".into(),
            text: "hi".into(),
            ts: "1700.000".into(),
            thread_ts: Some("1700.000".into()),
            team_id: Some("T0".into()),
            event_id: Some("Ev0".into()),
        };
        let v = SlackEvent::Mention(f).payload_json();
        assert_eq!(v["user"], "U999");
        assert_eq!(v["channel"], "C123");
        assert_eq!(v["text"], "hi");
        assert_eq!(v["thread_ts"], "1700.000");
        assert_eq!(v["team_id"], "T0");
        assert_eq!(v["event_id"], "Ev0");
    }

    #[test]
    fn missing_required_fields_returns_none() {
        // No `user` field → can't build MessageFields.
        let p = payload_with(json!({
            "type": "app_mention",
            "channel": "C123",
            "text": "hi",
            "ts": "1700.000",
        }));
        assert!(from_events_api_payload(&p).is_none());
    }
}
