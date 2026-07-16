# Spike A — Terminal render/paint viability

**Date:** 2026-07-15  
**Host:** Apple Silicon (`aarch64`), macOS 26.5.1  
**Build:** `cargo run --release` (gpui 0.2.2 + vendored libghostty-vt)  
**Display:** Real on-screen GPUI windows (not headless). Numbers are
trustworthy for the paint-closure methodology; they do **not** include
vsync/present wait.

**Plan:** `docs/plans/2026-07-15-terminal-spike-a-render-viability.md`  
**Scaffold:** `spikes/terminal-render/` (throwaway except liftable `paint.rs`)

---

## Verdict

**Render contract = full-snapshot repaint** (dirty-bitset cache is optional).

Decision tree gate: **S1 `full_redraw` p95 at 200×50 = 2.774 ms ≤ 8.3 ms**.
Full per-frame reshape+paint of a realistic maximized grid already fits the
frame budget, so Ghostty dirty-row tracking is not *load-bearing* for the
contract. S2 still helps (especially at 400×100 and on `partial_update`), but
the simpler full-snapshot shape is the right default.

---

## Headline p95 (paint closure, ms)

| Fixture | Grid | S1 p95 | S2 p95 |
| --- | --- | ---: | ---: |
| full_redraw | 80×24 | 2.939 | 1.109 |
| full_redraw | **200×50** | **2.774** | 2.447 |
| full_redraw | 400×100 | 5.964 | 4.098 |
| partial_update | 80×24 | 1.953 | 1.659 |
| partial_update | 200×50 | 3.372 | 2.662 |
| partial_update | 400×100 | 6.440 | 4.499 |

Budget line: **8.3 ms**. All S1/S2 ASCII-grid p95s above are under budget,
including the 400×100 stress (no cliff until PerCell wide/emoji — below).

---

## Full percentile tables (paint_ms)

Warmup discarded = 60; retained n = 500. Placement: PerRow for ASCII
fixtures; PerCell for `wide_and_sgr` (see alignment).

### S1 — per-row shape, no cache

| fixture | grid | place | p50 | p95 | p99 | snap p95 | input→1st |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: |
| full_redraw | 80×24 | PerRow | 1.509 | 2.939 | 2.967 | 0.044 | 2.946 |
| partial_update | 80×24 | PerRow | 0.619 | 1.953 | 3.358 | 0.010 | 2.932 |
| wide_and_sgr | 80×24 | PerCell | 1.744 | 3.725 | 3.753 | 0.001 | 5.671 |
| full_redraw | 200×50 | PerRow | 1.718 | 2.774 | 3.158 | 0.071 | 5.151 |
| partial_update | 200×50 | PerRow | 1.883 | 3.372 | 3.636 | 0.031 | 5.052 |
| wide_and_sgr | 200×50 | PerCell | 5.906 | 6.274 | 6.504 | 0.001 | 10.947 |
| full_redraw | 400×100 | PerRow | 5.360 | 5.964 | 6.044 | 0.108 | 10.511 |
| partial_update | 400×100 | PerRow | 5.787 | 6.440 | 6.639 | 0.006 | 10.500 |
| wide_and_sgr | 400×100 | PerCell | 16.130 | 16.486 | 16.725 | 0.001 | 23.152 |

### S2 — per-row shape, content-hash `ShapedLine` cache

| fixture | grid | place | p50 | p95 | p99 | snap p95 | input→1st | hits | misses |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| full_redraw | 80×24 | PerRow | 0.323 | 1.109 | 2.112 | 0.036 | 3.227 | 13416 | 24 |
| partial_update | 80×24 | PerRow | 0.451 | 1.659 | 2.205 | 0.010 | 2.896 | 11739 | 1701 |
| wide_and_sgr | 80×24 | PerCell | 1.648 | 3.165 | 3.720 | 0.001 | 6.350 | 0 | 0 |
| full_redraw | 200×50 | PerRow | 1.263 | 2.447 | 2.691 | 0.071 | 4.819 | 27950 | 50 |
| partial_update | 200×50 | PerRow | 1.479 | 2.662 | 3.153 | 0.032 | 5.127 | 26273 | 1727 |
| wide_and_sgr | 200×50 | PerCell | 6.009 | 6.387 | 6.506 | 0.001 | 11.500 | 0 | 0 |
| full_redraw | 400×100 | PerRow | 3.837 | 4.098 | 4.264 | 0.107 | 11.361 | 55900 | 100 |
| partial_update | 400×100 | PerRow | 4.124 | 4.499 | 4.666 | 0.005 | 10.956 | 54223 | 1777 |
| wide_and_sgr | 400×100 | PerCell | 16.224 | 16.671 | 16.898 | 0.001 | 24.058 | 0 | 0 |

`full_redraw` content is position-deterministic (no frame salt) → S2 hits after
the first shape per row. `partial_update` rewrites 3 rows/frame → ~3 misses/frame
(≈1700 over 560 frames). PerCell path does not use the row cache.

---

## 400×100 stress

Does **not** fall off a cliff for PerRow ASCII: S1 full_redraw p95 **5.96 ms**,
S2 **4.10 ms** — still inside 8.3 ms. The cliff is PerCell `wide_and_sgr`
(p95 **~16.5 ms**): per-cell `shape_line` × ~40k cells dominates.

---

## input → first-paint

One number per (fixture × grid), from first `vt_write` through completion of
the first reflecting paint (ms):

| fixture | 80×24 | 200×50 | 400×100 |
| --- | ---: | ---: | ---: |
| full_redraw (S1) | 2.95 | 5.15 | 10.51 |
| partial_update (S1) | 2.93 | 5.05 | 10.50 |
| wide_and_sgr (S1) | 5.67 | 10.95 | 23.15 |

---

## Wide / emoji alignment

**Per-row `shape_line` does not keep CJK/emoji on the monospace grid.**

Probe string `a日b😀c` against `col * cell_w` (tolerance 0.75px) failed on
`.ZedMono` + system fallbacks. The harness therefore paints `wide_and_sgr`
with **PerCell** placement (shape each non-blank cell at `col * cell_w`).
ASCII fixtures stayed on PerRow for the S1/S2 decision numbers.

Implication for the real contract: either (a) per-cell glyph/run placement for
wide/emoji rows, or (b) a shaping mode that forces monospace advances — do not
assume naive per-row `shape_line` is grid-safe.

---

## What this implies for `lens-terminal`

1. **Default contract: full-snapshot repaint** each frame (emit all visible
   rows). Dirty tracking is an optional optimization, not a requirement.
2. Keep a clean liftable `paint_grid` (see `spikes/terminal-render/src/paint.rs`)
   that can later grow an optional S2 cache if 400×100 / dense SGR needs it.
3. Plan for **grid-forced glyph placement** when `CellWide::Wide` / emoji
   appear; PerRow shaping alone will drift.
4. Snapshot cost is negligible (p95 ≪ 0.2 ms); shaping+primitive emission is
   the paint cost.

Raw TSV mirror: `spikes/terminal-render/MEASUREMENTS.md`.

---

## Lift-time correctness fixes (codex cross-family review of `paint.rs`)

Three real defects found in the liftable `paint.rs`. **None affect the spike's
verdict or perf numbers** (they're immaterial under a constant-font full-snapshot
repaint), but each must be fixed when the mapping is lifted into
`crates/lens-terminal`:

1. **[High] S2 cache key omits `font_size`/font.** The content hash
   (`paint.rs` `collect_rows`) covers grapheme/fg/bg/bold/selected but not
   `font_size` or font identity, so on zoom/font-change the cache serves a stale
   `ShapedLine` against new cell geometry. Also the `u64`-only key with no
   full-key equality check means a hash collision paints the wrong row. Fix: key
   the cache on `(font_size, font, content)` and retain the full key for equality
   (or fold font metrics into the hash + store the key).

2. **[Med] Alignment probe (`per_row_alignment_ok`) doesn't verify the cell after
   the emoji.** It checks the *start* of `日` and `😀` but never the glyph after
   `😀`, so an emoji that shapes narrower than 2 cells passes the probe while
   misaligning everything downstream. Fix: assert the advance *consumed* by each
   wide/emoji glyph (check the next cell's start), not just the wide glyph's start.

3. **[Med] Dirty state cleared before paint; paint errors discarded.**
   `collect_rows` marks rows + snapshot `Clean` *before* the `shaped.paint()`
   calls, whose `Result`s are dropped (`let _ =`). Under a dirty-driven renderer a
   failed glyph paint would not be retried (state already clean) and the error is
   silent. Fix: clear dirty only after confirmed-successful paint, and surface
   paint errors.

Codex confirmed the rest is solid: wide/spacer column accounting correct
(`Wide` → 2-cell bg, `SpacerTail` skipped after coverage, `SpacerHead`
non-rendering), no terminal-data panic paths (both `unwrap`s are on fixed
literals).

---

## Real-capture replay + a build requirement it surfaced

`terminal-render --replay=<capture.frames.jsonl> [--cols --rows --placement=per-row]`
feeds a Spike-B capture's server→client binary frames into a real `libghostty-vt`
terminal and paints it — end-to-end proof (omnigent PTY bytes → VT parse → GPUI
paint). Verified live on the captured Claude Code TUI: **VT parse, colors, and
layout are all correct.**

**New build requirement surfaced: `lens-terminal` MUST bundle a real monospaced
font.** With `.ZedMono` unavailable the fallback is proportional, and neither
placement is acceptable: **PerCell** jams each glyph into a `ch_advance('0')` cell →
visible per-char slack ("spaced-out" text); **PerRow** flows text naturally but
without a true monospace face the columns don't grid-align (box-drawing / logo /
status-bar drift). A bundled monospace font makes cell width == glyph advance, so
PerRow reads clean AND aligns. Deferred to the real build (glyph placement is
already on the punch-list above).
