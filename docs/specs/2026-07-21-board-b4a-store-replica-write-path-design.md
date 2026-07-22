# Board B-4a — store→replica write-path foundation — design

**Written:** 2026-07-21 · **Status:** LOCKED — grilled, gpt-5.6 codex spec-review folded,
§3 re-grilled and settled. Ready for writing-plans. ·
**Depends on:** B-1 (`SqliteBoardStore` + `BoardStore` trait, `8100cc8`),
B-2 (packer + container, `14b474c`), B-3 (group chrome, `ac9d5ae`) ·
**Feeds:** B-4b (collapse), B-4c (drag/move), B-4d (grouping menus), B-5
(multi-connection scoping), B-6 (archive-as-board)

> **§3 write model (settled after the codex review + a §3 re-grill).** The original
> "inline synchronous write-then-reload" **violated a MANDATORY repo rule** (AGENTS.md:19 /
> `.agents/rust-ui.md`: *all disk I/O off-thread via `cx.background_spawn`; the UI thread only
> `cx.update`/`cx.notify`*). §3 is now **off-thread store access (`Arc<Mutex>` +
> `cx.background_spawn`) behind a main-thread in-memory replica**, with a **serialized
> single-in-flight `run_op`** (a light form of [[state-model-single-writer-decision]] — no
> command-channel actor; the board has no write *stream*). Renders read the in-memory replica,
> **never SQLite**. Single-in-flight is required for correctness *and* test determinism (§2).

B-4 was decomposed into a **foundation slice (B-4a, this doc)** plus three
interaction follow-ons (B-4b/c/d). B-4a replaces the ephemeral
`build_ephemeral_layout` stub with a persisted, writable `BoardLayout` sourced
from `SqliteBoardStore`, establishing the store→replica seam every later slice
rides on. It ships **no user interactions** — the board renders from the real
store and survives restarts.

## 0. Codex review dispositions (folded)

All ten findings from the gpt-5.6 spec review were confirmed against code and folded:
1. **Off-thread I/O (MANDATORY)** → §3 rewritten to `Arc<Mutex>` + `cx.background_spawn` (was inline). 2. **Commit-then-reload divergence** → the op returns the *committed* layout; no reload step to fail (§3.2). 3. **`new()` couldn't observe FleetStore** → takes `Entity<FleetStore>`, reconciles current keys at construction (§2, §3.3). 4. **Degraded contradictions** → explicit state enum; reconcile also gated; default layout seeds a default board (§5). 5. **`Loaded{rows,skipped}` not `.value`; partial loads** → unpack `rows`, surface `skipped` as degraded (§3.2, §5). 6. **Convergence not conn-scoped** → placed keys are `(ConnectionId, SessionId)`; cross-conn dup noted (§3.3, §4). 7. **`SQLITE_BUSY` on open** → open before actors + busy timeout; open failure → `LoadFailed` (§5, §6). 8. **O(k·N) reconcile** → batch `place_sessions` op + benchmark (§3.3). 9. **Tombstone-resurrection already prevented** → deferred guard deleted; replaced with a churn test (§8). 10. **Wrong FleetStore rationale** → corrected (§3.1).

---

## 1. Scope & non-goals

**In scope:** a `BoardReplica` gpui entity (main-thread, in-memory layout) + **off-thread
store access** (`Arc<Mutex>` + `background_spawn`); the async write path (ops return the
committed layout); the
session-lifecycle reconcile (batched); rewiring `pack_and_render` to read the replica;
non-fatal error handling + a non-blocking banner; a demo-seeded group; retiring
`build_ephemeral_layout` + the B-3 `test_layout` seam; **the mandatory perf benchmark**.

**Non-goals (deferred):** user interactions → B-4b/c/d; multi-connection → B-5;
session-archive handling → B-6 (§8); nested-group runtime → B-5. **No spike** for the
store wiring. Off-thread I/O is mandatory (AGENTS.md) but does **not** need a full
command-channel actor — a stream of writes would justify that, and the board has none
(discrete user ops + occasional reconciles). B-4a uses the lightest compliant shape:
`Arc<Mutex<store>>` + `cx.background_spawn` per op (§2). The perf benchmark is mandatory.

---

## 2. Components

**Read/write split (the crux, per the "reads happen constantly" question).** Renders
read the layout every frame (60–120fps); they read the **in-memory** `BoardReplica.layout`
and **never touch SQLite**. That is the entire point of the replica: the store is
canonical/persisted, the replica is the in-memory copy render walks for free. SQLite is
touched **only** on the rare paths — initial load, a mutation, and reconcile-on-placement.
So the off-thread machinery only ever handles infrequent, discrete ops; a full
command-channel actor would optimize a throughput path that doesn't exist here.

**Store access (off-thread, serialized single-in-flight).** `BoardReplica` holds an
`Arc<Mutex<Box<dyn BoardStore>>>` and runs **one op at a time** through a general
`run_op` path (below). Each op runs inside a `cx.background_spawn` closure that locks the
store, runs the op, computes the resulting `BoardLayout`, and posts it back via `cx.update`
(→ set `layout` + `notify`). **Single-in-flight is load-bearing, not incidental:** two
concurrent `background_spawn` tasks complete in thread-pool order, so their replies could
apply out of commit order — leaving the in-memory `layout` regressed at quiescence (it does
*not* self-heal without another trigger) and making tests non-deterministic. With one op
outstanding, replies land in enqueue order → correct at quiescence, deterministic tests, and
the mutex is effectively uncontended (it only satisfies `Arc` sharing across the spawn
boundary). This is **not** a persistent actor — the queue lives on the main-thread entity and
each op is a transient spawn; nothing loops forever or pins the `Connection` to a thread.

**`BoardReplica` (main-thread gpui entity, `crates/lens-ui/src/board/replica.rs`):**

```
BoardReplica {
    store: Arc<Mutex<Box<dyn BoardStore>>>, // locked inside background_spawn only
    conn: ConnectionId,                     // pinned to the app Connection.id (§4)
    layout: BoardLayout,                    // in-memory; every render reads this, no I/O
    state: ReplicaState,                    // Loading | Writable | Degraded | LoadFailed | Stale (§5)
    fleet: Entity<FleetStore>,              // observed for session-lifecycle reconcile (§3.3)
    in_flight: bool,                        // an op is spawned and not yet applied
    pending: VecDeque<Op>,                  // serialized queue (load/place/…; writes in B-4b+)
}
```

`Op` is an internal enum the store runner dispatches on: `Load`,
`PlaceSessions(Vec<(ConnectionId, SessionId)>)` in B-4a; `SetCollapsed`/`MoveItem`/
`CreateGroup`/… added by B-4b/c/d (**the seam is `run_op`, so those add variants with no
serialization rework**).

Interface:
- `BoardReplica::new(store, conn, fleet, cx) -> Entity<Self>` — takes the fleet
  (review #3), starts in `Loading`, enqueues `Load` first, installs the `FleetStore`
  observer, and enqueues a reconcile of the current `fleet.cards` snapshot (cards may
  predate the subscription). `Load` is first in the queue, so the first reconcile runs
  **after** the initial layout has landed.
- `layout(&self) -> &BoardLayout` (the free, in-memory render read), `state(&self)`,
  `is_writable(&self)`.
- `run_op(&mut self, op, cx)` (private seam) — the serialized runner: enqueue `op`, then
  `pump` — if not `in_flight` and `pending` non-empty, pop one, set `in_flight`, spawn it;
  on reply apply the committed layout (or transition state on `Err`, §5), clear `in_flight`,
  `pump` again. Applies in enqueue order by construction.
- `write(&mut self, op, cx)` — **iff** `is_writable()`, `run_op(op)`; else no-op + surface
  the state (§5). B-4b/c/d call this with mutation ops. Explicit writes **always enqueue**
  (a user's collapse/drag must never be dropped).
- `reconcile(&mut self, cx)` — diff `fleet.cards` keys vs placed `(conn, session)` keys; if
  any missing and writable, `run_op(PlaceSessions(missing))` — but **coalesced**: skip
  enqueuing if a reconcile is already pending/in-flight (idempotent; a redundant one is
  safe to drop, unlike a write).
- `BoardReplica::in_memory_for_test(fleet, cx)` — `:memory:` store + fixed conn.

The UI thread only does `cx.update`/`cx.notify`; all SQLite is inside `background_spawn`.

---

## 3. Data flow

### 3.1 Read path

`pack_and_render` reads `self.replica.read(cx).layout()` instead of
`build_ephemeral_layout`. The `board_tree` walk, packing, culling, and B-3 chrome are
unchanged. `BoardView` gains `replica: Entity<BoardReplica>` and observes it (layout /
banner changes) **and** `FleetStore` (membership/focus). **Correction (review #10):**
card *content* (status/cost) does **not** flow through `FleetStore` — each
`SessionCardView` observes its own `SessionCard` entity (`card/view.rs:52`);
`FleetStore` notifies on membership/focus. So `BoardView`'s `FleetStore` observation is
for *which* cards exist and focus, not their content.

### 3.2 Write path — serialized `run_op` → background_spawn → committed layout

Every op (load, place, and later collapse/move/group) runs off-thread through the single
serialized `run_op` path (§2):

1. `write` checks `is_writable()`; if not, no-op + surface the state (§5). Then `run_op`
   enqueues the op. No db access on the main thread.
2. `pump` spawns **one** `cx.background_spawn` closure (only if nothing is in flight) that
   locks the store, runs the op in a transaction and, on commit, computes the resulting
   `BoardLayout`, returning `Ok(layout)` — or `Err` (commit failed / degraded).
3. Back on the main thread, `cx.update` applies the committed layout (→ `layout` +
   `notify`), clears `in_flight`, and `pump`s the next queued op.

Because ops apply in enqueue order (single-in-flight), the in-memory `layout` is always the
latest committed state at quiescence — no out-of-order regress, deterministic in tests. This
also **dissolves review finding #2** (commit-then-reload divergence): there is no separate
reload that can fail after a commit — the op returns the layout it just committed
atomically. On `Err`, the replica transitions to `Stale`/`Degraded` (§5) and does **not**
blindly retry a non-idempotent op (`create_group`).

**Perf measurement is three distinct things (§7).** All SQLite is off-thread, so it never
touches the frame budget — the op round-trip is a *latency* metric, **not** a frame-budget
one (conflating them was wrong). The frame-budget path (AGENTS.md "**MANDATORY** … 120fps/8.3ms
target, 90fps/11.1ms regression") is the **per-frame render in lens-ui** — `pack_and_render`'s
`board_tree` walk + per-tile element build + B-3 rollup folds + cull + gpui layout/paint — and
its mandatory check is an **E2E on-device measurement** (the pure `lens-core` `pack()` bench is
only a slice, not the proof). (The earlier "inline is fine below ~1000 items" reasoning is
retired — non-compliant and un-benchmarked.)

### 3.3 Session-lifecycle reconcile (batched, additive, conn-pinned)

On each `FleetStore` change (and once at construction), `reconcile`:
- Computes placed keys as **`(ConnectionId, SessionId)`** (review #6 — a `SessionId`-only
  set cannot detect a cross-conn duplicate).
- Diffs live `fleet.cards` keys (paired with the pinned `conn`) against placed keys.
- Enqueues **one batched `PlaceSessions`** for the missing set (review #8 — k separate
  `place_session` calls each persist the board → ~O(k·N); one batched transaction is O(N)).
  Skips entirely (no store traffic) when nothing is new — the frequent membership
  notifies stay cheap.

**Convergence (review #6, narrowed).** Two placement sources exist: this loop (from
`fleet`, pinned conn) and `load_layout`'s built-in reconcile (from the `sessions` table,
each row's real conn). `place_session` + the unique index dedup the **exact**
`(conn, session)` tuple, so with a single connection (§4) they converge to one row. **But
`load_layout` is not conn-scoped:** a pre-existing `(other_conn, same_session)` row plus a
`(lens-app, same_session)` placement would be two tiles. Impossible under today's single
connection; flagged for **B-5** to scope reconcile/load by connection. B-4a tracks
`(conn, session)` keys so the case is at least detectable.

**Additive; tombstone already handled (review #9).** `fleet.cards` is add-only, so
reconcile only ever adds. A **tombstoned** session is never (re)placed: `place_session`
checks `tombstoned_at` and no-ops (`board.rs:437`, tested `board.rs:1076`). So the
resurrection loop the grill worried about **cannot happen** — the earlier deferred guard
is removed (§8). A stale fleet key just yields repeated cheap no-op skips (tested, §7).

---

## 4. Pinned connection (PROVISIONAL, → B-5)

`FleetStore` retains no per-session conn, and Lens is single-connection today — the app
uses one `Connection`, `ConnectionId::new("lens-app")` (`main.rs:306`), and
`SqliteControlStore` writes that same id into `sessions` (`main.rs:525`). **`BoardReplica.conn`
must be that id** — this is the constraint that makes the two placement sources converge
(§3.3). Prod = `"lens-app"`; demo/test = a fixed id. **PROVISIONAL** per
[[premature-layer-boundary-binding]]; **B-5** generalizes to per-session conn (and
conn-scoped reconcile, review #6).

---

## 5. Error handling, `ReplicaState`, and the banner

All store failures are **non-fatal** (the UI never panics — AGENTS.md) and never block the
app. A **silent empty board is the actively-bad UX** (reads as data loss, invites
rebuilding onto a phantom board), so B-4a is non-fatal, **not silent**, and
**self-protecting** (no writes when it can't persist).

**`ReplicaState` (explicit — review #4):** `Loading` (initial), `Writable`, `Degraded`
(read-only, data present), `LoadFailed` (read-only, empty), `Stale` (a write committed but
the reply/next-op failed — read-only until reopen). `is_writable()` is `Writable` only.
**`run_op` gates only *write* ops** (`PlaceSessions`/`SetCollapsed`/…) on `is_writable()`
(review #4 — reconcile mutates too, so it gates alongside user writes). A **`Load`/recovery
op is always allowed**, in any state — recovery is a read/reopen, not a write (see Recovery).

**Cases (corrected against `open_db`/`open`, review #4/#5/#7):**
- **`ReadOnlyDegraded`** — the version cell is a *future* version; reads allowed, writes
  refused. `load_layout` succeeds only while the old reader still matches the schema. →
  `Degraded`; board renders the user's data read-only; light "changes won't save" banner.
- **Open failure** — real corruption, I/O, a degraded-but-incompatible schema (open
  immediately queries `board_items`, so open itself can `Err`), **or `SQLITE_BUSY`** (many
  `SqliteControlStore` connections already hold `lens.db` — `fleet/live.rs:71`; `open` has
  no busy timeout — `db.rs:54`). → `LoadFailed`; empty layout **seeded with a default
  board** (review #4 — `BoardLayout::default()` is empty, so `default_board_id()` would
  otherwise error); "Couldn't load your board — data on disk is untouched" banner.
- **Partial load** — `load_layout` returns non-empty `Loaded.skipped` (corrupt rows
  deliberately skipped, kept observable). → `Degraded` (write-gated) + a banner noting some
  items couldn't be read (review #5). Unpack `Loaded.rows` (not `.value`).

**Recovery (automatic + manual).** Because `StoreMode` is immutable (review #4), recovery
is a **reopen-`Load`** (a fresh `open` behind the mutex), not a mere reload — and because
`Load` is always allowed (above), a non-`Writable` replica is never permanently stranded.
While non-`Writable`, a **`FleetStore` notify** *or* the banner's **"Retry"** enqueues a
recovery `Load`: a clean open+load → `Writable` (+ retire banner); still-failing → stays
`Degraded`/`LoadFailed`/`Stale`. It's **bounded** — `run_op` is single-in-flight +
coalesced, so at most one recovery attempt runs at a time even under frequent notifies. (A
min-interval throttle is a trivial add if a persistently-degraded store + chatty fleet ever
makes reopen attempts noisy — noted, not built.)

**Mechanics:** the banner is a small **non-blocking, dismissible** notice over the board
area (never a modal; the rest of Lens stays usable), driven by `ReplicaState`, with a
**"Retry"** affordance that triggers a recovery `Load`.

---

## 6. Construction & startup order

- **App (live):** open the board store on `data_dir/lens.db` **before the session
  actors start** (review #7 — minimizes `SQLITE_BUSY`), and give `open_db` a bounded
  **busy timeout** so a transient lock retries rather than fails. conn = `"lens-app"`. Wire
  `BoardReplica` into the `BoardView::mount` sites (`main.rs:110,165`).
- **Demo (fake):** in-memory store (`:memory:`; `CONTROL_DDL` still makes the empty
  `sessions` table) + `"conn_demo"`. **Seed a group at construction** (before the first
  fleet reconcile, so its members aren't re-placed loose), with members also spawned as
  fake fleet sessions — rendering B-3 group chrome live for the first time.
- **Tests:** `BoardReplica::in_memory_for_test(fleet, cx)`.

`BoardView::mount` gains a `replica: Entity<BoardReplica>` parameter; call sites are the
app (2×), the acceptance tests, and the migrated B-3 fixture test.

If a bounded busy timeout doesn't fully close the open-time `SQLITE_BUSY` window, that's
just another path into `LoadFailed` (§5) — non-fatal, banner, retry-reopen.

---

## 7. Testing

`BoardStore` admits an in-memory store; real-window harness per [[gpui-test-noop-text-system]].
Ops are async (`background_spawn`), so tests drive `run_until_parked` to settle replies.

- **Load renders persisted placements**; **new session placed + persists across reopen**.
- **Convergence / no double-place** with pinned conn → exactly one card row.
- **Group renders B-3 chrome via the real path** (migrated fixture test: seed group +
  members, assert `group_chrome_for_test`).
- **`.cached()` freeze regression (the real risk, review #10):** spawn a session →
  `run_until_parked` → the new card **renders and keeps animating** (reconcile fired, no
  freeze); seed a group → its **member cards render + tick** (closes the B-3
  `absolute_group` render-time member-read carryforward, `board/mod.rs:384`); assert
  **group-rollup freshness** when a member's cost changes.
- **Serialized `run_op` / determinism:** ops apply in enqueue order (single-in-flight); a
  `load` chained before the first reconcile; two overlapping writes apply in order (no
  out-of-order regress), verified deterministically under `run_until_parked`.
- **State gating + recovery:** `Degraded`/`LoadFailed`/`Stale` replicas **refuse** write ops
  (`write` **and** `reconcile`) but a **recovery `Load` is accepted** and transitions a
  now-healthy store back to `Writable` (retiring the banner); `LoadFailed` renders an empty
  **default-board** layout (not a panic); partial load (`skipped` non-empty) → `Degraded` +
  banner.
- **Tombstoned fleet key** stays absent with no reload/notify churn (review #9).
- **Reconcile idempotent / batched** — two reconciles → one row per session; a k-session
  batch issues one `PlaceSessions`.

**Perf — three distinct measures (§3.2):**
1. **Frame-budget E2E (MANDATORY, lens-ui, on-device):** extend the [[wave-perf-fps-attribution]]
   `measure.sh` rig to render a board of **N items *including a group*** (seedable via the B-4a
   demo) and sample FPS/CPU at realistic (~100) + stress (~1000+) N; hold 120fps/8.3ms target,
   90fps regression line. First runtime exercise of group render-time member reads at scale;
   closes B-2 Task 6's residual (at-scale cull CPU never measured on the real app).
2. **`lens-core` pack/`board_tree` criterion bench (supporting, gate-automatable):** the pure
   packing math — a CI regression signal, not the frame-budget proof (matches the existing
   `persist_throughput`/`reduce_throughput` benches).
3. **Op-latency (off-frame):** `load` + batched `PlaceSessions` at N as wall-clock, confirming
   sessions appear promptly and the mutex isn't held excessively — explicitly *not* a
   frame-budget assertion.

Full `xtask gate` green (incl. the lens-core pack bench building).

---

## 8. Seams & deferred decisions (recorded)

- **B-4b/c/d** ← `write(cmd)` over collapse / move / group ops; B-4c drag hit-testing vs
  packer geometry (spike candidate).
- **B-5** ← per-session `ConnectionId` + **conn-scoped reconcile/load** (review #6);
  multiple boards; externally-discovered-session landing policy.
- **Session-archive handling → B-6 (decided).** Two `archived` flags: `board_items.archived`
  (groups only) vs `sessions.archived` (`SessionState.archived`). B-4a reconcile keys only
  on `tombstoned_at`, so an archived-not-tombstoned session's card stays on the active
  board — same as under the stub (pre-existing; likely unreachable until B-6 loads archived
  sessions). B-6 owns prune-vs-render-filter-vs-move.
- **Archived rows inflate persisted N → B-6.** `load_items` loads all rows; archived groups
  pay structural-write cost though `board_tree` hides them. If B-6 flags-in-place, it should
  move archived subtrees to a separate archive board/table or skip them in persist.
- **~~Tombstone-resurrection guard~~ — removed (review #9).** Already prevented by
  `place_session`'s tombstone check; no guard owed.
