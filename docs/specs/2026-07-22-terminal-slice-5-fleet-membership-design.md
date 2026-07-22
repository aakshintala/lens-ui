# Terminal Slice 5 — Fleet membership + fleet policy (design)

**Date:** 2026-07-22 · **Branch:** `terminal-slice-5-fleetstore` (off `main` after the
Slices 0–4 landing `514f96b`) · **Status:** design, pre-plan.

Slice 5 is the first **lens-ui/lens-core-hosted** terminal slice. Slices 0–4 delivered
the whole `lens-terminal` module (transport, engine, render, interaction, lifecycle
*mechanisms*) plus the standalone `lens-terminal-demo` rig. Slice 5 makes a terminal a
**member of the existing `FleetStore`** and adds the **fleet-level policy** that only a
fleet host can own: memory-pressure eviction, idle auto-sleep, `session.superseded`
redirect, and the host→terminal resource-signal forwarding that closes Slice 4's
`4404`-first deferral.

## 1. Scope

**In scope (headless model/policy layer):**

1. Terminal membership in `FleetStore` — accounted, LRV-orderable, pressure-addressable.
2. Session→terminal lifecycle **cascade** (Sleep/Wake/End/Supersede fan out to owned terminals).
3. Fleet memory-pressure **LRV eviction** (Sleep-only; see §7).
4. Terminal **idle auto-sleep** (~10 m hidden-idle).
5. **`session.superseded` redirect** (sub-slice **5-super**): lens-core surface first, then FleetStore re-parents owned terminals into the target session.
6. **Resource-signal forwarding + `4404`-first adoption reconciliation** (sub-slice **5-resource** + 5-main): lens-client models the `resource.created`/`resource.deleted` payload; FleetStore forwards it to the owning terminal and reconciles it against the co-arriving bridge close.
7. **Scrollback-cap correctness fix** (bytes rename + real default) — folds in because it defines the LRV budget ceiling (§11).

**Out of scope → Slice 6:** any on-screen terminal tile in the real lens-ui board; production
placement/focus/theming; live E2E through lens-ui; integrated in-app perf sanity check. The
`lens-terminal-demo` remains the module visual/perf rig. **Validation for Slice 5 is
tests + the demo/live-rider rigs, not an in-app tile** — none of the seven items needs
rendering in the board to be exercised.

**Deferred (unchanged):** session auto-sleep *policy* (the ~10 m session threshold, an open
decision) stays a seam; byte-accurate retained-byte accounting + selective byte-trim stay
the parked fail-closed FFI conditional (Slice 3 found the estimate ordinally reliable).

## 2. Domain model

A **terminal is a child of a session**. `TerminalTarget` carries an explicit
`session_id: SessionId`; `TerminalKey { terminal_name, session_key }`; the server derives the
opaque `TerminalId` from `(terminal_name, session_key)`. One session owns **N** terminals (one
per `terminal_name`). Terminals are **not** peers of session cards.

`FleetStore` gains a terminal index **nested by owning session** (approach B — see §2.1):

```
struct TerminalMember {
    tab: Entity<TerminalTab>,
    last_viewed: u64,           // UiClock millis; updated on focus/visibility → LRV ordering
    hidden: bool,               // not the focused/active terminal
    // retained_bytes_estimate is read live from TerminalTab's EngineInspect, not cached here.
}
// FleetStore { …existing…, terminals: HashMap<SessionId, HashMap<TerminalKey, TerminalMember>> }
```

The **outer key is the owning `SessionId`** — the true ownership key, so parent→child is
structural. Cascade is an `O(1)` lookup of the session's terminals; re-parent-on-supersede moves
the inner map A→B; session-end drops `terminals.remove(&session_id)`. Fleet-wide LRV/pressure
iteration flattens: `terminals.values().flat_map(|m| m.values())`.

### 2.1 Why nested, not flat — and the deferred consolidation

There is **no `Session` abstraction** in the tree: a session is scattered across 5 parallel
`HashMap<SessionId, _>` in `FleetStore` (`cards`, `command_txs`, `pollers`, `stream_bridges`,
`reader_factories`, `reconcile_epochs`), and `SessionCard` is a pure view model that owns no
infrastructure. A flat `HashMap<TerminalKey, TerminalMember>` with a `session_id` back-link would
add a **6th parallel structure** and perpetuate that smell. The **nested map** gives terminals a
structural per-session home (local cascade / re-parent / teardown) **without touching the 5 landed
maps** — the pragmatic realization of "a session owns its terminals," scoped to this slice.

**Deferred follow-up (not Slice 5):** a real `Session` struct consolidating the 5 per-session
maps **+ terminals** into one record is the correct long-term fix, but migrating just-landed
transcript/board code (focus, poller routing, focused-replica, reconcile-epoch) is its own
reviewed refactor — a regression there would be hard to attribute inside a feature slice. Tracked
as a SPEC-GAP; the nested map is shaped so it drops into that `Session` struct later unchanged.

**Two independent Sleep triggers**, both landing on the same Slice-4 primitive
`TerminalHostEvent::Sleep`:

1. **Cascade (ownership):** session lifecycle fans *down* to its terminals — session Sleep →
   Sleep terminals; Wake → Wake; End/Archive → tear down; **superseded (A→B) → re-parent
   `session_id` A→B, engine retained**.
2. **Independent fleet policy:** a *hidden* terminal is Slept on its own under memory pressure
   (§7) or idle timeout (§8), **even while its parent session stays live**. The focused terminal
   is never slept by policy.

## 3. Sub-slice decomposition

Built and reviewed as three coordinated sub-slices (workstream discipline: composer-2.5 author,
≥1 cross-family review per seam, TDD, frequent commits).

| Sub-slice | Layers | Deliverable |
| --- | --- | --- |
| **5-super** | lens-client (done) → lens-core → lens-ui | `session.superseded` surfaced as an actor control outcome; FleetStore re-parents owned terminals into the target session. (T-0/transcript is now on `main`; its landing left the `folds.rs` Superseded arm untouched — no merge coordination needed.) |
| **5-resource** | lens-client → lens-core | Model the `resource.created` payload (`terminal_id`/`terminal_name`/`session_key`) + surface `resource_id`+`session_id` on delete; forward both as actor control outcomes. Prerequisite for forwarding + `4404`-first. |
| **5-main** | lens-ui `FleetStore` (+ small lens-terminal API fix) | Membership, cascade, pressure LRV, idle auto-sleep, resource-signal forwarding, `4404`-first reconciliation, scrollback-cap fix. |

Build order: **5-super and 5-resource first** (both are lens-core/client surface changes 5-main
consumes), then 5-main. 5-super and 5-resource are independent of each other and can be built in
either order / parallel.

## 4. lens-core / lens-client surfaces (5-super, 5-resource)

### 4.1 Control-outcome channel

Session-level control signals reach `FleetStore` through the **`ActorOutcome`** control path
(the same path that already carries `Parked`/`TransportChanged`/`Slept`), **not** through
`ActorFeed` (summary/transcript). Two new variants:

```
ActorOutcome::Superseded { target_conversation_id: String, reason: String }
ActorOutcome::TerminalResource(TerminalResourceSignal)   // Created{…} | Deleted{ terminal_id }
```

The reducer already sees `SessionEvent::Superseded { conversation_id, target_conversation_id,
reason }` (lens-client models it fully at `stream/event.rs:88`) but folds it to nothing
(`folds.rs:135`). 5-super emits a value-carrying `StreamUpdate::Superseded { … }`; the actor
(`actor/feed.rs`) maps that StreamUpdate to the `ActorOutcome::Superseded` control outcome.

**Routing to FleetStore:** the store-level hook already exists (transcript work) — the poller
holds `store: WeakEntity<FleetStore>` and already routes the feed batch through
`FleetStore::fold_session_feed` and transport outcomes through `FleetStore::apply_transport`. So
the two new control outcomes attach to the **same outcome arm** via a new store method
(`FleetStore::on_session_control(id, signal)`), leaving `apply_transport` and the card-bound
outcomes untouched. No new routing plumbing — just a new match arm + store method.

### 4.2 Resource-event modeling (5-resource)

lens-client parses `session.resource.created` as a **bare marker** today (`event.rs:68`, no
payload) and discards `session_id` on delete. Per memory `terminal-resource-event-granularity`,
omnigent 0.5.1 `resource.created` carries the full resource + key; `.deleted` carries only the id.
5-resource:

- Extend `SessionEvent::ResourceCreated` to carry `{ resource_id, resource_type, terminal_name,
  session_key }` (parse the resource object; `terminal_*` present only when
  `resource_type == "terminal"`). Extend `ResourceDeleted` to surface the already-parsed
  `session_id` (currently `_session_id`, discarded).
- Reducer emits value-carrying `StreamUpdate::TerminalResource{Created,Deleted}` for
  `resource_type == "terminal"` (non-terminal resources keep the existing `ResourcesChanged`
  marker — no behavior change for the board's resource pill).
- Actor maps those to `ActorOutcome::TerminalResource(_)`.

The `TerminalResourceSignal` payload is shaped to feed `TerminalHostEvent::ResourceCreated {
session_id, terminal_id, terminal_name, session_key }` / `ResourceDeleted { terminal_id }`
directly (the host event already exists, Slice 4). Map `resource_id → TerminalId`.

## 5. FleetStore terminal membership (5-main)

- **Open/register:** `FleetStore::open_terminal(target: TerminalTarget, options, host)` calls
  `lens_terminal::open(...)`, wraps the returned `Entity<TerminalTab>` in a `TerminalMember`, and
  inserts it under `terminals[session_id][terminal_key]` (creating the inner map on first
  terminal for that session), then subscribes to its `TerminalEvent` stream. `session_id` comes
  from the `TerminalTarget`. Tests use the `open_with_engine_for_test` seam (no live transport).
- **Visibility / LRV stamping:** focusing or showing a terminal stamps `last_viewed = clock.now`
  and clears `hidden`; blurring/hiding sets `hidden`. LRV order = ascending `last_viewed` over
  `hidden == true` members, flattened across all sessions' inner maps.
- **Close:** removing a member drops the entity (Slice-1b teardown is off-foreground/panic-free)
  and unsubscribes.

`FleetStore` already owns a `clock: Arc<dyn UiClock>` (manual-clock-injectable) — reused for LRV
timestamps and the idle tick, so all of §7/§8 is deterministic in tests.

## 6. Session→terminal cascade

A session-lifecycle signal fans out to `terminals.get(&session_id)` — the session's inner map:

| Session signal | Fan-out to owned terminals |
| --- | --- |
| Sleep (seam / user) | `TerminalHostEvent::Sleep` |
| Wake | `TerminalHostEvent::Wake` |
| End / Archive | tear down + remove member |
| Superseded (A→B) | move the inner terminal map A→B (§10), **no** Sleep — engine retained |

The **session-sleep trigger itself stays a seam** (the existing no-op `FleetStore::wake_session`
class): Slice 5 builds the *fan-out*, exercised in tests via an explicit "session slept/woke"
signal and in the demo via a chord. Session auto-sleep policy is not built here.

## 7. Fleet memory-pressure policy (Sleep-only, graduated)

**Forced simplification:** the vendored `libghostty-vt` exposes **no runtime byte-trim** and
`max_scrollback` is open-time-only (Slice 3). So the spec's "warning → trim histories, critical →
disconnect tabs" collapses to **one runtime lever: Sleep** (full engine + scrollback teardown,
final viewport retained, explicit reattach — Slice-4 `Sleep`). Warning and critical differ only
in aggression:

- **`MemoryPressure::Warning`** — Sleep hidden terminals in LRV order until estimated retained
  bytes fall under a soft budget (tunable; provisional target derived from the fleet estimate).
- **`MemoryPressure::Critical`** — Sleep **all** hidden terminals.
- The **focused terminal is never slept** by pressure. If only the focused terminal remains and
  pressure persists, Slice 5 does nothing further (never silently drops PTY bytes; keeping the
  active tab connected is the spec's last-resort rule; true byte-trim stays the parked FFI
  conditional).

**Signal source is a seam.** No `macOS didReceiveMemoryWarning` hook exists in the tree. Slice 5
adds `FleetStore::on_memory_pressure(level)` driven by tests + a demo chord; wiring it to a real
OS memory-warning source is Slice 6 / lens-app.

**LRV ordering** uses the per-terminal **`retained_bytes_estimate`** (read live via `EngineInspect`,
Slice 3) to pick which hidden terminals to sleep and to measure "under budget". Estimate is
ordinal-reliable (Slice-3 Job B), which is all LRV needs.

## 8. Terminal idle auto-sleep

A hidden terminal idle for a threshold (provisional ~10 m; STATUS open decision) auto-sleeps.
Driven off `FleetStore`'s `clock`: a periodic idle tick compares `clock.now − last_viewed`
against the threshold for each `hidden` member and Sleeps those over it. Deterministic under
`ManualUiClock`. The focused terminal never idle-sleeps (its `last_viewed` is continuously
refreshed while focused). Independent of the session's own idle/auto-sleep (deferred).

## 9. Resource-signal forwarding + `4404`-first reconciliation

**Forwarding:** on `ActorOutcome::TerminalResource(_)` for a session, `FleetStore` forwards it as
`TerminalHostEvent::ResourceCreated/Deleted` to **every owned terminal in that session** (the tab
filters to its own identity — Slice-4 contract). This is the "host forwards resource-generation
signals" seam the generation guard + adoption were built against.

**`4404`-first reconciliation (closes the Slice-4 deferral):** the bridge close (a `4404` on
agent reset, surfaced tab-side) and the host `resource.deleted` arrive on **independent
transports**. Slice 4 fixed the *clobber* direction but left the case where a `4404` lands
*before* `resource.deleted` on an `OpenOrCreate` reset → the tab goes `Detached` without
re-adopting the successor. `FleetStore` is the single component that sees **both** the terminal's
`TerminalEvent` (bridge/lifecycle) and the session's resource control outcomes, so it owns the
ordering: it forwards the `resource.deleted`/`resource.created` pair to the tab so the tab's
existing generation guard + `ReplacementWaiting` adoption fires regardless of which transport
won the race. The exact reconciliation rule (buffer window vs. idempotent re-drive) is a plan
decision; the tab's adoption machinery already exists — Slice 5 only guarantees the host delivers
the resource signals in a form the tab can act on.

The **real ordering proof** is an opt-in live rider (P7/P8 class, needs live omnigent); the
deterministic host-forwarding + reconciliation logic is unit-tested with injected outcomes. The
standalone demo cannot synthesize the co-emitted `4404`, so it does not reproduce the race — same
constraint documented in Slice 4.

## 10. `session.superseded` redirect (5-super)

On `ActorOutcome::Superseded { target_conversation_id, reason }` for session **A**, `FleetStore`
moves A's inner terminal map to session **B** (`terminals.remove(&A)` → merge into
`terminals[B]`), **retaining each engine** (the server moved the same PTY / `TerminalId` into B —
spec lines 343–345). No Sleep, no fresh engine. Subsequent cascade/forwarding for B then reaches
the re-parented terminals. Whether a terminal must re-key its `session_key` (part of its inner
`TerminalKey`) on the move, or the redirect is transparent at the WS layer, is a plan decision to
verify against the live contract (`reason` is currently always `"clear"`); if the key changes,
the inner-map entry is re-inserted under the new `TerminalKey` during the move.

## 11. Scrollback-cap correctness fix

Latent production bug in `lens-terminal`: `open()` → `policy.rs:250` defaults `max_scrollback` to
`1000`, but Slice 3 proved `max_scrollback` is a **BYTE budget, not lines** (vendored doc wrong;
memory `terminal-max-scrollback-bytes-and-worker-stack`). So the default is ~1000 bytes ≈ **~7
rows** at 80 cols, and the public field `TerminalOpenOptions.scrollback_lines` feeds a "lines"
number straight into a bytes parameter.

Fix (folds into 5-main because it defines the LRV budget ceiling and makes cap/estimate/budget all
byte-coherent):

- Rename public `TerminalOpenOptions::scrollback_lines` → `scrollback_bytes`
  (`#[non_exhaustive]` + `with_scrollback_bytes` setter; lens-ui is the only consumer, on-branch).
- Default to `10_000_000` (Ghostty's decimal app default — spec line 415).
- Fix the doc comment on `TerminalOptions.max_scrollback` / the field to say **bytes**.

## 12. Seams — real vs. test/demo-driven

| Concern | Slice 5 | Driven by |
| --- | --- | --- |
| Terminal membership / LRV / cascade fan-out | **built real** | tests + demo |
| Pressure LRV eviction (Sleep-only) | **built real** | `on_memory_pressure(level)` seam (test/demo chord) |
| Idle auto-sleep | **built real** | `ManualUiClock` tick (test) / demo |
| Resource forwarding + `4404`-first reconciliation | **built real** | injected control outcomes (test) + live rider (real ordering) |
| Superseded redirect | **built real** | injected `Superseded` outcome (test) + live rider |
| Session-sleep **trigger** | seam (existing no-op class) | explicit "session slept" signal (test/demo) |
| OS memory-warning **source** | seam | Slice 6 / lens-app |
| On-screen terminal tile | **not built** | Slice 6 |

## 13. Testing strategy

- **lens-core:** reducer tests for `StreamUpdate::Superseded` + `StreamUpdate::TerminalResource*`;
  actor tests that the mapped `ActorOutcome` control variants emit.
- **lens-client:** parse tests for the `resource.created` payload + `resource.deleted` `session_id`.
- **lens-ui (`FleetStore`, headless, `ManualUiClock` + `open_with_engine_for_test`):**
  membership register/close; LRV ordering; cascade fan-out per signal; pressure Warning/Critical
  Sleep selection (focused exempt, budget honored); idle auto-sleep threshold; resource-signal
  forwarding to the right owned terminals; `4404`-first reconciliation (both transport orders →
  adoption fires); superseded re-parent (engine retained, subsequent signals reach B).
- **Live riders (opt-in, live omnigent):** superseded redirect + `4404`-first real ordering
  (P7/P8 class). Not in the headless gate.
- **Gate:** `cargo run -p xtask -- gate` (workspace clippy `-D warnings` + fmt + tests). No new
  real-window harness needed (no rendering in Slice 5).

## 14. Risks & coordination

- **T-0 merge collision (5-super): RESOLVED.** T-0/transcript is now merged into `main` (this
  branch is rebased on it), and its landing left the `folds.rs` `Superseded` arm untouched, so
  5-super edits that block on top of the landed version — no coordination, no conflict.
- **Actor control routing:** the fleet-level store hook already exists (`fold_session_feed` /
  `apply_transport`, poller holds `WeakEntity<FleetStore>`). Adding the new control arm +
  `on_session_control` must not perturb the card-bound outcome path or the focused-replica
  routing (existing coalescing/decay + reconcile-epoch tests must stay green).
- **Live-contract unknowns (verify in the rider, don't invent):** whether superseded re-keys
  `session_key`; the exact `4404`↔`resource.deleted` interleaving on a real reset. The design
  keeps both behind the tab's existing (live-proven) generation-guard/adoption machinery, so
  Slice 5 forwards signals rather than re-implementing lifecycle.

## 15. Completion-matrix mapping (anti-drop)

| Design spec matrix row | Closed by |
| --- | --- |
| `session.superseded` redirect (retained engine follows `TerminalId`) | 5-super + §10 |
| Fleet memory-pressure trim/disconnect (LRV) | §7 (Sleep-only, per FFI gap) |
| lens-ui integration: terminal as `FleetStore` member (minimal) | §5 (membership; surface → Slice 6) |
| Slice-4 deferral: `4404`-first adoption ordering | §9 |
| (new) scrollback cap byte-correctness | §11 |
