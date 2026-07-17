> **⚠ SUPERSEDED / RESOLVED (2026-07-17).** The "per-cell perf block" below was a
> **debug-build measurement artifact** — the gate ran the harness in debug (~5.4×
> slower than release). In release the per-cell path meets the 120fps budget with
> headroom; shaping is ~0.06ms, so the per-glyph shape cache / C-a reopen
> recommended here is **retired**. See
> `docs/plans/2026-07-16-terminal-slice-1c-perf-resolution.md` for the resolution.
> The blocker narrative below is kept for history only.

# Handoff — Terminal Slice 1c: correctness DONE, per-cell perf BLOCKS merge

Resume artifact for the **per-cell paint perf blocker** that stops Slice 1c from
merging. Slice 1c's correctness (T1–T7) is done, green, and committed; the T8
perf gate is **red by design** (firm budgets, user-directed). 1d is not started.

## State (one line)

Slice 1c render layer built inline (Opus, TDD, real-window harness) — **all
correctness green**, but the **uncached per-cell paint path misses the firm perf
budgets 2.2–2.4×**, so 1c **does not merge**. User decision: keep budgets firm,
fail, escalate, **pause + hand off**. Everything committed on `terminal-ws`
(unpushed), commits `874c817`→`ae12b8b`.

## What's done (T1–T8, committed)

- **T1** scaffold + real-window harness (`tests/render_realwindow.rs`,
  `[[test]] harness=false required-features=["test-util"]`). **Menlo resolves to
  the real Menlo family on this hardware.**
- **T2** fail-closed Menlo gate (family, `'0'`/`'i'` monospace advance, CJK +
  post-emoji re-sync, box-drawing). **Deviation (user-approved): dropped the
  emoji's-own-left-edge (col 4) probe** — on real Menlo the emoji left edge
  drifts 2.857px under *per-row* shaping (CJK fallback glyph <2 cells), but the
  renderer never per-row-shapes an emoji (wide rows → per-cell), so the ASCII
  grid / box-drawing / CJK / post-emoji-resync (all exact) are the real
  invariants. Guarded by the T4 debug-assert.
- **T3** paint `Frame` PerRow + backgrounds; drop `RowShapeCache`; count paint
  errors (never `unwrap`).
- **T4** PerCell routing (`row_needs_per_cell` = single SSOT) + **debug-assert
  that PerRow never receives a wide cell** (the invariant behind the T2 fix).
- **T5** full SGR resolver: inverse/faint/bold/italic, single/curly underline on
  the `TextRun`, double/dotted/dashed underline + overline as decoration quads
  coloured with `underline_quad_color` (I10a), invisible width-preserving spaces
  (I10b), blink = steady no-op. 6 pure tests.
- **T6** `TerminalTab` embeds `TabRenderState`, renders via the shared
  `render_element` (I6); `set_frame_for_test`. **Deviation (user-approved):**
  `render_test_api` gated on **`test-util`** not `cfg(test)` (integration tests
  link the crate's *normal* build; the harness runs `--features test-util`);
  `render` is a private module with `pub` items (mirrors `engine`) so
  `paint_frame` stays out of the public API (I12 intent).
- **T7** render Inspect ring (zero-cost disabled), reads the full `RenderStats`;
  `TerminalTab::{render_inspect,set_render_inspect_enabled}`.
- **T8** perf phases + Criterion Frame benches + xtask executes the harness on
  macOS. **Gate red by design (see blocker).**

Clippy clean `-D warnings` in **both** the normal and `test-util` builds; fmt
clean; Criterion benches compile (`--no-run`).

## ⛔ The blocker

End-to-end p95 measured in the real window, **no shape cache** (dropped per plan
mandate C-a):

| Workload | Path | p95 | Budget | Status |
| --- | --- | ---: | ---: | --- |
| ASCII 200×50 | per-row | 6.3ms | 8.3ms | ✓ |
| Realistic dense-wide 200×50 | per-cell | **18.0ms** | 8.3ms | ✗ 2.2× |
| Realistic dense-wide 400×100 | per-cell | **48.4ms** | 20ms interim | ✗ 2.4× |
| Pathological 100%-wide+50%-emoji 200×50 | per-cell | 17.3ms | 30ms guard | (guard) |

**Root cause:** the per-cell path runs one `shape_line` + one `paint` **per cell,
every frame** (~3.5µs/cell). A dense-wide 200×50 screen is ~5000 per-cell shapes
→ 18ms. The 2.77ms spike verdict that justified dropping the cache (plan C-a) was
the **per-ROW ASCII** case — per-cell dense-wide, where the cache matters, was
never its basis. Per-row (the common case) is one `shape_line`/row and is fine.

**The gate red-fails at `PerfWide200x50`.** That is the intended escalation
signal; `cargo run -p xtask -- gate` fails on macOS until this is fixed.

## Recommended fix (needs a decision — reopens C-a)

**Per-glyph shape cache** keyed on `(grapheme, font-variant, fg, underline/strike
flags)`. A dense-wide screen has ~10 distinct glyphs repeated thousands of times
→ ~99.8% hit → `shape_line` calls drop from ~5000 to ~10/frame. This is **not**
the cache C-a removed (that was a *broken whole-row content-hash* cache); a
per-glyph cache keys on the exact style inputs, so no staleness. Expect 400×100
to drop hard; 200×50 to sub-budget **likely** but must be re-measured (paint cost
stays per cell — if paint alone is still >8.3ms, consider batching narrow runs
within a per-cell row, or a glyph atlas). Cross-family review the cache — it's the
load-bearing change.

Alternatives if the cache underdelivers: batched sub-line shaping for narrow runs
inside per-cell rows; glyph atlas; or (last resort, needs sign-off) relax the
dense-wide budgets to interim regression ceilings and defer absolute perf to
Slice 4 (the design already defers absolute 8.3ms@400×100 to Slice 4).

## Exact next actions

1. Get a decision on the fix approach (per-glyph cache recommended). It reopens
   plan mandate **C-a** — do not reopen unilaterally.
2. Implement + re-measure against the **firm** budgets in `render_realwindow`
   (`WARMUP 60`, `MEASURE 120`; keep the window focused or the GPUI RAF throttles
   — measure in the foreground). Iterate until `PerfWide200x50` ≤ 8.3ms and
   `PerfWide400x100` ≤ 20ms.
3. Cross-family review the cache diff (family ≠ Opus author).
4. Green gate (`cargo run -p xtask -- gate`, incl. `render_realwindow`), merge 1c
   to `terminal-ws`, then start **Slice 1d** (plan
   `docs/plans/2026-07-16-terminal-slice-1d-convergence.md`, T1–T7 delegable to
   composer; T8 demo + T9 live rider inline — needs omnigent 0.5.1).

## Watch-outs

- **1b flake:** `engine::handle::tests::stop_publishes_final_frame_before_join`
  fails intermittently under full-suite parallelism (timing; passes in isolation
  and usually in-suite). Pre-existing, not from 1c. Fix or mark before relying on
  a green `cargo test --workspace`.
- The GPUI harness **throttles `request_animation_frame` when unfocused** → perf
  phases crawl. Run focused / foreground when measuring.
- `render_test_api` / `render_bench_api` are feature-gated; `--features test-util`
  is required to build/run the harness. Test-only fixtures (`sgr_frame`,
  `mixed_ascii_wide_frame`, `pathological_wide_emoji_frame`) are
  `cfg(any(test, test-util))` (not `bench`) to stay dead-code-free in the bench build.

## Read first

- `docs/plans/2026-07-16-terminal-slice-1c-lens-terminal-render.md` (the plan;
  note the two committed deviations above supersede its literal C1/I12/Menlo text).
- Memory `[[gpui-test-noop-text-system]]` (why the harness exists),
  `[[terminal-render-ptyattach-spikes-executed]]` (spike verdict — note the
  2.77ms was per-row), and the new memory on the per-cell perf finding.
