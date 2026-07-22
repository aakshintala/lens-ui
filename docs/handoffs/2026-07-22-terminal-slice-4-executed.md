# Terminal Slice 4 — EXECUTED (lifecycle mechanisms)

**Date:** 2026-07-22 · **Branch:** `terminal-ws` (unpushed) · **Plan:** `docs/plans/2026-07-22-terminal-slice-4-lifecycle-mechanisms.md`

Slice 4 = host-agnostic **lifecycle mechanisms** in `lens-terminal` (pure module + demo + xtask — no `lens-ui`/`lens-core`, so `terminal-ws → main` stays mergeable): a full generation guard, Sleep/Wake teardown-recreate, `ReplacementWaiting` exact-key successor adoption, an explicit Reattach for `4405`, and a non-panicking engine-spawn-failure policy. `Ended` stays **inert** (no 0.5.1/0.6.0 termination signal — verified).

## Commits (on `terminal-ws`, `8ff7cc8..f5ced39`, 14 commits)

| Task | Commit(s) | What |
| --- | --- | --- |
| 1 | `8ff7cc8` | Pure `GenerationGuard` reducer (`generation.rs`) + `TerminalHostEvent::ResourceCreated/Deleted` + `DetachedDetail::{IdentityChanged,ReplacementTimedOut,EngineSpawnFailed}`. |
| 2 | `a49d51e` | Fallible `EngineHandle::spawn -> Result<_, EngineSpawnError>` → `Detached(EngineSpawnFailed)`, never panic (folds the Slice-3 Minor; orphan-safe forwarder reclaim). |
| 3 | `8d6ddde` + `3205130` | Correlate resource signals → `ReplacementWaiting`/`Detached`/adopt; `reconnect_epoch` cancellation. Fix bound the cancellation to the real `schedule_reconnect` exit arms (`on_reconnect_exit_{success,fatal,exhausted}`). |
| 4 | `1fa29be` + `226f980` | Exact-key successor adoption (fresh engine) + bounded 30s `ReplacementWaiting` timeout. Fix removed a `cfg(test)` zero-duration timer race (no-op arm in test; `fire_replacement_timeout_now` is the deterministic seam). |
| 5 | `7c95e88` | Sleep / Wake / Reattach host actions. Wake re-checks `is_dirty()` **at apply time** (not epoch-only). Sleep gated to live-ish states. |
| 6 | `2cd9767` + `80e8656` + `57601d3` | Demo `ctrl-alt-{s,w,r,x,d}` chords (Sleep/Wake/Reattach/reset-adopt/reset-timeout) + on-screen help; opt-in P7/P8 live riders (`LENS_LIVE_SLEEP_WAKE`/`LENS_LIVE_REATTACH`). Gate-fix: **`rss_probe.rs` (demo crate) was a missed Task-2 call-site** for the now-`Result` spawn — fixed. |
| WB | `f5ced39` | **Whole-branch-review Critical fix** (see below). |
| docs | `38ca20d`, `8d3b61b`, SPEC-GAPS | Plan + Grok plan-review + SPEC-GAPS; a pre-existing Slice-3 fmt drift. |

## Review discipline (per user: Grok-4.5 per-task, Codex reserved for workstream end)

Every task: composer-2.5 author → **Grok-4.5** cross-family review (write-to-file) → fix wave → self-verify. Plan itself Grok-pre-reviewed (3 Critical / 6 Important folded before execution). Then a **whole-branch Grok review** over the integrated diff.

**Whole-branch review caught one Critical the per-task reviews structurally could not see** — a cross-seam race: `apply_bridge_event` (Slice-1d close handler, still draining the retained `policy_rx` after Slice-4 tears down the runtime) had **no lifecycle gate**, so a late/queued bridge close (the common **`4404`** on agent reset) clobbered `ReplacementWaiting`→adopt or yanked `Sleeping`→`Reconnecting`. **Fixed (`f5ced39`):** gate `apply_bridge_event` to no-op in `ReplacementWaiting|Sleeping|Detached|Ended`; `on_detach` now splits teardown (`ClientDetached`→transport-only keeps the engine; else full); bridge `StopDetached` routes through `on_detach` (folding the epoch-bump + `adopt_in_flight`-clear it was missing). 3 regression tests. **Lesson:** per-task reviews miss races between a new subsystem and an existing handler sharing a channel — the whole-branch pass is load-bearing.

## Verification

- **Headless gate GREEN:** `rustfmt --check`; `clippy -p lens-terminal --all-targets -- -D warnings` **and** `--features test-util,live-tests` (both 0); `clippy -p lens-terminal-demo --all-targets`; `lens-terminal --lib` **206/206** (at `--test-threads=4`); benches compile.
- **Known flake (NOT a Slice-4 regression):** `engine::handle` mouse/wheel timeout tests slow-run (~15–32 s) + fail under default all-CPU parallelism when the machine is loaded; pass isolated / at `--test-threads=2..4`. Pre-existing oversubscription class (memory `worker-stall-gate-busy-spin-flake`). **Gate commands must not pipe `cargo test` through `| tail` (masks the exit code) and should cap `--test-threads`.**

## ⏳ OPEN — foreground gate (run before merge)

The real-window harnesses frame-starve from a headless subprocess — run in the session foreground:

```
! cargo run -p xtask -- gate
```

Expect `gate: all checks passed` (real-window `render_realwindow`/`stream_perf_realwindow`/`mouse_realwindow`/`input_realwindow`/`presentation_realwindow` + benches + drift). Re-run any load-flaked component isolated. **Optional demo smoke** (foreground): launch `lens-terminal-demo`, exercise `ctrl-alt-s/w/r/x/d`. **Optional live riders** (needs a live omnigent + `LENS_OMNIGENT_*`): `LENS_LIVE_SLEEP_WAKE=1` (P7), `LENS_LIVE_REATTACH=1` (P8).

## Deferrals (documented, not silent drops)

- **Reconnect-path "full" generation guard** — DEFERRED to an upstream generation token (no-token race; `preflight_reconnect` only GETs existence). Common reset path is `4404` clean-detach; residual is rare+mild (stale scrollback only, active viewport correct). SPEC-GAPS updated; memory `terminal-resource-event-granularity`.
- **`4404`-first adoption ordering** — the bridge close vs host resource-signal arrive on independent transports; Slice 4 fixed the *clobber* direction but a `4404` landing *before* the `resource.deleted` on an `OpenOrCreate` reset goes `Detached` without re-adopt. Not module-resolvable → **Slice-5 integration** (real bridge↔host event model / FleetStore forwarding). SPEC-GAPS updated.
- **Inspect** omits Slice-4 correlation state (`generation`/`saw_delete`/`reconnect_epoch`/`adopt_in_flight`) → Slice-6 integrated inspect (plan allowed skip).
- **Demo** can't synthesize the co-emitted `4404`, so the demo reset path can't reproduce the Critical-1 race — deterministic tests cover it; live riders + Slice-5 cover the real ordering.

## Next session — start here

1. **Run the foreground gate** (above). If green →
2. **Merge `terminal-ws → main`** — first terminal landing on main (low-risk: `lens-terminal` + demo + xtask only). User's call.
3. Then on a fresh branch off main: **Slice 5** (lens-ui minimal `FleetStore` membership + fleet policy; `session.superseded` as sub-slice 5-super, lens-core-first) — and **resolve the `4404`-first adoption ordering** there.
