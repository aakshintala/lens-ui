use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui::{Context, Entity};
use lens_core::domain::board::{
    Board, BoardItemKind, BoardLayout, DEFAULT_BOARD_ID, DEFAULT_BOARD_NAME, PlacementTarget,
};
use lens_core::domain::ids::{BoardId, BoardItemId, ConnectionId, SessionId};
use lens_core::persist::{BoardStore, PersistError, SqliteBoardStore, StoreMode};

use crate::fleet::store::FleetStore;

pub(crate) const MAX_RETRIES: u32 = 5;

pub(crate) enum Op {
    Load {
        initial: bool,
    },
    PlaceSessions(Vec<(ConnectionId, SessionId)>),
    /// B-4b: idempotent group-collapse write (set the flag to an absolute value).
    SetCollapsed {
        group_id: BoardItemId,
        collapsed: bool,
    },
}

enum OpOutcome {
    Loaded {
        layout: BoardLayout,
        skipped_empty: bool,
        mode: StoreMode,
    },
    Placed {
        layout: BoardLayout,
        skipped_empty: bool,
        mode: StoreMode,
    },
    Wrote {
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
    pub(crate) layout: Arc<BoardLayout>,
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
            layout: Arc::new(default_board_layout()),
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

    /// Production ctor. `store` is the bootstrap-opened handle (Task 8), or `None` if that
    /// open failed; `path` lets `ensure_open`/recovery (re)open. `None` + a bad path →
    /// `LoadFailed` with the real `conn` (not a test ctor).
    pub fn new(
        store: Option<Box<dyn BoardStore + Send>>,
        path: PathBuf,
        conn: ConnectionId,
        fleet: Entity<FleetStore>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::build(store, path, conn, None, fleet, cx)
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

    /// Test ctor with a caller-supplied path and NO pre-opened store: `ensure_open`
    /// opens `path` on the first Load. A good pre-seeded path → Load reads it; a bad
    /// path → open fails → LoadFailed (used by Task 5 recovery tests).
    #[cfg(test)]
    pub(crate) fn for_test_file(
        fleet: Entity<FleetStore>,
        path: std::path::PathBuf,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::build(None, path, ConnectionId::new("conn_test"), None, fleet, cx)
    }

    /// Test ctor with a caller-supplied store (e.g. a fault-injecting double) + the `path`
    /// its recovery reopen will use. On a persistent failure the double is dropped and
    /// `path` is reopened as a plain store (that's how the recovery test heals).
    #[cfg(test)]
    pub(crate) fn for_test_store(
        fleet: Entity<FleetStore>,
        store: Box<dyn BoardStore + Send>,
        path: std::path::PathBuf,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::build(
            Some(store),
            path,
            ConnectionId::new("conn_test"),
            None,
            fleet,
            cx,
        )
    }

    pub(crate) fn run_op(&mut self, op: Op, cx: &mut Context<Self>) {
        self.pending.push_back(op);
        self.pump(cx);
    }

    pub fn layout(&self) -> Arc<BoardLayout> {
        Arc::clone(&self.layout)
    }

    pub fn state(&self) -> ReplicaState {
        self.state
    }

    pub fn banner_dismissed(&self) -> bool {
        self.banner_dismissed
    }

    pub fn dropped_writes(&self) -> u32 {
        self.dropped_writes
    }

    pub fn dismiss_banner(&mut self) {
        self.banner_dismissed = true;
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
                Some(Op::SetCollapsed { .. }) if !self.is_writable() => {
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
            } => {
                self.op_retries = 0;
                self.recovery_in_flight = false;
                // A fresh load is a fresh health assessment — un-dismiss so a NEW incident
                // (recover→Writable→later Degraded, or a fresh Degraded) surfaces its banner
                // instead of being hidden by an earlier dismissal (codex final-review #6).
                self.banner_dismissed = false;
                self.layout = Arc::new(layout);
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
                self.layout = Arc::new(layout);
                self.state = load_state(mode, skipped_empty); // ~always Writable; consistent
                self.reconcile_in_flight = false;
                self.note_place_result(); // suppress stuck keys (Task 6, C1)
                self.reconcile(cx); // re-diff on reply (Task 6)
            }
            OpOutcome::Wrote {
                layout,
                skipped_empty,
                mode,
            } => {
                self.op_retries = 0;
                self.layout = Arc::new(layout);
                self.state = load_state(mode, skipped_empty);
            }
            OpOutcome::Failed { op, err } => {
                self.on_op_failed(op, err, cx); // Task 5
            }
        }
        cx.notify();
        self.pump(cx);
    }

    fn placed_key_strings(&self) -> HashSet<(String, String)> {
        self.layout
            .items
            .iter()
            .filter_map(|i| match &i.kind {
                BoardItemKind::Card { conn, session } => {
                    Some((conn.as_str().to_string(), session.as_str().to_string()))
                }
                _ => None,
            })
            .collect()
    }

    fn missing_keys(&self, cx: &Context<Self>) -> Vec<(ConnectionId, SessionId)> {
        let placed = self.placed_key_strings();
        // snapshot fleet keys, then diff (avoids holding the fleet borrow)
        let mut live: Vec<SessionId> = self.fleet.read(cx).cards.keys().cloned().collect();
        // Deterministic placement order: `cards` is a HashMap (random iteration), so sort by
        // session id before placing — matches the retired build_ephemeral_layout's order and
        // keeps board layout / tests stable across runs (codex Task-8 review).
        live.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        live.into_iter()
            .filter_map(|s| {
                let k = (self.conn.as_str().to_string(), s.as_str().to_string());
                if placed.contains(&k) || self.suppressed.contains(&k) {
                    None
                } else {
                    Some((self.conn.clone(), s))
                }
            })
            .collect()
    }

    fn reconcile(&mut self, cx: &mut Context<Self>) {
        if !self.is_writable() {
            return;
        }
        let missing = self.missing_keys(cx);
        if missing.is_empty() {
            return;
        }
        if self.reconcile_in_flight {
            return; // coalesce; the in-flight place's reply re-diffs
        }
        self.reconcile_in_flight = true;
        self.run_op(Op::PlaceSessions(missing), cx); // pump records last_attempt
    }

    /// C1: an attempted key STILL missing after its place is tombstoned/stuck → suppress it,
    /// so re-diff-on-reply cannot re-enqueue it forever.
    fn note_place_result(&mut self) {
        let placed = self.placed_key_strings();
        for (c, s) in std::mem::take(&mut self.last_attempt) {
            let k = (c.as_str().to_string(), s.as_str().to_string());
            if !placed.contains(&k) {
                self.suppressed.insert(k);
            }
        }
    }

    fn on_op_failed(&mut self, op: Op, err: PersistError, cx: &mut Context<Self>) {
        // The op that just failed is no longer in `pending` (pump moved it out); remember
        // whether it was a write so a terminal failure counts it too (below).
        let current_is_write = !matches!(op, Op::Load { .. });
        // Transient (SQLITE_BUSY/LOCKED beyond busy_timeout): keep the op, back off, retry.
        // SEAM (B-4d): this re-enqueues the WHOLE op. Safe here because B-4a's ops are
        // idempotent (Load; PlaceSessions skips already-present) — a `place then compose-reload`
        // whose *post-commit read* fails transiently just re-runs the place as a no-op. B-4d's
        // CreateGroup is NOT idempotent: a commit followed by a SQLITE_BUSY in the reload would
        // double-create on retry. Before B-4d adds non-idempotent write ops, run_op_inner must
        // signal commit phase (e.g. OpFailure { committed }) so a *post-commit* failure goes
        // Stale (recover by reload) instead of replaying. (Design §8 seam; reconciles M5.)
        if err.is_transient() && self.op_retries < MAX_RETRIES {
            self.op_retries += 1;
            let backoff = Duration::from_millis(50u64 << self.op_retries.min(6)); // 100,200,…,≤3200ms
            self.schedule_retry(op, backoff, cx);
            return;
        }
        // Persistent (or retries exhausted).
        self.op_retries = 0;
        self.reconcile_in_flight = false;
        self.recovery_in_flight = false;
        self.last_attempt.clear();
        match op {
            Op::Load { initial: true } => {
                self.state = ReplicaState::LoadFailed;
                self.layout = Arc::new(default_board_layout()); // never loaded → render empty default, no panic
            }
            Op::Load { initial: false } => {
                // Failed RECOVERY: preserve visible data; a writable store just lost writability.
                if self.state == ReplicaState::Writable {
                    self.state = ReplicaState::Stale;
                } // else keep Degraded/LoadFailed/Stale + existing layout
            }
            Op::PlaceSessions(_) => {
                self.state = ReplicaState::Stale; // keep current layout
            }
            Op::SetCollapsed { .. } => {
                self.state = ReplicaState::Stale; // keep current layout
            }
        }
        // Persistent failure: queued writes won't succeed on replay — drop (banner names them).
        // The op that just failed terminally is itself an unsaved write (it was already
        // popped from `pending`), so count it too (codex final-review Important #2).
        let mut dropped = self
            .pending
            .iter()
            .filter(|o| !matches!(o, Op::Load { .. }))
            .count() as u32;
        if current_is_write {
            dropped += 1;
        }
        self.dropped_writes = self.dropped_writes.saturating_add(dropped);
        self.pending.retain(|o| matches!(o, Op::Load { .. }));
        self.banner_dismissed = false;
        cx.notify();
    }

    fn schedule_retry(&mut self, op: Op, backoff: Duration, cx: &mut Context<Self>) {
        self.pending.push_front(op); // preserve ordering
        self.in_flight = true; // hold the single-in-flight slot across the backoff
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(backoff).await;
            this.update(cx, |this, cx| {
                this.in_flight = false;
                this.pump(cx);
            })
            .ok();
        })
        .detach();
    }

    /// The B-4b/c/d write seam: B-4a exercises it only in tests (no user interactions yet),
    /// so it's dead in the non-test build until the interaction slices call it.
    pub(crate) fn write(&mut self, op: Op, cx: &mut Context<Self>) -> WriteDisposition {
        if !self.is_writable() {
            // Count the rejected user write so the banner names the loss (§5 "never
            // *silently* drop a user write"; write() is the user-write entry point —
            // reconcile places go via run_op). B-4b ships the first real user write.
            self.dropped_writes = self.dropped_writes.saturating_add(1);
            self.banner_dismissed = false; // re-surface the banner on a rejected gesture
            cx.notify();
            return WriteDisposition::Rejected(self.state);
        }
        self.run_op(op, cx);
        WriteDisposition::Accepted
    }

    fn begin_recovery(&mut self, cx: &mut Context<Self>) {
        if self.recovery_in_flight {
            return; // coalesce: at most one recovery in flight (bounded, §5)
        }
        self.recovery_in_flight = true;
        self.run_op(Op::Load { initial: false }, cx); // Load is always allowed, any state
    }

    pub fn retry_recovery(&mut self, cx: &mut Context<Self>) {
        self.banner_dismissed = false;
        self.begin_recovery(cx);
    }

    fn on_fleet_change(&mut self, cx: &mut Context<Self>) {
        if self.is_writable() {
            self.reconcile(cx);
        } else if matches!(
            self.state,
            ReplicaState::Degraded | ReplicaState::LoadFailed | ReplicaState::Stale
        ) {
            self.begin_recovery(cx); // automatic recovery on fleet activity (§5)
        }
        // Loading: initial Load in flight; nothing to do.
    }
}

fn run_op_blocking(slot: &mut StoreSlot, op: Op) -> OpOutcome {
    match run_op_inner(slot, &op) {
        Ok(outcome) => outcome,
        Err(err) => {
            // Drop the handle only on a PERSISTENT error, so recovery reopens fresh. A
            // transient BUSY/LOCKED keeps the (working) connection — the retry runs against
            // it rather than churning a reopen.
            if !err.is_transient() {
                slot.store = None;
            }
            OpOutcome::Failed { op, err }
        }
    }
}

fn run_op_inner(slot: &mut StoreSlot, op: &Op) -> lens_core::persist::Result<OpOutcome> {
    // Recovery (a non-initial Load) is a FRESH reopen (§5): drop any current handle so a
    // Degraded/stale handle can't be silently reused. A persistent failure already dropped
    // it; a Degraded *success* retained it — this is the path that lets a fixed file heal.
    // (A transient failure during recovery re-reopens on each retry — acceptable churn on a
    // rare path.)
    if matches!(op, Op::Load { initial: false }) {
        slot.store = None;
    }
    if slot.store.is_none() {
        slot.store = Some(Box::new(SqliteBoardStore::open(&slot.path)?)); // first-open or recovery
    }
    let Some(store) = slot.store.as_deref() else {
        return Err(PersistError::ReadOnly); // unreachable (just opened); typed, never a panic
    };
    match op {
        Op::Load { .. } => {
            let (layout, skipped_empty, mode) = read_committed(store)?;
            Ok(OpOutcome::Loaded {
                layout,
                skipped_empty,
                mode,
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
        Op::SetCollapsed {
            group_id,
            collapsed,
        } => {
            store.set_collapsed(group_id, *collapsed)?; // persist (idempotent, absolute value)
            let (layout, skipped_empty, mode) = read_committed(store)?;
            Ok(OpOutcome::Wrote {
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
    use std::cell::Cell;
    use std::sync::Arc;

    use gpui::prelude::*; // AppContext etc. for cx.new/update in tests

    use lens_core::domain::board::{BoardItemKind, DEFAULT_BOARD_ID};
    use lens_core::domain::ids::BoardItemId;
    use lens_core::persist::{Loaded, StoreMode};

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
        let fleet = cx.update(test_fleet);
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
        let fleet = cx.update(test_fleet);
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
            // Load-bearing for ORDER (not just "both landed"): single-in-flight commits
            // 'a' before 'b', so 'a' appends at ordinal 0 and 'b' at 1. If the ops applied
            // out of enqueue order, 'b' would take ordinal 0 → this fails.
            let mut cards: Vec<(i32, String)> = r
                .layout()
                .items
                .iter()
                .filter_map(|i| match &i.kind {
                    BoardItemKind::Card { session, .. } => {
                        Some((i.ordinal, session.as_str().to_string()))
                    }
                    _ => None,
                })
                .collect();
            cards.sort_by_key(|(ord, _)| *ord);
            let sessions: Vec<String> = cards.into_iter().map(|(_, s)| s).collect();
            assert_eq!(sessions, vec!["a".to_string(), "b".to_string()]);
        });
    }

    // Load-bearing for the Loaded arm applying `self.layout` (codex Task-4 review): the
    // replica starts with the empty default board, so an empty-store Load can't prove the
    // loaded layout was applied. Seed a distinctive card on disk BEFORE the replica opens
    // it; if apply_outcome's Loaded arm omitted `self.layout = layout`, this fails.
    #[gpui::test]
    async fn load_reads_persisted_card(cx: &mut gpui::TestAppContext) {
        let fleet = cx.update(test_fleet);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("board.db");
        {
            let store = SqliteBoardStore::open(&path).unwrap();
            store
                .place_session(
                    &ConnectionId::new("conn_test"),
                    &SessionId::new("persisted_x"),
                    &PlacementTarget {
                        board_id: None,
                        parent_item_id: None,
                        ordinal: None,
                    },
                )
                .unwrap();
        }
        let replica = cx
            .update(|cx| cx.new(|cx| BoardReplica::for_test_file(fleet.clone(), path.clone(), cx)));
        cx.run_until_parked();
        replica.read_with(cx, |r, _| {
            assert_eq!(r.state(), ReplicaState::Writable);
            let sessions: Vec<String> = r
                .layout()
                .items
                .iter()
                .filter_map(|i| match &i.kind {
                    BoardItemKind::Card { session, .. } => Some(session.as_str().to_string()),
                    _ => None,
                })
                .collect();
            assert_eq!(sessions, vec!["persisted_x".to_string()]);
        });
        // keep the tempdir alive through the assertion (for_test_file holds no _tempdir)
        drop(dir);
    }

    #[gpui::test]
    async fn failed_initial_load_seeds_default_board(cx: &mut gpui::TestAppContext) {
        let fleet = cx.update(test_fleet);
        let replica = cx.update(|cx| {
            cx.new(|cx| BoardReplica::for_test_file(fleet.clone(), "/dev/null/nope.db".into(), cx))
        });
        cx.run_until_parked();
        replica.read_with(cx, |r, _| {
            assert_eq!(r.state(), ReplicaState::LoadFailed);
            assert_eq!(
                r.layout().default_board_id().unwrap().as_str(),
                DEFAULT_BOARD_ID
            );
        });
    }

    #[gpui::test]
    async fn set_collapsed_round_trips_and_persists(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("b.db");
        // Seed a group into a real store on `path`, capture its id, drop the handle.
        let gid = {
            let store = SqliteBoardStore::open(&path).unwrap();
            store
                .create_group(&BoardId::new(DEFAULT_BOARD_ID), None, 0, "G")
                .unwrap()
        };

        let fleet = cx.update(test_fleet);
        let replica = cx
            .update(|cx| cx.new(|cx| BoardReplica::for_test_file(fleet.clone(), path.clone(), cx)));
        cx.run_until_parked(); // Load the seeded group (collapsed == false).

        let is_collapsed = |r: &BoardReplica| {
            matches!(
                r.layout().item(&gid).map(|it| &it.kind),
                Some(BoardItemKind::Group {
                    collapsed: true,
                    ..
                })
            )
        };
        replica.read_with(cx, |r, _| assert!(!is_collapsed(r), "seeded expanded"));

        replica.update(cx, |r, cx| {
            r.write(
                Op::SetCollapsed {
                    group_id: gid.clone(),
                    collapsed: true,
                },
                cx,
            );
        });
        cx.run_until_parked();
        replica.read_with(cx, |r, _| {
            assert_eq!(r.state(), ReplicaState::Writable);
            assert!(is_collapsed(r), "flag flipped in the committed layout");
        });

        // Reopen the same path in a fresh replica — the collapse persisted.
        let fleet2 = cx.update(test_fleet);
        let replica2 =
            cx.update(|cx| cx.new(|cx| BoardReplica::for_test_file(fleet2, path.clone(), cx)));
        cx.run_until_parked();
        replica2.read_with(cx, |r, _| {
            assert!(is_collapsed(r), "persisted across reopen")
        });
    }

    #[gpui::test]
    async fn set_collapsed_refused_when_non_writable(cx: &mut gpui::TestAppContext) {
        // A LoadFailed replica (bad path) refuses the write (banner honesty).
        let fleet = cx.update(test_fleet);
        let replica = cx.update(|cx| {
            cx.new(|cx| BoardReplica::for_test_file(fleet, "/dev/null/nope.db".into(), cx))
        });
        cx.run_until_parked();
        let before = replica.read_with(cx, |r, _| {
            assert_eq!(r.state(), ReplicaState::LoadFailed);
            r.dropped_writes()
        });
        let disp = replica.update(cx, |r, cx| {
            r.write(
                Op::SetCollapsed {
                    group_id: BoardItemId::new("g_x"),
                    collapsed: true,
                },
                cx,
            )
        });
        assert!(matches!(
            disp,
            WriteDisposition::Rejected(ReplicaState::LoadFailed)
        ));
        // Contract (§5): a rejected user write is COUNTED so the banner names the loss
        // (codex final-review Important #2). It is never silently dropped.
        replica.read_with(cx, |r, _| assert_eq!(r.dropped_writes(), before + 1));
    }

    #[gpui::test]
    async fn write_rejected_when_non_writable(cx: &mut gpui::TestAppContext) {
        let fleet = cx.update(test_fleet);
        let replica = cx.update(|cx| {
            cx.new(|cx| BoardReplica::for_test_file(fleet.clone(), "/dev/null/nope.db".into(), cx))
        });
        cx.run_until_parked(); // → LoadFailed
        let d = replica.update(cx, |r, cx| {
            r.write(
                Op::PlaceSessions(vec![(r.conn.clone(), SessionId::new("x"))]),
                cx,
            )
        });
        assert_eq!(d, WriteDisposition::Rejected(ReplicaState::LoadFailed));
        replica.read_with(cx, |r, _| assert!(r.pending.is_empty()));
    }

    // Fault-injecting BoardStore: fails the first `fail_loads` load_layout calls with
    // `err`, then delegates to a real store. `Cell` under &self is safe — the replica's
    // store mutex serializes access. Send (Cell<u32> + SqliteBoardStore + fn ptr all Send);
    // the mutex supplies Sync.
    struct FlakyStore {
        inner: SqliteBoardStore,
        fail_loads: Cell<u32>,
        fail_places: Cell<u32>, // leading place_sessions calls that fail with `err`
        err: fn() -> PersistError,
        mode: StoreMode,  // what mode() reports (simulate a Degraded handle)
        place_noop: bool, // place_sessions succeeds but places nothing (simulate a tombstoned/unplaceable key)
    }

    fn busy_err() -> PersistError {
        PersistError::synthetic_busy()
    }

    impl BoardStore for FlakyStore {
        fn mode(&self) -> StoreMode {
            self.mode
        }
        fn load_layout(&self) -> lens_core::persist::Result<Loaded<BoardLayout>> {
            let n = self.fail_loads.get();
            if n > 0 {
                self.fail_loads.set(n - 1);
                return Err((self.err)());
            }
            self.inner.load_layout()
        }
        fn place_session(
            &self,
            conn: &ConnectionId,
            session: &SessionId,
            target: &PlacementTarget,
        ) -> lens_core::persist::Result<()> {
            self.inner.place_session(conn, session, target)
        }
        fn remove_session(
            &self,
            conn: &ConnectionId,
            session: &SessionId,
        ) -> lens_core::persist::Result<()> {
            self.inner.remove_session(conn, session)
        }
        fn place_sessions(
            &self,
            placements: &[(ConnectionId, SessionId)],
            target: &PlacementTarget,
        ) -> lens_core::persist::Result<()> {
            if self.place_noop {
                return Ok(()); // "committed" but placed nothing → the key stays missing
            }
            let n = self.fail_places.get();
            if n > 0 {
                self.fail_places.set(n - 1);
                return Err((self.err)());
            }
            self.inner.place_sessions(placements, target)
        }
        fn create_group(
            &self,
            board_id: &BoardId,
            parent_item_id: Option<BoardItemId>,
            ordinal: i32,
            name: &str,
        ) -> lens_core::persist::Result<BoardItemId> {
            self.inner
                .create_group(board_id, parent_item_id, ordinal, name)
        }
        fn move_item(
            &self,
            item_id: &BoardItemId,
            new_board_id: &BoardId,
            new_parent: Option<BoardItemId>,
            new_ordinal: i32,
        ) -> lens_core::persist::Result<()> {
            self.inner
                .move_item(item_id, new_board_id, new_parent, new_ordinal)
        }
        fn ungroup(&self, group_id: &BoardItemId) -> lens_core::persist::Result<()> {
            self.inner.ungroup(group_id)
        }
        fn rename(&self, item_id: &BoardItemId, name: &str) -> lens_core::persist::Result<()> {
            self.inner.rename(item_id, name)
        }
        fn archive(&self, item_id: &BoardItemId) -> lens_core::persist::Result<()> {
            self.inner.archive(item_id)
        }
        fn set_collapsed(
            &self,
            group_id: &BoardItemId,
            collapsed: bool,
        ) -> lens_core::persist::Result<()> {
            self.inner.set_collapsed(group_id, collapsed)
        }
        fn set_color(&self, group_id: &BoardItemId, token: &str) -> lens_core::persist::Result<()> {
            self.inner.set_color(group_id, token)
        }
    }

    // A PERSISTENT initial-load failure → LoadFailed; the handle is dropped, so
    // retry_recovery reopens the good `path` as a plain store → Writable. Load-bearing:
    // if Load were gated in LoadFailed, or recovery didn't reopen, it stays LoadFailed.
    #[gpui::test]
    async fn recovery_reopens_degraded_handle(cx: &mut gpui::TestAppContext) {
        let fleet = cx.update(test_fleet);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("board.db");
        // The file itself is a healthy ReadWrite store, but this handle REPORTS Degraded
        // while its load SUCCEEDS — so the replica goes Degraded and RETAINS the handle.
        // Recovery must force a fresh reopen of `path` (real ReadWrite) to reach Writable.
        // Load-bearing for the reopen contract: without the force-reopen in run_op_inner,
        // recovery reuses this Degraded handle and stays Degraded forever.
        let inner = SqliteBoardStore::open(&path).unwrap();
        let flaky: Box<dyn BoardStore + Send> = Box::new(FlakyStore {
            inner,
            fail_loads: Cell::new(0),
            fail_places: Cell::new(0), // load succeeds…
            err: || PersistError::ReadOnly,
            mode: StoreMode::ReadOnlyDegraded, // …but reported Degraded → handle retained
            place_noop: false,
        });
        let replica = cx.update(|cx| {
            cx.new(|cx| BoardReplica::for_test_store(fleet.clone(), flaky, path.clone(), cx))
        });
        cx.run_until_parked();
        replica.read_with(cx, |r, _| assert_eq!(r.state(), ReplicaState::Degraded));

        replica.update(cx, |r, cx| r.retry_recovery(cx));
        cx.run_until_parked();
        replica.read_with(cx, |r, _| assert_eq!(r.state(), ReplicaState::Writable));
        drop(dir);
    }

    // Retry cap: a transient error that never clears exhausts MAX_RETRIES and falls through
    // to the persistent branch (LoadFailed) — no infinite retry loop.
    #[gpui::test]
    async fn transient_exhausts_retries_then_load_failed(cx: &mut gpui::TestAppContext) {
        let fleet = cx.update(test_fleet);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("board.db");
        let inner = SqliteBoardStore::open(&path).unwrap();
        let flaky: Box<dyn BoardStore + Send> = Box::new(FlakyStore {
            inner,
            fail_loads: Cell::new(u32::MAX), // always BUSY
            fail_places: Cell::new(0),
            err: busy_err,
            mode: StoreMode::ReadWrite,
            place_noop: false,
        });
        let replica = cx.update(|cx| {
            cx.new(|cx| BoardReplica::for_test_store(fleet.clone(), flaky, path.clone(), cx))
        });
        cx.run_until_parked(); // Load #1 → BUSY → op_retries=1
        for _ in 0..(MAX_RETRIES + 1) {
            cx.executor()
                .advance_clock(std::time::Duration::from_millis(5000)); // ≥ max backoff (50<<6)
            cx.run_until_parked();
        }
        replica.read_with(cx, |r, _| {
            // the (MAX_RETRIES+1)th failure falls through to persistent → LoadFailed.
            assert_eq!(r.state(), ReplicaState::LoadFailed);
            assert_eq!(r.op_retries, 0);
        });
        drop(dir);
    }

    // A TRANSIENT (BUSY) initial-load failure retries with backoff (keeping the handle),
    // then succeeds. Load-bearing: op_retries climbs per attempt and resets on success,
    // and the eventually-loaded layout carries the seeded card.
    #[gpui::test]
    async fn transient_busy_retries_then_loads(cx: &mut gpui::TestAppContext) {
        let fleet = cx.update(test_fleet);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("board.db");
        let inner = SqliteBoardStore::open(&path).unwrap();
        inner
            .place_session(
                &ConnectionId::new("conn_test"),
                &SessionId::new("x"),
                &PlacementTarget {
                    board_id: None,
                    parent_item_id: None,
                    ordinal: None,
                },
            )
            .unwrap();
        let flaky: Box<dyn BoardStore + Send> = Box::new(FlakyStore {
            inner,
            fail_loads: Cell::new(2),
            fail_places: Cell::new(0),
            err: busy_err,
            mode: StoreMode::ReadWrite,
            place_noop: false,
        });
        let replica = cx.update(|cx| {
            cx.new(|cx| BoardReplica::for_test_store(fleet.clone(), flaky, path.clone(), cx))
        });
        cx.run_until_parked(); // Load #1 → BUSY → backoff 100ms
        replica.read_with(cx, |r, _| {
            assert_eq!(r.state(), ReplicaState::Loading);
            assert_eq!(r.op_retries, 1);
        });
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(100)); // 50<<1
        cx.run_until_parked(); // Load #2 → BUSY → backoff 200ms
        replica.read_with(cx, |r, _| assert_eq!(r.op_retries, 2));
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(200)); // 50<<2
        cx.run_until_parked(); // Load #3 → success
        replica.read_with(cx, |r, _| {
            assert_eq!(r.state(), ReplicaState::Writable);
            assert_eq!(r.op_retries, 0);
            assert!(r.layout().items.iter().any(|i| matches!(
                &i.kind,
                BoardItemKind::Card { session, .. } if session.as_str() == "x"
            )));
        });
        drop(dir);
    }

    #[gpui::test]
    async fn fleet_session_gets_placed_and_persists(cx: &mut gpui::TestAppContext) {
        let fleet = cx.update(test_fleet);
        let replica = cx.update(|cx| cx.new(|cx| BoardReplica::for_test(fleet.clone(), cx)));
        cx.run_until_parked();
        fleet.update(cx, |f, cx| {
            f.spawn_fake_session(SessionId::new("s1"), cx);
        });
        cx.run_until_parked();
        replica.read_with(cx, |r, _| {
            let placed: Vec<_> = r
                .layout()
                .items
                .iter()
                .filter_map(|i| match &i.kind {
                    BoardItemKind::Card { session, .. } => Some(session.as_str().to_string()),
                    _ => None,
                })
                .collect();
            assert_eq!(placed, vec!["s1".to_string()]);
        });
    }

    #[gpui::test]
    async fn double_reconcile_idempotent(cx: &mut gpui::TestAppContext) {
        let fleet = cx.update(test_fleet);
        let replica = cx.update(|cx| cx.new(|cx| BoardReplica::for_test(fleet.clone(), cx)));
        cx.run_until_parked();
        fleet.update(cx, |f, cx| {
            f.spawn_fake_session(SessionId::new("s1"), cx);
        });
        cx.run_until_parked();
        replica.update(cx, |r, cx| r.reconcile(cx));
        cx.run_until_parked();
        replica.read_with(cx, |r, _| {
            let n = r
                .layout()
                .items
                .iter()
                .filter(|i| matches!(i.kind, BoardItemKind::Card { .. }))
                .count();
            assert_eq!(n, 1);
        });
    }

    // C1: a fleet card whose key can never be placed (place_noop simulates a tombstoned/
    // unplaceable session — FleetStore never drops it) must be SUPPRESSED after one attempt,
    // else re-diff-on-reply re-enqueues it forever and run_until_parked never settles. This
    // test PASSING (settling + suppressed) is the load-bearing proof: without note_place_result
    // the pump loops and this hangs.
    #[gpui::test]
    async fn unplaceable_fleet_key_settles_no_loop(cx: &mut gpui::TestAppContext) {
        let fleet = cx.update(test_fleet);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("board.db");
        let inner = SqliteBoardStore::open(&path).unwrap();
        let flaky: Box<dyn BoardStore + Send> = Box::new(FlakyStore {
            inner,
            fail_loads: Cell::new(0),
            fail_places: Cell::new(0),
            err: || PersistError::ReadOnly,
            mode: StoreMode::ReadWrite,
            place_noop: true, // reconcile's place "succeeds" but never actually places the key
        });
        let replica = cx.update(|cx| {
            cx.new(|cx| BoardReplica::for_test_store(fleet.clone(), flaky, path.clone(), cx))
        });
        cx.run_until_parked(); // initial Load → Writable
        fleet.update(cx, |f, cx| {
            f.spawn_fake_session(SessionId::new("s_dead"), cx);
        });
        cx.run_until_parked(); // reconcile → place(no-op) → re-diff → suppress → SETTLE

        replica.read_with(cx, |r, _| {
            assert!(
                !r.in_flight && r.pending.is_empty(),
                "reconcile settled — no infinite re-diff loop"
            );
            assert!(
                r.suppressed
                    .contains(&("conn_test".to_string(), "s_dead".to_string())),
                "the unplaceable key was suppressed after one attempt"
            );
        });
        drop(dir);
    }

    // Production new(None, bad path) → LoadFailed with the REAL conn (not a test ctor).
    #[gpui::test]
    async fn new_with_none_store_and_bad_path_is_load_failed(cx: &mut gpui::TestAppContext) {
        let fleet = cx.update(test_fleet);
        let replica = cx.update(|cx| {
            cx.new(|cx| {
                BoardReplica::new(
                    None,
                    "/dev/null/nope.db".into(),
                    ConnectionId::new("lens-app"),
                    fleet.clone(),
                    cx,
                )
            })
        });
        cx.run_until_parked();
        replica.read_with(cx, |r, _| {
            assert_eq!(r.state(), ReplicaState::LoadFailed);
            assert_eq!(r.conn.as_str(), "lens-app");
        });
    }

    // A card arriving WHILE a reconcile place is in-flight coalesces; the reply's re-diff
    // must catch it. Held in-flight by failing s1's first place transiently (backoff window).
    // Load-bearing for re-diff-on-reply: without it, s2 is stranded and this fails.
    #[gpui::test]
    async fn coalesced_late_card_caught_by_re_diff(cx: &mut gpui::TestAppContext) {
        let fleet = cx.update(test_fleet);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("board.db");
        let inner = SqliteBoardStore::open(&path).unwrap();
        let flaky: Box<dyn BoardStore + Send> = Box::new(FlakyStore {
            inner,
            fail_loads: Cell::new(0),
            fail_places: Cell::new(1), // s1's first place fails (BUSY) → held in backoff
            err: busy_err,
            mode: StoreMode::ReadWrite,
            place_noop: false,
        });
        let replica = cx.update(|cx| {
            cx.new(|cx| BoardReplica::for_test_store(fleet.clone(), flaky, path.clone(), cx))
        });
        cx.run_until_parked(); // Load → Writable

        fleet.update(cx, |f, cx| {
            f.spawn_fake_session(SessionId::new("s1"), cx);
        });
        cx.run_until_parked(); // reconcile → place([s1]) → BUSY → backoff (in-flight, reconcile_in_flight held)
        replica.read_with(cx, |r, _| {
            assert!(r.in_flight, "s1's place is held in backoff");
            assert!(r.reconcile_in_flight, "coalescing gate is up");
        });

        // s2 arrives DURING the in-flight place → its reconcile coalesces (does not enqueue).
        fleet.update(cx, |f, cx| {
            f.spawn_fake_session(SessionId::new("s2"), cx);
        });
        cx.run_until_parked();

        // release the backoff: s1 place retries + succeeds → Placed re-diff catches s2.
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(500));
        cx.run_until_parked();

        replica.read_with(cx, |r, _| {
            let placed: HashSet<String> = r
                .layout()
                .items
                .iter()
                .filter_map(|i| match &i.kind {
                    BoardItemKind::Card { session, .. } => Some(session.as_str().to_string()),
                    _ => None,
                })
                .collect();
            assert!(
                placed.contains("s1") && placed.contains("s2"),
                "re-diff on reply caught the coalesced late card s2 (placed: {placed:?})"
            );
            assert!(!r.in_flight && !r.reconcile_in_flight && r.pending.is_empty());
        });
        drop(dir);
    }

    // A PERSISTENT place failure (reconcile's PlaceSessions) → Stale, and the last loaded
    // layout is PRESERVED (not blanked) — codex final-review I4.
    #[gpui::test]
    async fn persistent_place_failure_goes_stale_preserving_layout(cx: &mut gpui::TestAppContext) {
        let fleet = cx.update(test_fleet);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("board.db");
        let inner = SqliteBoardStore::open(&path).unwrap();
        let flaky: Box<dyn BoardStore + Send> = Box::new(FlakyStore {
            inner,
            fail_loads: Cell::new(0),
            fail_places: Cell::new(1), // the reconcile place fails once…
            err: || PersistError::ReadOnly, // …persistently (non-transient)
            mode: StoreMode::ReadWrite,
            place_noop: false,
        });
        let replica = cx.update(|cx| {
            cx.new(|cx| BoardReplica::for_test_store(fleet.clone(), flaky, path.clone(), cx))
        });
        cx.run_until_parked(); // Load → Writable
        replica.read_with(cx, |r, _| assert_eq!(r.state(), ReplicaState::Writable));

        fleet.update(cx, |f, cx| {
            f.spawn_fake_session(SessionId::new("s1"), cx);
        });
        cx.run_until_parked(); // reconcile → place fails persistently → Stale

        replica.read_with(cx, |r, _| {
            assert_eq!(r.state(), ReplicaState::Stale);
            assert!(!r.is_writable());
            // layout preserved (default board still there), not blanked:
            assert_eq!(
                r.layout().default_board_id().unwrap().as_str(),
                DEFAULT_BOARD_ID
            );
            // The write that failed TERMINALLY is counted (codex final-review Important #2):
            // this exercises the op-agnostic `current_is_write` accounting in on_op_failed.
            assert_eq!(r.dropped_writes(), 1);
        });
        drop(dir);
    }
}
