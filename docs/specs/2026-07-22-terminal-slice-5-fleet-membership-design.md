# Terminal Slice 5 — Fleet membership + fleet policy (design)

**Date:** 2026-07-22 · **Branch:** `terminal-slice-5-fleetstore` (off `main` after the
Slices 0–4 landing `514f96b`) · **Status:** design, **grilled + revised 2026-07-22**
(decision ledger Q1–Q10 below), pre-plan.

Slice 5 makes a terminal a **member of the existing `FleetStore`** and adds the
**fleet-level policy** that only a fleet host can own: memory-pressure eviction, idle
auto-sleep, `session.superseded` redirect, and the host→terminal resource-signal
forwarding that closes Slice 4's `4404`-first deferral. Slices 0–4 delivered the whole
`lens-terminal` module (transport, engine, render, interaction, lifecycle *mechanisms*)
plus the standalone `lens-terminal-demo` rig.

**Not a pure "headless model/policy layer" any more.** Grilling established that two of the
seven items — `4404`-first (§9) and the `session.superseded` retain-engine redirect (§10) —
**cannot** be done by host-side FleetStore code forwarding signals; they require deliberate
`lens-terminal` **lifecycle state-machine** changes on the `ReplacementWaiting`/adopt seam
that Slice 4 froze. See sub-slice **A** (§3) — landed and whole-branch-reviewed *before* the
FleetStore policy work builds on it.

## Decision ledger (grill Q1–Q10, 2026-07-22)

| # | Decision | Home |
| --- | --- | --- |
| Q1 | Visibility stamping is a real `FleetStore::set_terminal_visible(...)` seam **Slice 6 wires**; policy is dormant-by-construction until then. | §5, §12 |
| Q2 | Just-opened terminal = `hidden=false`, `last_viewed=now`. `open_terminal` touches only the new member (no single-visible invariant). Only `set_terminal_visible` + cascade/policy flip `hidden`. | §5 |
| Q3 | `OpenOrCreate` `4404`/`TerminalNotFound` → `ReplacementWaiting` (not hard `Detached`); `Existing` keeps hard-detach. This is a **lens-terminal lifecycle change** (sub-slice A). | §9 |
| Q4 | Pressure Warning = **ordinal fraction-freed** (LRV coldest-first); Critical = sleep-all-hidden. RSS can't attribute per-terminal (wrong tool for selection/stop); OS source = Slice-6 seam. | §7 |
| Q5 | Cascade Sleep = all owned; **Wake = non-hidden only**. `Starting`-race closed by `pending_sleep` on `TerminalMember`. Idle tick ~30s. | §6, §8 |
| Q6–Q8 | Supersede = FleetStore **loads B headlessly** + moves member A→B + **drives `TerminalHostEvent::Transfer { new_session:B }`**. `session_key` **not** rekeyed (retained `TerminalId` ⇒ retained key). | §10 |
| Q7 | **Retain the engine** across `/clear` (scrollback survives); de-risked (no double-feed). Retain iff `session_id` changed; same-session agent-switch stays **fresh**. | §9, §10 |
| Q9 | Re-split into A/B/C/D (see §3). | §3 |
| Q10 | Policy eligibility = **`hidden && Live`** — transient lifecycles exempt (never destroy a mid-supersede retained engine). | §7, §8 |

## 1. Scope

**In scope (headless model/policy layer):**

1. Terminal membership in `FleetStore` — accounted, LRV-orderable, pressure-addressable.
2. Session→terminal lifecycle **cascade** (Sleep/Wake/End/Supersede fan out to owned terminals).
3. Fleet memory-pressure **LRV eviction** (Sleep-only; see §7).
4. Terminal **idle auto-sleep** (~10 m hidden-idle).
5. **`session.superseded` redirect**: FleetStore loads the target session B, re-parents A's owned terminals into B, and **drives a retain-engine `Transfer`** so scrollback survives `/clear` (§10).
6. **Resource-signal forwarding + `4404`-first adoption reconciliation**: lens-client models the `resource.created`/`resource.deleted` payload; a **lens-terminal lifecycle change** (`OpenOrCreate` `4404` → `ReplacementWaiting`) lets the tab re-adopt regardless of the `4404`↔`resource.deleted` race; FleetStore forwards/drives the signals (§9).
7. **Scrollback-cap correctness fix** (bytes rename + real default) — a **latent-bug fix** (bytes-not-lines), independent of the LRV budget (§11).

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
   Sleep **all** owned terminals; Wake → Wake **only non-hidden** owned terminals (Q5 — waking
   hidden ones just re-inflates memory policy reclaimed); End/Archive → tear down; **superseded
   (A→B) → load B + re-parent + retain-engine `Transfer`** (§10).
2. **Independent fleet policy:** a *hidden, `Live`* terminal is Slept on its own under memory
   pressure (§7) or idle timeout (§8), **even while its parent session stays live**.
   **Non-hidden terminals are never policy-slept**, and policy eligibility is
   **`hidden && Live`** (Q10) — transient lifecycles (`Starting`, `ReplacementWaiting`,
   `Reconnecting`) are exempt so policy never destroys a mid-supersede retained engine.

**Cascade/`Starting` race (Q5):** `on_sleep` only fires from `Live | Reconnecting |
ReplacementWaiting`, so a terminal caught in `Starting` when a Sleep cascade arrives would
silently ignore it and come up `Live` under a slept session — a **leak** neither policy path
reaps (both gate on `hidden==true`, and it opened `hidden=false`). Fix: `TerminalMember` carries
`pending_sleep: bool`; a cascade Sleep against a not-yet-sleepable tab sets it instead of firing,
and FleetStore's existing `TerminalEvent` subscription applies the deferred `Sleep` on the next
transition into a sleepable state. Cleared on show/focus (`set_terminal_visible(true)`) and on
End/Archive teardown.

## 3. Sub-slice decomposition (re-split, Q9)

Built and reviewed as four coordinated sub-slices (workstream discipline: composer-2.5 author,
≥1 cross-family review per seam, TDD, frequent commits). Re-split from the original
`5-super`/`5-resource`/`5-main` because grilling surfaced two `lens-terminal` **lifecycle**
changes that must be isolated + whole-branch-reviewed *before* FleetStore builds on them.

| Sub-slice | Layer | Deliverable | Proof |
| --- | --- | --- | --- |
| **A · terminal-lifecycle** | lens-terminal | • `OpenOrCreate` `4404`/`TerminalNotFound` → `ReplacementWaiting` (not hard `Detached`) **[Q3]**<br>• `enter_replacement_waiting` → transport-only teardown, **retain frozen engine** **[Q7]**<br>• cross-session re-attach op + `TerminalHostEvent::Transfer { new_session }`; retain engine iff `session_id` changed **[Q7]**<br>• scrollback-cap fix **[§11]** | unit + demo-driven host events; **whole-branch review** (S4 `ReplacementWaiting`/adopt/bridge seam) |
| **B · core-surface** | lens-client + lens-core | • lens-client: extend `ResourceCreated` payload + surface `ResourceDeleted` `session_id`<br>• `Superseded` fold → `StreamUpdate::Superseded` → `ActorOutcome::Superseded`<br>• `TerminalResource{Created,Deleted}` fold → `ActorOutcome::TerminalResource`<br>• persisted-item path (`reduce/mod.rs map_item`) **only if D needs it** (see §10) | reducer + actor + parse tests |
| **C · fleet-membership** | lens-ui `FleetStore` (self-contained) | • nested membership, `open_terminal`, `set_terminal_visible`, visible-on-open **[Q1/Q2]**<br>• lightweight `retained_bytes_estimate()` accessor (2 atomics)<br>• cascade Sleep-all / **Wake-non-hidden** + `pending_sleep` **[Q5]**<br>• pressure Warning=fraction-freed / Critical=all-hidden **[Q4]**<br>• idle auto-sleep, ~30s tick **[Q8/Q5]** | headless FleetStore tests (`ManualUiClock` + `open_with_engine_for_test`) — **no dep on A/B** |
| **D · fleet-integration** | lens-ui `FleetStore` (cross-layer) | • resource-signal forwarding to owned terminals<br>• `4404`-first reconciliation driving **[Q3]**<br>• supersede: load B headlessly + move member A→B + drive `Transfer` **[Q6/Q7]** | injected-outcome tests + **live riders** (supersede scrollback, `4404`-first ordering); **whole-branch review** (actor-outcome + terminal-event cross-seam) |

**Build order:** `A ∥ B ∥ C` (all independent — A/B are producer crates; C tests entirely against
`open_with_engine_for_test`), then **D** (needs A + B + C). Live riders after D.

```
   ┌─ A (terminal-lifecycle) ─┐
   ├─ B (core-surface) ───────┤   independent, parallel
   └─ C (fleet-membership) ───┘
                              ▼
              D (fleet-integration) → live riders
```

## 4. lens-core / lens-client surfaces (sub-slice B)

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
(`folds.rs:135`). B emits a value-carrying `StreamUpdate::Superseded { … }`; the actor
(`actor/feed.rs`) maps that StreamUpdate to the `ActorOutcome::Superseded` control outcome.

**Routing to FleetStore:** the store-level hook already exists (transcript work) — the poller
holds `store: WeakEntity<FleetStore>` and already routes the feed batch through
`FleetStore::fold_session_feed` and transport outcomes through `FleetStore::apply_transport`. So
the two new control outcomes attach to the **same outcome arm** via a new store method
(`FleetStore::on_session_control(id, signal)`), leaving `apply_transport` and the card-bound
outcomes untouched. No new routing plumbing — just a new match arm + store method.

### 4.2 Resource-event modeling (sub-slice B)

lens-client parses `session.resource.created` as a **bare marker** today (`event.rs:68`, no
payload) and discards `session_id` on delete. Per memory `terminal-resource-event-granularity`,
omnigent 0.5.1 `resource.created` carries the full resource + key; `.deleted` carries only the id.
Sub-slice B:

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

**Persisted-item path (open, resolve in D planning):** resource events are also persisted as
`resource_event` conversation items, which today fold to the valueless `ResourcesChanged` marker
(`reduce/mod.rs` `map_item → None`). This *only* matters if D discovers B's transferred terminal
via B's **snapshot** rather than via the FleetStore-driven `Transfer` (§10). Since Q8 drives the
`Transfer` from the `superseded` event directly, the item path is likely **unnecessary** — add the
`map_item` extension to B only if D's plan actually depends on it.

## 5. FleetStore terminal membership (sub-slice C)

- **Open/register:** `FleetStore::open_terminal(target: TerminalTarget, options, host)` calls
  `lens_terminal::open(...)`, wraps the returned `Entity<TerminalTab>` in a `TerminalMember`, and
  inserts it under `terminals[session_id][terminal_key]` (creating the inner map on first
  terminal for that session), then subscribes to its `TerminalEvent` stream. `session_id` comes
  from the `TerminalTarget`. **A just-opened terminal is `hidden=false`, `last_viewed=clock.now`
  (Q2)** — open is an explicit host action, so the user is about to use it. `open_terminal`
  **touches only the new member** — showing a second terminal in a session does *not* background
  the first; whether that becomes a single-visible invariant is Slice-6 tab-UI (Q2). Tests use the
  `open_with_engine_for_test` seam (no live transport).
- **Visibility / LRV stamping — the Slice-6 seam (Q1):** `FleetStore::set_terminal_visible(
  session_id, terminal_key, visible)` stamps `last_viewed = clock.now` + clears `hidden` (true) or
  sets `hidden` (false). **This is the one method Slice 6's board tile will call** — the analogue of
  the board's existing `apply_visibility_gate → SessionCardView::set_visible`. Until Slice 6 calls
  it, every member stays `hidden=false`, so pressure/idle policy is **dormant-by-construction**
  (nothing is eligible). Exercised in Slice 5 via tests + a demo chord only. LRV order = ascending
  `last_viewed` over `hidden == true` members, flattened across all sessions' inner maps.
- **Estimate accessor:** LRV reads a new lightweight `EngineHandle::retained_bytes_estimate()`
  (2 atomic loads) rather than cloning a full `EngineInspect` per member per tick. The estimate is
  maintained by the worker unconditionally (no `set_inspect_enabled` needed); it is ordinal, stale
  ≤16 ms, 0 until first sample, and undercounts alt-screen — all tolerable for ordinal LRV.
- **Close:** removing a member drops the entity (Slice-1b teardown is off-foreground/panic-free)
  and unsubscribes.

`FleetStore` already owns a `clock: Arc<dyn UiClock>` (manual-clock-injectable) — reused for LRV
timestamps and the idle tick, so all of §7/§8 is deterministic in tests.

## 6. Session→terminal cascade

A session-lifecycle signal fans out to `terminals.get(&session_id)` — the session's inner map:

| Session signal | Fan-out to owned terminals |
| --- | --- |
| Sleep (seam / user) | `TerminalHostEvent::Sleep` to **all** owned (whole session dormant). Not-yet-sleepable (`Starting`) → set `pending_sleep` (Q5). |
| Wake | `TerminalHostEvent::Wake` to **non-hidden** owned only (Q5) — hidden ones stay `Sleeping` (waking them just re-inflates memory policy reclaimed). |
| End / Archive | tear down + remove member (clears `pending_sleep`). |
| Superseded (A→B) | load B + move member A→B + drive retain-engine `Transfer` (§10) — **no** Sleep, scrollback retained. |

The **session-sleep trigger itself stays a seam** (the existing no-op `FleetStore::wake_session`
class): Slice 5 builds the *fan-out*, exercised in tests via an explicit "session slept/woke"
signal and in the demo via a chord. Session auto-sleep policy is not built here.

## 7. Fleet memory-pressure policy (Sleep-only, graduated)

**Forced simplification:** the vendored `libghostty-vt` exposes **no runtime byte-trim** and
`max_scrollback` is open-time-only (Slice 3). So the spec's "warning → trim histories, critical →
disconnect tabs" collapses to **one runtime lever: Sleep** (full engine + scrollback teardown,
final viewport retained, explicit reattach — Slice-4 `Sleep`). Warning and critical differ only
in aggression:

- **`MemoryPressure::Warning { free_fraction }`** (Q4) — Sleep **eligible** terminals in LRV order
  (coldest `last_viewed` first) until `free_fraction` of the eligible-set estimate-sum is freed.
  The fraction is an **explicit input** snapshotted at call time, *not* a self-referential
  "budget derived from the fleet estimate" (that was circular / arbitrary). Selection **and** stop
  live in ordinal estimate-space, which the ordinal estimate supports.
- **`MemoryPressure::Critical`** — Sleep **all** eligible terminals.
- **Eligibility = `hidden && Live` (Q10).** Non-hidden terminals are never policy-slept; transient
  lifecycles (`Starting`, `ReplacementWaiting`, `Reconnecting`) are exempt so pressure never
  destroys a mid-supersede **retained engine** (§10). If only non-hidden terminals remain and
  pressure persists, Slice 5 does nothing further (never silently drops PTY bytes; keeping visible
  tabs connected is the last-resort rule; true byte-trim stays the parked FFI conditional).

**Why not RSS (Q4):** RSS is a *process-global* scalar — it cannot attribute memory to a specific
terminal, which is exactly what selection ("which one?") and the stop condition ("freed enough?")
need; and freed heap doesn't promptly return to the OS, so an RSS control loop lags and
over-evicts. RSS is the right tool only for the *trigger* ("are we under pressure?"), which is the
Slice-6 seam below. True per-terminal *bytes* would come from the parked libghostty byte-accounting
FFI, never from RSS.

**Signal source is a seam.** No `macOS didReceiveMemoryWarning` / `DISPATCH_SOURCE_TYPE_MEMORYPRESSURE`
hook exists in the tree. Slice 5 adds `FleetStore::on_memory_pressure(Warning{free_fraction} |
Critical)` driven by tests + a demo chord (fraction supplied directly → deterministic); wiring it
to a real OS memory-warning source — which decides what a real warning maps to — is Slice 6 / lens-app.

**LRV ordering** uses the per-terminal ordinal **`retained_bytes_estimate`** (via the lightweight
accessor, §5) to pick which eligible terminals to sleep and to measure "freed enough". Ordinal
reliability (Slice-3 Job B) is all LRV needs.

## 8. Terminal idle auto-sleep

A `hidden && Live` terminal idle for a **threshold** (provisional ~10 m; STATUS open decision)
auto-sleeps. Driven off `FleetStore`'s `clock`: a periodic idle **tick** (cadence ~30 s — the
threshold is minutes-scale, so no finer granularity is needed; distinct from the threshold)
compares `clock.now − last_viewed` against the threshold for each **eligible** (`hidden && Live`,
Q10) member and Sleeps those over it. Deterministic under `ManualUiClock` (advance clock + fire
tick). Non-hidden terminals never idle-sleep (their `last_viewed` is refreshed on show/focus).
Independent of the session's own idle/auto-sleep (deferred).

## 9. Resource-signal forwarding + `4404`-first reconciliation (sub-slices A + D)

**Forwarding (D):** on `ActorOutcome::TerminalResource(_)` for a session, `FleetStore` forwards it
as `TerminalHostEvent::ResourceCreated/Deleted` to **every owned terminal in that session** (the
tab filters to its own identity — Slice-4 contract).

**`4404`-first needs a lens-terminal LIFECYCLE change (Q3) — forwarding alone is insufficient.**
Grilling verified the Slice-4 framing was wrong: once a `4404` lands *first*, the tab is already
`Detached` (`apply_bridge_event` → `on_detach(TerminalGone)`, engine torn down), and in `Detached`
`on_resource_signal` is a `_ => None` **no-op** — forwarding `resource.deleted`/`resource.created`
afterward does nothing (`saw_delete` never set, `adopt_successor` never called; the only exit from
`Detached` is `on_reattach`, gated to `ClientDetached`/4405). And `FleetStore` is **not** in the
bridge path — the tab self-detaches autonomously — so the host cannot reorder the race.

Fix (sub-slice A): for an `OpenOrCreate` target, a `4404`/`TerminalNotFound` bridge close enters
**`ReplacementWaiting`** (the terminal is defined by its key and *will* be recreated), **not** hard
`Detached`. `Existing` targets (key `None`, non-recreatable) keep hard-detach. Then **both** race
orders — `4404`-first and `resource.deleted`-first — converge on `ReplacementWaiting`, and the
subsequent matching `resource.created` adopts via the existing `AdoptSuccessor` path. The
replacement timeout still bounds the wait. This is a deliberate deviation from Slice 4's frozen
state machine and gets **A's whole-branch review**.

The **real ordering proof** is an opt-in live rider (P7/P8 class, needs live omnigent, sub-slice D);
the deterministic forwarding + the tab lifecycle transition are unit-tested (both orders → adoption
fires). The standalone demo cannot synthesize the co-emitted `4404`, so it does not reproduce the
race — same constraint documented in Slice 4.

## 10. `session.superseded` redirect (sub-slices A + D)

**Ground truth (verified against omnigent 2026-07-22 — memory
`terminal-supersede-vs-agentswitch-semantics`):** `/clear` rotates conversation **A** to a
**brand-new** conversation **B**; the server **`transfer`s the terminal live** (same
`terminal_id`, tmux pane keeps running, only the owning session changes). It emits, in order,
`session.resource.deleted` on **A**, `session.resource.created` on **B** (same id), then
`session.superseded { target: B, reason: "clear" }` on **A** — all persisted. A stays alive but
loses the terminal + runner binding. omnigent's own web-app **auto-follows** (aborts A, binds B);
it does *not* keep streaming A. So a client **must load B** to keep the (live, transferred)
terminal — it cannot stay on A, and it cannot re-parent into a session that isn't loaded.

**FleetStore handling (D), on `ActorOutcome::Superseded { target_conversation_id: B, .. }`:**

1. **Load B headlessly** — adopt B as a live `FleetStore` session (poller + membership + card) so
   B's control outcomes route and cascade reaches the terminal. (The *view* auto-follow — focus
   moving to B — stays Slice 6; Slice 5 only makes B reachable.) Verify in planning that the
   existing session-open path accepts "load B now" cleanly; if session-loading is UI-entangled,
   the terminal follow slips to Slice 6.
2. **Move the member** `terminals[A][key] → terminals[B][key]`. `session_key` is **not** rekeyed —
   a retained `TerminalId` implies a retained `(terminal_name, session_key)`, so the inner
   `TerminalKey` is unchanged and cannot collide with B's own terminals (session-scoped keys).
3. **Drive a retain-engine `Transfer`** (Q7/Q8): send `TerminalHostEvent::Transfer { new_session:
   B }` to the moved tab, which re-attaches under B keeping the **frozen engine** — so scrollback
   survives `/clear` (the user can scroll back past the clear).

**Retain-engine is a deliberate lens-terminal change (A), and it's de-risked.** A's delete drives
the tab to `ReplacementWaiting`; **`enter_replacement_waiting` uses transport-only teardown** (keep
the engine frozen) *instead of* today's `teardown_runtime_full`, because the delete precedes the
`superseded` event — the engine must survive until `Transfer` arrives. `Transfer` reuses the
existing engine-retaining reconnect machinery (`teardown_transport_off_foreground` +
`on_reconnect_success`, retargeting `current_session = B`). **No double-feed:** the attach contract
is a current-screen clear+redraw with *no byte-replay* (`docs/spikes/2026-07-15-pty-attach-contract.md`),
and `engine/reconnect_seed.rs` tests already prove a retained engine + re-attach does not duplicate
scrollback.

**Retain vs. fresh discriminator = `session_id`-changed (Q7):** the tab retains the frozen engine
through `ReplacementWaiting` unconditionally, then at adopt time — **`session_id` changed
(cross-session) → reuse engine (supersede)**; **same `session_id` → fresh engine (in-session
agent-switch)**. Agent switch `kill-server`s the pane server-side (omnigent), so its scrollback
would describe a *dead* program — fresh is correct; and its successor create arrives next-turn,
past the 30 s `REPLACEMENT_WAIT`, so it naturally times out → fresh. The two cases can't be
confused. Same-session agent-switch keeps today's Slice-4 fresh-engine behavior.

The **real proof** is an opt-in live rider (supersede scrollback preserved, sub-slice D). The demo
can exercise `Transfer` via a direct host-event chord but cannot synthesize the real
delete-then-superseded ordering.

## 11. Scrollback-cap correctness fix

Latent production bug in `lens-terminal`: `open()` → `policy.rs:250` defaults `max_scrollback` to
`1000`, but Slice 3 proved `max_scrollback` is a **BYTE budget, not lines** (vendored doc wrong;
memory `terminal-max-scrollback-bytes-and-worker-stack`). So the default is ~1000 bytes ≈ **~7
rows** at 80 cols, and the public field `TerminalOpenOptions.scrollback_lines` feeds a "lines"
number straight into a bytes parameter.

Fix (sub-slice A — same crate; a **latent-bug fix**, *not* budget-defining. Q4 made the Warning
budget ordinal fraction-freed, so there is no absolute byte ceiling for it to define; the 10 MB is
a per-terminal **cap**, not a preallocation, and the aggregate is managed by policy):

- Rename public `TerminalOpenOptions::scrollback_lines` → `scrollback_bytes`
  (`#[non_exhaustive]` + `with_scrollback_bytes` setter; lens-ui is the only consumer, on-branch).
- Default to `10_000_000` (Ghostty's decimal app default — spec line 415).
- Fix the doc comment on `TerminalOptions.max_scrollback` / the field to say **bytes**.

## 12. Seams — real vs. test/demo-driven

| Concern | Slice 5 | Driven by |
| --- | --- | --- |
| Terminal membership / LRV / cascade fan-out | **built real** (C) | tests + demo |
| **Terminal visibility (`hidden`/`last_viewed`)** | **seam `set_terminal_visible` (C)** — **Slice 6 wires** | test/demo chord (Q1) |
| Pressure LRV eviction (Sleep-only, `hidden && Live`) | **built real** (C) | `on_memory_pressure(Warning{free_fraction}\|Critical)` seam (test/demo chord) |
| Idle auto-sleep (~30 s tick, `hidden && Live`) | **built real** (C) | `ManualUiClock` tick (test) / demo |
| `4404`-first: `OpenOrCreate` → `ReplacementWaiting` **lifecycle change** | **built real** (A) | unit + demo host events; live rider (real race) |
| Resource forwarding + `4404`-first driving | **built real** (D) | injected control outcomes (test) + live rider |
| Supersede: load B + move member + retain-engine `Transfer` | **built real** (A+D) | injected `Superseded` (test) + live rider (scrollback) |
| Session-sleep **trigger** | seam (existing no-op class) | explicit "session slept" signal (test/demo) |
| OS memory-warning **source** | seam | Slice 6 / lens-app |
| **View auto-follow to B on supersede** (focus moves) | **not built** | Slice 6 |
| On-screen terminal tile | **not built** | Slice 6 |

## 13. Testing strategy

- **A · lens-terminal (unit + demo):** `OpenOrCreate` `4404` → `ReplacementWaiting` (both race
  orders → `AdoptSuccessor` fires); `enter_replacement_waiting` retains the engine (transport-only
  teardown — update the existing `runtime.is_none()` assertion); `Transfer` re-attaches under a new
  `session_id` reusing the frozen engine (retain iff `session_id` changed, fresh if same); the
  `reconnect_seed` no-double-feed invariants extend to the `Transfer` path; scrollback-cap
  bytes/default.
- **B · lens-core / lens-client:** reducer tests for `StreamUpdate::Superseded` +
  `StreamUpdate::TerminalResource*`; actor tests that the mapped `ActorOutcome` variants emit;
  parse tests for the `resource.created` payload + `resource.deleted` `session_id`.
- **C · lens-ui (`FleetStore`, headless, `ManualUiClock` + `open_with_engine_for_test`):**
  membership register/close; visible-on-open; `set_terminal_visible` stamping; LRV ordering;
  cascade Sleep-all / **Wake-non-hidden**; `pending_sleep` deferred sleep (`Starting` → Live);
  pressure Warning fraction-freed / Critical all (**`hidden && Live` eligibility**, non-hidden +
  transient exempt); idle threshold + ~30 s tick.
- **D · lens-ui (cross-layer):** resource forwarding to the right owned terminals; `4404`-first
  driving (both orders → adoption fires); supersede load-B + member move + `Transfer` (engine
  retained, subsequent signals reach B, `session_key` unchanged).
- **Live riders (opt-in, live omnigent, sub-slice D):** supersede scrollback-survives-`/clear` +
  `4404`-first real ordering (P7/P8 class). Not in the headless gate.
- **Reviews:** whole-branch on **A** (S4 `ReplacementWaiting`/adopt/bridge seam) and **D**
  (actor-outcome + terminal-event cross-seam), per the "new handler sharing state" discipline.
- **Gate:** `cargo run -p xtask -- gate` (workspace clippy `-D warnings` + fmt + tests). No new
  real-window harness needed (no rendering in Slice 5).

## 14. Risks & coordination

- **T-0 merge collision (sub-slice B): RESOLVED.** T-0/transcript is merged into `main` (this branch
  is rebased on it), and its landing left the `folds.rs` `Superseded` arm untouched — B edits that
  block on top of the landed version, no coordination, no conflict.
- **Two deliberate `lens-terminal` lifecycle changes (A) on the S4-frozen seam.** `OpenOrCreate`
  `4404` → `ReplacementWaiting` (§9) and the retain-engine transport-only teardown + `Transfer`
  (§10) both touch the `ReplacementWaiting`/adopt/bridge state machine — the exact seam where S4's
  whole-branch review caught the bridge-clobber Critical. Isolated in A, landed + whole-branch
  reviewed + live-ridden **before** C/D build on it.
- **Actor control routing (D):** the fleet-level store hook already exists (`fold_session_feed` /
  `apply_transport`, poller holds `WeakEntity<FleetStore>`). The new control arm + `on_session_control`
  must not perturb the card-bound outcome path or focused-replica routing (existing coalescing/decay
  + reconcile-epoch tests stay green).
- **Headless load-of-B (D):** supersede requires FleetStore to adopt a brand-new session B mid-stream
  (poller + membership + card). Verify in D planning that the existing session-open path accepts this
  cleanly; if it's UI-entangled, the terminal follow slips to Slice 6.
- **Live-contract residuals (verify in the rider, don't invent):** the exact `4404`↔`resource.deleted`
  interleaving on a real reset (A converges both orders on `ReplacementWaiting`, so either is safe);
  the retain-engine `Transfer` viewport/scrollback behavior against a real `/clear`. `session_key`
  rekey is **resolved** (retained `TerminalId` ⇒ retained key), no longer an unknown.

## 15. Completion-matrix mapping (anti-drop)

| Design spec matrix row | Closed by |
| --- | --- |
| `session.superseded` redirect (retained engine follows `TerminalId`) | §10 (A retain-engine `Transfer` + D load-B/move) |
| Fleet memory-pressure trim/disconnect (LRV) | §7 (Sleep-only per FFI gap; Warning=fraction-freed, `hidden && Live`) |
| lens-ui integration: terminal as `FleetStore` member (minimal) | §5 (C membership; visibility seam + surface → Slice 6) |
| Slice-4 deferral: `4404`-first adoption ordering | §9 (A lifecycle change + D driving) |
| (new) scrollback cap byte-correctness | §11 (A) |
