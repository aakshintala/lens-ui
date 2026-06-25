# Performance

Performance is the prime objective. "Buttery smooth" is enforceable only as
numbers — these are them. Severity per `AGENTS.md`.

## Frame budget

- **MANDATORY** Target **120fps = 8.3ms/frame** on Apple Silicon (ProMotion).
- **MANDATORY** **90fps = 11.1ms** is the regression line. Dropping below 90fps
  in any interaction is a bug, not a tuning note.

## What "smooth" means (the three measured things)

1. **Frame time** — gpui paint duration per frame.
2. **Interaction latency** — input event → first paint.
3. **Scroll/stream jank** — dropped-frame count while the transcript streams.

## Benchmark levels

- **MANDATORY** Benchmark-or-it's-not-done: a perf-critical module ships with a
  benchmark, or it is not done. A logic core ships with tests.
- **MANDATORY** Benchmarks exist at every stack level:
  - **`lens-client`** — SSE/WS parse + codec throughput (`criterion`).
  - **state store** — event → store-update cost.
  - **render** — frame-timing harness against the budget above.
- **DEFAULT** Track regressions in CI; a benchmark crossing the 90fps line fails.

## Tooling

`criterion` for micro-benchmarks; a gpui frame-timing harness for render.
Profile on Apple Silicon, release builds only — never benchmark debug.
