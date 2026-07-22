# Terminal Slice 1c — `lens-terminal` full-snapshot GPUI render Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Lift the Spike A painter onto Lens-owned `Frame` data inside `lens-terminal`, gate system Menlo, map full SGR, and land fail-closed GPUI frame-timing + Inspect for the render path — without wiring transport or the engine wake loop (Slice 1d).

**Architecture:** A new `render` module paints an immutable `Arc<Frame>` into a GPUI window via full-snapshot quads + glyphs. The spike's `collect_rows` is discarded; paint helpers are re-targeted to `FrameRow`/`FrameCell`/`Rgb`. Shared `TabRenderState` owns `latest_frame` + `cell_metrics` + the exact canvas-building code used by both `TerminalTab` and the real-window test host. **All text-system, paint, Menlo-gate, and perf assertions run in one macOS `harness = false` executable** that starts `Application::new().run()` and drives work from a real canvas paint callback (gpui's `#[gpui::test]` / `TestAppContext` installs `NoopTextSystem` and is forbidden for these). Ordinary `#[test]` covers only pure logic (`resolve_cell_paint`, `row_needs_per_cell`, `RenderInspectShared`). Slice 1d only swaps the frame *source* into `TabRenderState`.

**Tech Stack:** Rust 2024 / rustc 1.91; **gpui 0.2.2** (pinned); Criterion 0.5 (`bench` feature, Frame-construction only); landed `Frame` types from Slice 1b. **No `libghostty-vt` import in the render module.** Omnigent pin **0.5.1** unchanged.

## Global Constraints

- gpui **0.2.2** + omnigent **0.5.1** pins unchanged; do not bump either.
- **MANDATORY** No Ghostty type escapes the engine/render boundary — zero `libghostty_vt::*` imports under `crates/lens-terminal/src/render/`.
- **MANDATORY** Never block the gpui foreground; paint is pure foreground primitive emission from an already-built `Frame` (no VT parse, no I/O, no engine join).
- **MANDATORY** UI never panics: missing frame → modeled placeholder; paint `Result` errors surface into `RenderStats` / Inspect, never `unwrap` on the paint path.
- **MANDATORY** Gate = `rustfmt` + `cargo clippy --workspace --all-targets -- -D warnings` + `cargo test --workspace` + **on macOS, EXECUTE** `cargo test -p lens-terminal --test render_realwindow` (the real-window harness — not `--no-run`). Criterion benches remain `--no-run` compile-only.
- **MANDATORY** **Forbidden:** `#[gpui::test]` / `TestAppContext` for any test that touches the text system, `resolve_font`, `shape_line`, `paint_frame`, Menlo gate, alignment probe, or canvas paint. gpui 0.2.2's test platform installs `NoopTextSystem` (`platform/test/platform.rs:105`, `platform.rs:594-688`) which accepts every font as `FontId(1)` and fabricates advances — those tests are false-green.
- **MANDATORY** macOS-only Menlo posture; Menlo gate is fail-closed inside the real-window harness. If it fails on real hardware → stop and reopen `lens-fonts` (do not soft-pass).
- **MANDATORY** Inspect + benchmarks land in this slice: render Inspect ring (pure `#[test]`); Criterion Frame-construction benches (`--no-run` in xtask); **paint p95 fail-closed inside `render_realwindow`**, executed by xtask gate on macOS.
- **MANDATORY** Drop the spike `RowShapeCache` (S2). No re-introduction in 1c.
- **MANDATORY** Perf fail-closed (inside `render_realwindow`, exit nonzero on miss): ASCII 200×50 p95 ≤ **8.3 ms**; dense wide/emoji 200×50 p95 ≤ **8.3 ms**; dense wide/emoji 400×100 p95 ≤ **20.0 ms** interim. Absolute **8.3 ms @ 400×100** explicitly re-scoped to Slice 4.
- **MANDATORY** No menu of alternatives in this plan. One mechanism per seam (below).
- Frequent commits: one commit per task deliverable.
- Ground truth: `docs/specs/2026-07-16-terminal-workstream-design.md`; `docs/spikes/2026-07-15-terminal-render-viability.md`; `spikes/terminal-render/src/{paint,main,measure}.rs`; `crates/lens-terminal/src/engine/frame.rs`.

### Locked mechanisms (review fixes — one each)

| ID | Mechanism |
| --- | --- |
| **C1/C5** | One `[[test]] harness = false` binary: `crates/lens-terminal/tests/render_realwindow.rs`. Single process; sequential workloads; `Application::new().run()`; assertions from **canvas paint callback** (mirror spike `main.rs:174-217`). `cx.quit()` exits the process on macOS (`main.rs:132-138`) — on failure call `std::process::exit(1)` before quit; on full success `std::process::exit(0)` after quit path. xtask `gate` **executes** this test on macOS. |
| **I6** | `render::state::TabRenderState` owns `latest_frame`, `cell_metrics`, inspect handle, `last_stats`, and the **exact** canvas element builder. `TerminalTab` embeds it; the real-window host embeds it. `set_frame_for_test` delegates to state. **No `Client` in 1c tests.** `TerminalTab`+`open()` E2E is Slice 1d. |
| **I10a** | `ResolvedCellPaint.underline_quad_color: Rgb` — every decoration quad (overline, double/dotted/dashed underline) uses it. |
| **I10b** | Per-row shaping: invisible cells emit **width-preserving** spaces/runs (advance kept, no visible glyph). Per-cell path may skip shaping entirely for invisible cells. |
| **I12** | **No `bench_api` re-export of `paint_frame`.** Keep `paint_frame` `pub(crate)`. Harness uses `#[cfg(test)] pub mod render_test_api` in `lib.rs`. Criterion uses `#[cfg(feature = "bench")] pub mod render_bench_api` that re-exports **fixtures only** (Frame builders). |

### Locked 1c↔1d seam

```rust
// crates/lens-terminal/src/render/paint.rs
pub(crate) fn paint_frame(
    frame: &Frame,
    origin: Point<Pixels>,
    metrics: &CellMetrics,
    window: &mut Window,
    cx: &mut App,
) -> RenderStats;

// crates/lens-terminal/src/render/metrics.rs
impl CellMetrics {
    pub fn resolve_menlo(window: &Window) -> CellMetrics;
}
pub(crate) fn menlo_gate_ok(window: &Window, metrics: &CellMetrics) -> MenloGateResult;

// crates/lens-terminal/src/render/state.rs
pub(crate) struct TabRenderState {
    pub latest_frame: Option<Arc<Frame>>,
    pub cell_metrics: Option<CellMetrics>,
    pub inspect: RenderInspectShared,
    stats_slot: Rc<RefCell<Option<RenderStats>>>,
}
impl TabRenderState {
    pub fn new() -> Self;
    pub fn set_frame(&mut self, frame: Arc<Frame>);
    pub fn last_stats(&self) -> Option<RenderStats>;
    /// Builds the focus-tracked div + canvas (or placeholder). Used by
    /// TerminalTab::render AND the real-window host — one implementation.
    pub fn render_element(
        &mut self,
        focus: &FocusHandle,
        placeholder_title: &str,
        lifecycle_dbg: &str,
        window: &mut Window,
        cx: &mut App,
    ) -> Div;
}

// crates/lens-terminal/src/lib.rs
#[cfg(any(test, feature = "test-util"))]
pub fn set_frame_for_test(&mut self, frame: Arc<Frame>, cx: &mut Context<Self>);
// → self.render.set_frame(frame); cx.notify();
```

**1d** writes `self.render.latest_frame` from `EngineHandle::latest_frame()` on wake. It must not rewrite `paint_frame` or `TabRenderState::render_element`.

### Quoted landed types

From `crates/lens-terminal/src/engine/frame.rs`:

```rust
pub struct Rgb { pub r: u8, pub g: u8, pub b: u8 }

pub struct CellStyle {
    pub bold: bool, pub italic: bool, pub faint: bool, pub blink: bool,
    pub inverse: bool, pub invisible: bool, pub strikethrough: bool,
    pub overline: bool, pub underline: UnderlineStyle,
    pub underline_color: Option<Rgb>,
}

pub enum UnderlineStyle { None, Single, Double, Curly, Dotted, Dashed }

pub struct FrameCell {
    pub col: u16, pub grapheme: String, pub fg: Rgb, pub bg: Option<Rgb>,
    pub wide: bool, pub selected: bool, pub style: CellStyle,
}

pub struct FrameRow { pub cells: Vec<FrameCell> }

pub struct Frame {
    pub cols: u16, pub rows: u16,
    pub default_fg: Rgb, pub default_bg: Rgb,
    pub grid: Vec<FrameRow>,
}
```

`EngineHandle::latest_frame(&self) -> Option<Arc<Frame>>` — 1d only.

### Quoted gpui 0.2.2 APIs

```rust
pub struct TextRun {
    pub len: usize,
    pub font: Font,
    pub color: Hsla,
    pub background_color: Option<Hsla>,
    pub underline: Option<UnderlineStyle>,       // gpui's
    pub strikethrough: Option<StrikethroughStyle>,
}
// NO overline field on TextRun.

pub struct UnderlineStyle {  // gpui — only these three fields
    pub thickness: Pixels,
    pub color: Option<Hsla>,
    pub wavy: bool,
}
pub struct StrikethroughStyle { pub thickness: Pixels, pub color: Option<Hsla> }

pub fn font(family: impl Into<SharedString>) -> Font;
impl Font { pub fn bold(mut self) -> Self; pub fn italic(mut self) -> Self; }

pub fn shape_line(...) -> ShapedLine;           // NOT Result
pub fn paint(...) -> Result<()>;                // MUST surface errors
pub fn resolve_font(&self, font: &Font) -> FontId;
pub fn get_font_for_id(&self, id: FontId) -> Option<Font>;
pub fn ch_advance(&self, font_id: FontId, font_size: Pixels) -> Result<Pixels>;
```

**Underline / overline mapping:**
- `crate::UnderlineStyle::Single` → `TextRun.underline` (`wavy: false`)
- `Curly` → `TextRun.underline` (`wavy: true`)
- `Double` / `Dotted` / `Dashed` → decoration quads colored with `underline_quad_color`
- `overline` → 1px quad colored with `underline_quad_color`
- `blink` → steady no-op in 1c

**Cache:** Drop `RowShapeCache` entirely.

---

## File Structure

- `crates/lens-terminal/src/render/mod.rs` — module root.
- `crates/lens-terminal/src/render/metrics.rs` — `CellMetrics`, `resolve_menlo`, `MenloGateResult`, `menlo_gate_ok`, `per_row_alignment_ok`.
- `crates/lens-terminal/src/render/paint.rs` — `paint_frame`, SGR resolver, PerRow/PerCell, `RenderStats`.
- `crates/lens-terminal/src/render/state.rs` — `TabRenderState` (shared canvas builder).
- `crates/lens-terminal/src/render/inspect.rs` — `RenderInspect` + ring.
- `crates/lens-terminal/src/render/fixtures.rs` — synthetic `Frame` builders (`#[cfg(any(test, feature = "bench"))]`).
- `crates/lens-terminal/src/lib.rs` — `mod render;`; embed `TabRenderState`; `set_frame_for_test`; `#[cfg(test)] pub mod render_test_api`; `#[cfg(feature = "bench")] pub mod render_bench_api`.
- `crates/lens-terminal/tests/render_realwindow.rs` — **the** real-window harness (`harness = false`).
- `crates/lens-terminal/Cargo.toml` — `[[test]] name = "render_realwindow" harness = false`; `test-util`; `[[bench]] name = "render"`.
- `crates/lens-terminal/benches/render.rs` — Criterion **Frame construction only** (no paint, no Window).
- `crates/xtask/src/main.rs` — after workspace tests, on macOS: **run** `cargo test -p lens-terminal --test render_realwindow`.

---

### Task 1: Scaffold render + `TabRenderState` + real-window harness skeleton

**Files:**
- Create: `crates/lens-terminal/src/render/{mod,metrics,paint,state,inspect,fixtures}.rs`
- Create: `crates/lens-terminal/tests/render_realwindow.rs`
- Modify: `crates/lens-terminal/src/lib.rs` (`mod render;`, `render_test_api`)
- Modify: `crates/lens-terminal/Cargo.toml`

**Interfaces:**
- Produces: `CellMetrics::resolve_menlo`, stub `paint_frame` → `RenderStats::default()`, `TabRenderState::new` / `set_frame` / `render_element`, harness binary that opens a real window and asserts Menlo family via `get_font_for_id` from inside the **canvas paint** callback.

- [ ] **Step 1: Register the harness in `Cargo.toml`**

```toml
[features]
bench = []
test-util = []

[[test]]
name = "render_realwindow"
path = "tests/render_realwindow.rs"
harness = false
```

- [ ] **Step 2: Expose harness API under `cfg(test)` in `lib.rs`**

```rust
#[cfg(test)]
pub mod render_test_api {
    pub use crate::render::fixtures::*;
    pub use crate::render::metrics::{
        CellMetrics, MenloGateResult, menlo_gate_ok, per_row_alignment_ok,
    };
    pub use crate::render::paint::{
        RenderStats, paint_frame, resolve_cell_paint, row_needs_per_cell,
    };
    pub use crate::render::state::TabRenderState;
}
```

Integration tests compile the crate with `cfg(test)`, so `--test render_realwindow` can `use lens_terminal::render_test_api::*`.

- [ ] **Step 3: Write the failing harness (real window)**

`tests/render_realwindow.rs`:

```rust
//! Real-window render harness. NOT under #[gpui::test] — NoopTextSystem is useless.
//! `cx.quit()` exits the process on macOS gpui; run ALL workloads in ONE
//! Application::run. On failure: std::process::exit(1). On success: exit(0).

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    Application, Context, IntoElement, Render, TitlebarOptions, Window, WindowBounds,
    WindowOptions, canvas, prelude::*, px, size,
};
use lens_terminal::render_test_api::CellMetrics;

fn main() {
    Application::new().run(move |cx| {
        cx.open_window(
            WindowOptions {
                titlebar: Some(TitlebarOptions {
                    title: Some("lens-terminal render_realwindow".into()),
                    ..Default::default()
                }),
                focus: true,
                window_bounds: Some(WindowBounds::Windowed(gpui::Bounds::centered(
                    None,
                    size(px(800.0), px(600.0)),
                    cx,
                ))),
                ..Default::default()
            },
            move |_window, cx| {
                cx.new(|_cx| HarnessView {
                    phase: Phase::ResolveMenlo,
                    metrics: None,
                    state: None, // TabRenderState filled in later tasks
                    samples: Vec::new(),
                    perf_budget_ms: 0.0,
                })
            },
        )
        .expect("open_window");
        cx.activate(true);
    });
}

#[derive(Clone, Copy)]
enum Phase {
    ResolveMenlo,
    // Task 2+: MenloGate, PaintAscii, PaintWideRouting, PaintSgr,
    // PerfAscii200x50, PerfWide200x50, PerfWide400x100, Done,
}

struct HarnessView {
    phase: Phase,
    metrics: Option<CellMetrics>,
    state: Option<lens_terminal::render_test_api::TabRenderState>,
    samples: Vec<std::time::Duration>,
    perf_budget_ms: f64,
}

impl Render for HarnessView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        window.request_animation_frame();

        match self.phase {
            Phase::ResolveMenlo => {
                canvas(
                    |_bounds, _window, _cx| {},
                    move |_bounds, _prepaint, window, _cx| {
                        let m = CellMetrics::resolve_menlo(window);
                        let font_id = window.text_system().resolve_font(&m.font);
                        let family = window
                            .text_system()
                            .get_font_for_id(font_id)
                            .map(|f| f.family.to_string())
                            .unwrap_or_default();
                        if family != "Menlo" {
                            eprintln!(
                                "resolve_menlo: expected family Menlo, got {family:?}"
                            );
                            std::process::exit(1);
                        }
                        // Later tasks: advance phase via entity update.
                        // T1 only: success exit after Menlo family check.
                        std::process::exit(0);
                    },
                )
                .size_full()
            }
        }
    }
}
```

Note: subsequent tasks change the harness so phases advance via `cx.update` / stored state and only `exit(0)` after the final phase. T1 alone may `exit(0)` after ResolveMenlo; T2+ replace that exit with phase advancement and a single terminal exit.

- [ ] **Step 4: Run harness — expect FAIL** (module missing)

Run: `cargo test -p lens-terminal --test render_realwindow`

Expected: compile failure (`render` / `CellMetrics` / `render_test_api` not found).

- [ ] **Step 5: Minimal implementation**

`render/mod.rs`:

```rust
pub(crate) mod fixtures;
pub(crate) mod inspect;
pub(crate) mod metrics;
pub(crate) mod paint;
pub(crate) mod state;
```

`render/metrics.rs` — `CellMetrics` with Menlo fonts:

```rust
use gpui::{Font, Pixels, Window, font, px};

#[derive(Clone, Debug)]
pub struct CellMetrics {
    pub cell_w: Pixels,
    pub cell_h: Pixels,
    pub font_size: Pixels,
    pub font: Font,
    pub bold_font: Font,
    pub italic_font: Font,
    pub bold_italic_font: Font,
}

impl CellMetrics {
    pub fn resolve_menlo(window: &Window) -> Self {
        let font_size = px(14.0);
        let base = font("Menlo");
        let bold = base.clone().bold();
        let italic = base.clone().italic();
        let bold_italic = base.clone().bold().italic();
        let font_id = window.text_system().resolve_font(&base);
        let cell_w = window
            .text_system()
            .ch_advance(font_id, font_size)
            .unwrap_or(px(8.4));
        let cell_h = window.line_height();
        Self {
            cell_w,
            cell_h,
            font_size,
            font: base,
            bold_font: bold,
            italic_font: italic,
            bold_italic_font: bold_italic,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenloGateResult {
    pub ok: bool,
    pub reason: &'static str,
}

pub(crate) fn menlo_gate_ok(_window: &Window, _metrics: &CellMetrics) -> MenloGateResult {
    MenloGateResult {
        ok: false,
        reason: "not implemented",
    }
}

pub(crate) fn per_row_alignment_ok(_window: &Window, _metrics: &CellMetrics) -> bool {
    false
}
```

`render/paint.rs` stub:

```rust
use gpui::{App, Pixels, Point, Window};
use crate::Frame;
use super::metrics::CellMetrics;

#[derive(Clone, Debug, Default)]
pub struct RenderStats {
    pub rows_painted: u32,
    pub cells_bg: u32,
    pub shapes: u32,
    pub per_row_rows: u32,
    pub per_cell_rows: u32,
    pub paint_errors: u32,
    pub paint_micros: u64,
}

pub(crate) fn paint_frame(
    _frame: &Frame,
    _origin: Point<Pixels>,
    _metrics: &CellMetrics,
    _window: &mut Window,
    _cx: &mut App,
) -> RenderStats {
    RenderStats::default()
}
```

`render/state.rs` (committed form — `stats_slot` is the only last-stats mechanism):

```rust
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    App, Div, FocusHandle, Window, canvas, div, point, prelude::*,
};

use crate::Frame;
use super::inspect::RenderInspectShared;
use super::metrics::CellMetrics;
use super::paint::{RenderStats, paint_frame};

pub(crate) struct TabRenderState {
    pub latest_frame: Option<Arc<Frame>>,
    pub cell_metrics: Option<CellMetrics>,
    pub inspect: RenderInspectShared,
    stats_slot: Rc<RefCell<Option<RenderStats>>>,
}

impl TabRenderState {
    pub fn new() -> Self {
        Self {
            latest_frame: None,
            cell_metrics: None,
            inspect: RenderInspectShared::new(),
            stats_slot: Rc::new(RefCell::new(None)),
        }
    }

    pub fn set_frame(&mut self, frame: Arc<Frame>) {
        self.latest_frame = Some(frame);
    }

    pub fn last_stats(&self) -> Option<RenderStats> {
        self.stats_slot.borrow().clone()
    }

    pub fn render_element(
        &mut self,
        focus: &FocusHandle,
        placeholder_title: &str,
        lifecycle_dbg: &str,
        window: &mut Window,
        _cx: &mut App,
    ) -> Div {
        if self.cell_metrics.is_none() {
            self.cell_metrics = Some(CellMetrics::resolve_menlo(window));
        }
        let metrics = self.cell_metrics.clone();
        let frame = self.latest_frame.clone();
        let inspect = self.inspect.clone();
        let stats_slot = Rc::clone(&self.stats_slot);
        let placeholder = format!("{placeholder_title} — {lifecycle_dbg}");

        let el = div().track_focus(focus).size_full();
        match frame {
            None => el.child(placeholder),
            Some(frame) => el.child(canvas(
                |_, _, _| {},
                move |bounds, _, window, cx| {
                    let Some(metrics) = metrics.as_ref() else { return };
                    let stats = paint_frame(
                        &frame,
                        point(bounds.origin.x, bounds.origin.y),
                        metrics,
                        window,
                        cx,
                    );
                    inspect.record_paint(&stats);
                    *stats_slot.borrow_mut() = Some(stats);
                },
            )),
        }
    }
}
```

`inspect.rs` — `RenderInspectShared` with `new`, `clone`, no-op `record_paint` until T7.

`fixtures.rs` — empty module under `#[cfg(any(test, feature = "bench"))]`.

- [ ] **Step 6: Run harness — expect PASS**

Run: `cargo test -p lens-terminal --test render_realwindow`

Expected: process exits 0.

- [ ] **Step 7: Commit**

```bash
git add crates/lens-terminal/src/render/ crates/lens-terminal/src/lib.rs \
  crates/lens-terminal/Cargo.toml crates/lens-terminal/tests/render_realwindow.rs
git commit -m "$(cat <<'EOF'
feat(lens-terminal): render scaffold + real-window harness (Slice 1c)

EOF
)"
```

---

### Task 2: Menlo gate + alignment probe in the real-window harness

**Files:**
- Modify: `crates/lens-terminal/src/render/metrics.rs`
- Modify: `crates/lens-terminal/tests/render_realwindow.rs` (add `Phase::MenloGate`; advance from ResolveMenlo instead of exit)

**Interfaces:**
```rust
pub struct MenloGateResult { pub ok: bool, pub reason: &'static str }
pub(crate) fn menlo_gate_ok(window: &Window, metrics: &CellMetrics) -> MenloGateResult;
pub(crate) fn per_row_alignment_ok(window: &Window, metrics: &CellMetrics) -> bool;
```

**Codex fix (b):** assert start of `日`, start of `😀`, **and** start of `c` after the emoji (expected cols 1, 4, 6; tol 0.75px). Also box-drawing `"┌─┐"`.

- [ ] **Step 1: Extend harness — MenloGate phase**

After ResolveMenlo stores `metrics`, next paint runs:

```rust
let gate = menlo_gate_ok(window, metrics);
if !gate.ok {
    eprintln!(
        "Menlo gate FAILED: {} — reopen lens-fonts; do not soft-pass",
        gate.reason
    );
    std::process::exit(1);
}
// advance phase / exit 0 if this is still the last phase
```

- [ ] **Step 2: Run — expect FAIL**

Run: `cargo test -p lens-terminal --test render_realwindow`

Expected: exit 1 (`not implemented`).

- [ ] **Step 3: Implement gate + probes**

```rust
use gpui::{Hsla, Rgba, SharedString, TextRun, Window, px};

fn rgb_to_hsla(c: crate::Rgb) -> Hsla {
    Hsla::from(Rgba {
        r: f32::from(c.r) / 255.0,
        g: f32::from(c.g) / 255.0,
        b: f32::from(c.b) / 255.0,
        a: 1.0,
    })
}

pub(crate) fn per_row_alignment_ok(window: &Window, metrics: &CellMetrics) -> bool {
    let sample = "a日b😀c";
    let run = TextRun {
        len: sample.len(),
        font: metrics.font.clone(),
        color: rgb_to_hsla(crate::Rgb { r: 255, g: 255, b: 255 }),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let shaped = window.text_system().shape_line(
        SharedString::from(sample),
        metrics.font_size,
        &[run],
        None,
    );
    let tol = px(0.75);
    let x_cjk = shaped.x_for_index(sample.find('日').unwrap());
    let x_emoji = shaped.x_for_index(sample.find('😀').unwrap());
    let x_after = shaped.x_for_index(sample.find('c').unwrap());
    (x_cjk - metrics.cell_w).abs() <= tol
        && (x_emoji - metrics.cell_w * 4.0).abs() <= tol
        && (x_after - metrics.cell_w * 6.0).abs() <= tol
}

fn box_drawing_alignment_ok(window: &Window, metrics: &CellMetrics) -> bool {
    let sample = "┌─┐";
    let run = TextRun {
        len: sample.len(),
        font: metrics.font.clone(),
        color: rgb_to_hsla(crate::Rgb { r: 255, g: 255, b: 255 }),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let shaped = window.text_system().shape_line(
        SharedString::from(sample),
        metrics.font_size,
        &[run],
        None,
    );
    let tol = px(0.75);
    let mut ok = true;
    let mut byte = 0usize;
    for (col, ch) in sample.chars().enumerate() {
        let x = shaped.x_for_index(byte);
        ok &= (x - metrics.cell_w * col as f32).abs() <= tol;
        byte += ch.len_utf8();
    }
    ok
}

pub(crate) fn menlo_gate_ok(window: &Window, metrics: &CellMetrics) -> MenloGateResult {
    let font_id = window.text_system().resolve_font(&metrics.font);
    let Some(resolved) = window.text_system().get_font_for_id(font_id) else {
        return MenloGateResult {
            ok: false,
            reason: "get_font_for_id returned None",
        };
    };
    if resolved.family.as_ref() != "Menlo" {
        return MenloGateResult {
            ok: false,
            reason: "resolved font family is not Menlo (fallback?)",
        };
    }
    let adv0 = window
        .text_system()
        .ch_advance(font_id, metrics.font_size)
        .unwrap_or(px(0.0));
    let adv_i = window
        .text_system()
        .advance(font_id, metrics.font_size, 'i')
        .map(|s| s.width)
        .unwrap_or(px(0.0));
    if (adv0 - adv_i).abs() > px(0.5) {
        return MenloGateResult {
            ok: false,
            reason: "Menlo advances for '0' and 'i' diverge",
        };
    }
    if !per_row_alignment_ok(window, metrics) {
        return MenloGateResult {
            ok: false,
            reason: "post-emoji / CJK per-row alignment probe failed",
        };
    }
    if !box_drawing_alignment_ok(window, metrics) {
        return MenloGateResult {
            ok: false,
            reason: "box-drawing alignment probe failed",
        };
    }
    MenloGateResult {
        ok: true,
        reason: "ok",
    }
}
```

- [ ] **Step 4: Run — expect PASS** (or exit 1 → stop, reopen `lens-fonts`)

Run: `cargo test -p lens-terminal --test render_realwindow`

Expected: exit 0.

- [ ] **Step 5: Commit**

```bash
git add crates/lens-terminal/src/render/metrics.rs crates/lens-terminal/tests/render_realwindow.rs
git commit -m "$(cat <<'EOF'
test(lens-terminal): fail-closed Menlo gate in real-window harness (Slice 1c)

EOF
)"
```

---

### Task 3: Lift backgrounds + PerRow onto `Frame` (no cache); harness paint assertion

**Files:**
- Modify: `crates/lens-terminal/src/render/paint.rs`
- Modify: `crates/lens-terminal/src/render/fixtures.rs`
- Modify: `crates/lens-terminal/src/render/state.rs` (stats_slot wired)
- Modify: `crates/lens-terminal/tests/render_realwindow.rs` (`Phase::PaintAscii`)

**Interfaces:** Working `paint_frame` for ASCII; `RenderStats` populated; paint errors counted (codex fix c). Cache dropped (codex fix a).

- [ ] **Step 1: Fixtures + harness PaintAscii phase**

```rust
// fixtures.rs
pub fn ascii_frame(cols: u16, rows: u16, fill: char) -> Frame {
    let mut grid = Vec::with_capacity(rows as usize);
    for _ in 0..rows {
        let mut cells = Vec::with_capacity(cols as usize);
        for col in 0..cols {
            cells.push(FrameCell {
                col,
                grapheme: fill.to_string(),
                fg: Rgb { r: 220, g: 220, b: 220 },
                bg: None,
                wide: false,
                selected: false,
                style: CellStyle::default(),
            });
        }
        grid.push(FrameRow { cells });
    }
    Frame {
        cols,
        rows,
        default_fg: Rgb { r: 220, g: 220, b: 220 },
        default_bg: Rgb { r: 12, g: 12, b: 12 },
        grid,
    }
}
```

Harness: create `TabRenderState`, `set_frame(Arc::new(ascii_frame(40, 10, 'A')))`, return `state.render_element(...)` from `HarnessView::render`, on a subsequent RAF read `state.last_stats()`:

```rust
let stats = state.last_stats().expect("painted");
if stats.rows_painted != 10 || stats.paint_errors != 0 || stats.shapes < 1 {
    eprintln!("PaintAscii stats bad: {stats:?}");
    std::process::exit(1);
}
```

- [ ] **Step 2: Run — expect FAIL** (stub stats)

Run: `cargo test -p lens-terminal --test render_realwindow`

Expected: exit 1 (`rows_painted` 0).

- [ ] **Step 3: Implement paint lift**

```rust
const SELECTION_BG: Rgb = Rgb { r: 40, g: 60, b: 120 };

fn effective_bg(cell: &FrameCell) -> Option<Rgb> {
    if cell.selected {
        Some(SELECTION_BG)
    } else {
        cell.bg
    }
}

// paint_backgrounds(frame, ...) — wide cells get 2× cell_w
// shape_row_line(row, ...) — bold-only fonts in T3; full SGR in T5
// paint_per_row — surface shaped.paint Result into paint_errors
// paint_frame — default_bg quad + backgrounds + per-row for all rows (T4 adds routing)
```

No `RowShapeCache`. No Ghostty imports. Canvas writes `stats_slot`.

- [ ] **Step 4: Run — expect PASS**

Run: `cargo test -p lens-terminal --test render_realwindow`

Expected: exit 0.

- [ ] **Step 5: Commit**

```bash
git add crates/lens-terminal/src/render/paint.rs crates/lens-terminal/src/render/fixtures.rs \
  crates/lens-terminal/src/render/state.rs crates/lens-terminal/tests/render_realwindow.rs
git commit -m "$(cat <<'EOF'
feat(lens-terminal): paint Frame via PerRow; drop shape cache; surface paint errors (Slice 1c)

EOF
)"
```

---

### Task 4: PerCell path + wide-row routing; harness assertion

**Files:**
- Modify: `crates/lens-terminal/src/render/paint.rs`
- Modify: `crates/lens-terminal/src/render/fixtures.rs`
- Modify: `crates/lens-terminal/tests/render_realwindow.rs` (`Phase::PaintWideRouting`)

**Interfaces:**
```rust
pub(crate) fn row_needs_per_cell(row: &FrameRow) -> bool {
    row.cells.iter().any(|c| c.wide)
}
```

- [ ] **Step 1: Pure unit test + harness phase**

```rust
#[test]
fn row_needs_per_cell_detects_wide() {
    let narrow = FrameRow {
        cells: vec![FrameCell {
            col: 0,
            grapheme: "a".into(),
            fg: Rgb { r: 255, g: 255, b: 255 },
            bg: None,
            wide: false,
            selected: false,
            style: CellStyle::default(),
        }],
    };
    let wide = FrameRow {
        cells: vec![FrameCell {
            col: 0,
            grapheme: "日".into(),
            fg: Rgb { r: 255, g: 255, b: 255 },
            bg: None,
            wide: true,
            selected: false,
            style: CellStyle::default(),
        }],
    };
    assert!(!row_needs_per_cell(&narrow));
    assert!(row_needs_per_cell(&wide));
}
```

Harness: `set_frame(mixed_ascii_wide_frame(20, 2))` — row0 all narrow, row1 has one `wide: true`. Assert `last_stats.per_row_rows == 1 && per_cell_rows == 1 && paint_errors == 0`.

- [ ] **Step 2: Run pure test FAIL then harness FAIL**

Run: `cargo test -p lens-terminal row_needs_per_cell_detects_wide`

Expected: FAIL not found (then PASS after Step 3).

Run: `cargo test -p lens-terminal --test render_realwindow`

Expected: exit 1 on routing until Step 3.

- [ ] **Step 3: Implement PerCell + routing in `paint_frame`**

```rust
for (row_i, row) in frame.grid.iter().enumerate() {
    let y = origin.y + metrics.cell_h * (row_i as f32);
    if row_needs_per_cell(row) {
        per_cell_rows += 1;
        let (s, e) = paint_per_cell_row(row, y, origin.x, metrics, window, cx);
        shapes += s;
        paint_errors += e;
    } else {
        per_row_rows += 1;
        // shape_row_line + paint, count errors
    }
}
```

- [ ] **Step 4: Run — expect PASS**

Run: `cargo test -p lens-terminal row_needs_per_cell_detects_wide`

Expected: PASS.

Run: `cargo test -p lens-terminal --test render_realwindow`

Expected: exit 0.

- [ ] **Step 5: Commit**

```bash
git add crates/lens-terminal/src/render/paint.rs crates/lens-terminal/src/render/fixtures.rs \
  crates/lens-terminal/tests/render_realwindow.rs
git commit -m "$(cat <<'EOF'
feat(lens-terminal): PerCell placement for wide rows (Slice 1c)

EOF
)"
```

---

### Task 5: Full-SGR mapping (I10a underline_quad_color + I10b invisible width)

**Files:**
- Modify: `crates/lens-terminal/src/render/paint.rs`
- Modify: `crates/lens-terminal/tests/render_realwindow.rs` (`Phase::PaintSgr`)

**Interfaces:**
```rust
pub(crate) enum UnderlineQuadKind { None, Double, Dotted, Dashed }

pub(crate) struct ResolvedCellPaint {
    pub fg: Rgb,
    pub bg: Option<Rgb>,
    pub font: Font,
    pub underline: Option<gpui::UnderlineStyle>,
    pub strikethrough: Option<gpui::StrikethroughStyle>,
    pub skip_glyph: bool,
    pub overline: bool,
    pub underline_quad_kind: UnderlineQuadKind,
    pub underline_quad_color: Rgb,
}

pub(crate) fn resolve_cell_paint(
    cell: &FrameCell,
    default_bg: Rgb,
    metrics: &CellMetrics,
) -> ResolvedCellPaint;
```

**I10a:** `underline_quad_color = cell.style.underline_color.unwrap_or(fg_after_inverse_and_faint)`. Every decoration quad uses `rgb_to_rgba(resolved.underline_quad_color)`.

**I10b:** In `shape_row_line` / `assemble_row_text_for_test`, invisible cells append `' '` × (1 or 2 if wide) and keep `expected_col` advancing. Per-cell path skips shaping when `skip_glyph`.

**Blink:** steady no-op.

- [ ] **Step 1: Pure `#[test]` cases**

```rust
#[test]
fn resolve_maps_italic_bold_inverse_faint_invisible() { /* ... */ }

#[test]
fn resolve_curly_underline_sets_textrun_wavy() { /* ... */ }

#[test]
fn resolve_double_underline_sets_quad_kind_and_quad_color() {
    let cell = FrameCell {
        col: 0,
        grapheme: "u".into(),
        fg: Rgb { r: 255, g: 255, b: 255 },
        bg: None,
        wide: false,
        selected: false,
        style: CellStyle {
            underline: crate::UnderlineStyle::Double,
            underline_color: Some(Rgb { r: 0, g: 255, b: 0 }),
            ..CellStyle::default()
        },
    };
    let r = resolve_cell_paint(&cell, Rgb { r: 0, g: 0, b: 0 }, &dummy_metrics_fonts());
    assert!(r.underline.is_none());
    assert_eq!(r.underline_quad_kind, UnderlineQuadKind::Double);
    assert_eq!(r.underline_quad_color, Rgb { r: 0, g: 255, b: 0 });
}

#[test]
fn blink_is_steady_noop() { /* skip_glyph == false with blink: true */ }

#[cfg(test)]
pub(crate) fn assemble_row_text_for_test(row: &FrameRow) -> String { /* shared with shape_row_line */ }

#[test]
fn shape_row_invisible_preserves_width() {
    let row = FrameRow {
        cells: vec![
            FrameCell {
                col: 0,
                grapheme: "A".into(),
                fg: Rgb { r: 255, g: 255, b: 255 },
                bg: None,
                wide: false,
                selected: false,
                style: CellStyle::default(),
            },
            FrameCell {
                col: 1,
                grapheme: "B".into(),
                fg: Rgb { r: 255, g: 255, b: 255 },
                bg: None,
                wide: false,
                selected: false,
                style: CellStyle {
                    invisible: true,
                    ..CellStyle::default()
                },
            },
            FrameCell {
                col: 2,
                grapheme: "C".into(),
                fg: Rgb { r: 255, g: 255, b: 255 },
                bg: None,
                wide: false,
                selected: false,
                style: CellStyle::default(),
            },
        ],
    };
    let text = assemble_row_text_for_test(&row);
    assert_eq!(text, "A C"); // space where invisible B was
}
```

`dummy_metrics_fonts()` builds `CellMetrics` via `font("Menlo")` variants with placeholder `cell_w`/`cell_h`/`font_size` — no Window.

Harness `PaintSgr`: frame with italic/underline-double/selected cells; assert `paint_errors == 0` after paint.

- [ ] **Step 2: Run pure tests — expect FAIL**

Run: `cargo test -p lens-terminal resolve_maps_ resolve_double shape_row_invisible blink_is_steady`

Expected: FAIL not found.

- [ ] **Step 3: Implement resolver + wire paint**

```rust
pub(crate) fn resolve_cell_paint(
    cell: &FrameCell,
    default_bg: Rgb,
    metrics: &CellMetrics,
) -> ResolvedCellPaint {
    let _blink_steady = cell.style.blink;

    let mut fg = cell.fg;
    let mut bg = effective_bg(cell);
    if cell.style.inverse {
        let bg_for_swap = bg.unwrap_or(default_bg);
        bg = Some(fg);
        fg = bg_for_swap;
    }
    if cell.style.faint {
        fg = Rgb { r: fg.r / 2, g: fg.g / 2, b: fg.b / 2 };
    }

    let font = match (cell.style.bold, cell.style.italic) {
        (true, true) => metrics.bold_italic_font.clone(),
        (true, false) => metrics.bold_font.clone(),
        (false, true) => metrics.italic_font.clone(),
        (false, false) => metrics.font.clone(),
    };

    let underline_quad_color = cell.style.underline_color.unwrap_or(fg);
    let ul_hsla = Some(rgb_to_hsla(underline_quad_color));

    let (underline, underline_quad_kind) = match cell.style.underline {
        crate::UnderlineStyle::None => (None, UnderlineQuadKind::None),
        crate::UnderlineStyle::Single => (
            Some(gpui::UnderlineStyle {
                thickness: px(1.0),
                color: ul_hsla,
                wavy: false,
            }),
            UnderlineQuadKind::None,
        ),
        crate::UnderlineStyle::Curly => (
            Some(gpui::UnderlineStyle {
                thickness: px(1.0),
                color: ul_hsla,
                wavy: true,
            }),
            UnderlineQuadKind::None,
        ),
        crate::UnderlineStyle::Double => (None, UnderlineQuadKind::Double),
        crate::UnderlineStyle::Dotted => (None, UnderlineQuadKind::Dotted),
        crate::UnderlineStyle::Dashed => (None, UnderlineQuadKind::Dashed),
    };

    let strikethrough = if cell.style.strikethrough {
        Some(gpui::StrikethroughStyle {
            thickness: px(1.0),
            color: Some(rgb_to_hsla(fg)),
        })
    } else {
        None
    };

    ResolvedCellPaint {
        fg,
        bg,
        font,
        underline,
        strikethrough,
        skip_glyph: cell.style.invisible,
        overline: cell.style.overline,
        underline_quad_kind,
        underline_quad_color,
    }
}

fn paint_decoration_quads(
    cell: &FrameCell,
    resolved: &ResolvedCellPaint,
    cell_origin: Point<Pixels>,
    metrics: &CellMetrics,
    window: &mut Window,
) {
    let width = if cell.wide {
        metrics.cell_w * 2.0
    } else {
        metrics.cell_w
    };
    let color = rgb_to_rgba(resolved.underline_quad_color);
    if resolved.overline {
        window.paint_quad(fill(
            Bounds::new(cell_origin, size(width, px(1.0))),
            color,
        ));
    }
    let ul_y = cell_origin.y + metrics.cell_h - px(2.0);
    match resolved.underline_quad_kind {
        UnderlineQuadKind::None => {}
        UnderlineQuadKind::Double => {
            window.paint_quad(fill(
                Bounds::new(point(cell_origin.x, ul_y), size(width, px(1.0))),
                color,
            ));
            window.paint_quad(fill(
                Bounds::new(point(cell_origin.x, ul_y - px(2.0)), size(width, px(1.0))),
                color,
            ));
        }
        UnderlineQuadKind::Dotted => {
            let mut x = cell_origin.x;
            while x < cell_origin.x + width {
                let seg = px(2.0).min(cell_origin.x + width - x);
                window.paint_quad(fill(
                    Bounds::new(point(x, ul_y), size(seg, px(1.0))),
                    color,
                ));
                x = x + px(4.0);
            }
        }
        UnderlineQuadKind::Dashed => {
            let mut x = cell_origin.x;
            while x < cell_origin.x + width {
                let seg = px(4.0).min(cell_origin.x + width - x);
                window.paint_quad(fill(
                    Bounds::new(point(x, ul_y), size(seg, px(1.0))),
                    color,
                ));
                x = x + px(8.0);
            }
        }
    }
}
```

Invisible handling inside row text assembly:

```rust
if resolved.skip_glyph {
    let cells = if cell.wide { 2u16 } else { 1u16 };
    for _ in 0..cells {
        text.push(' ');
    }
    // push/merge a space run (font/color don't matter for spaces)
    expected_col = cell.col.saturating_add(cells);
    continue;
}
```

- [ ] **Step 4: Run — expect PASS**

Run: `cargo test -p lens-terminal resolve_ maps_ shape_row_invisible blink_is_steady`

Expected: PASS.

Run: `cargo test -p lens-terminal --test render_realwindow`

Expected: exit 0.

- [ ] **Step 5: Commit**

```bash
git add crates/lens-terminal/src/render/paint.rs crates/lens-terminal/tests/render_realwindow.rs
git commit -m "$(cat <<'EOF'
feat(lens-terminal): full SGR + underline_quad_color + invisible width preserve (Slice 1c)

EOF
)"
```

---

### Task 6: Wire `TerminalTab` to `TabRenderState` (no Client in 1c tests)

**Files:**
- Modify: `crates/lens-terminal/src/lib.rs`
- Modify: `crates/lens-terminal/tests/render_realwindow.rs` (confirm host uses `TabRenderState::render_element` only)

**Interfaces:**
```rust
pub struct TerminalTab {
    // existing fields…
    render: crate::render::state::TabRenderState,
}

impl Render for TerminalTab {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let title = self.presentation.identity_title.clone();
        let life = format!("{:?}", self.lifecycle);
        self.render
            .render_element(&self.focus_handle, &title, &life, window, cx)
    }
}

#[cfg(any(test, feature = "test-util"))]
pub fn set_frame_for_test(&mut self, frame: Arc<Frame>, cx: &mut Context<Self>) {
    self.render.set_frame(frame);
    cx.notify();
}
```

**Committed test seam:** Real-window harness embeds `TabRenderState` and paints through `render_element` (already from T3). Pure unit test:

```rust
#[test]
fn tab_render_state_starts_empty() {
    let s = crate::render::state::TabRenderState::new();
    assert!(s.latest_frame.is_none());
    assert!(s.last_stats().is_none());
}
```

**Do not** construct `TerminalTab`, `Client`, or any inert-Client helper in 1c. `TerminalTab`+`open()` E2E = Slice 1d.

- [ ] **Step 1: Write pure test**

As above.

- [ ] **Step 2: Run — expect FAIL** until embed compiles

Run: `cargo test -p lens-terminal tab_render_state_starts_empty`

Expected: FAIL / missing type until state is public to tests via `render_test_api`.

- [ ] **Step 3: Embed `render: TabRenderState` in `starting()`; implement `Render` + `set_frame_for_test`**

Initialize: `render: TabRenderState::new()`. Delete any draft inert-Client / duplicated canvas code.

- [ ] **Step 4: Run — expect PASS**

Run: `cargo test -p lens-terminal tab_render_state_starts_empty`

Expected: PASS.

Run: `cargo test -p lens-terminal --test render_realwindow`

Expected: exit 0.

- [ ] **Step 5: Commit**

```bash
git add crates/lens-terminal/src/lib.rs crates/lens-terminal/src/render/state.rs
git commit -m "$(cat <<'EOF'
feat(lens-terminal): TerminalTab embeds TabRenderState canvas path (Slice 1c)

EOF
)"
```

---

### Task 7: Render Inspect (pure tests)

**Files:**
- Modify: `crates/lens-terminal/src/render/inspect.rs`
- Modify: `crates/lens-terminal/src/lib.rs`

**Interfaces:**
```rust
const RING_CAP: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RenderInspectEvent {
    pub kind: RenderInspectEventKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum RenderInspectEventKind {
    FramePainted {
        micros: u64,
        per_row_rows: u32,
        per_cell_rows: u32,
        paint_errors: u32,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RenderInspect {
    pub enabled: bool,
    pub frames_painted: u64,
    pub last_paint_micros: u64,
    pub last_per_row_rows: u32,
    pub last_per_cell_rows: u32,
    pub last_paint_errors: u32,
    pub recent: Vec<RenderInspectEvent>,
}

impl RenderInspectShared {
    pub fn new() -> Self;
    pub fn set_enabled(&self, enabled: bool);
    pub fn record_paint(&self, stats: &RenderStats); // return immediately if !enabled
    pub fn snapshot(&self) -> RenderInspect;
}

// TerminalTab:
pub fn set_render_inspect_enabled(&self, enabled: bool);
pub fn render_inspect(&self) -> RenderInspect;
```

- [ ] **Step 1: Pure failing test**

```rust
#[test]
fn render_inspect_ring_empty_when_disabled_and_records_when_enabled() {
    let shared = RenderInspectShared::new();
    let stats = RenderStats {
        rows_painted: 10,
        cells_bg: 0,
        shapes: 10,
        per_row_rows: 10,
        per_cell_rows: 0,
        paint_errors: 0,
        paint_micros: 123,
    };
    shared.record_paint(&stats);
    assert!(shared.snapshot().recent.is_empty());
    assert_eq!(shared.snapshot().frames_painted, 0);

    shared.set_enabled(true);
    shared.record_paint(&stats);
    let snap = shared.snapshot();
    assert_eq!(snap.frames_painted, 1);
    assert_eq!(snap.last_paint_micros, 123);
    assert!(matches!(
        snap.recent[0].kind,
        RenderInspectEventKind::FramePainted { micros: 123, .. }
    ));

    shared.set_enabled(false);
    assert!(shared.snapshot().recent.is_empty());
}
```

- [ ] **Step 2: Run — expect FAIL**

Run: `cargo test -p lens-terminal render_inspect_ring_empty`

Expected: FAIL not found.

- [ ] **Step 3: Implement** (copy structure from `engine/inspect.rs`)

`TabRenderState` canvas already calls `inspect.record_paint`.

- [ ] **Step 4: Run — expect PASS**

Run: `cargo test -p lens-terminal render_inspect_`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/lens-terminal/src/render/inspect.rs crates/lens-terminal/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(lens-terminal): render Inspect ring (zero-cost when disabled) (Slice 1c)

EOF
)"
```

---

### Task 8: Fail-closed perf in `render_realwindow` + xtask executes harness + Criterion Frame benches

**Files:**
- Modify: `crates/lens-terminal/tests/render_realwindow.rs` (phases `PerfAscii200x50`, `PerfWide200x50`, `PerfWide400x100`)
- Modify: `crates/lens-terminal/src/render/fixtures.rs` (`dense_wide_emoji_frame`)
- Create: `crates/lens-terminal/benches/render.rs`
- Modify: `crates/lens-terminal/Cargo.toml`
- Modify: `crates/lens-terminal/src/lib.rs` (`render_bench_api`)
- Modify: `crates/xtask/src/main.rs`

**Committed budgets (harness `std::process::exit(1)` on miss):**

| Workload | Grid | p95 ceiling |
| --- | ---: | ---: |
| ASCII full redraw | 200×50 | 8.3 ms |
| Dense wide/emoji | 200×50 | 8.3 ms |
| Dense wide/emoji | 400×100 | 20.0 ms interim |

Warmup 60, measure 120. Time the canvas paint body around `paint_frame` (via `TabRenderState`). Absolute 8.3 ms @400×100 → Slice 4.

**Deleted by this task:** any `#[ignore]` smokes, any `assert_eq!(8.3, 8.3)`, any `bench_api` that re-exports `paint_frame`.

- [ ] **Step 1: Add perf phases to harness**

```rust
const BUDGET_MS: f64 = 8.3;
const BUDGET_400_INTERIM_MS: f64 = 20.0;
const WARMUP: usize = 60;
const MEASURE: usize = 120;

fn percentile_ms(samples: &[std::time::Duration], p: f64) -> f64 {
    let mut v: Vec<f64> = samples.iter().map(|d| d.as_secs_f64() * 1000.0).collect();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let idx = ((v.len() as f64 - 1.0) * p).round() as usize;
    v[idx.min(v.len() - 1)]
}

// Each perf phase:
//   state.set_frame(Arc::new(fixture));
//   accumulate Duration for WARMUP+MEASURE paints (Instant around paint_frame
//   inside canvas — store samples on HarnessView via Rc<RefCell<Vec<Duration>>>
//   written from the canvas closure, same pattern as stats_slot);
//   let p95 = percentile_ms(&samples[WARMUP..], 0.95);
//   eprintln!("SMOKE {cols}x{rows} p95_ms={p95:.3} budget={budget}");
//   if p95 > budget { std::process::exit(1); }
```

`dense_wide_emoji_frame(cols, rows)`: every row contains at least one `wide: true` cell (CJK/emoji pattern) so PerCell is used for all rows.

- [ ] **Step 2: Run harness — expect real p95 gate**

Run: `cargo test -p lens-terminal --test render_realwindow`

Expected: exit 0 with printed p95 lines, or exit 1 if over budget (then optimize paint — do not raise the 200×50 budget).

- [ ] **Step 3: Criterion Frame-construction only**

`lib.rs`:

```rust
#[cfg(feature = "bench")]
pub mod render_bench_api {
    pub use crate::render::fixtures::{ascii_frame, dense_wide_emoji_frame};
}
```

`Cargo.toml`:

```toml
[[bench]]
name = "render"
harness = false
required-features = ["bench"]
```

`benches/render.rs`:

```rust
use criterion::{Criterion, black_box, criterion_group, criterion_main};
use lens_terminal::render_bench_api::dense_wide_emoji_frame;

fn bench_frame_build(c: &mut Criterion) {
    c.bench_function("render_dense_wide_emoji_frame_200x50", |b| {
        b.iter(|| black_box(dense_wide_emoji_frame(200, 50)));
    });
}

criterion_group!(benches, bench_frame_build);
criterion_main!(benches);
```

- [ ] **Step 4: xtask gate executes harness on macOS**

In `crates/xtask/src/main.rs` `gate()`, after the existing `cargo test -p … lens-terminal` block and before drift:

```rust
if cfg!(target_os = "macos") {
    run(&[
        "test",
        "-p",
        "lens-terminal",
        "--test",
        "render_realwindow",
    ])?;
} else {
    println!("gate: skip render_realwindow (macOS-only real GPUI text system)");
}
```

Existing `bench -p lens-terminal --features bench --no-run` stays (compiles `engine` + `render` benches).

- [ ] **Step 5: Verify full gate**

Run: `cargo run -p xtask -- gate`

Expected: on macOS, `render_realwindow` executes and exits 0; Criterion `--no-run` still green.

- [ ] **Step 6: Commit**

```bash
git add crates/lens-terminal/tests/render_realwindow.rs crates/lens-terminal/benches/render.rs \
  crates/lens-terminal/Cargo.toml crates/lens-terminal/src/ crates/xtask/src/main.rs
git commit -m "$(cat <<'EOF'
test(lens-terminal): fail-closed real-window perf gate + xtask executes it (Slice 1c)

EOF
)"
```

---

## Self-Review

### Spec coverage

| 1c design requirement | Task |
| --- | --- |
| Lift paint; discard collect_rows; no Ghostty in render | T3 |
| Codex (a) drop broken shape cache | T3 + Global Constraints |
| Codex (b) alignment probe checks cell after emoji | T2 (real window) |
| Codex (c) surface paint errors | T3 |
| PerRow ASCII / PerCell wide | T4 |
| Full SGR + selection from `selected` | T5 |
| underline_color on all decoration quads (I10a) | T5 `underline_quad_color` |
| invisible preserves PerRow advance (I10b) | T5 |
| Menlo live gate fail-closed; fallback lens-fonts | T2 |
| Shared canvas via `TabRenderState`; 1d swaps source only | T1 / T6 |
| No Client / no inert-Client menu (I6) | T6 |
| Inspect ring, zero-cost off | T7 |
| Fail-closed perf in real window; xtask **executes** (C1/C5) | T1 backbone + T8 |
| No `#[gpui::test]` text/paint (C1) | Global Constraints + all tasks |
| No pub re-export of `paint_frame` (I12) | T8 `render_bench_api` fixtures only |
| Completion matrix Render / Inspect / Benchmarks | T1–T8 |

### Placeholder scan

- Removed all “or / prefer / if X doesn’t exist” menus.
- Single harness, single `TabRenderState`, fixtures-only Criterion export, single xtask execution for Menlo/paint/perf.
- No `#[ignore]` smokes; no tautological budget asserts; no `NoopTextSystem` tests for text/paint.

### Type-consistency check

- `paint_frame` remains `pub(crate)`; harness reaches it through `#[cfg(test)] render_test_api` and `TabRenderState`.
- `TabRenderState::{latest_frame, cell_metrics, inspect, set_frame, render_element, last_stats}` consistent across T1/T3/T6.
- `ResolvedCellPaint.underline_quad_color: Rgb` used by all decoration quads.
- `set_frame_for_test` → `self.render.set_frame` only.
- Perf budgets: 8.3 / 8.3 / 20.0 interim; Slice 4 owns absolute 8.3 @400×100.
- `crate::UnderlineStyle` vs `gpui::UnderlineStyle` always qualified.
- xtask `gate` on macOS runs `cargo test -p lens-terminal --test render_realwindow` (execute, not `--no-run`).
