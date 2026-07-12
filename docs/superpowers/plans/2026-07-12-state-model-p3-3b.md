# State-Model P3-3b Implementation Plan — recovery semantics + scaffold-id

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land the P3-3b engine slice — park-as-exit recovery with a single user-gated reconnect, no-silent-drop send-failure semantics, durable scaffold-id dedup, and the C2–C4 tech-debt cleanups — plus a headless `lens-drive` binary to exercise the actor end-to-end before `lens-ui` exists.

**Architecture:** All work is inside the existing `lens-core` actor/reduce/persist stack (crates already built through P3-3a) plus one new `crates/lens-drive` binary. The through-line is the reconnect/catch-up path in `actor/runloop.rs`: D24 makes park terminal (actor exits), D30 rewrites the frontier/catch-up/commit code for scaffold-id dedup, and D28 hangs held-bubble landed-detection off the in-actor reconnect. D27 reshapes the send-outcome enums. These cluster because C1/D30 rewrites the same catch-up/frontier/commit code D24 and D28 touch.

**Tech Stack:** Rust (workspace), `rusqlite` (bundled, WAL), `crossbeam-channel` (actor `Select`), `async-channel` (bridge), `lens-client` (pinned omnigent 0.5.1 contract), `serde_json` (lens-drive I/O).

**Decisions SSOT:** `docs/superpowers/specs/2026-07-08-state-model-engine-design.md` §2.4 (D24–D31) + the live-verify appendix. App-arch amended sections: `docs/design/app-architecture-and-state-model.md` §3.5, §13.1.

## Global Constraints

- **Delegation (CLAUDE.md):** each task's *implementation* is executed by **composer-2.5** via `cursor-delegate`. Heavy tasks (T1/T5/T6) are pinned to the algorithm here so composer is a deterministic executor, not an author. **Every task gets a cross-family review** (grok-4.5 via `cursor-delegate`, or `codex exec -s read-only` for gpt-5.5). Tasks marked **[MANDATORY seam review]** (T1, T2, T5, T6) touch subtractive/contract surface — the review is non-negotiable.
- **Pre-build gate:** before executing **T5 (D30)** and **T6 (D28)**, their plan sections get a cross-family review (grok-4.5). Do not dispatch T5/T6 execution until that review lands and any spec-flaws are folded back into this doc.
- **Green bar every task:** `cargo test -p lens-core` (+ `-p lens-drive` from T4 on) passes, `cargo clippy --all-targets -- -D warnings` clean, `cargo fmt --check` clean. No dead code, no `#[allow]` added without a one-line justification.
- **No new omnigent contract assumptions** beyond the pinned 0.5.1 surface. Where a decision wants a field the contract doesn't expose (D28 `pending_inputs` content), implement the degraded path and leave a `TODO(omnigent client-message-id)`.
- **`pending_user` stays RAM-only** (D29 / D-P2-5/6). No decision here persists client-optimistic send intent to disk.
- **Injected `Clock` only** — no wall-clock reads in actor/reduce/persist code (deterministic tests depend on it).
- **conv-key ids reused, never minted** — the lens-client↔lens-core id boundary from P0 stands.

---

## File Structure

**Modified (lens-core):**
- `crates/lens-core/src/actor/transport.rs` — `ParkReason` gains `Forbidden`/`NotFound`; doc comment corrected (park = actor exits). `ActorTransport::Parked` variant retained only as a transient marker within a batch (see T1); the resting state is "no actor".
- `crates/lens-core/src/actor/outcome.rs` — `ActorOutcome::SendLost` gains `content`; `Parked` becomes the single terminal disconnect outcome; `StoppedRemoved`/`StoppedTombstone` fold into `Parked{reason}`.
- `crates/lens-core/src/actor/api.rs` — `CommandOutcome`: `SendFailed`/`SendDenied` gain `content`; add `SendPending{lens_pending_id}`; **delete `SendRejected`**. `SessionApi` trait gains `fetch_status`.
- `crates/lens-core/src/actor/runloop.rs` — the big one: park→exit surgery (`apply_reduced_batch`, `run` loop), send-site 3-fate split (`handle_command`), frontier→store-frontier + provisional commit + `id→call_id` reconcile (`run` seed, `commit_terminal_prefix`, catch-up path), C3 recursion→iteration, C4 `RunCtx` bundling, D28 held-landed reconciler hook.
- `crates/lens-core/src/actor/scheduler.rs` — add `reconnect(session_id)` + `parked: HashMap<SessionId, ParkReason>`; `reconnect` re-reads live status via `fetch_status` before respawn.
- `crates/lens-core/src/reduce/reconcile.rs` — D28 held-bubble landed-detection (content-match vs catch-up delta), producing the drop/keep/lost verdict set consumed by the actor.
- `crates/lens-core/src/persist/schema.rs` — `TRANSCRIPT_DDL` gains `provisional INTEGER NOT NULL DEFAULT 0`; bump `SCHEMA_VERSION` 1→2.
- `crates/lens-core/src/persist/transcript.rs` — `upsert_item` gains a `provisional` param; `frontier()` returns newest **non-provisional** ordinal/id; new `reconcile_store_item(store_item, ...)` that rewrites a resident provisional row's id, preserves ordinal, clears the flag; `TranscriptStore` trait extended accordingly.
- `crates/lens-core/src/persist/mod.rs` — `TranscriptStore` trait signature updates (provisional param, reconcile method).

**Created:**
- `crates/lens-drive/Cargo.toml`, `crates/lens-drive/src/main.rs` — headless JSON-lines driver binary (T4).
- `crates/lens-drive/README.md` — usage + the bright-line ("dumps state, never renders").

**Docs (T7):**
- `docs/design/app-architecture-and-state-model.md` §4.1 (identity/dedup bullet) + §6.2 (schema sketch: `provisional`) — apply the D30 amendment the spec §7.1 flagged as not-yet-edited.
- `docs/STATUS.md`, memory files.

---

## Task 1: D24 park-as-exit + D25/D26 scheduler reconnect seam

**[HEAVY — algorithm-pinned] [MANDATORY seam review — subtractive edits to merged P3-3a]**

**Files:**
- Modify: `crates/lens-core/src/actor/transport.rs`
- Modify: `crates/lens-core/src/actor/outcome.rs`
- Modify: `crates/lens-core/src/actor/api.rs` (SessionApi::fetch_status)
- Modify: `crates/lens-core/src/actor/runloop.rs:561-604` (park mapping), `runloop.rs:958-1014` (run loop), `runloop.rs:408-417` (Send-while-parked reject)
- Modify: `crates/lens-core/src/actor/scheduler.rs`
- Test: `crates/lens-core/src/actor/scheduler.rs` (tests mod) + `runloop.rs` (tests mod)

**Interfaces:**
- Produces:
  - `ParkReason` = `{ Unauthorized, SessionFailed, RetriesExhausted, Forbidden, NotFound }`.
  - `ActorOutcome::Parked { reason: ParkReason }` — the sole terminal disconnect outcome (was 3 park-sets + `StoppedRemoved` + `StoppedTombstone`).
  - `SessionApi::fetch_status(&self, id: &SessionId) -> Result<SessionStatus, ClientError>` (wraps `Sessions::get`; returns `SessionStatus::{Idle,Running,Failed}`).
  - `FleetScheduler::reconnect(&mut self, conn, session_id, events, updates, stores, clock, api) -> Result<(), FleetSchedulerError>` — re-reads live status via `api.fetch_status`, then respawns via the same disk-load path as `wake`.
  - `FleetScheduler::mark_parked(&mut self, session_id: &SessionId, reason: ParkReason)` — **the atomic park-bookkeeping entry (R2-5).** Called when the caller drains an `ActorOutcome::Parked{reason}`: it **removes+joins the exited handle from the registry AND records `parked[id]=reason` in one step**, so a subsequent `reconnect` never races an `AlreadyRunning` against a dead-but-not-yet-reaped actor. Do NOT leave "caller does `take_handle` then `parked.insert`" as two separate caller steps — that's the race codex found.
  - `FleetScheduler::parked: HashMap<SessionId, ParkReason>` + `FleetScheduler::park_reason(&self, id) -> Option<ParkReason>`.
- Consumes: `Sessions::get`/`SessionStatus` (lens-client, already exists); `spawn_actor`, `ControlStore::load_session` (existing).

**The park→exit rewrite (pin this exactly):**

Today (`apply_reduced_batch`, runloop.rs:561-604), a terminal `Disconnected(reason)` either sets `*transport = Parked{..}; *parked = true` (3 reasons) or emits `StoppedRemoved`/`StoppedTombstone` + `Break` (Forbidden/NotFound). The `parked` bool then drops the events arm from the `run` loop `Select` (runloop.rs:960-964), leaving a resident commands-only actor.

D24 collapses all five reasons to one behavior: **emit `ActorOutcome::Parked{reason}` and return `LoopControl::Break`.** The actor thread exits; the reader/connection/thread free. The `parked` bool, the `Parked` transport-arm in the `run` `Select`, and the `Send`-while-parked reject all become dead and are **deleted**.

```rust
// apply_reduced_batch, replacing lines 561-604:
if let Some(reason) = disconnect_reason {
    let park = match reason {
        DisconnectReason::Unauthorized => ParkReason::Unauthorized,
        DisconnectReason::SessionFailed => ParkReason::SessionFailed,
        DisconnectReason::RetriesExhausted => ParkReason::RetriesExhausted,
        DisconnectReason::Forbidden => ParkReason::Forbidden,
        DisconnectReason::NotFound => ParkReason::NotFound,
    };
    *reconcile_in_flight = false;
    let _ = output.outcomes.send_blocking(ActorOutcome::Parked { reason: park });
    return (LoopControl::Break, false);
}
```

Then delete every `parked: &mut bool` parameter threaded through `apply_reduced_batch`, `finish_reconnected_catchup`, `replay_buffered_batch`, `process_main_loop_event`, `invoke_catchup_and_replay`, `run_catchup`, and the `run` loop; delete the `Some(sel.recv(&events))`/`None` split in the `run` loop `Select` (events arm is now unconditional); delete the `handle_command` `Send`-while-parked branch (runloop.rs:412-417) so the `else` body becomes the whole arm. `ActorTransport::Parked` stays defined (a batch may still observe it transiently) but is never a resting state; correct its doc comment to "recoverable terminal — the actor is exiting; recovery is a fresh respawn."

> **Removing `parked` is the bulk of the diff and the whole reason for the MANDATORY seam review.** The reviewer's job: confirm no path now blocks forever on a dropped events arm, and that catch-up (which greedily drains `events`) can't observe a half-torn transport.

**The scheduler reconnect seam (D25/D26):**

`reconnect` mirrors `wake` (scheduler.rs:39-65) with two deltas: (1) it is the recovery entry for a *Disconnected* (not Slept) session — the disk lifecycle is already `Active`, so no lifecycle flip is needed, but (2) it **must re-read live status first** (D26) so a stale pre-disconnect status is never trusted.

```rust
pub fn reconnect(
    &mut self,
    conn: &ConnectionId,
    session_id: &SessionId,
    events: Receiver<ServerStreamEvent>,
    updates: async_channel::Sender<StreamUpdate>,
    stores: ActorStores,
    clock: Box<dyn Clock + Send>,
    api: Box<dyn SessionApi + Send>,
) -> Result<(), FleetSchedulerError> {
    if self.registry.contains_key(session_id) {
        return Err(FleetSchedulerError::AlreadyRunning);
    }
    // D26: re-test reality. A `failed` server session resets to `idle` across a
    // server restart; never trust a pre-disconnect status. We fetch it so a caller
    // (UI) can shape the reconnect message, but the respawn proceeds regardless —
    // nothing is auto-terminal (D25). A fetch error is itself a park reason, not a hard stop.
    let _live_status = api.fetch_status(session_id).ok(); // advisory; respawn regardless
    let state =
        crate::persist::ControlStore::load_session(stores.control.as_ref(), conn, session_id)
            .map_err(|e| FleetSchedulerError::Persist(e.to_string()))?
            .ok_or(FleetSchedulerError::SessionNotFound)?;
    // lifecycle already Active for a Disconnected session — no flip.
    let handle = spawn_actor(state, events, updates, stores, clock, api);
    self.parked.remove(session_id);
    self.registry.insert(session_id.clone(), handle);
    Ok(())
}
```

When the caller (drive/UI) drains an `ActorOutcome::Parked{reason}` it calls `mark_parked(id, reason)` — the **single atomic** step that reaps the exited handle and records the reason (R2-5); it must NOT do `take_handle` + `parked.insert` as two steps (a `reconnect` in between would hit `AlreadyRunning` on a dead actor). `reconnect` clears `parked[id]`. `park_reason` is a read accessor for UI display. `reason` is advisory only — it never removes the reconnect option (D25). Test both orderings: `mark_parked` → `reconnect` (normal) and a `reconnect` issued before the `Parked` drain (must not wedge — the caller serializes on the outcome, or `reconnect` tolerates a still-registered exited handle by reaping it).

- [ ] **Step 1: Extend `ParkReason` + fix transport doc.** Add `Forbidden`, `NotFound` variants (transport.rs); correct the `Parked` doc comment to "actor is exiting". Run `cargo build -p lens-core`; expect compile errors at the match sites (that's the guide for the rest).

- [ ] **Step 2: Fold outcome variants.** In outcome.rs: delete `StoppedRemoved`, `StoppedTombstone`; add `content: String` to `SendLost`. Update `ActorOutcome` doc. `cargo build -p lens-core` — expect errors where those variants were matched.

- [ ] **Step 3: Add `SessionApi::fetch_status`.** In api.rs, extend the trait:

```rust
/// D26: re-read live server status on reconnect/attach. Blocking GET /session.
fn fetch_status(&self, id: &SessionId) -> Result<SessionStatus, ClientError>;
```

Wire the real `Sessions`-backed impl (wherever `SessionApi for <real>` lives — search `impl SessionApi for`) to call `self.get(id, GetOpts::default())?.status()`. Add `fetch_status` to the `runloop`/`scheduler` test `MockApi`s (a `status_script: Mutex<VecDeque<Result<SessionStatus, ClientError>>>`, defaulting `Ok(SessionStatus::Idle)`).

- [ ] **Step 4: Write the failing park→reconnect round-trip test** (scheduler.rs tests mod). Mirrors `wake_roundtrip_sleep_cycle` (scheduler.rs:250):

```rust
#[test]
fn park_then_reconnect_respawns_and_refreshes_status() {
    let dir = tempfile::tempdir().unwrap();
    let stores = test_stores(dir.path());
    seed_connection(&stores);
    // Seed an ACTIVE (not Slept) session with a small transcript — a Disconnected
    // session's persisted lifecycle is Active (D26).
    let mut state = fresh_state();
    state.lifecycle = SessionLifecycle::Active;
    stores.control.upsert_session(&state, 1_700_000_000_000).unwrap();
    for (id, ord) in [("item_0", 0), ("item_1", 1)] {
        seed_message_item(&*stores.transcript, ord, id, id);
    }

    let conn = ConnectionId::new("conn_1");
    let sid = SessionId::new("conv_1");

    // fetch_status returns idle (a `failed` session that healed across restart);
    // fetch_items returns the forward tail.
    let tail = item_list_from_messages(&["item_2"], false);
    let (api, _mock) = MockApi::with_status(
        VecDeque::from([Ok(SessionStatus::Idle)]),
        VecDeque::from([Ok(tail)]),
    );

    let (_ev_tx, ev_rx) = crossbeam_channel::bounded(64);
    let (up_tx, up_rx) = async_channel::bounded(64);
    let mut scheduler = FleetScheduler::new();
    scheduler
        .reconnect(&conn, &sid, ev_rx, up_tx, stores, test_clock(), api)
        .expect("reconnect respawns from Active disk");
    assert!(scheduler.is_running(&sid));
    assert!(scheduler.park_reason(&sid).is_none(), "reconnect clears park reason");

    // Forward catch-up materializes the tail past frontier 1.
    let mut saw_tail = false;
    while let Ok(u) = up_rx.recv_blocking() {
        if matches!(u, StreamUpdate::TranscriptAdvanced { committed_ordinal: 2 }) {
            saw_tail = true;
            break;
        }
    }
    assert!(saw_tail, "reconnect runs forward catch-up from disk frontier");

    scheduler.take_handle(&sid).unwrap().stop_and_join();
}
```

- [ ] **Step 5: Run it — expect FAIL** (`reconnect`/`park_reason`/`MockApi::with_status` undefined). Run: `cargo test -p lens-core park_then_reconnect -- --nocapture`.

- [ ] **Step 6: Implement `reconnect` + `parked` map + `park_reason`** (scheduler.rs, code above). Add the `parked` field to the struct + `new()`/`Default`.

- [ ] **Step 7: Execute the park→exit surgery in runloop.rs** per the pinned rewrite above: replace lines 561-604; delete all `parked: &mut bool` params + the `run`-loop events-arm split + the `Send`-while-parked branch. `cargo build -p lens-core` until clean.

- [ ] **Step 8: Add a runloop test that a terminal disconnect exits the actor** (runloop.rs tests mod). Feed a stream event that reduces to `Disconnected(Unauthorized)`; assert the actor emits `ActorOutcome::Parked{reason: Unauthorized}` and the thread joins (does not hang). Use the existing `one_output_item_done_event` fixtures as a model for constructing events.

- [ ] **Step 9: Run all — expect PASS.** `cargo test -p lens-core` && `cargo clippy -p lens-core --all-targets -- -D warnings` && `cargo fmt --check`.

- [ ] **Step 10: Commit.**

```bash
git add crates/lens-core/src/actor crates/lens-core/src/reduce
git commit -m "feat(state-model): D24/D25/D26 park=exit + user-gated reconnect + live-status re-read"
```

- [ ] **Step 11: MANDATORY cross-family seam review** of the diff (grok-4.5 via cursor-delegate). Focus: no blocked-forever path after events-arm becomes unconditional; reconnect status-refresh advisory-not-gating; `Parked` folding loses no reason. Apply findings, amend, re-run Step 9.

---

## Task 2: D27 send-failure 3-fate split (content-carrying outcomes)

**[LIGHT — closed spec, composer authors] [MANDATORY seam review — outcome enum shape]**

**Files:**
- Modify: `crates/lens-core/src/actor/api.rs` (CommandOutcome)
- Modify: `crates/lens-core/src/actor/runloop.rs:408-496` (Send handler), `runloop.rs:1034` (`rollback_pending`)
- Test: `runloop.rs` tests mod

**Interfaces:**
- Produces (CommandOutcome, final shape):
  - `SendAccepted { lens_pending_id, ack }` — unchanged.
  - `SendFailed { lens_pending_id, content: String, error: String }` — **+content**.
  - `SendDenied { lens_pending_id, content: String, reason: Option<String> }` — **+content**.
  - `SendPending { lens_pending_id }` — **new**; held/maybe-landed, bubble stays, no content.
  - (`SendRejected` **deleted** — D24 makes it moot; done in T1's `handle_command` edit.)
- **Invariant (state this in the doc comment):** a send outcome carries `content` **iff it removes the bubble**. Fail/Denied/Lost restore-to-composer → carry content; Pending keeps the bubble (the bubble is the home of the text) → no content.

**The rewrite (send `Err` arm, runloop.rs:469-492):** the `Mapped::rolls_back_send()` classification already splits "definite fail" (rollback) from "held" (keep). Map it to the three fates:

```rust
Err(e) => {
    let m = map_client_error(&e);
    let content = state
        .pending_user
        .iter()
        .find(|p| p.pending_id == lens_pending_id)
        .map(|p| p.content.clone())
        .unwrap_or_default();
    if m.rolls_back_send() {
        rollback_pending(state, &lens_pending_id);
        if !emit_pending_user(output, state) {
            return LoopControl::Break;
        }
        // Denied (Auth403) vs Failed (Network/404/other-4xx): both remove the bubble
        // and restore to composer; Denied carries the server reason.
        let outcome = match m {
            Mapped::LostAccess | Mapped::Denied => CommandOutcome::SendDenied {
                lens_pending_id,
                content,
                reason: Some(e.to_string()),
            },
            _ => CommandOutcome::SendFailed {
                lens_pending_id,
                content,
                error: e.to_string(),
            },
        };
        let _ = output.outcomes.send_blocking(ActorOutcome::Command(outcome));
    } else {
        // Held (5xx/401/ContractMismatch/Parse): bubble stays, soft pending, no content.
        let _ = output.outcomes.send_blocking(ActorOutcome::Command(
            CommandOutcome::SendPending { lens_pending_id },
        ));
    }
}
```

Also update the `Ok(ack) if ack.denied` arm (runloop.rs:438-448) to carry `content` in its `SendDenied` (read `content` before `rollback_pending`).

> **Note the `503 runner_unavailable` path is already correct:** it maps to `Mapped::ServerTransient` (5xx) which is `!rolls_back_send()`, so it hits the held branch → `SendPending`, bubble retained. No special-case needed.

- [ ] **Step 1: Reshape `CommandOutcome`** (api.rs): +content on SendFailed/SendDenied, add SendPending, doc the content-iff-removed invariant. `cargo build -p lens-core` — errors at the send site guide the rest.
- [ ] **Step 2: Write failing tests** (runloop.rs tests): (a) a `Network` send-error emits `SendFailed{content: "<typed text>", ..}` and clears the bubble; (b) a `Server{503}` send-error emits `SendPending` and **retains** the bubble; (c) an `Auth{403}` emits `SendDenied{content, ..}`. Drive via `MockApi` send_script.
- [ ] **Step 3: Run — expect FAIL.** `cargo test -p lens-core send_fate -- --nocapture`.
- [ ] **Step 4: Implement the Err-arm + denied-ack rewrite** (code above).
- [ ] **Step 5: Run — expect PASS.** Full green bar (test/clippy/fmt).
- [ ] **Step 6: Commit.**
```bash
git commit -am "feat(state-model): D27 send-failure 3-fate split — content-iff-bubble-removed"
```
- [ ] **Step 7: MANDATORY cross-family seam review** (grok-4.5). Focus: content read before `rollback_pending` (not after — the bubble is gone by then); held vs fail classification matches Table B. Apply, re-green.

---

## Task 4: `lens-drive` headless JSON-lines driver binary

*(Numbered T4 — it sits after T1/T2 so it targets the final `ActorOutcome`/`CommandOutcome` shapes. There is no T3; the outcome-shape lock is the reason for the ordering.)*

**[LIGHT — composer authors skeleton] [standard cross-family review]**

Built here (after T1/T2 lock the final `ActorOutcome`/`CommandOutcome` shapes) so it targets final types, and so the D30 live-verify rider (T5) and any re-check of T1/T2 can run *through* it against a live omnigent.

**Files:**
- Create: `crates/lens-drive/Cargo.toml`, `crates/lens-drive/src/main.rs`, `crates/lens-drive/README.md`
- Modify: workspace root `Cargo.toml` (add `crates/lens-drive` to members)

**Interfaces:**
- Consumes: `lens_core::actor::{spawn_actor, ActorHandle, ActorStores, SessionCommand}`, `ActorOutcome`, `StreamUpdate`; `lens_client` (connection + `Sessions` + stream); `lens_core::persist::{SqliteControlStore, SqliteTranscriptStore}`.
- CLI (keep tiny): `lens-drive --base-url <url> --session <conv_id> [--script <path>]`. Reads newline commands from `--script` or stdin: `send <text>`, `sleep`, `reconnect`, `stop`, `snapshot` (dump current SessionState). Emits **one JSON object per line** to stdout: `{"kind":"outcome","outcome":<ActorOutcome-as-json>}` and `{"kind":"state","state":<compact SessionState>}`.

**Bright line (put in README + a top-of-file comment):** lens-drive **dumps state as JSON, it never renders**. No markdown, no transcript layout, no virtualization. The moment it grows a rendering concern, that work belongs to `lens-ui`, not here. It is a second consumer of the §13.2 seam contracts (`StreamUpdate`/`SessionCommand`/`ActorOutcome`) — its value is proving those seams are drivable from outside the actor, and giving a repeatable harness for the live-verify riders.

- [ ] **Step 1: Scaffold the crate** — `Cargo.toml` (bin, deps on `lens-core`, `lens-client`, `serde_json`, `clap` or hand-rolled arg parse — prefer minimal), add to workspace members. `cargo build -p lens-drive`.
- [ ] **Step 2: Wire connect + stream + spawn_actor.** Open a real `lens-client` connection to `--base-url`, subscribe the stream for `--session`, open `SqliteControlStore`/`SqliteTranscriptStore` under a temp/`--data-dir` path, `spawn_actor`. (Reuse the connect+resolve-`conv_`-id pattern from `crates/lens-capture` — copy the ~20 lines; do NOT extract a shared lib yet, per the two-tools-isn't-an-umbrella decision.)
- [ ] **Step 3: Command loop.** Read stdin/script lines, map to `SessionCommand`, send on the handle; a background reader drains `handle.outcomes` and the `updates` channel, printing JSON lines. `snapshot` prints the current reduced state (obtain via a `Promote`/`Rebased` round-trip, or a dedicated read — simplest: print each `Rebased`/`TranscriptAdvanced` as it flows).
- [ ] **Step 4: Manual smoke against a live server** (installing-omnigent-from-source skill if none running): `lens-drive --base-url … --session … <<< $'send hello\nsnapshot\nstop'`; confirm JSON lines for the send outcome + state appear. No automated test asserted here (it's a live tool); a `--help` smoke + `cargo build` is the CI gate.
- [ ] **Step 5: README** (usage + the bright line).
- [ ] **Step 6: Commit.**
```bash
git add crates/lens-drive Cargo.toml
git commit -m "feat(lens-drive): headless JSON-lines actor driver (dumps state, never renders)"
```
- [ ] **Step 7: Cross-family review** (grok-4.5). Focus: no rendering creep; clean seam consumption; connect/resolve duplication is deliberate not accidental.
- [ ] **Step 8: D24/D27 live-verify riders through lens-drive.** Drive a real session: induce a park (e.g. `stop_session` out-of-band → `Disconnected`) and confirm the actor exits + `Parked` line appears + a scripted `reconnect` respawns and catches up; issue a send during a server blip and confirm `SendPending` retention. Record findings in the T7 memory update.

---

## ✅ PRE-BUILD REVIEW — COMPLETE (grok-4.5-xhigh, 2026-07-12) — 10 findings folded

The cross-family review ran and **refuted two of the author's own gap analyses** and found three additional serious correctness bugs. All 10 findings verified against code and folded into T5/T6 below. Corrections to the author's earlier assumptions:

- **D28 is genuinely THREE-way, not two-way (was Finding 4).** The wire `pending_inputs` DOES carry `content` (omnigent OpenAPI `additionalProperties:true`); `lens-client`'s `PendingInput` *deliberately dropped it* (`sessions.rs:77-79`: "Add it when a consumer needs it"). So path (2) is *not* contractually impossible — it just needs `PendingInput` widened by one shape-tolerant field. **T6 now widens `PendingInput` and implements the real three-way** (snapshot stamp → catch-up landed → lost). Collapsing to two-way would false-`SendLost` a native held send while the server still holds the input → duplicate on resend.
- **The SCHEMA_VERSION bump does NOT degrade old files to read-only (was Finding 5).** `db.rs:48-50`: `v < current → ReadWrite + re-stamp`; `TRANSCRIPT_DDL` is `CREATE TABLE IF NOT EXISTS` → it will NOT add the new columns to an existing file → a stamped-v2 file missing `provisional`/`call_id` breaks on first query. **T5 now carries an explicit column migration** (PRAGMA `table_info` check → `ALTER TABLE ADD COLUMN`), not a no-op. The "no rows today" fact reduces blast radius but does not make the old wording true; local P3-3a dev DBs would otherwise brick.

The three additional serious findings (all folded): **F1** store-frontier vs `next_ordinal` conflation → `UNIQUE(ordinal)` collision; **F2** the catch-up delta isn't materialized at the hook site (`CatchupResult` carries only events/commands); **F3** a D30 *folded re-fetch* is false-landed evidence for a held content-match → silent drop of a held duplicate send.

---

## Task 5: D30 scaffold-id dedup at persist + C2/C3/C4

**[HEAVY — algorithm-pinned] [MANDATORY seam review — durable-id reconcile]**

**Files:**
- Modify: `crates/lens-core/src/persist/schema.rs` (DDL + SCHEMA_VERSION)
- Modify: `crates/lens-core/src/persist/transcript.rs` (upsert provisional param, frontier, reconcile method)
- Modify: `crates/lens-core/src/persist/mod.rs` (`TranscriptStore` trait)
- Modify: `crates/lens-core/src/actor/runloop.rs` (`run` frontier seed :922-933, `commit_terminal_prefix` :1087, catch-up `upsert_catchup_item` :201 + `invoke_catchup_and_replay`; C3 recursion→iteration; C4 `RunCtx`)
- Test: `transcript.rs` tests + `runloop.rs` tests

**Interfaces:**
- Produces (TranscriptStore trait):
  - `upsert_item(&self, ordinal: i64, item: &Item, provisional: bool) -> Result<i64>` — **+provisional**. Live commits pass `true`; catch-up appends pass `false`.
  - **TWO distinct cursors (F1 — do NOT conflate):**
    - `store_frontier(&self) -> Result<Option<(i64, ItemId)>>` — newest **non-provisional** `(ordinal, item_id)`. Sole use: the `/items?after=<id>` catch-up cursor, so it never pages after a live/provisional id.
    - `next_ordinal_seed(&self) -> Result<i64>` — `MAX(ordinal)+1` over **ALL** rows (provisional included), because provisional rows occupy ordinal slots. Sole use: seeding the actor's append cursor. The old `frontier()` is REPLACED by these two; `run` seeds `next_ordinal` from `next_ordinal_seed()`, and catch-up pages from `store_frontier()`.
  - `reconcile_store_item(&self, store_item: &Item, live_key: &LiveKey) -> Result<ReconcileOutcome>` — folds a resident provisional row matching `live_key` into the store id; **F6-safe** (see below). Returns `Folded{ordinal}` or `NoMatch`.
  - `LiveKey` = `{ id: ItemId, call_id: Option<CallId> }` — the `id → call_id` precedence key. `call_id = Some(..)` iff the store item is a `FunctionCall` **or** `FunctionCallOutput` (F9 — both split; both write the `call_id` column on provisional commit).
  - `ReconcileOutcome` = `{ Folded { ordinal: i64 }, NoMatch }`.

**The scaffold-id model (pin exactly):**

Scaffold harnesses mint a fresh store id on persist ≠ the live SSE id; native = live==store. Lens commits live under the live id (provisional) and later catches up `/items` (store id). Without dedup that is two disk rows + a poisoned cursor (`/items?after=<live fc_* id>` is unknown to the server).

Fix, **uniform, no `if scaffold` branch:**
1. **Every live commit lands `provisional = 1`** (commit_terminal_prefix path).
2. **Catch-up reconciles each `/items` row against resident provisional rows** by key precedence **`id → call_id`**:
   - Compute `LiveKey` for the store row: `id` = the store row's id; `call_id` = `Some(..)` iff the item is a `FunctionCall`/`FunctionCallOutput` (the only kinds that split — probe-confirmed messages do NOT split).
   - `reconcile_store_item` looks for a resident **provisional** row that matches: **native/message ⇒ same `id`** (no-op rewrite, clears provisional); **scaffold tool ⇒ same `call_id`** (rewrite `item_id` live→store, **preserve `ordinal`**, clear provisional).
   - If a provisional row matched → the store row is folded in (no new ordinal). If none matched → append as a normal non-provisional row at `next_ordinal`.
3. **`store_frontier` = newest non-provisional id** → the catch-up cursor never pages after a live id (dissolves the poisoned cursor). **`next_ordinal` seeds from `next_ordinal_seed` = MAX over ALL rows (F1)** — provisional rows hold ordinals, so seeding from `store_frontier` would under-count and collide on `UNIQUE(ordinal)`. Additionally, on every `Folded{ordinal}` the actor does `next_ordinal = max(next_ordinal, ordinal + 1)`. **Benign-gap note (R2-3):** the F6 store-id-exists branch deletes a provisional row and returns the *surviving* (lower) store ordinal, so `max()` won't advance — a one-slot ordinal gap can remain where the deleted provisional sat. This is harmless: ordinals need only be **monotonic, not contiguous** (`load_items` is `ORDER BY ordinal`; the append cursor never reuses the gap → no collision). Do not add a recompute.

**Cost (document, don't optimize):** a healthy native session re-fetches from last-reconciled forward on reconnect (idempotent `id`-match, bounded). Opportunistic provisional-promotion is deferred.

**Schema edit + REAL migration (schema.rs + transcript.rs — F5):**
```rust
pub const SCHEMA_VERSION: u32 = 2; // was 1 — provisional + call_id columns (D30)
// TRANSCRIPT_DDL items table (fresh files) gains:
//   provisional INTEGER NOT NULL DEFAULT 0,
//   call_id     TEXT,
```
> **`CREATE TABLE IF NOT EXISTS` does NOT add columns to an existing file, and `db::open_db` UPGRADES a `v < current` file (ReadWrite + re-stamp) rather than degrading it (verified db.rs:48-50).** So a bump alone bricks any existing v1 transcript. `SqliteTranscriptStore::open` must, after `open_db`, run an idempotent column migration: `PRAGMA table_info(items)` → if `provisional`/`call_id` absent, `ALTER TABLE items ADD COLUMN provisional INTEGER NOT NULL DEFAULT 0` / `ADD COLUMN call_id TEXT`. **Run it UNCONDITIONALLY on every `ReadWrite` open (R2-7 caveat), NOT only when `open_db` reports a version bump** — `open_db` re-stamps the version *before* returning, so a crash between the stamp and the ALTER would otherwise leave a v2-stamped file missing the columns forever; a `table_info`-gated ALTER on every open is idempotent and self-heals that window. (Guard: swallow "duplicate column" on a race.) Pre-release there is no production data, but local P3-3a dev DBs exist — the migration keeps them working; a "wipe transcript DBs" policy is the cruder alternative. `SCHEMA_VERSION` is shared with the control store (schema.rs:5) — the control file also re-stamps v2, harmlessly (its `CREATE TABLE IF NOT EXISTS` needs no new columns); confirm at the seam review that control needs no migration.

**The reconcile method (transcript.rs) — the heart of the task, F6-safe:**
```rust
fn reconcile_store_item(&self, store_item: &Item, live_key: &LiveKey) -> Result<ReconcileOutcome> {
    self.guard_write()?;
    let tx = self.conn.unchecked_transaction()?; // single txn — F6
    // Precedence id → call_id; FunctionCall + FunctionCallOutput carry call_id (F9);
    // if two provisionals share a call_id, take the oldest: ORDER BY ordinal LIMIT 1.
    let matched_ordinal: Option<i64> = self.find_provisional_match(&tx, live_key)?;
    let outcome = match matched_ordinal {
        Some(ord) => {
            // F6: item_id is the PK. A PK-changing UPDATE fails if the store id ALREADY
            // exists as another row (duplicate catch-up page, or a prior NoMatch append).
            // So: if store id present → delete the provisional live row, keep the store row.
            // Else → rewrite the provisional row's item_id → store id, preserve ord, clear flag.
            let existing_store_ord: Option<i64> = tx.query_row(
                "SELECT ordinal FROM items WHERE item_id = ?1", [store_item.id.as_str()],
                |r| r.get(0)).optional()?;
            match existing_store_ord {
                // R2-3: store id already present (duplicate catch-up page / prior NoMatch
                // append). Delete the now-redundant provisional row, and REFRESH the
                // surviving store row from the current /items body (defensive — /items is
                // immutable per D23, but never keep a possibly-stale row). Report the
                // SURVIVING row's ordinal, not the deleted provisional's.
                Some(store_ord) => {
                    tx.execute("DELETE FROM items WHERE ordinal = ?1 AND provisional = 1", [ord])?;
                    tx.execute(
                        "UPDATE items SET kind = ?1, payload = ?2 WHERE item_id = ?3",
                        params![item_kind_token(&store_item.kind), json_string(store_item)?,
                                store_item.id.as_str()])?;
                    ReconcileOutcome::Folded { ordinal: store_ord }
                }
                // Normal fold: rewrite the provisional row's PK live→store id, preserve ord.
                None => {
                    tx.execute(
                        "UPDATE items SET item_id = ?1, live_seq = NULL, provisional = 0,
                           call_id = NULL, kind = ?2, payload = ?3 WHERE ordinal = ?4",
                        params![store_item.id.as_str(), item_kind_token(&store_item.kind),
                                json_string(store_item)?, ord])?;
                    ReconcileOutcome::Folded { ordinal: ord }
                }
            }
        }
        None => ReconcileOutcome::NoMatch, // caller appends fresh (non-provisional)
    };
    tx.commit()?;
    Ok(outcome)
}

// find_provisional_match(tx, live_key): SELECT ordinal FROM items WHERE provisional = 1 AND (
//   item_id = :id  OR  (:call_id IS NOT NULL AND call_id = :call_id)
// ) ORDER BY ordinal LIMIT 1     -- match the DEDICATED call_id column, not a JSON extract.
```

**F2 plumbing (consumed by T6):** the catch-up fetch loop (`run_catchup`, runloop.rs:230-305) upserts items straight to disk and today returns only `buffered_events`/`deferred_commands` (`CatchupResult`, runloop.rs:175-179) — **the fetched item bodies are not retained**. T5 extends the catch-up path to accumulate `Vec<String>` of **user-message texts from rows that reconciled `NoMatch` and were appended as NEW store rows** (a folded id-match is NOT a new landing — F3). Accumulate across all iterated catch-up rounds (C3). This vector is D28's `catchup_user_contents` input; without it T6 cannot run.

**C2/C3/C4 ride this diff — SEPARATE COMMITS (F7):**
- **C2 (frontier Err fail-closed):** runloop.rs:923-932 currently guesses `0` on a `frontier()` error → risks a `UNIQUE(ordinal)` collision. Change to: on `Err` from `next_ordinal_seed`/`store_frontier`, emit `PersistError` and **park** (`ActorOutcome::Parked` + return, per D24) — do not spawn with a guessed cursor.
- **C3 (recursion→iteration):** the `invoke_catchup_and_replay` → `replay_buffered_batch` → `finish_reconnected_catchup` → `invoke_catchup_and_replay` cycle (runloop.rs:652/716/860) nests a stack frame per buffered `Reconnected`. Flatten to a `while` loop. **Separate commit** — this is a control-flow refactor on the same path that owns D19 defer-commit ordering; bundling it with the persistence-semantics change makes a red reconnect test un-bisectable (F7).
- **C4 (`RunCtx` bundling):** the 5× `#[allow(clippy::too_many_arguments)]` (runloop.rs:500/630/758/818, spawn_actor_dual:110) — bundle the threaded refs into a `struct RunCtx<'a>` and pass `&mut RunCtx`. **Delegate to composer as a pure mechanical refactor**, done last, **separate commit**. Remove the `#[allow]`s.

- [ ] **Step 1: Schema + migration — add `provisional` + `call_id` columns, bump SCHEMA_VERSION to 2, add the `PRAGMA table_info` → `ALTER TABLE ADD COLUMN` migration in `SqliteTranscriptStore::open` (F5).** Write a failing test first: open a store, close it, hand-downgrade its `meta.schema_version` to 1 and drop the columns (or open with an old DDL), reopen → assert `provisional`/`call_id` exist and are queryable. `cargo test -p lens-core migrate -- --nocapture` (FAIL → implement → PASS).
- [ ] **Step 2: Write failing transcript tests** (transcript.rs tests): (a) `store_frontier_ignores_provisional` — a provisional row past a non-provisional one → `store_frontier` returns the non-provisional ordinal/id; (b) `next_ordinal_seed_counts_provisional` — same fixture → `next_ordinal_seed` returns MAX-all + 1 (F1); (c) `reconcile_scaffold_tool_folds_by_call_id` — provisional-commit a FunctionCall under `fc_*`, reconcile with a store row sharing `call_id`, fresh `msg_*` id → one row, ordinal preserved, id == store id, provisional cleared; (d) `reconcile_message_folds_by_id` — same-id no-op fold; (e) `reconcile_when_store_id_already_present_deletes_provisional` (F6) — pre-insert the store id as a row, then reconcile a provisional with matching key → provisional row deleted, store row intact, no PK error.
- [ ] **Step 3: Run — expect FAIL.** `cargo test -p lens-core reconcile -- --nocapture`.
- [ ] **Step 4: Implement** `upsert_item(+provisional)`, `store_frontier` (WHERE provisional=0), `next_ordinal_seed` (MAX all rows), `find_provisional_match`, F6-safe `reconcile_store_item`, `ReconcileOutcome`, `LiveKey`; extend the `TranscriptStore` trait + the test-mod mock impls (runloop.rs:1329 `MockTranscript`, `failing_transcript_stores`). `cargo build` clean.
- [ ] **Step 5: Wire the actor commit + catch-up paths (COMMIT 1)** — `commit_terminal_prefix` passes `provisional=true`; catch-up calls `reconcile_store_item`, appends only on `NoMatch`, and on `NoMatch`-append of a user message pushes its text into the F2 accumulator; on `Folded{ord}` advances `next_ordinal = max(next_ordinal, ord+1)` (F1); `run` seeds `next_ordinal` from `next_ordinal_seed` with **C2** fail-closed; catch-up pages from `store_frontier`. Existing golden-order + refire tests (runloop.rs:1745/1825) stay green.
```bash
git commit -am "feat(state-model): D30 scaffold-id dedup-at-persist — provisional + dual-cursor + id→call_id fold"
```
- [ ] **Step 6: C3 (COMMIT 2)** — flatten the catch-up recursion to iteration; preserve D19 defer-commit ordering + the F2 accumulation across rounds. Re-run all reconnect/catch-up tests.
```bash
git commit -am "refactor(state-model): C3 catch-up recursion → iteration"
```
- [ ] **Step 7: C4 (COMMIT 3)** — delegate the `RunCtx` bundling to composer; remove the 5× `#[allow(too_many_arguments)]`. Full green bar.
```bash
git commit -am "refactor(state-model): C4 RunCtx arg-bundling"
```
- [ ] **Step 8: MANDATORY cross-family seam review** (grok-4.5) over commits 1–3. Focus: F6 PK-rewrite branch (store-id-exists → delete-provisional) is collision-free; dual-cursor (F1) can't strand a live tail nor collide; migration (F5) is idempotent + control-store unaffected; C3 iteration preserves D19 ordering + F2 accumulation; C2 park-on-Err doesn't dead-lock. Apply, re-green.
- [ ] **Step 9: D30 live-verify rider through lens-drive** — drive a real **scaffold** harness session (e.g. an SDK-driven one), issue a tool-calling turn, and confirm via `snapshot` + on-disk row inspection that a tool item lands as ONE row (folded), not two. Record in T7 memory.

---

## Task 6: D28 held-bubble landed-detection → `SendLost`

**[HEAVY — algorithm-pinned] [MANDATORY seam review — reconcile correctness]**

**Files:**
- Modify: `crates/lens-client/src/sessions.rs` (widen `PendingInput` — F4)
- Modify: `crates/lens-core/src/reduce/reconcile.rs` (three-way held reconcile)
- Modify: `crates/lens-core/src/actor/runloop.rs` (call the reconciler on the in-actor `Reconnected` path; emit `SendLost{content}`)
- Test: `sessions.rs` tests + `reconcile.rs` tests + `runloop.rs` tests

**Interfaces:**
- Produces:
  - `PendingInput { pending_id: String, content: Option<String> }` — **+content** (F4). Shape-tolerant (`Option`, `#[serde(default)]`) — the wire field is `additionalProperties:true`; model it now that D28 consumes it.
  - `reconcile_held_landed(pending, snapshot_pending_inputs, catchup_new_user_contents) -> Vec<LostSend>` where `LostSend = { lens_pending_id: String, content: String }`.
- Consumes: `state.pending_user`; the snapshot's `pending_inputs` (see the **R2-1 data-path note below** — it is NOT available at the hook site today); and **only the user-message texts of catch-up rows that reconciled `NoMatch` and were appended** (the T5/F2 accumulator — NOT the full fetched set, NOT folded rows).

**R2-1 — the snapshot `pending_inputs` must be plumbed to the hook site (round-2 finding 1).** `fold_snapshot` (reduce/snapshot.rs:72-81) consumes the snapshot and emits only the marker `StreamUpdate::SnapshotRestored` — the `pending_inputs[].content` is discarded during reduce, so it is NOT resident at the catch-up finish site. Fix: **widen `StreamUpdate::SnapshotRestored` to carry `Vec<PendingInput>`** (the snapshot's pending inputs), or capture them in `apply_reduced_batch` before `reduce` drops them. Path 1 cannot stamp without this. (This is the same materialize-before-it's-gone class as F2 — I introduced it when adding path 1.) Add a runloop test where path 1 stamping requires the plumbed `content`.

**The THREE-way model (F4 — corrected from the pre-build review):**

The wire carries per-input `content`; once `PendingInput` is widened, all three D28 paths are reachable. For each **held** bubble (both `server_pending_id` and `store_item_id` are `None`) with content `C`, evaluate **in this order (F8)**:
1. **`C` matches a `snapshot_pending_inputs[].content`** ⇒ *landed-pending* ⇒ **stamp** the bubble's `server_pending_id` from that input's `pending_id`, **keep** it (it's a real, still-queued input the server holds — dropping/restoring here would duplicate on resend). Consume that pending-input slot.
2. **`C` matches a `catchup_new_user_contents` entry** ⇒ *landed* ⇒ drop the bubble silently (it materialized as a real store item). Consume that delta slot.
3. **Neither** ⇒ *lost* ⇒ emit `SendLost{lens_pending_id, content}`, remove the bubble ⇒ UI restores to composer.

**Ordering matters (F8):** path 1 before path 2 before path 3. A held send the server still lists as pending must be *kept+stamped*, not restored, even if some homonym text also appears in the catch-up delta.

**Bias — UNIQUE + TEMPORAL match only (pin; refines the spec's FIFO-min-match — round-2 findings 2 & 4).** Both reviewers independently found the spec's "drop `min(N,M)` FIFO" rule *unsafe* for duplicate content — it stamps an arbitrary `pending_id` (path 1) or silently drops an unrelated same-content message (path 2). So the pinned rule is stricter than the spec text, with the same conservative intent:
- **Uniqueness:** act (stamp in path 1 / drop in path 2) **only when the match is unambiguous** — exactly one held bubble with content `C` *and* exactly one available slot with content `C`. `pending_inputs` has **no ordering contract** (in-memory index — generated.rs:8747), so a positional FIFO stamp can bind the wrong `pending_id` → the later `session.input.consumed` (which keys on `server_pending_id`, reconcile.rs) then fails to clear it → a stale bubble.
- **Temporal lower-bound (path 2):** a held send can only have landed *after* it was typed. Only count a catch-up user row as a landing for a held bubble if the row's `created_at ≥ bubble.created_at`. This screens out an unrelated pre-existing same-content message in the gap.
- **Ambiguity resolution:** any duplicate/ambiguous group ⇒ **path 1: leave unstamped + keep** (re-evaluated next reconnect — no wrong id, no loss); **path 2: `SendLost`/restore** (visible, user-gated duplicate beats silent data loss).
- **Residual (document):** even unique + temporal content-match cannot distinguish "my held send landed" from "an unrelated identical message landed after I typed mine." This is inherent to content-matching without an idempotency key; the robust fix is the deferred omnigent **client-message-id** (spec §2.4 / memory). The bias makes the residual a *visible duplicate*, never a silent drop. Frontier-anchoring also deferred.

**Content extraction (F10 — pin):** a "user-message" is an item with `role == User`; its content is the concatenation of its `output_text`/`input_text` block texts in order (define one `fn user_text(item) -> Option<String>` helper and use it for both the catch-up delta and the match). `pending_inputs[].content` compares against the same normalized form.

**Why the delta excludes folded rows (F3):** a D30 *folded* catch-up row is a re-fetch of an item already committed live — it is NOT new evidence that a *held* send landed. If a first "hello" landed live (provisional, later folded) and a second "hello" is held after a 5xx, feeding the folded first "hello" into path 2 would silently drop the second — data loss. Only `NoMatch`-appended user rows are genuine new landings.

**Where it runs (pin):** only on the **in-actor stream reconnect (Path 1)** — `pending_user` RAM-intact. Park→respawn (Path 2) lost `pending_user` on actor exit; that's D29/arch-B, not this task. Call `reconcile_held_landed` inside the catch-up finish path **after** items fold, with the T5/F2 accumulator + the snapshot `pending_inputs`. (`reconcile_snapshot` still handles *stamped* bubbles via `pending_inputs` by id — leave it; this is additive for the held set.)

```rust
/// D28: three-way held-bubble reconcile. Path 1 (snapshot pending_inputs → stamp+keep),
/// path 2 (catch-up NEW user rows → drop), path 3 (else → SendLost). Conservative:
/// ambiguity → lost, never a silent drop. FIFO oldest-first within equal-content groups.
pub fn reconcile_held_landed(
    pending: &mut Vec<PendingUserMessage>,
    snapshot_pending_inputs: &[PendingInput], // widened: carries content
    catchup_new_user_contents: &[String],     // NoMatch-appended user texts only (F3)
) -> Vec<LostSend> {
    // UNIQUE + TEMPORAL match (round-2). Walk `pending` in FIFO order:
    //   held bubble C (created_at = t):
    //     path1: if EXACTLY ONE pending_input has content==C (unique) → stamp its
    //            pending_id, consume it, KEEP. If >1 candidate/collision → leave
    //            unstamped + KEEP (ambiguous; re-evaluated next reconnect).
    //     path2: else if EXACTLY ONE unconsumed delta slot has content==C AND that
    //            row's created_at >= t (temporal) → consume it, DROP (landed).
    //            If ambiguous/none → fall through.
    //     path3: else → push LostSend{pending_id, C}; DROP (actor emits SendLost).
    //   non-held bubble → KEEP (reconcile_snapshot's job).
    todo!("unique+temporal index walk per the comment — deterministic; see tests")
}
```
> The `todo!` marks the shape, not a placeholder — the three ordered paths + the uniqueness/temporal guards are fully pinned above. The reviewer confirms: path order, unique-only stamping, temporal lower-bound, ambiguity→keep/lost.

- [ ] **Step 1: Widen `PendingInput`** (sessions.rs) — add `content: Option<String>` with `#[serde(default)]`; update the doc comment (was "add when a consumer needs it" — now it does). Add a deserialize test: a `pending_inputs` fixture with `content` populates the field; one without leaves `None`. Full green bar for `-p lens-client`. **Cross-family review this contract-surface change** (touches hardened lens-client — memory `lens-client-foundation-gotchas`).
- [ ] **Step 2: Plumb snapshot `pending_inputs` to the hook site (R2-1)** — widen `StreamUpdate::SnapshotRestored` to carry `Vec<PendingInput>` (or capture in `apply_reduced_batch` before `reduce`); update `fold_snapshot` (reduce/snapshot.rs) + all `SnapshotRestored` match sites (runloop.rs:533, tests). Verify the existing `detailed_mode_emits_rebased_after_snapshot_restored` test still passes. Commit this plumbing separately (it's a `StreamUpdate` contract change — cross-family review).
- [ ] **Step 3: Write failing reconcile tests** (reconcile.rs tests): (a) `held_matching_unique_pending_input_is_stamped_and_kept` — held "hi" (t=100), `pending_inputs=[{p9,"hi"}]`, delta `[]` → empty `lost`, bubble kept with `server_pending_id==Some("p9")`; (b) `held_landed_in_catchup_delta_dropped` — held "hi" (t=100), inputs `[]`, delta `[("hi", created_at=200)]` → empty `lost`, bubble gone; (c) `held_absent_everywhere_is_lost`; (d) `path1_precedes_path2`; (e) **`duplicate_pending_input_content_left_unstamped`** — two `pending_inputs` both "hi", one held "hi" → NOT stamped (ambiguous), kept, empty `lost`; (f) **`temporal_screens_older_gap_message`** — held "hi" (t=200), delta `[("hi", created_at=100)]` → the older row does NOT count as a landing → `SendLost` (t=100 < 200); (g) `stamped_bubble_untouched`.
- [ ] **Step 4: Run — expect FAIL.** `cargo test -p lens-core reconcile_held -- --nocapture`.
- [ ] **Step 5: Implement `reconcile_held_landed` + `LostSend` + the `user_text` helper** per the unique+temporal algorithm. (Requires the T5/F2 accumulator to carry each row's `created_at`, not just its text — thread that through.)
- [ ] **Step 6: Run — expect PASS** (unit level).
- [ ] **Step 7: Wire the actor hook** — after catch-up folds items on the in-actor reconnect, call `reconcile_held_landed(&mut state.pending_user, snapshot_pending_inputs, &catchup_new_user_rows)`, emit `ActorOutcome::SendLost{lens_pending_id, content}` per `LostSend`, then `emit_pending_user`. Runloop tests: (i) held bubble absent from inputs + NoMatch-delta → `SendLost` + cleared; (ii) delta is a *folded* copy of the held content → still `SendLost` (F3 — folded ≠ landing).
- [ ] **Step 8: Run all — expect PASS.** Full green bar.
- [ ] **Step 9: Commit.**
```bash
git commit -am "feat(state-model): D28 three-way held reconcile (unique+temporal stamp/land/lost) + widen PendingInput.content"
```
- [ ] **Step 10: MANDATORY cross-family seam review** (grok-4.5). Focus: path order 1→2→3 (F8); unique-only stamping + temporal bound (R2 findings 2 & 4 — never stamp an arbitrary id, never drop on ambiguity); folded rows excluded (F3); runs only on Path 1 (RAM-intact); `PendingInput` + `SnapshotRestored` widenings are shape-tolerant.

---

## Task 7: Docs, amendments, and closeout

**[LIGHT — mechanical] [Opus review of the amendment wording]**

**Files:**
- Modify: `docs/design/app-architecture-and-state-model.md` §4.1 (identity/ordering/dedup bullet) + §6.2 (schema sketch)
- Modify: `docs/STATUS.md` (+ STATUS-ARCHIVE.md entry)
- Modify/Create: memory files

- [ ] **Step 1: Apply the D30 app-arch amendment the spec §7.1 flagged as not-yet-edited.** In §4.1's "Identity, ordering, dedup" bullet, document the `id → call_id` precedence + provisional flag + store-frontier cursor (scaffold tools split, messages do not). In §6.2's schema sketch, add the `provisional` + `call_id` columns to the transcript `items` table. Cross-reference spec §2.4 D30.
- [ ] **Step 2: Record the contract facts** as a memory (`state-model-p3-3b-contract-gaps`): (a) omnigent `pending_inputs[]` carries `{pending_id, content}` — lens-client originally modeled only `pending_id`; D28 widened it → real three-way reconcile (stamp/land/lost); (b) content-match is heuristic (no idempotency key) → the robust fix is the deferred omnigent **client-message-id** request; (c) `SessionApi` gained `fetch_status` for D26. Link `[[omnigent-two-id-space-reconciliation]]`, `[[state-model-p3-3b-grilling]]`.
- [ ] **Step 3: File the omnigent client-message-id contract feature-request** (issue text in the memory or the omnigent sibling repo tracker) — the robust fix for both D28 held-send dedup and scaffold tool dedup.
- [ ] **Step 4: STATUS update** — P3-3b DONE; forward pointer to the next thread (Bucket B viewport / `lens-ui` crate, or `lens-ui` arch-B composer-draft). Per the end-of-session-status-update memory rule.
- [ ] **Step 5: Final whole-branch review** (Opus synthesis of the seam-review findings + a codex/gpt-5.5 cross-family pass over the full P3-3b diff, per review-spend-policy: one consolidated end-of-branch review). Apply, re-green.
- [ ] **Step 6: Commit + integrate to main** (per integration-workflow: all tests pass + zero warnings/dead code → merge straight to main).
```bash
git commit -am "docs(state-model): P3-3b closeout — app-arch §4.1/§6.2 D30 amendment + STATUS + memory"
```

---

## Self-Review (author's checklist — run against spec §2.4)

- **D24** → T1 (park=exit surgery). ✓
- **D25** → T1 (`reconnect` = respawn, `parked` map, nothing auto-terminal). ✓
- **D26** → T1 (`fetch_status` re-read on reconnect; lifecycle stays Active). ✓ **Surfaced gap:** `SessionApi` had no status read — added as a task step.
- **D27** → T2 (3-fate content-carrying split; SendPending; delete SendRejected). ✓
- **D28** → T6 (**three-way** held reconcile: snapshot stamp → catch-up landed → lost). ✓ **Pre-build review corrected the author:** the wire DOES carry `pending_inputs[].content` — T6 widens `PendingInput` and implements the real three-way (F4). Delta excludes D30-folded rows (F3); `CatchupResult` extended to carry the NoMatch-appended user texts (F2).
- **D29** → honored by omission: `pending_user` stays RAM-only (global constraint); D28 runs Path 1 only; no survival persistence built. ✓ Guardrail (outcome enum can express a future "landed→clear") preserved — `SendLost` restore-on-loss is fail-safe.
- **D30** → T5 (dedup-at-persist, provisional + **dual cursor** + id→call_id). ✓ **Pre-build review corrected the author:** store-frontier and `next_ordinal` are SEPARATE cursors (F1); PK-rewrite is store-id-exists-safe (F6); the SCHEMA_VERSION bump carries a real `ALTER TABLE` migration, not a read-only degrade (F5).
- **D31** → T5 (C2 fail-closed, C3 iteration **as a separate commit — F7**, C4 RunCtx separate commit; C5 deferred — not planned, correct). ✓
- **lens-drive** → T4. ✓
- **§7.1 unapplied D30 amendment** → T7 Step 1. ✓
- **Task order** matches the handoff hint (D24 → D27 → tool → D30 → D28 → docs); the pre-build gate ran and its 10 findings are folded above.
- **Placeholder scan:** `reconcile_held_landed` carries a `todo!` that marks impl-shape only — the three-path FIFO algorithm is fully pinned in prose + the inline comment, with test cases (a–f) that constrain it. No open-ended placeholders.
- **Type consistency:** `LiveKey`/`ReconcileOutcome` (T5), `LostSend` (T6), `PendingInput.content` (T6 Step 1) defined where produced; `SendFailed/SendDenied/SendPending/SendLost` shapes consistent between api.rs (T2) and emission sites (T2/T6); `store_frontier`/`next_ordinal_seed` replace the old `frontier()` at every call site (T5 Step 4); `fetch_status` consistent between api.rs (T1 Step 3) and scheduler (T1 Step 6).

## Pre-build review ledger (grok-4.5-xhigh, 2026-07-12)

| # | Finding | Verdict | Folded into |
|---|---|---|---|
| F1 | store-frontier vs `next_ordinal` conflation → `UNIQUE(ordinal)` collision | CONFIRMED | T5 dual-cursor |
| F2 | catch-up delta not materialized (`CatchupResult` carries only events/cmds) | CONFIRMED | T5 F2 accumulator |
| F3 | D30-folded re-fetch = false landed-evidence → silent drop of held dup | NEW | T6 delta excludes folded |
| F4 | wire `pending_inputs.content` exists; two-way was wrong | REFUTED author | T6 widen PendingInput, three-way |
| F5 | SCHEMA_VERSION bump upgrades (not read-only) + CREATE won't ALTER | REFUTED author | T5 explicit migration |
| F6 | PK-changing UPDATE unsafe if store id already exists | NEW | T5 store-id-exists branch |
| F7 | C3 bundled with D30 = un-bisectable | CONFIRMED | T5 separate commits |
| F8 | T6 path ordering vs snapshot underspecified | NEW | T6 path 1→2→3 pin |
| F9 | call_id on both FunctionCall + Output; same-call_id tiebreak | NEW | T5 LiveKey + ORDER BY |
| F10 | "user-message content" extraction ambiguous | NEW | T6 `user_text` helper |

**Round 2 (gpt-5.5 via codex, 2026-07-12) — adversarial verify of the round-1 fixes + T1–T4:**

| # | Finding | Verdict | Folded into |
|---|---|---|---|
| R2-1 | snapshot `pending_inputs` discarded by `fold_snapshot` → path 1 has no data at hook site | FIX-INCOMPLETE | T6 widen `SnapshotRestored` (Step 2) |
| R2-2 | FIFO stamp binds wrong `pending_id` for duplicate content (no wire ordering) | NEW BUG | T6 unique-only stamping |
| R2-3 | F6 delete branch keeps stale store row + returns wrong ordinal | NEW BUG | T5 refresh store row + surviving ordinal; benign-gap note |
| R2-4 | "NoMatch-appended user row" isn't sufficient landed evidence (unrelated same-content) | FIX-INCOMPLETE | T6 temporal lower-bound + ambiguity→lost |
| R2-5 | scheduler `parked` bookkeeping racy (`AlreadyRunning` on dead actor) | FIX-INCOMPLETE | T1 `mark_parked` atomic |
| R2-6 | **Part C: D28 dead code under D24?** | OK-VERIFIED (not dead) | — (Path 1 = transient reconnect, actor alive) |
| R2-7 | T1/T2/F1/F5 sound; migration must run unconditionally per open | OK + caveat | T5 unconditional migration |
