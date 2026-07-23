# Handoff — Terminal Slice 5: B+C done, A grill paused mid-Q1

**Authored:** 2026-07-22 · **Updated:** 2026-07-23 · **Branch:**
`terminal-slice-5-fleetstore` · **Design:**
`docs/specs/2026-07-22-terminal-slice-5-fleet-membership-design.md`

## START HERE (cold-boot for a new session)

State on branch `terminal-slice-5-fleetstore` (nothing pushed, tree clean):
- **B done** (`1bbcdef`) + **C done** (`2c12a91`) + compile-unblock (`9b5c541`).
- **B+C cross-family review DONE + fully resolved (2026-07-23).** codex `gpt-5.6`
  review returned DON'T-MERGE with 4 HIGH blockers + 4 MED/LOW
  (`docs/reviews/2026-07-23-slice5-BC-codex.md`). All resolved test-first:
  - `694870b` — findings 1 (control signals off the lossy `OutcomeRing` →
    reliable `send_blocking`), 2 (`cascade_wake` clears `pending_sleep`), 3 (new
    `TerminalHostEvent::End` → teardown → `Ended`), 4 (`real-window` feature
    isolates the harness trigger from `test-util` unification). 5+7 **re-scoped to
    Slice 6** (no production caller of the FleetStore terminal API exists yet).
  - `6e169ae` — findings 6 (engine 2-atomic retained-bytes fast path) + 8
    (lens-drive structured JSON). Composer-authored, Opus-reviewed.
  - `b12676e` — finding 3 COMPLETION: an **Opus re-review**
    (`docs/reviews/2026-07-23-slice5-BC-fixes-opus.md`) caught that the initial
    `open()` attach spawn was unguarded (resurrected an `Ended` tab, re-leaking);
    now epoch-guarded like the other 4 spawns. Opus re-review otherwise MERGE.
  - `e5c72fb` — cherry-picked the main-branch terminal-test speed fix (31s→3s).
  - Gate green: combined `--lib` (client 174 + core 311 + terminal 208 + ui 177),
    clippy `-D warnings` + fmt clean. Real-window bins no longer activate in the
    ordinary gate. (Did NOT run real-window harnesses — they hang headless.)
- **A grill DONE + A plan written (2026-07-23).** Q1–Q6 all answered; decisions
  baked into the plan `docs/plans/2026-07-23-terminal-slice-5-A-lifecycle.md`:
  - Q1 unified `adopt(session,tid)` (same→fresh/drop retained · changed→reuse).
  - Q2 A's agent-switch scope = "don't regress + don't leak"; auto-reopen-after-
    timeout → D. Q3 `Transfer { new_session }` through adopt()'s reuse branch +
    a cross-session no-double-feed test. Q4 `Existing` 4404 stays hard-detach.
    Q5 `scrollback_lines`→`scrollback_bytes`, default 10 MB, doc=bytes. Q6 whole
    slice-5 (A+B+C+D) merges to main together after D + final review + live riders.
  - Plan = 4 TDD tasks: (1) scrollback fix · (2) 4404→ReplacementWaiting · (3)
    retain frozen engine + unified adopt() · (4) `TerminalHostEvent::Transfer` +
    no-double-feed test. Every spawn epoch-guarded at apply (finding-3 lesson).

**Do next, in order:**
1. **Execute the A plan** `docs/plans/2026-07-23-terminal-slice-5-A-lifecycle.md`
   task-by-task: composer-2.5 authors each task, Opus reviews the diff between
   tasks, then a fresh-Opus whole-slice-A review at the end. Start with Task 1
   (scrollback fix — isolated/low-risk).
2. Then **D** (fleet-integration, needs A+B+C) → live riders (supersede scrollback,
   4404-first ordering) → final whole-branch Opus review → merge whole slice-5.

**REVIEW ROUTING (2026-07-23):** codex/gpt-5.6 quota exhausted for the week — use a
FRESH Opus subagent (builds+tests) or grok-4.5 via cursor-delegate for reviews, NOT
`codex exec`. (memory `codex-quota-exhausted-week-2026-07-23`.)

Everything below is the full grounding; the grill skill (`userSettings:grilling`)
governs Q-by-Q, one question at a time with a recommended answer.

## Execution decision (settled this session)

**Build sequentially, not in parallel.** Evaluated farming A/B/C to parallel
Opus/composer streams. Verdict: **no real wall-clock gain** — A is the long pole
*and* gates D (frozen-seam lifecycle rewrite + whole-branch review + live rider
before D can start); B/C would just finish early and wait. Parallel adds gate
contention (3 cargo gates oversubscribe one box), merge reconciliation, and
divides attention on the one slice least tolerant of it (A). Reviews/merges
serialize on me regardless.

**Order:** `B` (done) → `C` (done) → `A` (grilling) → `D` (+ live riders).
B and C were both delegated to composer and committed while A stays blocked on
the grill. **A and D remain — A needs the grill (Q1 below) + your input.**

## Sub-slice C — DONE, committed, PENDING review

- **Commit:** `2c12a91`. Composer-2.5 authored; **Opus fixed a flaky test** (below).
- **Gate green:** fmt + clippy `-D warnings` (lens-ui + lens-terminal) + `cargo test
  -p lens-ui` (173 unit + 6 acceptance). Verified deterministic: 40× single + 5×
  full parallel suite.
- **Files:** new `lens-ui/src/fleet/terminal.rs` (membership/policy + 11 tests);
  `fleet/{store.rs,mod.rs}`, `lens-ui/src/lib.rs`, `lens-ui/Cargo.toml` (deps
  lens-terminal + test-util dev-dep); `lens-terminal/{src/lib.rs,Cargo.toml}`
  (`retained_bytes_estimate` accessor + `test-util` feature).
- **API:** `FleetStore::{open_terminal, set_terminal_visible, close_terminal,
  cascade_sleep, cascade_wake, cascade_end, on_memory_pressure, idle_tick}`;
  `MemoryPressure::{Warning{free_fraction}, Critical}`; `TerminalMember`;
  `TerminalTab::retained_bytes_estimate() -> usize`.
- **Flaky-test fix (Opus):** the pressure tests read `retained_bytes_estimate`,
  which the worker samples ASYNC on build. It raced to 0 → `total_estimate==0` →
  nothing slept (~60% failure). `spawn_tab_with_rows` now barriers on a post-feed
  build (`frames_built` advance), gated on `feed_newlines > 0` because `build_now`
  is a no-op when the engine isn't dirty (would else hang the zero-feed tests).
- **⚠️ Composer judgment calls to validate in review:** (1) inner-map key is
  `TerminalKeyId` (fields of TerminalKey) because `TerminalKey` lacks `Hash`; (2)
  `open_terminal` on an `Existing` target maps `terminal_id → terminal_name` with
  empty `session_key`; (3) `test-util` wires `lens-client/test-util` for
  `Client::stub_for_test`.
- **Compile-unblock (`9b5c541`, separate commit):** B's new `ActorOutcome`/
  `StreamUpdate` variants broke exhaustive matches in lens-ui (card/model, focused,
  poller) + lens-drive. Added arms — no-op where control-path carries no card/replica
  delta; **poller.rs arm is interim — D replaces it with real routing**; lens-drive
  gets real JSON. Workspace compiles green.

## Sub-slice B — DONE, committed, PENDING review

- **Commit:** `1bbcdef` on `terminal-slice-5-fleetstore`. Authored by
  **composer-2.5**. Tree clean.
- **Gate green:** `cargo fmt --all -- --check` + `cargo clippy -p lens-client -p
  lens-core --all-targets -- -D warnings` + `cargo test -p lens-client -p
  lens-core`, all exit 0. **+10 tests (501→511)**: 2 lens-client parse, 4
  reducer (`folds.rs`), 4 actor (`feed.rs`).
- **Files:** `lens-client/src/stream/event.rs`; `lens-core/src/{reduce/update.rs,
  reduce/folds.rs, actor/outcome.rs, actor/feed.rs, actor/runloop.rs, actor/mod.rs}`.

**Public surface added (A/C/D authors depend on this):**
```rust
// lens-client
SessionEvent::ResourceCreated { resource_id: String, resource_type: String,
    terminal_name: Option<String>, session_key: Option<String> }  // terminal_* Some iff type=="terminal"
SessionEvent::ResourceDeleted { resource_id: String, resource_type: String, session_id: String }

// lens-core
StreamUpdate::Superseded { target_conversation_id: String, reason: String }
StreamUpdate::TerminalResourceCreated { terminal_id: TerminalId, terminal_name: String,
    session_key: String, session_id: SessionId }
StreamUpdate::TerminalResourceDeleted { terminal_id: TerminalId }

pub enum TerminalResourceSignal {
    Created { terminal_id: TerminalId, terminal_name: String, session_key: String, session_id: SessionId },
    Deleted { terminal_id: TerminalId },
}
ActorOutcome::Superseded { target_conversation_id: String, reason: String }
ActorOutcome::TerminalResource(TerminalResourceSignal)
```
Control routing: `feed::control_outcome_from_update` maps the 3 StreamUpdates →
ActorOutcome; `runloop::apply_reduced_batch` pushes to the outcome ring;
`feed::feed_updates` strips them before the ActorFeed FIFO. Payload is shaped to
feed Slice-4's `TerminalHostEvent::ResourceCreated/Deleted` directly (map
`resource_id → TerminalId`). `reduce/mod.rs map_item` left untouched (§4.2 defer).

**⚠️ NEXT ACTION on B: cross-family review (codex `gpt-5.6`, `codex exec
-s read-only`)** — composer authored, review diversity is MANDATORY before merge.
Validate composer's 4 judgment calls:
1. `session_id` on `TerminalResourceCreated` sourced from `state.id` (reducing
   actor's session) — spec put it on the signal, not on the client event. Check
   this is the right session on the supersede path (created fires on **B**).
2. Terminal `ResourceCreated` missing `terminal_name`/`session_key` → falls back
   to `ResourcesChanged` (fail-safe). Confirm acceptable.
3. `map_item` left as-is (deferred). Correct per §4.2.
4. `lens-ui` will not compile until D adds match arms for the new StreamUpdate/
   ActorOutcome variants — expected, out of B scope. (So workspace gate is red
   until D; per-crate gate for lens-client/lens-core is green.)

## Sub-slice A — grill IN PROGRESS (paused mid-Q1)

A = the risky `lens-terminal` lifecycle slice. Grill was grounded in the actual
state machine before questioning. **Grounding established (verified in code):**

- **ReplacementWaiting already exists and is already entered on the reset delete**
  via `on_resource_signal` → `GenerationVerdict::AwaitReplacement` (`lib.rs:1883`).
  The delete-first supersede/reset case is already handled. Slice 4 left open only
  the **4404-first** case.
- **`enter_replacement_waiting` currently `teardown_runtime_full`s the engine**
  (`lib.rs:1897`). A's "retain frozen engine" = swap to
  `teardown_transport_off_foreground` — but that's **global to every entry** into
  ReplacementWaiting (today only the AwaitReplacement caller; A adds the 4404 caller).
- **`apply_bridge_event` already early-returns for ReplacementWaiting/Sleeping/
  Detached/Ended** (`lib.rs:2152`, the S4 bridge-clobber fix). A late 4404 is
  already dropped. Gap is only a 4404 **while Live**: today → `on_close` →
  `StopDetached{TerminalGone}` → `on_detach` → full teardown → `Detached`.
- **`policy.on_close` is pure/target-agnostic** and `TerminalGone` in the bridge
  path arises only from a 4404 → the OpenOrCreate-vs-Existing branch must live in
  `apply_bridge_event` (on the `StopDetached{TerminalGone}` arm), not `policy.rs`.
- **No `Transfer` host event exists yet** — A adds it. `current_session:
  Option<SessionId>` (`lib.rs:460`) is the field the "session_id changed"
  discriminator compares against. `adopt_successor` (`lib.rs:1914`) currently does
  a fresh `discover_and_attach`. `REPLACEMENT_WAIT = 30s` (`lib.rs:1912`).

**A reframed into 5 concrete changes:**
1. Branch `StopDetached{TerminalGone}` on `self.target` kind in `apply_bridge_event`
   (OpenOrCreate → `enter_replacement_waiting`; Existing → `on_detach`).
2. `enter_replacement_waiting`: `teardown_runtime_full` → `teardown_transport_off_foreground`
   (retain frozen engine). Global to all callers.
3. Teach adoption reuse-vs-fresh (see Q1).
4. Add `TerminalHostEvent::Transfer { new_session }` + handler.
5. Scrollback-cap fix (§11): `TerminalOpenOptions::scrollback_lines` →
   `scrollback_bytes` (`#[non_exhaustive]` + `with_scrollback_bytes`), default
   `10_000_000`, fix `max_scrollback` doc to say **bytes**. (`policy.rs:250` default
   is currently `1000` ≈ ~7 rows — latent prod bug.)

### Q1 — PENDING USER ANSWER (adoption shape)

Once step 2 retains the engine, the tab holds a live engine in ReplacementWaiting.
Two adoption entry points:
- **autonomous `adopt_successor`** (a `resource.created` for our key) — can *only*
  be **same-session** (agent-switch), since the tab only gets signals for its own
  session and supersede's create fires on B.
- **host-driven `Transfer { new_session: B }`** — cross-session supersede (D-driven).

**My recommendation:** unify both through one `adopt(session_id, terminal_id, cx)`
that checks `session_id == self.current_session`: **same → fresh** (explicitly drop
the retained engine, `discover_and_attach` as today); **changed → reuse**
(transport-only re-attach on the frozen engine, retarget `current_session = B`).
Makes "retain iff session_id changed" a real runtime check and forces us to close
the **new engine-leak** step 2 introduces (adopt_successor's same-session path never
had a retained engine to clean up before; now it does). Alternative: two separate
paths with hardcoded fresh/reuse, leaning on "autonomous adopt is always same-session"
as an unchecked invariant. **I favor unified.**

### Remaining grill branches (queued, not yet asked)

- **Q2** agent-switch >30s timeout interaction: successor create arriving past
  `REPLACEMENT_WAIT` → `on_detach(ReplacementTimedOut)` → full teardown (drops
  retained engine, no leak) → `Detached`. Confirm this preserves "today's Slice-4
  fresh behavior" and clarify what makes agent-switch work end-to-end (FleetStore
  re-open vs. adopt). Is making agent-switch *work* even in A's scope, or just
  "don't regress + don't leak the retained engine"?
- **Q3** `Transfer` shape: only needs `new_session` (server transfers live, same
  `terminal_id` → keep `current_tid`)? Reuses `teardown_transport_off_foreground`
  + `on_reconnect_success` retargeting `current_session=B`? Confirm no-double-feed
  extends to this path (`engine/reconnect_seed.rs`).
- **Q4** Existing-target 4404 stays hard-detach — confirm (trivial, it's the design).
- **Q5** Scrollback-cap: default `10_000_000` bytes vs the 64 MiB worker stack
  (memory `terminal-max-scrollback-bytes-and-worker-stack`) — safe? Only consumer
  of `scrollback_lines` is lens-ui, on-branch? (quick grep before asking)
- **Q6** Land discipline: does A merge to `main` before C/D build on it (spec says
  "landed + whole-branch-reviewed *before*"), or stay on-branch with the whole
  workstream merging together? (Note `integration-workflow`: solo merges straight
  to main.)

## Tomorrow, in order
1. Answer Q1 → finish A grill (Q2–Q6) → write A plan (TDD tasks).
2. Cross-family review of **B + C together** via codex (`codex exec -s read-only`,
   gpt-5.6) — batched per spend policy; both composer-authored lens-* changes.
   Validate B's 4 + C's 3 judgment calls (listed in each section). Can run in
   background while grilling A resumes.
3. Execute A (composer author + Opus supervision on the frozen seam) → whole-branch
   review → live rider → land. Then D (needs A + B + C).

## Commits on branch this session (all on `terminal-slice-5-fleetstore`)
- `1bbcdef` B (core-surface) · `c4d25e7` docs · `9b5c541` compile-unblock ·
  `2c12a91` C (fleet-membership) · `1826cc3` docs (C landed + B+C review batched).
  Nothing pushed. **B + C owe cross-family review before any merge to main.**
