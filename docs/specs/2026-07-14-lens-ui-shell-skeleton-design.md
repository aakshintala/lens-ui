# lens-ui shell skeleton — design

**Status:** Approved 2026-07-14 (brainstorming); **revised 2026-07-14 after
cross-family review** (codex/gpt + grok-4.5-xhigh) surfaced that the board must be
driven by the D10 dual-mode `SummaryUpdate` feed, not gated `StreamUpdate`. This
revision folds in the confirmed findings. First rendering consumer of the state
model.
**Depends on:** `lens-core` (actor/`FleetScheduler`/`SummaryUpdate`/`StreamUpdate`/
`SessionCommand`/`ActorOutcome`, through P3-3b, live-verified vs omnigent 0.5.1);
`lens-client` (built REST surface incl. `put_read_state` + `viewer_*` read
fields); framework lock (gpui 0.2.2); application shell/layout; state model §9
(D10 dual-mode) / §10 (list poll) / §13.2 (seams).
**Feeds:** the parallel surface workstreams (transcript, terminal, workspace,
permissions) — they plug into the slot API + `SessionAttach` this skeleton
publishes.

---

## 1. Purpose & the risk this retires

`lens-ui` is the **first rendering consumer** of the state model. This skeleton
retires the one risk that cannot be parallelized: the **D10 dual-mode
async-stream → state → render bridge** at fleet scale —

- **N background-warm cards** rendered from the coarse **`SummaryUpdate`** feed
  (Summary mode), updating **independently without cross-invalidation**, given
  gpui has **no per-field subscription** (framework §4.2);
- the **promote↔demote** mode-switch on focus/blur (background `Summary` ↔
  focused `Detailed`), which the review identified as the missed load-bearing
  mechanism.

The deliverable is the **wiring contract + slot API**, *proven* by the board
session-cards surface driven to a **complete** state from an **enriched**
`SummaryUpdate`. Then transcript / terminal / workspace fan out in parallel.

**This spans two crates:** a **lens-core** phase (§3) makes the feed complete and
mode-switchable; a **lens-ui** phase (§4–§9) builds the board on it.

---

## 2. Crate layout

- **`crates/lens-ui`** (lib) — views, view-models, `FleetStore` (owns the
  `FleetScheduler` + the promote/demote policy), the per-session poller, the slot
  API, `ContentTab`, the synthetic feed. No `main`. Unit-testable.
- **`crates/lens-app`** (bin) — window bootstrap, gpui `Application`, theme,
  chooses the feed source (synthetic **or** real `FleetScheduler`), `main`.
- **`crates/lens-core`** (edited, §3) — scheduler dual-mode plumbing, enriched
  `SummaryUpdate`, `has_unseen_result`.

`lens-ui` sees only the channel types — `Receiver<SummaryUpdate>`,
`Receiver<StreamUpdate>`, `Sender<SessionCommand>`, `Receiver<ActorOutcome>` — so
synthetic and live are drop-in. Channels carry **no session id**, so demux is
**one channel-set per actor** (per-session poller); a shared bus would be wrong.

---

## 3. lens-core phase — make the feed complete & mode-switchable

Three engine changes (each gets cross-family review; they touch the actor):

### 3.1 Scheduler dual-mode plumbing (the real gap)

`FleetScheduler::wake`/`reconnect` currently call `spawn_actor` (hardcoded
`OutputMode::Detailed`) and plumb **only** `updates: Sender<StreamUpdate>` — the
scheduler **cannot emit `SummaryUpdate` today**. Change: `wake`/`reconnect` also
accept a `summaries: Sender<SummaryUpdate>` and an initial `OutputMode` (or a
`focused: bool`), spawning background sessions in `Summary`. The dual-mode
runloop (`Promote`→`Rebased`+`Detailed`, `Demote`→`Summary`) already exists
(`runloop.rs:523-535, 683-706`); this exposes it through the scheduler.

### 3.2 Enrich `SummaryUpdate` (Change 1)

`SummaryUpdate::from_state` (`actor/summary.rs`) copies 6 fields today. Extend the
struct + function to also carry the §5.1 card chrome — **all already on
`SessionState`, present even when demoted** (demote drops only `items`):
`llm_model`/`model_override`, `cumulative_cost`, `context_window` (+ existing
`last_total_tokens` → ctx %), `sandbox_status`, `git_branch`/`workspace`,
`reasoning_effort`, and an **activity summary** (derived: `todos.activeForm` ▸
in-flight tool ▸ blank, shell §5.2). This keeps the coarse ms–s cadence (no
per-token deltas) — D10's scale property holds.

### 3.3 `has_unseen_result` for the Ready wave (Change 2A)

"Ready = idle **with an unacknowledged completed turn**." Completion is
stream-derivable; "unacknowledged" is viewer-relative and resolved **locally**:

- New `SessionState.has_unseen_result: bool` (Lens-local, persisted). The actor
  **sets** it on a turn completion (`response.completed` / status → idle after
  activity) **while not focused**; **clears** it on `Promote` (= focus).
- `SummaryUpdate` carries the flag; the card wave computes `Ready = idle &&
  has_unseen_result`.
- **Fully stream-driven — no poll, no server round-trip, no lag.**

**Forward-compat (lens-ui side, cheap):** on focus, also call the *already-built*
`Sessions::put_read_state(id, now, false)` so the omnigent web UI / a second Lens
instance converge. Reading *other* devices' acks (`viewer_unread`/
`viewer_last_seen` off the fleet poll) is **deferred to board-v2** — it is the
only poll-bound part and never makes the local board laggy.

---

## 4. State-binding contract (lens-ui) — the load-bearing part

### 4.1 Who folds what (three layers)

- **Event → state: lens-core reducer, off-thread.** Already turns SSE into
  `SummaryUpdate` (Summary) or fine-grained `StreamUpdate` (Detailed). The UI
  never sees raw events.
- **Feed → foreground field: the per-session poller in `FleetStore`.** One
  `cx.spawn` task per session, a `select` over `{summaries, updates, outcomes}`
  that **drains each channel exactly once**, **coalesces a ready burst**, applies
  the projection, then does **gated** notifies (lens-store §85-105 is the
  batching precedent). This is the single foreground dispatch site.

Slogan: **reduce-once in lens-core → dispatch-once in the poller → project.**

### 4.2 Foreground objects & the dual-mode fold

Per warm session, one **`SessionCard`** gpui `Entity` (always resident). The
poller patches it from **whichever feed is live for its mode**:

- **background / `Summary`:** apply `SummaryUpdate` (copy-assign of scalars — the
  enriched §3.2 fields).
- **focused / `Detailed`:** `Promote` emits a `Rebased(scalars_baseline)` reseed,
  then `StreamUpdate` **scalar** deltas patch the same card fields
  (`StatusChanged`, `UsageChanged`, `ModelChanged`, …). Transcript deltas
  (`TranscriptAdvanced` watermark, streaming-tail) route to the **full replica**
  — **deferred with the transcript** (focused slot is empty in the skeleton).

So the card renders identically in both modes; the skeleton **proves the
mode-switch** (background Summary → focus Promote/Detailed → blur Demote),
including the `has_unseen_result` clear on Promote.

**Routing corrections (from review, vs the actual enum):** `SnapshotRestored`
carries only `Vec<PendingInput>` and does **not** seed card scalars (`Rebased`
does); `Rebased` clears only `items` (it is *not* "scalars only" — it still
carries collections/scratch/lifecycle, so consume only the card-relevant
subset); `ResourcesChanged` is a **valueless marker**, so repo/branch stay live
only via `SummaryUpdate`/`Rebased` snapshots, not an incremental value-carrying
delta.

### 4.3 `FleetStore` & ownership

`FleetStore` is a gpui `Entity` that **owns the `FleetScheduler`** (not loose
`ActorHandle`s — the scheduler owns those privately and `ActorHandle` is not
`Clone`; cloning its `async_channel::Receiver` would create *competing*
consumers, not a broadcast). It also owns:

- the map `(ConnectionId, SessionId) → SessionCard` **at the UI layer** (each a
  **separate** entity),
- the board's ordinal slot layout (shell §4.1),
- **the promote/demote policy** (§9 registry responsibility): the focused session
  is Promoted; all others Demoted. On subscribe, a background session is woken
  and Demoted (or woken directly in `Summary` once §3.1 lands). The poller is the
  **sole** consumer of each session's outcome channel.

**Multi-connection caveat:** `FleetScheduler` keys its registry by `SessionId`
**only**, not `(ConnectionId, SessionId)` (`scheduler.rs:17`). The UI map is
composite-keyed, but true multi-server is **precluded below `FleetStore`** until
the engine registry is re-keyed. Skeleton = **one connection**.

### 4.4 The gpui isolation invariant (the actual mechanism)

Per-session entities are **necessary but not sufficient**: notifying an entity
dirties it **and every ancestor**, and ordinary `Entity<V>` is not paint-cached
(review Critical #1, `gpui/window.rs:1304-1317`). No-cross-invalidation therefore
requires **pinning the observe topology**, not gating notifies:

1. **Each `SessionCard` view observes its own card entity** (`cx.observe`), not a
   shared store.
2. **`FleetStore` is notified ONLY on membership/layout changes** — never on a
   card's scalar fold. A card update must not touch the store entity, or the
   whole board re-renders.
3. **Cards are mounted as stable, cached views** (`AnyView` / retained element)
   so an unchanged sibling's paint is **reused**, not recomputed.

The root/board *will* still invalidate on a membership change; the guarantee is
that **unchanged sibling cards do no render/paint work**. (The §3.6 "notify-gating
on unchanged ScratchChanged" from the prior draft is **dropped** — Summary mode
is already coarse-cadence, so there is no per-token thrash to gate.)

### 4.5 Commands down + `ActorOutcome`

- **Down:** card kebab / focus events → `FleetStore` → the scheduler's handle →
  `SessionCommand` (`Sleep`, `Send`, `Promote`, `Demote`, `Stop`). **`Interrupt`
  is NOT wired** — `SessionCommand::Stop` exits the *Lens actor loop*; it does not
  send the server an interrupt, and there is no `Interrupt` variant. A real
  interrupt is a **new lens-core command path**, out of skeleton scope.
- **`ActorOutcome`** (drained by the same poller): `Parked` → card connection
  state (shell §5.4); `Slept`/`SleepDeclined` matter once Sleep is wired;
  `SendLost`/`TransportChanged`/`PersistError` logged in the skeleton.

---

## 5. Slot API & window recompose

- **Window** = `nav rail │ main area`, recomposing **board state** ↔ **focused
  state** (shell §3).
- **Board state:** nav rail + the ordinal reflow grid of `SessionCard` views.
- **Focused state:** `nav rail │ boards(shrunk) │ chat │ navigator │
  working-area`, with **chat/navigator/working-area as real but empty labeled
  slot containers** the surface authors target.

### 5.1 Navigation model (no global ESC)

Native harness TUIs run inside a terminal surface and the TUI-native toggle
design forwards raw input to the harness — **ESC must reach the harness**, so
there is **no global ESC→board binding**. Navigation is a card-click toggle:

- Click a card → focus that session (recompose; `FleetStore` **Promotes** it and
  Demotes the previously-focused one).
- In focused state (boards shrunk, visible): click a **different** card → switch
  focus (promote new / demote old); click the **currently-focused** card → toggle
  back to board state (Demote it).
- `⌘\` collapses/expands the boards column. `⌘D` deep-focus deferred.

ESC stays **surface-local**.

### 5.2 `ContentTab` + `SessionAttach` (the terminal seam)

The working-area slot is a **single-tile, single-content mount** hosting one
`ContentTab`. **Dispatch is decided now, not deferred** (review: an
`impl IntoElement` trait is not object-safe and cannot be compiled against): the
mount holds an **`AnyView`** (an `Entity<T: Render + ContentTab>` erased to
`AnyView`); `ContentTab` is a thin object-safe capability trait:

```rust
trait ContentTab {              // object-safe; dispatch = AnyView mount
    fn title(&self) -> SharedString;
    // focus/blur arrive through gpui's FocusHandle on the view itself,
    // not through this trait — keeps it object-safe.
}
```

**`SessionAttach`** (what the terminal workstream codes against) — corrected: it
carries **identity + a WS-attach capability**, *not* a `terminal_notifs`
receiver. `TerminalNotif` does not exist; `session.terminal.activity` folds to
`StreamUpdate::TerminalPendingChanged(bool)` on the **normal feed** (the reducer
emits no value-carrying activity), so terminal-pending rides the card feed, not a
side channel:

```rust
struct SessionAttach {
    connection_id: ConnectionId,
    session_id: SessionId,
    // WS-attach capability: identity + a factory to open the byte stream + resize.
    // The typed WS terminal client is UNBUILT in lens-client (a genuine dependency
    // of the terminal workstream, not provided here).
    attach: TerminalAttachCapability,
}
```

**Deferred to workspace fan-out:** splits, tab-bar, launchers, +badge, preview
tabs, content persistence.

---

## 6. The board-cards proving surface

The card renders shell §5.1 chrome from the **enriched `SummaryUpdate`** — coarse
summary, never a transcript: status icon tile + **wave**, `<STATUS>`/`<Title>`,
`<harness> · <model>`, **activity line**, `📁 repo ⑂ branch`, footer (host pill ·
`~$spend` cumulative, `—` when `None` · `ctx %` bar), connection-state takeover
(§5.4).

**Wave ladder** (shell §5.1) — now fully derivable from the enriched feed:
Needs-input (`needs_attention`), **Ready** (`idle && has_unseen_result`, §3.3),
Working (`running/launching/waiting`), Failed (`status`/`last_task_error`), Slept
(lifecycle). A couple of kebab commands wired (Sleep→`Sleep`, Send→`Send`) — **not
Interrupt**.

### 6.1 Acceptance test — what the skeleton exists to prove

Notify counts alone prove only poller gating, **not gpui render isolation**
(review). The test mounts a **real board + N real card views** in gpui's
`TestAppContext` (headless, no display server — `gpui/app/test_context.rs`):

1. settle the initial frame; instrument per-card `Render`/paint counters + the
   board/root counter;
2. inject an enriched `SummaryUpdate` on session B; drive the executor/frame;
3. assert **B's card re-renders, A's card does no render/paint work** (root may
   invalidate — the guarantee is unchanged-sibling reuse, §4.4);
4. **mode-switch:** focus B → assert Promote (`Rebased`+`Detailed`), card stays
   correct, `has_unseen_result` clears; blur → Demote (back to `SummaryUpdate`).

---

## 7. Dev feed & verification

- **Synthetic `FakeFleet`** (`lens-ui` test-support) — **per-session** channel
  sets emitting scripted `SummaryUpdate` (background) / `StreamUpdate` (on
  promote) and accepting `SessionCommand`s. Powers hermetic tests + the §6.1
  acceptance test; stages N independent sessions deterministically.
- **Live-verify gate** — `lens-app` on the **real `FleetScheduler` + omnigent
  0.5.1** (the `lens-drive` path, extended to N sessions + Summary mode). Note:
  `lens-drive` is single-session Detailed — a **weak** analogy for the N-card +
  Summary risk, so live-verify must exercise ≥2 background-Summary cards + a
  promote/demote cycle, not just reuse the drive harness shape.

---

## 8. Scope boundary (explicit)

**In:** the §3 lens-core phase (scheduler dual-mode plumbing, `SummaryUpdate`
enrichment, `has_unseen_result` + `put_read_state`-on-focus); `lens-app`/`lens-ui`
split; `FleetStore` (owns scheduler + promote/demote policy) + per-session poller
+ the §4.4 isolation invariant; board state + enriched card chrome + full wave
ladder; focused-state empty slots + click-toggle recompose (with promote/demote);
`ContentTab`(AnyView) + `SessionAttach`(attach capability) + placeholder tab;
minimal theme tokens; `FakeFleet` + live-verify; the §6.1 acceptance test.

**Out (owned by later slices):**

- transcript rendering & markdown; the **full replica / disk `RowSource`** (D23) —
  *transcript fan-out*;
- workspace / diff / editor; splits / launchers / preview / persistence —
  *workspace fan-out*;
- terminal internals + the **unbuilt typed WS terminal client** — *the parallel
  terminal workstream* (plugs into `ContentTab`/`SessionAttach`);
- a real server **Interrupt** command path (new lens-core command);
- permissions/elicitation forms; Bridge inbox; search; Canvas; Concierge;
  multi-board / groups / archive;
- the **REST-poll coarse-status path** for Slept/archived/non-warm cards
  (state-model §10). The lens-client fleet poll (`Sessions::list`, `GET
  /v1/sessions`) **is already built**; `SummaryUpdate` is explicitly a two-producer
  projection ("actor here; §10 poll later", `summary.rs`). The missing work is
  **lens-core scheduling/coarse-projection integration**, not a lens-client gap.
  Owned by the **board-v2 continuation of this skeleton**; skeleton board =
  warm/active only;
- **inbound** cross-device read-state reconciliation (`viewer_*` off the poll) —
  board-v2; and **multi-connection** (needs the `FleetScheduler` registry re-keyed
  to `(ConnectionId, SessionId)`).

---

## 9. Testing strategy

- **Hermetic `lens-ui` tests** over `FakeFleet`: the §6.1 acceptance assertions
  (independent cards, single-card repaint, mode-switch), card chrome per
  `SummaryUpdate`/`StreamUpdate` variant, wave ladder, command-down path.
- **lens-core tests** for §3: `SummaryUpdate` enrichment (`from_state` populates
  the new fields incl. when demoted), `has_unseen_result` set/clear
  (completion-while-unfocused, cleared on Promote), scheduler wake-in-Summary
  emits `SummaryUpdate`.
- **Live-verify** (§7) as the acceptance gate.
- Gate: `cargo clippy --workspace --all-targets -- -D warnings` + `fmt` clean,
  tests green.

---

## 10. Open / deferred (tracked, not blocking)

- `⌘D` deep-focus, `⌘\` polish — fold in with the focused surfaces.
- Multi-server / connection badge — needs the engine registry re-key.
- Send-recovery / `SendLost` UX — with the composer.
- **Board-v2** — the REST-poll path, Slept/archived/groups/multi-board, inbound
  cross-device read-state. The named continuation that owns the §8 poll deferral.

---

## Appendix A — cross-family review disposition (2026-07-14)

Reviews: codex/gpt + grok-4.5-xhigh, both read the spec against lens-core source.
Findings accepted after independent verification:

- **Board on wrong feed (D10 dual-mode)** — accepted; §3/§4 rebuilt on
  `SummaryUpdate` + promote/demote. *(grok Critical #1.)*
- **gpui isolation not automatic** — accepted; §4.4 pins observe topology + cached
  views. *(both Critical.)*
- **Terminal seam invented `TerminalNotif`** — accepted; §5.2 → attach capability,
  `TerminalPendingChanged` on the normal feed, WS client noted unbuilt. *(both.)*
- **`Interrupt`→`Stop` wrong** — accepted; unwired, §4.5/§8. *(codex Critical.)*
- **Routing false claims** (`SnapshotRestored`/`Rebased`/`ResourcesChanged`) —
  accepted; §4.2. *(both.)*
- **Ownership/keying** (`FleetStore` owns scheduler; `SessionId`-only key) —
  accepted; §4.3. *(both.)*
- **Poller needs select+batch** — accepted; §4.1. *(codex.)*
- **Acceptance test underspecified** — accepted; §6.1 mounts real views. *(both.)*
- **Ready needs read-state** — accepted; §3.3 stream-derived local + forward-compat
  `put_read_state`. *(codex.)*
- **Actor fundamentals correct** (live reduce→emit, `TranscriptAdvanced`
  watermark, channel types, `ActorOutcome` set) — confirmed; preserved. *(both.)*
