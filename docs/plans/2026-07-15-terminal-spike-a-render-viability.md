# Terminal Spike A — Render/Paint Viability (protocol-plan)

> **For the implementer (grok):** This is a **spike protocol**, not a rigid TDD
> task list. The deliverable is a *decision + numbers*, not production code. You
> have latitude to explore the paint mapping; you do **not** have latitude to
> change the fixtures, the measurement methodology, or the decision tree — those
> are fixed so the result is trustworthy. Work in `spikes/terminal-render/`
> (new, throwaway). Commit frequently with `spike(terminal-render): …`.

**Goal:** Decide whether Ghostty's dirty-row tracking buys a partial-repaint win
under GPUI's paint model, or whether the terminal render contract should be built
around full-snapshot repaint — and confirm we can paint a realistic terminal grid
within the p95 ≤ 8.3ms frame budget.

**Architecture:** A standalone GPUI 0.2.2 window whose root view paints a fixed
`cols × rows` cell grid from a `libghostty-vt` `RenderState` snapshot. Byte
fixtures drive the terminal; the paint closure is instrumented with `Instant` to
collect per-frame paint cost. No `crates/lens-terminal`, no actor, no input
handling. Throwaway scaffold; the **cell → (background quad + shaped glyph run)
paint mapping is written to be liftable** into the real render contract later.

**Tech Stack:** Rust, `gpui = "0.2.2"` (crates.io), `libghostty-vt` (vendored,
`vendor/libghostty-rs/libghostty-vt`, consumed by `path`).

## Global Constraints

- Native GPUI paint only — **not** `gpui-component`. A fixed cell grid is neither
  markdown nor a form.
- Perf target: **p95 frame paint ≤ 8.3ms** at a realistic large grid.
- **input → first-paint measured separately** from steady-state frame cost.
- Measured on: **release build** (`--release`), Apple Silicon.
- Spike lives under `spikes/terminal-render/`, excluded from the workspace lint
  gate (spikes are outside the production lint wall — see root `Cargo.toml`
  `exclude`). Do **not** add `[lints] workspace = true`.
- No Ghostty type needs to be hidden here (that's the `lens-terminal` boundary,
  out of scope) — but keep the paint-mapping function signature clean so it lifts.

---

## Verified API reference (build from these — do not invent)

These are the real gpui 0.2.2 and libghostty-vt entry points, source-verified.
If a call doesn't compile as written, read the cited file rather than guessing a
different shape.

### libghostty-vt (feed + render)

`vendor/libghostty-rs/libghostty-vt/src/terminal.rs`, `render.rs`, `screen.rs`:

```rust
use libghostty_vt::{Terminal, TerminalOptions, RenderState};
use libghostty_vt::render::{RowIterator, CellIterator, Dirty};
use libghostty_vt::screen::CellWide;

let mut terminal = Terminal::new(TerminalOptions { cols, rows, max_scrollback })?;
terminal.vt_write(bytes);                 // feed VT bytes

let mut render_state = RenderState::new()?;
let mut rows = RowIterator::new()?;       // create ONCE, reuse every frame
let mut cells = CellIterator::new()?;     // create ONCE, reuse every frame

// per frame:
let snapshot = render_state.update(&terminal)?;   // Snapshot
match snapshot.dirty()? { Dirty::Clean | Dirty::Partial | Dirty::Full => {} }
let colors = snapshot.colors()?;          // .background/.foreground/.palette[256]
let (ncols, nrows) = (snapshot.cols()?, snapshot.rows()?);

let mut row_iter = rows.update(&snapshot)?;
while let Some(row) = row_iter.next() {
    let row_dirty = row.dirty()?;         // per-row dirty flag
    let mut cell_iter = cells.update(&row)?;
    while let Some(cell) = cell_iter.next() {
        let graphemes: Vec<char> = cell.graphemes()?;         // empty => blank cell
        let fg = cell.fg_color()?.unwrap_or(colors.foreground);
        let bg = cell.bg_color()?;                            // Option<RgbColor>
        let style = cell.style()?;                            // bold/underline/…
        let selected = cell.is_selected()?;
        // wide-char handling via raw cell:
        let wide = cell.raw_cell()?.wide()?;   // CellWide::{Narrow,Wide,SpacerTail,SpacerHead}
        // SpacerTail => DO NOT render (it's the 2nd half of a wide char)
    }
    row.set_dirty(false);                 // caller MUST reset per-row dirty
}
snapshot.set_dirty(Dirty::Clean)?;        // caller MUST reset global dirty too
```

**Dirty tracking is two-layer** and the caller resets *both*: global
`Dirty::{Clean,Partial,Full}` via `snapshot.set_dirty`, and per-row via
`row.set_dirty(false)`. Setting one does not reset the other. `RgbColor` has
`.r/.g/.b: u8`.

### gpui 0.2.2 (paint) — all cited to `~/.cargo/registry/src/*/gpui-0.2.2/`

```rust
use gpui::{
    Application, WindowOptions, Window, App, Context, Render, IntoElement,
    Bounds, Point, Size, Pixels, px, point, Hsla, Rgba, rgb, hsla,
    canvas, fill,
};
use gpui::{TextRun, font};

// Bootstrap (examples/paths_bench.rs, examples/painting.rs):
Application::new().run(|cx| {
    cx.open_window(WindowOptions::default(), |window, cx| {
        cx.new(|cx| GridView::new(window, cx))
    }).unwrap();
    cx.activate(true);
});

// GridView: impl Render
fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
    window.request_animation_frame();     // drive continuous repaint
    canvas(
        |_bounds, _window, _cx| { /* prepaint */ },
        |bounds, _prepaint, window, cx| { /* paint: see mapping below */ },
    ).size_full()
}

// Cell metrics (compute once):
let font_id = window.text_system().resolve_font(&font(".ZedMono"));
let font_size = px(14.0);
let cell_w: Pixels = window.text_system().ch_advance(font_id, font_size)?;  // advance of '0'
let cell_h: Pixels = window.line_height();

// Background quad for a cell:
let rect = Bounds::new(point(px(x0), px(y0)), Size { width: cell_w, height: cell_h });
window.paint_quad(fill(rect, Rgba { r, g, b, a: 1.0 }));   // Rgba from RgbColor u8/255.0

// Text: shape a run/line then paint it:
let run = TextRun { len: text.len(), font: font(".ZedMono"), color: fg_hsla,
                    background_color: None, underline: None, strikethrough: None };
let shaped = window.text_system().shape_line(text.into(), font_size, &[run], None);
shaped.paint(point(px(x0), px(y0)), cell_h, window, cx)?;

// Low-level alternative (exact per-cell glyph placement):
// window.paint_glyph(point(px, baseline_y), font_id, glyph_id, font_size, color)?;
```

Reference examples in the crate: **`examples/input.rs`** (custom `Element` +
`shape_line` + `paint_quad` + `ShapedLine::paint`), **`examples/painting.rs`**
(`canvas` + `paint_quad`). There is **no** `TerminalElement` in gpui 0.2.2 — you
are writing the first cell-grid painter against this crate.

---

## The exploration question (what you're actually deciding)

GPUI is immediate-mode at the element level: the `canvas` paint closure **re-runs
in full every frame**, so "partial repaint" does **not** mean skipping draw calls
for clean rows — you must still emit quads/glyphs for all visible rows every
frame. The only thing dirty tracking can save is **re-shaping**: shaping text
(`shape_line`) is the expensive step, so a per-row cache keyed by row content lets
clean rows reuse a cached `ShapedLine` instead of re-shaping.

So the spike answers: **is shaping the bottleneck, or is primitive emission the
bottleneck?**
- If shaping dominates → the per-row `ShapedLine` cache buys a real partial-repaint
  win → the render contract is built around the dirty bitset.
- If primitive emission (paint_quad/paint per cell) dominates and full repaint is
  already under budget → dirty tracking is useless complexity → the contract is
  full-snapshot repaint.

You will also hit a real sub-question during exploration: `shape_line` does its
own layout, which can drift from the fixed monospace grid for wide chars, emoji
(proportional fallback), and ligatures. If per-row `shape_line` visually
misaligns the grid, fall back to per-cell glyph placement (`paint_glyph` at
computed `x = col * cell_w`). Report which you needed.

---

## Fixtures (fixed — do not change)

Implement each as a function returning VT bytes, in `spikes/terminal-render/src/fixtures.rs`.
Fixtures are the *paint stress patterns*, not feature coverage.

1. **`full_redraw(cols, rows)`** — worst case, all-dirty. Emit `\x1b[2J\x1b[H`
   then fill every cell: for each row, a full line of rotating ASCII glyphs with a
   rotating truecolor SGR foreground (`\x1b[38;2;r;g;bm`) changing every ~8 cells.
   Re-emitted every frame → `Dirty::Full` each frame.

2. **`partial_update(cols, rows, frame_n)`** — typical case. Static full-screen
   painted once; then per frame rewrite only **3 rows** (cursor-address
   `\x1b[{row};1H`, write a fresh timestamped-style line). Expect `Dirty::Partial`
   with ~3 dirty rows.

3. **`wide_and_sgr(cols, rows)`** — shaping stress + correctness. A few rows of
   CJK (`日本語…`) and emoji (`😀🚀…`) interleaved with narrow ASCII, plus rows of
   dense truecolor SGR runs (color change every cell). Exercises `CellWide::Wide`/
   `SpacerTail` and worst-case run fragmentation.

Grid sizes to sweep (cols × rows): **80×24**, **200×50**, and a stress
**400×100**. (200×50 ≈ maximized 14"/16" MBP terminal; 400×100 is deliberately
oversized headroom.)

---

## Measurement methodology (fixed)

- Instrument the **paint closure body only** with `std::time::Instant` — start at
  closure entry (after `RenderState::update`, which you time separately as
  "snapshot cost"), stop at closure end. Record both.
- Run each (fixture × grid-size × strategy) config for **≥ 500 frames**; discard
  the **first 60** (warm-up / shaping-cache cold). Compute **p50 / p95 / p99 /
  max** over the retained frames. Print a table.
- **input → first-paint:** measure separately — from the first `vt_write` of a
  fixture to the completion of the first paint that reflects it. One number per
  fixture; not mixed into steady-state percentiles.
- Strategies to measure for the *decision*:
  - **S1 — per-row shape, no cache:** `shape_line` every row every frame.
  - **S2 — per-row shape, cached:** cache `ShapedLine` per row keyed by a hash of
    the row's (grapheme, fg, bg, style) content; reshape only on cache miss.
  - (Per-cell `paint_glyph` is a fallback only if S1/S2 misalign the grid — if you
    use it, measure it too and say why.)
- The comparison that decides it: **S1 full_redraw p95** vs **budget**, and **S2
  partial_update p95** vs **S1 partial_update p95** (does the cache actually help
  the typical case?).

---

## Decision tree (the deliverable)

Produce this verdict in the findings doc, with the numbers behind it:

- **S1 full_redraw p95 ≤ 8.3ms at 200×50** → dirty tracking is *optional*; render
  contract = full-snapshot repaint (simpler). (Still report whether S2 helps, but
  the contract doesn't need it.)
- **S1 full_redraw p95 > 8.3ms, but S2 partial_update p95 ≤ 8.3ms** → dirty
  tracking is *load-bearing*; contract built around the per-row `ShapedLine` cache
  + the dirty bitset.
- **Even S2 partial_update p95 > 8.3ms** → deeper problem (GPU/atlas path, a
  different element strategy) — stop and escalate with the numbers, don't paper
  over it.

Also report: the 400×100 stress numbers (does it fall off a cliff?), the
input→first-paint numbers, and whether wide/emoji forced per-cell glyph placement.

---

## Task decomposition (exploration increments, each ends at a commit)

### Task 1 — Scaffold + blank window
**Files:** Create `spikes/terminal-render/Cargo.toml`, `spikes/terminal-render/src/main.rs`.
Add `spikes/terminal-render` to root `Cargo.toml` `exclude`.
- [ ] `Cargo.toml`: `gpui = "0.2.2"`, `libghostty-vt = { path = "../../vendor/libghostty-rs/libghostty-vt" }`, `publish = false`, no workspace lints.
- [ ] `main.rs`: bootstrap a GPUI window with a `GridView` root that paints a
      solid background via `canvas`/`paint_quad`. Confirm it opens with `cargo run -p terminal-render --release`.
- [ ] Commit.

### Task 2 — Cell-grid paint mapping (S1: per-row shape, no cache)
**Files:** Create `spikes/terminal-render/src/paint.rs` (the liftable mapping),
`src/fixtures.rs`.
- [ ] `fixtures.rs`: implement `full_redraw`, `partial_update`, `wide_and_sgr` (bytes).
- [ ] `paint.rs`: a function `paint_grid(snapshot, cell_w, cell_h, window, cx)`
      that iterates rows/cells (verified API above), emits a background quad per
      non-default-bg cell and a shaped line per row (S1), honoring `CellWide`
      (skip `SpacerTail`, advance 2 cells for `Wide`). This signature is the
      liftable artifact — keep it free of harness/timing concerns.
- [ ] Wire `full_redraw` at 80×24, confirm it visually renders (colors, glyphs).
- [ ] Verify wide/emoji alignment; if broken, add the `paint_glyph` per-cell path
      and note it.
- [ ] Commit.

### Task 3 — Instrumentation + measurement harness
**Files:** Create `spikes/terminal-render/src/measure.rs`.
- [ ] Time `RenderState::update` (snapshot cost) and the paint closure separately
      with `Instant`; accumulate per-frame samples; after N frames print a
      p50/p95/p99/max table per config.
- [ ] Add input→first-paint measurement.
- [ ] Run the full sweep for **S1** (all 3 fixtures × 3 grid sizes). Record numbers.
- [ ] Commit (include the recorded numbers in the commit body or a scratch file).

### Task 4 — S2 per-row ShapedLine cache + comparison
**Files:** Modify `paint.rs` (add the cache), `measure.rs`.
- [ ] Add a per-row `ShapedLine` cache keyed by row content hash; reshape on miss,
      reuse on hit. Reset/evict on grid resize.
- [ ] Re-run the sweep for **S2**. Record numbers side-by-side with S1.
- [ ] Commit.

### Task 5 — Findings doc + decision
**Files:** Create `docs/spikes/2026-07-15-terminal-render-viability.md`.
- [ ] Write the verdict per the decision tree, with the S1/S2 tables, the 400×100
      stress result, input→first-paint, and the wide/emoji alignment finding.
- [ ] State explicitly which render-contract shape the finding implies
      (full-snapshot vs dirty-bitset-cached).
- [ ] Commit.

---

## What is out of scope (do not build)

- Cursor rendering, selection overlay, scrollback scrolling (render-contract
  features; they don't move the lock decision).
- The `crates/lens-terminal` boundary, the off-thread actor, WS input.
- Any real byte feed — synthetic fixtures only. (The real captured corpus from
  Spike B upgrades these later; not needed for the decision.)

## Handoff back

When done, hand me (Claude/Opus) the findings doc + the `paint.rs` mapping. I
review, and a free codex pass reviews the liftable `paint.rs` specifically (higher
bar — it's the kept artifact). The scaffold (`main.rs`/`measure.rs`/`fixtures.rs`)
is throwaway.
