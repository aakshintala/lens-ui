# lens-ui shell skeleton — design

**Status:** Approved 2026-07-14 (brainstorming). **Two cross-family review rounds
folded in** (codex/gpt + grok-4.5-xhigh, both vs lens-core source): round 1
corrected the feed (D10 dual-mode `SummaryUpdate`, not gated `StreamUpdate`);
round 2 (verify-the-fixes) drove the **unified `ActorFeed` channel** decision and
the implementation-precision fixes below. First rendering consumer of the state
model.
**Depends on:** `lens-core` (actor/`FleetScheduler`/`SummaryUpdate`/`StreamUpdate`/
`SessionCommand`/`ActorOutcome`, through P3-3b, live-verified vs omnigent 0.5.1);
`lens-client` (REST surface incl. `put_read_state` + `viewer_*` read fields);
framework lock (gpui 0.2.2); application shell/layout; state model §9 (D10
dual-mode) / §10 (list poll) / §13.2 (seams).
**Feeds:** the parallel surface workstreams (transcript, terminal, workspace,
permissions) — they plug into the slot API + `SessionAttach` this skeleton
publishes.

---

## 1. Purpose & the risk this retires

`lens-ui` is the **first rendering consumer** of the state model. This skeleton
retires the risk that cannot be parallelized: the **D10 dual-mode
async-stream → state → render bridge** at fleet scale —

- **N background-warm cards** rendered from the coarse **`SummaryUpdate`** feed
  (Summary mode), updating **independently without cross-invalidation** (gpui has
  no per-field subscription, framework §4.2);
- the **promote↔demote** mode-switch on focus/blur (background `Summary` ↔
  focused `Detailed`), proven to be **order-correct through the transition**.

Deliverable: the **wiring contract + slot API**, proven by the board
session-cards surface driven to a **complete** state from an **enriched**
`SummaryUpdate`. Then transcript / terminal / workspace fan out in parallel.

**This spans two crates:** a **lens-core** phase (§3) makes the feed unified,
complete, and mode-switchable; a **lens-ui** phase (§4–§7) builds the board on it.

---

## 2. Crate layout

- **`crates/lens-ui`** (lib) — views, view-models, `FleetStore` (owns the
  `FleetScheduler` + the promote/demote policy), the per-session poller, the slot
  API, `ContentTab`, the synthetic feed. No `main`. Unit-testable.
- **`crates/lens-app`** (bin) — window bootstrap, gpui `Application`, theme,
  chooses the feed source (synthetic **or** real `FleetScheduler`), `main`.
- **`crates/lens-core`** (edited, §3) — unified `ActorFeed` channel, scheduler
  dual-mode plumbing, enriched `SummaryUpdate`.

`lens-ui` sees only the channel types — `Receiver<ActorFeed>`,
`Sender<SessionCommand>`, `Receiver<ActorOutcome>` — so synthetic and live are
drop-in. Channels carry **no session id**, so demux is **one channel-set per
actor** (per-session poller); a shared bus would be wrong.

---

## 3. lens-core phase — unify, complete, and mode-switch the feed

Four engine changes (each gets cross-family review — they touch the actor):

### 3.1 Unified `ActorFeed` channel (the keystone)

Today the actor holds **two** senders and emits on exactly one per its
`OutputMode` (verified: `runloop.rs:682-710, 1064-1076` are both
`match mode { Detailed => updates.send, Summary => summaries.send }` — never
both). Across a **transition**, the two independently-buffered channels can be
**reordered by a lagging consumer** (a queued pre-Promote `SummaryUpdate` applied
*after* the `Rebased`), regressing the card. Fix: **merge to one FIFO channel**:

```rust
enum ActorFeed { Summary(SummaryUpdate), Detailed(StreamUpdate) }
// actor: feed: async_channel::Sender<ActorFeed>   (replaces updates + summaries)
```

A single channel preserves the actor's send-order **by construction** — the race
is gone with no epoch/barrier/discard logic. **Blast radius (do it now, while
small):** `spawn_actor`/`spawn_actor_dual`, `FleetScheduler::wake`/`reconnect`,
`lens-drive`, and the actor tests that `recv()` the old channels (mechanical
unwrap churn). Doing this before the transcript slice accretes a second consumer
is strictly cheaper than after.

### 3.2 Scheduler dual-mode plumbing + spawn-in-Summary

`FleetScheduler::wake`/`reconnect` currently call `spawn_actor` (hardcoded
`OutputMode::Detailed`) and plumb only the `StreamUpdate` sender — the scheduler
**cannot emit Summary today** (`scheduler.rs:43-106`, `runloop.rs:116-136`).
Change: they accept the unified `feed` sender **and an initial `OutputMode`**, and
**spawn background sessions directly in `Summary`**. Do **not** wake-in-Detailed
then Demote: catch-up runs before the command select (`runloop.rs:964-1060`) and
commands are deferred during catch-up (`~903`), so Detailed output escapes before
the Demote lands, and `spawn_actor` drops the summary receiver
(→ `SummaryConsumerGone`). Direct `Summary` spawn is required.

### 3.3 Emit-on-transition + seed-on-spawn

With the unified feed these are trivial and **required for a live card**:

- **Seed on spawn:** a Summary-mode actor emits an initial
  `ActorFeed::Summary(from_state)` after catch-up, so the card has data before the
  first live event (today `run()` starts at catch-up then select — no seed).
- **Emit on Demote:** `Demote` today only flips the mode (`runloop.rs:534`); add
  an immediate `ActorFeed::Summary(from_state)` so blur returns the card to the
  summary projection instead of freezing on the last Detailed frame. (`Promote`
  already emits `Rebased` — symmetric.)

### 3.4 Enrich `SummaryUpdate` + expose the completed-turn counter

`SummaryUpdate::from_state` (`actor/summary.rs`) copies 6 fields today. Extend the
struct + function to carry the §6 card chrome. **Correction from review: `Demote`
drops *nothing* — it only flips mode; all scalars stay on `SessionState`** (items
are cleared only on the *emitted* `Rebased` clone, `scalars_baseline`), so every
field below is available in Summary mode:

- from `SessionState` directly: `llm_model`/`model_override`, `agent_name`,
  `cumulative_cost`, `context_window` (+ existing `last_total_tokens` → ctx %),
  `sandbox_status`, `git_branch`/`workspace`, `reasoning_effort`; an **activity
  summary** (derived: `todos.activeForm` ▸ in-flight tool ▸ blank).
- **`last_completed_turn: u32`** — copy `state.stream.turn` (bumped on
  `response.completed`, `reduce/mod.rs:136`). Drives the Ready wave (§3.5).
- **`harness`** — **not on `SessionState` today** (it lives only on
  `lens-client::SessionSnapshot`). Add a lens-core `SessionState.harness` field
  folded from the snapshot at bootstrap, so `<harness> · <model>` (shell §5.1)
  renders. *(Alternatively render model-only and defer harness — but the field is
  cheap and the chrome wants it.)*

Cadence stays coarse (ms–s) — D10's scale property holds; no per-token deltas.

### 3.5 Ready wave — completed-turn counter + Lens-local ack (no migration)

"Ready = idle **with an unacknowledged completed turn**." Rather than a persisted
`has_unseen_result` flag (which would need a real SQLite ALTER + a `Promote`-path
write), derive it from the exposed counter + **Lens-local** ack:

- `SummaryUpdate.last_completed_turn` advances on each turn completion.
- `FleetStore` holds a per-card `acked_turn` (RAM); **on focus it sets
  `acked_turn = last_completed_turn`**. `Ready = status==idle &&
  last_completed_turn > acked_turn`.
- **No persisted flag, no migration, no `Promote`-path coupling.** (`acked_turn`
  may persist Lens-side later for across-restart correctness — a refinement.)

**Forward-compat (cheap):** on focus, also call the already-built
`Sessions::put_read_state(id, now, false)` **on a background executor**
(`cx.background_spawn` — it is a *blocking* client call, never on the gpui
thread) so the web UI / a second Lens instance converge. Reading *other* devices'
acks (`viewer_unread`/`viewer_last_seen` off the fleet poll) is **board-v2**.

---

## 4. State-binding contract (lens-ui) — the load-bearing part

### 4.1 Who folds what

- **Event → state: lens-core reducer, off-thread.** Emits `ActorFeed::Summary`
  (Summary) or `ActorFeed::Detailed(StreamUpdate)` (Detailed). The UI never sees
  raw events.
- **Feed → foreground field: the per-session poller in `FleetStore`.** One
  `cx.spawn` task per session, a `select` over **`{feed, outcomes}`** (now only
  *two* channels — the unified feed collapsed the data channels), draining each
  once and **coalescing a ready burst** before one entity update (lens-store
  `lib.rs:85-105` is the batching precedent), then gated notifies. Single
  foreground dispatch site.

Slogan: **reduce-once in lens-core → dispatch-once in the poller → project.**

### 4.2 Foreground object & the dual-mode fold (now order-safe)

Per warm session, one **`SessionCard`** gpui `Entity` (always resident). The
poller patches it from the unified feed, whose **ordering across the mode-switch
is guaranteed by §3.1**:

- **background / `Summary`:** `ActorFeed::Summary` → copy-assign the enriched
  scalars (incl. activity + `last_completed_turn`).
- **focused / `Detailed`:** `Promote` emits `Rebased` (scalar reseed), then
  `StreamUpdate` deltas patch the **same** card fields. The focused fold must
  consume not only `StatusChanged`/`UsageChanged`/`ModelChanged` but also
  **`TodosChanged`/`ScratchChanged`** (or the activity line stalls while
  focused). `TranscriptAdvanced` + streaming-tail route to the full replica —
  **deferred with the transcript** (focused slot is empty in the skeleton).
- `git_branch`/`workspace` refresh only on `Rebased`/summary snapshots
  (`ResourcesChanged` is a **valueless marker** — no incremental branch delta).

The card renders identically in both modes; the skeleton **proves the mode-switch
is order-correct** (background Summary → Promote/Detailed → Demote-emits-Summary),
plus the Ready ack on focus.

**Routing corrections (vs the enum):** `SnapshotRestored` carries only
`Vec<PendingInput>` and does **not** seed card scalars (`Rebased` does); `Rebased`
clears only `items` (still carries collections/scratch/lifecycle — consume only
the card-relevant subset).

### 4.3 `FleetStore` & ownership

`FleetStore` is a gpui `Entity` that **owns the `FleetScheduler`** (not loose
`ActorHandle`s — the scheduler owns those privately, `ActorHandle` is not `Clone`,
and cloning its receiver would make *competing* consumers, not a broadcast). It
also owns:

- the map `(ConnectionId, SessionId) → SessionCard` **at the UI layer** (each a
  **separate** entity) + `acked_turn` per card;
- the board's ordinal slot layout (shell §4.1);
- **the promote/demote policy** (§9 registry responsibility): the focused session
  is Promoted; all others are spawned/held in `Summary`. The poller is the
  **sole** consumer of each session's `outcomes`.

**Multi-connection caveat:** `FleetScheduler` keys its registry by `SessionId`
**only** (`scheduler.rs:17`). The UI map is composite-keyed, but true multi-server
is **precluded below `FleetStore`** until the engine registry is re-keyed to
`(ConnectionId, SessionId)`. Skeleton = **one connection**.

### 4.4 The gpui isolation invariant (the actual mechanism)

Per-session entities are **necessary but not sufficient**: notifying an entity
dirties it **and every ancestor**, and `Entity<V>` is **not paint-cached unless
you ask** (review, `gpui-0.2.2 window.rs:1304-1317`, `view.rs:99-105,202-215`).
No-cross-invalidation requires **all** of:

1. **Each `SessionCard` view observes its own card entity** (`cx.observe`), never
   a shared store.
2. **`FleetStore` is notified ONLY on membership/layout changes** — never on a
   card's scalar fold (else the whole board re-renders).
3. **Cards are mounted as `AnyView` wrapped in `.cached(style)`** with **stable
   entity IDs** and **stable card bounds** — bare `AnyView` is uncached, and
   paint reuse also requires unchanged bounds/content-mask/text-style.

The board/root *will* still re-render on a membership change (ancestor dirty);
the guarantee is that **unchanged sibling cards do no render/paint work**. (The
prior draft's "notify-gating on unchanged ScratchChanged" is **dropped** — Summary
cadence is already coarse; there is no per-token thrash to gate.)

### 4.5 Commands down + `ActorOutcome`

- **Down:** card kebab / focus → `FleetStore` → scheduler handle →
  `SessionCommand` (`Sleep`, `Send`, `Promote`, `Demote`, `Stop`). **`Interrupt`
  is NOT wired** — `SessionCommand::Stop` exits the *Lens actor loop*
  (`runloop.rs:497`); it does not send the server an interrupt, and there is no
  `Interrupt` variant. A real interrupt is a **new lens-core command path**, out
  of scope.
- **`ActorOutcome`** (drained by the same poller): `Parked` → card connection
  state; `Slept`/`SleepDeclined` once Sleep is wired; `SendLost`/
  `TransportChanged`/`PersistError`/`SummaryConsumerGone` logged.

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
there is **no global ESC→board binding**. Card-click toggle:

- Click a card → focus it (`FleetStore` **Promotes** it, Demotes the previous
  focus, sets its `acked_turn`).
- In focused state (boards shrunk, visible): click a **different** card → switch
  focus; click the **currently-focused** card → toggle back to board (Demote).
- `⌘\` collapses/expands the boards column. `⌘D` deep-focus deferred. ESC stays
  **surface-local**.

### 5.2 `ContentTab` + `SessionAttach` (the terminal seam)

The working-area slot is a **single-tile, single-content mount**. **Dispatch is
decided: the mount holds a small `TabHandle { view: AnyView, title: SharedString }`**
— an `Entity<T: Render + ContentTab>` erased to `AnyView`, with the **title stored
alongside** (it cannot dispatch through `AnyView`). `ContentTab` is a thin
object-safe capability marker; focus/blur arrive via gpui's `FocusHandle` on the
view.

**`SessionAttach`** (what the terminal workstream codes against) carries
**identity + a WS-attach capability**, *not* a notifications receiver:

```rust
struct SessionAttach {
    connection_id: ConnectionId,
    session_id: SessionId,
    attach: TerminalAttachCapability,   // open byte stream + resize
}
```

Corrections: `session.terminal.activity` folds to **nothing** (`folds.rs:125-136`,
the reducer emits no delta); only `terminal_pending` →
`StreamUpdate::TerminalPendingChanged(bool)`, which rides the normal feed;
terminal resource create/delete surfaces as the generic `ResourcesChanged`
marker. The **typed WS terminal client is UNBUILT in lens-client** (REST
create/delete/transfer only) — a genuine dependency of the terminal workstream,
not provided here.

**Deferred to workspace fan-out:** splits, tab-bar, launchers, +badge, preview
tabs, content persistence.

---

## 6. The board-cards proving surface

The card renders shell §5.1 chrome from the **enriched `SummaryUpdate`** — coarse
summary, never a transcript: status icon tile + **wave**, `<STATUS>`/`<Title>`,
`<harness> · <model>`, **activity line**, `📁 repo ⑂ branch`, footer (host pill ·
`~$spend` cumulative, `—` when `None` · `ctx %` bar), connection-state takeover
(§5.4).

**Wave ladder** (shell §5.1) — fully derivable from the enriched feed:
Needs-input (`needs_attention`), **Ready** (`idle && last_completed_turn >
acked_turn`, §3.5), Working (`running/launching/waiting`), Failed (`status`/
`last_task_error`), Slept (lifecycle). Kebab commands wired: Sleep→`Sleep`,
Send→`Send` — **not Interrupt**.

### 6.1 Acceptance test — what the skeleton exists to prove

Notify counts prove only poller gating, **not gpui render isolation**. The test
mounts a **real board + N real card views** in gpui's `TestAppContext` (headless,
`gpui/app/test_context.rs`):

1. settle the first frame; instrument per-card `Render`/paint counters + the
   board/root counter; cards mounted `.cached(...)`;
2. inject an enriched `SummaryUpdate` on session B; drive the frame; assert **B's
   card re-renders, A's card does no render/paint work** (root may invalidate —
   the guarantee is unchanged-sibling reuse, §4.4);
3. **mode-switch order-safety:** with a *lagging* poller, enqueue Summary frames
   then Promote then Detailed frames on the unified feed; assert the card ends on
   the Detailed projection (never regresses to a stale Summary), and blur emits a
   Summary that restores the coarse projection; `acked_turn` updates on focus.

---

## 7. Dev feed & verification

- **Synthetic `FakeFleet`** (`lens-ui` test-support) — **per-session** unified
  `ActorFeed` channels emitting scripted `Summary`/`Detailed` frames + accepting
  `SessionCommand`s. Powers hermetic tests + §6.1; stages N independent sessions
  and the lagging-poller transition deterministically.
- **Live-verify gate** — `lens-app` on the **real `FleetScheduler` + omnigent
  0.5.1** (the `lens-drive` path, but `lens-drive` is single-session Detailed — a
  **weak** analogy, so live-verify must exercise **≥2 background-Summary cards + a
  promote/demote cycle**, not just reuse the drive shape).

---

## 8. Scope boundary (explicit)

**In:** the §3 lens-core phase (unified `ActorFeed`; scheduler dual-mode plumbing
+ spawn-in-Summary; emit-on-Demote + seed-on-spawn; enrich `SummaryUpdate` incl.
`harness` field + `last_completed_turn`); Ready via counter + Lens-local ack +
`put_read_state`-on-focus (off-thread); `lens-app`/`lens-ui` split; `FleetStore`
(owns scheduler + promote/demote policy) + per-session poller + the §4.4 isolation
invariant; board state + enriched card chrome + full wave ladder; focused-state
empty slots + click-toggle recompose (promote/demote); `ContentTab`/`TabHandle` +
`SessionAttach` + placeholder tab; minimal theme tokens; `FakeFleet` + live-verify;
the §6.1 acceptance test.

**Out (later slices):**

- transcript rendering & markdown; the **full replica / disk `RowSource`** (D23) —
  *transcript fan-out* (also where the Detailed feed gets a real consumer);
- workspace / diff / editor; splits / launchers / preview / persistence —
  *workspace fan-out*;
- terminal internals + the **unbuilt typed WS terminal client** — *the parallel
  terminal workstream* (plugs into `ContentTab`/`SessionAttach`);
- a real server **Interrupt** command path (new lens-core command);
- permissions/elicitation forms; Bridge inbox; search; Canvas; Concierge;
  multi-board / groups / archive;
- the **REST-poll coarse-status path** for Slept/archived/non-warm cards (state
  model §10). The fleet poll (`Sessions::list`) **is already built**;
  `SummaryUpdate` is explicitly a two-producer projection ("actor here; §10 poll
  later"). Missing work = **lens-core scheduling/coarse-projection integration**,
  owned by **board-v2**; skeleton = warm/active only;
- **inbound** cross-device read-state (`viewer_*` off the poll) — board-v2;
  **multi-connection** — needs the `FleetScheduler` registry re-keyed;
- persisting `acked_turn` across restarts — refinement.

---

## 9. Testing strategy

- **Hermetic `lens-ui` tests** over `FakeFleet`: §6.1 assertions (independent
  cards, single-card repaint under `.cached`, mode-switch order-safety), card
  chrome per feed variant, wave ladder incl. Ready-via-`acked_turn`, command-down.
- **lens-core tests** for §3: unified `ActorFeed` ordering preserved across a
  Promote/Demote transition; emit-on-Demote; seed-on-spawn; spawn-in-Summary emits
  Summary (not `SummaryConsumerGone`); `SummaryUpdate` enrichment (`from_state`
  populates new fields incl. in Summary mode); `last_completed_turn` tracks
  `stream.turn`.
- **Live-verify** (§7) as the acceptance gate.
- Gate: `cargo clippy --workspace --all-targets -- -D warnings` + `fmt` clean.

---

## 10. Open / deferred (tracked, not blocking)

- `⌘D` deep-focus, `⌘\` polish — with the focused surfaces.
- Multi-server / connection badge — needs the engine registry re-key.
- Send-recovery / `SendLost` UX — with the composer.
- **Board-v2** — the REST-poll path, Slept/archived/groups/multi-board, inbound
  cross-device read-state. The named continuation that owns the §8 poll deferral.

---

## Appendix A — cross-family review disposition (2026-07-14)

Two rounds, codex/gpt + grok-4.5-xhigh, both vs lens-core source.

**Round 1 (design direction) — all accepted after verification:**
board on wrong feed → rebuilt on D10 `SummaryUpdate` + promote/demote; gpui
isolation not automatic → §4.4; terminal seam invented `TerminalNotif` → attach
capability; `Interrupt`→`Stop` wrong → unwired; routing false claims → §4.2;
ownership/keying → §4.3; poller needs select+batch → §4.1; acceptance test
underspecified → §6.1; Ready needs read-state → §3.5. Actor fundamentals (live
reduce→emit, `TranscriptAdvanced` watermark, channel types, `ActorOutcome` set)
confirmed correct.

**Round 2 (verify-the-fixes) — all folded in:**
- **NEW Critical: dual-channel mode-switch reorder** (lagging consumer across a
  transition) → **unified `ActorFeed` channel** §3.1 (fixes by construction;
  user-approved to do now while blast radius is small).
- Demote emits nothing / no spawn seed → §3.3 emit-on-Demote + seed-on-spawn.
- wake-then-Demote unsafe (catch-up defers commands, drops summary rx) →
  spawn-in-Summary §3.2.
- gpui `.cached(style)` + stable IDs/bounds required, not bare `AnyView` → §4.4.
- `harness` has no lens-core field → add `SessionState.harness` §3.4;
  `agent_name` added; "demote drops items" corrected (drops nothing).
- `has_unseen_result` persistence + ALTER migration → **avoided** via
  `last_completed_turn` counter + Lens-local `acked_turn` §3.5.
- `put_read_state` is blocking → off-thread §3.5.
- focused-mode fold must include `TodosChanged`/`ScratchChanged` → §4.2.
- `AnyView` erases `title` → `TabHandle{view,title}` §5.2; terminal fold wording
  corrected (`terminal.activity` → nothing) §5.2.
