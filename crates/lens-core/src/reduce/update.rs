use crate::domain::controls::{
    Elicitation, ModelOption, PendingUserMessage, SandboxStatus, SkillSummary, Todo,
};
use crate::domain::ids::{AgentId, ResponseId};
use crate::domain::item::StreamScratch;
use crate::domain::scalars::{ErrorInfo, SessionStatusValue};
use crate::domain::session::SessionState;
use crate::domain::usage::{Cost, PresenceViewer};
use lens_client::sessions::PendingInput;
use smallvec::SmallVec;
use std::sync::Arc;

/// The reducer's output: which part of `SessionState` a `reduce()` call changed.
/// Value-carrying (D8): each delta deposits its just-reduced value into the
/// foreground replica via pure copy-assignment. `SmallVec<[_; 2]>` because most
/// events touch 0–2 groups.
#[derive(Clone, Debug, PartialEq)]
pub enum StreamUpdate {
    // ── transcript deltas (value-carrying) ──
    /// D23: disk-canonical transcript watermark. The actor emits this AFTER a
    /// commit-on-terminal write-through; the focused replica reads
    /// `(last_rendered, committed_ordinal]` off `TranscriptStore` (RowSource — deferred UI).
    TranscriptAdvanced {
        committed_ordinal: i64,
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
    // reserved — no live SSE producer in P3; title mutates via SnapshotRestored/Rebased only unless a future fold emits this delta.
    CollaborationModeChanged(Option<String>),
    ModelOptionsChanged(Option<Vec<ModelOption>>),
    TodosChanged(Vec<Todo>),
    SkillsChanged(Vec<SkillSummary>),
    SandboxChanged(Option<SandboxStatus>),
    TerminalPendingChanged(bool),
    ElicitationsChanged(Vec<Elicitation>),
    PendingUserChanged(Vec<PendingUserMessage>),
    ChildSessionChanged,
    PresenceChanged(Vec<PresenceViewer>),
    ResourcesChanged,
    AgentChanged {
        agent_id: AgentId,
        agent_name: Option<String>,
    },
    // reserved — no live SSE producer in P3; title mutates via SnapshotRestored/Rebased only unless a future fold emits this delta.
    TitleChanged(Option<String>),
    LastTokensChanged(Option<u64>),
    ContextWindowChanged(Option<u64>),
    /// The session's live active response changed. `Some` on `response.in_progress`,
    /// `None` on any terminal `response.*` (idle/unknown). Sourced from
    /// `response.in_progress.response.id` — the only working in-process liveness source.
    ActiveResponseChanged(Option<ResponseId>),

    // ── reconnect / bootstrap lifecycle (passthrough for the UI banner) ──
    Reconnecting {
        attempt: u32,
    },
    Reconnected,
    Disconnected(lens_client::stream::DisconnectReason),
    /// D28: snapshot `pending_inputs` plumbed for held-bubble path-1 stamping on reconnect.
    SnapshotRestored(Vec<PendingInput>),

    // D9: once-at-attach full baseline
    Rebased(Box<SessionState>),
}

pub type Updates = SmallVec<[StreamUpdate; 2]>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::scalars::SessionStatusValue;

    #[test]
    fn updates_smallvec_stays_inline_for_two() {
        let mut u: Updates = SmallVec::new();
        u.push(StreamUpdate::StatusChanged(SessionStatusValue::Idle));
        u.push(StreamUpdate::TranscriptAdvanced {
            committed_ordinal: 0,
        });
        assert_eq!(u.len(), 2);
        assert!(
            !u.spilled(),
            "the [_; 2] inline cap must hold the common case"
        );
    }
}
