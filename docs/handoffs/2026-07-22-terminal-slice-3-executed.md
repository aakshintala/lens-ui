# Terminal Slice 3 — EXECUTED (byte-accounting + perf acceptance)

**Date:** 2026-07-22 · **Branch:** `terminal-ws` (unpushed) · **Plan:** `docs/plans/2026-07-22-terminal-slice-3-byte-accounting-perf.md`

Slice 3 = a thin per-tab retained-bytes **estimate** through `EngineInspect`, plus two demo-hosted perf jobs (A: sustained multi-tab streaming; B: one-tab-per-process RSS sweep + ordinal-fidelity gate). Pure `lens-terminal` + demo + xtask — no `lens-ui`/`lens-core`, so `terminal-ws → main` stays mergeable after Slice 4.

## Commits (on `terminal-ws`)

| Commit | What |
| --- | --- |
| `af0b605` | Task 1 — retained-rows sample in `maybe_publish`; `EngineInspect.total_rows` + `retained_bytes_estimate` (= `total_rows × cols × PER_CELL_BYTES`, provisional 4); vendored `total_rows()` accessor, no re-vendor. |
| `67e8192` | **Engine fix** — 64 MiB worker stack (`WORKER_STACK_BYTES`); libghostty scrollback ops overflow the ~2 MiB default at ~2000+ rows. Regression `large_scrollback_feed_does_not_overflow_worker_stack`. |
| `d5a1d13` → `48d0d3e` | Task 2 — `rss_probe` demo bin (Job B measurement); fix sizes `max_scrollback` as a BYTE budget + dirty-then-build sample. |
| `89aca90` | Task 3 — `xtask terminal-rss-sweep` (Job B gate): probe orchestration + `check_ordinal_fidelity` fail-close. |
| `f30f894` | Task 4 — `stream_perf_realwindow` real-GPUI Job A gate; wired into the macOS `xtask gate`. |

## Two real bugs surfaced (both fixed)

1. **Worker-thread stack overflow** (`67e8192`). Feeding enough output to grow a large scrollback (~2000+ rows @ 200 cols) crashed the worker with SIGABRT "stack overflow" inside libghostty's scrollback page operations — a **real product bug** (any terminal with lots of output). Fix = 64 MiB lazily-committed stack; verified to 50k rows.
2. **`max_scrollback` is a BYTE budget, not a line count** — the vendored `TerminalOptions` doc comment ("Maximum number of lines") is misleading. The design spec's "10,000,000-byte scrollback" is correct. Harnesses must size it by bytes. (Memory `terminal-max-scrollback-bytes-and-worker-stack`.)

Corollary: `build_now` is a no-op when the engine isn't dirty (`total_rows` samples only on a fresh build) — harnesses feed a final byte before `build_now`.

## Job A evidence (`stream_perf_realwindow`, real macOS display)

```
STREAM paint_p95_ms=3.393 (budget 5.5) build_p95_ms=0.564 (budget 3) delta_rss_bytes=107020288
stream_perf_realwindow: all budgets OK  (exit 0)
```
4 engines, sustained dense wide/emoji feed, visible tab painted (PerCell); hidden-tab build-suppression assertion held. Single-frame PerCell was already settled (render_realwindow); this covers the residual p95-under-sustained-streaming risk — comfortably in budget.

## Job B evidence (`xtask terminal-rss-sweep`, cols=200)

```
  compressible   rows=1002   estimate=      801600 rss=    15433728
  incompressible rows=1002   estimate=      801600 rss=    15237120
  compressible   rows=5002   estimate=     4001600 rss=    26181632
  incompressible rows=5002   estimate=     4001600 rss=    22151168
  compressible   rows=20002  estimate=    16001600 rss=    66191360
  incompressible rows=20002  estimate=    16001600 rss=    48103424
  compressible   rows=50002  estimate=    40001600 rss=   147931136
  incompressible rows=50002  estimate=    40001600 rss=   100368384
  median rss/estimate ratio = 5.54
  OK — estimate is ordinally reliable (no RSS/estimate ordering flips)
```

- Estimate and RSS both monotonic with `total_rows`; no ordering flips → **byte-accurate FFI conditional NOT triggered** (the estimate is ordinally reliable for LRV trimming). The fail-closed escalation path (`GHOSTTY_TERMINAL_DATA_*` byte selector + re-vendor) stays parked per the design spec.
- **Surprise (recorded):** at equal `total_rows`, **compressible content uses MORE RSS than incompressible** — the opposite of the "scrollback is compressed" assumption. It does not flip the estimate↔RSS ordering, so it does not trip the gate; it is exactly the content-blindness Job B exists to measure.
- `PER_CELL_BYTES` left at the provisional **4** (ordinal-only; actual RSS/estimate ≈ 2.5–3.7× at scale, but the value doesn't affect the ordinal use). Fold a calibrated value only if a future need is absolute, not ordinal.

## Gate / review

- Full `xtask gate`: **GREEN** pre-review-fix (`gate: all checks passed` — all crate tests, both clippy configs, both real-window harnesses, benches compile, no contract drift). Post-review-fix (`b802044`), every changed component re-verified green individually — `lens-terminal` lib 120/120, `rss_probe` 3/3, `xtask` 5/5, Job A real-window, the RSS sweep, and `render_realwindow` isolated (PerfWide400x100 **4.743 ms** / budget 8). **One caveat:** a post-fix full-gate re-run tripped `render_realwindow` `PerfWide400x100` at **8.248 ms > 8 ms** — a load-induced flake of the pre-existing Slice-1c gate (paint times ~2–3× normal under full-gate concurrency), **not** a Slice-3 regression (Slice 3 touches nothing on the paint path); confirmed by the clean 4.743 ms isolated re-run. That 400×100 budget has thin margin under heavy load (see codex I7); re-baseline if it flaps.
- Consolidated cross-family review (codex `gpt-5.6-sol`, `codex exec -s read-only`): **0 Critical, 7 Important, 1 Minor.** Four fixed in `b802044` (see below); four documented as tracked follow-ups.
- Known parallel-load test flake (`engine::handle` mouse/hidden-tab timeouts under `cargo test` oversubscription) is pre-existing ([[worker-stall-gate-busy-spin-flake]]); isolated runs pass 5/5.

## Codex review — fixed (`b802044`)

- **I1 (product bug):** retained-rows sample rode the *visible-only* `maybe_publish` build path, so a HIDDEN tab — the prime fleet-trim target — reported a stale/zero estimate. Moved to a throttled worker-loop sample that runs regardless of visibility. Regression `retained_rows_track_a_hidden_tab`.
- **I2 (partial):** `VtEngine::total_rows()` now returns `Option`; skip the sample on FFI error instead of recording a spurious 0.
- **I3:** `rss_probe` distinguishes `FeedError::Stopped` (exit) from `Full` (retry) — no infinite loop / sweep hang on a dead worker.
- **I4:** `rss_probe` is fail-closed (non-zero exit on drain timeout / under-retention / RSS=0); the sweep bails on any non-zero probe exit.

## Codex review — tracked follow-ups (NOT yet fixed)

These do not block Slice 3 (the gate + both jobs pass on real, verified numbers), but the next session / Slice 5 should pick them up:

- **I2 (alt-screen), Important — inherent C-ABI limit.** `total_rows()` is the *active screen*'s total; a program on the alternate screen (vim/less) reports ~viewport rows though primary scrollback is retained → the estimate under-counts and can invert victim ordering for such tabs. No vendored primary+alt-sum accessor exists → this is an **escalation candidate for the byte-accurate-FFI conditional**, same class as the parked selector. Documented in `vt.rs::total_rows` + memory `terminal-max-scrollback-bytes-and-worker-stack`.
- **I5, Important — USER DECISION.** `check_ordinal_fidelity` only flags *strictly-larger-estimate-but-smaller-RSS* pairs; it deliberately does **not** flag the equal-`total_rows` adversarial pair even when RSS diverges a lot (50 002 rows: 148 MB compressible vs 100 MB incompressible, equal estimate). This matches the **spec's** escalation criterion ("compression *flips* LRV ordering, not merely *scales* it") — so the gate is spec-compliant — but it means the headline equal-rows pair is collected and not gated. **Decide:** (a) keep spec criterion (equal-estimate divergence is "scale", tolerated; LRV uses RSS for absolute budget), or (b) tighten the gate to also flag equal-estimate RSS spread. I did not change the criterion unilaterally.
- **I6, Important — Job-A false-green risk.** `stream_perf_realwindow` ignores the visibility-flip result and asserts no *post-flip* progress; under channel saturation or a dead feeder it could keep painting a stale frame and pass. Add a post-flip `frames_built`/`bytes_fed` progress assertion.
- **I7, Important — Job-A build-p95 precision.** Build p95 samples the cached `last_build_micros` once per UI frame (aliasing: misses/double-counts builds); the added `total_rows()` FFI cost also sits outside the timed `record_frame_built` region. Sample per distinct build (guard on `frames_built` advancing). NB: paint_p95 has run as high as **5.0 ms vs the 5.5 budget under load** — margin is thin; consider re-baselining if it flaps.
- **Minor:** the worker's `.expect()` makes 64 MiB stack-reservation / thread-creation failure a process panic (the prior `thread::spawn` also panicked); the larger per-engine *virtual* reservation raises fleet exposure (lazily committed, so physical cost is only what's touched).

## Next

Author the **Slice 4** (lifecycle mechanisms) plan; after 3+4, **merge `terminal-ws → main`** (first terminal landing on main), then Slices 5+6 on a fresh branch (lens-ui/lens-core integration).
