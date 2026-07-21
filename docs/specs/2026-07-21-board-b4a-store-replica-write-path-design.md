# Board B-4a — store→replica write-path foundation — design

**Written:** 2026-07-21 · **Status:** design locked (user-approved + grilled) ·
**Depends on:** B-1 (`SqliteBoardStore` + `BoardStore` trait, shipped `8100cc8`),
B-2 (packer + scroll/culling container, shipped `14b474c`), B-3 (group chrome,
shipped `ac9d5ae`) · **Feeds:** B-4b (collapse), B-4c (drag/move), B-4d (grouping
menus), B-5 (multi-connection scoping), B-6 (archive-as-board)

B-4 (drag/move + context-menu grouping) was decomposed at design time into a
**foundation slice (B-4a, this doc)** plus three interaction follow-ons
(B-4b/c/d). B-4a is the load-bearing, riskiest piece: it replaces the ephemeral
`build_ephemeral_layout` stub with a persisted, writable `BoardLayout` sourced
from `SqliteBoardStore`, and establishes the store→replica seam that every
interaction slice and B-5/B-6 build on. It ships **no user interactions** — the
board renders from the real store and survives restarts; that is the whole
deliverable.

This design was grilled; §§3–5 and §8 carry the resolved decisions.

---

## 1. Scope & non-goals

**In scope:**
- A new `BoardReplica` gpui entity (lens-ui) owning the persisted `BoardLayout`,
  a `BoardStore` handle, the pinned `ConnectionId`, and a `read_only` flag.
- The **write-then-reload** write path (store-canonical; a single private helper
  B-4b/c/d call), **write-gated by `read_only`**.
- The **session-lifecycle reconcile** loop that lazily `place_session`s live
  FleetStore sessions the store hasn't seen — replacing the stub's implicit "all
  fleet cards → loose cards".
- Rewiring `BoardView::pack_and_render` to read the replica's `BoardLayout`.
- **Non-fatal error handling** + a **non-blocking banner** (degraded / load-failure).
- **Demo seeds a group** so B-3 group chrome renders live for the first time.
- **Retirements:** delete `board/layout_adapter.rs` (`build_ephemeral_layout`)
  and the B-3 `test_layout` injection seam (`test_layout` field +
  `set_test_layout_for_test`); migrate B-3's group fixture test to seed a
  `BoardReplica` via a fake store.

**Explicitly deferred (with reasons):**
- **User interactions → B-4b/c/d.** Collapse toggle + §7 collapsed-tile render
  (B-4b); drag/move/reorder (B-4c); create-group/ungroup/rename/recolor context
  menus (B-4d). B-4a exposes the write seam they call, nothing more.
- **Multi-connection / connection-scoping → B-5.** B-4a places under one pinned
  `ConnectionId` (§4). Per-session connection tracking is a B-5 concern.
- **Session-archive handling → B-6** (§8, decided). B-4a's reconcile handles only
  **tombstone** (delete); an archived-but-not-tombstoned session is left as-is
  (renders on the active board, exactly as under the ephemeral stub).
- **Tombstone-resurrection guard → deferred** (§8). Unreachable in B-4a (no
  runtime delete/tombstone path); guarded when session deletion lands.
- **No spike.** Wiring against a proven store API + proven gpui entity/observe
  patterns. The drag hit-testing risk lives in B-4c.

---

## 2. Component — `BoardReplica` (new, `crates/lens-ui/src/board/replica.rs`)

The single home for board placement, keeping `FleetStore` focused on live
sessions. Owns:

```
BoardReplica {
    store: Box<dyn BoardStore>,   // SqliteBoardStore in prod; in-memory in demo/tests
    layout: BoardLayout,          // the in-memory replica; store is canonical
    conn: ConnectionId,           // pinned to the app Connection.id (§4)
    read_only: bool,              // set on degraded mode / load failure (§5)
}
```

Public interface:
- `BoardReplica::new(store, conn, cx) -> Entity<Self>` — loads the initial layout
  via `store.load_layout()`; on error, `read_only = true` + empty default layout
  (§5). Constructed by the caller (app / demo / test), passed into
  `BoardView::mount` (mirrors how `fleet` is passed).
- `layout(&self) -> &BoardLayout` — the read handle `BoardView` renders from.
- `is_read_only(&self) -> bool` — B-4b/c/d and the banner read this.
- `write(&mut self, cx, f: impl FnOnce(&dyn BoardStore, &ConnectionId) -> Result<T>) -> Result<T>`
  — the single write path (§3.2). No-ops with an error if `read_only`. B-4b/c/d
  call this, e.g. `replica.write(cx, |s, _| s.set_collapsed(&g, true))`.
- `reconcile_sessions(&mut self, live: impl Iterator<Item = &SessionId>, cx)` —
  the session-lifecycle placement loop (§3.3).
- `BoardReplica::in_memory_for_test(cx) -> Entity<Self>` — `:memory:` store +
  fixed test conn, to keep test call sites terse.

`BoardReplica` **observes `FleetStore`** and calls `reconcile_sessions` on
change. It reads `fleet.cards` **keys** only — never card entity *content* — so
it is clear of the `.cached()` dirty-tracking trap ([[viewport-reentry-freeze]]),
which is a *render-time* card-entity read, not an observe-effect key read.

---

## 3. Data flow

### 3.1 Read path

`BoardView::pack_and_render` reads `self.replica.read(cx).layout()` instead of
calling `build_ephemeral_layout`. The `board_tree` walk, packing, band-culling,
and B-3 group chrome are **unchanged**. `BoardView` gains a
`replica: Entity<BoardReplica>` field and observes it **in addition to**
`FleetStore` — it still needs `FleetStore` directly for per-card
`SessionCardView`s and for card content changes (status/cost) that don't touch
placement and so wouldn't notify via the replica.

### 3.2 Write path — write-then-reload (store-canonical)

Every mutation flows through `BoardReplica::write`:

1. If `read_only`, return an error without touching the store (§5).
2. Call the `BoardStore` op — persists to `lens.db`.
3. `self.layout = self.store.load_layout()?.value` — reload the canonical layout.
4. `cx.notify()`.

Store is always canonical; the replica cannot diverge. A store-op error
propagates out of `write` (in B-4a the only caller is `reconcile`; user-write
callers arrive in B-4b/c/d and surface it to the user); the layout is untouched
on error because the reload only runs after a successful op.

**`load_layout` is not a pure read.** In `ReadWrite` mode it *reconciles*: adds
cards for `sessions`-table rows and **prunes cards whose session is tombstoned**.
So write-then-reload also re-runs session reconcile + tombstone prune on every
write — idempotent, and it means tombstone pruning happens continuously, not only
at startup. In fake/demo mode the `sessions` table is empty, so this reconcile is
a no-op (absence ≠ delete — it never nukes a `BoardReplica`-placed card).

**Inline-cost tripwire.** `write`/`reconcile` do synchronous SQLite on the main
(render) thread. Reads stay sub-ms; the cost is the dirty-write transaction, which
today **re-persists every item** (O(N) writes per mutation). At a few hundred
items this is single-digit ms (imperceptible); it crosses a one-frame hitch
(~10–30 ms) around ~1,000 items and becomes a visible hitch near ~10,000. It fires
only on session **appearance** or a discrete board **write**, never per frame.
Lens's shape (dozens–low-hundreds of active sessions) sits far below the line.
**Escape ladder, in order:** (1) persist only the changed item (kills the O(N)
multiplier, ~10× headroom — a store-layer tweak); (2) move `load_layout`/persist
to `cx.background_spawn` (removes main-thread blocking). Decision rule: inline is
correct until board item count is plausibly in the thousands **or** a real-window
test shows a measurable frame hitch — and the first fix is "persist the delta,"
not "go async."

### 3.3 Session-lifecycle reconcile (additive, conn-pinned)

Replaces the stub's implicit "every fleet card is a loose card". On each
`FleetStore` change, `reconcile_sessions`:

- Computes placed sessions (walk `layout.items` for
  `BoardItemKind::Card { session, .. }`).
- For each live session **not** already placed:
  `store.place_session(&self.conn, session, &PlacementTarget::default())`, then
  reload **once** at the end. If nothing is new, it's a cheap key-diff with **no**
  reload — so the frequent status/cost notifies don't trigger SQLite work.
- Idempotent: `place_session` returns `Ok` early if `(conn, session)` already has
  a card row.

**Convergence constraint (load-bearing).** There are two placement mechanisms:
this loop (source = `FleetStore`) and `load_layout`'s built-in reconcile (source =
the control-db `sessions` table, written by `SqliteControlStore` in live mode with
each session's *real* conn). They **must** use the identical conn for a session,
or the same session gets two card rows under two conns → a duplicate tile. B-4a
pins `BoardReplica.conn` to the app's real `Connection.id` (§4), which **is** the
id `SqliteControlStore` writes — so the two converge (`place_session` dedups) to
one row. In fake/demo mode the `sessions` table is empty, so `BoardReplica` is the
sole populator (under the fixed test/demo conn) and there is nothing to converge
with.

**Additive.** A session leaving `fleet.cards` keeps its persisted slot — but note
`FleetStore.cards` is **add-only** today (it never removes a card, even on
sleep/disconnect), so at runtime reconcile only ever *adds*. The only removal
mechanism is `load_layout`'s tombstone prune (live mode). See §8 for the latent
tombstone-resurrection interaction (deferred).

---

## 4. Pinned connection (PROVISIONAL, → B-5)

`place_session`/`load_layout` key card rows on `(ConnectionId, SessionId)`, but
`FleetStore` retains no per-session conn (its `spawn_live_session` takes a
`&Connection` and stores none; the `wake_session` TODO confirms this), and Lens is
single-connection today — the app uses exactly one `Connection` with
`ConnectionId::new("lens-app")` (`main.rs:306`), and `SqliteControlStore` writes
that same id into the `sessions` table.

**Decision:** `BoardReplica.conn` **must be the app's real `Connection.id`**
(`"lens-app"` in prod; a fixed id like `"conn_demo"` / a test id elsewhere). This
is not a free choice — it is the constraint that makes the two placement sources
converge (§3.3). Marked **PROVISIONAL** per [[premature-layer-boundary-binding]];
**B-5 connection-scoping** generalizes to per-session conn tracking and threads
the real conn through reconcile + the interaction writes.

---

## 5. Error handling & the read-only gate

All store failures are **non-fatal** and never block the app — consistent with the
control-db's degraded-tolerant contract everywhere else ([[state-model-P2-persistence]]:
schema-degrade → read-only, never crash). The board must never crash Lens, and —
critically — a **silent empty board is the actively-bad UX** (it reads as data
loss and invites the user to rebuild onto a phantom-empty board, manufacturing a
real merge mess). So B-4a is non-fatal **but not silent**, and **self-protecting**
(read-only when it can't persist).

Two distinct cases:

- **Degraded (`ReadOnlyDegraded`) — common.** The db opened but is schema-mismatched
  / lightly corrupt. `load_layout` **succeeds** (reads work) — so the board
  **renders the user's data normally**; only *writes* fail. `read_only = true`.
  Banner: a light "Board changes won't save right now" indicator. Low surprise —
  their board is right there, just not rearrangeable.
- **Load failure — rare.** `load_layout` `Err`s (real corruption / disk / I-O).
  `read_only = true` + an **empty default layout** (default board, no items).
  Banner: "Couldn't load your board — your data on disk is untouched; changes are
  paused." Explains the empty state so it doesn't read as a wipe.

Mechanics:
- **`read_only` gates `write`** (§3.2 step 1) — so a degraded/failed board *refuses*
  edits (B-4b/c/d interactions no-op with the failure surfaced) instead of silently
  dropping them or building onto a phantom board. This gate is the load-bearing
  safety property and ships in B-4a regardless of the banner.
- **`reconcile` `place_session` failure** (transient/degraded): log + skip, keep the
  last-good layout; the next observe retries.
- **Banner:** a small **non-blocking, dismissible** notice rendered over the board
  area (not a modal, never blocks the rest of Lens). Reads `is_read_only()` + a
  cause enum (`Degraded` / `LoadFailed`). Retire it if the store recovers on a later
  reload.
- The rest of Lens (sessions, terminals, everything non-board) stays fully usable.

---

## 6. Construction across contexts

- **App (live):** `main.rs` already opens `SqliteControlStore` on `data_dir/lens.db`.
  B-4a opens a **second** `SqliteBoardStore` on the **same** `lens.db` — two rusqlite
  connections to one file, WAL-safe (the tier is already WAL) and exactly B-1's
  intent (the board lives in the control db); the board store's reconcile `SELECT`s
  the `sessions` table the control store writes, with WAL committed-read visibility.
  conn = `"lens-app"`. Wire `BoardReplica` into the `BoardView::mount` call sites
  (`main.rs:110,165`).
- **Demo (fake):** in-memory `SqliteBoardStore` (`:memory:`; `CONTROL_DDL` still
  creates the empty `sessions` table) + `"conn_demo"`. **Seed a group** at
  construction — before the first fleet observe, so reconcile finds its members
  already placed (in the group) and doesn't re-place them loose — with members that
  are also spawned as fake fleet sessions (so their `SessionCardView`s exist). This
  renders B-3 group chrome live on-device for the first time.
- **Tests:** `BoardReplica::in_memory_for_test(cx)` + fixed conn.

`BoardView::mount` gains a `replica: Entity<BoardReplica>` parameter; call sites are
the app (2×), the acceptance tests, and the migrated B-3 fixture test.

---

## 7. Testing

The `BoardStore` trait admits a fake/`:memory:` store, so B-4a is fully testable
without a live server. Real-window harness per [[gpui-test-noop-text-system]].

- **Load renders persisted placements:** seed a store with a group + loose cards,
  build `BoardReplica`, assert `layout()` yields the expected `board_tree`.
- **New session is placed + persists:** `reconcile_sessions` over a fleet with a
  fresh session → lands on default root; reopen the store → placement survived.
- **Convergence / no double-place:** with `conn` pinned, a session present in *both*
  the `sessions` table and the fleet yields exactly **one** card row.
- **Group renders B-3 chrome via the real path:** migrated fixture test — seed a
  colored group + members, render `BoardView`, assert the `group_chrome_for_test`
  snapshot (ring accent, folded rollup, header).
- **`.cached()` freeze regression (the real risk):** mount, spawn a session into the
  fleet, `run_until_parked`, assert the new card **renders and keeps animating**
  (proves reconcile fired *and* nothing froze); a second test seeds a group and
  asserts its **member cards render + tick** (closes the B-3 `absolute_group`
  member-read-during-render carryforward — §8).
- **Read-only gate:** a `read_only` replica **refuses** a `write` (returns error,
  layout unchanged); a degraded-mode store renders its persisted layout read-only.
- **Additive / reconcile idempotent:** two reconciles of the same fleet → one card
  row per session (no duplicates).

Full `xtask gate` green.

---

## 8. Seams & deferred decisions (recorded)

- **B-4b** ← `write(|s,_| s.set_collapsed(...))` + §7-mockup collapsed-tile render +
  caret click. First user write; proves the path end-to-end.
- **B-4c** ← drag/move: gpui `on_drag`/`on_drop` hit-testing against packer
  `(gx,gy,fc,fr)` geometry → `write(|s,_| s.move_item(...))`. Spike candidate.
- **B-4d** ← context menus: `write` over `create_group`/`ungroup`/`rename`/`set_color`.
- **B-5** ← per-session `ConnectionId` (retires §4's provisional single conn);
  multiple boards; externally-discovered-session landing policy.
- **Tombstone-resurrection guard (deferred).** `FleetStore.cards` is add-only, but
  `load_layout` prunes tombstoned sessions on every reload. So once a runtime
  delete/tombstone path exists, a tombstoned-but-still-in-fleet session would be
  re-placed by reconcile → pruned → re-placed (flicker). **Unreachable in B-4a** (no
  runtime tombstone path). When session **deletion** lands (B-4d/B-5-era, which will
  redesign removal holistically), guard reconcile against it — e.g. skip placing a
  session whose `SessionCard.lifecycle == Deleted`, or add a tombstone check to
  `place_session`.
- **Session-archive handling → B-6 (decided).** There are **two** `archived` flags:
  `board_items.archived` (groups only, set by `archive()`) and `sessions.archived`
  (`SessionState.archived`, server-reported, in the `sessions` table). A "card that's
  archived" means the *session* is archived. B-4a's reconcile keys only on
  `tombstoned_at`, **not** `sessions.archived`, so an archived-not-tombstoned
  session's card renders on the active board — **the same as under the ephemeral
  stub** (pre-existing, and likely unreachable until the app loads archived sessions,
  which is B-6's job). B-6 (archive-as-board) owns where archived sessions live and
  how they leave the active board; it should decide **prune vs render-filter vs
  move-to-archive-board**, and preserve placement if useful.
- **Archived rows inflate write-cost N → B-6.** `load_items` loads **all**
  `board_items` rows (no `archived` filter) and reconcile re-persists them, so
  archived groups pay the §3.2 write cost even though `board_tree` filters them from
  the render. Bounded only if archiving prunes/moves rows. If B-6 chooses
  *flag-in-place* archiving, it should move archived subtrees to a separate archive
  board/table **or** make persist skip unchanged archived subtrees, so history can't
  drift toward the §3.2 tripwire. Store-layer fix, not an architecture change.
