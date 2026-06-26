# Handoff — lens-client Foundation execution

**Date:** 2026-06-25
**Purpose:** Everything a fresh session needs to *execute* the lens-client
Foundation plan. This is an execution handoff, not a design doc.

## Read order (entry points)

1. `AGENTS.md` + `CLAUDE.md` — shared rules + Claude-only delegation rules.
2. `docs/adr/0001-omnigent-contract-pinning.md` — why we're frozen at a commit; don't `git pull` the sibling omnigent checkout by reflex.
3. `docs/design/typed-client.md` — the **contract** (what the crate does). Ground truth.
4. `docs/design/typed-client-implementation.md` — the **build decisions** (D1–D4) layered on top.
5. `docs/superpowers/plans/2026-06-25-lens-client-foundation.md` — the plan to execute (6 TDD tasks).

## Execution protocol (chosen)

**Subagent-driven** (`superpowers:subagent-driven-development`), with this routing:

- **Build:** dispatch each plan task to **`cursor-delegate` on `composer-2.5`**. Hand it the task's exact steps verbatim (files, TDD steps, code, commands, expected output) + the Global Constraints block from the plan.
- **Review after every task:** the **Opus** (Claude) lead reviews the task diff before the next task starts — checks: matches the plan + decisions (esp. D2 no-async, `generated.rs` not hand-edited), tests are real and fail-first, `cargo test` + `cargo clippy -- -D warnings` + `cargo fmt --check` clean. Fix-loop with the same delegate until green, then proceed.
- **Diversity (cross-family) review:** **only at the end of the plan, and at big integration seams** — *not* per task.
  - **Seam A (optional, lead's discretion): Task 4 codegen.** `generated.rs` is load-bearing; if composer's `xtask` or the generated output looks non-trivial/surprising, route a `gpt-5.5`/`gemini-3.5` review of the `xtask` + a spot-check of generated types before building on them.
  - **Seam B (required): end of Plan 1.** Once Task 6's gated live handshake is green, route one cross-family review (`gpt-5.5` or `gemini-3.5`) over the whole foundation diff.
  - **Stated deviation:** this relaxes CLAUDE.md's per-change diversity-review MANDATORY to per-seam. Reason: Foundation tasks are small and mechanical, the Opus lead reviews each one, and there is almost no temporal/stateful logic in this phase (that risk lands in the Streaming plan, where per-task diversity review should return).

## Pre-flight

- omnigent installed from source at the pin (for Task 6's live test). Verify per `installing-omnigent-from-source`:
  - `omnigent --version` → `0.3.0.dev0 (36b2a11c, ...)`
  - `git -C ../omnigent rev-parse --short HEAD` → `36b2a11c` (must match the pin; do NOT pull it forward).
  - Daemon up; `omnigent server status` → note the ephemeral port for `LENS_OMNIGENT_URL`.
- Workspace already configured (edition 2024, toolchain 1.91.1, strict lints, `crates/*` + `spikes/*` members).

## Ground-truth reminders (don't relearn the hard way)

- **Pin is frozen** at `36b2a11c`; advancing is a separate owned task (ADR-0001). Next re-vendor trigger = a surface that needs a HEAD-only route, or `0.3.0` releasing.
- **D2:** sync/blocking only — no `tokio`, no `flume`, no async runtime in the crate.
- **`generated.rs`** is regenerated via `cargo run -p xtask -- codegen`, never hand-edited. Tweaks go in wrapper modules.
- **Default `cargo test` must pass with no server**; live tests are `--features live-tests` + `LENS_OMNIGENT_URL`.
- `openapi.json`'s `info.version` is a stale `0.1.0` — ignore it; the pin is `0.3.0.dev0`.
- Composer profile (LOW confidence, N=1 — `[[composer-delegation-profile]]`): may be weak on temporal/stateful logic. Foundation has little of that; relevant for the Streaming plan.

## Scope of this session

- **In:** Plan 1 Tasks 1–6 (scaffold, ids, connection, codegen, http/gate, handshake).
- **Out (separate plans, gated on `generated.rs`):** REST surface, SSE taxonomy + reader thread, reconnect, WS terminal, `xtask drift`/`live-test`, golden-SSE captures. Plan those after Task 4 lands and the generated types are concrete.

## Definition of done (Plan 1)

- All 6 tasks committed; `cargo test -p lens-client` green (serverless); clippy + fmt clean.
- Gated live handshake passes against the pinned server.
- End-of-plan cross-family review applied and its findings resolved.
- Next: write Plan 2 (REST surface) against the now-concrete `generated.rs`.
