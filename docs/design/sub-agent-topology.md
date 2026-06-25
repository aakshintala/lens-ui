# Sub-agent topology

The multi-agent model: child-session trees, the topology UI, and how a
parent surfaces its children. Greenfield in Lens — no inherited invariant
to fight.

**Status:** Draft, 2026-06-23. Written fresh against omnigent `0.3.0.dev0`.
**Depends on:** the state model (reads `SessionState.parent_session_id`,
`ChildSession`'s richer `ChildSessionSummary` mirror; the per-child
`SessionHandle`).
**Seams to:** the application shell (the rail/tree/list container + the
card's activity-line line + the volatile session tray), the transcript doc
(the in-transcript spawn span — transcript §8.6).

---

## 1. Scope & boundaries

**This document owns:**

- **The child-session model** — `ChildSessionSummary` mirror, the
  parent↔child relationship, child-stream ownership (§2).
- **The topology decision** (capability map §0.7-B) — rail/tree vs. flat
  list vs. hybrid; the recommendation + the render behavior (§3).
- **The parent's activity-line + tray surfacing** — how a short-lived child
  appears on the parent's card before drill-in (§4).
- **Deep-focus into a child** — navigating from a parent's spawn span to the
  child's own focused-session window (§5).
- **Pending-elicitation aggregation** — the `pending_elicitations_count`
  field on `ChildSessionSummary` + how it rolls up (§6).

**This document does NOT own:**

- The transcript's in-transcript spawn span (transcript doc §8.6) —
  collapsed card, live summary, output on completion.
- The card visual (the application shell — this document supplies the data
  for the activity line + tray + rail).
- The parent's own session state (the state model — this document reads +
  adds child refs).
- Elicitation handling for a child's mirrored elicitation (the permissions
  document — `target_session_id` mirror routing).

---

## 2. The child-session model

Children are real sessions — they have their own `/stream`, their own
`SessionState`, their own `SessionHandle` in the state-model registry. The
parent links to them via `SessionState.parent_session_id` (on the child)
and a `Vec<ChildSession>` on the parent (derived from
`session.child_session.updated` events).

```rust
pub struct ChildSession {
    pub id: SessionId,
    pub connection_id: ConnectionId,      // same as parent's
    pub parent_session_id: SessionId,
    pub title: Option<String>,            // tool/name
    pub tool: Option<String>,             // the tool that spawned this child
    pub session_name: Option<String>,
    pub kind: String,                     // "sub_agent"
    pub agent_id: AgentId,
    pub agent_name: Option<String>,
    pub current_task_id: Option<String>,
    pub current_task_status: Option<String>,
    pub busy: bool,
    pub labels: BTreeMap<String, String>,
    pub last_task_error: Option<ErrorInfo>,
    pub last_message_preview: Option<String>,
    pub pending_elicitations_count: u32,   // 0.2.0 — feeds badges
    pub created_at: i64,
    pub updated_at: i64,
}
```

`ChildSessionSummary` is **not exposed as a named schema in
`openapi.json` components** — only the event `SessionChildSessionUpdatedEvent`
is. The typed client hand-writes the mirror from
`omnigent/server/schemas.py:558` and the contract test pins the round-trip
(typed client §9 + §10).

`session.child_session.updated` carries a **PARTIAL** summary — live runner
deltas carry only the fields that changed (a status delta omits
`last_message_preview`; a preview delta carries only it); the **full** summary
arrives on snapshot / `GET …/child_sessions`. So the state model **merges
present fields over the cached child row** rather than replacing it (typed
client §3 caveat). `session.created` (child variant) handles live incremental
creation. The state model folds both into the parent's `Vec<ChildSession>`.

**Task-derived fields are commonly `None` in 0.3.0.** `agent_name`,
`current_task_id`, and `current_task_status` are recorded on the child's *latest
task* (`schemas.py:558` field docs: "`None` if no tasks exist"). Decision A
retired the standalone Task entity (task = session), so these frequently come
back `None`. The child-row label **must fall back** to `tool` → `title` →
`session_name` rather than assuming `agent_name`; never render an empty label
because `agent_name` was null.

**`GET …/child_sessions` is direct-children only.** It does not return
grandchildren. Any recursive rollup (§6) requires a multi-hop client fetch
(walk each child's own `child_sessions`), not a single call.

---

## 3. Topology decision (capability map §0.7-B)

**Option landscape:**

- **(i) Rail/tree** — a sub-agent rail/tree under the parent card with
  navigation into each child's stream.
- **(ii) Flat list** — surface children as an inspectable list only (the
  activity-line line + tray).
- **(iii) Hybrid** — summary list on the card, drill-in opens child's own
  focused-session view.

**Resolved (capability map §0.7-B): hybrid, with the tray "Sub-agents" segment
as the focused-parent home.**

Reasoning: 0.2.0's `ChildSessionSummary` is rich enough
(`pending_elicitations_count`, `current_task_status`, `last_message_preview`,
`agent_name`, `busy`) that a flat list underuses it. The tree surfaces each
child's status at a glance, and the "open ↗" navigation into a child's own
focused-session window is free (the child is a real session in the registry). It
scales to Polly-style orchestrators where a parent may have 5-10 children in
parallel worktrees.

**Three zones, each for a different context:**

1. **The parent card (unfocused glance)** — the activity line (shell §5.2) shows
   a **compact rollup**: "⧗ waiting · subagent X" or "▶ N children · M busy · K
   need you" as a priority-1 blocker when the parent is blocked on a child.
   This is a glance, **not an expandable rail on the board** (a board tile is too
   small); clicking focuses the parent (opening the tray below). **Children
   never get their own board cards.**
2. **The volatile tray "Sub-agents" segment (focused-parent — the PRIMARY
   home)** — when the parent is focused, this segment (shell §14) is the home for
   the child tree: collapsed shows "N children · M busy · K need you", expands to
   a small tree with per-child status pills (busy / pending-elicitation /
   current task). Clicking a child opens its focused-session window.
3. **Drill-in** — `open ↗` on a child opens its own focused-session window.
   The child is a full session: transcript, workspace, composer, etc. — all
   present. The breadcrumb at the top shows "‹ parent › / child" so the user
   can navigate back.

**Scalability:** the tree is bounded — a parent with many children (or deep
nesting) collapses to a "N children · M busy" chip with a **popover or side-pane
escalation** for the full list. The exact threshold is an implementation detail;
the spec pins the tray-segment home.

---

## 4. The parent's activity-line + tray surfacing

Per the priority order (shell §5.2):

1. **Blocker/wait** — if the parent is `Waiting` on a sub-agent, the activity
   line reads `⧗ waiting · subagent X` (or `… N subagents`). Clicking focuses
   the parent and expands the tray's "Sub-agents" segment.
2. **Current task** — otherwise `session.todos.activeForm`.
3. **In-flight tool** — fallback.
4. **Blank** — idle.

This is the **primary glanceable surfacing** — the user supervising the
fleet sees "this parent is blocked on a child" without opening anything.
The tray segment is the secondary surfacing — the user looking at the
focused parent sees the live child statuses above the composer.

---

## 5. Deep-focus into a child

The child is a first-class session: navigating to it opens its own focused-
session window (shell §7). `navigate_to_session(connection_id, child_id)` is
the state-model's unified navigation funnel — no special path. The child has
its own transcript, workspace, composer, terminals, etc.

**Breadcrumb** — the focused-session header shows "‹ parent name › / child
name" so the user can navigate back to the parent. Back is a one-key affordance
(e.g. ⌘[) — the shell's navigation keyboard model.

**Sub-agent depth** — a child can itself spawn children (transcript §8.6's
depth-1 peek generalizes); the breadcrumb shows "‹ root › / mid › leaf".
Lens doesn't cap depth (the server does if it does); the rail/tree visual is
recursive.

---

## 6. Pending-elicitation aggregation

0.2.0 `pending_elicitations_count` on `ChildSessionSummary` rolls up:

- A **child's `pending_elicitations_count`** drives a badge on the child's
  row in the tray tree ("⚠ 2 pending").
- The **parent's activity line** promotes to `⧗ waiting · subagent X needs
  you` when any direct child has `pending_elicitations_count > 0`.
- **Recursive rollup** — a grandchild's pending elicitation propagates up via
  the parent's rollup of its children's statuses. The state model owns the
  rolling fold; this document owns how it renders. **Caveat:**
  `GET …/child_sessions` returns *direct children only*, so the rollup is a
  client-side multi-hop walk (fetch each child's children), not a server-provided
  recursive count. For deep trees this is a fan-out fetch the liveness layer must
  bound.

**Plural pending elicitations on the parent stream.** A fan-out parent mirrors
**multiple** child prompts onto its own stream, each keyed by `target_session_id`
(integration test `test_two_children_elicitations_isolated_on_parent_stream`).
The parent's pending state is therefore a `Vec`/map, not a single `Option` — the
composer docks one focused prompt at a time while the card/Bridge badge shows the
count, and resolve routes by `target_session_id` to the correct child.

**`target_session_id` mirror routing** — when a child's elicitation mirrors
into the parent's stream (permissions doc §5), the parent's Bridge
badge + the transcript's pending-approval marker both surface "from
sub-agent X" — identifying the source. The parent can resolve it from its
own composer (the typed client routes the resolve via
`POST /v1/sessions/{target_session_id}/elicitations/{id}/resolve`).

---

## 7. Open questions

- **Tray-segment visual tuning** — the exact affordance (always-expanded tree
  for N≤3, chip + popover/side-pane escalation for N>3 or deep nesting) is a
  shell call at first build.
- **Multi-depth breadcrumb performance** — deep trees shouldn't blow the
  breadcrumb; a depth-N collapse is an implementation detail.
- **Child tombstones** — when a child session completes and is cleaned up
  server-side, its card should remain visible (as a completed entry in the
  parent's tray tree) until the parent's turn completes. The state model's
  registry keeps a tombstone for the child until the parent settles; this
  document specs the visual.