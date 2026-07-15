# lens-ui shell skeleton — design

**Status:** Approved 2026-07-14 (brainstorming). **Two cross-family review rounds
folded in** (codex/gpt + grok-4.5-xhigh, both vs lens-core source): round 1
corrected the feed (D10 dual-mode `SummaryUpdate`, not gated `StreamUpdate`);
round 2 (verify-the-fixes) drove the **unified `ActorFeed` channel** decision and
the implementation-precision fixes below. **Round 3 (grill)** then **Round 4
(cross-family review of the round-3 diff — codex/gpt-5.6-sol + grok-4.5-xhigh,
2026-07-15)** folded in — see Appendix A. Round 4 corrected several of round-3's
own fixes (continuous-ack Detailed turn-source, ack-reset-on-seed, the wrong
"mode-exclusive" backpressure claim, read-state semantics). First rendering
consumer of the state model.
**Depends on:** `lens-core` (actor/`FleetScheduler`/`SummaryUpdate`/`StreamUpdate`/
`SessionCommand`/`ActorOutcome`, through P3-3b, live-verified vs omnigent 0.5.1);
`lens-client` (REST surface incl. `put_read_state` + `viewer_*` read fields);
framework lock (gpui 0.2.2); application shell/layout; state model §9 (D10
dual-mode) / §10 (list poll) / §13.2 (seams).
**Feeds:** the parallel surface workstreams (transcript, terminal, workspace,
permissions) — they plug into the slot API (`ContentTab`/`TabHandle`) this
skeleton publishes; the terminal stream is *hosted* by lens-ui via
`lens-terminal::open(...)` (§5.2), not via a lens-ui-published attach type.

---

## 1. Purpose & the risk this retires

`lens-ui` is the **first rendering consumer** of the state model. This skeleton
retires the risk that cannot be parallelized: the **D10 dual-mode
async-stream → state → render bridge** at fleet scale. (Precisely: the §6.1
hermetic test is a **mechanism proof**; the risk is *retired* only when it is
paired with **live-verify at N≥10** (§7) — the hermetic test alone doesn't
exercise the real scheduler or production `refreshing` paths, §6.1.) The bridge —

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

**Implementation-sequencing gate (hard phase boundary, not soft ordering).**
The **one-way door is the channel shape — §3.1 + §3.2 only** (merging the two
senders + the dual-mode spawn signature). *That* pair lands as a
**separately-reviewed, separately-merged lens-core milestone — cross-family + Opus
review — BEFORE any lens-ui view code**, because it's Opus-level actor-touching
work (CLAUDE.md) and reversing the public channel shape later is expensive.
**§3.3–§3.5 are additive** (seed/emit, enrich `SummaryUpdate`, Ready plumbing) and
**may land in parallel** with FakeFleet-backed view code — narrowed per review
(over-gating them needlessly serializes the build).

**Gate evidence must exercise Summary mode — `lens-drive` alone does NOT
(review-corrected).** `lens-drive` is single-session **Detailed-only**, so it
cannot validate the unified channel's Summary/interleave paths. The §3.1+§3.2
gate therefore requires **new lens-core tests**: a Summary-mode actor with
**nonempty startup catch-up** (emits `updates` then `summaries` — the interleave
that motivates the merge), plus **reconnect / deferred-transcript-commit** on the
unified channel. `lens-drive` green is necessary but not sufficient.

### 3.1 Unified `ActorFeed` channel (the keystone)

Today the actor holds **two** senders. The main-batch/pending-user paths are
mode-exclusive (`runloop.rs:682-710, 1064-1076`), **but the actor as a whole is
not** — corrected by cross-family review (codex/5.6-sol vs source): catch-up
sends `TranscriptAdvanced` on `updates` **regardless of `OutputMode`**
(`runloop.rs:416`), deferred-commit likewise (`runloop.rs:796`), and Promote
sends `Rebased` on `updates` **before** flipping `mode = Detailed`
(`runloop.rs:523`). So a Summary-mode actor with nonempty catch-up genuinely
emits on `updates` **then** `summaries`. Across that interleaving the two
independently-buffered channels can be **reordered by a lagging consumer** (a
queued pre-Promote `SummaryUpdate` applied *after* the `Rebased`), regressing the
card. Fix: **merge to one FIFO channel**:

```rust
enum ActorFeed { Summary(SummaryUpdate), Detailed(StreamUpdate) }
// actor: feed: async_channel::Sender<ActorFeed>   (replaces updates + summaries)
```

A single channel preserves the actor's send-order **by construction** — the race
is gone with no epoch/barrier/discard logic. The single FIFO is *required*
precisely because the actor legitimately interleaves the two sub-streams (above);
this is the real justification (an earlier "production is mode-exclusive → no
backpressure downside" note was **wrong** and is struck, Appendix A round 4).

**Capacity / backpressure contract.** The feed stays **`bounded(64)`** (today's
size). The actor uses `send_blocking`, so a lagging poller applies **backpressure
that is Lens-side and lossless** — the reducer slows, nothing is dropped (correct:
the actor is our thread, not omnigent). The poller **drains-and-coalesces per
wakeup** (§4.1), so a mode-transition Summary lands **≤1 coalesced batch behind**
any Detailed backlog — bounded, never a freeze; §6.1 step 3 exercises this. No
actor-side latest-wins Summary coalescing now (Summary cadence is coarse ms–s,
can't fill 64) — the escape hatch *if* profiling ever shows Summary starvation.

**Blast radius (do it now, while small):** `spawn_actor`/`spawn_actor_dual`,
`FleetScheduler::wake`/`reconnect`, `lens-drive`, and the actor tests that
`recv()` the old channels (mechanical unwrap churn). Doing this before the
transcript slice accretes a second consumer is strictly cheaper than after.

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

### 3.5 Ready wave — warm fast-path echo of read-state (no migration)

"Ready = idle **with an unacknowledged completed turn**." Ready is a **live
"just finished, look now" nudge, not durable state** — that framing (from the
grill's cross-family review) resolves what used to be a muddle. The skeleton
derives it from the warm feed at **turn-completion latency**, with a local ack
that is the **optimistic echo of the `put_read_state` we write on blur**.

**Server read-state is a convenience layer, not an authority — and correcting an
earlier error:** the server's unread is **not** `viewer_unread` alone.
`viewer_unread` is only the explicit *marked-unread override*; the actual unread
dot is derived from the **triple** `updated_at > viewer_last_seen && status
finished` (`openapi.json:3957-3964`). It is also **in-memory and resets on a
server restart**. So board-v2 cross-device convergence (while the server is up)
needs all three fields, not `viewer_unread`; and there is **no across-restart
Ready anywhere** — which is fine, because Ready is ephemeral by design (§ ack
bullets: a restart/reconnect shows the card's true idle/failed status). **No
omnigent ask** — nothing is missing; a durable Ready is not a goal. Regimes:
**warm/live → feed counter (instant, skeleton); cross-device while server-up →
read-state triple (poll, board-v2).**

- `SummaryUpdate.last_completed_turn` advances on each turn completion (`=
  state.stream.turn`). **Why a counter, not a RAM boolean edge:** the feed
  coalesces bursts (§4.1), so a naive `running→idle` edge-detector can miss the
  transition; `last_completed_turn > acked` is robust to coalescing. That
  robustness is the *only* reason it's a counter.
- **The card's turn counter has TWO sources — bind both** (review-corrected):
  `last_completed_turn` is a `SummaryUpdate` field, so it only arrives in
  **Summary** mode. In **Detailed** (focused) mode no `SummaryUpdate` flows —
  `stream.turn` rides `StreamUpdate::ScratchChanged` (`reduce/mod.rs` emits it on
  `Completed`, carrying `scratch.turn == stream.turn`; also reseeded on `Rebased`).
  So the card maintains its turn counter from **`SummaryUpdate.last_completed_turn`
  (Summary) OR `ScratchChanged.turn`/`Rebased` (Detailed)**. Without the Detailed
  source, continuous-ack has nothing to advance while focused → the exact bug it
  exists to kill.
- `FleetStore` holds a per-card `acked_turn` (RAM).
  `Ready = status==idle && turn_counter > acked_turn`.
- **Reset `acked_turn = seed.<turn>` on EVERY seed, not just card creation**
  (review-corrected). The card outlives the actor: a Parked→reconnect **respawn**
  re-seeds with `turn=0`, but a persisted `acked_turn` would suppress Ready. So
  treat every post-spawn seed (birth *and* respawn) as the ack baseline. This also
  means Ready is **not preserved across a reconnect/server restart** — the card
  shows its true idle/failed status, which is **correct**: Ready is a live "just
  finished, look now" nudge, not durable state; the result is still in the
  transcript. (Do **not** rely on "turn always resets to 0 on spawn":
  `spawn_actor_dual` preserves a supplied `SessionState.stream.turn`
  (`runloop.rs:141`); only the persistence-reload + current wake/reconnect paths
  start at 0. Resetting acked from the *seed value* is robust either way.)
- **Ack rule = continuous-while-focused, NOT set-once-on-focus.** The focused
  card keeps `acked_turn == turn_counter` for as long as it is focused — set on
  Promote **and re-advanced on every completion (via the Detailed `ScratchChanged`
  source above) while focused**. On Demote it **freezes**. Rationale: acking only
  on focus leaves turns completed *during* focus un-acked, so the focused card
  would light **Ready while you watch it** (and stay stale-Ready on blur).
  Continuous-ack = "`acked_turn` = the last turn you could have seen": a turn
  completed **while focused** never raises Ready; only a completion **after blur**
  does. The `Ready` formula is unchanged — only the ack-update rule.
- **No persisted flag, no migration, no `Promote`-path coupling.**

**Forward-compat (cheap) — `put_read_state` on BLUR, not focus** (review-corrected).
On **Demote**, call `Sessions::put_read_state(id, now, /*unread*/ false)` **on a
background executor** (`cx.background_spawn` — it is a *blocking* client call,
never on the gpui thread). Blur is when the local ack freezes, so local and server
agree by construction (one write per focus-session, coalesced). A focus-time PUT
would **diverge**: a turn completing after it advances the local ack but not
server `last_seen`, and the server stores timestamps **verbatim** (`openapi.json`
read-state: server does not interpret), so the next poll re-marks unread. Reading
*other* devices' read-state (the triple) is **board-v2**; there it becomes the
cross-device convergence source (server-up only) and `acked_turn` degrades to a
pure optimistic-latency shim.

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
4. **`SessionCard` is a FIXED-SIZE tile — pinned outer W×H, no fold changes it.**
   This is not automatic: gpui's cache reuse keys on `cache_key.bounds == bounds`
   (`view.rs:207-216`), and the board is an **ordinal reflow grid** whose packing
   algorithm is **unspecified** (`application-shell-and-layout.md`), so a card that
   *content-sizes* would grow/shrink under a fold and shift the bounds of the
   **downstream siblings that reflow past it** (not necessarily *every* sibling —
   review-corrected; a card whose bounds are unchanged still reuses its cache). To
   remove the hazard entirely, **pin explicit tile width/height** and absorb every
   variable element **inside** it: the activity line is a **reserved slot** (blank
   when idle, never collapsing), repos render as **exactly one row + a `·+N`
   overflow badge** (never a row-per-repo), long strings ellipsize (§6). Failed's
   **Retry** and the §5.4 connection-state takeover are **reserved slots too**, not
   content that grows the tile. Any full-detail affordance (repo list) is a
   **floating overlay** (hover tooltip: dirties/repaints the hovered card but does
   **not reflow siblings**) — never inline expansion, which would resize the tile.

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
  focus). The focused card's `acked_turn` then **tracks `last_completed_turn`
  continuously** until blur (§3.5 continuous-ack), freezing on Demote.
- In focused state (boards shrunk, **always visible** in the skeleton): click a
  **different** card → switch focus; click the **currently-focused** card → toggle
  back to board (Demote).
- **`⌘.` = back to board** (Demote the focused session). A dedicated `⌘`-chord,
  **not ESC**: ESC is reserved for harness forwarding (a bare key in the TUI-native
  raw-input stream — and ESC = interrupt is the agentic convention, which working
  *through* the forwarder is exactly what we want). No cancel/stop collision: in
  this category interrupt is ESC, not `⌘.`. This makes board-return
  **keyboard-reachable** (not mouse-only) and **independent of the boards column**.
  - **Safety is a mechanism, not a claim (review-corrected):** `⌘.` must be a
    registered **app-level `Action` that takes routing priority over the terminal
    tab's key handler** so it can never leak into the PTY. Verified by the §6.1
    dispatch test (terminal focused → `⌘.` fires Demote, **zero PTY bytes sent**).
- **`⌘\` (collapse boards column) is DEFERRED** (§10) — kept out of the skeleton so
  the focused card's click-return target is always present; `⌘.` covers the
  keyboard path and any future collapse. `⌘D` deep-focus deferred. ESC stays
  **surface-local**.

### 5.2 `ContentTab` mount + the terminal integration seam

The working-area slot is a **single-tile, single-content mount**. **Dispatch is
decided: the mount holds a small `TabHandle { view: AnyView, title: SharedString }`**
— an `Entity<T: Render + ContentTab>` erased to `AnyView`, with the **title stored
alongside** (it cannot dispatch through `AnyView`). The **title must be
updatable** by the mount, not write-once: dynamic-title content (the terminal tab
sources title/lifecycle from `TerminalTab::presentation()`) refreshes it as the
adapter observes change events. `ContentTab` is a thin object-safe capability
marker; focus/blur arrive via gpui's `FocusHandle` on the view. In the skeleton
the mount holds a **placeholder** tab.

**Terminal integration — the seam runs lens-ui → lens-terminal, not the reverse.**
The sibling terminal workstream (`lens-terminal-ws/docs/specs/2026-07-14-terminal-workstream-design.md`,
both docs in planning as of 2026-07-14) is explicit that **`lens-ui` is not a
dependency of it** and that integrating the tab into lens-ui is *out of its
scope*. So **lens-ui depends on `lens-terminal`** and **hosts** its tab; there is
**no lens-ui-published attach type** for the terminal stream to code against
(the earlier `SessionAttach`/`TerminalAttachCapability` sketch was the wrong
shape *and* wrong direction — **dropped**). The consumed contract, as corrected
by the terminal agent's post-grill reconciliation:

```rust
// owned/exported by lens-terminal (identity is NOT a flat tuple):
pub enum TerminalTarget {
    Existing     { session_id: SessionId, terminal_id: TerminalId }, // exactly this
                                                                     // resource; never
                                                                     // adopts a successor
    OpenOrCreate { session_id: SessionId, key: TerminalKey },        // logical slot;
                                                                     // discover/create;
                                                                     // follows only an
                                                                     // exact-key heir
}
pub enum AccessIntent { Automatic, ReadOnly }   // rides in TerminalOpenOptions
// no `host` param — the tab exposes its seam via methods + two typed streams:
pub fn open(target: TerminalTarget, client: Arc<Client>,
            options: TerminalOpenOptions, cx: &mut App) -> Entity<TerminalTab>;
```

- **Target resolution (list/create/attach) lives privately inside `open()`**,
  which returns immediately in `Starting` (discovery/create/attach run
  off-thread; failures become lifecycle values, not constructor errors). lens-ui
  never lists, creates, resolves, or attaches terminal resources.
- **Access is intent, not authority.** `AccessIntent` rides inside
  `TerminalOpenOptions` (access intent + scrollback limit + initial prefs). lens-ui
  may force `ReadOnly` but must **not** assert authoritative write; under
  `Automatic`, session ownership + *server authorization* decide the effective
  mode and lens-terminal downgrades if ownership/permission is absent or lost.
- **Host seam is LOCKED (terminal grill closed) — two typed streams + two
  methods, no callback trait, no `host` constructor param:**
  - `TerminalTab::focus_handle(cx)` — direct, host-driven focus.
  - `TerminalTab::presentation()` — latest atomic title/lifecycle/access/progress
    (the `ContentTab` adapter reads this for tab chrome, incl. the dynamic title).
  - inbound **`TerminalHostEvent`** (lens-ui drives *into* the tab): session
    Sleep/wake/reset, `session.superseded`, resource-generation signals, pref
    changes, memory pressure, typed host-request responses.
  - outbound **`TerminalEvent`** (lens-ui consumes): presentation changes + host
    requests — user-gesture URL opens, permissioned OSC 52 clipboard writes,
    background notifications (permissioned ones carry a typed request-id →
    response). **No arbitrary `RequestClose`; no client transfer request.**
- **lens-ui owns:** choosing `Existing` vs `OpenOrCreate` from the user action;
  resolving `ConnectionId → Arc<Client>`; access intent via `TerminalOpenOptions`;
  feeding `session.superseded` as a `TerminalHostEvent` (never the schema-hidden
  internal transfer route) — **BUT blocked today: the reducer drops the event's
  `target_conversation_id` (`folds.rs` marker-only), so this cannot work until
  lens-core surfaces it (SPEC-GAPS cross-spec risk; terminal-integration-era, not
  the skeleton).** Do not treat superseded-observation as wired. Also lens-ui owns
  **wrapping** the returned `Entity<TerminalTab>` in a `ContentTab` adapter
  (reading `presentation()` for title/lifecycle); app chrome / routing / policy.
  `lens-terminal` can't implement lens-ui's `ContentTab` (no dependency edge that
  way), so lens-ui adapts.
- **lens-ui does NOT own:** terminal list/create/attach REST, terminal WS
  details, replacement/reconnect policy, effective authorization, or Ghostty /
  transport types.
- **⚠ lens-core dependency for the supersession responsibility (terminal-integration
  era, NOT skeleton).** `session.superseded` carries `target_conversation_id`, but
  the reducer currently folds `SessionEvent::Superseded` to **nothing** (marker-only,
  `folds.rs:136`) — the payload is dropped, so lens-ui cannot get the redirect
  target from the feed. Before the terminal slice can honor "feed
  `session.superseded` to the tab," lens-core must **surface it** (e.g.
  `StreamUpdate::Superseded { target_conversation_id, reason }`). It's transient /
  live-only / no-replay (0.5.1 contract), so the durable `message`-item counterpart
  is a separate reload path. Recorded here + flagged to the terminal agent; out of
  skeleton scope (placeholder tab doesn't supersede).
- **Skeleton scope:** publish only `ContentTab`/`TabHandle` + the placeholder;
  the shapes above are the **locked joint contract**, mirrored in that repo's
  `SPEC-GAPS.md` (terminal agent owns that file). Not built here.

Corrections: `session.terminal.activity` folds to **nothing** (`folds.rs:125-136`,
the reducer emits no delta); only `terminal_pending` →
`StreamUpdate::TerminalPendingChanged(bool)`, which rides the normal feed;
terminal resource create/delete surfaces as the generic `ResourcesChanged`
marker. The **typed WS terminal client is UNBUILT in lens-client** (REST
create/delete/transfer only) — owned by the terminal workstream (its
`TerminalAttachment`), not provided here.

**Deferred to workspace fan-out:** splits, tab-bar, launchers, +badge, preview
tabs, content persistence.

---

## 6. The board-cards proving surface

The card renders shell §5.1 chrome from the **enriched `SummaryUpdate`** — coarse
summary, never a transcript: status icon tile + **wave**, `<STATUS>`/`<Title>`,
`<harness> · <model>`, **activity line**, `📁 repo ⑂ branch`, footer (host pill ·
`~$spend` cumulative, `—` when `None` · `ctx %` bar), connection-state takeover
(§5.4).

**Fixed-tile chrome rules (the §4.4 bounds invariant, made concrete).** The card
is a **fixed-size tile**; every element occupies a reserved slot so no fold
changes outer bounds:

- **Activity line** — a **reserved slot**, blank when idle (not "active cards
  only / absent" — an absent row would change height on active↔idle). Ellipsizes.
- **Repo/branch** — **exactly one row.** Show the **primary** repo (first by
  **stable** workspace order — never reorders under a fold) `📁 <repo> ⑂ <branch>`;
  if >1 repo, suffix a compact **`·+N` badge** on the same row. `0` repos → `—`,
  slot still reserved.
- **Full repo list** — a **hover tooltip** (floating overlay; repaints only the
  hovered card, never reflows the grid). Not inline, not a per-repo row stack.
- All scalar strings (`<Title>`, model, activity, branch) **ellipsize** within the
  fixed bound.

> **Supersedes shell §5.1's "one row per repo" for the board tile.** §5.1 says
> multi-repo sessions "show a row each" — that content-sizes the card and defeats
> §4.4. Board tile = one row + `·+N` + hover tooltip; a full per-repo view belongs
> to the focused surfaces. (Shell-doc reconciliation, like the terminal seam.)

**Wave ladder** (shell §5.1) — fully derivable from the enriched feed:
Needs-input (`needs_attention`), **Ready** (`idle && turn_counter > acked_turn`,
dual turn-source, §3.5), Working (`running/launching/waiting`), Failed (`status`/
`last_task_error`), Slept (lifecycle). Kebab commands wired: Sleep→`Sleep`,
Send→`Send` — **not Interrupt**.

### 6.1 Acceptance test — what the skeleton exists to prove

Notify counts prove only poller gating, **not gpui render isolation**. The test
mounts a **real board + N real card views** in gpui's `TestAppContext` (headless,
`gpui/app/test_context.rs`):

1. settle the first frame; instrument per-card `Render`/paint counters + the
   board/root counter; cards mounted `.cached(...)`;
   - **Frame-driver caveat (impl):** drive redraws by `card.update(cx, |_, cx|
     cx.notify())` — the notify's **effect-flush** is what draws (test-support
     `flush_effects` auto-draws dirty windows at `refreshing=false`);
     `run_until_parked` only drains executor tasks, it does not itself draw
     (review precision). **Do NOT use `cx.refresh()` / `refresh_windows()`** — they
     set `window.refreshing=true`, which makes gpui *ignore* `.cached()`
     (`view.rs:100-101`, reuse guard `!window.refreshing`), so every card repaints
     and the isolation assertion fails on correct code.
2. inject an enriched `SummaryUpdate` on session B; drive the frame; assert **B's
   card re-renders, A's card does no render/paint work** (root may invalidate —
   the guarantee is unchanged-sibling reuse, §4.4);
   - **Size-invariance sub-assertion (else this test gives false confidence):** a
     single fixed-geometry injection can't prove bounds-stability — and "A doesn't
     paint" proves nothing if **A precedes B** in the grid (A's bounds don't move
     even under content-sizing). So: place a **downstream** sibling C (packs *after*
     B), fold B with a change that *would* grow it under content-sizing — activity
     line idle→present **and** repos `1 → 3` (must collapse to one row + `·+2`) —
     and assert **C's bounds are byte-for-byte unchanged and C does no paint work**.
     That proves the fixed-tile invariant (§4.4 pt 4), which an upstream-only
     assertion silently misses.
   - Also assert **FleetStore notify count == 0** on a scalar fold (§4.4 pt 2) — a
     leaked store-notify would full-repaint the board while the per-card asserts
     still pass.
3. **mode-switch order-safety:** with a *lagging* poller, enqueue Summary frames
   then Promote then Detailed frames on the unified feed; assert the card ends on
   the Detailed projection (never regresses to a stale Summary), and blur emits a
   Summary that restores the coarse projection.
   - **continuous-ack (§3.5) — exercise the Detailed turn-source, not a Summary
     counter bump:** while focused, complete a turn **via `StreamUpdate::ScratchChanged`
     (Detailed mode — no `SummaryUpdate` flows while focused)** and assert the card
     does **not** go Ready; then Demote and complete another turn and assert it
     **does** go Ready. This catches the real bug (continuous-ack having no
     Detailed-mode source), which a `last_completed_turn++` (Summary-only) fixture
     would false-pass.
   - **`⌘.` dispatch (§5.1):** with a terminal-focused tab, fire `⌘.`; assert Demote
     fires **and zero PTY bytes are sent** (the app-level Action wins over the
     terminal key handler).

**This is a mechanism proof, not "D10 retired" (review calibration).** §6.1 +
`FakeFleet` prove the sibling-cache / mode-order / ack mechanics; they do **not**
exercise the real N-warm scheduler or production `refreshing=true` paths. D10-at-
fleet-scale is retired only together with **live-verify at N≥10** (§7).

---

## 7. Dev feed & verification

- **Synthetic `FakeFleet`** (`lens-ui` test-support) — **per-session** unified
  `ActorFeed` channels emitting scripted `Summary`/`Detailed` frames + accepting
  `SessionCommand`s. Powers hermetic tests + §6.1; stages N independent sessions
  and the lagging-poller transition deterministically.
- **Live-verify gate (this is what actually retires D10, with §6.1)** — `lens-app`
  on the **real `FleetScheduler` + omnigent 0.5.1** (the `lens-drive` path, but
  `lens-drive` is single-session Detailed — a **weak** analogy, so live-verify must
  exercise **≥10 background-Summary cards + promote/demote cycles**, not just reuse
  the drive shape). N≥10 is the fleet-scale bar the hermetic test can't stand in for.

---

## 8. Scope boundary (explicit)

**In:** the §3 lens-core phase (unified `ActorFeed`; scheduler dual-mode plumbing
+ spawn-in-Summary; emit-on-Demote + seed-on-spawn; enrich `SummaryUpdate` incl.
`harness` field + `last_completed_turn`); Ready via counter (dual turn-source) +
Lens-local continuous-ack + `put_read_state`-on-**blur** (off-thread);
`lens-app`/`lens-ui` split; `FleetStore`
(owns scheduler + promote/demote policy) + per-session poller + the §4.4 isolation
invariant; board state + enriched card chrome + full wave ladder; focused-state
empty slots + click-toggle recompose (promote/demote) + `⌘.` back-to-board;
`ContentTab`/`TabHandle` +
placeholder tab (terminal seam = *consume* `lens-terminal::open`, §5.2, not built
here); minimal theme tokens; `FakeFleet` + live-verify;
the §6.1 acceptance test.

**Out (later slices):**

- transcript rendering & markdown; the **full replica / disk `RowSource`** (D23) —
  *transcript fan-out* (also where the Detailed feed gets a real consumer);
- workspace / diff / editor; splits / launchers / preview / persistence —
  *workspace fan-out*;
- terminal internals + the **unbuilt typed WS terminal client** (`TerminalAttachment`)
  — *the parallel terminal workstream*; lens-ui hosts its `Entity<TerminalTab>` by
  consuming `lens-terminal::open(...)` and wrapping it in a `ContentTab` adapter;
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
  Promote/Demote transition; **Summary-mode actor with nonempty catch-up emits
  `updates` then `summaries` on the single FIFO in order** (the interleave that
  motivates §3.1 — `lens-drive` can't cover it, Detailed-only); reconnect /
  deferred-transcript-commit on the unified channel; emit-on-Demote; seed-on-spawn;
  spawn-in-Summary emits Summary (not `SummaryConsumerGone`); `SummaryUpdate`
  enrichment; `last_completed_turn` tracks `stream.turn`. **The channel-shape
  subset (§3.1+§3.2) is the hard merge gate (§3 preamble): green + cross-family/Opus
  review + `lens-drive` still works, before any lens-ui view code. §3.3–§3.5 tests
  may land alongside view code.**
- **Live-verify** (§7) as the acceptance gate.
- Gate: `cargo clippy --workspace --all-targets -- -D warnings` + `fmt` clean.

---

## 10. Open / deferred (tracked, not blocking)

- `⌘D` deep-focus, **`⌘\` collapse boards column** (deferred from the skeleton per
  §5.1 to keep the click-return target always present) — with the focused surfaces.
  (`⌘.` back-to-board **is** in the skeleton.)
- Multi-server / connection badge — needs the engine registry re-key.
- Send-recovery / `SendLost` UX — with the composer.
- **Board-v2** — the REST-poll path, Slept/archived/groups/multi-board, inbound
  cross-device read-state. The named continuation that owns the §8 poll deferral.

---

## Appendix A — cross-family review disposition (2026-07-14 … 07-15)

Four rounds. R1–R3 codex/gpt + grok-4.5-xhigh vs lens-core source; R4
codex/gpt-5.6-sol (source-audit) + grok-4.5-xhigh (design-logic) vs the R3 diff.

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

**Round 3 (grill, 2026-07-14) — folded in:**
- Acceptance-test frame-driver: `refresh()` sets `window.refreshing` which makes
  gpui *ignore* `.cached()` (`view.rs:100-101`) → §6.1 pins targeted `notify` +
  `run_until_parked`, forbids `refresh()`. (Impl caveat, not design.)
- Ready wave reframed §3.5: `acked_turn` is the **warm fast-path echo of
  read-state**, not a parallel scheme; `viewer_unread` (poll, board-v2) is the
  complementary non-warm/cross-device source. Pinned initial `acked_turn = seed
  turn`; documented the `turn`-resets-on-spawn invariant; **deleted** the
  "persist `acked_turn` later" line (it would *suppress* Ready, not fix it —
  across-restart correctness needs the monotonic server signal = board-v2);
  counter-vs-boolean rationale (robust to burst-coalescing) recorded.
- §4.4 isolation had an unstated precondition: cache reuse keys on stable
  `bounds`, but §5.1 cards content-size (activity line "active-only"; row-per-repo)
  → a size-changing fold reflows the ordinal grid and repaints siblings. Fix:
  **fixed-size tile** invariant (§4.4 pt 4 + §6) — activity line reserved/blank;
  repos = **one row + `·+N` badge + hover-tooltip full list** (supersedes shell
  §5.1 "row per repo" for the board tile); ellipsize; overlay not inline. §6.1
  gains a **size-invariance sub-assertion** (activity 0→1, repos 1→3, siblings
  still don't paint).
- Navigation §5.1: board-return was **mouse-only + had a `⌘\`-collapse dead-end**
  (focused card is the only return target but collapse hides it; ESC reserved).
  Fix: **`⌘.` back-to-board** (`⌘`-chord, safe vs harness forwarding unlike ESC;
  keyboard-reachable, column-independent) + **defer `⌘\`** so the click target is
  always present.
- Ready ack timing corrected §3.5/§5.1/§6.1: **continuous-ack-while-focused**, not
  set-once-on-focus — a set-once ack lights Ready on the focused card *while you
  watch it* and leaves it stale-Ready on blur. Ack now tracks `last_completed_turn`
  while focused, freezes on Demote; only post-blur completions raise Ready.
- §3 elevated from "skeleton plumbing" to a **merge-gated lens-core milestone**
  (§3 preamble + §9): one-way-door actor change, cross-family+Opus review before
  view code. *(The round-3 "backpressure cleared: mode-exclusive" note was **wrong**
  — see round 4.)*
- Terminal seam corrected vs the sibling terminal-workstream design (both docs
  in planning): `SessionAttach`/`TerminalAttachCapability` **dropped** — wrong
  shape and wrong direction (lens-ui depends on lens-terminal, hosts+adapts its
  `Entity<TerminalTab>`). §5.2 now records the *consumed* contract, corrected by
  the terminal agent's reconciliation (terminal grill now **closed**): identity is
  a **`TerminalTarget` enum** (`Existing`{sess,term} vs `OpenOrCreate`{sess,key} —
  never a flat tuple); **access is `AccessIntent`** inside `TerminalOpenOptions`
  (intent, not authority; server authz is authoritative); constructor is
  `open(target, client, options, cx)` (**no `host` param**). Host seam **now
  locked**: `focus_handle(cx)` + `presentation()` methods, inbound
  `TerminalHostEvent` (lens-ui drives `session.superseded` etc.) + outbound
  `TerminalEvent` (URL/OSC-52/notify), **no `RequestClose`/transfer**. `TabHandle`
  title made updatable (dynamic-title content). Joint contract mirrored in
  `lens-terminal-ws/docs/SPEC-GAPS.md` (owned there).

**Round 4 (cross-family review of the round-3 diff, 2026-07-15) — codex/gpt-5.6-sol
(source-audit) + grok-4.5-xhigh (design-logic); both said "do not pass round-3
yet." All folded in — several correct round-3's *own* fixes:**
- **§3.5 continuous-ack had no turn source in Detailed mode** — `last_completed_turn`
  is Summary-only; focused turns ride `ScratchChanged`. Bound the card counter to
  **both** sources; §6.1 now exercises the Detailed path (a Summary-only fixture
  false-passes).
- **`acked_turn` must reset on *every* seed, not just card creation** — the card
  outlives the actor; a respawn re-seeds `turn=0`. Also don't rely on "turn resets
  on spawn" (`spawn_actor_dual` preserves a supplied turn). Reset from the seed
  value. Across-restart/reconnect Ready is **ephemeral by design** (card shows true
  status) — no omnigent ask.
- **§3.1 "mode-exclusive → no backpressure" was factually WRONG** — the actor emits
  on both sub-streams (catch-up `updates` then `summaries`; Promote before flip).
  This *strengthens* §3.1 (single FIFO is required for that interleave); justification
  rewritten + `bounded(64)` lossless-backpressure contract added.
- **§3.5 read-state mischaracterized** — `viewer_unread` is only the override; real
  unread = triple (`viewer_last_seen`+`updated_at`+status), and it's in-memory
  (resets on server restart). Reframed; board-v2 uses the triple.
- **`put_read_state` moved focus→blur** (local ack diverged from server `last_seen`).
- **§3 gate narrowed to §3.1+§3.2** (channel shape); §3.3–3.5 parallelizable.
  **`lens-drive` (Detailed-only) can't validate Summary/interleave** → gate needs
  Summary-mode + catch-up + reconnect tests.
- **§4.4 "every sibling" → downstream-siblings-that-reflow**; pin tile W×H; Retry +
  §5.4 takeover are reserved slots; tooltip "no sibling reflow" (not "repaints only
  hovered"). §6.1 uses a **downstream** sibling C + byte-equal bounds + FleetStore
  notify==0.
- **§1/§6.1 "retires D10" → mechanism-proof + live-verify N≥10.**
- **`⌘.` safety = app-level Action priority over the terminal key handler** + §6.1
  dispatch test (0 PTY bytes). Key kept (ESC=interrupt is the agentic convention).
- **§5.2 superseded**: don't assert lens-ui observes it — blocked on the lens-core
  `folds.rs` drop (SPEC-GAPS).
