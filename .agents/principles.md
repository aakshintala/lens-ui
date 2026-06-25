# Principles

High-level engineering principles. Severity per `AGENTS.md`.

- **MANDATORY** Performance is the prime objective. Every layer carries a
  benchmark; perf is designed in, not tuned in later. → `performance.md`
- **MANDATORY** Functional core, imperative shell. Pure, deterministic logic
  cores; side-effects at the edges. This is what makes modules independently
  testable *and* benchmarkable.
- **MANDATORY** Deep modules, narrow interfaces. Small public surface,
  substantial implementation. A consumer understands a module without reading
  its internals; internals change without breaking consumers.
- **MANDATORY** Ground-truth discipline. Cite the pinned
  `vendor/omnigent-0.3.0.dev0/openapi.json` for every contract assertion.
  Pin-and-verify — never trust memory. omnigent `0.3.0.dev0` is a moving target;
  each module keeps a "what breaks if X changes" seam.
- **MANDATORY** Errors are values. Model failure explicitly with `Result` and
  typed error states; the UI degrades gracefully (see `rust-ui.md`).
- **MANDATORY** Introspectable at every layer (debuggability + extensibility).
  Each layer (`lens-client`, state store, render) implements a stable `Inspect`
  interface returning a typed, serializable snapshot **on demand**, and emits
  state-transition events to a ring buffer ("what now" + "how it got here"). An
  agent can read state at any layer without touching internals. Always present
  but **access-gated** (local-only / permission). **Zero-cost when not being
  read** — snapshot-on-demand, off the hot path; introspection must never fight
  the frame budget (`performance.md`).
- **DEFAULT** YAGNI. The design set (`docs/design/`) defines scope. Don't build
  for hypothetical futures.
- **DEFAULT** Resilience is first-class. Reconnect / no-replay / ring-buffer
  behavior is part of the design, not bolted on.
