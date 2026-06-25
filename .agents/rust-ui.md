# Rust + gpui UI rules

Severity per `AGENTS.md`. Sourced from the design set (framework §2.1, state
model §8) and gpui perf reality.

## Threading & I/O

- **MANDATORY** Never block the foreground thread. All network/disk/SSE/WS I/O
  runs in `cx.background_spawn`; the UI thread only does `cx.update`/`cx.notify`.
- **MANDATORY** Bounded channels with backpressure. The pump → channel → store
  path coalesces/drops under stream bursts — never grows unboundedly.

## Render

- **MANDATORY** Render functions are cheap and allocation-light. No `format!`,
  parsing, or heavy compute in `render` — precompute in the model/view-model;
  render only reads. Hot paths reuse buffers.
- **MANDATORY** Virtualize unbounded lists. Transcript, board, and search use
  windowed / `uniform_list` rendering — never materialize off-screen items.
- **DEFAULT** Scope `cx.notify()` to the smallest entity so subscribers don't
  over-render.
- **DEFAULT** Pass handles / `Arc` / indices into views, not deep clones of
  large state.

## Correctness & types

- **MANDATORY** The UI never panics the process. No `.unwrap()`/`.expect()` on
  runtime-fallible paths; render error states as values.
- **MANDATORY** Typed end-to-end. Server enums stay typed from `lens-client` →
  view-model → render. No stringly-typed event dispatch.
- **MANDATORY** `unsafe` requires a `// SAFETY:` justification + review.
- **DEFAULT** Newtype IDs (`SessionId`, `AgentId`) over raw `String`/`Uuid`.

## Style & hygiene

- **MANDATORY** `clippy` clean + `rustfmt` (deny warnings in CI).
- **MANDATORY** A perf-critical widget ships with a benchmark; a logic core ships
  with tests (→ `performance.md`).
- **DEFAULT** Design tokens for all spacing/color/type — no hardcoded values
  (ties to the design language).
