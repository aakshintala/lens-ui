# Board B-4a — store→replica write-path foundation — design

**Written:** 2026-07-21 · **Status:** design locked (user-approved) ·
**Depends on:** B-1 (`SqliteBoardStore` + `BoardStore` trait, shipped `8100cc8`),
B-2 (packer + scroll/culling container, shipped `14b474c`), B-3 (group chrome,
shipped `ac9d5ae`) · **Feeds:** B-4b (collapse), B-4c (drag/move), B-4d (grouping
menus), B-5 (multi-connection scoping)

B-4 (drag/move + context-menu grouping) was decomposed at design time into a
**foundation slice (B-4a, this doc)** plus three interaction follow-ons
(B-4b/c/d). B-4a is the load-bearing, riskiest piece: it replaces the ephemeral
`build_ephemeral_layout` stub with a persisted, writable `BoardLayout` sourced
from `SqliteBoardStore`, and establishes the store→replica seam that every
interaction slice and B-5/B-6 build on. It ships **no user interactions** — the
board renders from the real store and survives restarts; that is the whole
deliverable.

---

## 1. Scope & non-goals

**In scope:**
- A new `BoardReplica` gpui entity (lens-ui) owning the persisted `BoardLayout`
  and a `BoardStore` handle.
- The **write-then-reload** write path (store-canonical; a single private helper
  that B-4b/c/d call).
- The **session-lifecycle reconcile** loop that lazily `place_session`s live
  FleetStore sessions the store hasn't seen — replacing the stub's implicit "all
  fleet cards → loose cards".
- Rewiring `BoardView::pack_and_render` to read the replica's `BoardLayout`
  instead of fabricating one each render.
- **Retirements:** delete `board/layout_adapter.rs` (`build_ephemeral_layout`)
  and the B-3 `test_layout` injection seam (`test_layout` field +
  `set_test_layout_for_test`); migrate B-3's group fixture test to seed a
  `BoardReplica` via a fake store.

**Explicitly deferred (with reasons):**
- **User interactions → B-4b/c/d.** Collapse toggle + §7 collapsed-tile render
  (B-4b); drag/move/reorder (B-4c); create-group/ungroup/rename/recolor context
  menus (B-4d). B-4a exposes the `write()` helper they call, nothing more.
- **Multi-connection / connection-scoping → B-5.** B-4a places everything under
  one `ConnectionId` (§4). Per-session connection tracking is a B-5 concern
  (STATUS: "FleetStore connection-scoping").
- **Runtime removal / pruning.** Placement is **additive** (§3.3). A session
  leaving `fleet.cards` (slept/disconnected) keeps its persisted slot — that is
  the point of persistence (Sleep≠Archive). B-1 already prunes tombstoned rows at
  startup `load_layout`; a runtime prune policy stays deferred.
- **No spike.** This is wiring against a proven store API (all `BoardStore` ops
  exist) and proven gpui entity/observe patterns. The drag hit-testing risk lives
  in B-4c.

---

## 2. Component — `BoardReplica` (new, `crates/lens-ui/src/board/replica.rs`)

The single home for board placement, keeping `FleetStore` focused on live
sessions. Owns:

```
BoardReplica {
    store: Box<dyn BoardStore>,   // SqliteBoardStore in prod; fake/temp in tests
    layout: BoardLayout,          // the in-memory replica; store is canonical
    conn: ConnectionId,           // PROVISIONAL single connection (§4)
}
```

Public interface:
- `BoardReplica::new(store: Box<dyn BoardStore>, conn: ConnectionId, cx) -> Entity<Self>`
  — loads the initial layout via `store.load_layout()` (which applies B-1's
  startup reconcile / tombstone prune).
- `layout(&self) -> &BoardLayout` — the read handle `BoardView` renders from.
- `write(&mut self, cx, f: impl FnOnce(&dyn BoardStore, &ConnectionId) -> Result<T>) -> Result<T>`
  — the single write path (§3.2). Runs `f` against the store, reloads the layout,
  notifies. B-4b/c/d call this, e.g.
  `replica.write(cx, |s, _| s.set_collapsed(&g, true))`.
- `reconcile_sessions(&mut self, live: impl Iterator<Item = &SessionId>, cx)` —
  the session-lifecycle placement loop (§3.3).

`BoardReplica` **observes `FleetStore`** and calls `reconcile_sessions` on
change. It does not read card *content* — only the set of session ids to place.

---

## 3. Data flow

### 3.1 Read path

`BoardView::pack_and_render` currently calls
`build_ephemeral_layout(self.fleet.read(cx))`. It changes to read
`self.replica.read(cx).layout()`. Everything downstream — the `board_tree` walk,
packing, band-culling, and B-3 group chrome — is **unchanged**. `BoardView` gains
a `replica: Entity<BoardReplica>` field and observes it (in addition to
`FleetStore`, which it still needs for the per-card `SessionCardView`s).

### 3.2 Write path — write-then-reload (store-canonical)

Every mutation flows through `BoardReplica::write`:

1. Call the `BoardStore` op (`place_session` / `create_group` / `move_item` /
   `ungroup` / `rename` / `archive` / `set_collapsed` / `set_color`) — persists
   to `lens.db`.
2. `self.layout = self.store.load_layout()?.value` — reload the canonical layout.
3. `cx.notify()` — re-render.

Store is always canonical; the replica cannot diverge. Reload cost is negligible
at board scale (a handful of items, one control-db read). A store-op error
propagates out of `write` (the caller/UI surfaces it); the layout is untouched on
error because the reload only runs after a successful op.

### 3.3 Session-lifecycle reconcile (additive)

Replaces the stub's implicit "every fleet card is a loose card". On each
`FleetStore` change, `reconcile_sessions`:

- Computes the set of sessions already placed in `layout` (walk items for
  `BoardItemKind::Card { session, .. }`).
- For each live session **not** already placed:
  `store.place_session(&conn, session, &PlacementTarget::default())` (default =
  default board root append slot), then reload once at the end.
- Idempotent: `place_session` returns `Ok` early if `(conn, session)` already has
  a card row, so re-fires are cheap and safe.

**Additive only.** A session that leaves `fleet.cards` is **not** removed — its
persisted slot survives (a slept session keeps its place; Sleep≠Archive). A
placed session whose live `SessionCardView` is gone renders no tile (the existing
`absolute_card` returns `None`), leaving an empty slot until an explicit
remove/prune lands in a later slice. This is correct persistence behavior, not a
gap.

---

## 4. Single-connection decision (PROVISIONAL)

`place_session` and `load_layout` key card rows on `(ConnectionId, SessionId)`,
but `FleetStore` does not retain a per-session `ConnectionId` (its
`spawn_live_session` takes a `&Connection` but stores none; the `wake_session`
TODO confirms connection context isn't retained), and Lens is effectively
single-connection today.

**Decision:** `BoardReplica` holds one `ConnectionId` — the app's live
connection id in prod, a fixed id in tests — and places everything under it.
Marked **PROVISIONAL** per [[premature-layer-boundary-binding]]. **B-5
connection-scoping** generalizes to per-session connection tracking; when it
lands it threads the real conn through `reconcile_sessions` and the interaction
writes. Consistency holds meanwhile because a given session is always placed
under the same (single) conn id, so `place_session`'s dedup is stable.

---

## 5. Retirements

- Delete `crates/lens-ui/src/board/layout_adapter.rs` and its
  `build_ephemeral_layout` call site in `pack_and_render`.
- Delete the B-3 test seam: the `test_layout: Option<BoardLayout>` field, its
  `mount` init, `set_test_layout_for_test`, and the `unwrap_or_else` layout
  source in `pack_and_render`.
- Migrate the B-3 group fixture test (`board_group_renders_chrome_and_rollup`) to
  construct a `BoardReplica` over a fake/temp store seeded with a group +
  members, and drive `BoardView` through the real read path. This is a strictly
  stronger test than the injection seam (it exercises store→replica→render).

---

## 6. Testing

The `BoardStore` trait admits a fake or temp-file `SqliteBoardStore`, so B-4a is
fully unit/acceptance-testable without a live server:

- **Load renders persisted placements:** seed a store with a group + loose cards,
  build `BoardReplica`, assert `layout()` yields the expected `board_tree`.
- **New session is placed + persists:** `reconcile_sessions` over a fleet with a
  fresh session → assert it lands on the default root; reopen the store → assert
  the placement survived.
- **Group renders B-3 chrome via the real path:** the migrated fixture test —
  seed a colored group + members, render `BoardView`, assert the
  `group_chrome_for_test` snapshot (ring accent, folded rollup, header).
- **Additive on disappearance:** place a session, drop it from the fleet,
  reconcile → assert the placement still exists (no auto-remove).
- **Reconcile is idempotent:** two reconciles of the same fleet → one card row per
  session (no duplicate placements).

Full `xtask gate` green; the acceptance test uses the real-window harness per
[[gpui-test-noop-text-system]].

---

## 7. Seams (referenced, not built here)

- **B-4b** ← `write(|s,_| s.set_collapsed(...))` + §7 collapsed-tile render +
  caret click. First user write; proves the path end-to-end.
- **B-4c** ← drag/move: gpui `on_drag`/`on_drop` hit-testing against packer
  `(gx,gy,fc,fr)` geometry → `write(|s,_| s.move_item(...))`. Spike candidate.
- **B-4d** ← context menus: `write` over `create_group`/`ungroup`/`rename`/
  `set_color`.
- **B-5** ← per-session `ConnectionId` (retires §4's provisional single conn);
  multiple boards; externally-discovered-session landing policy.
- **B-3 carryforward (verify here):** `absolute_group` reads member
  `SessionCard` entities during `render` to fold the rollup. Now that groups are
  reachable through the real store, confirm this does not re-trip the `.cached()`
  freeze ([[viewport-reentry-freeze]]); if it does, hoist the fold into
  `sync_card_views`. (B-4a makes groups renderable via a seeded store even before
  B-4d's create-group UI, so this is verifiable here.)
