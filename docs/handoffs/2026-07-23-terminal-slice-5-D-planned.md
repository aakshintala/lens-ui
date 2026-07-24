# Handoff — Terminal Slice 5, Sub-slice D (fleet-integration) PLANNED + REVIEWED

**Date:** 2026-07-23
**Branch:** `terminal-slice-5-fleetstore` (D stays here — no independent merge)
**Plan:** `docs/plans/2026-07-23-terminal-slice-5-D-fleet-integration.md` (commit `4baa644`, revised `f9d3108`)
**Plan review:** `docs/reviews/2026-07-23-terminal-5-D-plan-review-grok.md` (grok-4.5, source-checked)
**Design SSOT:** `docs/specs/2026-07-22-terminal-slice-5-fleet-membership-design.md` §3 (D row), §4.1, §4.2, §9, §10, §13, §14
**Resume ledger:** `.superpowers/sdd/progress.md`

## Status: D is PLANNED + cross-family-reviewed. **NOT started.** Next session executes Task 1.

Nothing in `crates/` was touched this session. Only three doc commits:

| Commit | Change |
|--------|--------|
| `4baa644` | D implementation plan — 7 TDD tasks, 4 deferred design questions resolved |
| `f9d3108` | folded grok-4.5 review (2 Critical + 6 findings) → READY-TO-EXECUTE |
| *(this)* | STATUS + handoff |

## START HERE (cold boot)

1. Read the plan `docs/plans/2026-07-23-terminal-slice-5-D-fleet-integration.md` top-to-bottom — its "Resolved planning questions" and "Known trap" sections carry decisions you must not re-litigate.
2. Execute **subagent-driven** (memory-established pattern, same as sub-slice A): fresh `composer-2.5` per task via `cursor-delegate` (isolation `None`, server cwd) + a **fresh cross-family review per task**. Codex quota is exhausted this week (memory `codex-quota-exhausted-week-2026-07-23`) → use an **Opus** subagent or `grok-4.5`. composer authors, so composer cannot review its own task.
3. Per-task gate before each commit: `cargo test -p lens-ui --lib -- --test-threads=4` + `cargo clippy -p lens-ui --all-targets -- -D warnings` + `cargo fmt --all -- --check`. Task 1 also gates `-p lens-terminal`. **Do not run real-window harnesses.**
4. Tick the ledger `.superpowers/sdd/progress.md` after each task.

## Why D needed planning at all

The design deferred four questions to "D planning". All four are now resolved **and verified against source** (not assumed):

1. **Headless load-B is reachable but not free.** `spawn_live_session` (`store.rs:353`) is *not* UI-entangled — no `Window`/focus/user gesture; `fleet_verify.rs:73` already calls it headlessly. But `FleetStore` retains **no** `Connection`/`Client`/`data_dir` (`store.rs:59-77`), and a brand-new B must be seeded to the control store first (`scheduler.rs:103-105` → `SessionNotFound`), which needs a `SessionSnapshot` from `GET /v1/sessions/{id}`. **User decision: full supersede in D behind a `SessionLoader` seam** — store logic headless-tested with a fake; the real GET→seed→spawn lives in `lens-app` and is proven by the live rider.
2. **The GET is blocking** (`sessions.rs:1281`) → the seam returns a `Task` so IO runs on the background executor.
3. **The persisted-item `map_item` path (§4.2) is NOT needed** — `Transfer` is driven off the `Superseded` outcome (Q8), not B's snapshot. **Consequence: there is no fallback**, which is why the rider must assert the live event order actually contains `session.superseded` on A.
4. **DESIGN AMENDMENT — §13's D bullet "4404-first driving (both orders → adoption fires)" becomes "forwarding fidelity at D".** `lens-ui` cannot synthesize a `4404` (`apply_bridge_event` is private; `live_tab_for_test` is crate-private), and a for-test tab has `generation: None`, so `on_resource_signal` early-returns before adoption (`lens-terminal/src/lib.rs:1917-1918`). A's `fourohfour_first_then_delete_create_adopts` already binds the chain with **production-authored** state; the rider proves real ordering. The rejected alternative — a `bind_identity_for_test` seam — would hand-author state production owns (memory `false-green-probe-drives-production-path`).

## The trap D exists to avoid

`register_terminal_member` captures `session_for_sub` **into the subscription closure** at insert time (`crates/lens-ui/src/fleet/terminal.rs:241-247`), and `on_terminal_presentation_changed` early-returns when `terminals[session][key]` is absent (`:274-280`). A naive `terminals[A].remove(k)` → `terminals[B].insert(k, m)` therefore leaves the moved member's subscription calling back with **session A**, where the key no longer exists — silently killing sub-slice C's deferred `pending_sleep` for the transferred terminal. **Task 4 rebuilds the subscription against B** and its test asserts the post-move deferred sleep still fires.

## Task list (7)

| # | Deliverable | Crate |
|---|---|---|
| 1 | `test-util` host-event recorder on `TerminalTab` (forwarding has no other observable effect on an unbound tab) | lens-terminal |
| 2 | `SessionControl` + `on_session_control` routing arm + resource forwarding to owned terminals | lens-ui |
| 3 | `SessionLoader` seam + `session_loader`/`supersede_epoch`/`supersede_in_flight` fields + `FakeSessionLoader` | lens-ui |
| 4 | `move_terminal_members` — re-parent A→B with a **rebound** subscription | lens-ui |
| 5 | `Superseded` → load B → move → drive `Transfer` (+ guards) | lens-ui |
| 6 | real `AppSessionLoader` (background GET → `seed_disk` → `spawn_live_session`) + `main.rs` wiring | lens-app |
| 7 | live riders + whole-branch review | — |

## Plan-review outcome (grok-4.5 → NEEDS-REVISION → folded → READY-TO-EXECUTE)

Two Criticals were real compile/runtime bugs in the first draft; both fixed:

- **C1** — `session_loader` must be `pub(crate)`: Task 5's `on_supersede` lives in sibling module `terminal.rs`, where a `store.rs`-private field is invisible even inside `impl FleetStore`. (Exactly why `terminals` is already `pub(crate)`.)
- **C2** — the loader must **not** be invoked inline from `on_supersede`. The poller calls `on_session_control` under an active `FleetStore` update, and **gpui entity updates are not re-entrant** — the fake loader's `store.update` would panic. `load()` now runs inside the spawned task via `cx.update`, and the trait documents the rule.
- **I1** — added `supersede_epoch` (apply-time staleness guard, mirroring A's `reconnect_epoch` discipline) + `supersede_in_flight` (dedup), with tests.
- **I2/I3** — Task 6 rewritten against the real `main.rs` (`prep.conn`/`prep.data_dir` inside `if let Some(prep)` at `:101` — the draft wrongly said `config.*` at `:104`) and `GetOpts` pinned to `lens_client::sessions`.
- **M1** — grok's source-check **resolved** a soft spot I had flagged: `with_engine_for_test` sets `Lifecycle::Live` (`lib.rs:587-590`) and `is_sleepable` accepts `Live` (`terminal.rs:47-51`), so Task 4's `pending_sleep` assertion is valid. Hedge removed.
- All cited `file:line` claims were verified accurate.

## Deferred to live riders (unchanged from A)

- Cross-session supersede **success** path + scrollback-survives-`/clear` (design §10).
- `4404`-first real interleaving (design §9).
- Transfer `output_gap` visual — A's review flagged `on_reconnect_success` sets `output_gap = true` on the Transfer reuse path, possibly spurious when server B replays clear+redraw.
- **New (I6):** assert the live event sequence includes `session.superseded` on A — D has no `map_item` fallback by design.

## NEXT

1. **Execute D**, Task 1 → Task 7.
2. **Live riders** against a live omnigent 0.5.1 (skill `installing-omnigent-from-source`).
3. **Merge whole slice-5 (A+B+C+D) to `main` together** after D + final whole-branch review + green riders. No sub-slice merges independently.

## Known flake (not a regression)

`engine::handle::tests::wheel_no_tracking_local_scrolls_without_egress` can time out (~15s) under file-lock contention in a parallel gate. Isolated re-run passes in 0.00s. Known oversubscription class — memory `worker-stall-gate-busy-spin-flake`. Run gates with `--test-threads=4`; if only this test fails with a ~15s runtime, re-run it isolated.
