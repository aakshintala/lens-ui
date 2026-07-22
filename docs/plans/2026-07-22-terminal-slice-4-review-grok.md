# Slice 4 plan review — Grok-4.5

## Critical

### 1. `reconnect_epoch` guards only the success arm — stale Fatal/RetriesExhausted still win
- **Location:** Task 3 Step 5; `lib.rs` `schedule_reconnect` (~1822–1894)
- **Why:** Plan snapshots `epoch` and drops a stale `Ok((resource, attach))`, but the existing loop still does unguarded `on_detach` on `ReconnectOutcome::Fatal` and on `retry.next_delay → None` (RetriesExhausted). After `ResourceDeleted` → `enter_replacement_waiting` (epoch++) or Sleep/adopt, a in-flight reconnect that then 404s (common: old instance gone) or exhausts retries will call `on_detach` and abort `ReplacementWaiting` / `Sleeping` / post-adopt `Live`.
- **Fix:** Apply the same `reconnect_epoch != epoch` check (and no-op) on **every** `weak.update` exit from the reconnect loop: Fatal, RetriesExhausted, and success. Optionally also bail the loop early when epoch mismatches before the next sleep/attempt.

### 2. Wake apply path does not re-check `generation.is_dirty()`
- **Location:** Task 5 Step 4 `on_wake` success arm
- **Why:** Dirty is checked only before spawning `discover_and_attach`. While the attach is in flight, lifecycle stays `Sleeping`, so Task 5’s Sleeping special-case correctly sets `saw_delete` on a late `ResourceDeleted`. The completion arm still calls `on_attached` if `reconnect_epoch` matches → **Wake can land `Live` on a generation that did not survive**, violating spec §Deliberate Sleep/wake (“reattaches only if the same observed resource generation survived; else Detached”).
- **Fix:** In the success `weak.update`, require `lifecycle == Sleeping && !generation.is_dirty()` (and epoch match) before `on_attached`; else `on_detach(IdentityChanged)` and `close_parts_off_foreground`.

### 3. Ratified §Identity deviation — reconnect does **not** restore Identity semantics
- **Location:** Design note; Task 1 reducer; claim vs `policy.rs::preflight_reconnect` (~217–227); spec §Identity (346–350), SPEC-GAPS “consult resource event history”
- **Why:** Treating matching `created` without prior `deleted` as `Unchanged` is fine for a lagged **self-echo**. The plan’s recovery story (“missed-delete recreation → next reconnect preflight GET”) is wrong as Identity recovery:
  1. `preflight_reconnect` only `GET`s existence — it does **not** consult persisted resource-event history / duplicate `resource.created` (spec + SPEC-GAPS require that for “full” generation guard on reconnect).
  2. On `Network`/`Internal` close (not `4404`), GET succeeds on the reused id → `on_reconnect_success` **retains the old engine** and sets `Live` + `output_gap` — silent **history mix** with a new PTY generation (spec: same-ID recreation outside positive reset → `Detached`; positive reset → **fresh** engine).
  3. Because `saw_delete` stayed false, later Sleep→Wake also treats the generation as clean and will fresh-attach without `IdentityChanged` — manual Sleep/Wake is not a reliable detector of the missed delete.
  4. Common agent-reset path often still `4404` → `TerminalGone` (Detach via close policy), so this is not “permanently stuck with zero paths,” but the deviation **does** leave a real Live-on-wrong-generation window with no automatic Detached/fresh-engine recovery, and the plan’s “recovered by reconnect” wording papers over that.
- **Fix (pick one, document loudly):**
  - **A (preferred for Slice 4 “full” guard):** On reconnect preflight, consult resource-event history (or equivalent); duplicate `created` for attached id → `Detach(IdentityChanged)` or force fresh-engine path — closes the degraded case the deviation creates; and/or
  - **B:** Keep echo=`Unchanged`, but rewrite the design note: reconnect is **not** Identity recovery; accepted residual race remains; add a test that Network-close + same-id GET retains engine (documents the gap). Do not claim preflight GET “lands on fresh instance” as a fix.
- **Verdict on safety:** Deviation is **not** “genuinely unsafe” for the self-echo rationale alone, but the **recovery reasoning is false** and can cause a real correctness bug (Live + retained scrollback on a new generation) when delete is missed and the WS fails as retryable rather than `4404`.

## Important

### 4. Task 1 commit breaks the crate: new `TerminalHostEvent` arms unused
- **Location:** Task 1 Steps 2 + 8 vs `lib.rs` `on_host_event` (~598–647)
- **Why:** Same-crate exhaustive match on `TerminalHostEvent` will fail once `ResourceCreated`/`ResourceDeleted` are added without arms (Task 3 wires them later). Intermediate commit is red.
- **Fix:** In Task 1, add no-op arms (or `_` if you intentionally widen) for the new variants; Task 3 replaces no-ops with real wiring. Same for Task 5’s `Reattach` if added before its handler.

### 5. `output_gap` not cleared on adopt / Wake / Sleep
- **Location:** Task 4 adoption (`on_attached`); Task 5 Sleep/Wake; `lib.rs` `on_attached` (~1470) never touches `output_gap`
- **Why:** Spec: Sleep/Wake add **no** gap; adoption is a clean fresh terminal (plan: `output_gap` false). After any prior reconnect, `presentation.output_gap` stays `true` forever across adopt/wake unless cleared.
- **Fix:** Clear `output_gap` in `enter_replacement_waiting`/`on_sleep`, and set explicitly in adopt/wake success (`false`) vs reattach/reconnect success (`true`). Prefer a `gap: bool` on the attach-application path as the plan’s open item suggests.

### 6. Spec coverage: reconnect-path generation guard still not “full”
- **Location:** Self-Review §1; completion matrix “generation guard (full)” on reconnect **and** wake; Slice 4 bullet
- **Why:** Wake gets `is_dirty`; live stream gets reducer; **reconnect** never gains history/duplicate-`created` detection. Combined with finding 3, Slice 4 under-delivers the matrix row the plan claims.
- **Fix:** Explicit Task (or Task 3/4 step) for preflight history consultation, or amend Self-Review + handoff to defer it with SPEC-GAPS citation (not “covered”).

### 7. Spec coverage: during-reconnect resize ordering
- **Location:** Completion matrix row “Resize end-to-end … during-reconnect **4**”; plan omits entirely
- **Why:** Matrix assigns reconnect-window resize/newest-size-before-input behavior to Slice 4; no task/test.
- **Fix:** Add a Task 3/5 acceptance test (or explicit deferral note if intentionally slipped) for resize while `Reconnecting`.

### 8. `on_sleep` allowed from `Detached` (incl. `ClientDetached`)
- **Location:** Task 5 `on_sleep` early-return only `Sleeping | Ended`
- **Why:** `ClientDetached` retains engine via `teardown_transport_off_foreground`. Sleep calls `teardown_runtime_full` → destroys reattach capability, then labels the tab `Sleeping`. Surprising and drops the 4405 reattach path.
- **Fix:** Only accept Sleep from `Live | Reconnecting | ReplacementWaiting` (and maybe `Starting` no-op).

### 9. Partial engine spawn: forwarder thread orphaned on worker spawn failure
- **Location:** Task 2 `spawn_from_parts`; `forwarder.rs` has **no** `Drop` join
- **Why:** `InputForwarder::spawn` succeeds then `spawn_worker` fails → `Err` drops forwarder without `sever_and_join` → detached forwarder thread until process exit.
- **Fix:** On worker spawn err, `input_forwarder.sever_and_join()` (or implement `Drop` that signals stop without joining on fg — but this construction path is off-fg) before returning `Err`.

### 10. `on_detach` always `teardown_runtime_full` — fine for plan’s Detach call sites, but Reattach comment overclaims
- **Location:** Task 5 Reattach docs (“4405 / exhausted-retry”); `reattach_available` only for `ClientDetached` (`on_detach` / policy)
- **Why:** RetriesExhausted cannot Reattach under the written `reattach_available` gate; comment/behavior mismatch. Not a logic bug if intentional.
- **Fix:** Narrow the comment to 4405/`ClientDetached` only, or separately define when RetriesExhausted offers reattach.

## Minor

### 11. Sleeping special-case vs Task 3 early-return — OK if Task 5 patches first
- **Location:** Task 3 `on_resource_signal` allow-list; Task 5 note
- **Why:** Mutation-inside `on_signal` **does** work: `saw_delete = true` before `AwaitReplacement` is returned; ignoring the verdict while Sleeping is sufficient. Risk is only implementing Task 3 allow-list literally and forgetting Task 5’s Sleeping branch before Sleep tests.
- **Fix:** In Task 3, already structure `on_resource_signal` with an explicit `Sleeping => { guard.on_signal; return }` stub so Task 5 only fills behavior.

### 12. `AdoptSuccessor` can double-fire while adopt is in flight
- **Location:** Task 4; reducer leaves `saw_delete` true until `on_attached` rebuilds guard
- **Why:** Duplicate matching `created` (or redelivery) while `ReplacementWaiting` schedules a second `discover_and_attach`; epoch cancel helps but wastes work.
- **Fix:** Clear a “adopt_in_flight” flag / bump epoch at first Adopt and ignore further Adopt until settle; or reset guard state to “awaiting apply.”

### 13. Plan names `spawn_from_parts_for_test` — does not exist
- **Location:** Task 2 Step 4
- **Why:** Code has `spawn_with_cmd_cap_for_test` / private `spawn_from_parts` only.
- **Fix:** Grep and update only real call sites.

### 14. Demo `Cmd+S` / `Cmd+R` may collide with macOS / future app menu
- **Location:** Task 6 Step 1
- **Why:** SPEC-GAPS notes dead/fragile global shortcuts; demo-only is fine but document conflicts.
- **Fix:** Prefer demo-local unused chords; document in demo help.

## Nit

### 15. `map_engine_spawn_error()` test is tautological
- **Location:** Task 2 Step 1
- **Why:** Asserting a free fn that returns a constant doesn’t prove `discover_and_attach` maps `EngineSpawnError`.
- **Fix:** `#[cfg(test)]` inject at `EngineHandle` boundary or map `Result` in a tiny wrapper used by both production and test.

---

**Verdict:** SHIP-WITH-FIXES

Ratified deviation is acceptable for self-echo avoidance, but **not** safe to ship under the plan’s stated reconnect-recovery rationale — fix cancellation (Critical 1), Wake dirty re-check (Critical 2), and either implement reconnect generation detection or rewrite the design note without claiming Identity recovery (Critical 3). Task 1 compile break and `output_gap`/spec-coverage items should land in the same pass.
