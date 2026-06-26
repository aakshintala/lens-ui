//! The `Sessions` subservice and the generalized `/events` write path.
//!
//! `SessionEventInput` here is the **hand-written** typed enum for the subset of
//! events Lens sends — distinct from `crate::generated::SessionEventInput`, which
//! is the raw `{type, data, model_override, tools}` wire container. Discriminators
//! and payload shapes are pinned to omnigent 0.3.0.dev0 source
//! (`server/routes/sessions.py`, `entities/conversation.py`); never guess them.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::client::Client;
use crate::error::Result;
use crate::http::decode_json;
use crate::ids::{ElicitationId, SessionId};

/// Session status as reported by the REST surface (snapshot + list). Only three
/// values reach REST; the server collapses `waiting`→`running` and never emits
/// `launching` on parents (`sessions.py:1792-1811`). The richer 5-value SSE
/// status (`SessionStatusEvent`) is modeled separately in the streaming plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatus {
    Idle,
    Running,
    Failed,
}

/// Ack for `POST /v1/sessions/{id}/events` (HTTP 202). The openapi declares an
/// empty body, but the route always returns a small JSON ack — model it with
/// defaults so an empty or future-extended body still deserializes.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct SendEventAck {
    /// Whether the event was queued to the runner. Control events report `false`.
    #[serde(default)]
    pub queued: bool,
    /// Store item id for persisted item events (`message`, …). For
    /// `function_call_output` this echoes the `call_id`.
    #[serde(default)]
    pub item_id: Option<String>,
    /// Pending id for the native-terminal `message` bypass path.
    #[serde(default)]
    pub pending_id: Option<String>,
    /// Set when a policy denied a user `message`.
    #[serde(default)]
    pub denied: bool,
    /// Human-readable denial reason (paired with `denied`).
    #[serde(default)]
    pub reason: Option<String>,
    /// Elicitation id for the `mcp_elicitation` path.
    #[serde(default)]
    pub elicitation_id: Option<String>,
}

/// Consumer reply action for an `approval` event (MCP `ElicitResult` semantics).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ElicitationAction {
    Accept,
    Decline,
    Cancel,
}

impl ElicitationAction {
    fn as_str(self) -> &'static str {
        match self {
            ElicitationAction::Accept => "accept",
            ElicitationAction::Decline => "decline",
            ElicitationAction::Cancel => "cancel",
        }
    }
}

/// A client-submitted session event. Only the subset Lens sends is modeled;
/// the server accepts a larger dispatch table (pinned in `ALLOWED_EVENT_TYPES`).
#[derive(Clone, Debug, PartialEq)]
pub enum SessionEventInput {
    /// A user message. `content` is a list of open content blocks, e.g.
    /// `{"type":"input_text","text":"…"}`. `role` is always `"user"` on send.
    Message {
        content: Vec<Value>,
        model_override: Option<String>,
        tools: Option<Vec<Value>>,
    },
    /// A client-side tool result.
    FunctionCallOutput { call_id: String, output: String },
    /// Reply to an outstanding elicitation. Wire `data` is flat:
    /// `{elicitation_id, action, content?}`.
    Approval {
        elicitation_id: ElicitationId,
        action: ElicitationAction,
        content: Option<Map<String, Value>>,
    },
    /// Interrupt the active turn.
    Interrupt,
    /// Request context compaction (control event `"compact"`, not the
    /// `"compaction"` item type).
    Compact,
    /// Terminate the live session (reclaim the runner). Owner-gated server-side.
    StopSession,
}

impl SessionEventInput {
    /// The wire `type` discriminator.
    pub fn discriminator(&self) -> &'static str {
        match self {
            SessionEventInput::Message { .. } => "message",
            SessionEventInput::FunctionCallOutput { .. } => "function_call_output",
            SessionEventInput::Approval { .. } => "approval",
            SessionEventInput::Interrupt => "interrupt",
            SessionEventInput::Compact => "compact",
            SessionEventInput::StopSession => "stop_session",
        }
    }

    /// Serialize into the wire envelope: `{ "type": <discrim>, "data": <payload>, .. }`.
    pub fn to_json(&self) -> Value {
        let mut obj = Map::new();
        obj.insert("type".into(), json!(self.discriminator()));

        let data: Value = match self {
            SessionEventInput::Message { content, .. } => {
                json!({ "role": "user", "content": content })
            }
            SessionEventInput::FunctionCallOutput { call_id, output } => {
                json!({ "call_id": call_id, "output": output })
            }
            SessionEventInput::Approval {
                elicitation_id,
                action,
                content,
            } => {
                let mut d = Map::new();
                d.insert("elicitation_id".into(), json!(elicitation_id.as_str()));
                d.insert("action".into(), json!(action.as_str()));
                if let Some(c) = content {
                    d.insert("content".into(), Value::Object(c.clone()));
                }
                Value::Object(d)
            }
            SessionEventInput::Interrupt
            | SessionEventInput::Compact
            | SessionEventInput::StopSession => json!({}),
        };
        obj.insert("data".into(), data);

        // `model_override` / `tools` are envelope-level and only meaningful for `message`.
        if let SessionEventInput::Message {
            model_override,
            tools,
            ..
        } = self
        {
            if let Some(m) = model_override {
                obj.insert("model_override".into(), json!(m));
            }
            if let Some(t) = tools {
                obj.insert("tools".into(), json!(t));
            }
        }

        Value::Object(obj)
    }
}

/// The full set of `type` discriminators the server's `POST /events` route
/// accepts (`_ALLOWED_EVENT_TYPES`, omnigent 0.3.0.dev0). Lens only *sends* the
/// six modeled by `SessionEventInput`, but the contract test pins the whole set
/// so a re-vendor that adds/removes a type is a conscious change. Kept sorted.
pub const ALLOWED_EVENT_TYPES: [&str; 30] = [
    "approval",
    "compact",
    "compaction",
    "error",
    "external_assistant_message",
    "external_codex_collaboration_mode_change",
    "external_codex_subagent_start",
    "external_compaction_status",
    "external_conversation_item",
    "external_elicitation_resolved",
    "external_model_change",
    "external_output_reasoning_delta",
    "external_output_text_delta",
    "external_reasoning_effort_change",
    "external_session_interrupted",
    "external_session_status",
    "external_session_todos",
    "external_session_usage",
    "external_subagent_start",
    "function_call",
    "function_call_output",
    "interrupt",
    "mcp_elicitation",
    "message",
    "native_tool",
    "reasoning",
    "resource_event",
    "slash_command",
    "stop_session",
    "terminal_command",
];

/// The session subservice — borrows the `Client` for the duration of a call.
pub struct Sessions<'a> {
    client: &'a Client,
}

impl<'a> Sessions<'a> {
    pub(crate) fn new(client: &'a Client) -> Self {
        Self { client }
    }

    /// `POST /v1/sessions/{id}/events` — submit a typed event. Returns the
    /// server's ack (queued/item_id/denial). Blocking.
    pub fn send_event(&self, id: &SessionId, evt: &SessionEventInput) -> Result<SendEventAck> {
        let conn = self.client.conn();
        let url = conn.url(&format!("/v1/sessions/{id}/events"))?;
        let resp = conn
            .auth
            .apply(self.client.http().post(url).json(&evt.to_json()))
            .send()?;
        let status = resp.status().as_u16();
        let body = resp.text()?;
        decode_json("sessions/events", status, &body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ElicitationId;
    use serde_json::json;

    #[test]
    fn message_serializes_role_user_and_content() {
        let evt = SessionEventInput::Message {
            content: vec![json!({"type": "input_text", "text": "Hello"})],
            model_override: None,
            tools: None,
        };
        assert_eq!(evt.discriminator(), "message");
        assert_eq!(
            evt.to_json(),
            json!({
                "type": "message",
                "data": {"role": "user", "content": [{"type": "input_text", "text": "Hello"}]}
            })
        );
    }

    #[test]
    fn message_includes_model_override_and_tools_when_present() {
        let evt = SessionEventInput::Message {
            content: vec![json!({"type": "input_text", "text": "hi"})],
            model_override: Some("anthropic/claude".into()),
            tools: Some(vec![json!({"type": "function", "function": {"name": "f"}})]),
        };
        let v = evt.to_json();
        assert_eq!(v["model_override"], json!("anthropic/claude"));
        assert_eq!(
            v["tools"],
            json!([{"type": "function", "function": {"name": "f"}}])
        );
    }

    #[test]
    fn function_call_output_carries_call_id_and_output() {
        let evt = SessionEventInput::FunctionCallOutput {
            call_id: "call_abc".into(),
            output: "{\"ok\":true}".into(),
        };
        assert_eq!(evt.discriminator(), "function_call_output");
        assert_eq!(
            evt.to_json(),
            json!({"type": "function_call_output", "data": {"call_id": "call_abc", "output": "{\"ok\":true}"}})
        );
    }

    #[test]
    fn approval_is_flat_with_elicitation_id_and_action() {
        let evt = SessionEventInput::Approval {
            elicitation_id: ElicitationId::new("elicit_1"),
            action: ElicitationAction::Accept,
            content: Some(serde_json::Map::from_iter([(
                "choice".to_string(),
                json!("a"),
            )])),
        };
        assert_eq!(evt.discriminator(), "approval");
        assert_eq!(
            evt.to_json(),
            json!({"type": "approval", "data": {"elicitation_id": "elicit_1", "action": "accept", "content": {"choice": "a"}}})
        );
    }

    #[test]
    fn approval_omits_content_when_none() {
        let evt = SessionEventInput::Approval {
            elicitation_id: ElicitationId::new("elicit_2"),
            action: ElicitationAction::Decline,
            content: None,
        };
        assert_eq!(
            evt.to_json(),
            json!({"type": "approval", "data": {"elicitation_id": "elicit_2", "action": "decline"}})
        );
    }

    #[test]
    fn control_events_send_empty_data() {
        for (evt, ty) in [
            (SessionEventInput::Interrupt, "interrupt"),
            (SessionEventInput::Compact, "compact"),
            (SessionEventInput::StopSession, "stop_session"),
        ] {
            assert_eq!(evt.discriminator(), ty);
            assert_eq!(evt.to_json(), json!({"type": ty, "data": {}}));
        }
    }

    #[test]
    fn allowed_event_types_is_the_pinned_30() {
        // Pinned to omnigent 0.3.0.dev0 (sessions.py _ALLOWED_EVENT_TYPES,
        // = ITEM_TYPE_TO_DATA_CLS keys ∪ control/external extras). Sorted.
        assert_eq!(ALLOWED_EVENT_TYPES.len(), 30);
        let mut sorted = ALLOWED_EVENT_TYPES;
        sorted.sort_unstable();
        assert_eq!(
            sorted, ALLOWED_EVENT_TYPES,
            "keep ALLOWED_EVENT_TYPES sorted"
        );
    }

    #[test]
    fn every_sent_discriminator_is_server_allowed() {
        for evt in [
            SessionEventInput::Message {
                content: vec![],
                model_override: None,
                tools: None,
            },
            SessionEventInput::FunctionCallOutput {
                call_id: "c".into(),
                output: "o".into(),
            },
            SessionEventInput::Approval {
                elicitation_id: crate::ids::ElicitationId::new("e"),
                action: ElicitationAction::Accept,
                content: None,
            },
            SessionEventInput::Interrupt,
            SessionEventInput::Compact,
            SessionEventInput::StopSession,
        ] {
            assert!(
                ALLOWED_EVENT_TYPES.contains(&evt.discriminator()),
                "{} not in ALLOWED_EVENT_TYPES",
                evt.discriminator()
            );
        }
    }

    #[test]
    fn ack_parses_queued_with_item_id() {
        let ack: SendEventAck =
            serde_json::from_str(r#"{"queued": true, "item_id": "item_42"}"#).unwrap();
        assert!(ack.queued);
        assert_eq!(ack.item_id.as_deref(), Some("item_42"));
        assert!(!ack.denied);
    }

    #[test]
    fn ack_parses_control_event_not_queued() {
        let ack: SendEventAck = serde_json::from_str(r#"{"queued": false}"#).unwrap();
        assert!(!ack.queued);
        assert_eq!(ack.item_id, None);
    }

    #[test]
    fn ack_parses_policy_denial() {
        let ack: SendEventAck =
            serde_json::from_str(r#"{"queued": false, "denied": true, "reason": "blocked"}"#)
                .unwrap();
        assert!(ack.denied);
        assert_eq!(ack.reason.as_deref(), Some("blocked"));
    }

    #[test]
    fn session_status_deserializes_rest_values() {
        use serde_json::json;
        assert_eq!(
            serde_json::from_value::<SessionStatus>(json!("idle")).unwrap(),
            SessionStatus::Idle
        );
        assert_eq!(
            serde_json::from_value::<SessionStatus>(json!("running")).unwrap(),
            SessionStatus::Running
        );
        assert_eq!(
            serde_json::from_value::<SessionStatus>(json!("failed")).unwrap(),
            SessionStatus::Failed
        );
        // "waiting" is collapsed to "running" server-side and never reaches REST; reject it.
        assert!(serde_json::from_value::<SessionStatus>(json!("waiting")).is_err());
    }

    #[test]
    fn ack_tolerates_unknown_and_missing_fields() {
        // Empty body (openapi's under-specified `{}`) must still deserialize.
        let ack: SendEventAck = serde_json::from_str("{}").unwrap();
        assert!(!ack.queued);
        // Unknown extra fields are ignored, not an error.
        let ack2: SendEventAck =
            serde_json::from_str(r#"{"queued": true, "future_field": 1}"#).unwrap();
        assert!(ack2.queued);
    }
}
