use smallvec::SmallVec;

/// The reducer's output: which part of `SessionState` a `reduce()` call changed.
/// DRAFT (spec D6): marker-style at P1 (no payload, no `apply()` — no replica exists
/// yet). The P3 walking skeleton ratifies whether apply carries a payload or re-reads a
/// shared snapshot. `SmallVec<[_; 2]>` because most events touch 0–2 groups.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StreamUpdate {
    // ── transcript deltas ──
    /// A new canonical item was appended at `index`.
    ItemAppended {
        index: usize,
    },
    /// An existing canonical item at `index` was updated in place (dedup-by-id hit).
    ItemUpdated {
        index: usize,
    },
    /// `StreamScratch` (in-progress message/reasoning) changed — the live preview bubble.
    ScratchChanged,

    // ── scalar / collection folds ──
    StatusChanged,
    UsageChanged,
    ModelChanged,
    ReasoningEffortChanged,
    CollaborationModeChanged,
    ModelOptionsChanged,
    TodosChanged,
    SkillsChanged,
    SandboxChanged,
    TerminalPendingChanged,
    ElicitationsChanged,
    ChildSessionChanged,
    PresenceChanged,
    ResourcesChanged,
    /// `agent_id`/`agent_name` changed AND an `AgentChanged` transcript marker was pushed.
    AgentChanged,
    TitleChanged,

    // ── reconnect / bootstrap lifecycle (passthrough for the UI banner) ──
    Reconnecting {
        attempt: u32,
    },
    Reconnected,
    Disconnected,
    /// Coarse: the snapshot chrome scalars were bulk-restored (bootstrap or reconnect).
    SnapshotRestored,
}

pub type Updates = SmallVec<[StreamUpdate; 2]>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn updates_smallvec_stays_inline_for_two() {
        let mut u: Updates = SmallVec::new();
        u.push(StreamUpdate::StatusChanged);
        u.push(StreamUpdate::ItemAppended { index: 0 });
        assert_eq!(u.len(), 2);
        assert!(
            !u.spilled(),
            "the [_; 2] inline cap must hold the common case"
        );
    }
}
