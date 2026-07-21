# Design — B-1: Board data model & persistence (`BoardLayout`)

**Date:** 2026-07-18
**Status:** design approved (brainstorm), pending user review of this doc
**Scope:** the **data model & persistence** for the board only — the concrete
recursive Board→(Card | Group) tree, ordinal-slot representation, the SQLite
schema + mutation ops, and the placement/lifecycle rules. This is **B-1 of the
six-way §4 board decomposition** (see `docs/SPEC-GAPS.md` → "Board (§4)
implementation specs"). It is the keystone every other B-spec reads/writes.

**Explicitly NOT in scope** (own specs): adaptive packing + scroll + viewport
culling (B-2), group rendering + cost/count/age aggregation (B-3), drag/movement
+ context-menu grouping (B-4), multiple boards + rail switcher (B-5),
archive-as-board surface (B-6). This spec provides the *types and storage* those
consume; it does not render or mutate-via-gesture.

**Grounds:** `application-shell-and-layout.md` §4 (§4.1 ordinal slots, §4.2
recursive Lens-local tree, §4.5 movement, §4.6 archive) and
`app-architecture-and-state-model.md` §9 — this spec **defines the `BoardLayout`
placeholder** (`app-architecture-and-state-model.md:1067`) as a concrete type and
its persistence in the control-tier SQLite store (state model §6.2).

**Supersedes:** the STATUS "B6/B7/B8" framing. B7 "stable ordinal ordering" is
**not a sort** — it is §4.1 ordinal slots, defined here. There is no separate
ordering task.

---

## 1. The core principle: placement ≠ content

The board splits into two orthogonal concerns that this spec deliberately keeps
apart:

- **Placement** — *where* a card or group sits: the recursive tree, ordinal
  slots, group membership, collapse/archive flags. **Lens-local, user-owned,
  persisted here** as `BoardLayout`.
- **Content** — *what a card shows*: status, title, cost, waves, activity line.
  Lives in `SessionCard` (lens-ui `FleetStore`), fed by the coarse
  `SummaryUpdate` feed (state model §9); persisted per-session in the `sessions`
  table. **Not owned by this spec.**

`BoardView` renders by walking `BoardLayout`'s tree and, for each card item,
looking up that session's `SessionCard` in `FleetStore`. **A card item stores no
session data — only a reference** (`connection_id` + `session_id`). This is the
invariant that keeps board state small and mutation-cheap: dragging a card is a
placement write, never a content copy.

### Consequence: grouping is 100% user-driven

There is **no derivation of groups from `workspace`** (the session's project-dir
path). `workspace` is display-only card content. Group membership is created
solely by user action (B-4). A fresh fleet is a **flat list of loose cards**;
groups appear only when the user makes them. (Decision: "loose cards until
grouped", 2026-07-18 brainstorm.) The earlier `group_of(&SessionCard)` seed-seam
is **deleted** — it does not exist in this model.

---

## 2. Entity model

- **Board** — a top-level container that appears as a nav-rail entry (§4.4). One
  **default board** is seeded on first run; creating more is B-5. A board holds
  an ordered list of items at its root.
- **Item** — a node within a board, one of:
  - **Card** — a placement of exactly one session. Carries only the reference
    `(connection_id, session_id)`.
  - **Group** — a named container of items (cards or sub-groups), **arbitrarily
    nested** (§4.2). Carries name, color, collapse + archive flags, and a
    reserved default-config slot (§7.6 seam).
- **Ordinal slot** — an item's position is its **integer index within its
  parent** (§4.1), never x/y pixels. Loose cards and groups interleave freely at
  any level ("no forced foldering", §4.2).

```
Board "Main"
├─ Card(session A)                 ordinal 0   (loose)
├─ Group "auth work"               ordinal 1
│  ├─ Card(session B)              ordinal 0
│  └─ Card(session C)              ordinal 1
└─ Card(session D)                 ordinal 2   (loose)
```

---

## 3. Schema (control tier — `lens.db`)

Board layout is **cross-connection** (a board may hold cards from multiple
omnigent servers; focus is cross-connection per state model §9), so it lives in
the **single control DB** (`lens.db`), not a per-session transcript file. Added
to `CONTROL_DDL`; **`SCHEMA_VERSION` bumps 2 → 3** (additive — see §6).

```sql
CREATE TABLE boards (
  id         TEXT PRIMARY KEY,
  name       TEXT NOT NULL,
  ordinal    INTEGER NOT NULL,          -- order among boards in the rail
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

CREATE TABLE board_items (
  item_id        TEXT PRIMARY KEY,
  board_id       TEXT NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
  parent_item_id TEXT REFERENCES board_items(item_id) ON DELETE CASCADE,  -- NULL = board root
  ordinal        INTEGER NOT NULL,       -- index within parent (§4.1)
  kind           TEXT NOT NULL,          -- 'card' | 'group'

  -- card only:
  session_conn_id TEXT,
  session_id      TEXT,

  -- group only:
  group_name   TEXT,
  color_token  TEXT,                     -- stable border/tint color (B-3 owns palette/picker)
  collapsed    INTEGER NOT NULL DEFAULT 0,
  archived     INTEGER NOT NULL DEFAULT 0,   -- Lens-local group archive (§4.6)
  group_config TEXT,                     -- reserved for §7.6 group quick-add; NULL in B-1

  created_at   INTEGER NOT NULL
);

-- exactly one placement per session across all boards:
CREATE UNIQUE INDEX board_items_session
  ON board_items(session_conn_id, session_id) WHERE kind = 'card';

CREATE INDEX board_items_parent ON board_items(board_id, parent_item_id, ordinal);
```

**Why an adjacency tree, not a JSON blob.** It preserves the store's existing
"stable, denormalized read contract" (schema.rs) — other subsystems can query it
(e.g. "which board is session X on"). Reparent is a single
`UPDATE parent_item_id, ordinal`. A nested-JSON board would force a whole-blob
rewrite on every drag and be opaque to relational readers. Boards are bounded
(§4.4), so the tree is small and joins are cheap.

**`color_token` is stored, not derived by index** — so a group keeps its color
across reflows and reorders. B-3 owns the palette and the picker; B-1 just
persists the chosen token (nullable → B-3 assigns a default on group creation).

---

## 4. Placement & lifecycle

**Ordinal management.** Dense integers, renumbered among siblings on
insert/move/remove. Boards are bounded, so a full sibling renumber is cheap;
sparse/fractional ordinals are an unneeded optimization and are deferred.

**Session discovery → placement.** A session becomes known via the §10 list-poll,
a live stream, in-Lens creation, or a fork/share created outside Lens. On first
sight (no `board_items` card row for its `(conn, session)`), it is **appended as
a loose card to the default board root** at the next ordinal.
- In-Lens creation via the new-session dialog *may* name a target board+group
  (§7.6) — that target overrides the default append. The dialog itself is the
  agent-definition/§7.6 seam, not B-1; B-1 exposes `place_session(conn, session,
  target)` with `target` defaulting to "default board root".

**Session removal.**
- **Delete** (§5.3 — remove server-side + local record): remove the card item.
- **Tombstone** (`sessions.tombstoned_at`): treat as delete for placement — drop
  the card item.

**Archive vs Sleep vs Delete** (state model §3; §4.6):
- **Archive** (server `archived=true`, and/or Lens-local group `archived`): the
  card item **persists**; it is hidden by the archive filter at render (B-6), and
  its cost still rolls into the group total (§5). This is the routine
  "finished — delete the worktree, archive the card" path; **cost is retained**.
- **Sleep**: pure content/lifecycle state (dimmed, stays visible) — **no
  placement change**.

**Group operations** (mutation ops here; the *gestures* are B-4):
- **create_group(board, parent, ordinal, name)** → insert a group item.
- **move_item(item, new_parent, new_ordinal)** → reparent + renumber both
  sibling sets. Guards: a group cannot move into its own descendant (cycle).
- **ungroup(group)** → children reparent to the group's parent starting at the
  group's slot; the group row is deleted (§4.5).
- **archive(item)** / **rename(item, name)** / **set_collapsed(group, bool)** /
  **set_color(group, token)**.

**Startup reconcile.** On load, any session present in the `sessions` table (or
arriving via poll) that lacks a card item is placed (loose, default board). This
is also the **upgrade path**: after the v2→v3 migration, existing sessions have
no `board_items` rows and are lazily placed on first load/poll — no backfill
migration needed.

**Empty groups** are allowed to persist (the user may be mid-arrangement).
Whether to auto-prune a group that becomes empty is a **B-3/B-4 render/interaction
policy**, not a storage rule.

---

## 5. Cost is derived, never stored here

`board_items` carries **no cost column**. A group's aggregate spend is a pure sum,
computed at render (B-3): walk the group's card items → read each session's
`cumulative_cost` from `FleetStore` (server `total_cost_usd`, exact, no price
table — §0.7-I) → sum. Storing the aggregate would be a denormalized cache
requiring invalidation on every poll/stream cost tick, for no benefit, and would
break placement≠content.

Per-session cost is already persisted as content (`sessions.cumulative_cost`,
`cost_json`, `usage_by_model`) and time-sampled (`cost_samples`, feeding the §21
global today/7d/30d readout). None of that moves here.

**Deletion-retention preference (noted, deferred to the cost view / §21).** Group
cost sums *extant* members. **Archived** cards keep their row → **cost retained**
(matches the user's routine workflow: delete worktree → archive card → cost still
counts, feeds the "N done" pill). Only a true **Delete** drops a session's
historical spend from the group. The user's preference is that **lifetime project
spend should ideally survive session deletion** — but that requires retaining cost
independent of session existence, which is a cost-view decision (§21 retention/
aggregation is still open). B-1 stays derived-from-members; §21 owns whether to
persist spend across deletion.

---

## 6. Persistence architecture

- **lens-core `BoardStore`** (peer to `ControlStore`): opens against `lens.db`,
  loads the full tree at startup, and exposes typed mutation ops
  (`place_session`, `remove_session`, `create_group`, `move_item`, `rename`,
  `archive`, `set_collapsed`, `set_color`, `ungroup`, board CRUD). **Every
  mutation writes through to SQLite** in the same call (no dirty-flush queue;
  boards are small and edits are user-paced).
- **No background actor.** Unlike sessions (which stream via an off-thread
  actor, state model §3), board layout is not streamed — it changes only on user
  action. It is a plain persisted struct, loaded once, mutated in place.
- **lens-ui replica**: `BoardLayout` is held in a gpui `Entity` (mirroring how
  `FleetStore` relates to the store today). UI mutations call `BoardStore` ops
  (which persist) and `cx.notify()`; `BoardView` observes and re-renders. The
  gpui-side tree is the render replica; SQLite is canonical.

**Migration (v2 → v3).** Additive: `CREATE TABLE boards`, `CREATE TABLE
board_items` + indexes. No existing data changes. The per-file migration gate
(state model §6.3) runs the additive DDL; the startup reconcile (§4) lazily
populates placements. The schema-version degrade path is unaffected (a v3 store
opened by a v2 binary degrades per the existing rule).

---

## 7. Seams & cross-spec dependencies

- **B-2 (packing/scroll)** consumes the tree read-only: an ordered walk of a
  board's items with group nesting. B-1 exposes an iteration/read API
  (`board_tree(board_id) -> Vec<Item>` or a visitor) shaped for the packer.
- **B-3 (group render/aggregation)** reads group metadata (name, color,
  collapse) + walks members for cost/count/age rollups.
- **B-4 (movement)** drives the mutation ops; hit-testing/ordinal-snap is B-4,
  the resulting `move_item` is here.
- **B-5 (multi-board)** adds board CRUD UI + the "which board does an
  externally-discovered session land on" policy (active vs a dedicated
  "Unsorted" board — flagged, not decided; B-1 default is the single default
  board root).
- **§7.6 group quick-add** populates `group_config` (default new-session config);
  reserved column, NULL in B-1. Cross-referenced to the new-session dialog /
  agent-definition surface, not absorbed.
- **FleetStore key mismatch (seam).** `board_items` keys cards by
  `(connection_id, session_id)`; `FleetStore` is currently keyed by `SessionId`
  alone. Single-connection today, so the lookup is unambiguous; **connection-
  scoping `FleetStore` is multi-connection work (B-5 / state model)**. B-1's
  schema is correct now; the UI lookup collapses `conn` until then.

---

## 8. Judgment calls (veto points — carried from brainstorm, un-vetoed)

1. **Board is its own entity**, not a root group — it has rail identity + a
   switch shortcut (§4.4). Groups are strictly nested within a board.
2. **One placement per session, globally** (the unique index) — a session can't
   appear on two boards. Matches "detach *moves* a conversation" single-instance
   thinking (§3).
3. **Externally-discovered sessions land loose on the default board.** Revisited
   by B-5 when multiple boards exist (active board vs "Unsorted").
4. **`color_token` stored on the group** (stable across reflows); B-3 owns the
   palette.

---

## 9. Testing

- **Pure tree ops** (no DB): construct a `BoardLayout`, exercise
  `move_item`/`ungroup`/`create_group`/ordinal-renumber; assert the tree shape +
  dense ordinals + the cycle guard (group into own descendant rejected).
- **Persistence round-trip**: open an in-memory/temp `lens.db`, run mutations,
  reopen, assert the tree reloads identically (ordinals, nesting, flags).
- **Placement policy**: discover a session with no card item → asserts loose
  append at the default board's next root ordinal; re-discovery is idempotent
  (unique index holds, no dup).
- **Lifecycle**: archive keeps the row (+ still walkable for cost); delete/
  tombstone removes it; ungroup reparents children to the correct slot.
- **Migration**: open a v2 store, upgrade, assert v3 tables exist and existing
  sessions place lazily (no backfill), and a pre-existing session gets a loose
  card on first load.

---

## 10. Open / deferred

- **Default board name/id** — seed a stable id + a default name on first run;
  rename is B-5. (Minor; decide at build — not "Home" per §6's "no redundant
  Home".)
- **Externally-discovered landing board** with N boards — §8.3, → B-5.
- **Lifetime-spend-survives-deletion** — §5, → cost view / §21.
- **Empty-group auto-prune** — → B-3/B-4.
- **`FleetStore` connection-scoping** — → B-5 / state model.
