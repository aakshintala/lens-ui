//! Coarse card-summary projection for background-warm sessions (§6, D10).

use crate::domain::SessionState;
use crate::domain::controls::{SandboxStatus, TodoStatus};
use crate::domain::ids::HostId;
use crate::domain::item::ItemKind;
use crate::domain::scalars::SessionStatusValue;
use crate::domain::usage::Cost;

/// Coarse card-summary — distinct from `StreamUpdate` (spec §6). Two producers
/// (actor here; §10 poll later). apply = copy-assignment of scalars.
#[derive(Clone, Debug, PartialEq)]
pub struct SummaryUpdate {
    pub status: SessionStatusValue,
    pub title: Option<String>,
    pub last_total_tokens: Option<u64>,
    pub host_id: Option<HostId>,
    pub needs_attention: bool,
    pub subagent_active: bool,
    pub llm_model: Option<String>,
    pub model_override: Option<String>,
    pub agent_name: Option<String>,
    pub cumulative_cost: Cost,
    pub context_window: Option<u64>,
    pub sandbox_status: Option<SandboxStatus>,
    pub git_branch: Option<String>,
    pub workspace: Option<String>,
    pub reasoning_effort: Option<String>,
    pub activity_summary: String,
    pub last_completed_turn: u32,
    pub harness: Option<String>,
}

fn activity_summary(s: &SessionState) -> String {
    if let Some(todo) = s.todos.iter().find(|t| t.status == TodoStatus::InProgress) {
        return todo.active_form.clone();
    }
    // First-started in-flight tool: scan items in order (deterministic) rather than
    // iterating unpaired_calls (a HashMap → nondeterministic pick with >1 tool).
    for item in &s.items {
        if let ItemKind::FunctionCall { call_id, name, .. } = &item.kind
            && s.stream.unpaired_calls.contains_key(call_id)
        {
            return name.clone();
        }
    }
    String::new()
}

impl SummaryUpdate {
    pub fn from_state(s: &SessionState) -> Self {
        Self {
            status: s.status,
            title: s.title.clone(),
            last_total_tokens: s.last_total_tokens,
            host_id: s.host_id.clone(),
            needs_attention: !s.pending_elicitations.is_empty()
                || s.status == SessionStatusValue::Failed,
            // TODO(§9): derive from child-session registry once it exists.
            subagent_active: false,
            llm_model: s.llm_model.clone(),
            model_override: s.model_override.clone(),
            agent_name: s.agent_name.clone(),
            cumulative_cost: s.cumulative_cost.clone(),
            context_window: s.context_window,
            sandbox_status: s.sandbox_status.clone(),
            git_branch: s.git_branch.clone(),
            workspace: s.workspace.clone(),
            reasoning_effort: s.reasoning_effort.clone(),
            activity_summary: activity_summary(s),
            last_completed_turn: s.stream.turn,
            harness: s.harness.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::controls::{SandboxStatus, Todo, TodoStatus};
    use crate::domain::ids::{AgentId, CallId, ConnectionId, ItemId, SessionId};
    use crate::domain::item::{BlockContext, Item, ItemKind};
    use crate::domain::usage::Cost;
    use std::sync::Arc;

    fn base() -> SessionState {
        SessionState::new(
            ConnectionId::new("c"),
            SessionId::new("conv"),
            AgentId::new("ag"),
        )
    }

    #[test]
    fn from_state_copies_card_chrome_and_last_completed_turn() {
        let mut s = base();
        s.llm_model = Some("opus".into());
        s.model_override = Some("sonnet".into());
        s.agent_name = Some("coder".into());
        s.cumulative_cost = Cost {
            total_cost_usd: Some(1.25),
            ..Cost::default()
        };
        s.context_window = Some(200_000);
        s.last_total_tokens = Some(12_000);
        s.sandbox_status = Some(SandboxStatus {
            stage: "ready".into(),
            detail: None,
        });
        s.git_branch = Some("main".into());
        s.workspace = Some("/tmp/proj".into());
        s.reasoning_effort = Some("high".into());
        s.harness = Some("claude-native".into());
        s.stream.turn = 7;
        s.todos.push(Todo {
            content: "wire feed".into(),
            status: TodoStatus::InProgress,
            active_form: "wiring the feed".into(),
        });

        let u = SummaryUpdate::from_state(&s);
        assert_eq!(u.llm_model.as_deref(), Some("opus"));
        assert_eq!(u.model_override.as_deref(), Some("sonnet"));
        assert_eq!(u.agent_name.as_deref(), Some("coder"));
        assert_eq!(u.cumulative_cost.total_cost_usd, Some(1.25));
        assert_eq!(u.context_window, Some(200_000));
        assert_eq!(u.last_total_tokens, Some(12_000));
        assert_eq!(
            u.sandbox_status.as_ref().map(|sb| sb.stage.as_str()),
            Some("ready")
        );
        assert_eq!(u.git_branch.as_deref(), Some("main"));
        assert_eq!(u.workspace.as_deref(), Some("/tmp/proj"));
        assert_eq!(u.reasoning_effort.as_deref(), Some("high"));
        assert_eq!(u.harness.as_deref(), Some("claude-native"));
        assert_eq!(u.last_completed_turn, 7);
        assert_eq!(u.activity_summary, "wiring the feed");
        assert_eq!(u.status, s.status);
        assert_eq!(u.title, s.title);
        assert_eq!(u.host_id, s.host_id);
        assert!(!u.needs_attention);
        assert!(!u.subagent_active);
    }

    #[test]
    fn activity_summary_falls_back_to_in_flight_tool_name() {
        let mut s = base();
        let call_id = CallId::new("call_1");
        let item_id = ItemId::new("fc_1");
        s.stream
            .unpaired_calls
            .insert(call_id.clone(), item_id.clone());
        s.items.push(Arc::new(Item {
            id: item_id,
            seq: None,
            ctx: BlockContext {
                agent: None,
                depth: 0,
                response_id: None,
            },
            created_at: 1,
            kind: ItemKind::FunctionCall {
                call_id,
                name: "bash".into(),
                arguments: serde_json::json!({}),
                status: "in_progress".into(),
                agent_name: None,
            },
        }));
        assert_eq!(SummaryUpdate::from_state(&s).activity_summary, "bash");
    }

    #[test]
    fn activity_summary_is_deterministic_with_two_in_flight_tools() {
        let mut s = base();
        let call_id_1 = CallId::new("call_1");
        let call_id_2 = CallId::new("call_2");
        let item_id_1 = ItemId::new("fc_1");
        let item_id_2 = ItemId::new("fc_2");
        s.stream
            .unpaired_calls
            .insert(call_id_1.clone(), item_id_1.clone());
        s.stream
            .unpaired_calls
            .insert(call_id_2.clone(), item_id_2.clone());
        s.items.push(Arc::new(Item {
            id: item_id_1,
            seq: None,
            ctx: BlockContext {
                agent: None,
                depth: 0,
                response_id: None,
            },
            created_at: 1,
            kind: ItemKind::FunctionCall {
                call_id: call_id_1,
                name: "bash".into(),
                arguments: serde_json::json!({}),
                status: "in_progress".into(),
                agent_name: None,
            },
        }));
        s.items.push(Arc::new(Item {
            id: item_id_2,
            seq: None,
            ctx: BlockContext {
                agent: None,
                depth: 0,
                response_id: None,
            },
            created_at: 2,
            kind: ItemKind::FunctionCall {
                call_id: call_id_2,
                name: "grep".into(),
                arguments: serde_json::json!({}),
                status: "in_progress".into(),
                agent_name: None,
            },
        }));
        assert_eq!(SummaryUpdate::from_state(&s).activity_summary, "bash");
    }
}
