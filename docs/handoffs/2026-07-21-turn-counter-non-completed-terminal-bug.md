# Bug: turn-completion counter only bumps on `response.completed`

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
