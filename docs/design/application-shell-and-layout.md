# Application shell & layout

The containers and chrome that everything else docks into: the board/home,
the focused-session window (chat + collapsible working area beside chat, ⌘D
deep-focus), the resource-rail navigator, global search/nav/palette, app-state
chrome, Bridge, the Canvas surface, Concierge chrome, and theme.

This document owns the **shell** — the containers and the navigation. The
surface documents (transcript, workspace, agent definition, permissions,
sub-agent topology, server lifecycle) own the *content that fills the slots*,
written container-agnostic.

**Status:** Draft, 2026-06-23. Grounded in the capability map's design-
language section.
**Depends on:** the application architecture & state model document
(reads `AppState`, `SessionState`, `BridgeRouter`, `ConnectionApp`).
**Feeds:** every surface document — they dock into the slots defined here.

Framework-neutral; the framework document owns the gpui lock and any
substrate-specific pieces. Framework divergence points are isolated in §17.

---

## 1. Scope & load-bearing decisions

**This document owns:**

- **Window anatomy** — the zones and how they recompose between the board and a
  focused session (§3).
- **Board** — the recursive tree of Card | Group; adaptive layout; grouping;
  movement; multiple boards (§4).
- **The card** — anatomy, the status/urgency wave, the activity line, the
  context menu, connection state (§5).
- **The global nav rail** — destinations, the health dot, Archive (§6).
- **The focused-session window** — chat-primary layout, the shrinking boards
  column, the working area (tab+split), header, composer, persistence,
  multi-window (§7).
- **Working area & launchers** — tab-bar launcher clusters, the navigator-vs-
  content split, singleton vs multi-instance surfaces (§8).
- **Search** — global modal, the persistent session search, the keyboard model,
  `Search(scope, container)` (§9).
- **Bridge** — the collapsed surface (Inbox + Log + Knowledge sub-panes),
  `Bridge(scope, container)`. Capture → Inbox → Concierge-files-to-Knowledge.
  §10 owns the structure.
- **The Canvas surface** — agent-presented visual elements (§12).
- **The Concierge chrome** — the chief-of-staff agent's persistent surface
  (§13).
- **The volatile session tray** — Tasks/Changes/Terminals above the composer
  (§14).
- **Header & composer** — the slim status bar and the composer action hub (§15).
- **The annotation engine** — comment-anywhere → send-to-agent, shared across
  Review / Plan Viewer / file-comments / Canvas / any surface (§16).
- **App-state chrome** — connection/health (multi-server), onboarding, error
  surfacing (§17).
- **Theme** — first-class, file-based, semantic tokens (§18).

**This document does NOT own:**

- The wire/reconnect/SSE taxonomy (the typed client); domain model, reducer,
  persistence, command flow, liveness (the state model); *how a transcript item
  renders* (the transcript doc); workspace/terminal/file **data** (the
  workspace doc); agent registry / harness / model controls **content** (the
  agent-definition doc); permission/elicitation **lifecycle and widget** (the
  permissions doc); sub-agent **topology semantics** (the sub-agent topology
  doc); server/runner **bootstrap & supervision** (the server lifecycle doc); the
  framework choice (the framework doc).

### Load-bearing decisions

| # | Decision | Section |
|---|----------|---------|
| ① | Positions are **ordinal slots**, never pixels — deterministic, reflowable | §4.1 |
| ② | A **Board** is a recursive tree of **Card \| Group**; boards/groups are Lens-local | §4.2 |
| ③ | The card's **whole-card wave** encodes urgency on a corrected ladder | §5.1 |
| ④ | **Left rail = destinations** (Boards · Bridge · Archive · Settings); no separate right rail | §6, §8 |
| ⑤ | **Navigators** (Files/Search) = a panel + tab-bar toggles; **content** = working-area tabs | §8 |
| ⑥ | **Global search = modal; session search = persistent** — one result surface | §9 |
| ⑦ | **`Bridge(scope, container)`** and **`Search(scope, container)`** — one parameterized surface each | §9, §10 |
| ⑧ | **One annotation engine** across all surfaces (comment-anywhere → send-to-agent) | §16 |
| ⑨ | Health is **per-connection, rolled up** — Lens connects to **N servers** | §17.1 |
| ⑩ | **Theming is first-class & file-based** | §18 |
| ⑪ | **Bridge is a left-rail destination** (decision H, capability map §0.7-H) | §11 |
| ⑫ | **Canvas is a side-pane tab** per session (agent-presented visuals) | §12 |

**Core principle (recurs throughout):** *one parameterized, responsive component
mounted in multiple containers.* The board card, `Bridge(scope, container)`,
`Search(scope, container)`, and the annotation engine are all instances.

---

## 2. Terms

- **Board / home** — the default view: groups of cards.
- **Group** — a user-defined container of items (a project/task), Lens-local;
  may carry a default new-session config.
- **Item (board)** — a **Card** or a **Group**. Groups nest; loose cards allowed.
- **Card** — the coarse, glanceable representation of one **session**. Never the
  live renderer (the transcript doc owns that).
- **Ordinal slot** — a card/group's position expressed as an index within its
  parent, *not* x/y pixels.
- **Focused session** — a session opened into the focused-session window.
- **Working area** — the tab+split surface that holds content (files, Review,
  terminals, web, Bridge, Plan, Canvas).
- **Navigator panel** — the shared single-slot panel hosting Files or Search,
  left of the working area.
- **Connection** — one omnigent server Lens is attached to (local · remote ·
  managed-sandbox host). A session belongs to one connection.

---

## 3. Window anatomy

The window recomposes between two states; both share the **left nav rail** and
a single **main area** that the rail routes.

**Board state** (no session focused):

```
[ nav rail ] [ main area: the active board (cards + groups) ]
```

**Focused-session state:**

```
[ nav rail ] [ boards (shrunk, stays visible) ] [ chat ] [ navigator panel? ] [ working area ]
```

- **Nav rail** (left, slim, icon-only, **hover-expands** labels) routes the main
  area. §6.
- **Boards stay visible (shrunk)** when a session is focused, for ambient
  awareness (the waves) and click-to-switch-focus; collapsible (⌘\) for a
  full-width session. The shrink reflows cleanly *because positions are ordinal*
  (§4.1) — cards landing wrong on a narrow board is gone.
- **Chat** is the always-present primary column of a focused session (capability
  map decision F).
- **Working area** is the tab+split content surface; the **navigator panel**
  (Files/Search) docks to its left when toggled (§8).
- There is **no separate right icon-rail** — launchers live on the working-area
  tab bar (§8.1). Multi-window: any destination or session can **⤢ detach** to
  its own window (decision G); a detached window is just another state-model
  subscriber. **Detach *moves* a conversation** — it is shown in **≤1 window**;
  navigating to an already-detached conversation **raises its window** rather than
  cloning it (single-instance-per-document).

---

## 4. Board

### 4.1 Ordinal slots, not pixels

A card or group's position is **its index within its parent**, never x/y. This
is the keystone fix for cards drifting on relaunch, auto-placement guessing
wrong, and the layout breaking when the board is narrowed by a focused
session. With ordinal slots, **relaunch is deterministic and resize is a pure
reflow** — "move it" means drag it to a new slot, where it **snaps and stays**.

### 4.2 The recursive board

A **Board** holds an ordered list of **items**; an item is a **Card** or a
**Group**; a **Group** holds items (cards or sub-groups) — **arbitrarily
nested**. **Loose cards** may sit at any level alongside groups. No forced
foldering.

Boards and groups are **Lens-local** — pure client-side organization, persisted
in the state model's SQLite store, **not server entities**. The server only
sees sessions; you move cards between groups/boards freely with zero server
effect. A **group may carry a default new-session config** (agent · repos ·
host · branch-policy) — this powers the frictionless group quick-add (§7.6).

### 4.3 Adaptive layout

Card chrome is **identical at every width**; only the *arrangement* adapts to
the item count — a **count-aware balanced packing** (1 → centered; 3 → row;
4 → 2×2; 6 → 3×2 …), never a lonely stretched row. The spec fixes this as
**behavior** (deterministic, count-aware, ordinal-slotted); the exact packing
algorithm is an implementation detail.

### 4.4 Multiple boards

A board is bounded; when it gets crowded you spin up another. Boards live in
the nav rail (§6). **⌘⇧1–⌘⇧9 switch to board N** (separate modifier from
⌘1-9 card-jump so they never compete); ⌘K also switches boards (§9).

### 4.5 Grouping & movement

- **Create a group:** drag a card onto another, or an explicit "New group", or
  right-click → "New group from selection".
- **Move within a board:** drag to reorder (ordinal snap); drag in/out of groups
  and nested groups.
- **Move across boards:** drag a card onto a board in the rail, or right-click →
  **Move to board ▸**.
- **Group affordances:** header (name · aggregate spend · count · age · collapse ·
  ⋯), **＋ quick-add session**, rename, collapse, nest, ungroup, **archive
  group**, move group.

### 4.6 Archive

Archive mirrors the server `archived` flag and **hides** a card/group from the
default board/listing. It is visibility and organization, not resource
lifecycle; Sleep is the action that closes Lens-local observation and sends
best-effort `stop_session` (state model §3). Archive is a **nav-rail
destination** (§6) rendered with the *same recursive board UI* — so **archived
groups represent themselves for free**. A scope filter (this board / all) +
search + **restore-to-origin**. A group's inline "Completed (N)" peek is a
glance that deep-links into Archive filtered to that group. UI may offer a
composed "Archive and Sleep" command, but plain Archive is the server flag.

---

## 5. The card

### 5.1 Anatomy & the wave

The card is the atomic board unit and a **coarse session summary** — never the
live transcript renderer.

```
[ whole-card glow = the wave ]
┌────┐  <STATUS>            (uppercase, status-colored)
│ ⟳  │  <Title>
└────┘  <harness> · <model>
<activity line>                          (active cards only)
📁 <repo> ⑂ <branch>                     (one row per repo — multi-repo aware)
[host] ~$spend                    <ctx %>
▓▓▓░░░░░░░░░░░░░░░░░░░░░░░░░░░░░          (context-window progress bar)
```

Reference render: `docs/design/renders/board-home.html`.

- **A status-colored icon tile** (left) — a glyph per wave state (↻ Working,
  ☾ Slept, ✓ Ready, 🔔 Needs-input, ! Failed) on a tinted square; the fastest
  glanceable status read. *(Build detail: ships a proper status + harness-provider
  icon set; the renders use unicode placeholders.)*
- **`<harness> · <model>`** — the harness is shown (Lens spans 16; native
  harnesses get a small `TUI` tag), then the model.
- **Activity line** (active cards only) — the "why does it need you / what's it
  doing" glance (`⏸ approve: prod migration`, `✎ refactoring auth`,
  `▸ 3 children · 1 busy`); §5.2.
- **`📁 repo ⑂ branch` rows — one per repo** (multi-repo aware; the branch in a
  status-tinted mono). Replaces the older "no cwd" rule: the repo name (not an
  absolute path) + branch is more informative, and multi-repo sessions show a
  row each. **No message snippet.**
- **Footer** — a host/runner pill (`local` · `arca` · `sandbox`), `~$spend`
  (cumulative, server `total_cost_usd`), and `<ctx %>`, over a **context-window
  progress bar** (status-colored fill).
- The **wave** is the whole-card glow; **color + pulse-rate encode urgency** on
  the **corrected ladder** (a Working agent is busy and does *not* need you →
  calm; Ready wants you; Needs-input is blocked on you). The full **5** session
  statuses (`idle / launching / running / waiting / failed`) come **only from the
  SSE stream of Active sessions**; the REST poll that feeds Slept and
  non-active archived cards is **coarse 3-state** (`idle / running / failed` —
  `waiting` collapses to `running`, `launching` to `idle`). So a poll-fed card
  uses the persisted last fine-grained status (state model §2.2) rather than
  regressing to `idle`. The wave is **derived** from the status +
  `pending_elicitations` count + the Lens lifecycle state:

| Wave | Treatment | Urgency | Derived from |
|--------|-----------|---------|---------|
| **Needs-input** | fast pulse, orange | highest | `pending_elicitations` non-empty (own or mirrored from a child; the wire field is a list / `pending_elicitations_count`). **Sticky** until acted on; **overrides Slept dimming** (the poll still carries the count, so a slept card with a pending approval still glows orange). *(Orange, not red — the reference render `board-home.html` is the pixel SSOT and uses orange here; it keeps Needs-input distinct from Failed-red.)* |
| **Ready** | steady, blue | high | `status == idle` **and an unacknowledged turn completion** — there's a result to look at. Sticky until you focus/view it, then neutral |
| **Working** | calm shimmer, green | medium | `status ∈ {running, launching, waiting}` with no pending elicitation (`launching` shows "starting…"; `waiting` = parked on its own async/sub-agent work — busy, doesn't need you) |
| **Slept** | dimmed card + **Resume** | lowest | Lens lifecycle: local observation closed and best-effort `stop_session` sent after quiescence (§ state model 3); card stays visible, dimmed |
| **Failed** | steady, red + **Retry** | ≈ Ready (rare) | `status == failed` / `last_task_error` |

> **"Scheduled" is reserved but not in v1** — there is no scheduled/cron session
> status in omnigent `0.5.1` (`queued` turns are momentary). The `status.scheduled`
> token stays reserved in the theme for a future "agent on a loop" state; the v1
> ladder is the five rows above.

### 5.2 The activity line

Replaces the snippet; on **active cards only**. The agent's current focus or
blocker, resolved by **priority**: ① **blocker/wait** (waiting on a subagent ·
a long-running tool · for Needs-input, the pending approval) ▸ ② **current
task** = `session.todos.activeForm` (task-level, e.g. "Refactoring auth
module" — *not* "editing auth.rs") ▸ ③ **in-flight tool** (fallback) ▸ ④
blank. Grounded in the API; degrades gracefully.

### 5.3 Context menu (= header kebab)

`Open · Open-in-window │ Interrupt · Sleep · Fork │ Move-to-board ▸ ·
Move-to-group ▸ · New-group · Pin │ Rename · Archive · **Stop session** ·
**Delete…** │ Copy id · Share link · Export · Reveal workspace · **Switch
agent ▸**`. State-dependent: Slept → **Resume**, Failed → **Retry**. The four
"remove-ish" actions are distinct (state model §3): **Sleep** (after strict
quiescence, close Lens observation, best-effort `stop_session`, **dim, stays
visible**; also the auto action after ~10-min quiet) / **Archive** (PATCH server
`archived=true`, **hide** in the drawer; manual housekeeping) / **Stop session**
(the explicit user-facing server stop for a still-visible session, owner-only) /
**Delete** (remove server-side + local record, confirms). **Switch agent ▸** opens the
agent-definition picker for a live handoff (state model §12.2; capability map
decision J) — **disabled while the session is busy (running)** and **hidden for
sub-agent sessions** (the server rejects those: 409 / 400).

### 5.4 Connection state on the card

Per-session connection trouble **takes over the card's status line** (you
can't trust "Working" while disconnected): amber **↻ Reconnecting**
(transient) / red **⚠ Runner offline** (stalled). There is **no per-session
connection dot in the header and no chat banner** — the card is always visible
(board or shrunk column), so it is the home for per-session status. Global
health is the rail dot (§17.1).

The card carries a **connection badge** (e.g. "Local", "Internal dev",
"Sandbox") to distinguish which omnigent server owns the session — visible at
a glance when you have N connections.

> **Seam → sub-agent topology:** a short-lived sub-agent surfaces as a line in
> the parent's activity line ("⧗ waiting · subagent X") / the Terminals-style
> tray, **not its own card**. Full sub-agent topology is the sub-agent
> topology document.

---

## 6. The global nav rail

Slim, **icon-only, hover-expands**. Two behaviors live here, cleanly split
from the working-area launchers (§8):

- **Destinations (route the main area):** **Boards** (the switcher — each
  board is a rail entry; no redundant "Home") · **Bridge** · **Archive** ·
  **Settings**, plus a **health dot** (§17.1) pinned at the bottom.
- **Boards, Bridge, and Archive replace the main area** (Archive is
  board-*like*). **Bridge** opens as a **shrinking working-area tab**
  (§10) — a deliberate per-item difference: board-level views replace; content
  opens a tab.
- **Search is *not* on the rail** — it's a modal / navigator (§9).

Any destination can **⤢ detach to its own window** (decision G).

**The Concierge chrome** lives in the rail, with a small avatar + an "ask
Concierge" quick-input (§13).

---

## 7. The focused-session window

### 7.1 Composition

`nav rail │ boards (shrunk) │ chat (primary) │ [navigator panel] │ working
area`. The boards column **stays visible** (ambient awareness) and **collapses
(⌘\)** for full-width. Chat is always present (capability map decision F). A
⌘D deep-focus mode (capability map §0.6) is a third state: hide chat AND the
boards column, maximize the working area; second press restores both. Useful
for review-heavy supervision.

### 7.2 The working area

A **tab + split** surface (standard editor-group paradigm: tabs within tiles,
recursive splits). Several tiles allowed; resizable. Content tabs are **peer
tabs** — a file beside a terminal beside a diff beside a Canvas beside a web
view. See §8 for the launcher taxonomy.

> **Implementation note (mount seam).** The code seam for mounting any surface
> into a slot is the concrete `TabHandle` (`AnyView` + title + focus) built by a
> per-surface `*_tab` factory; the `ContentTab` trait is an **inert marker**
> today. A *polymorphic* content-tab protocol (shared lifecycle: on_close/on_blur,
> command routing) is **deliberately deferred** until a second real UI surface
> exists to design against — owned by the terminal-UI-integration slice. Tracked
> in `docs/SPEC-GAPS.md` → "Cross-spec risks discovered during design." (Transcript
> **T-2** standardized on `TabHandle` + factory as the first real surface.)

### 7.3 Persistence extends to content

The state model's persistence covers not just *which* tabs/tiles are open but
their **content state** (within reason): unsaved file buffers, diff/terminal
scroll position, terminal scrollback (the **Lens-side ring buffer** — the
server keeps none). Reopen a session and your layout + working state are
restored.

### 7.4 The session header (slim status bar)

Left = **status · title · connection badge · host · live cost**. Right =
**Share · ⤢ detach · ✕ close · ⋯ kebab**. (No per-session connection dot —
connection trouble shows on the **card**, §5.4.) It does **not** repeat
model/branch/workspace (those are on the card, visible beside the focused
session) and does **not** carry the context meter (that's on the composer).

Also carries **presence/co-viewers** — the `session.presence` data from the
state model: a small "X, Y also viewing" chip in the header (permissions doc
owns the ownership affordances, e.g. "you don't own this session" when there's
a different owner).

### 7.5 The composer (action hub)

The composer carries the actionable controls: multiline input + **attachments**
(multimodal) + slash `/` + bang `!` + `@`-mentions, plus the relocated
turn/session controls — **interrupt/stop** (the send button morphs while
streaming), **fork**, **compact**, **retry**, **model & reasoning-effort**
selectors, **collaboration-mode** selector (0.2.0 codex-native Plan), and the
**context meter** (paired with Compact). Rare lifecycle actions (rename ·
stop-session · archive · copy-id · export · switch-agent) live in the **kebab
+ right-click-on-card**, not the composer. (Retry also appears inline on an
errored response — transcript doc.)

### 7.6 New-session / create flow

- **One-screen dialog** for the net-new flow: agent (the agent-definition
  picker) · host/connection · **repos** (＋ add-repo → multi-repo) · advanced
  (model/effort, collapsed) · target board+group · **host_type: external vs.
  managed** (capability map §0.7-A) · Create.
- **Group quick-add** for the common in-project case: from a group's ＋,
  inherits the group's default config → just a branch + Create, no picker, no
  worktree/branch noise.
- **Launch points:** rail ＋ (new board/session), board ＋, ⌘K, and group ＋
  (quick-add). The **wizard was rejected** (extra clicks don't pay in a
  power-user app).
- **Repo mechanics are a workspace-doc seam** (§16): per-repo source ·
  base-off · new-branch · **sparse-checkout (paths)** · **worktree provider**
  (pluggable; default `git worktree`).

---

## 8. Working area & launchers

### 8.1 The tab-bar launchers

There is **no separate right icon-rail**. Launchers are **icons-only on the
working-area tab bar**, in three clusters, with open tabs to their right:

```
[ 🔎 Search · 📁 FileTree ]  |  [ 📓 Bridge · ⊟ Review ]  |  [ 🖥 Term⁺ · 🌐 Web⁺ · 📄 File⁺ · 🎨 Canvas⁺ ]   ‖   <tabs →>
```

### 8.2 Navigators vs content (the split)

- **Navigators** (🔎 Search · 📁 Files) toggle a **shared single-slot panel**
  left of the working area; you *browse* there and selecting opens **content**
  into a working-area tab. Files is a **multi-root tree** (decision A). Files is
  on-demand in the panel — not an always-on tree; ⌘P quick-opens.
- **Content** (📓 Bridge · ⊟ Review · 🖥 Terminal · 🌐 Web · 📄 File · Plan ·
  🎨 Canvas) are **working-area tabs**.

### 8.3 Singleton vs multi-instance

- **Singletons** (one per session; re-invoke = focus; **no + badge**): **Review**
  (aggregate diff, scroll-to-file) · **Bridge** (the session notebook tab) ·
  **Canvas** (the session's agent-presented visual surface — singleton per
  session; agents add content to it, you don't open multiple canvases for one
  session).
- **Multi-instance** (N tabs; **+ badge** = new tab): **Files · Terminal · Web
  · File** (you can open >1 file terminal / web view / file tab). The badge
  encodes the rule.

### 8.4 Preview tabs

Opening from a navigator/result uses a **reused preview tab** (single-click
previews and *replaces*; ⏎ or edit promotes to a permanent tab). This is what
lets you rip through many files against a stable result list (§9.3).

### 8.5 Routing

A tray **Change → Review** (scrolled to the hunk); **Files / ⌘P → Editor** (a
File tab — a **"comfortable editor," top of band 2b**: highlight, find/replace,
multi-cursor, folding; **no LSP/IDE intelligence**, which omnigent's contract
can't feed anyway. Tier + build owned by framework §4.4; edit-write path by
workspace §3). Source-control folds into Review + the Changes tray — there is no
separate ⑂ icon.

---

## 9. Search

### 9.1 The model

Search is `Search(scope, container)`. **Global search is a transient modal**
(it does not need to persist — selecting navigates and dismisses); the **only
persistent search surface is session content search**. This is what resolves
"two result surfaces is silly" — there is exactly one persistent one.

### 9.2 Keyboard model

- **⌘K** — quick-nav between sessions / boards / groups (navigation only).
- **⌘1–⌘9** — jump to card N on the active board; **⌘⇧1–⌘⇧9** — switch to board N.
- **⌘P** — session palette (this session: actions + file quick-open).
- **⌘⇧P** — global palette (app actions + the global-search modal).
- **⌘⇧F** — session content search (the persistent navigator panel).
- **⌘I** — Bridge entry / jump-to-next-needs-input (§11).
- **⌘⇧C** — floating Concierge chat panel (transient or 📌-pinned alongside your work; §13).
- **`` ^` ``** — toggle an interactive terminal in the focused session (its
  env/worktree); a working-area terminal tab (§8.1).

Scope is **contextual**: with no session focused, search is global only; with a
session focused, session content search is available (⌘⇧F).

### 9.3 Containers

- **Overlay** (⌘K/⌘P/⌘⇧P) — transient; top results inline; pick → navigate &
  close; Esc dismisses.
- **Navigator panel** (⌘⇧F) — the persistent session content search; results
  stay put while hits open as **preview tabs** in the working area (§8.4).
- **Global** = the modal; no persistent global surface.

---

## 10. Bridge — the collapsed surface

Bridge is the **single rail destination** that unifies the fleet-wide actionable
queue and the knowledge notebook into one surface. Three sub-panes, distinct
modes:

| Sub-pane | Mode | Carries | Entry |
|---|---|---|---|
| **Inbox** | action-oriented | pending elicitations, agent-to-agent relays, planning todos, deferred notes, quick-captured ⌘⇧N items; Allow/Deny/Cancel for elicitations, Resolve/Reply/Undefer/Send-to for others | ⌘⇧I opens Inbox filtered to "You / pending"; ⌘⇧N captures into Inbox |
| **Log** | read-only | chronological session log with day/week/month rollup summaries | the default landing sub-pane when opening Bridge from the rail |
| **Knowledge** | read-oriented | settled facts (Memories) + long-form pages (Wiki) | direct author or Concierge-files-here |

### 10.1 `Bridge(scope, container)`

One surface, built once: **scope ∈ {all · project · session}** filters the
data and sets the landing sub-pane; **container ∈ {working-area tab · window}**
sets density.
- **Global** (scope=all) — opened from the rail; a working-area tab that
  **shrinks the board** (not a board takeover). Resizable / ⤢ detachable for a
  deep read.
- **Per-project** — scoped to a board's project.
- **Per-session** — a working-area **singleton tab** (peer to Review), scope=
  session.

Bridge is **content** (full pages / feed), hence a working-area tab — *not* a
navigator panel.

### 10.2 Inbox — the actionable queue

```
[ filters bar: All · You · Projects · Agents · Deferred | badge counts ]
[ NEEDS YOU (N) ─────────────────────── ⌘I cycles ]   ← pinned band
[   pending elicitations only · Allow/Deny/Cancel    ]
[ Everything else ───────────────────────────────── ]
[   reverse-chron stream: relays · todos · notes …   ]
```

**Organization: a pinned "Needs you" band + a stream** (decision H, resolved
2026-06-24 mockup pass — `docs/design/renders/bridge-inbox.html`). Pending
elicitations (the things *blocking an agent*) pin to a top band that matches
⌘I's "act now" semantics — ⌘I cycles exactly this set. Everything else (relays,
planning todos, deferred notes) flows in a reverse-chron stream below. The
filter chips slice both zones.

Each item card:

- **Header** — `from → to` (e.g. "Concierge → Concierge", "You → Concierge",
  "Agent X → Agent Y", "System → You" for a pending elicitation). Timestamp.
  Status pill (`● pending` / `● delivered` / `DEFERRED`).
- **Body** — the rendered text + structured payload.
- **Actions row** — Resolve · Reply · Undefer · Discuss with [agent] · Send
  to… ; **for elicitations**: **Allow / Deny / Cancel** (the same verbs the
  composer docks for the focused-session case; the Inbox is the cross-fleet
  secondary surface).

**Inbox eligibility** — Inbox carries all form-mode elicitations (binary +
structured + mirrored child), agent-to-agent relays, planning todos, and
deferred/quick-captured notes. `url`-mode OAuth stays at the composer (one-shot
auth, not a queue item). See the permissions doc §5 for the `target_session_id`
mirror routing case.

### 10.3 Badge counts

The rail dot on the Bridge icon shows the **All** count (or the **You** count
if non-zero). **⌘I** focus-navigates to the next-needs-input agent (primary,
capability map §0.6); **⌘⇧I** opens the Inbox filtered to "You / pending"
(secondary). Counts come from the state model's `BridgeRouter.badge_counts`.

### 10.4 Capture → Inbox → Concierge-files-to-Knowledge

Quick-capture (⌘⇧N from anywhere) parks a **raw, context-stamped note** into the
per-project **Inbox** (under the "Deferred" / "Notes" filter). **There is no
manual promote**: items persist; the **Concierge retrieves them, discusses,
and files a proper memory into Knowledge when earned** (linking back to the
Inbox note). The Log records what happened regardless — automatic entries
written by the Concierge / the user / the system.

### 10.5 Cross-fleet routing

When you Reply or Send to… an agent, the Bridge uses
`POST /v1/sessions/{id}/comments/send` per the state model's routing fabric
(omnigent-labeled comments as the carrier). This is how an agent-to-agent
relay deliverable surfaces *inside the target session's transcript* as
structured feedback — the target agent sees it via its omnigent stream.

---

## 11. ⌘I — jump to next-needs-input agent

**⌘I** is the control-room primary navigation primitive (capability map
§0.6). It focus-navigates to the next session (cross-connection) with a
pending elicitation — *not* into the Bridge Inbox. The Bridge Inbox is the
secondary view of the same data; ⌘I is the one-keystroke "go to the agent
that needs me" primitive.

- **⌘I** — jump to the next session (cross-connection) with a pending
  elicitation; opens the focused-session window with the composer docked
  widget visible (Allow/Deny/Cancel).
- **⌘⇧I** — open the Bridge Inbox filtered to "You / pending" (the queue
  surface; useful for batch triage).

Both entry points read from the same `BridgeRouter` data in the state model;
⌘I is the "act now" focus-navigation, ⌘⇧I is the "see the queue" view.

---

## 12. The Canvas surface

The **drawing surface an agent can present visual elements in**: diagrams,
recorded interactions, custom visualizations. A **side-pane tab, singleton per
session** (§8.3). The name "canvas" is reserved for this surface — the board
overview is the *board*, never the canvas.

### 12.1 What's rendered

- **Agent-presented visuals** — the agent emits structured payloads (via an MCP
  tool Lens exposes locally) describing shapes, text, arrows, plot data, or
  embedded HTML/SVG; Lens renders natively.
- **Recorded interactions** — the agent can pin a transcript turn or a tool
  call's output to the canvas as a visual block — "this is what we tried,
  here's the diff it produced."
- **Custom visualizations** — provider-specific (web search summaries, code
  interpreter outputs, image generation) can dock into the Canvas alongside
  their transcript rendering.

### 12.2 The seam

The agent-definition document pins the MCP tool the agent calls to draw to
the Canvas; this document specifies the Canvas as a **rendering target** the
working area hosts. The contract is: agent → MCP payload → state model →
Canvas tab renders. Gemini-native ops like computer-use screen captures may
also render here when they don't fit the transcript.

---

## 13. The Concierge chrome

The **long-standing chief-of-staff agent** (state model §12.3). Because you
converse with it *about* your other sessions, its defining surface is one you
see **alongside** your work, not instead of it. What the shell adds:

- **The floating Concierge panel** — a **draggable, resizable, pinnable**
  floating chat panel (the Concierge's transcript + a mini-composer, reusing the
  transcript renderer — transcript §18) that overlays the main area. Popped by
  **⌘⇧C** (or the rail avatar / footer). Two modes on **one surface**:
  - **Transient (default)** — type → Enter posts to the Concierge → Esc
    dismisses; focus returns to your work session. The "quick — should I approve
    this?" case, *with the reply visible* (unlike a blind input).
  - **Pinned (📌)** — stays floating beside your focused work, showing the live
    Concierge conversation while you keep working. Remembers position/size.

  It does **not** steal the focused-session window or the keyboard focus
  (Esc returns to your work). **Fully navigating** into the Concierge session
  (normal nav) still gives it the full focused-session window for a deep
  sit-down — the floating panel is the *alongside* mode.
- **A small avatar in the rail** — the Concierge is pinned Active by default.
  Its status dot mirrors the Bridge's needs-attention badge (because
  the Concierge triages incoming items, its session status and the
  Bridge's pending count are correlated). Clicking it pops the floating panel.
- **Concierge-down affordances** — if the Concierge's session 404s, the avatar
  dims, and the rail surfaces a "Concierge offline" state; Lens re-creates it
  on next launch (state model §12.3).

The Concierge itself is an ordinary agent (`~/.omnigent/agents/concierge.yaml`)
— the agent-definition document owns its spec; this document owns only its
shell chrome.

---

## 14. The volatile session tray

Volatile, list-shaped session state lives in a **parked segmented bar above
the composer** — *not* a persistent side panel (which would steal working-area
and doesn't fit volatile content). Segments: **Tasks · Changes · Terminals ·
Sub-agents**, collapsed by default, each expands upward on demand.

- **Tasks** — the agent's `session.todos`: collapsed shows count + active
  task, hover-expands the list; on **completion** a task drops into the
  transcript as a timeline marker (completions only). Rendered inline in the
  chat per session.todos, NOT here as a full surface; this tray is a compact
  summary that deep-links into the transcript.
- **Changes** — the changed-file list (the workspace doc's `changes`, per
  git-root, multi-root aware); click → Review, scrolled.
- **Terminals** — running terminals; click → the terminal's working-area tab.
- **Sub-agents** — the focused parent's child sessions (decision B; sub-agent
  topology doc): collapsed shows "N children · M busy · K need you", expands to
  a small tree with per-child status pills; click a child → its own
  focused-session window (breadcrumb back). This is the **primary focused-parent
  home** for the sub-agent tree (only present when the session has children).

**No Todos panel.** There is no separate "Todos tab" (capability map §0.6); the
agent's live `session.todos` render inline in the chat
(conversation transcript doc) and surface as a summary here. Cross-session
**planning todos** live in the Bridge (state model §11), not here.

---

## 15. Header & composer

Covered in §7.4–§7.5. Summary: **header = a slim status readout**; **composer
= the action hub**; **rare lifecycle = kebab + card right-click**. The split
was an explicit redistribution — the card is already the identity readout,
actions belong by the input, and rare actions don't deserve permanent chrome.

---

## 16. The annotation engine

One engine: **comment-anywhere → send-to-agent.** A selection on a surface
opens an **anchored comment thread** (you + agent replies); **Send to agent**
bundles the comments as structured, anchored feedback → the agent revises → a
new version; addressed comments resolve.

- **Plan Viewer** — a working-area surface rendering **Markdown/HTML** with this
  annotation layer; replaces ">"-quoting for planning.
- It shares the **same engine** with **Review** (code diff) and **file-
  comments** (transcript doc's jtbd 5/9).
- **Universal annotation:** the engine is a **cross-surface primitive** — it
  should cover *all* working-area surfaces (terminal output regions, web views,
  **Canvas visuals**, anything), not just docs/diffs. Build once; reuse
  everywhere.

---

## 17. App-state chrome

### 17.1 Connection & health (multi-server)

A single Lens client connects to **N servers at once** — each a
`ConnectionApp` in the state model. **A session belongs to one connection**;
the card's connection/host pills reflect it. Therefore:

- The **left-rail dot is a roll-up**: green (all up) · amber (any degraded /
  reconnecting) · red (any down) · purple (any contract-mismatch).
- **Hover → a per-connection popover**: each server's status (up?, runner
  bound, active streams / cap, last event seq, contract version, sandbox
  provisioning state) + **＋ Add / Manage connections**.
- **Per-session** trouble is on the **card** (§5.4); **reconnect is
  transient** (the typed client's snapshot+items+reopen+dedup auto-recovers; a
  thin amber indicator only if it lingers); **runner-offline is loud +
  actionable** (Reconnect / Rebind — server-lifecycle doc).
- The **connections model is the server lifecycle document**; the health
  *surface* is here.

### 17.2 First-run / onboarding

**The local server is always-on baseline infrastructure** — Lens spawns and
supervises a local `omnigent server` on first run **regardless** of which
work-connections the user adds, because the **Concierge** can only live there
(state model §12.3; server lifecycle §3, §10). So first-run bootstraps the local
server, then = **"add your first *work* connection"**: a welcome that adds
**Local** (use the baseline local server for your work too), **Remote** (paste a
URL + an auth method — bearer / cookie / forwarded-email), or **Managed
sandbox** (managed-sandbox host provisioning — server lifecycle doc owns the
backend; this document owns the wizard). This is the *same* flow as the health
popover's **＋ Add connection**. Connected-but-empty → a friendly **＋ New
session** CTA; other empty states (no agents, empty group) follow the same
pattern. Bootstrap/supervision = the server lifecycle document; the welcome +
empty states = here.

### 17.3 Error surfacing (three altitudes)

- **Card** — flips to `✕ Failed` (board glance).
- **In-transcript** — a structured `response.error` block (source · code ·
  message) with inline **Retry / Edit & resend**; `response.retry` shows a
  quiet "retrying…" that resolves silently. *Most errors live here.*
- **App toast** — the rare **non-turn** error only (create-flow validation,
  auth expiry, unexpected server error).

Routing: turn error → transcript + card; transient retry → quiet inline;
non-turn → toast. Connection/runner trouble is the health system (§17.1), not
an "error" here.

### 17.4 Residency & notifications

For a control-room app whose whole value is "the agent needs you," Lens must
reach you when its window isn't in front. It does that by being a **resident
app**, not a foreground-only one.

- **Closing the window does not quit** (⌘W / red-button = hide to a **menu-bar
  presence**; **⌘Q** quits fully). Resident Lens keeps the always-on local
  server (server-lifecycle §3), the Concierge, and a **background poll** alive,
  so it can detect needs-attention while backgrounded. The menu-bar icon shows
  the aggregate **needs-you** count (mirrors the rail Bridge badge).
- **Native OS notifications** fire on a **needs-attention transition** — a
  session entering **Needs-input** (a pending elicitation, own or mirrored from
  a child; permissions §5). Content: `🔔 <session> needs you — <activity line>`
  (e.g. "payments api needs you — approve: prod migration"). Detection works
  even for **slept/remote** sessions because the background poll carries
  `pending_elicitations_count` (state model §10); a needs-input session is
  excluded from auto-sleep so its stream stays live (state model §3.2).
- **Deep-links** route a click back into the exact spot:
  `lens://session/{connection_id}/{session_id}` and
  `lens://elicitation/{connection_id}/{session_id}/{elicitation_id}` raise Lens
  and run `navigate_to_session` (+ dock the elicitation widget), or open the
  Bridge Inbox for batch triage.
- **What notifies is configurable** — **Needs-input** on by default; **Failed**
  and **Ready** optional (off by default, to avoid noise); per-connection mute,
  Do-Not-Disturb / quiet hours. A flood of simultaneous needs-input **coalesces**
  into one "N agents need you" notification (the Concierge can summarize rather
  than ping per item).
- **Fully quit (⌘Q):** no notifications until next launch — the Inbox shows
  everything pending on relaunch. True fire-when-quit needs a server-side push
  channel (APNs/webhook) omnigent doesn't expose; that's a **v2 reach** behind a
  clean seam (server-lifecycle owns the push channel if ever built), not v1.

---

## 18. Theme

**Theming is first-class and file-based.** A theme is a **native semantic-token
file** — `bg.*`, `text.*`, `border`, `accent`, and **`status.*`** (which drives
the card wave, statuses, and banners, so they stay consistent and themeable).

- **Default = dark-first** ("Lens Dark Deep"). Light is shipped via the same
  semantic tokens. A **Settings picker + hot-reload**; user themes drop into a
  themes dir.
- **Importers** map external formats → our tokens: **base16 first** (trivial,
  vast), **VS Code themes** as the marquee fast-follow, **terminal themes**
  (iTerm/Alacritty) → the terminal surface.
- Mechanics: a gpui `Theme` struct (semantic names, not raw hex at call sites).
  Compact density throughout.

---

## 19. Seams to other documents

| Document | This document provides (the home) | They own (the content/mechanics) |
|------|--------------------------|----------------------------------|
| **transcript** | chat column position, composer, the transcript's container | item rendering, streaming, the Review *content* |
| **workspace** | navigator panel, Changes/Terminals tray, Review tab, new-session repo rows | `changes`/`diff`/`search`/terminals, multi-root worktrees, worktree provider (base/branch/sparse/provider) |
| **agent-definition** | the new-session dialog shell, the model/effort/collaboration-mode controls' placement (composer), the switch-agent picker placement (card kebab) | agent registry/picker, harness, the controls themselves, the switch-agent bundle |
| **permissions** | where the elicitation/permission widget appears (in-transcript + attention) | elicitation lifecycle, the widget; reconcile with the transcript doc |
| **sub-agent topology** | a child's presence as a line in the parent's activity tray | sub-agent topology / hierarchy navigation |
| **server-lifecycle** | the connect flow UI, the health surface, onboarding | connections model, server/runner bootstrap & supervision |
| **state model (Bridge router)** | the Bridge surface, its filters/badges/keyboard entry | the router, the badge counts, the routing fabric |
| **state model (Concierge)** | the Concierge's rail avatar + quick-input | the Concierge's session, MCP tool surface, lifecycle |

---

## 20. Framework-divergent points

Collected so the rest of the spec stays framework-neutral.

- **The board** is the one real spike — ordinal slots + adaptive packing are
  framework-neutral *behavior*; the rendering substrate differs. gpui is a GPU
  canvas, well-suited; reference (gpui-flow) shows pan/zoom + viewport culling.
  **Note**: because positions are ordinal slots (not free-form x/y), the board
  is *simpler* than a free-form canvas — no drag physics, no zoom; just a
  responsive reflow grid of cards — Lens's board is bounded + ordinal, not a
  free-form/infinite canvas.
- **Multi-window** (decision G) — gpui is multi-window native.
- **Theme** — a gpui `Theme` struct; same token schema.
- **Channel → UI marshal** — per the state model, the single divergent point
  (gpui entity update vs alternative runtime's IPC hop).
- Everything else (layout behavior, the parameterized-surface principle, the
  annotation engine, search/Bridge scoping, Canvas) is
  substrate-independent.

---

## 21. Open / deferred

- **Light theme sequencing** — an §18 sequencing call, not a capability
  decision.
- **User-customizable accent** — cheap with tokens. §18-optional.
- **Cost / usage view** (decision I — chrome **resolved**) — **two-axis**:
  per-card and per-project (Group) show **cumulative** spend from the server's
  `total_cost_usd` (exact, no price table); a **global** top-bar readout shows
  **time-windowed** spend (today / 7d / 30d), Lens-computed from the
  `cost_samples` series (state model §6.2). "Today" = local calendar day;
  7d/30d rolling. **Behavior when `total_cost_usd` is `None`** (server hasn't
  priced the session): show `—`/"unpriced", never `$0.00`. **Still open
  (→ discussion):** the cross-connection global rollup algorithm + the
  `cost_samples` retention/aggregation policy (how long samples are kept, how
  windows are bucketed) — state-model §6.2 owns it; not yet pinned.
- **VS Code / terminal theme importers** — fast-follow after base16 (§18).
- **Sub-agent placement** beyond the activity-line line — sub-agent topology
  document.
- **Canvas MCP contract** — the tool the agent calls to draw to the Canvas;
  pinned by the agent-definition document.
- **Floating Concierge panel + multi-window** (§13) — whether a 📌-pinned panel
  floats over the main window only, follows focus across detached windows
  (decision G), or can itself detach. Default: per-main-window; refine at build.

---

## 22. Condensed decisions ledger

1. **Ordinal slots, not pixels** — deterministic relaunch, clean reflow (§4.1).
2. **Recursive board** (Card | Group, nested, loose cards); boards/groups
   **Lens-local** (§4.2).
3. **Adaptive count-aware packing**, identical card chrome at any width (§4.3).
4. **Card wave** on the corrected urgency ladder; **activity line** = blocker
   ▸ task ▸ tool ▸ blank (§5).
5. **Left rail = destinations** (Boards · Bridge · Archive ·
   Settings + health dot + Concierge avatar); these replace the main area;
   Bridge opens as a working-area tab with three sub-panes (Inbox / Log /
   Knowledge) (§6, §10).
6. **Focused window** = nav · boards (shrink/collapse/⌘D deep-focus) · chat ·
   navigator · working area; **no right rail** (§7, §8).
7. **Launchers on the tab bar**; **navigators = panel**, **content = tabs**;
   **singletons** (Review, Bridge, Canvas) vs **multi-instance** (Files,
   Terminal, Web, File, +badge); **preview tabs** (§8).
8. **Global search = modal; session search = persistent**; `Search(scope,
   container)`; ⌘K/⌘P/⌘⇧P/⌘⇧F; ⌘I jump-to-next-agent (§9, §11).
9. **Bridge = collapsed surface** (Inbox + Log + Knowledge sub-panes),
   `Bridge(scope, container)`, capture → Inbox → Concierge-files-to-
   Knowledge; the Concierge is the filer (§10, §13).
10. **Bridge = left-rail destination** (fleet-wide queue across all
    connections; ⌘I cycles through next-needs-input) (§11).
11. **Canvas = side-pane tab, singleton per session** (agent-presented visuals)
    (§12).
12. **Volatile tray** (Tasks/Changes/Terminals/Sub-agents) above the composer;
    the Sub-agents segment is the focused-parent home for the child tree
    (decision B); no Todos surface — agent todos render inline in chat,
    planning todos live in Bridge (§14).
13. **Header = slim status + presence/co-viewers; composer = action hub;
    lifecycle = kebab/right-click; switch-agent = card kebab ▸** (§7.4–§7.5,
    §15, §5.3).
14. **One annotation engine** (Plan Viewer / Review / file-comments / Canvas /
    universal) (§16).
15. **Per-connection health, rolled up; N servers**; transient reconnect; loud
    runner-offline; first-run = add-connection; errors at three altitudes;
    **resident menu-bar app + native needs-input notifications with
    `lens://` deep-links** (⌘W hides, ⌘Q quits) (§17).
16. **First-class file-based theming**; dark-first; base16 importer first
    (§18).
