# Handoff — Terminal Slice 5, Sub-slice D (fleet-integration) EXECUTED + WHOLE-BRANCH-REVIEWED

**Date:** 2026-07-23
**Branch:** `terminal-slice-5-fleetstore` (D does **not** merge independently)
**Range:** `bb90247..9e9c62f` (11 commits)
**Plan:** `docs/plans/2026-07-23-terminal-slice-5-D-fleet-integration.md`
**Whole-branch review:** `.superpowers/sdd/whole-branch-review.md`
**Resume ledger:** `.superpowers/sdd/progress.md`
**Prior handoff:** `docs/handoffs/2026-07-23-terminal-slice-5-D-planned.md`

## Status: D is CODE-COMPLETE and READY-TO-MERGE. **Live riders are the one outstanding gate.**

Executed subagent-driven: composer-2.5 authored every task via cursor-delegate; a fresh
grok-4.5 cross-family review ran per task (codex quota exhausted this week). Two tasks
needed a fix pass. The whole-branch review found a Critical that no per-task review could
have seen.

## Commits

| Commit | Change |
|--------|--------|
| `620dc41` | `test-util` host-event recorder seam on `TerminalTab` (forwarding has no other observable effect on an unbound tab) |
| `8e198fb` | poller → typed `SessionControl` → `FleetStore::on_session_control`; resource signals fan out to owned terminals |
| `e8b0f65` | `SessionLoader` seam + `session_loader` / `supersede_epoch` / `supersede_in_flight` + `FakeSessionLoader` |
| `b3b9eb0` | `move_terminal_members` — re-parent A→B with a **rebound** subscription (the trap) |
| `3a832b0` | `Superseded` → load B → move → drive `Transfer` (+ 5 guards) |
| `e69b4f6` | fix pass: vacuous load-failure test + untested already-tracked guard |
| `aab685e` | discard supersede `reason` explicitly instead of `allow(dead_code)` on the field |
| `80a0014` | real `AppSessionLoader` (background GET → `seed_disk` → `spawn_live_session`) + `main.rs` wiring |
| `9b22bcf` | surface supersede load failures instead of discarding them |
| `e465f4a` | **CRITICAL fix** — scope `supersede_epoch` per source session |
| `9e9c62f` | poison `ReentrantSessionLoader` locks the deferred-`load` call site |

## The Critical the whole-branch review caught

**This is the headline finding, and it is the argument for keeping the whole-branch pass.**

`supersede_in_flight` was correctly keyed per target, but `supersede_epoch` was a single
**global** `u64` on `FleetStore`, bumped by every supersede.

Sequence: session A supersedes to B (epoch → 1, the spawned task captures 1). Before that
load finishes, an unrelated session C supersedes to D (epoch → 2). B's load succeeds — and
its completion evaluates `2 != 1` and returns early. **A's terminals stay under the dead
session A, no `Transfer` is driven, the scrollback follow silently fails, and B is left as a
fully-spawned wasted live session.** Nothing logs. Nothing panics.

Reachable in ordinary use: a fleet is multi-session by definition, and the guard's window
spans a network GET.

**Root cause:** the plan asked for an apply-time guard "mirroring sub-slice A's
`reconnect_epoch` discipline". But `reconnect_epoch` is per-**tab**, whereas supersedes are
per-**session**. Copying the discipline without re-deriving its scope produced a guard that
was correct in shape and wrong in key.

**Fix (`e465f4a`):** `supersede_epochs: HashMap<SessionId, u64>` keyed by `from`; bumped and
compared per source; the entry is removed after a winning completion. Locked by
`concurrent_independent_supersedes_both_complete` — which the reviewer verified genuinely
**overlaps** the two loads (both supersedes fire inside one `store.update`; a gated loader
parks each `load` on a channel until released; the test asserts both targets loaded and both
members still under A/C *before* release) — plus
`stale_supersede_completion_is_rejected`, which proves the guard still rejects a truly stale
same-source completion rather than having been weakened into uselessness.

## The second whole-branch fix: locking a contract the rider could not

`load` must never be invoked while a `FleetStore` update is active — gpui entity updates are
not re-entrant and the poller calls `on_session_control` under an active update. That shape
was implemented and documented, but **nothing would have caught a regression**: both loaders
defer their own store mutation, so an inline-`load` implementation would fail no test — and
the live rider would not catch it either, because the real loader also defers.

`9e9c62f` adds `ReentrantSessionLoader`, a **poison** fake that does the forbidden thing
(synchronously `store.update`s from inside `load`), plus a supersede test that must complete
without panicking. Move the `load` call inline and it panics on gpui re-entrancy, naming the
contract.

## Gate (final, `9e9c62f`)

- `cargo test -p lens-terminal -p lens-ui --lib -- --test-threads=4` → **217 / 191 passed, 0 failed**
- `cargo clippy -p lens-terminal -p lens-ui -p lens-app --all-targets -- -D warnings` → clean
- `cargo fmt --all -- --check` → clean
- `cargo run -p xtask -- gate` (run at `80a0014`) → **`gate: all checks passed`**, incl. the
  openapi drift check (`no drift: 60 client paths`)

The known `wheel_no_tracking_local_scrolls_without_egress` oversubscription flake appeared
once (Task 1) and passed isolated in ~0.01s.

## NEXT — live riders (the only outstanding gate)

Needs a live omnigent 0.5.1 (skill `installing-omnigent-from-source`).

1. **Supersede scrollback survives `/clear`** — open a terminal, write recognizable output,
   `/clear`, assert the terminal is still live under B **and** the pre-`/clear` output is
   still scrollable. This is the only real proof of the retain-engine `Transfer`.
   **Also capture the SSE bytes** and confirm A really emits `session.superseded` with
   `target_conversation_id` = B. D deliberately does **not** build the `map_item` fallback
   (design §4.2), so the entire supersede follow depends on that in-memory outcome arriving.
   If the live server ever rotates without a usable `superseded` outcome, D has no fallback
   and §4.2 must be reopened — this rider is what would catch it.
2. **`4404`-first real interleaving** — force the `4404` ↔ `resource.deleted` race, confirm
   the tab adopts regardless of order.
3. **Transfer `output_gap` visual** — sub-slice A's review flagged that
   `on_reconnect_success` sets `output_gap = true` on the Transfer reuse path, possibly
   spurious when server B replays clear+redraw. Confirm visually, then keep or suppress.
4. Record results here and in the ledger. **Do not merge on a red rider.**

A failed rider is now debuggable: `9b22bcf` makes supersede load failures print instead of
vanishing.

## Deferred by user decision (2026-07-23) — dedicated session right after the riders

**Foreground blocking handshake.** `AppSessionLoader` runs the GET off the foreground as
specified, but then does `Client::new` (a multi-RTT blocking handshake) back **on** the app
executor, and `spawn_live_session` does *another* `Client::new` plus a blocking stream open
(`store.rs:391-394`, `:407-408`) → mid-`/clear` UI hitch risk.

Deferred, not dismissed. Two reasons: `spawn_live_session` is shared with the app **startup**
path (`main.rs` spawn loop, `fleet_verify.rs`), so moving its blocking work off the
foreground is a cross-cutting signature change outside slice 5's scope; and decisively,
**nobody has measured the hitch** — rider 3 is the first observation of it, and the fix
should be sized against that measurement rather than speculation.

## Carried Minors (reviewer adjudicated "carry", not merge-blocking)

- `policy.rs` `engine_config_for_test` keeps a `#[cfg_attr(not(test), allow(dead_code))]` for
  a pre-existing feature-unification clippy failure; narrowing it to `#[cfg(test)]` is cleaner.
- The Task-2 Deleted-case test asserts `ResourceDeleted { .. }` only, not `terminal_id`.
- No same-session multi-tab fan-out test; the poller→`on_session_control` wiring is untested
  (tests call the store directly).
- The subscription-construction block is duplicated verbatim between `register_terminal_member`
  and `move_terminal_members` — plan-mandated, but the two must stay in lockstep.
- No tests for `move_terminal_members`' empty-source, multi-member, or insert-collision paths.
- Orphan B after an aborted completion is benign waste. Epoch-map growth is bounded by
  successful completions (a failed load leaves one entry per source until a later success).
- No stage prefix on `spawn_live_session` errors, unlike the loader's own.

## Design amendment recorded during D (do not re-litigate)

Design §13's sub-slice-D bullet "`4404`-first driving (both orders → adoption fires)" is
amended to **"forwarding fidelity at D; adoption remains sub-slice A's e2e + the live
rider."** `lens-ui` cannot synthesize a `4404` (`apply_bridge_event` is private,
`live_tab_for_test` is crate-private) and a for-test tab has `generation: None`, so
`on_resource_signal` early-returns before adoption. A's
`fourohfour_first_then_delete_create_adopts` already binds the chain with production-authored
state. Source-verified in planning.
