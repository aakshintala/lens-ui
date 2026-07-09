//! Foreground gpui replica for one session — pure copy-assignment via `apply`.

use gpui::{App, AppContext, Entity};
use lens_core::domain::SessionState;
use lens_core::reduce::StreamUpdate;

/// Deposit an already-reduced delta into the replica by pure copy-assignment.
/// NEVER re-derives, NEVER runs `reduce`, NEVER does I/O. O(1) per delta
/// (item bodies are `Arc`, so appends/updates move a pointer). (spec D8)
pub fn apply(state: &mut SessionState, update: StreamUpdate) {
    use StreamUpdate::*;
    match update {
        ItemAppended(item) => state.items.push(item),
        ItemUpdated { index, item } => {
            if index < state.items.len() {
                state.items[index] = item;
            }
        }
        ScratchChanged(scratch) => state.stream = (*scratch).clone(),
        StatusChanged(v) => state.status = v,
        UsageChanged(c) => state.cumulative_cost = c,
        ModelChanged {
            llm_model,
            model_override,
        } => {
            state.llm_model = llm_model;
            state.model_override = model_override;
        }
        ReasoningEffortChanged(v) => state.reasoning_effort = v,
        CollaborationModeChanged(v) => state.collaboration_mode = v,
        ModelOptionsChanged(v) => state.model_options = v,
        TodosChanged(v) => state.todos = v,
        SkillsChanged(v) => state.skills = v,
        SandboxChanged(v) => state.sandbox_status = v,
        TerminalPendingChanged(v) => state.terminal_pending = v,
        ElicitationsChanged(v) => state.pending_elicitations = v,
        PresenceChanged(v) => state.presence = v,
        AgentChanged {
            agent_id,
            agent_name,
        } => {
            state.agent_id = agent_id;
            state.agent_name = agent_name;
        }
        TitleChanged(v) => state.title = v,
        LastTokensChanged(v) => state.last_total_tokens = v,
        ContextWindowChanged(v) => state.context_window = v,
        Rebased(baseline) => *state = *baseline,
        // markers with no replica-visible payload in P3-1
        ChildSessionChanged
        | ResourcesChanged
        | SnapshotRestored
        | Reconnecting { .. }
        | Reconnected
        | Disconnected => {}
    }
}

pub struct SessionStore {
    entity: Entity<SessionState>,
}

impl SessionStore {
    pub fn new(cx: &mut App, initial: SessionState) -> Self {
        Self {
            entity: cx.new(|_cx| initial),
        }
    }

    pub fn entity(&self) -> &Entity<SessionState> {
        &self.entity
    }

    pub fn read<'a>(&self, cx: &'a App) -> &'a SessionState {
        self.entity.read(cx)
    }

    /// Apply one delta on the foreground and notify observers. Called by the
    /// drain bridge (Task 4).
    pub fn apply_on(&self, cx: &mut App, update: StreamUpdate) {
        self.entity.update(cx, |state, cx| {
            apply(state, update);
            cx.notify();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::SessionStore;
    use lens_core::domain::scalars::SessionStatusValue;
    use lens_core::domain::{AgentId, ConnectionId, SessionId, SessionState};
    use lens_core::reduce::StreamUpdate;

    fn state() -> SessionState {
        SessionState::new(
            ConnectionId::new("c"),
            SessionId::new("conv"),
            AgentId::new("ag"),
        )
    }

    fn sample_item(id: &str) -> lens_core::domain::Item {
        use lens_core::domain::ids::ItemId;
        use lens_core::domain::item::{BlockContext, ContentBlock, Item, ItemKind};
        use lens_core::domain::scalars::Role;

        Item {
            id: ItemId::new(id),
            seq: None,
            ctx: BlockContext {
                agent: None,
                depth: 0,
                turn: 0,
            },
            created_at: 0,
            kind: ItemKind::Message {
                role: Role::User,
                content: vec![ContentBlock {
                    kind: "text".into(),
                    text: Some("x".into()),
                    data: Default::default(),
                }],
            },
        }
    }

    #[test]
    fn apply_status_is_copy_assignment() {
        let mut s = state();
        super::apply(
            &mut s,
            StreamUpdate::StatusChanged(SessionStatusValue::Running),
        );
        assert_eq!(s.status, SessionStatusValue::Running);
    }

    #[test]
    fn apply_item_appended_pushes_shared_body() {
        let mut s = state();
        let item = std::sync::Arc::new(sample_item("item_1"));
        super::apply(
            &mut s,
            StreamUpdate::ItemAppended(std::sync::Arc::clone(&item)),
        );
        assert_eq!(s.items.len(), 1);
        assert!(std::sync::Arc::ptr_eq(&s.items[0], &item));
    }

    #[test]
    fn apply_rebased_replaces_whole_state() {
        let mut s = state();
        let mut baseline = state();
        baseline.title = Some("rebased".into());
        super::apply(&mut s, StreamUpdate::Rebased(Box::new(baseline)));
        assert_eq!(s.title.as_deref(), Some("rebased"));
    }

    #[gpui::test]
    fn store_applies_and_reads_back(cx: &mut gpui::TestAppContext) {
        let store = cx.update(|cx| SessionStore::new(cx, state()));
        cx.update(|cx| {
            store.apply_on(cx, StreamUpdate::StatusChanged(SessionStatusValue::Running));
        });
        let status = cx.read(|cx| store.read(cx).status);
        assert_eq!(status, SessionStatusValue::Running);
    }
}
