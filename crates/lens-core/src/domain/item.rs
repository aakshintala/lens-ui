//! The canonical conversation unit (§2.3/§2.4) + transient stream accumulators
//! (§4.2). `Item` is the durable, reduced unit the transcript and disk hold.

use crate::domain::ids::{AgentId, CallId, ItemId};
use crate::domain::scalars::{ErrorSource, Role};
use lens_client::generated::SessionResourceObject;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// Attribution stamped onto every `Item` by the reducer (§2.4). Pure
/// attribution — the durable "when" lives on `Item.created_at` (grilling 2026-07-08).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockContext {
    /// "coder" | "coder.researcher"; None = root.
    pub agent: Option<String>,
    /// 0 = root, 1 = sub-agent, …
    pub depth: u32,
    /// Turn within the response.
    pub turn: u32,
}

/// One block of message content (§2.3 `Message.content`). `text` for text blocks;
/// `data` carries the opaque remainder for non-text blocks.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ContentBlock {
    pub kind: String,
    pub text: Option<String>,
    #[serde(default)]
    pub data: Value,
}

/// The durable, reduced conversation unit (§2.3).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Item {
    /// THE dedup/identity key. Persisted items carry only `id`, no seq.
    pub id: ItemId,
    /// SSE `sequence_number` when seen live; None for `GET /items`. Live-overlap
    /// dedup only — never a storage key.
    pub seq: Option<u64>,
    pub ctx: BlockContext,
    /// Epoch MILLIS, stamped by the reducer from an injected clock. The durable
    /// "when" (§2.3, replaces the dropped `BlockContext.timestamp`).
    pub created_at: i64,
    pub kind: ItemKind,
}

/// The typed item union (§2.3), mirroring omnigent conversation items.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ItemKind {
    Message {
        role: Role,
        content: Vec<ContentBlock>,
    },
    FunctionCall {
        call_id: CallId,
        name: String,
        arguments: Value,
        /// Wire enum: in_progress | completed | action_required | incomplete.
        status: String,
        agent_name: Option<String>,
    },
    FunctionCallOutput {
        call_id: CallId,
        output: String,
        arguments: Value,
    },
    Reasoning {
        full_text: String,
        summary_text: String,
        encrypted: bool,
    },
    /// web_search_call, mcp_call, …
    NativeTool {
        tool_type: String,
        data: Value,
    },
    Compaction {
        summary: String,
        token_count: Option<u64>,
    },
    SlashCommand {
        name: String,
        raw: String,
    },
    TerminalCommand {
        command: String,
    },
    /// Persisted error banner (mirrors response.error).
    Error {
        source: ErrorSource,
        code: String,
        message: String,
    },
    /// env | terminal | file (workspace doc). See Decision D-P0-1.
    ResourceEvent {
        resource: SessionResourceObject,
    },
    /// Switch-agent marker; `from` is SYNTHESIZED by the reducer (the wire event
    /// carries only agent_id/agent_name).
    AgentChanged {
        from: AgentId,
        to: AgentId,
        at: i64,
    },
}

// ── §4.2 transient accumulators (RAM-only) ──

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct StreamScratch {
    pub open_message: Option<MessageAcc>,
    pub open_reasoning: Option<ReasoningAcc>,
    pub unpaired_calls: HashMap<CallId, ItemId>,
    /// Reduce-local (§4.1 attribution): current turn, bumped on `response.completed`.
    pub turn: u32,
    /// Reduce-local: current agent name for `BlockContext` stamping.
    pub current_agent: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MessageAcc {
    /// 0.2.0: terminal-observed correlation.
    pub message_id: Option<String>,
    pub text: String,
    pub block_index: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ReasoningAcc {
    pub full_text: String,
    pub summary_text: String,
    pub encrypted: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use lens_client::generated::Type;
    use serde_json::json;

    fn ctx() -> BlockContext {
        BlockContext {
            agent: None,
            depth: 0,
            turn: 0,
        }
    }

    #[test]
    fn message_item_roundtrips() {
        let item = Item {
            id: ItemId::new("item_1"),
            seq: Some(3),
            ctx: ctx(),
            created_at: 1_700_000_000_000,
            kind: ItemKind::Message {
                role: Role::Assistant,
                content: vec![ContentBlock {
                    kind: "text".into(),
                    text: Some("hi".into()),
                    data: Value::Null,
                }],
            },
        };
        let back: Item = serde_json::from_str(&serde_json::to_string(&item).unwrap()).unwrap();
        assert_eq!(back, item);
    }

    #[test]
    fn item_created_at_survives_roundtrip_as_i64_millis() {
        // Grilling revision: durable "when" is created_at millis; no monotonic value.
        let item = Item {
            id: ItemId::new("item_2"),
            seq: None,
            ctx: ctx(),
            created_at: 1_700_000_123_456,
            kind: ItemKind::SlashCommand {
                name: "clear".into(),
                raw: "/clear".into(),
            },
        };
        let back: Item = serde_json::from_str(&serde_json::to_string(&item).unwrap()).unwrap();
        assert_eq!(back.created_at, 1_700_000_123_456);
        assert_eq!(back, item);
    }

    #[test]
    fn every_itemkind_variant_roundtrips() {
        let kinds = vec![
            ItemKind::FunctionCall {
                call_id: CallId::new("call_1"),
                name: "read".into(),
                arguments: json!({"path": "a.rs"}),
                status: "in_progress".into(),
                agent_name: Some("coder".into()),
            },
            ItemKind::FunctionCallOutput {
                call_id: CallId::new("call_1"),
                output: "ok".into(),
                arguments: json!({"path": "a.rs"}),
            },
            ItemKind::Reasoning {
                full_text: "think".into(),
                summary_text: "t".into(),
                encrypted: false,
            },
            ItemKind::NativeTool {
                tool_type: "web_search_call".into(),
                data: json!({"q": "x"}),
            },
            ItemKind::Compaction {
                summary: "s".into(),
                token_count: Some(42),
            },
            ItemKind::TerminalCommand {
                command: "ls".into(),
            },
            ItemKind::Error {
                source: ErrorSource::Server,
                code: "boom".into(),
                message: "kaboom".into(),
            },
            ItemKind::AgentChanged {
                from: AgentId::new("a"),
                to: AgentId::new("b"),
                at: 1_700_000_000_000,
            },
        ];
        for kind in kinds {
            let back: ItemKind =
                serde_json::from_str(&serde_json::to_string(&kind).unwrap()).unwrap();
            assert_eq!(back, kind);
        }
    }

    #[test]
    fn resource_event_roundtrips() {
        let kind = ItemKind::ResourceEvent {
            resource: SessionResourceObject {
                environment: None,
                id: "default".into(),
                metadata: serde_json::Map::new(),
                name: "workspace".into(),
                object: "session.resource".into(),
                session_id: "conv_1".into(),
                type_: Type::Environment,
            },
        };
        let back: ItemKind = serde_json::from_str(&serde_json::to_string(&kind).unwrap()).unwrap();
        assert_eq!(back, kind);
    }

    #[test]
    fn stream_scratch_default_is_empty_and_roundtrips() {
        let s = StreamScratch::default();
        assert!(s.open_message.is_none());
        assert!(s.unpaired_calls.is_empty());
        assert_eq!(s.turn, 0);
        assert!(s.current_agent.is_none());
        let back: StreamScratch =
            serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
    }
}
