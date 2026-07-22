# Handoff — Board B-2+B-3 design locked; container/culling spike next

**Written:** 2026-07-20 · **Branch:** `main` · **Prereq:** B-1 shipped (`8100cc8`) ·
**Spec:** `docs/specs/2026-07-20-board-packing-and-group-rendering-design.md` (design locked) ·
**Pixel SSOT:** `docs/design/renders/board-home.html` (rewritten this session) ·
**Memory:** [[board-b2-b3-design]]

## TL;DR

The **B-2 (packing/scroll/culling) + B-3 (group rendering)** brainstorm is **complete and the
design is locked** — read the spec. The one thing standing between here and writing the two
implementation plans is a **gpui container/culling spike** (§20's "one real spike"). The user
wants the spike run in a **fresh session** (context hygiene), then the spec finalized with the
spike outcome, then **two plans** (B-2 engine, B-3 group chrome).

## What's locked (see spec for detail)

- **Model B (groups-as-tiles) + grid-snap packing.** Footprints `n≤3 → n×1`, `n≥4 → ⌈√n⌉ cols`.
  First-fit with hole-backfill, ordinal order, residual gaps accepted.
- **Header-lane geometry** — uniform `[header-lane][card-body][gap]` cells; every card fills the
  body-zone; group header in its top cell's lane; members occupy full cells → aligned, **zero
  dead space, no overlap**. Full-size 280×160 members; group ring in the inter-tile gap.
- **✓N-completed badge** (→ Archive), active count dropped. **Collapse render+toggle → B-4**
  (rollup aggregation stays in B-3 for the badge). **Drag/movement → B-4.** **Nesting**
  recursive-by-construction, depth-1 committed.
- **Focused rail** = same logic at 1 column (1×N vertical stacks, compact card variant) — the
  existing `focused-session.html` already reflects this.

## THE SPIKE — do this first in the new session

Container = **custom scrollable surface with absolutely-positioned tiles** (`list()`/`uniform_list`
are 1-D, can't do 2-D masonry). Build a **small real-window gpui program**
(`Application::new().run()`, `harness=false` — [[gpui-test-noop-text-system]]) that resolves:

1. **Scroll surface** — absolute-positioned children in an `overflow_scroll` div with explicit
   content height; **read the scroll offset each frame**.
2. **Render culling** — build only tiles whose `y`-range intersects `[scroll_top, scroll_top+vh]`
   (packer geometry → cheap filter). Confirm gpui doesn't force building all children.
3. **Timer gating on scroll** — container computes the visible set and **starts/stops each card's
   anim timer** from it, **retiring** the paint-time `last_bounds` gate (`card/view.rs:98-119`)
   and `recover_viewport_gates_on_reentry` (`board/mod.rs:115`). This fixes the
   scroll-into-view freeze at the root ([[viewport-reentry-freeze]] — today's gate is
   edge-triggered on focus↔board only).
4. **Measure** off-screen timer CPU via `measure.sh` (RELEASE build — [[terminal-slice-1c-executed]])
   to confirm culling saves CPU + set the overdraw margin.

**Deliverable:** GO/NO-GO on absolute-positioned-masonry + the measurement. Fallback (last
resort): `uniform_list` over *rows* — but rows break 2-D group tiles, so avoid.

Port the packer from `board-home.html` `pack()`/`render()` (it's the reference algorithm).
Spikes live under `spikes/` (excluded from the gate — [[xtask-gate-scope]]).

## After the spike

1. Fold the spike outcome (container decision, overdraw margin, measured cost) into §4/§8 of the spec.
2. Write **two plans** → `docs/plans/` ([[spec-plan-location-convention]] — NOT docs/superpowers):
   - **B-2** — packer (pure, port from SSOT) + `board_tree` read-API on `BoardLayout` +
     custom scroll container + culling + timer-gate-on-scroll (retire the old gate) + measure.
   - **B-3** — group ring/header/tint chrome + aggregation rollups (spend/age/✓N) + `group_of` seam.
3. Delegation per CLAUDE.md: default subagent work → `cursor-delegate` composer-2.5; cross-family
   review every non-trivial change; gpt-5.5 via `codex exec -s read-only`.

## Design scratch (this session, `scratchpad/`, ephemeral)

- `group-layout-options.html` — A/B/C model comparison (→ chose B).
- `grid-vs-free-packing.html` — grid-snap vs free (→ grid-snap; free buys ≈0 density here).
- `board-home-v2.html` — working file that became the promoted SSOT + the (cut) collapsed-tile mock.

## Open decisions for the user

- None blocking. Tunables (`HEADER`/`INSET`/`GAP` px, overdraw margin) are set during impl.
- **Not pushed** — this session's commits (spec + SSOT + handoff + status) are unpushed; push is
  a separate call ([[commit-when-finished]]).
