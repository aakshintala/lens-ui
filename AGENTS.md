# Lens — agent guide (shared)

**Lens** is a native macOS desktop client for the **omnigent** AI agent framework
(`omnigent-ai/omnigent`, pinned `0.3.0.dev0`), built in **Rust + gpui**. It is a
pure *client* of omnigent's HTTP+SSE+WS API. The core bet: all-Rust, no IPC/JS
boundary — server enums stay typed from client to render.

This file is the **shared** entry point for every agent (Claude, cursor, etc.).
Claude reads `CLAUDE.md` in addition.

## Severity

- **MANDATORY** — violating it fails review and blocks merge.
- **DEFAULT** — strong default; deviate only with a stated reason in the commit/PR.
- **GUIDANCE** — principle/heuristic; judgment applies.

## Critical rules (detail in `.agents/`)

- **MANDATORY** Performance is the prime objective. Target 120fps (8.3ms/frame);
  90fps (11.1ms) is the regression line. Every layer carries a benchmark. → `.agents/performance.md`
- **MANDATORY** Never block the gpui foreground thread; all I/O off-thread. → `.agents/rust-ui.md`
- **MANDATORY** The UI never panics the process; errors are modeled values.
- **MANDATORY** Typed end-to-end — no stringly-typed event dispatch.
- **MANDATORY** Benchmark-or-it's-not-done on perf paths; logic cores ship tests.
- **MANDATORY** Comments explain *why* / the non-obvious — never narrate code. → `.agents/code-style.md`
- **MANDATORY** Ground-truth discipline — cite the pinned contract, never trust memory. → `.agents/principles.md`
- **MANDATORY** Introspectable at every layer — gated, on-demand state snapshots + event stream; zero-cost when off. → `.agents/principles.md`
- **MANDATORY** `clippy` clean + `rustfmt`; `unsafe` needs a `// SAFETY:` note.

## Index

| Doc | Contains |
| --- | --- |
| `.agents/principles.md` | High-level engineering principles |
| `.agents/performance.md` | Frame budget, perf contract, benchmark levels |
| `.agents/rust-ui.md` | Rust + gpui UI coding rules |
| `.agents/code-style.md` | Comments + modularity |
| `.agents/tooling.md` | rust-analyzer/LSP, tree-sitter, command set |
| `.agents/doc-conventions.md` | Design-doc + STATUS/handoff convention |

## Architecture

Start at `docs/design/README.md`, then the keystone
`docs/design/capability-map-and-design-language.md`. Ground truth is
`vendor/omnigent-0.3.0/openapi.json` (pin: `OMNIGENT_PIN`).
