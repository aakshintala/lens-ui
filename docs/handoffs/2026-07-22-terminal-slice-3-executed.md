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

- Full `xtask gate`: **GREEN** pre-review-fix (`gate: all checks passed` — all crate tests, both clippy configs, both real-window harnesses, benches compile, no contract drift). Post-review-fix (`b802044`), every changed component re-verified green individually — `lens-terminal` lib 120/120, `rss_probe` 3/3, `xtask` 5/5, Job A real-window, the RSS sweep, and `render_realwindow` isolated (PerfWide400x100 **4.743 ms** / budget 8). **One caveat:** a post-fix full-gate re-run tripped `render_realwindow` `PerfWide400x100` at **8.248 ms > 8 ms** — a load-induced flake of the pre-existing Slice-1c gate (paint times ~2–3× normal under full-gate concurrency), **not** a Slice-3 regression (Slice 3 touches nothing on the paint path); confirmed by the clean 4.743 ms isolated re-run. **Re-baselined 2026-07-22:** that 400×100 budget was 8.0 ms with zero margin under gate load; now **10.0 ms** with an honest load-tail comment (isolated ~4.8–6.2 ms is the SSOT). See the follow-up triage below.
- Consolidated cross-family review (codex `gpt-5.6-sol`, `codex exec -s read-only`): **0 Critical, 7 Important, 1 Minor.** Four fixed in `b802044` (see below); four documented as tracked follow-ups.
- Known parallel-load test flake (`engine::handle` mouse/hidden-tab timeouts under `cargo test` oversubscription) is pre-existing ([[worker-stall-gate-busy-spin-flake]]); isolated runs pass 5/5.

## Codex review — fixed (`b802044`)

- **I1 (product bug):** retained-rows sample rode the *visible-only* `maybe_publish` build path, so a HIDDEN tab — the prime fleet-trim target — reported a stale/zero estimate. Moved to a throttled worker-loop sample that runs regardless of visibility. Regression `retained_rows_track_a_hidden_tab`.
- **I2 (partial):** `VtEngine::total_rows()` now returns `Option`; skip the sample on FFI error instead of recording a spurious 0.
- **I3:** `rss_probe` distinguishes `FeedError::Stopped` (exit) from `Full` (retry) — no infinite loop / sweep hang on a dead worker.
- **I4:** `rss_probe` is fail-closed (non-zero exit on drain timeout / under-retention / RSS=0); the sweep bails on any non-zero probe exit.

## Codex review — follow-up triage (2026-07-22 session)

Triaged against the rule "carry only genuine dependency limitations." Four of the six were ours and got fixed this session; one was a decision (resolved); two are honest C-ABI / dep-internals carries; one moves to Slice 4. Fixes verified per-component green (both real-window harnesses isolated release + `lens-terminal` clippy `--all-targets`; the only gate failure was the pre-existing `engine::handle` oversubscription flake, which passes isolated).

**Fixed this session:**

- **I6 — Job-A false-green (FIXED).** `stream_perf_realwindow` now captures the flipped tab's `(frames_built, bytes_fed)` at the flip and asserts at exit that BOTH advanced — proving the show path built new frames and the feeder stayed alive, instead of letting a stale frame / dead feeder pass the gate.
- **I7 — Job-A build-p95 aliasing (FIXED).** Build p95 now samples once per *distinct* build (guard on per-engine `frames_built` advancing), not once per UI frame. Measured build p95 moved 0.564 → **1.183 ms** (still ⪡ 3 ms budget) — the per-frame read had been diluting p95 with cheap cached reads on no-build frames. **I7 second half was already resolved by I1** (`b802044`): the `total_rows()` FFI runs in the worker loop (`worker.rs:346`), outside the timed `started..elapsed` region (`worker.rs:879-882`), so it never charged build timing.
- **`retained_bytes_estimate` naming/doc (FIXED).** The field name promised bytes but delivers an ordinal rank score (real RSS ≈ 2.5–3.7× at scale, up to ~19× for small tabs; content-blind). Added a loud field doc in `inspect.rs`: ordinal-only, never surface/budget as bytes. (Surfaced by a user question this session — the raw number IS misleading if read as bytes.)
- **`render_realwindow` 400×100 flake re-baselined (FIXED).** `BUDGET_WIDE_400_MS` 8.0 → **10.0** with an honest comment: isolated release is ~4.8–6.2 ms (the perf SSOT), but the phase is measured under whole-gate CPU load where it reached ~8.2 ms (~75% inflation, not the ~30% the old comment claimed). 10.0 clears that load tail while still tripping a ≥~1.5× regression. Isolated re-run this session: **4.779 ms**.
- **64 MiB stack comment (FIXED).** `worker.rs` now documents that the reservation is virtual/lazily-committed (physical = touched depth, so fleet cost is address space not RAM), that 64 MiB is empirical not derived, and that the `max_scrollback` **byte** cap bounds production rows well under the 50k verified — so it is safe-by-margin; revisit only if `max_scrollback` is raised ~10×.

**Resolved (decision, no code change):**

- **I5 — equal-`total_rows` RSS divergence.** DECIDED: **keep the spec criterion.** `check_ordinal_fidelity` flags ordering *flips*, not *scale* — an equal-estimate pair is genuinely tied for trim priority (the estimate is only a rank; LRV reads real RSS for absolute budget), and gating on the compressible-vs-incompressible spread would fire on essentially every run. Gate stays spec-compliant. The headline equal-rows pair remains collected + reported informationally, which is exactly what Job B exists to measure.

**Genuine dependency limits (still carried — honest escalation candidates):**

- **I2 (alt-screen), Important — inherent C-ABI limit.** `total_rows()` is the *active screen*'s total; a program on the alternate screen (vim/less) reports ~viewport rows though primary scrollback is retained → the estimate under-counts and can invert victim ordering for such tabs. No vendored primary+alt-sum accessor exists → **escalation candidate for the byte-accurate-FFI conditional**, same class as the parked selector. Documented in `vt.rs::total_rows` + memory `terminal-max-scrollback-bytes-and-worker-stack`.
- **Worker stack shape — uncharacterised dep internal.** We never determined whether libghostty's stack use grows with row count (cliff would move under a much larger `max_scrollback`) or is a fixed deep frame. 64 MiB is safe under the current byte cap; characterising the real requirement is libghostty-internals work, parked. (Documented in `worker.rs`.)

**Moved to Slice 4:**

- **Minor — worker `.expect()` panics on stack/thread creation failure.** Engine-spawn failure deserves a real lifecycle policy (graceful degradation vs. panic), which belongs to Slice 4 lifecycle mechanisms, not a spot-fix here. (The prior `thread::spawn` also panicked, so this is no regression.)

### Cross-family review OF the fixes (codex `gpt-5.6-sol`, read-only)

Ran a second codex pass over the fix diff (author = Claude → mandatory cross-family review). It returned 4 findings; adjudicated:

- **Torn read in `record_frame_built` (Medium) — FIXED.** Count was incremented before `last_build_micros` stored, both Relaxed → a reader could pair a fresh count with the previous build's duration. Now stores duration first, publishes the count with `Release`, and `snapshot` reads count-then-duration with `Acquire`.
- **Empty build-samples panic (Low) — FIXED.** Per-distinct-build sampling can yield zero samples; `percentile_ms` would index an empty vec. Now fails closed ("build/feed path stalled") — strengthens the gate.
- **I6 one-build-then-stall / dead-feeder false-pass (High) — FIXED.** The `>baseline` (≥1) bar was trivial. Replaced with a **sustained** floor: the flipped tab must build ≥ `MIN_POST_FLIP_BUILDS` (8; expected tens). Sustained builds require sustained dirtying require a live feeder, so one floor subsumes both the stale-frame and dead-feeder cases (and retires the weak bytes_fed check, which a draining backlog could satisfy on a dead feeder).
- **I7 misses builds when `frames_built` jumps >1 (High) — ACCEPTED w/ doc, not fixed.** The clean fix (drain a per-build event ring) is **impossible here**: the event ring is flooded by per-feed `BytesFed` events and evicts `FrameBuilt` within ms. With one `last_build_micros` slot, a >1 jump (only on a UI frame delayed >~32 ms) loses intermediate durations. Residual is small — build throttle ≈ frame rate so Δ>1 is rare, a systemic regression still shows on the Δ=1 majority, and build p95 runs ~2.5× under budget. Documented in the harness; revisit only if build p95 becomes load-bearing.

**Verification note:** `clippy` clean · `inspect` lib tests 16/16 (covers the ordering fix) · `render_realwindow` 4.779 < 10 · **`stream_perf_realwindow` foreground run CONFIRMED — `all budgets OK` (paint_p95 3.494 < 5.5, build_p95 0.559 < 3), I6 floor + empty-builds guard did not trip.** Operational gotcha recorded in memory `terminal-realwindow-harness-pitfalls` (trap #4): these `harness=false` real-window bins frame-starve/hang when launched from a headless subprocess (gpui's display link pumps frames only for a FOREGROUND window; `System Events` can't front the non-bundled binary) — run them via `!`/`xtask gate`, don't re-launch headless.

## Next session — start here

Slice 3 is **fully closed** (all follow-ups triaged: fixed / decided / carried-as-dep-limit / deferred). `terminal-ws` pushed through `59b3b06`.

1. **Author the Slice 4 (lifecycle mechanisms) plan.** Full generation guard, Sleep/wake teardown, `ReplacementWaiting`; `Ended` **inert** (no 0.5.1/0.6.0 termination signal — verified); module/demo, host-agnostic. Pure `lens-terminal` + demo, no `lens-ui`/`lens-core`. Fold in the deferred **Minor** (worker `.expect()` → real engine-spawn-failure policy).
2. **After Slice 4: merge `terminal-ws → main`** — first terminal landing on main (low-risk, `lens-terminal` + demo + xtask only).
3. Then on a fresh branch off main: **Slice 5** (lens-ui minimal `FleetStore` + fleet policy; `session.superseded` as sub-slice 5-super, lens-core-first) → **Slice 6** (full terminal surface + E2E-in-app).

Honest carries into later slices (NOT Slice-4 blockers): **I2 alt-screen** `total_rows` under-count and the **uncharacterised libghostty stack shape** — both escalation candidates for the parked byte-accurate-FFI conditional if a future need is absolute rather than ordinal.
