# Slice 1c perf block — resolution (the block was a debug-build artifact)

Supersedes the "per-cell perf blocker" framing in
`docs/handoffs/2026-07-16-terminal-slice-1c-percell-perf-block.md` and retires
its recommended fix (per-glyph shape cache / reopening plan mandate **C-a**).

## Finding

The Slice 1c perf gate measures the `render_realwindow` harness under **debug**
(`xtask` runs `cargo test --test render_realwindow` with no `--release`). Debug
is ~5.4× slower on the per-cell path (unoptimised `Font` clones + bounds checks).
The 8.3ms budget is a **120fps product target**; it was being checked against
**debug** numbers. In the profile that ships (release), the per-cell path already
meets 120fps with headroom.

### Measured (real window, WARMUP+MEASURE, this hardware, 2026-07-16)

| Workload | Path | Debug p95 | **Release p95** | 120fps budget |
| --- | --- | ---: | ---: | ---: |
| ASCII 200×50 | per-row | 6.0ms | **0.57ms** | 8.3ms ✓ |
| Dense-wide 200×50 | per-cell | 17.5ms | **2.52ms** | 8.3ms ✓ (3.3×) |
| Dense-wide 400×100 | per-cell | 48.4ms | **5.93ms** | 8.3ms ✓ (absolute, not the 20ms interim) |
| Pathological 100%-wide+50%-emoji | per-cell | 17.3ms | **2.37ms** | 30ms guard ✓ (12×) |

### Shape-vs-paint split (release, dense-wide 200×50)

`shape_line` = **0.06ms**, glyph `paint` enqueue = 0.26ms, rest of `paint_frame`
= ~2.2ms. Shaping is noise. The handoff's shape-cache recommendation would save
~0.06ms and was never the bottleneck. The residual ~2.2ms is dominated by
`resolve_cell_paint` being recomputed **2–3× per cell** (backgrounds pass, glyph
pass, decoration pass) — each doing a `Font` clone + HSLA conversion — plus a
full-grid background scan over cells that have no background.

## Resolution (approved scope: gate-release + tighten + cleanup)

1. **Gate in release.** Add `--release` to the `xtask` `render_realwindow`
   invocation. Perf budgets are shipping-profile targets and must be checked in
   the shipping profile. (Correctness phases — Menlo gate, paint routing, SGR —
   are profile-independent and ride along in the same release binary.)
2. **Tighten budgets to lock in release perf**, so a future regression that
   *doubles* release cost still trips the gate instead of hiding under 8.3ms.
   Calibrate to measured release numbers **after** the cleanup (step 3), with
   ~1.5–1.6× headroom to avoid flapping. Provisional: dense-wide 200×50 ≤ 4ms,
   dense-wide 400×100 ≤ 8.3ms (the absolute, no longer an interim), ASCII 200×50
   ≤ 1.5ms, pathological ≤ 8ms. Final numbers set from the post-cleanup run.
3. **Cheap cleanup for headroom** (serves the 90fps app-wide line): resolve each
   cell's paint **once per row** and share it across the background, glyph, and
   decoration passes; skip the background quad when the cell has no background.
   Visually identical (rows are vertically disjoint, so per-row bg→glyph→decor
   order equals the current global bg→glyph order). No new public API.

Not doing: per-glyph shape cache, glyph atlas, run-segmentation, C-a reopen. A
GPU glyph atlas (Ghostty's approach) remains the correct *long-term* direction
if we ever need to beat release by another large factor — that is a Slice-4-class
piece, out of scope here.

## Execution order

1. Commit A — gate `--release` + revert nothing (no temp code in tree).
2. Commit B — paint.rs cleanup (resolve-once-per-row + skip-empty-bg), TDD:
   existing T1–T7 correctness phases + paint-stats phases stay green; visual
   check by running the app.
3. Re-measure release; set tightened budgets (step 2 above) in a follow-up commit.
4. Cross-family review of Commit B diff (author = Opus → reviewer ≠ Opus family).
5. Green `cargo run -p xtask -- gate`; merge 1c to `terminal-ws`; then Slice 1d.

## Watch-outs

- The unfocused GPUI window throttles `request_animation_frame`; measure focused
  / foreground, or sample counts crawl (memory `gpui-test-noop-text-system`).
- The `1b` flake (`stop_publishes_final_frame_before_join`) is pre-existing and
  unrelated (handoff §Watch-outs).
- Debug perf is now explicitly *not* gated; debug dev sessions on dense-wide sit
  at ~17ms (≈60fps). Acceptable; debug absolute-ms budgets are too rustc-fragile
  to gate.
