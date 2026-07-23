# Lens — spec gaps backlog

Deferred design areas surfaced 2026-07-13 that have **no (or partial) spec coverage**.
Each is an *independent* subsystem — it gets its own spec → plan → implementation
cycle, not one mega-spec. This file is the parking lot so we don't forget them;
pick one, brainstorm it to a design doc, then strike it here.

Ordering below is by "blocks shipping Lens to a second human" (roughly).

## Ranked backlog

1. **App release / signing / update** — *zero coverage; biggest true void.*
   Code-signing, notarization, DMG/pkg packaging, auto-update (Sparkle or
   equivalent), release channels, **Lens app versioning** (distinct from the
   omnigent contract pin), crash reporting. Blocks any distribution beyond
   `cargo run`.

2. **Bundle omnigent into the `.app`** — *partial; coupled to #1.*
   Supervision itself is designed (`server-lifecycle.md` §3: hermetic `uv` env,
   supervise, recovery, contract gate). The gap is **shipping omnigent inside the
   signed bundle** so first-run needs no network and no system Python — today it's
   `uv tool install omnigent==…`, which assumes network + working uv and conflicts
   with notarization/gatekeeper + offline first-run. Solve alongside #1.

3. **Observability of Lens itself** — logging, crash reporting, a
   user-exportable diagnostics bundle. Prereq for anyone but the author filing a
   useful bug.

4. **Secrets / credential lifecycle** — Keychain is used ad hoc today (cursor SDK
   key, remote-connection tokens, harness API keys). No spec owns *where* these
   live, how they're scoped per-server/per-harness, and rotation. `E. Auth &
   multi-user` (README) resolves the *posture*, not the credential storage
   lifecycle.

5. **TUI-native harness handling** — ✅ **SPEC WRITTEN 2026-07-14**
   ([`docs/specs/2026-07-14-tui-native-toggle-design.md`](specs/2026-07-14-tui-native-toggle-design.md),
   commit `bf72ea3`; brainstormed → live spike → dual cross-family review →
   reworked). Blocked on build deps (Plan 7 terminal WS client + `lens-ui`
   viewport). Original gap framing below. — reading **(A)**: `claude-native` /
   `cursor-native` are PTY/TUI-only and **fold reasoning** into `output_text`
   (memory `live-event-recapture-findings`), so they can't produce a clean
   rendered reasoning stream. **Decision (2026-07-13):** the focused chat for a
   TUI-native harness offers a **per-session toggle** between the *rendered
   stream* and the *raw TUI*. Ties into the deferred **terminal WS attach
   (Plan 7)** in `workspace-and-terminals.md` and the two status/usage vocab
   normalizations. Needs its own spec: toggle placement, TUI surface (reuse the
   terminal WS attach surface?), how state/quiescence is tracked when the user is
   in raw-TUI mode.

6. **Onboarding / first-run product UX** — `server-lifecycle.md` §10 covers the
   first-run *backend*; the *product* empty-state ("no servers, no sessions —
   connect your first omnigent") is unspecified.

7. **Settings / preferences surface** — the STATUS "Tunables" (auto-sleep
   threshold, poll cadence, terminal scrollback/fleet-memory budgets, transcript truncation tiers,
   `cost_samples` cadence) have no UI home. Where the user sees/changes them.
   - **Known requirement — TUI-by-default global** (from the TUI-native toggle
     spec, 2026-07-13): a global preference "prefer the raw TUI when a harness
     offers one." The TUI-native toggle spec deliberately keeps per-session
     current-view as *runtime-only* (always initializes to rendered); the durable
     "I live in the TUI" default lives here as a global, not per-session disk
     persistence. When honored, a fresh session materialization of a TUI-native
     harness initializes to TUI (riding the same `starting TUI…` pending state)
     instead of rendered.
   - **Known requirement — font registry** (deferred here from the terminal
     workstream, 2026-07-16). Monospace font *selection* (terminal + chat code
     blocks) belongs to a runtime **font registry** owned by lens-app: enumerate
     `{system-resolvable} ∪ {optional bundled defaults} ∪ {user-supplied file}`,
     let the user pick, hand the chosen family name down (e.g. through
     `TerminalOpenOptions`). Until this lands, consumers use a **system monospace
     (`Menlo`)** referenced by name — zero bundling, guaranteed on macOS, grid-
     aligns. A `lens-fonts` bundle crate is **not** built now: without a registry
     it has nothing to do (system fonts resolve by name for free; the spike's
     alignment failure was a *missing* font, not the lack of a good system one).
     Bundling a brand default (e.g. JetBrains Mono, OFL) and Nerd-Font/powerline
     symbol fallback are decided *inside* this registry work, not before it.

8. **Data lifecycle / migration** — the two-tier SQLite store has a
   schema-version degrade path, but no app-level story for data location, backup,
   export, or "nuke and re-sync."

9. **Multi-machine identity** — two Lens instances (laptop + desktop) pointing at
   the same remote omnigent: independent replicas, or any Lens-side sync? Decide
   the posture even if the answer is "independent, no sync."

---

## Cross-repo seams (agreements to keep in sync, not backlog)

- **lens-ui ↔ lens-terminal integration seam** *(agreed 2026-07-14 during a grill
  of the lens-ui shell-skeleton design; recorded on both sides).* Direction is
  **lens-ui depends on lens-terminal** and *hosts* its tab — lens-ui is
  deliberately **not** a dependency of this workstream (§ this doc, "lens-ui is
  not a dependency"). Consequences both docs commit to:
  - **lens-terminal exports the constructor**
    `open(TerminalTarget, Arc<Client>, TerminalOpenOptions, cx) -> Entity<TerminalTab>`
    and public `TerminalTarget::{Existing { session_id, terminal_id },
    OpenOrCreate { session_id, key }}` plus
    `AccessIntent::{Automatic, ReadOnly}`. These values leak no Ghostty or
    transport types. `open` returns immediately in `Starting` and builds its
    own `TerminalAttachment` asynchronously.
  - **lens-ui owns routing and policy**: it chooses the logical target, resolves
    `ConnectionId → Arc<Client>`, supplies access intent/preferences, calls
    `open(...)`, and **wraps** the returned `Entity<TerminalTab>` in a lens-ui
    `ContentTab` adapter (lens-terminal cannot implement lens-ui's `ContentTab`
    because there is no dependency edge that way). It performs no terminal
    REST/WS work.
  - The host seam is one typed inbound `TerminalHostEvent` stream and one typed
    outbound `TerminalEvent` stream. Presentation updates atomically expose
    identity/reported title, lifecycle, effective access, and progress. Host
    requests cover user-gesture URL opens, permissioned OSC 52 clipboard writes,
    and background notifications. `TerminalTab::focus_handle(cx)` is direct,
    not a callback. There is no generic `RequestClose`.
  - Native `/clear` has no public terminal-transfer operation. `lens-ui` handles
    public `session.superseded`, then sends the typed supersession host event so
    the tab reattaches the same terminal under the target session. Lens never
    calls omnigent's schema-hidden internal transfer route.
  - lens-ui does **not** publish any attach type. An earlier lens-ui
    `SessionAttach { …, attach: TerminalAttachCapability }` sketch was **dropped**
    (wrong shape: no such capability exists, and it omitted `TerminalId`/access
    mode). If the `open(...)`/target shape changes here, update lens-ui §5.2.

---

## Upstream contract gaps (omnigent-side asks)

- **Immutable terminal generation identity** — omnigent 0.5.1 derives terminal
  IDs from `(terminal_name, session_key)` and may recreate a few server-owned
  terminal roles on attach while reusing that ID. It emits another live
  `session.resource.created` and normally persists a corresponding
  `ResourceEventData` item for reconnect discovery, but supplies no generation
  token and persistence is best-effort. Lens preflights GET and (via the
  Slice-4 live-stream generation guard) treats an **observed** duplicate
  creation / delete as a replacement, but cannot prove the remaining race away.
  Omnigent should expose an immutable generation/resource ID (or an equivalent
  durable replacement discriminator).
  - **Slice-4 status (2026-07-22):** the module guard correlates all *observed*
    `session.resource.created`/`.deleted` signals correctly (delete→create =
    adopt fresh engine; delete-alone = `ReplacementWaiting`/`Detached`). The
    **reconnect-path** "full" guard (resource-event-history consultation on
    preflight) is **NOT implemented** — `preflight_reconnect` only GETs
    existence. Residual bug: a *missed* live `deleted` + a *non-`4404`* retryable
    WS close retains the old engine → stale scrollback (active viewport stays
    correct; `output_gap` marker shown). Bounded because the common server-side
    teardown closes **`4404`** (`ws_bridge.py:80-83`) → clean `Detached`, no mix.
    **Practical narrowing at Slice 5** (the `FleetStore`'s persistent host
    session-event subscription shrinks the missed-live-delete window); **true
    closure needs this upstream token.** Do not claim reconnect recovers it.
    (Memory `terminal-resource-event-granularity`.)
  - **Slice-4 whole-branch review carry → Slice-5 integration (2026-07-22):**
    the bridge close (WS `4404`) and the host resource-signal are two views of
    the same agent reset arriving on **independent transports**, so their
    ordering is a race. Slice 4 fixed the *clobber* direction (a late bridge
    close can no longer overwrite `ReplacementWaiting`/`Sleeping` —
    `apply_bridge_event` is gated in those states). The **unfixed** direction is
    **`4404`-first**: on an `OpenOrCreate` reset where the WS `4404` lands
    *before* the `resource.deleted`, the tab goes `Detached(TerminalGone)` and a
    subsequent delete→create is ignored (no re-adopt from `Detached`) → the
    successor is not adopted. Not resolvable at the module level (a `4404` alone
    can't distinguish reset from genuine deletion, and speculatively waiting
    would violate the spec's "unexplained disappearance → `Detached`" for real
    deletions). Resolve when the **real bridge↔host event model** is designed in
    Slice 5 (FleetStore forwards resource signals + owns the WS lifecycle) —
    e.g. an `OpenOrCreate` tab that observes a positive reset while `Detached`
    could re-arm adoption, or the host could order/serialize the two channels.
    In Slice-4 (deterministic demo/tests) there is no real race, so demo
    adoption works. Do NOT claim `4404`-first adoption works in production.
10. **Keyboard shortcuts + macOS app menu** — *surfaced 2026-07-16 during the
    theming demo (Cmd+Q dead; ⌘. was silently focus-dependent).* gpui apps get
    **no standard macOS menu/shortcuts for free**: `Cmd+Q` (quit), `Cmd+W`,
    `Cmd+H`, `Cmd+M`, About, etc. are all dead until an app menu is wired via
    gpui's menu API. Today only two app-specific globals exist
    (`crates/lens-ui/src/shortcuts.rs`: `cmd-.` BackToBoard, `cmd-shift-t`
    ReloadTheme — the seed of this module). Needs: (a) the standard macOS app menu
    (Quit/Close/Hide/Minimize/About), (b) a coherent app-wide shortcut map (new
    session, switch/cycle session, focus composer/terminal, back, reload, maybe a
    command palette), (c) a single owner for keybinding + handler registration.
    **Hard rule** (learned the hard way — memory `gpui-global-vs-element-actions`):
    app-global commands MUST be `cx.on_action` globals, never element-level
    `.on_action`, which silently drop the keystroke when nothing in their subtree
    is focused. Small, self-contained; no omnigent dependency.

11. **Lens-owned MCP producer layer** — *surfaced 2026-07-16 designing the two new
    wave states (`docs/specs/2026-07-16-wave-states-scheduled-awaitingreview-design.md`).*
    Lens exposes its own MCP server to agents: `await_review` (ask a human to review
    a Canvas artifact), `schedule_wake` (park-until-T) + the wake-firing **scheduler**
    (Lens sends a message at T; also drives the `Ready`-style repaint timer in the
    poller so the `Scheduled` wave self-clears), **board control**, **messaging**, and
    **knowledge-base** tools. This layer is the **producer** for the `Scheduled` and
    `AwaitingReview` waves — the wave-side presentation contract is already locked (the
    spec above), but nothing sets the `scheduled_wake_at` / `awaiting_review` card
    fields outside `--demo` until this ships.
    - **`await_review` mechanics (decided):** **non-blocking** — a blocking MCP call
      would time out. The tool posts the review request to Lens and **returns control
      to the agent, who ends its turn** (session settles into the `AwaitingReview`
      wave). The human reviews the Canvas and submits comments, which flow back as a
      prompt message via **MessageCenter** (a SessionStart hook *or* a second MCP tool
      — Lens posts a "You've got Mail" message), and that return path **clears**
      `awaiting_review`.
    - **⚠ OPEN RISK (load-bearing):** a **remote** agent (managed host / omnigent
      server) must reach an MCP server running on the **user's local Mac**. If that
      transport doesn't work, the whole Lens-owned-signal model (both new waves +
      board/messaging/KB tools) needs a different shape. Resolve this **first** — it
      gates the layer. (The wave-side contract does not depend on it; `--demo` sets
      the fields directly, so that slice proceeds regardless.)
    - **Scheduling ownership:** built **Lens-owned** ("A"); a future omnigent
      `scheduled_until` forward ("B", sibling to the `client-message-id` ask) would
      populate the same source-agnostic `scheduled_wake_at` field with no
      `derive_wave` change. Native harness `/loop`/`ScheduleWakeup` are **invisible**
      (not forwarded) and out of scope until B.

## Board (§4) implementation specs

The board's **behavior** is resolved in `application-shell-and-layout.md` §4
(ordinal slots, recursive Card|Group tree, adaptive count-aware packing,
Lens-local persistence, movement, multiple boards, archive) — but the
**implementation** is un-designed: `BoardLayout` is a named placeholder in the
state model (`app-architecture-and-state-model.md:1067`), never a concrete type,
and the current `crates/lens-ui/src/board/mod.rs` is a flat `session_id`-sorted
flex-wrap grid with no groups, scroll, or persistence. Card chrome (§5) shipped
(waves B1–B5); these gaps are the remaining **board-level** (§4) work.

Decomposed 2026-07-18 (brainstorm) into six cohesive specs, each its own
brainstorm→spec→plan→build cycle. **This supersedes the old "B6/B7/B8" framing**
in STATUS — B7 "stable ordinal ordering" dissolves into B-1's ordinal slots (no
separate sort task). Order below is dependency order.

- **B-1 — Board data model & persistence (`BoardLayout`)** — ✅ **SPEC WRITTEN
  2026-07-18** ([`docs/specs/2026-07-18-board-data-model-persistence-design.md`](specs/2026-07-18-board-data-model-persistence-design.md),
  user-approved). *keystone; lens-core.*
  The concrete recursive **Board→(Card | Group)** tree; **ordinal-slot**
  representation (§4.1, index-within-parent, never pixels); Lens-local **SQLite
  schema + migration** (§4.2 — persisted in the state-model store, not a server
  entity); mutation ops (create/rename/archive board & group, move item to slot,
  reparent, ungroup); **where a new/polled session lands** (placement policy for
  sessions appearing via the §10 list-poll or created outside Lens); and the
  **auto-seed grouping rule** (session `workspace` project-dir → default Group,
  since group membership is Lens-owned, not `card.workspace`). Foundation every
  other B-spec reads/writes. Consumes the existing coarse `SummaryUpdate` feed
  (FleetStore/ActorFeed, already shipped) via a `group_of(&SessionCard)` seam.

- **B-2 — Adaptive packing & scroll (the layout engine)** — *lens-ui/gpui;
  §20's "one real spike."* The §4.3 **count-aware balanced packing** algorithm
  (pure, deterministic: 1→centered, 3→row, 4→2×2, 6→3×2 …, **never a lonely
  stretched row** — this is the fix for the mockup's rigid auto-fill grid, which
  is **not** faithful to §4.3); the gpui board element; the **scroll container**;
  off-screen **viewport culling** + **the anim-gate-on-scroll fix** (the STATUS
  hazard: today's `recover_viewport_gates_on_reentry` is edge-triggered on the
  focus↔board mode switch, so a card scrolling into view — no mode change —
  never resets its gate → frozen spinner; memory `viewport-reentry-freeze`).
  Partly rewrites the current `board/mod.rs` flat grid. Depends on B-1's tree.

- **B-3 — Group rendering & aggregation** — *lens-ui.* The group visual
  (colored border + faint color-matched body tint + header: name · aggregate
  spend · card count · age · collapse · ⋯ · ＋ quick-add); the **rollups**
  (spend from `cumulative_cost`, count, age, "N done" peek); persisted **collapse**
  state (via B-1). The mockup (`board-home.html` `.gwrap`) is the pixel ref for
  group chrome. Depends on B-1, B-2.

- **B-4 — Movement & grouping interaction** — *lens-ui.* §4.5 drag-to-reorder
  (ordinal snap), drag in/out of groups & nested groups, create-group gestures
  (drag card onto card · "New group" · right-click "New group from selection"),
  context-menu moves (Move-to-group ▸, Move-to-board ▸, New-group, Pin, ungroup,
  archive group), ⌘1–⌘9 card-jump. Mutates B-1. Depends on B-1, B-2.
  - **Op-wiring state (surfaced 2026-07-23, B-4c grill).** `BoardStore` exposes
    ~9 persisted mutations; the replica wires only `Load` / `PlaceSessions` /
    `SetCollapsed` (+ `MoveItem` in B-4c). The remaining group ops are **dead
    plumbing** — domain + store built and round-trip to SQLite, but no `Op` and no
    gesture — and split by the B-4a §8 non-idempotent-retry seam:
    - **Idempotent (absolute-value field write; shippable anytime like
      `SetCollapsed`, no seam):** `rename`, `set_color`. These can be pulled
      forward independently of B-4d.
    - **Non-idempotent (row insert/delete; gated on the §8 commit-phase seam
      because transient-retry re-enqueues the whole op):** `create_group`,
      `ungroup`, `archive`.
    B-4d owns wiring the non-idempotent tier + the context-menu triggers.

- **B-5 — Multiple boards + rail switcher** — *lens-ui + state.* §4.4 board-as-
  bounded + spin-up-new; nav-rail board entries (§6); ⌘⇧1–⌘⇧9 / ⌘K board switch;
  move-across-boards (drag onto a rail board / Move-to-board ▸); board CRUD.
  Depends on B-1, B-4.

- **B-6 — Archive-as-board** — *lens-ui + state.* §4.6 nav-rail Archive
  destination rendered with the **same recursive board UI** (archived groups
  represent themselves for free); scope filter (this board / all) + search +
  **restore-to-origin**; the group inline "Completed (N)" peek deep-links here.
  Mirrors the server `archived` flag (distinct from Sleep — state model §3).
  Depends on B-1, B-3.

**Seams (referenced, not folded in):** group **default new-session config**
(§4.2 / §7.6 quick-add) belongs to the new-session-dialog surface (agent-
definition seam), cross-referenced from B-1/B-3, not absorbed. The **coarse
card-summary feed** (§9 `SummaryUpdate`) already exists.

## Session & card lifecycle UI (create / end a session)

*Surfaced 2026-07-23 during the B-4c drag grill.* A board card is a projection two
levels down — omnigent server session → `FleetStore` `SessionCard` (via
`fleet/poller.rs`) → board card row (via `PlaceSessions`/reconcile). The lifecycle
write surface splits by layer, and the top layer is **genuinely unbuilt**:

- **Create / end a session → app-shell + omnigent, NOT the board.** "New agent"
  means telling the *server* (via omnigent) to create a session; the poller
  observes it and the card auto-appears. Ending is symmetric (server ends it →
  poller sees it vanish → card pruned). **This is the load-bearing gap: there is
  no new-session or end-session gesture anywhere.** Today's only session-creation
  path is `FakeFleet::spawn_session` (demo/tests); `FleetStore::{wake_session,
  retry_session}` are stubs (`TODO(state-model P3+)`). You cannot start or stop an
  agent from the UI. Ties into #6 (onboarding empty-state) and #10 ("new session"
  shortcut), but neither owns the gesture + server round-trip.
- **Board cards are add-only in production.** `BoardStore::{remove_session,
  archive}` exist and persist to SQLite but are **never called outside tests** (the
  `replica.rs:889/933` callers are the test `CountingStore`). Live card removal is
  reload-driven only (read-time tombstone-prune on `Load`), not gesture-driven.
- **Archive / pin / move a card → a board gesture** (B-4d op-wiring; surfaced on
  the B-6 archive-as-board destination). The only lifecycle mutation that
  legitimately belongs to the board write path.
- **Rename a card → n/a by design** — a card label is session-derived; domain
  `rename` is group-only (`board.rs:492` → `NotAGroup` for a card). Renaming a
  *session* would again be an app-shell/omnigent concern.

## Parked contract dependencies (omnigent-side asks)

- **LSP-proxy endpoint — gates any IDE-grade (band-3) file editing** (recorded
  2026-07-14, framework §4.4). The File-tab editor is scoped to a "comfortable
  editor" (top of band 2b: highlight/find-replace/multi-cursor/fold, all local).
  Band-3 intelligence (completions, diagnostics, go-to-def) is **blocked, not
  deferred-by-effort**: Lens is a pure REST/SSE/WS client and the worktree lives
  on the omnigent host, so a language server would have to run host-side with an
  LSP-proxy protocol over the wire — which omnigent does not expose. Unblocking
  band 3 needs **either** an omnigent LSP-proxy contract (sibling to the
  `client-message-id` ask) **or** a deliberate break of the pure-client boundary
  to run local language servers against *local-only* worktrees. Both are separate,
  larger decisions; neither is an editor-widget problem. Not scheduled.

## Cross-spec risks discovered during design

- **lens-core drops `session.superseded`'s redirect target — blocks terminal
  supersession reattach** (found 2026-07-15, grill of `docs/specs/2026-07-14-lens-ui-shell-skeleton-design.md`
  §5.2). The terminal workstream (`lens-terminal-ws`) delegates to lens-ui:
  observe public `session.superseded` and feed `target_conversation_id` into the
  terminal tab so it reattaches the same terminal under the new conversation
  (native `/clear` supersession; there is no public transfer route). But the
  reducer folds `SessionEvent::Superseded { .. }` to **nothing** — marker-only,
  `crates/lens-core/src/reduce/folds.rs:136` — so `target_conversation_id` never
  reaches the feed. Fix (terminal-integration era, **not** the lens-ui skeleton):
  lens-core must surface it, e.g. `StreamUpdate::Superseded { target_conversation_id,
  reason }`. Transient / live-only / no-replay in the 0.5.1 contract, so the
  durable `message`-item notice (persisted on the old conversation) is the
  separate reload path. Owner: whoever lands the terminal-integration slice;
  flagged to the terminal agent so they don't assume observation is free.

- **Polymorphic `ContentTab` mount protocol deferred — owned by terminal-UI-integration**
  (decided 2026-07-21, transcript T-2 brainstorm). The shell mounts any surface into a
  slot via the concrete `TabHandle` (`AnyView` + title + focus, `crates/lens-ui/src/slot/mod.rs:8`)
  built by a per-surface `*_tab` factory. The sibling `ContentTab` trait
  (`slot/mod.rs:6`) is an **inert marker** — no methods, nothing bounds on it, and the
  mount boundary erases the concrete type to `AnyView` anyway, so it does zero work.
  A *real* polymorphic protocol (shared surface lifecycle: on_close / on_blur, command
  routing, save-state) is **intentionally not built yet**: it needs a *second* concrete
  UI surface to design against, and `terminal-ws` ships `lens-terminal::open` (an engine)
  **without** UI integration. Building it now would bind a cross-layer seam against one
  concrete side — the [[premature-layer-boundary-binding]] failure. T-2 (the first real
  surface) standardized on `TabHandle` + `focused_transcript_tab(...)` and left the marker
  untouched. **Promotion** (add methods to `ContentTab`; stop erasing to bare `AnyView`,
  or hang closures on `TabHandle`) is a **small refactor** — impl the new trait for the
  two existing factories — owned by **whoever lands terminal UI integration**, with
  transcript + terminal as the two implementors. If terminal UI integration never lands,
  no protocol is ever needed. Design note also annotated at
  `application-shell-and-layout.md` §7.2. Owner: terminal-UI-integration slice.

- **Permissions spec — mode-change elicitations are TUI-only for native harnesses**
  (found 2026-07-14 spike, `docs/spikes/2026-07-14-tui-native-elicitation.md`).
  For `claude-native`, generic tool permissions round-trip fine from Lens's
  rendered `/resolve` path, but the **mode-change class** (e.g. `ExitPlanMode`
  "run in auto mode") **cannot be resolved from the API** — it structurally
  requires the harness TUI. The existing `permissions-and-elicitations.md` spec
  must (a) detect this class and route the user to the TUI toggle (or offer only
  round-trippable options) instead of a dead-end approve button, and (b) treat
  approval as pending until `elicitation_resolved`, never optimistic. Candidate
  omnigent bug report (like the client-message-id ask). The TUI-native toggle is
  the escape hatch this relies on.
