# Handoff — Transcript T-0 (authoritative turn identity) executed

**Written:** 2026-07-21 · **Branch:** `lens-transript` · **HEAD:** `4ef6ccd` (**PUSHED to origin, UNMERGED**)
**Slice range:** `c8e0c63..4ef6ccd` (6 code + 2 doc commits)
**Spec:** `docs/specs/2026-07-21-transcript-t0-turn-identity-design.md` (status: EXECUTED + live-rider passed)
**Plan:** `docs/plans/2026-07-21-transcript-t0-turn-identity.md`
**Memory:** [[t0-turn-identity-executed]] · [[t0-response-id-live-sourcing]] · [[transcript-turn-identity-response-id]]

## TL;DR

T-0 makes the server **`response_id`** the single authoritative turn signal end-to-end. Code-complete,
cross-family reviewed per task (codex gpt-5.6 + Opus synthesis), **gate green**, **live-rider passed**,
pushed to `origin/lens-transript`. **Deliberately unmerged** — the user is driving the rest of the
transcript workstream (T-1..T-7) on this branch before merging to `main`. T-1 is unblocked and is next.

**Verify green before any follow-up:**
```bash
cargo run -p xtask -- gate     # fmt + workspace clippy -D warnings + tests + drift. NO `cargo xtask` alias exists.
cargo test -p lens-core --test t0_live_rider    # the live rider (2 tests)
```

## What shipped

- **lens-client** (`stream/event.rs`) — `response_id: Option<String>` retained on `Item` variants
  (Message/FunctionCall/FunctionCallOutput/Error/ResourceEvent) + `Item::response_id()` accessor;
  `ResponseEvent::InProgress { response_id }` (was a unit variant). Byte-tested vs the 2026-07-21 captures.
- **lens-core domain** — `BlockContext.response_id: Option<ResponseId>` **replaces** the write-only
  `turn: u32`. ⚠️ The SEPARATE `StreamScratch.turn`/`state.stream.turn` live-turn **counter** is untouched
  (distinct field, feeds the card Ready policy). `SessionState.active_response: Option<ResponseId>` (RAM-only,
  never persisted). `StreamUpdate::ActiveResponseChanged(Option<ResponseId>)`. `ResponseId::from_wire(Option<&str>)`
  in `domain/ids.rs` is the single empty→None normalizer (used by runloop catch-up, reduce/mod live, folds).
- **lens-core reduce** — catch-up `wire_to_domain_item` maps the wire id; `output_item.done` stamps each item's
  **own** wire id; synthesized items (`finalize_message/reasoning`, `push_compaction/agent_changed`) fall back to
  `active_response` — wire items with an absent id stamp `None`, never the scalar (design finding #4).
  `response.in_progress` sets `active_response` + emits the delta; **every** terminal `response.*` clears it —
  `Completed` in `reduce/mod.rs` (where `stream.turn` bumps), `Failed|Incomplete|Cancelled` in `reduce/folds.rs`.
- **lens-core persist** — additive: nullable `response_id TEXT`, **SCHEMA_VERSION stays 3**, legacy `turn` col
  retained + written `0` (degrade-only), promoted in reconcile. `row_to_item` reads it.
- **lens-store** `apply` **mirrors** `active_response` into the foreground replica (design §4.3 "deposits into
  the foreground replica" — this is CORRECT, not a no-op). **lens-ui** `SessionCard::fold_detailed` gets a no-op
  ignore arm (summary card doesn't track liveness; transcript consumption = T-2).

## Review + live verification

- **Per-task cross-family review (codex gpt-5.6):** Task 1 — 1 Important (test-coverage gap) fixed. Task 2 — clean
  (SQL binding/positional-read alignment, additive migration, reconcile params all verified). Task 3 — 1 real
  Important (wire-vs-synthesized stamping conflation) **fixed** (`f410499`); 1 **false positive** (the lens-store
  mirror — adjudicated against design §4.3, left as-is); DRY consolidated to `from_wire`.
- **Live rider — passed:** `crates/lens-core/tests/t0_live_rider.rs` replays the real 0.5.1 SSE captures
  (`decode_all`→`reduce`) asserting per-item stamping, `active_response` set/clear, interrupt→retry **distinct**
  ids, turn-A's interrupted id never landing on an item, and delta `Some(A)→None→Some(B)`. Cross-checks expected
  ids against decoded wire (not vacuous). Plus a fresh `/items` drift-drive (`docs/spikes/captures/2026-07-21-t0-verify/`)
  re-confirming `response_id` present / `created_at` null today.

## Descoped by evidence (do NOT re-litigate as gaps)

- **`created_at` / durations → T-6.** Null on `/items`, absent from the live stream, present **only** on
  snapshot-embedded items as epoch **seconds** (domain wants millis). T-6's duration chip owns the snapshot pass.
- **`stream.turn` non-completed Ready-counter bug → separate Board handoff**
  (`docs/handoffs/2026-07-21-turn-counter-non-completed-terminal-bug.md`). Orthogonal to T-0.

## Next up — T-1 (ViewBlock projection pipeline, pure)

Spec `docs/specs/2026-07-21-transcript-t1-viewblock-projection-design.md` (written + cross-family reviewed).
**Needs a plan** (writing-plans). T-1 groups/gates entirely on `response_id` — which now exists as a real signal:
per-item `BlockContext.response_id` for grouping, session `active_response` for the liveness gate. Transcript
replica *consumption* of `ActiveResponseChanged` is **T-2** (T-0's criterion was only "the actor feed exposes
the delta").

## Gotchas / carry-forward

- **Merge coordination (design §9):** `terminal-ws` concurrently rewrites `runloop.rs`/`reduce/mod.rs` → textual
  merge surface with T-0. Whichever merges to `main` second reconciles. Logically independent.
- **Gate invocation:** `cargo run -p xtask -- gate` — there is **no** `cargo xtask` alias in this repo.
- **codex reviewer hang:** run `codex exec ... < /dev/null` in background shells or it blocks on stdin
  (cost one stuck review this session; see [[codex-as-reviewer]]).
- **Left running:** an omnigent server on `127.0.0.1:6767` from the rider drive — `omnigent stop` to clean.
