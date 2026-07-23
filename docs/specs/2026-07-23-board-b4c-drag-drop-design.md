# Board B-4c — drag-to-reorder / drag in-out of groups — design

**Written:** 2026-07-23 · **Status:** LOCKED — brainstormed + settled ·
**Depends on:** B-4a (`BoardReplica` + serialized commit-gated `run_op` write seam,
`c189d4c`), B-4b (`Op::SetCollapsed` template + the commit-gated-vs-optimistic decision
handed forward, `c75cabf`…`70cc419`), B-3 (group chrome `absolute_group`, `board/rollup.rs`),
B-2 (`lens_core::pack` packer + culling container), B-1 (`BoardLayout::move_item`, schema v3) ·
**Spike input:** `docs/spikes/2026-07-23-board-b4c-drag-reverse-hittest.md` (verdict GO) ·
**Feeds:** B-4d (drag-to-*create*-group — still gated on the §8 non-idempotent-retry seam),
B-5 (cross-board move — same `move_item`, different `new_board_id`).

> **Governing specs.** §8 "Seams & deferred decisions" of
> `docs/specs/2026-07-21-board-b4a-store-replica-write-path-design.md` (the write seam, the
> idempotent-retry contract, the "optimistic-drag-is-B-4c" earmark — **now retired**, see §2),
> `docs/specs/2026-07-22-board-b4b-collapse-toggle-design.md` (the `Op` template + the latency
> handoff), and `docs/specs/2026-07-20-board-packing-and-group-rendering-design.md` (masonry
> geometry). This spec builds on them; it does not re-derive them.

---

## 1. Scope

B-4c is **drag-to-reorder** (move a top-level tile to a new ordinal) plus **drag a card in or
out of a group**, with **edge auto-scroll** during the drag. All three motions are a **single**
domain write — `BoardLayout::move_item` (`crates/lens-core/src/domain/board.rs:359`) already
covers reorder (same board+parent, new ordinal), in/out-of-group (change `new_parent`), and —
for free, used by B-5 — cross-board (change `new_board_id`).

**Non-goals (deferred):**
- **Drag-to-*create*-group** ("drop card onto card → new group") → **B-4d**. It needs the
  non-idempotent-retry commit-phase seam B-4a deferred (design §8); a `move_item` does not.
- **Cross-board move** → B-5 (the primitive lands here, the multi-board UI does not).
- **"Append to end of board"** — **cut entirely.** It was a spike-era pixel→ordinal worry; under
  reflow-preview (§3) it evaporates (see §4.3). No append rule, no bottom drop zone.

---

## 2. Write model — commit-gated (decided, measured)

`Op::MoveItem` is **commit-gated**, mirroring `Op::SetCollapsed` verbatim: the in-memory layout
changes only when the off-thread persist reply lands. **No optimistic-apply, no
rollback-snapshot machinery** — the optimistic variant B-4a earmarked for B-4c is **retired.**

**Why (measured, not speculated).** The commit-gated-vs-optimistic difference is *exactly* the
off-thread `run_op` round-trip (both models pay the same post-change repaint frame). Measured
on-device (real event loop, `--demo` board, warm `SetCollapsed` round-trips — the same
single-row-write + full-reload path a move takes):

| samples (ms) | median | max | min |
|---|---|---|---|
| 1.09, 1.63, 0.69, 1.44, 0.79, 1.08, 0.58 | **1.08** | 1.63 | 0.58 |

~1ms is **1/16–1/10 of a single 16ms frame** — imperceptible. (The ~43ms `Load` round-trip
seen at startup is the *cold, full-board read* — thread-pool spin-up + SQLite open — **not** the
write path; it does not bear on this decision.)

Commit-gated is also **inherently rollback-free**: the dragged item never mutates the committed
layout until the write is confirmed, so a failed write is a *no-op* — nothing to snap back.
Failure handling is "discard the ephemeral preview" (§3), not "revert an applied mutation."

**Scale caveat (recorded, not a B-4c concern).** The post-write `read_committed` re-reads the
*whole* board layout (~1ms at 10 items). This already governs `SetCollapsed`/`PlaceSessions`
today. **The trigger to revisit optimistic is a full-reload that exceeds a frame at large board
sizes — not the interaction feel.** Human-scale manual drag is nowhere near it.

### 2.1 The op

A new variant on `board/replica.rs`'s `Op`, wired end-to-end exactly like `SetCollapsed`:

```rust
Op::MoveItem {
    item_id: BoardItemId,
    new_board_id: BoardId,
    new_parent: Option<BoardItemId>,
    new_ordinal: i32,
}
```

- **Commit-gate** (`pump` re-gate loop): `Some(Op::MoveItem { .. }) if !self.is_writable() =>
  drop + count`, identical to the `SetCollapsed` arm (`replica.rs:250`).
- **`run_op_inner` dispatch:** `store.move_item(item_id, new_board_id, new_parent,
  new_ordinal)?` then `read_committed(store)?` → `OpOutcome::Wrote` (reuse the `SetCollapsed`
  arm shape, `replica.rs:560`).
- **Store side:** a `BoardStore::move_item` persist method mirroring `set_collapsed` (the
  domain `move_item` exists; the store persist wrapper is the new plumbing).
- Idempotent-safe: re-running the same move against the committed layout is a no-op, so it
  rides B-4a's transient-retry (re-enqueue on `BUSY`) **without** any B-4d commit-phase work.

---

## 3. Feedback + drag state machine

This is the design's synthesis (the spike did not have the measured latency in hand). It
reconciles **commit-gated** (§2) with an **instant-feeling** drag by making the visual feedback
an *ephemeral, derived preview* rather than an applied-then-maybe-reverted mutation.

### 3.1 The two representations

1. **The ghost** — gpui `0.2.2`'s `on_drag(value, constructor)` renders a drag-preview view (the
   card content) that follows the cursor. Payload is the `BoardItemId`.
2. **The reflow-preview** — the masonry block re-packs `layout-with-dragged-item-moved-to-T`
   and renders the dragged item's footprint as an **empty placeholder/gap** (a reserved
   `fc × fr` slot), **not** a second card copy. The card *is* the ghost; the preview shows only
   where it will land. This also avoids spawning a card view for the preview slot.

The reflow-preview is a **pure function** of `(committed layout, dragged id, resolved target
T)`. It is never written to the store and never mutates `self.layout`.

### 3.2 States

```
Idle ──on_drag start──▶ Dragging ──on_drop──▶ Committing ──Wrote reply──▶ Idle
                          │                        │
                          │                        └──Failed reply──▶ Idle (+ banner)
                          └──drag cancelled────────────────────────▶ Idle
```

- **Idle.** Normal masonry render from `self.layout`.
- **Dragging.** The ghost follows the cursor. Per `on_drag_move`, resolve the cursor to a
  `DropTarget` (§4) against the **frozen snapshot** and render the reflow-preview for that
  target (gap placeholder). Also runs edge auto-scroll (§5).
- **Committing.** On drop, issue `Op::MoveItem` and **hold the last reflow-preview as the render
  source** while the ~1ms write is in flight. (The board does not flicker back to the pre-drag
  layout.)
- **Wrote reply.** The committed layout equals the held preview (same move, same pack), so the
  swap from preview → committed render is **invisible**. → Idle.
- **Failed reply.** Discard the preview → render the (unchanged) committed layout → Idle, and
  surface the existing replica banner. **No rollback snapshot** — the preview was never a
  committed edit, so failure is just "drop the derived view." This is why commit-gated needs no
  optimistic machinery yet still feels instant.

### 3.3 Why this is safe

The preview gives optimistic *feel* with commit-gated *safety*: the only mutable committed state
is `self.layout`, which changes solely on a `Wrote` reply. A crash, a lost reply, or a persistent
`Err` mid-drag can only ever leave the *unchanged* committed layout on screen — never a diverged
optimistic state that must be reconciled.

---

## 4. Reverse hit-test resolver

The crux the spike de-risked. Because the packer is **forward-only** (ordinal → px via
shortest-column backfill) and ordinal order is **not spatially monotonic** (a later, smaller
tile backfills a short column beside a tall group and sits physically *above* an earlier tile),
**there is no closed-form inverse** — the resolver **scans placed tiles**.

### 4.1 Frozen snapshot (loop-breaker)

On drag-start, freeze snapshot `S` = the current `pack::Placed` **with the dragged item
removed**. For the drag's entire duration, `resolve_drop` maps `cursor → DropTarget` against `S`
— **never against the live reflow-preview**. This decouples target *resolution* from the
displayed reshuffle: moving the cursor within the previewed (shifted) tiles cannot re-feed the
resolver, so there is no reflow feedback loop.

### 4.2 `resolve_drop(S, cursor) -> DropTarget { parent, ordinal }`

1. **Into an expanded group's body?** If the cursor is below a group's header band and within
   its box, target that group's member list. Members are a clean row-major `fc × fr` grid (tight
   `CARD_H + GAP` stride, matching `absolute_group`), so ordinal there **is** monotonic — locate
   the cell; bias one past it if the cursor is right of the cell's horizontal center. A cursor in
   an **empty trailing cell** of a partially-filled last row (`row*fc + col >= members`) →
   **append** to the member list (the codex-caught partial-last-row fix).
2. **Group header** → reorder the *group* among its top-level siblings (parent unchanged).
3. **Collapsed group rollup** → **top-level only** (no member drop zone; you cannot drop *into* a
   collapsed group without expanding it).
4. **Else top-level.** Nearest top-level tile by **clamped rect distance**, then insert
   before/after by which side of that tile's **vertical center** the cursor is on. O(n) scan per
   move (drops are rare).

The resolver is **total**: a cursor in an empty region below/beside all tiles still resolves to
the nearest tile + reading-order side — it never returns "nothing." (This is what makes the cut
of the "append to end" rule sound — §4.3.)

### 4.3 Ordinal convention (separate, tested step)

`resolve_drop` returns the **spatial insert-before** ordinal (index in the full sibling list).
`move_item` wants the index *after* the dragged item is removed from its list (its `move_item`
does `retain(!= id); insert(idx)`). `to_move_ordinal(spatial, dragged_sibling_index)` applies the
**−1 shift iff** the drag stays within the same parent **and** the dragged item sat before the
target. Keeping spatial resolution and index convention separate is deliberate — tangling them is
the off-by-one class the spike exists to kill.

**"Append to end" is cut.** Under reflow-preview the user reasons about *spatial position*, never
ordinals or "the end" — and on a masonry board the max-ordinal tile isn't even necessarily at the
visual bottom (backfill), so "drag to the bottom = end" was never coherent. The resolver being
total (§4.2) is the only requirement; no dedicated append target exists.

---

## 5. Edge auto-scroll

During **Dragging**, `on_drag_move` checks the cursor against the viewport: within an edge band
of the top or bottom, nudge `board_scroll` (`board/mod.rs:113`) toward that edge. Band width and
nudge velocity are **on-device tuning** (the RUN is the only proof); the mechanism is the
`on_drag_move` hook. Bind the drop/move handlers to the scrolled `content` element so
`cursor − bounds.origin` is content-local for free (bounds are painted → already scrolled).

---

## 6. Group drop semantics

- **Expanded group body** (below header band) → **into** the group.
- **Group header** → **reorder the group** among its top-level siblings.
- **Collapsed rollup** → **top-level only**.
- Catch-region tuning at the ring-gutter seam (where two adjacent group tint boxes overlap by the
  ring-gutter overhang, ~8px on the demo board) is **on-device tuning**, not a spec constant.

---

## 7. Folded minor (carried from B-4b)

The focused-rail group-reflow bug ([[board-b4b-executed]]): the packer stores an **unclamped
`fc`**, so a group does not reflow into the single-column focused rail (it keeps its board-width
`fc`). Folded into B-4c because B-4c touches exactly this `pack::Placed`/`fc` geometry. Fix: clamp
a group's effective `fc` to the available column count in the packer; add a focused-rail
group-reflow regression test.

---

## 8. Testing

**Pure resolver (carried from the spike, 10/10 — re-home into `lens_core::pack` or a
`board/drag.rs` unit):** before/after by center, **the non-monotonic backfill case explicitly**,
into-group member slots, header-drop stays top-level, collapsed group has no drop zone,
`to_move_ordinal` shift table, empty board, **partial-last-row empty-trailing-cell appends**.

**Write path (mirror the `SetCollapsed` tests, `replica.rs:748`):** `Op::MoveItem` round-trips
and persists (reorder + in/out-group); `Op::MoveItem` refused when `!is_writable`;
idempotent re-run is a no-op.

**Drag state machine (pure/entity):** Dragging→Committing holds the preview; `Wrote`→invisible
swap (committed == preview); `Failed`→discard preview, committed layout unchanged, banner shown;
cancel→Idle.

**Verify (real-window, "the RUN is the only proof"):** a real drag against the `--demo` board —
reorder, into-group, out-of-group — plus edge auto-scroll tuning. A headless drag test would have
to hand-set `active_drag` + fake a hitbox (test the harness, not the product,
[[false-green-probe-drives-production-path]]); the real-window run is the proof.

---

## 9. Spike hygiene

Delete `spikes/board-drag/` once this design lands (its verdict + geometry are folded in here).
