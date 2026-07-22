use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use gpui::{Context, Entity, prelude::*};
use lens_core::domain::board::{
    Board, BoardLayout, DEFAULT_BOARD_ID, DEFAULT_BOARD_NAME, PlacementTarget,
};
use lens_core::domain::ids::{BoardId, ConnectionId, SessionId};
use lens_core::persist::{BoardStore, PersistError, SqliteBoardStore, StoreMode};

use crate::fleet::store::FleetStore;

pub(crate) const MAX_RETRIES: u32 = 5;

pub(crate) enum Op {
    Load { initial: bool },
    PlaceSessions(Vec<(ConnectionId, SessionId)>),
}

enum OpOutcome {
    Loaded {
        layout: BoardLayout,
        skipped_empty: bool,
        mode: StoreMode,
        initial: bool,
    },
    Placed {
        layout: BoardLayout,
        skipped_empty: bool,
        mode: StoreMode,
    },
    Failed {
        op: Op,
        err: PersistError,
    },
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
    fn build(
        store: Option<Box<dyn BoardStore + Send>>,
        path: PathBuf,
        conn: ConnectionId,
        tempdir: Option<tempfile::TempDir>,
        fleet: Entity<FleetStore>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self {
            store: Arc::new(Mutex::new(StoreSlot { path, store })),
            conn,
            layout: default_board_layout(),
            state: ReplicaState::Loading,
            fleet,
            in_flight: false,
            pending: VecDeque::new(),
            reconcile_in_flight: false,
            recovery_in_flight: false,
            op_retries: 0,
            suppressed: HashSet::new(),
            last_attempt: Vec::new(),
            dropped_writes: 0,
            banner_dismissed: false,
            _tempdir: tempdir,
        };
        cx.observe(&this.fleet.clone(), |this: &mut Self, _f, cx| {
            this.on_fleet_change(cx)
        })
        .detach();
        this.run_op(Op::Load { initial: true }, cx);
        this
    }

    pub fn for_test(fleet: Entity<FleetStore>, cx: &mut Context<Self>) -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("board.db");
        let store: Box<dyn BoardStore + Send> =
            Box::new(SqliteBoardStore::open(&path).expect("open test store"));
        Self::build(
            Some(store),
            path,
            ConnectionId::new("conn_test"),
            Some(dir),
            fleet,
            cx,
        )
    }

    pub(crate) fn run_op(&mut self, op: Op, cx: &mut Context<Self>) {
        self.pending.push_back(op);
        self.pump(cx);
    }

    pub fn layout(&self) -> &BoardLayout {
        &self.layout
    }

    pub fn state(&self) -> ReplicaState {
        self.state
    }

    pub fn is_writable(&self) -> bool {
        state_is_writable(self.state)
    }

    fn pump(&mut self, cx: &mut Context<Self>) {
        if self.in_flight {
            return;
        }
        // Re-gate: drop write ops no longer allowed (state flipped after they queued).
        let op = loop {
            match self.pending.pop_front() {
                None => return,
                Some(Op::PlaceSessions(_)) if !self.is_writable() => {
                    self.dropped_writes = self.dropped_writes.saturating_add(1);
                    continue;
                }
                Some(op) => break op,
            }
        };
        self.in_flight = true;
        if let Op::PlaceSessions(ref keys) = op {
            self.last_attempt = keys.clone();
        }
        let store = Arc::clone(&self.store);
        cx.spawn(async move |this, cx| {
            let outcome = cx
                .background_executor()
                .spawn(async move {
                    let mut slot = store.lock().unwrap_or_else(|p| p.into_inner()); // poison → recover, never panic
                    run_op_blocking(&mut slot, op)
                })
                .await;
            this.update(cx, |this, cx| this.apply_outcome(outcome, cx))
                .ok();
        })
        .detach();
    }

    fn apply_outcome(&mut self, outcome: OpOutcome, cx: &mut Context<Self>) {
        self.in_flight = false;
        match outcome {
            OpOutcome::Loaded {
                layout,
                skipped_empty,
                mode,
                initial: _,
            } => {
                self.op_retries = 0;
                self.recovery_in_flight = false;
                self.layout = layout;
                self.state = load_state(mode, skipped_empty);
                if self.is_writable() {
                    self.reconcile(cx); // initial/post-recovery reconcile (Task 6)
                }
            }
            OpOutcome::Placed {
                layout,
                skipped_empty,
                mode,
            } => {
                self.op_retries = 0;
                self.layout = layout;
                self.state = load_state(mode, skipped_empty); // ~always Writable; consistent
                self.reconcile_in_flight = false;
                self.note_place_result(); // suppress stuck keys (Task 6, C1)
                self.reconcile(cx); // re-diff on reply (Task 6)
            }
            OpOutcome::Failed { op, err } => {
                self.on_op_failed(op, err, cx); // Task 5
            }
        }
        cx.notify();
        self.pump(cx);
    }

    fn reconcile(&mut self, _cx: &mut Context<Self>) {}

    fn note_place_result(&mut self) {}

    fn on_op_failed(&mut self, _op: Op, _err: PersistError, _cx: &mut Context<Self>) {
        self.state = ReplicaState::Stale;
    }

    fn on_fleet_change(&mut self, _cx: &mut Context<Self>) {}
}

fn run_op_blocking(slot: &mut StoreSlot, op: Op) -> OpOutcome {
    match run_op_inner(slot, &op) {
        Ok(outcome) => outcome,
        Err(err) => {
            slot.store = None; // reopen fresh on the next Load (recovery)
            OpOutcome::Failed { op, err }
        }
    }
}

fn run_op_inner(slot: &mut StoreSlot, op: &Op) -> lens_core::persist::Result<OpOutcome> {
    if slot.store.is_none() {
        slot.store = Some(Box::new(SqliteBoardStore::open(&slot.path)?)); // first-open or recovery
    }
    let Some(store) = slot.store.as_deref() else {
        return Err(PersistError::ReadOnly); // unreachable (just opened); typed, never a panic
    };
    match op {
        Op::Load { initial } => {
            let (layout, skipped_empty, mode) = read_committed(store)?;
            Ok(OpOutcome::Loaded {
                layout,
                skipped_empty,
                mode,
                initial: *initial,
            })
        }
        Op::PlaceSessions(keys) => {
            store.place_sessions(keys, &default_root_target())?; // persist
            let (layout, skipped_empty, mode) = read_committed(store)?; // reconciled read (M5 rebuttal)
            Ok(OpOutcome::Placed {
                layout,
                skipped_empty,
                mode,
            })
        }
    }
}

/// `load_layout` applies read-time reconcile (lazy-place + tombstone-prune), so this is
/// the authoritative committed view — for both Load and post-Place reads.
fn read_committed(
    store: &dyn BoardStore,
) -> lens_core::persist::Result<(BoardLayout, bool, StoreMode)> {
    let loaded = store.load_layout()?;
    let skipped_empty = loaded.skipped.is_empty();
    let layout = loaded.rows.into_iter().next().unwrap_or_default();
    Ok((layout, skipped_empty, store.mode()))
}

fn default_root_target() -> PlacementTarget {
    PlacementTarget {
        board_id: None,
        parent_item_id: None,
        ordinal: None,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use lens_core::domain::board::{BoardItemKind, DEFAULT_BOARD_ID};
    use lens_core::persist::StoreMode;

    use crate::clock::{ManualUiClock, UiClock};

    use super::*;

    fn test_fleet(cx: &mut gpui::App) -> Entity<FleetStore> {
        FleetStore::new(Arc::new(ManualUiClock::new(10_000)) as Arc<dyn UiClock>, cx)
    }

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

    #[gpui::test]
    async fn load_op_populates_layout_and_becomes_writable(cx: &mut gpui::TestAppContext) {
        let fleet = cx.update(|cx| test_fleet(cx));
        let replica = cx.update(|cx| cx.new(|cx| BoardReplica::for_test(fleet.clone(), cx)));
        cx.run_until_parked();
        replica.read_with(cx, |r, _| {
            assert_eq!(r.state(), ReplicaState::Writable);
            assert_eq!(
                r.layout().default_board_id().unwrap().as_str(),
                DEFAULT_BOARD_ID
            );
        });
    }

    #[gpui::test]
    async fn two_place_ops_apply_in_enqueue_order(cx: &mut gpui::TestAppContext) {
        let fleet = cx.update(|cx| test_fleet(cx));
        let replica = cx.update(|cx| cx.new(|cx| BoardReplica::for_test(fleet.clone(), cx)));
        cx.run_until_parked();
        let c = ConnectionId::new("conn_test");
        replica.update(cx, |r, cx| {
            r.run_op(
                Op::PlaceSessions(vec![(c.clone(), SessionId::new("a"))]),
                cx,
            );
            r.run_op(
                Op::PlaceSessions(vec![(c.clone(), SessionId::new("b"))]),
                cx,
            );
        });
        cx.run_until_parked();
        replica.read_with(cx, |r, _| {
            let n = r
                .layout()
                .items
                .iter()
                .filter(|i| matches!(i.kind, BoardItemKind::Card { .. }))
                .count();
            assert_eq!(n, 2);
        });
    }
}
