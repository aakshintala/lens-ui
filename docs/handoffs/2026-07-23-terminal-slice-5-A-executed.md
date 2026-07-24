# Handoff — Terminal Slice 5, Sub-slice A (lens-terminal lifecycle) EXECUTED

**Date:** 2026-07-23
**Branch:** `terminal-slice-5-fleetstore` (A stays here — no independent merge)
**Plan:** `docs/plans/2026-07-23-terminal-slice-5-A-lifecycle.md`
**Resume ledger:** `.superpowers/sdd/progress.md`
**Process:** subagent-driven (composer-2.5 authors via cursor-delegate + fresh-Opus per-task reviews + Opus whole-slice review). Codex quota exhausted this week → Opus for all reviews.

## Status: A COMPLETE. Whole-slice-A Opus review = READY-TO-MERGE (with B/C/D).

Gate at tip `6b65a28`: **216/216** (`cargo test -p lens-terminal --lib -- --test-threads=4`, 2.2s), `cargo clippy -p lens-terminal --all-targets -- -D warnings` clean, `cargo fmt --all -- --check` clean.

## Commits (A-start `c89e3e5` → `6b65a28`, 6 commits)

| Commit | Change |
|--------|--------|
| `d8b13c7` | Task 1 — `scrollback_lines`→`scrollback_bytes` (BYTE budget), default `10_000_000` applied at `policy.rs` consumer; `#[non_exhaustive]`; `engine_config_for_test` seam. |
| `de760e0` | Task 2 — `OpenOrCreate` 4404 (`TerminalGone`) while Live → `ReplacementWaiting`; `Existing` stays hard-detach. Behavioral only. |
| `eb7290a` | Task 3 — `enter_replacement_waiting` RETAINS frozen engine (transport-only teardown); `adopt_successor`→unified `adopt(session,tid)`: same-session→drop retained + `discover_and_attach` fresh; changed-session→REUSE retained via `preflight_reconnect`+`attach`+`on_reconnect_success`. Every spawn apply-time epoch-guarded. |
| `58bc36c` | Task 3 test-strengthen — same-session test now asserts synchronous `runtime.is_none()` (proves fresh-drop path, not reuse); folded 4404-first e2e asserts `== Detached`. |
| `dc43f09` | Task 4 — `TerminalHostEvent::Transfer { new_session }` → `adopt(new_session, current_tid)` (reuse branch); synchronous reuse discriminator test (engine-pointer identity pre-park). |
| `6b65a28` | Task 4 post-review — DELETE redundant engine-level transfer-seed test (byte-identical to reconnect-seed; no session concept at engine layer); `Transfer` degenerate-session doc note; reconnect_seed live-rider comment. |

## Frozen-seam checklist (Opus whole-slice, all PASS, cited to source)

- **A. Epoch guard** — every adopt/attach/reconnect spawn re-checks `reconnect_epoch` at apply, clears `adopt_in_flight` + closes freshly-built parts/attach off-foreground on mismatch. Only unguarded spawns are the stateless teardown/close helpers (correct).
- **B. No retained-engine leak** on all four exits: same-session adopt (sync drop before fresh), 30s timeout (full teardown), Existing 4404 (hard-detach, no retain), Transfer (reuse on success; nothing orphaned on failure/mismatch).
- **C. Transfer reuses the EXACT retained engine** via `rt.engine_arc()` — no rebuild, no `teardown_runtime_full`.
- **D. Slice-4 frozen-state gate holds** — `apply_bridge_event` still no-ops in `ReplacementWaiting|Sleeping|Detached|Ended`; new 4404 branch sits below the gate → can't clobber/resurrect (the Slice-4 whole-branch Critical class).
- **E. 4404-first coherence** — guard survives `enter_replacement_waiting` (not cleared), `on_resource_signal` processes signals in `ReplacementWaiting`; 4404→delete→create→adopt chain proven by e2e `fourohfour_first_then_delete_create_adopts` (binds the real path).
- **F. No `adopt_in_flight` wedge** — cleared on every terminal branch.

## Non-blocking findings (for the whole-slice-5 final triage)

- **Minor** — `10_000_000` default is a bare literal at `policy.rs` consumer + restated in the `lib.rs` doc comment (drift risk). Plan mandated the literal at the consumer; consider a `const DEFAULT_SCROLLBACK_BYTES`.
- **Nit** — `on_reconnect_success` sets `presentation.output_gap = true` on the Transfer reuse path too; may be spurious when server B replays clear+redraw. **Live-rider validates the visual.**

## Deferred to LIVE RIDERS (offline-untestable — design Q6)

- Cross-session **success** path: `current_session==B`, `Live`, real transport reinstall against session B (no offline `AttachHandle` builder). Unit test proves only the synchronous reuse discriminator (engine retained, same pointer).
- Cross-session **no-double-feed** guarantee (server B replays clear+redraw, not full history) — a transport contract, not an engine test.
- Rider list: supersede scrollback, 4404-first ordering, transfer `output_gap` visual.

## NEXT

1. **Sub-slice D** (fleet-integration) — builds on A+B+C.
2. **Live riders** against a live omnigent (the deferred items above).
3. **Merge whole slice-5 (A+B+C+D) to main together** after D + final whole-branch Opus review + live riders. A does NOT merge independently.

## Optional / not done (deliberately)

- Plan's "Demo host events" (a `Transfer` demo chord mirroring the Slice-4 `ctrl-alt-*` chords): deferred to the live-rider work, where a cross-session Transfer is actually exercisable (it can't succeed offline / without a server B).

## Known flake (not a regression)

`engine::handle::tests::wheel_no_tracking_local_scrolls_without_egress` timed out (~15s) once under file-lock contention during a background gate; isolated re-run 5/5 pass in 0.00s. Known oversubscription class — memory `worker-stall-gate-busy-spin-flake`. Gate note: run `--test-threads=4`; if only this test fails with a ~15s runtime, re-run isolated.
