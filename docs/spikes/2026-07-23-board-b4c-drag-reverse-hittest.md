# Spike: B-4c drag-drop reverse hit-test

**Date:** 2026-07-23 · **Branch:** `board-b4c-drag-spike` · **Code:** `spikes/board-drag/`
**Question:** Can drag-to-reorder / drag-in-out-of-groups work against the absolute-masonry
board, and how does a drop pixel map back to a `(parent, ordinal)` the store can commit?

**Verdict: GO.** No blocker. The one genuinely uncertain piece — inverting a forward-only
masonry — is tractable via a scan, proven by `spikes/board-drag` (9/9 tests). The remaining
work is *design* (drop-target convention + feedback), not feasibility. B-4c should proceed to
a brainstorm → plan → cross-family grill.

---

## The three unknowns, resolved

### 1. gpui mechanics — LOW RISK (confirmed at source, not run)

gpui `0.2.2` ships the full drag surface on `div`:
- `on_drag(value: T, constructor)` — starts a drag; `constructor` builds the drag-preview
  view (the "ghost" that follows the cursor). Payload `T` is arbitrary (our `BoardItemId`).
- `on_drag_move(Fn(&DragMoveEvent<T>, …))` — fires per mouse-move during a drag;
  `DragMoveEvent` carries `event: MouseMoveEvent` (global cursor) and `bounds` (the bound
  element's painted rect). This is where edge auto-scroll would live.
- `on_drop(Fn(&T, …))` / `can_drop(predicate)` — drop dispatch + gating.

Drop dispatch is **hitbox-based**: gpui stores the active drag on `App` and, on mouse-up,
dispatches to whatever painted bounds the cursor is over (`window.rs` active-drag path). It
does not care *how* the element was positioned. **So absolute-masonry positioning is
orthogonal to whether drag works** — the thing I worried about is a non-issue.

Caveat honored: the *real* proof of gpui drag is a real-window run (repo rule: "the RUN is
the only proof"; [[false-green-probe-drives-production-path]]). That belongs in the B-4c
build's verify step, **not** in this spike — a headless test would have to hand-set
`active_drag` and paint a fake hitbox, i.e. test the harness, not the product. Source-level
confirmation of the API + dispatch model is sufficient to call mechanics de-risked.

### 2. Write-path — LOW RISK, well-precedented

The domain primitive already exists: `BoardLayout::move_item(item_id, new_board_id,
new_parent, new_ordinal)` (`crates/lens-core/src/domain/board.rs:359`). One call covers all
three B-4c motions — reorder (same board+parent, new ordinal), in/out of a group (change
`new_parent`), and cross-board (B-5, change `new_board_id`). It rejects cycles and reassigns
sibling ordinals on both the old and new parent.

The only wiring B-4c adds is a new `replica::Op::MoveItem { .. }` mirroring the existing
`SetCollapsed` op end-to-end: enum variant → commit-gate arm (`!is_writable` refuse) →
`apply_op` dispatch calling a `store.move_item(...)` → `read_committed`. `SetCollapsed`
(`crates/lens-ui/src/board/replica.rs:23,547`) is the template. The store side needs a
`move_item` persist method (the collapse path added `set_collapsed` the same way).

### 3. Reverse hit-test — THE spike's payload — TRACTABLE via scan

**Why there's no formula.** `pack::pack` is forward-only: it walks items in ordinal order
and drops each into the shortest column (+GAP). Ordinal order is therefore **not spatially
monotonic** — a loose card at ordinal 2 backfills the short column *beside* a tall 2×2 group
(ordinal 0) and sits physically *above* the group's second row. You cannot invert `py →
ordinal` algebraically.

**The resolver: scan the placed tiles (which the renderer already computes).**
`resolve_drop(board, cols, cursor) -> DropTarget { parent, ordinal }`:

1. **Into a group?** If the cursor is inside an *expanded* group's body (below its header
   band, within its box), the drop targets that group's member list. Members are a clean
   row-major `fc × fr` grid (tight `CARD_H + GAP` stride, matching `absolute_group`), so the
   ordinal there **is** spatially monotonic — locate the cell, bias one past it if the cursor
   is right of the cell's horizontal center. This is the easy sub-case.
2. **Else top-level.** Pick the nearest top-level tile by clamped rect distance, then insert
   before/after by which side of that tile's vertical center the cursor is on. The scan (O(n)
   per drop; drops are rare) stands in for the missing inverse.

Collapsed groups render as a 1×1 rollup with **no member drop zone** — a drop on one resolves
top-level (you can't drop *into* a collapsed group without expanding it first).

**Ordinal convention is a separate, tested step.** `resolve_drop` returns the *spatial*
insert-before ordinal (index in the full sibling list). `move_item` wants the index *after*
the dragged item is removed from its list (`new_order.retain(!=id); insert(idx)`).
`to_move_ordinal(spatial, dragged_sibling_index)` does the −1 shift iff the drag stays within
the same parent and the dragged item sat before the target. Keeping geometry and index-
convention separate is deliberate — tangling them is the off-by-one this spike exists to kill.

Tests (`spikes/board-drag/src/lib.rs`, 10/10): before/after by center, **the non-monotonic
backfill case explicitly**, into-group member slots, header-drop stays top-level, collapsed
group has no drop zone, `to_move_ordinal` shift table, empty board, **partial-last-row empty
trailing cell appends** (the codex-caught case below).

**Cross-family review (codex / gpt-5.6-sol, 2026-07-23):** confirmed member origins,
`tile_size`/`block_*`, `top_level_ordinal`, and `to_move_ordinal` are all correct against
the real render + `move_item`. Found one Medium bug: `member_ordinal` in a *partially-filled
last group row* clamped a phantom trailing cell back to the last real member but tested
before/after against the *phantom* column's center → a cursor past the last member resolved
to *before* it. Fixed: a cursor in an empty trailing cell (`row*fc+col >= members`) now
appends. Regression test added.

---

## Design decisions this surfaced (for the brainstorm — NOT settled here)

1. **Insertion convention on masonry.** The spike implements the simplest defensible one —
   *nearest tile + reading-order side*. Alternatives to weigh:
   - **Insertion-marker** (draw a caret at the target seam): honest feedback, but consecutive
     ordinals aren't spatially adjacent under backfill, so a marker can jump columns — visually
     confusing.
   - **Reflow-preview** (tentatively re-pack with the dragged item at each candidate ordinal,
     pick the one minimizing cursor→slot distance): fully WYSIWYG, more compute per move,
     the block reshuffles live under the cursor.
   My lean: nearest-tile + a **live reflow preview** (re-pack on hover) for feedback, because
   masonry *will* reshuffle on commit and the user should see it before releasing. Confirm in
   the brainstorm.

2. **No natural "end of list" target.** A cursor below all content resolves to "after the
   nearest column's last tile", **not** append-to-end — masonry columns end at different
   heights (the demo's last probe shows a below-content cursor at x=140 landing at ordinal 1,
   *after the group*, not at the end). Needs an explicit rule: likely *append when
   `cursor.y > content_height`*, or a dedicated end-of-board drop zone.

3. **Group body vs header/ring semantics.** Spike rule: body (below header) = *into* group;
   header = reorder the group among its siblings. The ring gutter overhang (`GUTTER`) and the
   exact catch-region for "into group" vs "between groups" need on-device tuning.

4. **Drop in an inter-tile gap.** Nearest-tile always resolves *something*, so gaps never
   "fall through" — but whether a gap between two columns should prefer the left or right
   neighbour is a convention to pin.

5. **B-4d dependency unchanged.** Drag-created grouping ("drop card onto card → new group")
   still needs the non-idempotent-retry commit-phase seam that B-4a deferred (design §8).
   Plain reorder/move (this spike) does **not** — it's a single `move_item`, idempotent-safe
   like `SetCollapsed`. So B-4c (move/reorder) can land before that seam; drag-to-*group* is
   B-4d and stays gated.

---

## Follow-ups / carried

- **Auto-scroll during drag** (cursor near viewport top/bottom edge → nudge `board_scroll`)
  is unmodelled here; `on_drag_move` is the hook. Cheap, but needs a real-window pass.
- **Carried minor** (pre-existing, [[board-b4b-executed]]): focused-rail group reflow —
  `pack.rs` stores an unclamped `fc`, so groups don't reflow into the 1-col rail. Small;
  fold into B-4c since B-4c touches this geometry.
- Delete `spikes/board-drag/` once B-4c's design doc folds this verdict in (spike hygiene).
