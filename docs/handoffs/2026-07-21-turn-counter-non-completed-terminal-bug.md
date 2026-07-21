# Bug: turn-completion counter only bumps on `response.completed`

> **✅ RESOLVED 2026-07-21 — see "As-built resolution" at the bottom.** The fix
> shipped on `main` but with a **different shape** than the "Fix shape" section
> below proposed: two codex reviews overturned "bump Failed unconditionally" and
> "finalize is mandatory." The original analysis is kept intact for the record;
> read the as-built section for what actually landed and why.

**Date:** 2026-07-21 · **Status:** OPEN, unowned — ready for a fresh session to fix.
**Surfaced by:** transcript T-0 design review (turn-identity work) — carved out as a
**separate, independent bug** because it lives in the card/summary Ready-policy path,
not in transcript turn identity.
**Owning surface:** Board / wave card (§3.4 Ready policy consumer).
**Fix site:** `crates/lens-core/src/reduce/mod.rs` (lens-core reducer).
**Where to fix:** on **`main`** — this fix is logically independent of T-0 (branch
`lens-transript` worktree at `~/work/lens-transript`); see "Merge-collision heads-up"
below. This copy lives on main so the fix has a home outside the T-0 branch; the
original is committed on `lens-transript` (`8434f0c`).

---

## Symptom

A turn that ends via **cancel** or **incomplete** (e.g. user-interrupt, max-tokens/
length stop) produces work but the card **never flashes `Wave::Ready`** ("just
finished, glance") and never surfaces the completion. The card just goes quiet.

- **Cancelled / Incomplete:** no Ready glance, and no other surface catches it → the
  completion is invisible to an unfocused watcher. This is the sharp, user-visible bug.
- **Failed:** also misses the Ready bump, but Failed has an *independent* surface
  (`needs_attention` + `Wave::Failed`), so failure is still shown — just not via Ready.
  Lower severity, same root cause.

## Root cause

`reduce()` bumps the completion counter in **only one** `ResponseEvent` arm:

```rust
// crates/lens-core/src/reduce/mod.rs:132
ResponseEvent::Completed => {
    // ... finalize_message + finalize_reasoning ...
    state.stream.turn = state.stream.turn.saturating_add(1);   // <-- the only bump
    u.push(StreamUpdate::ScratchChanged(...));
    u.push(StreamUpdate::StatusChanged(state.status));
    u
}
```

The other terminal `ResponseEvent` variants — `Failed`, `Incomplete`, `Cancelled`
(and `CompactionFailed`) — **have no arm** and fall through the catch-all at
`reduce/mod.rs:172` (`_ => SmallVec::new()`). So a non-`Completed` terminal event:

1. does **not** bump `state.stream.turn`,
2. does **not** finalize open scratch (`finalize_message` / `finalize_reasoning`),
3. does **not** emit `StatusChanged`.

The variants exist and are parsed (`lens-client` `stream/event.rs:977-980`:
`response.failed` / `response.incomplete` / `response.cancelled`); the reducer just
drops them.

## Why that breaks Ready

`state.stream.turn` is a monotonic "just finished" completion counter (memory
`coalescing-feed-monotonic-trigger`). Its consumer chain:

```
state.stream.turn
  → CardSummary.last_completed_turn      (actor/summary.rs:71)
  → card.fold_feed advances it past seen_turn, stamps last_completed_at = now
                                         (lens-ui fleet/poller.rs:34-47 → card.fold_feed)
  → Wave::Ready while now - last_completed_at < READY_DECAY_MS
                                         (lens-ui card/wave.rs:51-58)
```

No bump → `last_completed_turn` never advances past `seen_turn` → no `last_completed_at`
stamp → no Ready edge. The counter is the trigger; a cancelled/incomplete turn never
pulls it.

## Fix shape (owning agent's call on exact semantics)

Add arms for `Failed` / `Incomplete` / `Cancelled` that bump the counter — and, to
match `Completed`, also finalize any open scratch and emit `StatusChanged`. Minimum
viable fix is the counter bump; finalizing scratch + status is the honest full fix
(otherwise a cancelled turn leaves `open_message`/`open_reasoning` dangling until the
next turn — see "Related" below).

**Ready-vs-Failed precedence is already handled** by the wave priority ladder
(`card/wave.rs:40` — Failed/AwaitingReview sit *above* Ready), so bumping the counter
**unconditionally** on any terminal event is safe: a Failed turn still renders
`Wave::Failed`, not Ready. No need to special-case which end-reasons are "allowed" to
flash Ready.

Open question for the owner: should `Incomplete` (length/max-tokens stop) be treated
as a normal completion (Ready) or as attention-worthy? Default recommendation: Ready —
it finished, the user should glance, nothing is wrong.

## Related (flag; verify, may be out of scope)

Because the same fall-through arms skip `finalize_message`/`finalize_reasoning`, a
cancelled/incomplete turn with an open message or reasoning accumulator leaves it
dangling in `StreamScratch` until the next turn overwrites/finalizes it. Status itself
is likely fine — it updates via the separate `SessionEvent::Status` path
(`lens-client` `stream/event.rs:51-54`), not the `ResponseEvent` terminal. Confirm on
device whether a cancelled turn's partial message renders correctly before deciding
whether scratch-finalization is in scope for this fix.

## Verification

- **Unit (lens-core `reduce`):** feed `OutputTextDelta` then `ResponseEvent::Cancelled`
  (and `Incomplete`, `Failed`); assert `state.stream.turn` incremented and (if adopting
  the full fix) `open_message`/`open_reasoning` finalized + a `StatusChanged` emitted.
  Mirror the existing `Completed` test at `reduce/mod.rs:199`.
- **Card (lens-ui):** advance `last_completed_turn` via a cancelled turn; assert
  `last_completed_at` stamps and `wave()` returns `Wave::Ready` within `READY_DECAY_MS`
  (see `card/wave.rs` tests).
- `xtask gate` green (fmt/clippy/test, zero warnings).

## Merge-collision heads-up (coordinate with transcript T-0)

T-0 (branch `lens-transript`) edits the **same** `reduce/mod.rs` `ResponseEvent` match
block (stamping `response_id` onto items) and changes `BlockContext` (`domain/item.rs`).
This fix adds arms to that same match. Whoever lands second reconciles the arms — both
touch roughly `reduce/mod.rs:98-172`. The two changes are logically independent (T-0 =
per-item `response_id` identity; this = the `stream.turn` Ready counter), so no design
conflict, just a textual merge in one function.

---

## As-built resolution (2026-07-21)

Shipped on `main`. Touches `crates/lens-core/src/reduce/mod.rs` (match arms) and
`reduce/folds.rs` (`fold_response_marker` routing). **The mechanism the original
root-cause described was slightly off:** the terminal events were not falling
through the `mod.rs` catch-all — they were intercepted earlier in
`fold_response_marker` (`Failed | Incomplete | Cancelled => smallvec![]`) and
early-returned empty. Same net effect, different site. The fix routes
`Incomplete | Cancelled` (and `Failed`) out of that marker and into real `reduce`
match arms.

**Two codex (gpt-5.6) reviews changed the design** from the "Fix shape" section:

1. **Do NOT bump on `Failed`.** The handoff claimed bumping unconditionally was
   safe because the wave ladder keeps `Wave::Failed` above `Ready`. That's only
   true *after* `status == Failed` is folded — and status arrives via a **separate**
   `SessionEvent::Status` event, **not atomically** with `response.failed`. In the
   window between the two, an unfocused card would flash a **transient green Ready**
   (`wave.rs:29` needs `status==Failed || last_task_error`; `wave.rs:52` fires Ready
   on `Idle + fresh last_completed_at`). So **only `Incomplete | Cancelled` bump.**
   `Failed` keeps its own surface (`Wave::Failed`) and never flashes Ready.

2. **DISCARD open scratch, do NOT finalize it.** The handoff argued finalize was
   mandatory to avoid cross-turn contamination. But finalizing a partial with
   `message_id: None` invents `msg_local_N` and commits it — and omnigent's durable
   `interrupted` `/items` row has its own server id. Message reconciliation keys on
   `item_id` **only** (`live_key_for_store_item`, `runloop.rs:237` — secondary keys
   exist only for `FunctionCall*`), so the two never fold → **permanent duplicate row
   surviving restart** (consistent with memory `omnigent-two-id-space-reconciliation`).
   Discarding (`open_message = None; open_reasoning = None`) still prevents
   contamination, avoids the duplicate, and avoids finding-4 blank items.

**Final semantics:**
- `Incomplete | Cancelled` → discard scratch + `turn += 1` + `ScratchChanged` + `StatusChanged`.
- `Failed` → discard scratch (emit `ScratchChanged` iff something was cleared) + **no bump**.
- `CompactionFailed` → unchanged marker (housekeeping, not a turn — never bumped).

**End-to-end coverage** is by composition of existing + new tests (no redundant
card test added): reducer bumps `stream.turn` (new `reduce::tests`) →
`CardSummary.last_completed_turn = s.stream.turn` (`actor/summary.rs:71`) →
`fold_summary` stamps `last_completed_at` on advance (`card/model.rs:378`) →
`derive_wave` → `Wave::Ready` (`card/wave.rs:99`). Gate green: 247 lens-core +
58 lens-ui tests, clippy clean, fmt clean (workspace clippy red only on the
pre-existing `spikes/board-container` dirt, outside the production `-p` gate).

### Deferred follow-ups

1. **✅ RESOLVED 2026-07-21 by a live run — discard is validated.** Drove a
   streaming `claude-sdk` turn against live omnigent 0.5.1 (`08285468`, server
   `127.0.0.1:6767`), interrupted mid-`output_text` via
   `POST /v1/sessions/{id}/events {"type":"interrupt"}`, then `GET /items`. Result:
   - The partial assistant message **IS persisted** durably to `/items` — one row,
     server id `msg_fa3a1e40…`, `status: "completed"`, ~4.9 KB of the partial essay,
     `interrupted` **unset** (that flag is native-turn-only per `openapi.json:1834`).
     No duplicate.
   - Event ordering on cancel: `output_text.delta`×N (`message_id: null`) →
     `session.interrupted` → **`response.output_item.done`** (flushes the canonical
     `msg_fa3a1e40`) → `response.cancelled` → `session.status`(idle) →
     `session.input.consumed` (`[System: interrupted]` **user** marker).
   - **Reducer trace:** `output_item.done` hits the `OutputItemDone` arm, which (for a
     `message_id:None` preview) commits the canonical message under the **server** id
     and clears `open_message` — *before* `response.cancelled` reaches the Cancelled
     arm. So the Cancelled-arm discard is a **no-op for the message**: the partial is
     preserved (server row), nothing is lost, and no synthetic `msg_local_N` is minted
     → **no finding-2 duplicate.**
   - **Note — the source docstring lies.** `omnigent/runner/app.py:10271`
     (`_append_cancellation_items`) marks "flush partial content on interrupt" as an
     unimplemented `.. todo:: Phase 2`, implying the partial is *not* persisted. The
     live run proves it **is** (via the `output_item.done` flush path, not that
     function). Memory `live-event-recapture-findings` applies: verify live, don't
     trust the source comment. **Discard is the correct defensive choice** for any path
     where `output_item.done` does *not* precede cancel (finalize would mint a synthetic
     id and risk duplicating the server row; discard defers to `/items`).
2. **Native `turn.*` terminal family.** `turn.completed` / `turn.failed` /
   `turn.cancelled` are deferred in `lens-client` (`stream/event.rs` → routed to
   `ServerStreamEvent::Unknown`), so a Codex-native turn that emits only its native
   terminal (no `response.*`) still never bumps the counter — **the same bug on the
   native-runner surface.** Out of scope here (the fix covers the `response.*`
   surface); model the `turn.*` variants when a native runner needs Ready.
