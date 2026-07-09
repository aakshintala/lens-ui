//! `SessionState` — the per-session view-model (§2.2). Mirrors omnigent's
//! `SessionResponse` plus Lens-local fields. Pure data; the reducer (P1) is the
//! only writer (single-writer invariant, §8).

use crate::domain::controls::PendingUserMessage;
use crate::domain::controls::{Elicitation, ModelOption, SandboxStatus, SkillSummary, Todo};
use crate::domain::ids::{AgentId, ConnectionId, HostId, RunnerId, SessionId};
use crate::domain::item::{Item, StreamScratch};
use crate::domain::scalars::{ErrorInfo, HostType, SessionLifecycle, SessionStatusValue};
use crate::domain::usage::{Cost, PresenceViewer};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionState {
    // ── Identity & binding ──
    pub connection_id: ConnectionId,
    pub id: SessionId,
    pub agent_id: AgentId,
    pub agent_name: Option<String>,
    pub runner_id: Option<RunnerId>,
    pub parent_session_id: Option<SessionId>,

    // ── Status & lifecycle ──
    pub status: SessionStatusValue,
    pub last_task_error: Option<ErrorInfo>,
    /// Epoch SECONDS (distinct from Item.created_at millis).
    pub created_at: i64,

    // ── Model & controls ──
    pub llm_model: Option<String>,
    pub model_override: Option<String>,
    pub model_options: Option<Vec<ModelOption>>,
    pub reasoning_effort: Option<String>,
    pub collaboration_mode: Option<String>,
    pub context_window: Option<u64>,
    pub last_total_tokens: Option<u64>,
    pub cumulative_cost: Cost,

    // ── Workspace & host ──
    pub workspace: Option<String>,
    pub git_branch: Option<String>,
    pub host_type: HostType,
    pub host_id: Option<HostId>,
    pub sandbox_status: Option<SandboxStatus>,
    /// Live `session.terminal_pending` fold (§4.1). RAM+persisted scalar.
    pub terminal_pending: bool,

    // ── Content ──
    pub items: Vec<Item>,
    pub todos: Vec<Todo>,
    pub skills: Vec<SkillSummary>,

    // ── Display & policy ──
    pub title: Option<String>,
    pub labels: BTreeMap<String, String>,
    pub permission_level: Option<u8>,
    pub pending_elicitations: Vec<Elicitation>,
    pub owner: Option<String>,

    // ── chrome: presence & co-viewers (RAM-only; excluded from P2 schema) ──
    pub presence: Vec<PresenceViewer>,

    // ── Lens-local transient (RAM only, never persisted) ──
    pub stream: StreamScratch,
    pub pending_user: Vec<PendingUserMessage>,

    // ── Lens-local persisted metadata ──
    pub archived: bool,
    pub lifecycle: SessionLifecycle,
    /// active-set LRU (epoch millis).
    pub last_focused_at: i64,
    /// reconcile cursor (typed client §7).
    pub last_seen_seq: Option<u64>,
}

impl SessionState {
    /// A fresh, empty session bound to `(connection, id)` with the given agent.
    /// Convenience constructor for the reducer/tests; all collections empty,
    /// status `Idle`, lifecycle `Active`.
    pub fn new(connection_id: ConnectionId, id: SessionId, agent_id: AgentId) -> Self {
        Self {
            connection_id,
            id,
            agent_id,
            agent_name: None,
            runner_id: None,
            parent_session_id: None,
            status: SessionStatusValue::Idle,
            last_task_error: None,
            created_at: 0,
            llm_model: None,
            model_override: None,
            model_options: None,
            reasoning_effort: None,
            collaboration_mode: None,
            context_window: None,
            last_total_tokens: None,
            cumulative_cost: Cost::default(),
            workspace: None,
            git_branch: None,
            host_type: HostType::External,
            host_id: None,
            sandbox_status: None,
            terminal_pending: false,
            items: Vec::new(),
            todos: Vec::new(),
            skills: Vec::new(),
            title: None,
            labels: BTreeMap::new(),
            permission_level: None,
            pending_elicitations: Vec::new(),
            owner: None,
            presence: Vec::new(),
            stream: StreamScratch::default(),
            pending_user: Vec::new(),
            archived: false,
            lifecycle: SessionLifecycle::Active,
            last_focused_at: 0,
            last_seen_seq: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_session_is_idle_active_and_empty() {
        let s = SessionState::new(
            ConnectionId::new("conn_1"),
            SessionId::new("conv_1"),
            AgentId::new("agent_1"),
        );
        assert_eq!(s.status, SessionStatusValue::Idle);
        assert_eq!(s.lifecycle, SessionLifecycle::Active);
        assert!(s.items.is_empty());
        assert!(!s.archived);
    }

    #[test]
    fn empty_session_roundtrips() {
        let s = SessionState::new(
            ConnectionId::new("conn_1"),
            SessionId::new("conv_1"),
            AgentId::new("agent_1"),
        );
        let back: SessionState = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn populated_session_roundtrips() {
        use crate::domain::ids::ItemId;
        use crate::domain::item::{BlockContext, ContentBlock, Item, ItemKind};
        use crate::domain::scalars::Role;

        let mut s = SessionState::new(
            ConnectionId::new("conn_1"),
            SessionId::new("conv_1"),
            AgentId::new("agent_1"),
        );
        s.status = SessionStatusValue::Running;
        s.title = Some("my session".into());
        s.labels.insert("env".into(), "prod".into());
        s.items.push(Item {
            id: ItemId::new("item_1"),
            seq: Some(1),
            ctx: BlockContext {
                agent: None,
                depth: 0,
                turn: 0,
            },
            created_at: 1_700_000_000_000,
            kind: ItemKind::Message {
                role: Role::User,
                content: vec![ContentBlock {
                    kind: "text".into(),
                    text: Some("hello".into()),
                    data: serde_json::Value::Null,
                }],
            },
        });
        let back: SessionState = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
    }
}
