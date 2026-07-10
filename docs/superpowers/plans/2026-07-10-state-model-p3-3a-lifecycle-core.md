# State-Model P3-3a — Lifecycle Core Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the session lifecycle core — disk-canonical transcript commit, actor-owned forward catch-up, a transport-only reader, and command-driven sleep/wake — so a session auto-sleeps when quiescent and wakes by respawn with its transcript intact.

**Architecture:** The pure reducer keeps mutating a *small* working set (`state.items`) but no longer emits item deltas; the actor commits **terminal** items to disk (`TranscriptStore`) in a contiguous prefix, prunes them from RAM, and emits a `TranscriptAdvanced { committed_ordinal }` watermark. On spawn and on reconnect the actor is the **sole** `/items` fetcher — a forward, `order=asc` catch-up from the disk frontier — so the `lens-client` reader is demoted to transport recovery only. Sleep is a `SessionCommand`; wake is a fresh respawn from disk driven by a skeletal `FleetScheduler` seam.

**Tech Stack:** Rust, `crossbeam-channel` (actor ingest `Select`), `async-channel` (actor→replica bridge), `rusqlite` (bundled/WAL), gpui (foreground replica), `smallvec`.

## Global Constraints

- **`generated.rs` is untouched** — codegen output, never hand-edit.
- **No `Value` to consumers** — lens-client read surfaces stay typed.
- **`cargo run -p xtask -- gate` is the green bar:** fmt → clippy (feature matrix, `-D warnings`) → test → `cargo bench --no-run` → drift. Spikes opt out.
- **Every task ends green** on `xtask gate` (or, mid-task, the crate's `cargo test` + `cargo clippy --all-targets -- -D warnings`).
- **TDD**: failing test → run-it-fails → minimal impl → run-it-passes → commit. One logical change per commit.
- **Cross-family review is MANDATORY on Tasks 3, 4, 5** — author with `cursor-delegate`/composer-2.5, review with **grok-4.5 via `cursor-delegate`** (a family other than the author). Tasks 3 (actor mutation of merged P3-1 code), 4 (temporal catch-up loop), and 5 (subtractive edit to the hardened `lens-client` crate) are the seams.
- **Spec SSOT:** `docs/superpowers/specs/2026-07-08-state-model-engine-design.md` §2.3 (D19–D23). Where this plan and the spec disagree, the spec wins — surface the conflict.
- **`i64` ordinal stays** — negative/anchored ordinals (D22 never-seen-huge scroll-back) must remain representable; nothing here may foreclose a negative-ordinal prepend.

---

## Orientation — the merged surfaces this plan revises

Read these before Task 3; they are P3-1/P3-2 code this plan deliberately changes.

- **`crates/lens-core/src/reduce/update.rs`** — `enum StreamUpdate`. Delete `ItemAppended`, `ItemUpdated`; add `TranscriptAdvanced`.
- **`crates/lens-core/src/reduce/items.rs`** — `push_item` (line ~176) emits the item deltas today; it will emit none but still mutate `state.items`. `map_item` carries `FunctionCall { status }`.
- **`crates/lens-core/src/reduce/mod.rs`** — `reduce()` dispatch; `OutputItemDone` (line ~89), `Completed` (line ~123) call `push_item`/`finalize_*`.
- **`crates/lens-core/src/actor/runloop.rs`** — `run()` event arm (line ~282) calls `persist_write_through` (line ~437, computes ordinal positionally from `state.items.len()` — **this positional scheme dies**) then `coalesce` (line ~493). `SessionCommand` (line ~23). Transport locals `transport`/`reconcile_in_flight`/`parked` (line ~163).
- **`crates/lens-store/src/lib.rs`** — `apply()` (line ~10) has the `ItemAppended`/`ItemUpdated` copy-assign arms to delete.
- **`crates/lens-core/src/persist/transcript.rs`** — `SqliteTranscriptStore`; `upsert_item` (line ~95) sets `ordinal=excluded.ordinal` on conflict. `load_items` orders by `ordinal`.
- **`crates/lens-core/src/persist/control.rs`** — `upsert_session` (line ~78); `created_at=excluded.created_at` (line ~119) is the D15 clobber.
- **`crates/lens-core/src/reduce/snapshot.rs`** — `fold_snapshot` (line ~20) never assigns `state.created_at` (the D15 fold defect).
- **`crates/lens-client/src/reconnect.rs`** — `trait Reopen` (3 methods: `open_stream`/`snapshot`/`items`), `HttpReopener`, `items_to_replay`. Task 5 shrinks this 3→2.
- **`crates/lens-client/src/stream/reader.rs`** — `bootstrap` (line ~113) and `reconnect` (line ~241) call `reopener.items()` + `items_to_replay`. Task 5 deletes those calls.
- **`crates/lens-client/src/actor`… no** — the actor's HTTP surface is `crates/lens-core/src/actor/api.rs` (`trait SessionApi`, only `send_event` today). Task 4 adds `fetch_items`.
- **Reference:** `SessionEventInput::StopSession` already exists (`lens-client/src/sessions.rs:797`) — the actor stops a session by `api.send_event(&id, &SessionEventInput::StopSession)`; **no new trait method for stop.**

---

### Task 1: D15 — `created_at` fold + first-non-zero persist guard; delete vestigial `last_seen_seq`

Independent, small. `created_at` is epoch **seconds** on `SessionState` (distinct from `Item.created_at` millis). This task also **deletes `SessionState.last_seen_seq`** — verified vestigial (no producer, no consumer; obsoleted by D19's item-id frontier resume; `/items` has no seq column and `/stream` won't replay-by-seq, confirmed against omnigent `31669e1b`). The lens-client reader's OWN `last_seen_seq` local (live overlap dedup) is a separate thing and stays untouched.

**Files:**
- Modify: `crates/lens-core/src/reduce/snapshot.rs` (`fold_snapshot`, ~line 20)
- Modify: `crates/lens-core/src/persist/control.rs` (`upsert_session`, ~line 119; drop `last_seen_seq` bind + conflict clause)
- Modify: `crates/lens-core/src/domain/session.rs` (remove the `last_seen_seq` field + its init)
- Modify: `crates/lens-core/src/persist/schema.rs` (drop the `last_seen_seq` column from the sessions DDL)
- Modify: `crates/lens-core/src/persist/map.rs` (drop `last_seen_seq` from INSERT/SELECT column lists + the row-map at index 30)
- Test: the above files' existing `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: `SessionSnapshot::created_at(&self) -> i64` (lens-client `sessions.rs:178`).
- Produces: `fold_snapshot` sets `state.created_at`; `upsert_session` preserves a non-zero stored `created_at`; `SessionState` no longer has `last_seen_seq`.

**Schema note:** dropping the column from the DDL means fresh DBs are clean; an existing dev DB keeps a harmless orphan `last_seen_seq` column (never read/written). No hard migration and no `SCHEMA_VERSION` bump required pre-release — the INSERT/SELECT simply stop referencing it. If `cargo test` surfaces a column-count mismatch against a checked-in fixture DB, delete/regenerate the fixture.

- [ ] **Step 1: Failing test — the fold sets `created_at`.** In `snapshot.rs` tests, add:

```rust
#[test]
fn fold_snapshot_sets_created_at_from_wire() {
    let mut s = crate::reduce::testutil::fresh_state();
    assert_eq!(s.created_at, 0);
    let snap = crate::reduce::testutil::snapshot_fixture(serde_json::json!({
        "id": "conv_1", "status": "running", "agent_id": "ag_1",
        "created_at": 1_700_000_000, "items": []
    }));
    fold_snapshot(&mut s, &snap);
    assert_eq!(s.created_at, 1_700_000_000);
}
```

- [ ] **Step 2: Run — fails** (`created_at` still 0).
Run: `cargo test -p lens-core fold_snapshot_sets_created_at_from_wire`
Expected: FAIL (assert 0 == 1_700_000_000).

- [ ] **Step 3: Implement the fold.** In `fold_snapshot`, alongside the other scalar assignments (e.g. after `state.archived = snap.archived();`):

```rust
state.created_at = snap.created_at();
```

- [ ] **Step 4: Run — passes.** `cargo test -p lens-core fold_snapshot_sets_created_at_from_wire` → PASS.

- [ ] **Step 5: Failing test — the persist guard.** In `control.rs` tests, add:

```rust
#[test]
fn upsert_session_keeps_existing_nonzero_created_at() {
    let d = tempfile::tempdir().unwrap();
    let store = SqliteControlStore::open(&d.path().join("lens.db")).unwrap();
    store.upsert_connection(&conn_record()).unwrap(); // existing test helper

    let mut s = session_fixture(); // existing helper; ensure conn/id match conn_record()
    s.created_at = 1_700_000_000;
    store.upsert_session(&s, 1).unwrap();

    // A later actor upsert with a not-yet-bootstrapped created_at=0 must NOT clobber.
    s.created_at = 0;
    store.upsert_session(&s, 2).unwrap();

    let loaded = store
        .load_session(&s.connection_id, &s.id)
        .unwrap()
        .unwrap();
    assert_eq!(loaded.created_at, 1_700_000_000, "non-zero created_at preserved");
}
```

*(If `session_fixture()`/`conn_record()` helpers differ in the file, reuse whatever the existing `control.rs` tests use to build a `SessionState` + `ConnectionRecord`; the shape above is the contract.)*

- [ ] **Step 6: Run — fails** (`excluded.created_at` clobbers to 0).
Run: `cargo test -p lens-core upsert_session_keeps_existing_nonzero_created_at` → FAIL (0 == 1_700_000_000).

- [ ] **Step 7: Implement the guard.** In `upsert_session`'s `ON CONFLICT … DO UPDATE SET` clause, replace `created_at=excluded.created_at` with:

```sql
created_at=CASE WHEN sessions.created_at != 0 THEN sessions.created_at ELSE excluded.created_at END,
```

- [ ] **Step 8: Run — passes + no regressions.** `cargo test -p lens-core` → PASS.

- [ ] **Step 9: Delete `SessionState.last_seen_seq` (compile-break first).** Remove the field (`domain/session.rs:75`) and its `SessionState::new` init (`:121`). This breaks `persist/control.rs` (upsert bind + conflict clause), `persist/map.rs` (INSERT/SELECT lists + `st.last_seen_seq = ...` row-map at index 30), and any test constructing it.

- [ ] **Step 10: Drop the persist bindings.** In `control.rs` `upsert_session`: remove `last_seen_seq` from the column list, the `VALUES` bind (`s.last_seen_seq.map(...)`), and the `last_seen_seq=excluded.last_seen_seq` conflict clause. In `map.rs`: remove it from the shared column-list constant and the row-map (re-index the columns after it — `updated_at` etc. shift down by one). In `schema.rs`: delete the `last_seen_seq INTEGER,` DDL line.

- [ ] **Step 11: Run — compiles + passes.** `cargo test -p lens-core` → PASS. Fix any test that set/read `last_seen_seq` (delete those lines). Confirm the P2 persist round-trip tests still pass with the narrower column set.

- [ ] **Step 12: Commit.**

```bash
git add crates/lens-core/src/reduce/snapshot.rs crates/lens-core/src/persist/ crates/lens-core/src/domain/session.rs
git commit -m "fix(state-model): D15 created_at fold + guard; delete vestigial last_seen_seq (D19 makes item-id frontier the resume cursor)"
```

---

### Task 2: Pure quiescence predicates (`transient_work_outstanding` + `is_quiesced` helper)

Pure, unit-testable, no actor thread. Foundation for Task 6 sleep. Verify first that none of these exist yet (only the `runloop.rs:~141` `#[allow(unused_assignments)]` quiescence-gate comment).

**Files:**
- Modify: `crates/lens-core/src/domain/session.rs` (add method on `SessionState`)
- Modify: `crates/lens-core/src/actor/runloop.rs` (add free `is_quiesced` helper + test)

**Interfaces:**
- Produces: `SessionState::transient_work_outstanding(&self) -> bool`; `fn is_quiesced(state: &SessionState, transport: &ActorTransport, reconcile_in_flight: bool) -> bool` (module-private in `runloop.rs`, consumed by Task 6).

- [ ] **Step 1: Failing tests — the predicate.** In `session.rs` tests:

```rust
#[test]
fn transient_work_outstanding_flags() {
    let mut s = SessionState::new(
        ConnectionId::new("c"), SessionId::new("conv"), AgentId::new("ag"),
    );
    assert!(!s.transient_work_outstanding(), "fresh idle session is quiet");

    s.status = crate::domain::scalars::SessionStatusValue::Running;
    assert!(s.transient_work_outstanding(), "running = work");
    s.status = crate::domain::scalars::SessionStatusValue::Idle;

    s.stream.open_reasoning = Some(Default::default());
    assert!(s.transient_work_outstanding(), "open reasoning = work");
    s.stream.open_reasoning = None;

    s.pending_user.push(crate::domain::controls::PendingUserMessage {
        pending_id: "lens_pend_1".into(), server_pending_id: None,
        store_item_id: None, content: "x".into(), created_at: 0,
    });
    assert!(s.transient_work_outstanding(), "unacked send = work");
}
```

- [ ] **Step 2: Run — fails** (method not found).
Run: `cargo test -p lens-core transient_work_outstanding_flags` → FAIL (no method).

- [ ] **Step 3: Implement.** In `impl SessionState`:

```rust
/// True while the session has in-flight work that must NOT be interrupted by an
/// auto-sleep (§2.3 D21). Pure; the actor ANDs this with transport liveness.
pub fn transient_work_outstanding(&self) -> bool {
    use crate::domain::scalars::SessionStatusValue::*;
    self.stream.open_message.is_some()
        || self.stream.open_reasoning.is_some()
        || !self.stream.unpaired_calls.is_empty()
        || !self.pending_user.is_empty()
        || !self.pending_elicitations.is_empty()
        || matches!(self.status, Running | Launching | Waiting)
}
```

- [ ] **Step 4: Run — passes.** `cargo test -p lens-core transient_work_outstanding_flags` → PASS.

- [ ] **Step 5: Failing test — the actor helper.** In `runloop.rs` tests:

```rust
#[test]
fn is_quiesced_requires_quiet_connected_and_settled() {
    let s = fresh_state(); // idle
    assert!(is_quiesced(&s, &ActorTransport::Connected, false));
    assert!(!is_quiesced(&s, &ActorTransport::Connected, true), "mid-reconcile");
    assert!(
        !is_quiesced(&s, &ActorTransport::Parked { reason: ParkReason::Unauthorized }, false),
        "parked is not quiesced"
    );
    let mut busy = fresh_state();
    busy.status = WireStatus::Running.into(); // or set domain status Running directly
    assert!(!is_quiesced(&busy, &ActorTransport::Connected, false));
}
```

*(Use the domain `SessionStatusValue::Running` directly if the `WireStatus` conversion isn't in scope — the point is a running state.)*

- [ ] **Step 6: Run — fails** (`is_quiesced` not found).

- [ ] **Step 7: Implement.** In `runloop.rs` (module scope, near `run`):

```rust
/// §2.3 D21: quiescent ⇔ no transient work ∧ transport live ∧ not mid-reconcile.
fn is_quiesced(state: &SessionState, transport: &ActorTransport, reconcile_in_flight: bool) -> bool {
    !state.transient_work_outstanding()
        && matches!(transport, ActorTransport::Connected)
        && !reconcile_in_flight
}
```

- [ ] **Step 8: Run — passes + full crate.** `cargo test -p lens-core` → PASS.

- [ ] **Step 9: Commit.**

```bash
git add crates/lens-core/src/domain/session.rs crates/lens-core/src/actor/runloop.rs
git commit -m "feat(state-model): pure transient_work_outstanding + actor is_quiesced (D21)"
```

---

### Task 3 — **[GROK SEAM #1]** Actor item-lifecycle: disk-canonical commit-on-terminal + watermark (D20 + D23)

This is the crux. It mutates merged P3-1 code and deletes the item-delta channel. **Author composer-2.5; review grok-4.5 via cursor-delegate before merge.**

#### Design (own this — implement exactly; flag any deviation for review)

The reducer stops emitting item deltas; it still mutates `state.items` (the working set). The **actor** owns transcript persistence:

1. **Actor owns `next_ordinal: i64`**, seeded on spawn from the disk frontier: `frontier().map(|(o,_)| o + 1).unwrap_or(0)`. The old positional scheme (`state.items.len()`) is **wrong under pruning** and is deleted.
2. **Commit the terminal *prefix*.** After each reduced batch, walk `state.items` front→back: while the front item `is_terminal()`, `upsert_item(next_ordinal, front)`, `next_ordinal += 1`, pop it from `state.items`; **stop at the first non-terminal item**. This guarantees (a) on-disk order == transcript order, (b) dense contiguous ordinals, (c) the watermark sits **below** every non-terminal item (D23 invariant: "not-yet-terminal items stay above the watermark").
3. **Watermark.** If ≥1 item committed this batch, push `StreamUpdate::TranscriptAdvanced { committed_ordinal: next_ordinal - 1 }`.
4. **Prune-after-write-through** (D20): committed items leave `state.items`. In-progress function calls (`status != "completed"`) stay resident for dedup, above the watermark.
5. **`Rebased` is scalars-only** (D23): clear `items` on the baseline before emit.
6. **Re-fire safety** (D20): a far-back re-fire of an already-committed id is a **blind idempotent disk upsert-by-id**. Change `upsert_item`'s conflict clause to **preserve the existing `ordinal`** (`ordinal=items.ordinal`, not `excluded.ordinal`) so a re-fire refreshes payload without moving the row. `reconcile()` (the full-truth path) is unaffected — it re-stamps ordinals inside its own transaction.

**Terminal predicate:** all `ItemKind` are terminal **except** `FunctionCall { status }` where `status != "completed"`.

**Known flagged hazard (defer, do not solve here):** for *scaffold* (non-native) sessions the harness publishes `fc_*` ids on `output_item.done` while the durable store appends its **own** id (D23 impl hazard, `sessions.py:9716`). A live `fc_*` commit and its store twin can double-commit under two ids. Native sessions are clean. **P3-3a keys on `Item::id()`; the scaffold-id reconciliation is a P3-3b/follow-up concern** — call it out for the reviewer, leave a `TODO(P3-3b, scaffold-id)`.

**Files:**
- Modify: `crates/lens-core/src/reduce/update.rs` (enum surgery)
- Modify: `crates/lens-core/src/reduce/items.rs` (`push_item` emits no delta; tests)
- Modify: `crates/lens-core/src/domain/item.rs` (`ItemKind::is_terminal`)
- Modify: `crates/lens-core/src/reduce/mod.rs` (any arm asserting on item deltas)
- Modify: `crates/lens-core/src/actor/runloop.rs` (`next_ordinal`, `commit_terminal_prefix`, split `persist_write_through`, `coalesce`, `scalars_baseline`)
- Modify: `crates/lens-core/src/persist/mod.rs` (add `frontier` to `TranscriptStore` trait)
- Modify: `crates/lens-core/src/persist/transcript.rs` (`frontier` impl; conflict clause)
- Modify: `crates/lens-store/src/lib.rs` (`apply`: delete item arms, add `TranscriptAdvanced` no-op)
- Test: all of the above `#[cfg(test)]` modules

**Interfaces:**
- Produces:
  - `StreamUpdate::TranscriptAdvanced { committed_ordinal: i64 }` (replaces `ItemAppended`/`ItemUpdated`).
  - `ItemKind::is_terminal(&self) -> bool`.
  - `TranscriptStore::frontier(&self) -> Result<Option<(i64, ItemId)>>` (max ordinal + its `item_id`; `None` when empty). Consumed by Task 4's catch-up.

#### Deletion inventory (every `ItemAppended`/`ItemUpdated` site — from grep)

- `reduce/update.rs:19,20` — delete the two variants; `81` — the smallvec test uses `ItemAppended`, retarget it to `TranscriptAdvanced` or delete.
- `reduce/items.rs:188,201` — `push_item` returns; `251,288,455-456` — tests asserting on the deltas → retarget to `state.items` assertions.
- `lens-store/src/lib.rs:13,14` — `apply` arms (delete); `193` — test → retarget.
- `actor/runloop.rs:447,454,465,497,513` — `persist_write_through`/`coalesce` (rewrite); `867,899,944-945` — tests → retarget to disk assertions / `TranscriptAdvanced`.

- [ ] **Step 1: Enum surgery + `is_terminal` (compile-break first).** In `update.rs` delete `ItemAppended(Arc<Item>)` and `ItemUpdated { index, item }`; add near the transcript group:

```rust
    /// D23: disk-canonical transcript watermark. The actor emits this AFTER a
    /// commit-on-terminal write-through; the focused replica reads
    /// `(last_rendered, committed_ordinal]` off `TranscriptStore` (RowSource — deferred UI).
    TranscriptAdvanced {
        committed_ordinal: i64,
    },
```

In `domain/item.rs`, `impl ItemKind`:

```rust
/// D23 commit rule: everything is terminal EXCEPT an in-progress function call.
pub fn is_terminal(&self) -> bool {
    !matches!(self, ItemKind::FunctionCall { status, .. } if status != "completed")
}
```

- [ ] **Step 2: `push_item` emits no delta.** In `items.rs` `push_item`, keep the `state.items` mutation, return an empty batch for both branches:

```rust
pub(crate) fn push_item(
    state: &mut SessionState, id: ItemId, kind: ItemKind, seq: Option<u64>, clock: &dyn Clock,
) -> Updates {
    let ctx = current_ctx(&state.stream);
    if let Some(idx) = state.items.iter().position(|it| it.id == id) {
        let existing = Arc::make_mut(&mut state.items[idx]);
        existing.kind = kind;
        existing.seq = seq.or(existing.seq);
    } else {
        state.items.push(Arc::new(Item { id, seq, ctx, created_at: clock.now_millis(), kind }));
    }
    smallvec![] // D23: no replica-facing item delta; the actor commits terminal items to disk.
}
```

- [ ] **Step 3: Retarget the reducer item tests.** In `items.rs` tests (lines ~188,201,251,288,455), replace `ItemUpdated`/`ItemAppended` assertions with working-set assertions, e.g.:

```rust
// was: assert_eq!(out, smallvec![StreamUpdate::ItemAppended(item)]);
push_item(&mut state, id.clone(), kind, None, &clock);
assert_eq!(state.items.last().unwrap().id, id);
// dedup-update case: assert the in-place mutation instead of ItemUpdated{index:0}
```

- [ ] **Step 4: Run reducer tests — compile + pass.** `cargo test -p lens-core reduce::` → PASS (the reducer no longer references deleted variants).

- [ ] **Step 5: Failing test — `frontier`.** In `transcript.rs` tests:

```rust
#[test]
fn frontier_returns_max_ordinal_and_its_id() {
    let d = tempdir().unwrap();
    let s = store(d.path());
    assert!(s.frontier().unwrap().is_none(), "empty transcript has no frontier");
    s.upsert_item(0, &item("item_a", 0, "a")).unwrap();
    s.upsert_item(1, &item("item_b", 0, "b")).unwrap();
    assert_eq!(s.frontier().unwrap(), Some((1, ItemId::new("item_b"))));
}
```

- [ ] **Step 6: Run — fails** (no `frontier`). Add to `trait TranscriptStore` (mod.rs):

```rust
/// Newest persisted item: `(max ordinal, its item_id)`, or `None` when empty.
/// Seeds the actor's `next_ordinal` and the D19 forward-catch-up `after` cursor.
fn frontier(&self) -> Result<Option<(i64, ItemId)>>;
```

Impl in `transcript.rs` (+ `use crate::domain::ids::ItemId;`):

```rust
fn frontier(&self) -> Result<Option<(i64, ItemId)>> {
    let row = self.conn.query_row(
        "SELECT ordinal, item_id FROM items ORDER BY ordinal DESC LIMIT 1",
        [],
        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
    );
    match row {
        Ok((ord, id)) => Ok(Some((ord, ItemId::new(id)))),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}
```

Add a `FailingTranscriptStore`/`Failing...`-style stub `frontier` (`Ok(None)`) wherever `TranscriptStore` is hand-implemented in tests (`runloop.rs` `FailingTranscriptStore`).

- [ ] **Step 7: Idempotent re-fire — conflict clause.** In `transcript.rs` `upsert_item_stmt`, change `ordinal=excluded.ordinal` to `ordinal=items.ordinal` (preserve stored position on an id conflict). Add a test:

```rust
#[test]
fn refire_by_id_keeps_original_ordinal() {
    let d = tempdir().unwrap();
    let s = store(d.path());
    s.upsert_item(5, &item("item_a", 0, "a")).unwrap();
    // A far-back re-fire arrives with a different (blind) ordinal — position must not move.
    s.upsert_item(99, &item("item_a", 0, "a-refire")).unwrap();
    assert_eq!(s.frontier().unwrap(), Some((5, ItemId::new("item_a"))));
    let rows = s.load_items().unwrap().rows;
    assert_eq!(rows.len(), 1);
}
```

Run: `cargo test -p lens-core transcript::` → PASS. *(Confirm `reconcile_matches_server_truth_by_id` still passes — reconcile re-stamps inside its own transaction and must be unaffected.)*

- [ ] **Step 8: Failing test — commit-terminal-prefix in the actor.** In `runloop.rs` tests, replace `batched_appends_persist_at_distinct_ordinals` with a prefix test driven through the run-loop. Two events: an in-progress function call (non-terminal — stays), then a completed message (terminal — commits) — assert only the terminal one hits disk and a `TranscriptAdvanced` is emitted:

```rust
#[test]
fn in_progress_call_blocks_commit_completed_message_advances_watermark() {
    let dir = tempfile::tempdir().unwrap();
    let stores = test_stores(dir.path());
    seed_connection(&stores);
    let (ev_tx, ev_rx) = crossbeam_channel::bounded(64);
    let (up_tx, up_rx) = async_channel::bounded(64);
    let handle = spawn_actor(fresh_state(), ev_rx, up_tx, stores, test_clock(), noop_api());

    // in-progress function call — non-terminal, must NOT commit
    ev_tx.send(parse_response(
        "response.output_item.done",
        r#"{"item":{"id":"fc_1","type":"function_call","status":"in_progress","name":"read","arguments":"{}","call_id":"toolu_1"}}"#,
    )).unwrap();
    // a finalized assistant message via response.completed — terminal, must commit at ordinal 0
    // (drive whatever event finalizes an open message in the existing corpus/helpers)
    // ... assert up_rx yields StreamUpdate::TranscriptAdvanced { committed_ordinal: 0 }
    // ... reopen the transcript store: exactly the message row is on disk, fc_1 is not.
    handle.stop_and_join();
}
```

*(Compose the terminal-message half from the existing `reduce::testutil`/corpus helpers used elsewhere in this test module; the assertion contract is: fc_1 absent from disk, message present at ordinal 0, one `TranscriptAdvanced{committed_ordinal:0}`.)*

- [ ] **Step 9: Run — fails** (positional `persist_write_through` still writes the in-progress call / no `TranscriptAdvanced`).

- [ ] **Step 10: Rewrite the actor persist path.** In `runloop.rs`:

Seed the cursor in `run()` before the loop (near the `send_seq` seed):

```rust
let mut next_ordinal: i64 = stores
    .transcript
    .frontier()
    .ok()
    .flatten()
    .map(|(o, _)| o + 1)
    .unwrap_or(0);
```

Split `persist_write_through` into scalar-only persistence (delete the `ItemAppended`/`ItemUpdated`/`base`/`append_i` machinery; keep the `touched_scalar → control.upsert_session` half) and add:

```rust
/// D20/D23: commit the terminal PREFIX of the working set to disk in order,
/// prune it from RAM, advance `next_ordinal`. Returns the new watermark iff ≥1
/// item committed. A non-terminal front item stops the prefix (it and everything
/// after stay above the watermark). A persist error stops the prefix and leaves
/// the item resident for the next batch (no ordinal gap, no RAM loss).
fn commit_terminal_prefix(
    stores: &ActorStores, state: &mut SessionState, next_ordinal: &mut i64, ring: &mut OutcomeRing,
) -> Option<i64> {
    let mut committed = false;
    while let Some(front) = state.items.first() {
        if !front.kind.is_terminal() {
            break;
        }
        match stores.transcript.upsert_item(*next_ordinal, front) {
            Ok(()) => {
                state.items.remove(0);
                *next_ordinal += 1;
                committed = true;
            }
            Err(e) => {
                ring.push(ActorOutcome::PersistError {
                    where_: "transcript.upsert_item",
                    message: e.to_string(),
                });
                break;
            }
        }
    }
    committed.then(|| *next_ordinal - 1)
}
```

In the event arm, after `reduce`+drain builds `batch`, replace the item-writing call:

```rust
persist_scalars(&stores, &state, &batch, clock.now_millis(), &mut ring); // renamed, scalar-only
if let Some(ord) = commit_terminal_prefix(&stores, &mut state, &mut next_ordinal, &mut ring) {
    batch.push(StreamUpdate::TranscriptAdvanced { committed_ordinal: ord });
}
```

*(`batch` must be `mut`; `commit_terminal_prefix` runs after reduce, before `coalesce`/emit, so the watermark reaches the replica in the same frame.)*

- [ ] **Step 11: `coalesce` + `Rebased` scalars-only + `apply`.**
  - `coalesce` (runloop.rs): drop the `ItemAppended | ItemUpdated` special cases; keep-last-by-discriminant now applies to **all** variants (`TranscriptAdvanced` included — at most one per batch, so trivially correct).
  - `Rebased`: add a helper and use it in both emit sites (the `Promote` arm and the `had_snapshot` arm):

```rust
fn scalars_baseline(state: &SessionState) -> Box<SessionState> {
    let mut b = state.clone();
    b.items.clear(); // D23: Rebased is scalars-only; baseline items come from disk on promote (deferred UI).
    Box::new(b)
}
```

  - `lens-store/src/lib.rs` `apply`: delete the `ItemAppended`/`ItemUpdated` arms; add `TranscriptAdvanced { .. } => {}` to the no-op marker group (RowSource consumer is deferred UI work). Retarget the `apply_item_appended_pushes_shared_body` test (line ~187) to assert `TranscriptAdvanced` is a no-op, or delete it.

- [ ] **Step 12: Run — the whole workspace.** `cargo test -p lens-core -p lens-store` → PASS. Fix any remaining references to the deleted variants (`detailed_mode_emits_rebased_after_snapshot_restored` should still pass — Rebased still fires, now scalars-only; assert `baseline.items.is_empty()` if you want the D23 guarantee pinned).

- [ ] **Step 13: Gate + commit.**

```bash
cargo run -p xtask -- gate
git add -A
git commit -m "feat(state-model): D20/D23 disk-canonical commit-on-terminal + watermark; drop item deltas"
```

- [ ] **Step 14: [MANDATORY] Cross-family review — grok-4.5 via cursor-delegate.** Focus the reviewer on: the commit-prefix ordering invariant (does a non-terminal front item ever strand a later terminal item in practice?), the `ordinal=items.ordinal` conflict-clause change vs `reconcile`, watermark-below-non-terminal correctness, and the flagged scaffold-`fc_*` double-commit hazard. Apply findings; re-review if non-trivial.

---

### Task 4 — **[GROK SEAM #2]** Actor-owned forward catch-up (D19)

The actor becomes the **sole** `/items` fetcher: a forward, `order=asc`, disk-frontier-anchored catch-up, mode-switched on the actor thread. Builds on Task 3's `frontier`/`next_ordinal`/`commit`. **Author composer-2.5; review grok-4.5 via cursor-delegate.**

#### Design (own this)

- **New actor HTTP capability:** extend `trait SessionApi` with `fetch_items(&SessionId, &ItemsPage) -> Result<ItemList>`. Real impl wraps `Sessions::items`. Injected exactly like `send_event`.
- **Trigger points:** (a) once on spawn (first-attach / wake), before entering the main `Select` loop; (b) whenever a processed batch contains `Reconnected`.
- **Mode-switched catch-up loop** (runs on the actor thread; `reconcile_in_flight = true` throughout, so `is_quiesced` is false and a concurrent Sleep is correctly declined):

```
fn run_catchup(api, stores, state, next_ordinal, events_rx, commands_rx, output, ring):
    reconcile_in_flight = true            // caller sets + emits TransportChanged
    let mut after = stores.transcript.frontier()?.map(|(_, id)| id.to_string())   // None ⇒ from oldest
    let mut buffered_events = Vec::new()
    let mut deferred_commands = Vec::new()
    loop:
        // 1. never block the reader's bounded channel: drain live events into RAM
        while let Ok(ev) = events_rx.try_recv(): buffered_events.push(ev)
        // 2. honor Stop immediately; stash other commands to replay post-catch-up
        while let Ok(cmd) = commands_rx.try_recv():
            match cmd { Stop => return CatchupAbort, other => deferred_commands.push(other) }
        // 3. one bounded blocking page
        let page = ItemsPage { after: after.clone(), order: Some("asc".into()),
                               before: None, limit: Some(CATCHUP_PAGE) }
        let list = match api.fetch_items(&state.id, &page):
            Ok(l) => l, Err(e) => { ring.push(PersistError/CatchupError); break }   // degrade: live-tail only
        for item in list.items():
            upsert_item(next_ordinal, item); after = Some(item.id().to_string()); next_ordinal += 1
        if wrote_any: output.send(TranscriptAdvanced { committed_ordinal: next_ordinal - 1 })
        if !list.has_more(): break
    // 4. replay deferred commands, then drain buffered live events through the normal reduce path
    return CaughtUp { buffered_events, deferred_commands }
```

- **Contiguity invariant:** catch-up items get ordinals continuing from the disk frontier; the buffered live tail reduces *after* catch-up, taking higher ordinals. No interleaving → no ordinal inversion.
- **`CATCHUP_PAGE`** const = `200`.
- **Store-id keying:** catch-up items come from `/items` (durable store) so their ids **are** store ids. Live `output_item.done` may carry harness `fc_*` for scaffold sessions — same flagged hazard as Task 3 (`TODO(P3-3b, scaffold-id)`); native sessions clean.
- **Deferred (D22):** a *first attach to a never-persisted large session* pages oldest-first over the whole history (slow but off the hot path). Lens-created sessions start empty and grow, so this is acceptable for 3a; the snapshot-tail-paint + negative-ordinal scroll-back optimization is D22, deferred whole. Do **not** foreclose negative ordinals.
- **Transient double-fetch (accepted):** until Task 5 lands, the reader *still* replays `/items` (`items_to_replay`) AND the actor now catches up — both write the same rows, idempotent by id. Task 5 removes the reader half.

**Files:**
- Modify: `crates/lens-core/src/actor/api.rs` (`SessionApi::fetch_items`)
- Modify: `crates/lens-core/src/actor/runloop.rs` (`run_catchup`, trigger on spawn + `Reconnected`, `CATCHUP_PAGE`)
- Modify: wherever the real `SessionApi` is implemented for wiring (search `impl SessionApi for` — the production adapter over `lens_client::Sessions`; add `fetch_items`). If none exists yet in-tree (P3-2 wired only `send_event` via a thin adapter), add the `Sessions::items` delegation there.
- Test: `runloop.rs` tests (scripted `MockApi` gains `fetch_items`)

**Interfaces:**
- Consumes: `TranscriptStore::frontier` (Task 3); `lens_client::sessions::{ItemsPage, ItemList}`; `ItemList::items()/has_more()`; `Sessions::items(&SessionId, &ItemsPage)`.
- Produces: `SessionApi::fetch_items(&self, id: &SessionId, page: &ItemsPage) -> Result<ItemList, ClientError>`.

- [ ] **Step 1: Extend `SessionApi`.** In `api.rs`:

```rust
use lens_client::sessions::{ItemList, ItemsPage, SendEventAck, SessionEventInput};

pub trait SessionApi: Send {
    fn send_event(&self, id: &SessionId, evt: &SessionEventInput) -> Result<SendEventAck, ClientError>;
    /// D19: the actor is the SOLE `/items` fetcher (forward catch-up). Blocking GET.
    fn fetch_items(&self, id: &SessionId, page: &ItemsPage) -> Result<ItemList, ClientError>;
}
```

Add `fetch_items` to every `impl SessionApi` (the production adapter delegates to `Sessions::items`; the test `MockApi`/`PanicApi` get a scripted/`panic!` impl).

- [ ] **Step 2: Failing test — two-page catch-up appends contiguously, live tail lands after.** In `runloop.rs` tests, extend `MockApi` to script `fetch_items` (a `VecDeque<Result<ItemList, ClientError>>`), then:

```rust
#[test]
fn catchup_pages_forward_from_frontier_then_applies_buffered_live_tail() {
    // disk frontier = item_2 @ ordinal 2; fetch_items scripts page1 [item_3,item_4] has_more=true,
    // page2 [item_5] has_more=false. A live output_item.done(item_6) is queued before spawn.
    // Assert: disk ends item_3..item_6 at ordinals 3..6 contiguous; a TranscriptAdvanced with
    // committed_ordinal>=5 is emitted from catch-up; item_6 (live) lands at ordinal 6 AFTER.
}
```

*(Seed the transcript store with item_0..item_2 before spawn so `frontier()` returns item_2/ordinal 2.)*

- [ ] **Step 3: Run — fails** (no catch-up; live event would race ahead of history).

- [ ] **Step 4: Implement `run_catchup` + wire triggers.** Add `const CATCHUP_PAGE: u32 = 200;`, the `run_catchup` function per the design pseudocode, and:
  - Call it once in `run()` after seeding `next_ordinal`, before the main loop; set `reconcile_in_flight = true` + emit `TransportChanged` around it; on return, `reduce` the `buffered_events` in order and re-dispatch `deferred_commands` through the existing command arms.
  - In the event arm, when `saw_reconnected`, invoke the same catch-up (the reader's `Reconnected` already flips `reconcile_in_flight`; catch-up runs, then clears it on completion).
  - Factor the buffered-event drain + deferred-command replay so both trigger sites share it.

- [ ] **Step 5: Run — passes.** `cargo test -p lens-core catchup_` → PASS. Confirm existing reconnect/send tests still pass (catch-up with an empty `fetch_items` script — mock returns an empty `ItemList` `has_more=false` — must be a no-op for tests that don't exercise it; give `MockApi` a default empty-page script).

- [ ] **Step 6: Gate + commit.**

```bash
cargo run -p xtask -- gate
git add -A
git commit -m "feat(state-model): D19 actor-owned forward catch-up (sole /items fetcher)"
```

- [ ] **Step 7: [MANDATORY] Cross-family review — grok-4.5 via cursor-delegate.** Focus: the buffer-then-drain ordinal-contiguity invariant, the reader-bounded-channel backpressure (does `try_recv`-draining events prevent a reader stall?), Stop-during-catch-up, `reconcile_in_flight` gating Sleep, and `fetch_items` error degradation (live-tail-only, no panic). Apply findings.

---

### Task 5 — **[GROK SEAM #3]** Reader → transport-only (D19, subtractive `lens-client`)

Now that the actor owns item recovery (Task 4), delete the reader's item-replay. **Subtractive on the hardened crate — MANDATORY cross-family review even though it only deletes.** Author composer-2.5; review grok-4.5.

#### Deletion inventory

- `reconnect.rs`: `trait Reopen` — delete `fn items(&self) -> Result<ItemList>` (3→2 methods: `open_stream`, `snapshot`). Delete `HttpReopener::items()`. Delete `pub(crate) fn items_to_replay(...)` + its test. Drop now-unused imports (`ItemList`, `ItemsPage`, `ResponseEvent` if only used there).
- `reader.rs`: `bootstrap` (line ~113) — drop the `.and_then(|snap| reopener.items()...)`; fetch snapshot only, emit `SnapshotRestored`, no replay loop. `reconnect` (line ~241) — delete the `reopener.items()` fetch block and the `items_to_replay(list)` loop; `open_stream` remains the last fallible call after `snapshot` (ordering guarantee preserved: `snapshot → open_stream`). Delete `use crate::reconnect::items_to_replay`.
- `reader.rs` tests: every mock `Reopen` (`ExhaustReopener`, the two `MockReopen`) — delete their `items()` impls and any `ItemList` fixtures; update assertions that expected replayed `OutputItemDone` events after `Reconnected`/`SnapshotRestored` (they now expect **none** — the actor catches up).

**Files:**
- Modify: `crates/lens-client/src/reconnect.rs`
- Modify: `crates/lens-client/src/stream/reader.rs`
- Test: `reader.rs` + `reconnect.rs` `#[cfg(test)]` modules

**Interfaces:**
- Produces: `trait Reopen { fn open_stream(...); fn snapshot(...); }` (2 methods). No `ItemList` on the reconnect path.

- [ ] **Step 1: Update the reconnect-sequence test to expect no replay.** In `reader.rs` tests, pick the reconnect test that currently asserts replayed `OutputItemDone` after `SnapshotRestored` and change it to assert the post-reconnect sequence is exactly `Reconnecting* → Reconnected → SnapshotRestored` with **no** `OutputItemDone` replay.

- [ ] **Step 2: Run — fails** (replay still emitted).

- [ ] **Step 3: Shrink `Reopen` + delete `items_to_replay`** (reconnect.rs) per the inventory. Compile-break the callers.

- [ ] **Step 4: Delete the reader replay calls** (reader.rs `bootstrap` + `reconnect`), fix mock `Reopen` impls (remove `items`).

- [ ] **Step 5: Run — passes.** `cargo test -p lens-client stream::` → PASS. `cargo test -p lens-client` → PASS.

- [ ] **Step 6: Gate + commit.**

```bash
cargo run -p xtask -- gate
git add -A
git commit -m "refactor(lens-client): D19 reader transport-only — delete item replay, Reopen 3→2"
```

- [ ] **Step 7: [MANDATORY] Cross-family review — grok-4.5 via cursor-delegate.** Focus: nothing else depended on `items()`/`items_to_replay`; the `snapshot → open_stream` ordering still can't strand an opened body; the Failed-snapshot terminal path (`SnapshotRestored → Disconnected{SessionFailed}`) is preserved. Apply findings.

---

### Task 6: Sleep command + wake respawn (D21)

`Sleep` is a `SessionCommand` processed in-loop (closes the scheduler→sleep TOCTOU); wake is a fresh respawn from disk.

**Files:**
- Modify: `crates/lens-core/src/actor/runloop.rs` (`SessionCommand::Sleep`, command arm, `ActorOutcome::Slept`/`SleepDeclined`)
- Modify: `crates/lens-core/src/actor/outcome.rs` (outcome variants)
- Test: `runloop.rs` tests

**Interfaces:**
- Consumes: `is_quiesced` (Task 2); `SessionEventInput::StopSession` (existing).
- Produces: `SessionCommand::Sleep`; `ActorOutcome::Slept` and `ActorOutcome::SleepDeclined`.

#### Design

`Sleep` arm: re-check `is_quiesced(&state, &transport, reconcile_in_flight)` **in-loop**. If not quiesced → emit `SleepDeclined`, continue (do NOT stop). If quiesced → set `state.lifecycle = SessionLifecycle::Slept`, `control.upsert_session(&state, now)` (durable flush), best-effort fire-and-forget `let _ = api.send_event(&state.id, &SessionEventInput::StopSession);`, emit `ActorOutcome::Slept`, then `break` (stop the actor).

**Flush is `lifecycle=Slept` only** (decided this session, supersedes the D21 sketch's `[lifecycle=Slept, last_seen_seq]`): `last_seen_seq` was deleted in Task 1 as vestigial — under D19 the resume cursor is the **disk item frontier**, and the server exposes no seq-based resume (`/items` = item-id cursor, `/stream` = no-replay, verified against omnigent `31669e1b`). No seq-tracking machinery in 3a.

- [ ] **Step 1: Failing test — sleep when quiescent flushes + stops.**

```rust
#[test]
fn sleep_when_quiescent_flushes_slept_and_stops() {
    let dir = tempfile::tempdir().unwrap();
    let stores = test_stores(dir.path());
    seed_connection(&stores);
    let (_ev_tx, ev_rx) = crossbeam_channel::bounded(64);
    let (up_tx, _up_rx) = async_channel::bounded(64);
    // MockApi that records a StopSession send and returns a benign ack.
    let (api, mock) = MockApi::succeed_with_ack(SendEventAck { queued: true, ..Default::default() });
    let handle = spawn_actor(fresh_state() /* idle => quiescent */, ev_rx, up_tx, stores, test_clock(), api);

    handle.commands.send(SessionCommand::Sleep).unwrap();
    match handle.outcomes.recv_blocking().unwrap() {
        ActorOutcome::Slept => {}
        other => panic!("expected Slept, got {other:?}"),
    }
    handle.join_without_stop(); // actor already stopped itself
    // reopen control store: lifecycle == Slept; StopSession was sent.
    assert!(matches!(mock.last_evt(), Some(SessionEventInput::StopSession)));
}
```

- [ ] **Step 2: Failing test — sleep while busy is declined, actor survives.**

```rust
#[test]
fn sleep_while_running_is_declined_and_actor_survives() {
    // ... fresh_state with status Running ...
    handle.commands.send(SessionCommand::Sleep).unwrap();
    match handle.outcomes.recv_blocking().unwrap() {
        ActorOutcome::SleepDeclined => {}
        other => panic!("expected SleepDeclined, got {other:?}"),
    }
    // actor still processes events afterward
    handle.stop_and_join();
}
```

- [ ] **Step 3: Run — fails** (`Sleep`/`Slept`/`SleepDeclined` undefined).

- [ ] **Step 4: Implement.** Add `Sleep` to `SessionCommand`; add `Slept { last_seen_seq: Option<u64> }` + `SleepDeclined` to `ActorOutcome`; add the command arm per the design. Ensure the arm can read the current `transport`/`reconcile_in_flight` locals.

- [ ] **Step 5: Run — passes.** `cargo test -p lens-core` → PASS.

- [ ] **Step 6: Gate + commit.**

```bash
cargo run -p xtask -- gate
git add -A
git commit -m "feat(state-model): D21 SessionCommand::Sleep — in-loop quiesce recheck, flush, stop"
```

---

### Task 7: `FleetScheduler` seam + deterministic wake round-trip + gated live-verify (D21)

A skeletal scheduler seam (spawn-on-wake / `Sleep`-command entry points) exercised by a deterministic, wall-clock-free round-trip test. §9 timer/LRU/focus policy stays deferred.

**Files:**
- Create: `crates/lens-core/src/actor/scheduler.rs` (skeletal `FleetScheduler`)
- Modify: `crates/lens-core/src/actor/mod.rs` (module wiring)
- Create: `crates/lens-core/tests/wake_roundtrip.rs` (integration round-trip) — or an in-module test if the spawn API isn't `pub`
- Modify: `crates/lens-client/tests/live_stream.rs` **or** a new gated `live_*` test (D17 live-verify rider)

**Interfaces:**
- Produces: `FleetScheduler` with `sleep(session_id)` (routes a `Sleep` command to a running actor) and `wake(session_id) -> ActorHandle` (respawns from disk: `load_session` → seed `next_ordinal` from `frontier` → open stream → catch-up). Injected `Clock`, `Reopen`-mock, temp `TranscriptStore` for the test.

#### Design

The scheduler is a thin registry seam — it does NOT own the 10-min timer / LRU / focus policy (deferred §9). It exposes exactly two lifecycle entry points so the round-trip is testable:
- `sleep`: `handle.commands.send(SessionCommand::Sleep)`.
- `wake`: load control scalars from disk (`ControlStore::load_session`), spawn a fresh actor (which seeds `next_ordinal` from `frontier` and runs catch-up).

Round-trip test (deterministic, no wall clock): seed disk with a slept session + a few transcript items → `wake` → assert the respawned actor's replica reflects the disk scalars and a `fetch_items`-scripted catch-up re-materializes the tail at the right ordinals → drive it idle → `sleep` → assert `lifecycle=Slept` back on disk and the actor stopped.

- [ ] **Step 1: Failing round-trip test** per the design (injected `ManualClock`, scripted `MockApi.fetch_items`, temp stores).

- [ ] **Step 2: Run — fails** (`FleetScheduler` absent).

- [ ] **Step 3: Implement the skeletal `FleetScheduler`** (`sleep`/`wake` only; a `HashMap<SessionId, ActorHandle>` registry is enough). Wire the module.

- [ ] **Step 4: Run — passes.** `cargo test -p lens-core wake_roundtrip` → PASS.

- [ ] **Step 5: Gated D17 live-verify (batched, not gate-blocking).** Add a `#[cfg(feature = "live-tests")]` test that, against a real pinned-0.4.0 server (`installing-omnigent-from-source`), drives a session to quiescence, sleeps it (`StopSession`), then wakes (respawn) and asserts the post-`stop_session` effects are durably re-fetchable via forward catch-up. Run it once manually this session; record the result in the handoff. It is **informational** — never in `xtask gate`.

- [ ] **Step 6: Gate + commit.**

```bash
cargo run -p xtask -- gate
git add -A
git commit -m "feat(state-model): D21 FleetScheduler seam + deterministic wake round-trip"
```

---

### Task 8: Docs + integrate

**Files:**
- Modify: `docs/STATUS.md` (Recent entry; move P3-3a to DONE)
- Create: `docs/handoffs/2026-07-10-state-model-p3-3a-execution.md`
- Modify: `.superpowers/sdd/progress.md` (P3-3a rollup; carry forward P3-3b deferrals)
- Update memory: `state-model-p3-3a-grilling` → add an "executed" note; new memory if a non-obvious gotcha surfaced (e.g. commit-prefix ordering, scaffold-id hazard).

- [ ] **Step 1: Write the handoff** — decisions as-built (commit-terminal-prefix, watermark, `ordinal=items.ordinal` re-fire, forward catch-up, transport-only reader, sleep/wake), the three grok review findings, the D17 live-verify result, and the carried-forward **flagged deferrals**: scaffold-`fc_*` double-commit (P3-3b), never-seen-huge first attach + negative-ordinal scroll-back (D22), the disk `RowSource` viewport/UI plan, `last_seen_seq`-at-sleep simplification.
- [ ] **Step 2: Update STATUS + progress.**
- [ ] **Step 3: Final gate.** `cargo run -p xtask -- gate` → green.
- [ ] **Step 4: Commit docs.**

```bash
git add -A
git commit -m "docs(state-model): P3-3a lifecycle core executed — STATUS/handoff/progress"
```

- [ ] **Step 5: Integrate + push.** Per the user's directive for this branch, push after completion. If on a feature branch, fast-forward merge to `main` (solo-project workflow, all tests green + zero warnings), then:

```bash
git push
```

---

## Self-Review

**Spec coverage (§2.3 D19–D23 + D15 rider):**
- D15 → Task 1. Absent-fold + persist guard both covered. ✓
- D19 forward catch-up + sole-fetcher → Task 4; transport-only reader → Task 5. ✓
- D20 working-set / prune-after-write-through / no byte-window → Task 3 (commit-terminal-prefix + prune). ✓
- D21 `is_quiesced`/`transient_work_outstanding` → Task 2; `Sleep`/wake → Task 6; `FleetScheduler` + round-trip → Task 7. ✓
- D22 never-seen-huge deferred whole → explicitly deferred in Task 4 design + Task 8 handoff; `i64` ordinal preserved. ✓
- D23 delete item deltas / `TranscriptAdvanced` watermark / commit-on-terminal / `Rebased` scalars-only / re-fire idempotence → Task 3. ✓ Disk `RowSource` (viewport) explicitly deferred.

**Task order** builds catch-up (Task 4) **before** deleting reader replay (Task 5); Task 3 provides `frontier`/`next_ordinal`/commit that Task 4 consumes. No broken intermediate (Task 3→4 has a transient idempotent double-fetch, called out).

**Placeholder scan:** deletion inventories are concrete (grep line numbers); the two "compose from existing corpus helpers" notes (Task 3 Step 8, Task 4 Step 2) point at the real in-file helpers rather than inventing fixtures — acceptable because the assertion contract is spelled out and the exact helper names live in the test module being edited.

**Type consistency:** `TranscriptAdvanced { committed_ordinal: i64 }`, `frontier() -> Result<Option<(i64, ItemId)>>`, `is_terminal(&self) -> bool`, `fetch_items(&self, &SessionId, &ItemsPage) -> Result<ItemList, ClientError>`, `Slept { last_seen_seq: Option<u64> }` — used consistently across producing/consuming tasks. `next_ordinal: i64` seeded from `frontier` in Task 3, reused by Task 4. `is_quiesced(&SessionState, &ActorTransport, bool)` defined Task 2, consumed Task 6.

**Open micro-decisions surfaced for the grok seams (not silently resolved):** commit-prefix ordering assumption (Task 3), scaffold-`fc_*` double-commit (Tasks 3/4), `last_seen_seq`-at-sleep simplification (Task 6), catch-up error degradation to live-tail-only (Task 4).
