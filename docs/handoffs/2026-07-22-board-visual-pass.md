# Handoff — Board visual pass (next session)

**Date:** 2026-07-22
**Branch:** `board-b4b` (NOT merged — held for this visual pass)
**Mission:** A full visual/design pass over the board before merging `board-b4b`
and before more board feature work. Two known issues seed the pass; expect to
find more.

## How to run

```
cargo run -p lens-app --features demo -- --demo
```

Demo now defaults to the **dark** palette (`LENS_THEME=light` overrides) and
seeds **two adjacent, distinctly-colored 2×2 groups** — "Demo group A" (blue:
needs-input, ready, working, failed) and "Demo group B" (orange: the quiet
four) — so their tint boxes meet in the inter-tile gap. `LENS_DEMO_N>1` adds
loose replicas at scale.

## `board-b4b` branch state (what's already done)

Code-complete, `xtask`-adjacent gate green (fmt + `clippy -p lens-ui/-p lens-app
--all-targets` + 98 lens-ui lib tests + motion/board suites), grok-4.5
cross-family reviewed clean. **Not** merged; **not** yet fully visually signed off.

- **B-4b collapse toggle** (`46d1db6..93744eb` + fixes): caret-only `⌄`/`▸`,
  commit-gated `SetCollapsed`, §7 collapsed 1×1 rollup tile, unified `✓N iff
  N>0`, deadline-wake for stale collapsed time-waves. Final codex + grok reviews
  clean.
- **Ring-gutter fix** (`62c8951` then `3801ce1`): the expanding attention ring
  (`card/motion.rs::render_expanding_ring`, NeedsInput/Failed) was leaking past
  the group border, and with the first gutter value (12) two adjacent groups'
  boxes overlapped. Final values: `RING_REACH_PX = 6`, `board::GUTTER =
  RING_REACH_PX + 1 = 7`. Two **compile-time** invariants pin it (`const _:()`
  asserts in `board/mod.rs`): `GUTTER >= RING_REACH_PX` (ring contained) and
  `2*GUTTER <= pack::GAP` (adjacent group boxes don't overlap; GAP=16).
  `pack::INSET` (5) is unchanged — it now only survives in member placement
  where it cancels.
- **Demo** (`77e98c4`): two adjacent colored groups + dark default via new
  no-`unsafe` theme entry point `theme::install_at_startup_with_default(mode)`.

**Last on-device open item:** confirm the trimmed ring/gutter reads clean —
blue/orange boxes clear with ~2px gap at the seam, group A's `failed` ring
stays inside its box. (The screenshots that drove the fix were the pre-trim
`GUTTER=12` build.)

## Issue 1 — group bg fill is subtle (cosmetic)

The group box already fills with `.bg(accent.opacity(0.07))` — a 7% wash of the
*same* accent as its `border_color(accent)` outline (both the expanded box at
`board/mod.rs` ~line 577 and the collapsed box ~line 739). On the dark bg 7% is
barely visible. **Decision for the pass:** keep 7% or bump to ~10–12% for more
presence without competing with the member cards. One-line change in both spots
(keep them equal — it's SSOT per-box).

## Issue 2 — groups don't reflow in the focused rail (real bug, pre-existing)

The focused-mode rail packs at `RAIL_W = 286` → `pack::cols_for_width(286) = 1`
column. A 2×2 group does **not** reflow to 1×N; it renders at full 2-wide width
(576px) and spills out of the 286px rail.

**Mechanism** (`crates/lens-core/src/pack.rs`):
- Line 102 clamps `fc` to `cols` **only for occupancy/collision** (`it.fc.min(cols)`).
- Line 111 stores `item: *it` — the **original, unclamped** item. So placement
  reserves a 1-wide slot but `absolute_group` renders `placed.item.fc = 2`.
- Even if the clamped `fc` were stored, `fr` is never re-derived from member
  count — a 4-member group clamped to 1 col needs `fr = 4`, not 2. `Item`
  discards member count after `foot()`, so the packer currently *can't* re-shape.

This predates B-4b (groups since B-2/B-3); independent of the ring-gutter work.

**Two fix directions (design call for the pass):**
1. **True reflow** — re-derive the footprint from member count clamped to cols
   (`fc' = min(fc, cols)`, `fr' = ⌈members / fc'⌉`) and store/render that. A
   4-member group becomes a 1×4 tall stack in the rail. Needs member count
   reaching the packer (thread it through `Item`, or clamp when the board builds
   items — cols is known in `pack_and_render`).
2. **Auto-collapse groups in the narrow rail** — render the 1×1 collapsed tile
   whenever `cols == 1` (or the group is too wide to fit). Cheap now because
   B-4b just built the collapsed-tile rendering; keeps the rail scannable, group
   shows its rollup, expand happens on the board. **Recommended lean.**

## Sequencing

- `terminal-ws` already **landed on `main` + pushed**. `board-b4b` has `main`
  merged in (`eebb8ce`), so its base is current.
- Do the visual pass **on `board-b4b`**, fix what's found (incl. Issues 1–2 if
  in scope, or split Issue 2 into its own board item), then: foreground gate →
  merge `board-b4b` → `main` + push → STATUS/memory update.
- Do **not** merge `board-b4b` until the visual pass signs off.
