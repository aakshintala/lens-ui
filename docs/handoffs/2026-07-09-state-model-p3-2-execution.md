# Handoff — execute state-model P3-2 (command semantics) — 2026-07-09

## TL;DR for the next session

The P3-2 plan is **written, cross-family reviewed, committed, and execution-ready.**
Start a **fresh session**, read the plan, and execute it subagent-driven (same shape
as P3-1). Nothing is blocking.

- **Plan:** `docs/superpowers/plans/2026-07-09-state-model-p3-2-command-semantics.md`
  (commit `bc3082d`, on `main`, **not pushed**). 10 TDD tasks.
- **Spec (SSOT):** `docs/superpowers/specs/2026-07-08-state-model-engine-design.md`
  §2.2 (D16, D18), §7, §13.1. Design-doc §13.1 (`docs/design/app-architecture-and-state-model.md`
  L1190–1237) is **already amended** with Tables A/B — Task 9 verifies, does not rewrite.
- **P3-1 foundation** this builds on: merged `1096a8c..f7c9a64`; the actor lives in
  `crates/lens-core/src/actor/`, the replica in `crates/lens-store/`.

## What P3-2 delivers

D16 (optimistic-send reconcile) + D18 (§13.1 error-mapping split), plus the three
P3-1 deferrals (M2 load-bearing, M1 optional, Nit). Task order:

1. Record the D16 ack fixture (finding already resolved — see below)
2. Plumb `cleared_pending_id` on `InputConsumed` (**lens-client prereq grok found**)
3. Model `pending_inputs` on `SessionSnapshot` (**lens-client prereq grok found**)
4. `PendingUserMessage` restructure + value-carrying `PendingUserChanged`
5. Sweep P3-1 deferrals (M2 must land; M1 optional; Nit)
6. `SessionCommand::Send` — optimistic bubble, POST, ack-stamp, rollback **[review seam]**
7. Reconcile precedence (1)→(2)→(3), live + reconnect **[review seam]**
8. D18 Table A — `Disconnected{reason}` → park/stop + `reconcile_in_flight` **[review seam]**
9. D18 Table B — `ClientError` → command outcomes + introspection ring **[review seam]**
10. Command-interleaving matrix + full P3-2 gate

## The D16 live-verify rider is RESOLVED — do NOT re-run it as a gate

Verified this session against **live omnigent 0.4.0 (pinned `31669e1b`)** and the route
source (`server/routes/sessions.py:19368-19379`, a bare dict return, no `response_model`
coercion):

- POST `/v1/sessions/{id}/events` ack is a **non-empty** body. Live: `{"queued":true,"item_id":"msg_…"}` (HTTP 202).
- Exactly **one** id per message POST: **non-native → `item_id`**; **healthy native → `pending_id`**.
- Precedence (1) native-by-`pending_id` + (2) store-by-`item_id` are the **common paths**;
  (3) content/ordinal is **defensive-only**.
- **GOTCHA (Tasks 6/7 must not code this away):** `native ⇏ pending_id`. A native session
  whose terminal is down fails the ensure-probe → server returns the failure-turn's
  **`item_id`**, not `pending_id`. Reconcile keys on *whichever id is present*, never on a
  harness/native flag. Test the `item_id`-only native-down shape explicitly.

Details in memory `state-model-p3-grilling` (D16 rider CONFIRMED block).

## The two prerequisites grok found (verified real — read these before Task 6)

Neither was in the original P3-2 handoff; grok surfaced them by reading the code and
both check out against the tree:

1. **`SessionEvent::InputConsumed` drops `cleared_pending_id`.** `RawInputConsumedData`
   (`crates/lens-client/src/stream/event.rs:314`) parses only `item_id` + `type`. The field
   IS in `generated.rs`. D16 precedence-(1) live native reconcile needs it. → **Task 2.**
   (Safe to add: lens-core's only match site is `folds.rs:132` `InputConsumed { .. }`, uses `..`.)
2. **`SessionSnapshot` does not model `pending_inputs`.** It's on the wire (6 hits in
   `generated.rs`; the live create-session response includes `"pending_inputs":[]`). Native
   reconnect re-hydrate needs it. → **Task 3.** Use the existing `de_null_default` pattern
   (`sessions.rs:34`) for null-tolerance.

## Execution guidance (mirror P3-1)

- **Subagent-driven-development.** Implementers = composer-2.5 (`cursor-delegate`) per
  CLAUDE.md. Per-task review = cross-family vs the composer author (Opus Agent; codex is a
  free gpt-5.5 path *if credits are back* — they were exhausted at P3-1's end, verify).
- **Mandatory review seams:** Tasks 6 & 7 (temporal send/reconcile) and Tasks 8 & 9
  (error/lifecycle transitions). Consolidated whole-branch review at close.
- **Gate per task:** `cargo fmt --check` + `cargo clippy --all-targets` (0 warn) + `cargo test`,
  scoped to the touched crate(s). `crates/lens-client/src/generated.rs` is NEVER hand-edited
  (confirm `git diff --stat` on it is empty).
- **The value-carrying-completeness rule bit P3-1 three times** — every `SessionState` field
  the reducer writes MUST emit a carrying `StreamUpdate` delta or the gpui replica silently
  misses it. New `PendingUserMessage` fields ride `PendingUserChanged(vec)` + an `apply` arm.
  See memory `state-model-p3-1-actor-foundation`.
- **`SessionApi` injection ripple (Risk 8a):** adding `api` to `spawn_actor`/`spawn_actor_dual`
  touches every P3-1 call site (walking-skeleton test, actor unit tests). `Box<dyn SessionApi
  + Send>` (Send-not-Sync, moved to the OS thread — same as `Box<dyn Clock + Send>`). Grep
  `spawn_actor` for the exact count before Task 6.
- **Risk 5a:** the actor's `crossbeam::Select` services nothing (incl. `Stop`) while blocked
  in `send_event` — require a finite HTTP timeout; Task 10 matrix asserts Stop-during-Send
  joins within ~timeout.

## Environment state (left ready)

- `../omnigent` checkout is on the pinned **`v0.4.0` (`31669e1b`)** (detached HEAD — correct
  resting state per `installing-omnigent-from-source`). Editable install + background daemon
  (`127.0.0.1:6767`) both serve 0.4.0. It was previously off-contract on `main`/0.5.0.dev0.
- No live omnigent needed to execute P3-2 — all tasks use scripted mock `SessionApi` +
  crossbeam events. The rider that needed a live server is already closed.

## Open decisions the executor owns (flagged in the plan, don't guess)

- Exact wire type of `pending_inputs[].content` (string vs content blocks) — Task 3, cite
  `generated.rs`/openapi; keep `serde_json::Value` out of lens-core.
- Whether `ClientError::Auth` distinguishes 403 or it arrives as `Server{status:403}` —
  Task 9, read the lens-client HTTP decode paths.
- M1 (Task 5) is optional — land only if regression tests are green and the reviewer is
  comfortable; otherwise defer to its own change. M2 must land regardless.
