# Handoff — Board B-2 (packing/scroll/culling) EXECUTED

**Written:** 2026-07-21 · **Branch:** merged `board-b2` → `main` (**UNPUSHED**) · **Commits:** `db5b7c2..14b474c` (10) ·
**Plan:** `docs/plans/2026-07-21-board-b2-packing-scroll-culling.md` · **Spec:** `docs/specs/2026-07-20-board-packing-and-group-rendering-design.md` ·
**Memory:** [[board-b2-executed]]

## TL;DR

B-2 is **done, reviewed, gate-green, merged to main (unpushed)**. The board now renders the real
adaptive-packing masonry: a pure packer in `lens-core`, an ordered `board_tree` walk, a custom
`overflow_scroll` container with off-screen culling, and a **container-driven visibility gate** that
retired the old paint-time gate and fixed the scroll/re-entry freeze at the root. **B-3 (group chrome)
is next — planning deferred to the next session** (user's call).

## What shipped (per task)

- **Task 1 — `lens-core::pack`** (`db5b7c2`, +`c45c94a` hardening): pure `foot`/`pack`/`cols_for_width`/
  `intersects_band` + geometry consts, ported from the GO spike. Review caught a public-boundary underflow
  (`Item.fc/fr = 0` → `r+fr-1` underflow) → clamped in `pack` (`fc.max(1).min(cols)`, `fr.max(1)`) + boundary test.
- **Task 2 — `BoardLayout::board_tree`** (`3822894`): ordered, group-aware, archived-group-skipping walk +
  `BoardNode` + `leaf_sessions`. Reuses `children`. Recursive (depth-1 committed).
- **Task 3 — ephemeral adapter** (`97916ec`): `build_ephemeral_layout(&FleetStore)` — the **basis-B stub**
  (all loose, deterministic session-id order, no groups). Marked `B-4-REPLACED STUB`; consumers blind to
  source via `&BoardLayout`.
- **Task 4 — scroll container + culling** (`c7bb438`, +`90966d1` fix): one `pack_and_render` for board
  (N-col) and rail (1-col, §5); absolute tiles in `overflow_scroll`; band-cull (overdraw 1×CELL_H). Review
  caught a real geometry bug — group members were children of the absolutely-positioned ring div (origin
  applied twice); fixed to render members as **content-space siblings** of a bare neutral placeholder box.
- **Task 5 — container-driven gate** (`0be5ccc` fmt, `9dd4f37` feat, `d8a4618` test, `14b474c` fmt):
  cards init HIDDEN; `BoardView` computes the visible set each render and pushes `set_visible` via
  `App::defer` (never reads sibling entities in render — the `.cached()` landmine). Retired the paint-time
  `last_bounds` gate + `recover_viewport_gates_on_reentry` + `last_mode`.
- **Task 6 — on-device**: release demo builds + launches clean; live paint-instrumentation confirmed the
  gate (animating cards tick, Slept frozen). Idle CPU ~11–20% with 8 visible animating cards.

## Load-bearing correctness note (the subtle one)

`set_visible(false)` **drops `anim_task` AND resets `anim_interval = None` directly** — NOT via render.
A culled card is absent from the element tree, so its `render` never runs to drop the timer; and the
render spawn-guard is `if desired != self.anim_interval`, so if the interval weren't reset the driver
would never respawn on return (the freeze itself). `set_visible(true)` only notifies — the card is back
in the tree, so render runs and spawns. This corrected a bug in the plan's own `set_visible` code (the
plan relied on render to drop the timer). Opus whole-branch review verified this holistically.

## Reviews

Every task cross-family reviewed (codex = gpt-5.6-sol high, per [[codex-as-reviewer]]). Two Important
findings caught + fixed (packer underflow; group double-origin geometry). Final **Opus whole-branch
review = READY**, no Critical/Important; confirmed the retired-gate swap is correct across card+board,
no dangling symbols, freeze tests are genuine (not vacuous) repros.

## Residuals / deferred (all clean seams)

- **B-3:** group chrome (ring/header/rollups) + `group_of` seam. B-2 renders a bare placeholder box arm to fill.
- **B-4:** deletes `build_ephemeral_layout`, wires the persisted `SqliteBoardStore` → replica + all write
  paths; group render-geometry becomes runtime-reachable → add a **group render-geometry test** then
  (inspection-verified only today). Collapse toggle, drag/move, create-group.
- **On-device CPU (optional):** the at-scale cull delta (cull-ON vs all-timers) was NOT re-measured on the
  real app — the demo's fixed 8 cards don't overflow. Spike measured it (~halves idle CPU, identical
  mechanism); `board_culls_offscreen_tiles` (40 cards) proves culling behaviorally. To close: bump
  `demo_cards` / shrink the demo window + sample.
- **Two B-2 test-strength nits (defer-OK):** archive test could assert `nodes.len() == 1`; in-group ordinal
  sort not independently exercised (same `children()` sort proven at root).
- **Minor (spec §8 tuning):** `pack_and_render` content width includes a trailing `GAP` (over-wide scroll
  extent by 16px; harmless).

## Next session

1. **B-3 planning** (group chrome + rollups + `group_of`) — brainstorm/plan, then subagent-driven execute.
2. **Unrelated, user-surfaced:** a transcript turn-counter bug awaits —
   `~/work/lens-transript/docs/handoffs/2026-07-21-turn-counter-non-completed-terminal-bug.md`.
3. **Push decision:** B-1 (`8100cc8`) + B-2 (`db5b7c2..14b474c`) are both on main **unpushed** — one
   deliberate push when ready.
