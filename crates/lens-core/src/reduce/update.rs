use crate::domain::controls::{Elicitation, ModelOption, SandboxStatus, SkillSummary, Todo};
use crate::domain::ids::AgentId;
use crate::domain::item::{Item, StreamScratch};
use crate::domain::scalars::{ErrorInfo, SessionStatusValue};
use crate::domain::session::SessionState;
use crate::domain::usage::{Cost, PresenceViewer};
use smallvec::SmallVec;
use std::sync::Arc;

/// The reducer's output: which part of `SessionState` a `reduce()` call changed.
/// Value-carrying (D8): each delta deposits its just-reduced value into the
/// foreground replica via pure copy-assignment. `SmallVec<[_; 2]>` because most
/// events touch 0–2 groups.
#[derive(Clone, Debug, PartialEq)]
pub enum StreamUpdate {
    // ── transcript deltas (value-carrying) ──
    ItemAppended(Arc<Item>),
    ItemUpdated {
        index: usize,
        item: Arc<Item>,
    },
    ScratchChanged(Arc<StreamScratch>),

    // ── scalar / collection folds — carry the just-reduced value ──
    StatusChanged(SessionStatusValue),
    LastTaskErrorChanged(Option<ErrorInfo>),
    UsageChanged(Cost),
    ModelChanged {
        llm_model: Option<String>,
        model_override: Option<String>,
    },
    ReasoningEffortChanged(Option<String>),
    CollaborationModeChanged(Option<String>),
    ModelOptionsChanged(Option<Vec<ModelOption>>),
    TodosChanged(Vec<Todo>),
    SkillsChanged(Vec<SkillSummary>),
    SandboxChanged(Option<SandboxStatus>),
    TerminalPendingChanged(bool),
    ElicitationsChanged(Vec<Elicitation>),
    ChildSessionChanged,
    PresenceChanged(Vec<PresenceViewer>),
    ResourcesChanged,
    AgentChanged {
        agent_id: AgentId,
        agent_name: Option<String>,
    },
    TitleChanged(Option<String>),
    LastTokensChanged(Option<u64>),
    ContextWindowChanged(Option<u64>),

    // ── reconnect / bootstrap lifecycle (passthrough for the UI banner) ──
    Reconnecting {
        attempt: u32,
    },
    Reconnected,
    Disconnected,
    SnapshotRestored,

    // D9: once-at-attach full baseline
    Rebased(Box<SessionState>),
}

pub type Updates = SmallVec<[StreamUpdate; 2]>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ids::ItemId;
    use crate::domain::item::{BlockContext, ContentBlock, ItemKind};
    use crate::domain::scalars::Role;

    #[test]
    fn updates_smallvec_stays_inline_for_two() {
        let mut u: Updates = SmallVec::new();
        u.push(StreamUpdate::StatusChanged(SessionStatusValue::Idle));
        u.push(StreamUpdate::ItemAppended(Arc::new(Item {
            id: ItemId::new("item_0"),
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
                    data: serde_json::Value::Null,
                }],
            },
        })));
        assert_eq!(u.len(), 2);
        assert!(
            !u.spilled(),
            "the [_; 2] inline cap must hold the common case"
        );
    }
}
