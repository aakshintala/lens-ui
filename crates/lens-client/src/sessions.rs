//! The `Sessions` subservice and the generalized `/events` write path.
//!
//! `SessionEventInput` here is the **hand-written** typed enum for the subset of
//! events Lens sends — distinct from `crate::generated::SessionEventInput`, which
//! is the raw `{type, data, model_override, tools}` wire container. Discriminators
//! and payload shapes are pinned to omnigent 0.3.0.dev0 source
//! (`server/routes/sessions.py`, `entities/conversation.py`); never guess them.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::ids::ElicitationId;

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
}
