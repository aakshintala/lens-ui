//! Foreground gpui replica for one session — pure copy-assignment via `apply`.

use gpui::{App, AppContext, Entity, Task};
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
        LastTaskErrorChanged(v) => state.last_task_error = v,
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

/// Foreground drain: event-driven wakeup (`recv().await`) + greedy `try_recv`
/// coalescing so a burst of deltas costs ONE `cx.notify()`/frame. Ends when the
/// actor drops its sender (channel closed). Detach the returned Task to run it.
pub fn spawn_apply_bridge(
    store: SessionStore,
    rx: async_channel::Receiver<StreamUpdate>,
    cx: &mut App,
) -> Task<()> {
    cx.spawn(async move |cx| {
        while let Ok(first) = rx.recv().await {
            let mut batch = smallvec::SmallVec::<[StreamUpdate; 8]>::new();
            batch.push(first);
            while let Ok(more) = rx.try_recv() {
                batch.push(more);
            }
            let applied = store.entity().update(cx, |state, cx| {
                for u in batch.drain(..) {
                    apply(state, u);
                }
                cx.notify();
            });
            if applied.is_err() {
                break; // replica entity released — nothing to update
            }
        }
    })
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

    #[gpui::test]
    async fn skeleton_off_thread_event_reaches_foreground(cx: &mut gpui::TestAppContext) {
        let (tx, rx) = async_channel::bounded::<StreamUpdate>(1024);

        let store = cx.update(|cx| SessionStore::new(cx, state()));
        let entity = store.entity().clone();
        let _bridge = cx.update(|cx| super::spawn_apply_bridge(store, rx, cx));

        // A plain OS thread stands in for the actor: emit a Rebased baseline, then a delta.
        let sender = std::thread::spawn(move || {
            let mut s = state();
            s.title = Some("baseline".into());
            tx.send_blocking(StreamUpdate::Rebased(Box::new(s)))
                .unwrap();
            tx.send_blocking(StreamUpdate::StatusChanged(SessionStatusValue::Running))
                .unwrap();
            // drop(tx) closes the channel and ends the bridge loop.
        });

        sender.join().expect("sender thread");
        cx.run_until_parked();
        let (title, status) = cx.read(|cx| {
            let s = entity.read(cx);
            (s.title.clone(), s.status)
        });
        assert_eq!(
            title.as_deref(),
            Some("baseline"),
            "Rebased baseline applied"
        );
        assert_eq!(
            status,
            SessionStatusValue::Running,
            "delta applied after baseline"
        );
    }

    #[gpui::test]
    async fn skeleton_coalesces_a_burst_into_few_notifies(cx: &mut gpui::TestAppContext) {
        let (tx, rx) = async_channel::bounded::<StreamUpdate>(1024);
        let store = cx.update(|cx| SessionStore::new(cx, state()));
        let entity = store.entity().clone();
        // Observe notify count.
        let notifies = std::rc::Rc::new(std::cell::Cell::new(0usize));
        let n2 = notifies.clone();
        let _sub = cx.update(|cx| cx.observe(&entity, move |_, _| n2.set(n2.get() + 1)));
        let _bridge = cx.update(|cx| super::spawn_apply_bridge(store, rx, cx));

        let sender = std::thread::spawn(move || {
            for i in 0..500u64 {
                tx.send_blocking(StreamUpdate::LastTokensChanged(Some(i)))
                    .unwrap();
            }
        });
        sender.join().expect("sender thread");
        cx.run_until_parked();
        let last = cx.read(|cx| entity.read(cx).last_total_tokens);
        assert_eq!(last, Some(499));
        let notify_count = notifies.get();
        assert!(
            notify_count < 500,
            "500 deltas coalesced into {notify_count} notifies"
        );
    }
}
