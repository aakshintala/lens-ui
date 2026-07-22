# Board container / culling spike — VERDICT: **GO**

Throwaway spike answering B-2 §4's four container/culling unknowns (spec
`docs/specs/2026-07-20-board-packing-and-group-rendering-design.md`). Real-window
gpui program (`Application::new().run()`, harness=false — a TestAppContext fakes
the text system and would false-green paint/layout; memory
`gpui-test-noop-text-system`). Fold this verdict into the spec, then delete the
crate.

Run:
- `cargo run -p board-container` — interactive (scroll, watch HUD + card ticks)
- `cargo run -p board-container -- --probe` — automated GO/NO-GO table, self-quits
- `./measure.sh` — release CPU, cull-ON vs `--all-timers`

## Verdict per unknown

**1. Scroll surface — GO.** Absolute-positioned tiles inside a *stateful*
`div().id(..).overflow_scroll().track_scroll(&handle)`, wrapping ONE in-flow
child of explicit `content_height` (from the packer). The explicit-height child
establishes the scroll extent even though every tile is `absolute` (out of
flow). `ScrollHandle::offset()` reads the current scroll each frame; **offset.y
is ≤ 0 when scrolled down**, so `scroll_top = -offset.y`. `set_offset` drives it
programmatically (used by the probe). Gotchas: the div must be stateful (`.id()`)
or `overflow_scroll`/`track_scroll` (on `StatefulInteractiveElement`) aren't in
scope; `Pixels.0` is private → `f32::from(px)`.

**2. Render culling — GO.** Build only tiles whose y-range intersects
`[scroll_top - margin, scroll_top + viewport_h + margin]`. Culled tiles are
simply absent from the child vec → **gpui never builds them**: proven by the
probe — at the top, `built = 9/56` tiles and the bottom card's `render_count`
stayed **0**. gpui does NOT force-build all children of a div; culling is purely
"don't hand it the child."

**3. Timer gating on scroll — GO (retires the old gate, fixes the freeze at
the root).** The container is the sole visibility authority. Cards init
**hidden** (`visible=false`, no timer). Each frame the container computes the
visible set from packer geometry (pure, no entity reads) and applies
`card.set_visible(bool)` **via `App::defer`** — OFF its own render path, so it
never touches sibling card entities inside `render`'s accessed-entity window
(the `.cached()` dirty-tracking landmine from memory `viewport-reentry-freeze`).
`set_visible(true)` starts the anim timer; `(false)` drops it.
  - Probe proof: card scrolled off-screen → `running=false, visible=false`, ticks
    frozen; scrolled back → **timer respawns, ticks resume**. This is exactly the
    scroll case the old edge-triggered `recover_viewport_gates_on_reentry`
    (focus↔board only) could not handle. Container-driven visibility **retires**
    both the paint-time `last_bounds` gate (`card/view.rs`) and the edge-trigger
    (`board/mod.rs`).
  - Key init subtlety: cards MUST start hidden. If they start visible, the first
    `set_visible(true)` early-returns (no state change) and the timer never
    spawns.

**4. Off-screen CPU — GO, culling ~halves idle CPU.** Release build, 56-tile
fixture, 3 cols, ~9 tiles visible, idle (no scroll):
  - cull-ON (visible band only): **~6.8% CPU**
  - `--all-timers` (no gating, every card's timer runs): **~15.3% CPU**
  - **Delta ≈ 8.5pts (~55%)** is the eliminated off-screen timer cost; it scales
    with off-screen tile count, so the saving grows on larger boards. (There is a
    compositor CPU floor on an idle window; the delta is the real signal.)

## Overdraw margin

`overdraw = CELL_H` (one full cell row, 200px). Covers the one-frame offset lag
(cull uses last frame's painted offset) with zero visible pop-in in the probe's
fast programmatic jumps. A single row is sufficient; no need for more.

## What the B-2 plan lifts from here

- `packer.rs` — pure port of the SSOT `pack()`/`foot()`; ready to move into
  `lens-core` (has unit tests against the §2.2 anchors + hole-backfill).
- The container render shape: explicit-height content child + absolute tiles +
  stateful scroll div + `f32::from(offset.y)` sign handling.
- The gate protocol: cards init hidden; container diffs visible set each frame;
  apply `set_visible` via `cx.defer` (never in render). Delete the old
  `last_bounds` gate + `recover_viewport_gates_on_reentry`.
- Overdraw = 1×CELL_H.
