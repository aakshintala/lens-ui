use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use gpui::Entity;
use lens_core::domain::board::{Board, BoardLayout, DEFAULT_BOARD_ID, DEFAULT_BOARD_NAME};
use lens_core::domain::ids::{BoardId, ConnectionId, SessionId};
use lens_core::persist::{BoardStore, StoreMode};

use crate::fleet::store::FleetStore;

pub(crate) const MAX_RETRIES: u32 = 5;

pub(crate) enum Op {
    Load { initial: bool },
    PlaceSessions(Vec<(ConnectionId, SessionId)>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplicaState {
    Loading,
    Writable,
    Degraded,
    LoadFailed,
    Stale,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteDisposition {
    Accepted,
    Rejected(ReplicaState),
}

pub(crate) struct StoreSlot {
    pub(crate) path: PathBuf,
    pub(crate) store: Option<Box<dyn BoardStore + Send>>,
}

pub struct BoardReplica {
    pub(crate) store: Arc<Mutex<StoreSlot>>,
    pub(crate) conn: ConnectionId,
    pub(crate) layout: BoardLayout,
    pub(crate) state: ReplicaState,
    pub(crate) fleet: Entity<FleetStore>,
    pub(crate) in_flight: bool,
    pub(crate) pending: VecDeque<Op>,
    pub(crate) reconcile_in_flight: bool,
    pub(crate) recovery_in_flight: bool,
    pub(crate) op_retries: u32,
    pub(crate) suppressed: HashSet<(String, String)>, // (conn,session) tombstoned/stuck (C1)
    pub(crate) last_attempt: Vec<(ConnectionId, SessionId)>, // keys of the in-flight PlaceSessions
    pub(crate) dropped_writes: u32,                   // banner honesty (M8)
    pub(crate) banner_dismissed: bool,
    pub(crate) _tempdir: Option<tempfile::TempDir>, // keeps test/demo file alive; None in prod
}

pub(crate) fn state_is_writable(s: ReplicaState) -> bool {
    matches!(s, ReplicaState::Writable)
}

/// Read succeeded; degrade on a future-schema store OR any skipped (corrupt) rows.
pub(crate) fn load_state(mode: StoreMode, skipped_empty: bool) -> ReplicaState {
    match mode {
        StoreMode::ReadOnlyDegraded => ReplicaState::Degraded,
        StoreMode::ReadWrite if skipped_empty => ReplicaState::Writable,
        StoreMode::ReadWrite => ReplicaState::Degraded,
    }
}

pub(crate) fn default_board_layout() -> BoardLayout {
    BoardLayout {
        boards: vec![Board {
            id: BoardId::new(DEFAULT_BOARD_ID),
            name: DEFAULT_BOARD_NAME.into(),
            ordinal: 0,
            created_at: 0,
            updated_at: 0,
        }],
        items: vec![],
    }
}

impl BoardReplica {
    pub fn layout(&self) -> &BoardLayout {
        &self.layout
    }

    pub fn state(&self) -> ReplicaState {
        self.state
    }

    pub fn is_writable(&self) -> bool {
        state_is_writable(self.state)
    }
}

#[cfg(test)]
mod tests {
    use lens_core::domain::board::DEFAULT_BOARD_ID;
    use lens_core::persist::StoreMode;

    use super::*;

    #[test]
    fn default_board_layout_has_a_default_board() {
        let l = default_board_layout();
        assert_eq!(l.default_board_id().unwrap().as_str(), DEFAULT_BOARD_ID);
        assert!(l.items.is_empty());
    }

    #[test]
    fn load_state_maps_mode_and_skips() {
        assert_eq!(
            load_state(StoreMode::ReadWrite, true),
            ReplicaState::Writable
        );
        assert_eq!(
            load_state(StoreMode::ReadWrite, false),
            ReplicaState::Degraded
        ); // skipped rows
        assert_eq!(
            load_state(StoreMode::ReadOnlyDegraded, true),
            ReplicaState::Degraded
        ); // future schema
    }

    #[test]
    fn is_writable_only_in_writable_state() {
        assert!(state_is_writable(ReplicaState::Writable));
        for s in [
            ReplicaState::Loading,
            ReplicaState::Degraded,
            ReplicaState::LoadFailed,
            ReplicaState::Stale,
        ] {
            assert!(!state_is_writable(s));
        }
    }
}
