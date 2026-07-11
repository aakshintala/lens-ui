# Handoff — execute state-model P3-3a (lifecycle core) — 2026-07-10

## TL;DR for the next session

The **P3-3a plan is written, grilled, and finalized** — all design decisions locked,
D19 source-verified against omnigent `31669e1b`. **Start a fresh session and execute it
subagent-driven** (same shape as P3-1/P3-2). Nothing is blocking.

- **Plan (execute this):** `docs/superpowers/plans/2026-07-10-state-model-p3-3a-lifecycle-core.md`
  — 8 tasks, TDD, `- [ ]` checkboxes. Header names the REQUIRED SUB-SKILL
  (`superpowers:subagent-driven-development`).
- **Spec SSOT:** `docs/superpowers/specs/2026-07-08-state-model-engine-design.md` §2.3 (D19–D23).
- **Builds on:** merged P3-1 (`crates/lens-core/src/actor/`, `crates/lens-store/`) + P3-2
  (D16/D18). This plan **revises** merged P3-1 code (item deltas, apply bridge) — deliberate.

## Update 2026-07-10 (later) — omnigent bumped 0.4.0 → 0.5.1; two P3-3a risks retired

A later session in this thread bumped the pin and de-risked two things this plan rests on.
**Nothing here changes the plan — both results confirm its assumptions.** (Details:
memory `[[omnigent-pin-bump-0-3-0]]`, `[[contract-coverage-gap-2026-07]]`; STATUS 2026-07-10.)

- **Pin is now `v0.5.1` (commit `08285468`), vendored `vendor/omnigent-0.5.1/`.** Contract
  delta vs 0.4.0 was all additions and **touched NONE of P3-3a's surfaces** — `/items`
  (cursor pagination), `GET /stream` (no-replay), and the snapshot are byte-for-byte the
  same. So the D19 source-verification below (done against `31669e1b`) **still holds** — the
  `/items` item-id-cursor resume path is unchanged. The two new SSE events
  (`session.mcp_startup`, `response.policy_denied`) are modeled marker-only in `folds.rs`;
  they are NOT transcript items and do not affect item-lifecycle (Task 3).
- **`turn.*` deferral VALIDATED live → Task 2 quiescence keying is correct.** Drove real
  turns across 4 harnesses (claude-sdk/codex/opencode/cursor); **no `turn.*` fired on any** —
  every harness expresses turn lifecycle via `response.in_progress` → `response.completed`/
  `.failed` + `session.status`. So `transient_work_outstanding()`/`is_quiesced()` keying on
  the response lifecycle (NOT on a `turn.*` family) is provably right; don't add `turn.*`.
- **Runner infra for Task 7's D17 live-verify:** the "offline runner" state is just no host
  daemon attached. Bring one up with `omnigent host http://127.0.0.1:6767 --non-interactive`;
  drive turns headlessly via the API (`omnigent run --harness … --server …` crashes on the
  missing TTY but creates the session + attaches the runner first). `omnigent stop` to tear
  down. Full recipe in `[[contract-coverage-gap-2026-07]]`.

## Execution protocol (per CLAUDE.md)

- **Author each task** with `cursor-delegate` / **composer-2.5**.
- **Review between tasks** with Opus (inline).
- **MANDATORY cross-family review at the three seams — Tasks 3, 4, 5 — with
  grok-4.5 via `cursor-delegate`** (a family other than the composer author). These are
  the actor-mutation (3), temporal catch-up (4), and subtractive-lens-client (5) seams.
  Mind Cursor-credit cost (`[[review-spend-policy]]`) — but the user explicitly asked for
  the third grok pass on Task 4, so all three seams get it.
- **Gate:** `cargo run -p xtask -- gate` green at every task boundary.
- **Integrate + PUSH after completion** — the user directed: push this branch along with
  the implementation once done (Task 8). Solo-project ff-merge to `main` if on a branch.

## The five locked decisions (do not re-litigate — folded into the plan)

1. **Ordinal assignment = commit-terminal-prefix** (Task 3). Walk `state.items` front→back;
   commit each terminal item at `next_ordinal++`, STOP at the first non-terminal. Dense
   contiguous ordinals + transcript order + trivial watermark. Rests on: an in-progress
   function call completes before any later item finalizes (golden-capture true; pin-and-verify).
2. **Reducer emits no item signal** (Task 3). `push_item` mutates `state.items`, returns no
   delta; the actor derives persistence by scanning the working set. Matches spec's "delete
   item-delta emission."
3. **`ordinal=items.ordinal` on conflict** (Task 3). A far-back re-fire refreshes payload
   without moving the row (idempotent-by-id). `reconcile()` re-stamps in its own txn, unaffected.
4. **`last_seen_seq` deleted** (Task 1) from `SessionState` + P2 schema/map/control. Vestigial:
   no producer, no consumer; D19's disk item-id frontier is the resume cursor. Sleep flush =
   `lifecycle=Slept` only. The lens-client reader's OWN `last_seen_seq` local (live overlap
   dedup) is a different thing — **do not touch it.**
5. **Scaffold `fc_*` double-commit deferred to P3-3b** (Tasks 3/4). Key durable rows on
   `Item::id()`; native sessions correct; scaffold hazard flagged `TODO(P3-3b, scaffold-id)`
   for the reviewer.

## Source-verification results (omnigent v0.4.0 `31669e1b`) — D19 grounded

Done this session at the user's request, before committing to D19:
- **`/items` is item-id cursor-paginated** (`after`/`before` = item id, `order`, `limit`;
  no seq param) — `sessions.py:16801 list_session_items`.
- **`GET /stream` is subscribe-from-now, no replay, no resume token** (params: `session_id`,
  `idle` only) — docstring: *"Does NOT replay history; clients reconcile via the snapshot
  endpoint"* — `sessions.py:19387`.
- **`sequence_number` is per-stream, assigned at serialize time, `None` on many events** —
  `schemas.py:2253`. Not a durable resume cursor. `last_event_seq` (heartbeat) is gap-
  DETECTION only, not replay. ⟹ D19's item-id frontier resume is the ONLY durable path; holds.
- **Scaffold two-id-space confirmed:** `NewConversationItem` has no id (`entities/conversation.py:652`);
  the store mints its own on `append()` (`:683`); the web UI dedupes live vs `/items` in one
  ephemeral `blocks` list by `call_id`(tools)/`itemId`(messages), never persisting live items.
  Memory `omnigent-two-id-space-reconciliation` — a working reference for the P3-3b fix.

## Task order (build catch-up BEFORE deleting reader replay — else broken intermediate)

1. **D15** `created_at` fold + P2 guard **+ delete `last_seen_seq`** (small, independent).
2. Pure `transient_work_outstanding()` + actor `is_quiesced()` (no thread).
3. **[GROK]** Actor item-lifecycle (D20+D23): delete item deltas, `is_terminal`, `frontier`,
   `next_ordinal`, commit-terminal-prefix, prune, `TranscriptAdvanced` watermark,
   `Rebased` scalars-only, apply-bridge subtractive.
4. **[GROK]** Actor forward catch-up (D19): `SessionApi::fetch_items`, mode-switched loop,
   buffer-then-drain, on spawn + `Reconnected`.
5. **[GROK]** Reader → transport-only (D19): delete `items()`/`items_to_replay`, `Reopen` 3→2.
6. `SessionCommand::Sleep` (in-loop quiesce recheck) + wake respawn.
7. `FleetScheduler` skeletal seam + deterministic round-trip test + gated D17 live-verify.
8. Docs (STATUS/handoff/progress) + **push**.

## Gotchas / non-obvious

- **Transient double-fetch (accepted):** between Task 4 and Task 5 the reader still replays
  `/items` AND the actor catches up — idempotent by id, Task 5 removes the reader half.
- **Blast radius:** Tasks 3 touches merged P3-1 (`items: Vec<Arc<Item>>`, apply-bridge copy-
  assign). Deliberate deletion, low-risk (no renderer consumes it yet). Keep the P1 pure-
  reducer contract intact — the actor prunes, the reducer still mutates a small `state.items`.
- **`native ⇏ pending_id`** carries from P3-2 — don't regress the send-reconcile keying.
- **`/items` persisted rows have no `seq`** — frontier/tail delimited by `item_id`, not sequence.
- **Catch-up = actor-thread mode-switched** — do NOT build a worker thread + third channel in 3a.
- **D17 live-verify (Task 7 Step 5)** is the only live-server dep; batch into one gated run
  (`installing-omnigent-from-source`, pinned 0.5.1 — server already reinstalled to
  `0.5.1 (08285468)` this thread; just restart it + attach a host daemon). Informational,
  never in `xtask gate`.

## Deferred to P3-3b (its own grilling+plan)

Scaffold-id reconciliation (call_id/content-stamp dedup — the omnigent web-UI mechanism);
held-bubble resume; `SendLost` re-derivation; command-path `Auth 403`/`NotFound` §9 escalation;
parked-feeder drain / outcome-channel wedge; never-seen-huge first attach + negative-ordinal
scroll-back (D22); the disk `RowSource` viewport/UI plan (windowed read, scroll-back paging,
id-upsert). Coupled to composer send-recovery + input-history (`[[composer-send-recovery-and-history]]`).
