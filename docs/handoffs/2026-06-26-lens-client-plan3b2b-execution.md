# Handoff — Plan 3b-2b execution (§7 no-replay reconnect state machine)

**Date:** 2026-06-26
**Branch:** `feat/lens-client-streaming`
**Commits:** `3d4048b..6d4dde3`
**Plan:** [`2026-06-26-lens-client-plan3b2b-reconnect-state-machine.md`](../superpowers/plans/2026-06-26-lens-client-plan3b2b-reconnect-state-machine.md)
**Protocol:** subagent-driven — composer-2.5 build per task (red→green→commit),
Opus controller per-task review, one consolidated gpt-5.5 cross-family review at
the end. Full task-by-task ledger in `.superpowers/sdd/progress.md` (section
"Plan 3b-2b").

## What shipped

The SSE reader thread (`crates/lens-client/src/stream/reader.rs`) is now the §7
reconnect state machine. On a transport drop or clean EOF it backs off, re-reads
the session snapshot + `/items`, re-opens the live stream, and emits synthetic
lifecycle markers on the existing mpsc channel — the consumer stays purely
event-driven and never sees raw reconnect mechanics.

| Task | Commit | Summary |
|---|---|---|
| 1 | `5294536` | 4 synthetic `ServerStreamEvent` variants + `DisconnectReason`; `PartialEq` on snapshot types |
| (hk) | `b838a66` | xtask fmt — pre-existing drift, unblocks workspace `cargo fmt --check` gate |
| 2 | `dff48a6` | `Normalizer::reset_seen_items` (history-replay dedup-reset seam) |
| 3 | `3e36f05` | `SseFrame::sequence_number()` raw-JSON peek |
| 4 | `9411bca` | `reconnect` module: `Reopen` trait, `HttpReopener`, `BACKOFF_MS`, `items_to_replay`; `ItemList::into_items` |
| 5 | `ae844b1` | reconnect state machine in the reader + 4 reconnect tests + 2 updated §7a tests |
| 6 | `934b066` | `Sessions::stream` wires `HttpReopener` (StubReopener bridge deleted) |
| review fix | `6d4dde3` | Critical + 2 user-decided + Minor fixes (see below) |
| docs | (this commit) | §7 reconciled; STATUS/ARCHIVE/handoff |

**Verification:** 119 lib tests, `cargo clippy -p lens-client --all-targets -D warnings`
clean, `cargo fmt --check` clean, `generated.rs` untouched, no `serde_json::Value`
on consumer surfaces.

## Emit contract (as-built)

On a successful reconnect, in order: `Reconnecting{attempt}` (1-based, per backoff
step) → `Reconnected{gap:None}` → `reset_seen_items` → `SnapshotRestored(snapshot)`
→ replayed `/items` history (each as `OutputItemDone`, sent directly, bypassing the
normalizer) → seq-deduped live tail. On give-up/stop: terminal `Disconnected{reason}`,
then the channel closes. Failed-status snapshot is terminal: `SnapshotRestored →
Disconnected{SessionFailed}` (no `Reconnected`).

`stop_reason`: 401→`Unauthorized`, 403→`Forbidden`, 404→`NotFound`,
failed-status→`SessionFailed`, backoff-exhausted→`RetriesExhausted`; everything
else (network/5xx/parse) is retryable.

## Cross-family review (gpt-5.5) outcome

DON'T-MERGE → fixed → MERGE-ready. 1 Critical, 3 Important, 1 Minor, all valid:
- **Critical:** body opened before `/items` fetch → retryable `/items` error dropped
  the opened no-replay body. Fixed: `snapshot → items → open_stream` (open_stream
  last fallible) + regression test. **This is the load-bearing lesson** — in a
  no-replay protocol, never perform a retryable op after acquiring a resource you'd
  discard on retry.
- **Important (user-decided):** drop `Reconnected` on failed-status; make
  `EventStream::spawn` return `Result` (`ClientError::ThreadSpawn`) — no panic.
- **Minor:** removed unused `_last_seen_seq` param.

## Deferred / follow-ups

- **`gap == Some(0)` contiguity proof** — v1 always emits `gap: None`. `resume_floor`
  is tracked + used to drop the overlap, but never promoted to `Some(0)`.
- **`/items` pagination/backfill** — replay is single-page best-effort; the reducer
  merges by `Item::id()`, so later events fill gaps. Revisit if captures show
  `has_more` truncation in practice.
- **Gated live reconnect smoke test** (Task 6 step 3) — not added; no scripted
  server-kill harness this session. TODO comment left in `Sessions::stream`. Run
  against a warm session + mid-stream drop at the next live-server session, asserting
  `Reconnecting → Reconnected → SnapshotRestored` then resume.
- **`live_stream` NOT re-run** (no server) — the whole branch is unit-verified only.
- **Minor cleanups (final-triage):** reconnect.rs test-module redundant re-imports;
  `MockReopen` redundant `open_stream_always_503` branch; `happy_idle_snapshot()`
  duplicated across the two reader test modules. All clippy-clean, none ship-blocking.

## Where 3c picks up

Plan 3c — **contract-drift CI** (outstanding B6): the passive alarm (startup taxonomy
diff + `xtask drift`) that makes tracking dev0 safe when `0.3.0` eventually tags.
The streaming taxonomy (`ServerStreamEvent`) and the reconnect surface are now
complete and stable to diff against.
