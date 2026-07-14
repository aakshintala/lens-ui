# lens-ui shell skeleton — design

**Status:** Approved 2026-07-14 (brainstorming). First rendering consumer of the
state-model §13.2 seams.
**Depends on:** `lens-core` (actor/`FleetScheduler`/`StreamUpdate`/`SessionCommand`/
`ActorOutcome`, built through P3-3b, live-verified vs omnigent 0.5.1); framework
lock (gpui 0.2.2, framework.md); application shell/layout (`docs/design/
application-shell-and-layout.md`); state model (`docs/design/
app-architecture-and-state-model.md` §13.2).
**Feeds:** the parallel surface workstreams (transcript, terminal, workspace,
permissions, sub-agent topology) — they plug into the slot API + `SessionAttach`
this skeleton publishes.

---

## 1. Purpose & the one risk this retires

`lens-ui` is the **first rendering consumer** of the state model. Everything
below the render layer is built and live-verified (`lens-client`, `lens-core`
through P3-3b, `lens-drive` as the headless precedent). This skeleton exists to
retire the **one risk that cannot be parallelized**: the async-stream → state →
render bridge (framework §2.1/§14) — specifically that **N warm sessions update
independently without cross-invalidation**, given gpui has **no per-field
subscription** (framework §4.2).

The skeleton is **not** an empty window. Its deliverable is the **wiring contract
+ a slot API**, *proven* by exactly one thin real surface — the board's session
cards — so that transcript / terminal / workspace / permissions can then fan out
in parallel against a stable seam.

**Non-goal:** building any of those surfaces. See §7 for the explicit scope
boundary.

---

## 2. Crate layout

- **`crates/lens-ui`** (lib) — all views, view-models, the `FleetStore`, the
  gpui↔lens-core bridge (the per-session poller), the slot API, the `ContentTab`
  trait, the synthetic feed for tests. No `main`. Unit-testable against the
  synthetic feed.
- **`crates/lens-app`** (bin) — window bootstrap, gpui `Application` setup, theme
  install, chooses the **feed source** (synthetic **or** the real
  `FleetScheduler`) and wires its channels into a `lens-ui` root view. `main`.

The feed source is selected at the `lens-app` boundary. `lens-ui` only ever sees
the channel types — `async_channel::Receiver<StreamUpdate>`,
`crossbeam_channel::Sender<SessionCommand>`,
`async_channel::Receiver<ActorOutcome>` — so synthetic and live are drop-in. The
channel *is* the seam; no new abstraction is introduced.

---

## 3. State-binding contract (the load-bearing part)

### 3.1 Who folds what

Two distinct layers; conflating them is the trap:

- **Event → state: `lens-core`'s reducer, off-thread.** `reduce(state, event,
  clock)` already turns raw SSE into *fine-grained* `StreamUpdate` deltas
  (`StatusChanged`, `UsageChanged`, `ModelChanged`, …). **The UI never sees or
  re-folds raw events.**
- **`StreamUpdate` → foreground field: the per-session poller in `FleetStore`.**
  One `cx.spawn` task per subscribed session `await`s the `async_channel` and is
  a single `match` over the variants; each arm writes the field(s) into the
  projection entity/entities and gates `cx.notify()` on an actual change.

There is **no separate "full replica that folds then projects."** The projection
entities *are* the foreground storage; the poller is the single fan-out /
dispatch site. The accurate slogan is **"reduce-once in lens-core → dispatch-once
in the poller → store in projections."**

### 3.2 Foreground objects

- **`SessionCard`** — a gpui `Entity`, a *lossy scalar projection*, **always
  resident** per warm session. It stores the chrome subset the card renders (§5)
  and is patched by the poller from these `StreamUpdate` variants: `StatusChanged`,
  `LastTaskErrorChanged`, `UsageChanged`, `ModelChanged`, `AgentChanged`,
  `PresenceChanged`, `SandboxChanged`, `TerminalPendingChanged`,
  `ElicitationsChanged` (→ count), `ContextWindowChanged`, `LastTokensChanged`,
  `TodosChanged` (→ activity line), `ScratchChanged` (→ *derived* activity
  summary), reconnect-lifecycle markers (→ connection state, shell §5.4).
  `Rebased(scalars_baseline)` / `SnapshotRestored` seed or re-seed its scalars.
  The **wave** is *derived* (status + elicitation-count + Lens lifecycle) per the
  shell §5.1 ladder — not a stored field.
- **Full replica (focused-session state)** — **deferred with the transcript
  fan-out.** The skeleton's focused state is empty slots (§4), so no full replica
  is materialized yet.

### 3.3 `FleetStore`

A gpui `Entity` owning:

- the map `(ConnectionId, SessionId) → SessionCard` (each a **separate** entity,
  so one card's `notify()` never repaints the board),
- the board's **ordinal slot** layout (shell §4.1),
- the per-session `ActorHandle`s (for command dispatch).

### 3.4 The bridge (framework §2.1)

On subscribe, `lens-app` hands `FleetStore` the channels for that session.
`FleetStore` spawns one `cx.spawn` poller per session that `await`s the
`StreamUpdate` and `ActorOutcome` receivers and applies each item to the card.

### 3.5 Routing contract (the durable pin)

The `StreamUpdate` stream is *almost entirely chrome + watermarks*: `Rebased`
carries `scalars_baseline(state)` (**chrome scalars only, no transcript items**;
`runloop.rs`), and `TranscriptAdvanced` is a `committed_ordinal` **watermark**
only. So the contract is **fold-once-per-session, project-many**, and the
partition is narrow:

| Delta class | Consumer(s) | Skeleton behavior |
|---|---|---|
| scalar folds, `ScratchChanged`, reconnect-lifecycle, `Rebased`/`SnapshotRestored` | card **and** (later) transcript — *shared live chrome* | card folds them |
| `TranscriptAdvanced` (committed-transcript watermark) | transcript **only** (disk-backed, D23 RowSource) | **dropped** — the actor has already persisted the rows to disk; no data loss |

**The sole transcript-only / disk-backed delta is `TranscriptAdvanced`.**
Everything else is live chrome the card consumes now and the transcript
projection will *also* observe later (it hooks into the same poller `match`;
`TranscriptAdvanced` is the arm that, there, triggers the disk read). Several
scalars are genuinely shared (the transcript reads context-meter / status /
usage / todos-inline per §13.2) — that is expected under project-many.

### 3.6 Granularity mechanism (no built-in selectors)

Because gpui `Entity::observe` notifies **all** observers on any `cx.notify()`
(framework §4.2), the poller **gates `notify()` on whether a card-visible field
actually changed**. In particular a high-frequency `ScratchChanged` whose derived
activity-line summary is unchanged repaints **nothing**. This is what keeps a
background session streaming tokens from thrashing the board.

### 3.7 Commands down + `ActorOutcome`

- **Down:** card kebab (and later composer) → `FleetStore` looks up the
  `ActorHandle` by `(conn, session)` → `commands.send(SessionCommand::…)`
  (`Stop`/`Promote`/`Demote`/`Send`/`Sleep`).
- **`ActorOutcome`:** the same poller drains the outcome channel; `Parked` →
  card connection state (shell §5.4). Others (`SendLost`, `TransportChanged`,
  `PersistError`, …) are logged in the skeleton — their user-facing handling
  (send-recovery, etc.) lands with the focused surfaces.

---

## 4. Slot API & window recompose

- **Window** = `nav rail │ main area`; the main area recomposes between two
  states (shell §3).
- **Board state:** `nav rail │ board` — the board is an ordinal reflow grid of
  `SessionCard` views (shell §4.1).
- **Focused state:** `nav rail │ boards(shrunk) │ chat │ navigator │
  working-area`, where **chat / navigator / working-area are real but empty
  labeled slot containers** that the parallel surface authors target.

### 4.1 Navigation model (no global ESC)

Native harness TUIs run inside a terminal surface and the TUI-native toggle
design forwards raw input to the harness — **ESC must reach the harness**, so
there is **no global ESC→board binding**. Navigation is by card click, as a
toggle:

- Click a card → focus that session (recompose to focused state).
- In focused state (boards shrunk but visible): click a **different** card →
  switch focus to it; click the **currently-focused** card → toggle back to
  board state.
- `⌘\` collapses/expands the boards column (shell §7.1). `⌘D` deep-focus is
  deferred.

ESC stays **surface-local** (dismiss within a surface), never a global back.

### 4.2 `ContentTab` + `SessionAttach` (the terminal seam)

The working-area slot is a **single-tile, single-content mount** hosting one
`ContentTab`. The contract is deliberately **thin — the three certainties only**:
render into the tile, a `title`, and focus/blur. Illustrative shape (the exact
dispatch — a `Render` view held as `AnyView`/`Entity<T: ContentTab>` vs an
object-safe trait vs enum — is a **planning detail**; `-> impl IntoElement` below
is not object-safe and is shown only to convey intent):

```rust
// illustrative — dispatch mechanism decided at plan time
trait ContentTab {
    fn render(&mut self, cx: &mut Context<Self>) -> impl IntoElement;
    fn title(&self) -> SharedString;
    fn on_focus(&mut self, cx: &mut Context<Self>);
    fn on_blur(&mut self, cx: &mut Context<Self>);
}
```

The **`SessionAttach`** handle is what the parallel terminal workstream codes
against — WS identity + the terminal-notification stream (terminal bytes flow
**directly from the typed client's WS**, *not* through the reducer, §13.2):

```rust
struct SessionAttach {
    connection_id: ConnectionId,
    session_id: SessionId,
    terminal_notifs: async_channel::Receiver<TerminalNotif>, // TerminalPendingChanged / terminal.activity
}
```

A trivial **placeholder** `ContentTab` validates the mount now; the terminal
implements `ContentTab` and drops in when its workstream is ready.

**Deferred to the workspace fan-out:** splits / recursive tiles, the tab-bar,
launchers, singleton-vs-multi-instance + badge, preview tabs, content-state
persistence (shell §7.2/§8).

---

## 5. The board-cards proving surface

The card renders shell §5.1 chrome from the projection — **coarse summary only,
never a transcript**:

- status-colored icon tile + **wave** glow (derived urgency, §5.1 ladder),
- `<STATUS>`, `<Title>`, `<harness> · <model>`,
- **activity line** (active cards) — resolved by priority: blocker/wait ▸
  `todos.activeForm` ▸ in-flight tool (from `ScratchChanged`) ▸ blank (§5.2),
- `📁 repo ⑂ branch` rows (multi-repo aware),
- footer: host/runner pill · `~$spend` (cumulative `total_cost_usd`; `—` when
  `None`, never `$0.00`) · `ctx %` over the context-window bar,
- **connection-state takeover** of the status line (§5.4) from the reconnect
  lifecycle markers / `Parked`.

A couple of card-kebab actions are wired (e.g. Interrupt→`Stop`, Sleep→`Sleep`)
to exercise the command-down path.

### 5.1 Acceptance test — what the skeleton exists to prove

Drive **N synthetic sessions** each streaming independent `StreamUpdate`s and
assert:

1. each card reflects its own session's chrome;
2. a scalar change on session B repaints **only** B's card (instrument
   notify/paint counts);
3. a high-frequency `ScratchChanged` on B whose derived activity summary is
   **unchanged** repaints **nothing** (§3.6).

---

## 6. Dev feed & verification

- **Synthetic `FakeFleet`** (in `lens-ui`, test-support) — emits scripted
  `StreamUpdate`s for N sessions into the same channel types and accepts
  `SessionCommand`s. Powers hermetic `lens-ui` tests + fast layout iteration; it
  is how the §5.1 acceptance test is staged deterministically. No new
  abstraction — it produces the same `async_channel` the actor does.
- **Live-verify gate** — `lens-app` wired to the **real `FleetScheduler` +
  omnigent 0.5.1** (the exact path `lens-drive` already runs). Drive ≥2 warm
  sessions; confirm cards paint from real bytes and commands land. This is the
  acceptance gate before the skeleton is "done," mirroring lens-core's live-rider
  discipline.

---

## 7. Scope boundary (explicit YAGNI)

**In:** the `lens-app`/`lens-ui` split; `FleetStore` + `SessionCard` entities +
the per-session poller + the §3.5 routing contract + the §3.6 granularity gate;
board state + card chrome; focused-state empty slots + the §4.1 click-toggle
recompose; `ContentTab` + `SessionAttach` + placeholder tab; a minimal theme
token set; the synthetic feed + live-verify; the §5.1 N-card acceptance test.

**Out (owned by later slices):**

- transcript rendering & streaming markdown; the **full replica / disk
  `RowSource`** (D23) — *transcript fan-out*;
- workspace / diff / editor — *workspace fan-out*;
- terminal internals + WS data path — *the parallel terminal workstream* (plugs
  into `ContentTab`/`SessionAttach`);
- splits / launchers / preview tabs / content persistence — *workspace fan-out*;
- permissions/elicitation forms — *permissions fan-out*;
- Bridge inbox, global search, Canvas, Concierge, multi-board / groups / archive;
- the **REST-poll coarse-status path** for Slept / archived / non-warm cards
  (state-model §10). **Verified unbuilt in both layers** (`FleetScheduler` manages
  warm actors only; `list_sessions` in `lens-core` is a *disk* control-store read,
  not a REST poll). Its owner is the **board/fleet continuation of this skeleton**
  ("board v2"), and it carries an **unbuilt lens-core/lens-client dependency**
  (periodic `GET /v1/sessions` → coarse 3-state → non-warm cards). A poll-fed card
  *stub* is explicitly rejected: it would mean faking an unbuilt cross-layer
  mechanism. **Skeleton board = warm/active sessions only.**
- multi-server rollup (`FleetStore` is keyed by `(ConnectionId, SessionId)` so it
  is *not precluded*, but the skeleton runs one connection).

---

## 8. Testing strategy

- **Hermetic `lens-ui` unit/integration tests** over `FakeFleet`: the §5.1
  acceptance assertions (independent cards, single-card repaint, no-op
  `ScratchChanged`), plus card-chrome rendering per `StreamUpdate` variant and
  the command-down path.
- **Live-verify** (§6) against real `FleetScheduler` + omnigent 0.5.1 as the
  acceptance gate.
- Gate: `cargo clippy --workspace --all-targets -- -D warnings` + `fmt` clean
  (AGENTS.md), tests green.

---

## 9. Open / deferred (tracked, not blocking)

- **`⌘D` deep-focus**, `⌘\` boards-collapse polish — shell §7.1, fold in when the
  focused surfaces land.
- **Multi-server / connection badge** on the card (shell §5.4) — data path is
  keyed for it; render deferred.
- **Send-recovery / `SendLost` UX** — belongs with the composer (focused surface).
- **Board v2** (the REST-poll path, Slept/archived/groups/multi-board) — the
  named continuation that owns the §7 poll deferral.
