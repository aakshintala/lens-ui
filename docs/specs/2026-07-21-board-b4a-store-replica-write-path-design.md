# Board B-4a — store→replica write-path foundation — design

**Written:** 2026-07-21 · **Status:** design in progress (grilled; then gpt-5.6
codex spec-review folded — **§3 threading reopened, see banner below**) ·
**Depends on:** B-1 (`SqliteBoardStore` + `BoardStore` trait, `8100cc8`),
B-2 (packer + container, `14b474c`), B-3 (group chrome, `ac9d5ae`) ·
**Feeds:** B-4b (collapse), B-4c (drag/move), B-4d (grouping menus), B-5
(multi-connection scoping), B-6 (archive-as-board)

> **⚠️ NEEDS RE-REVIEW — the write model changed materially.** The codex spec
> review found the original "inline synchronous write-then-reload" **violates a
> MANDATORY repo rule** (AGENTS.md:19 / `.agents/rust-ui.md`: *all disk I/O
> off-thread via `cx.background_spawn`; the UI thread only `cx.update`/`cx.notify`*).
> §3 is rewritten to an **off-thread store worker + main-thread replica** (the
> [[state-model-single-writer-decision]] actor/replica pattern). This reverses the
> grill's Q4 ("inline is fine") and reopens the write-model choice, so §3 warrants
> a fresh look before planning.

B-4 was decomposed into a **foundation slice (B-4a, this doc)** plus three
interaction follow-ons (B-4b/c/d). B-4a replaces the ephemeral
`build_ephemeral_layout` stub with a persisted, writable `BoardLayout` sourced
from `SqliteBoardStore`, establishing the store→replica seam every later slice
rides on. It ships **no user interactions** — the board renders from the real
store and survives restarts.

## 0. Codex review dispositions (folded)

All ten findings from the gpt-5.6 spec review were confirmed against code and folded:
1. **Off-thread I/O (MANDATORY)** → §3 rewritten to a background worker (was inline). 2. **Commit-then-reload divergence** → the worker returns the *committed* layout; no reload step to fail (§3.2). 3. **`new()` couldn't observe FleetStore** → takes `Entity<FleetStore>`, reconciles current keys at construction (§2, §3.3). 4. **Degraded contradictions** → explicit state enum; reconcile also gated; default layout seeds a default board (§5). 5. **`Loaded{rows,skipped}` not `.value`; partial loads** → unpack `rows`, surface `skipped` as degraded (§3.2, §5). 6. **Convergence not conn-scoped** → placed keys are `(ConnectionId, SessionId)`; cross-conn dup noted (§3.3, §4). 7. **`SQLITE_BUSY` on open** → open before actors + busy timeout; open failure → `LoadFailed` (§5, §6). 8. **O(k·N) reconcile** → batch `place_sessions` worker op + benchmark (§3.3). 9. **Tombstone-resurrection already prevented** → deferred guard deleted; replaced with a churn test (§8). 10. **Wrong FleetStore rationale** → corrected (§3.1).

---

## 1. Scope & non-goals

**In scope:** a `BoardReplica` gpui entity (main-thread) + an **off-thread board-store
worker**; the async write path (worker returns the committed layout); the
session-lifecycle reconcile (batched); rewiring `pack_and_render` to read the replica;
non-fatal error handling + a non-blocking banner; a demo-seeded group; retiring
`build_ephemeral_layout` + the B-3 `test_layout` seam; **the mandatory perf benchmark**.

**Non-goals (deferred):** user interactions → B-4b/c/d; multi-connection → B-5;
session-archive handling → B-6 (§8); nested-group runtime → B-5. **No spike** for the
store wiring, but the off-thread worker is new architecture for this crate (precedent:
the state-model actor) and the perf benchmark is mandatory, not optional (AGENTS.md).

---

## 2. Components

**`BoardStoreWorker` (off-thread).** Owns the `SqliteBoardStore` (the rusqlite
`Connection`), runs on `cx.background_spawn`, and is the **sole** thing that touches
the board db — honoring the MANDATORY off-thread-I/O rule. It receives typed commands
over a bounded channel and replies with the **committed** `BoardLayout` (+ a load
outcome). Commands: `Load`, `PlaceSessions(Vec<(ConnectionId, SessionId)>)` (batched),
and (for B-4b/c/d) the mutation ops (`SetCollapsed`, `MoveItem`, `CreateGroup`, …).
Each command runs the store op **and** returns the resulting layout, so the main thread
never reloads or re-reads the db. The worker owns store *reopen* (recovery — `mode` is
immutable, so recovering from degraded/failed requires a fresh `open`).

**`BoardReplica` (main-thread gpui entity, `crates/lens-ui/src/board/replica.rs`).**
Holds the layout replica + a handle to the worker + UI-facing state:

```
BoardReplica {
    worker: BoardStoreHandle,     // channel to the off-thread worker
    conn: ConnectionId,           // pinned to the app Connection.id (§4)
    layout: BoardLayout,          // last committed layout the worker sent back
    state: ReplicaState,          // Loading | Writable | Degraded | LoadFailed | Stale (§5)
    fleet: Entity<FleetStore>,    // observed for session-lifecycle reconcile (§3.3)
}
```

Interface:
- `BoardReplica::new(worker, conn, fleet, cx) -> Entity<Self>` — takes the fleet
  (fixing the review's finding #3), starts in `Loading`, spawns a `Load`, installs the
  `FleetStore` observer, and **immediately reconciles a snapshot of current
  `fleet.cards` keys** (cards may predate the subscription).
- `layout(&self) -> &BoardLayout`, `state(&self) -> ReplicaState`, `is_writable(&self)`.
- `write(&self, cmd, cx)` — enqueue a mutation command **iff** `is_writable()`; when the
  worker replies, apply the committed layout via `cx.update` + `cx.notify`. Async by
  construction (no blocking, no return-value layout). B-4b/c/d call this.
- `reconcile(&self, cx)` — diff `fleet.cards` keys vs placed `(conn, session)` keys;
  enqueue one **batched** `PlaceSessions` for the missing set (iff writable).
- `BoardReplica::in_memory_for_test(fleet, cx)` — `:memory:` worker + fixed conn.

Because the worker owns all I/O and the replica only does `cx.update`/`cx.notify`, the
UI thread never blocks (AGENTS.md), and there is no synchronous SQLite anywhere.

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

### 3.2 Write path — command → worker → committed layout (async, off-thread)

Every mutation is a command to the worker:

1. `BoardReplica::write` checks `is_writable()`; if not, it no-ops and surfaces the state
   (§5). No db access on the main thread.
2. The worker (off-thread) runs the store op in a transaction and, on commit, computes
   the resulting `BoardLayout`, replying `Ok(layout)` — or `Err` (commit failed / degraded).
3. On reply, the replica applies the committed layout via `cx.update` + `cx.notify`.

This **dissolves review finding #2** (commit-then-reload divergence): there is no separate
reload that can fail after a commit — the worker returns the layout it just committed
atomically. On a worker `Err`, the replica transitions to `Stale`/`Degraded` (§5) and
does **not** blindly retry a non-idempotent op (`create_group`).

**No inline cost model.** All SQLite is off-thread, so it never touches the frame budget.
Per AGENTS.md ("**MANDATORY** Benchmark-or-it's-not-done on perf paths; 120fps/8.3ms
target, 90fps/11.1ms regression line"), B-4a ships a **release-mode benchmark** of the
worker round-trip (Load + a batched PlaceSessions of N sessions) and asserts the main
thread stays within frame budget under a realistic and a stress fixture. (The earlier
"inline is fine below ~1000 items" reasoning is retired — it was both non-compliant and
un-benchmarked.)

### 3.3 Session-lifecycle reconcile (batched, additive, conn-pinned)

On each `FleetStore` change (and once at construction), `reconcile`:
- Computes placed keys as **`(ConnectionId, SessionId)`** (review #6 — a `SessionId`-only
  set cannot detect a cross-conn duplicate).
- Diffs live `fleet.cards` keys (paired with the pinned `conn`) against placed keys.
- Enqueues **one batched `PlaceSessions`** for the missing set (review #8 — k separate
  `place_session` calls each persist the board → ~O(k·N); one batched transaction is O(N)).
  Skips entirely (no worker traffic) when nothing is new — the frequent membership
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
the reply/next-op failed — read-only until reload). `is_writable()` is `Writable` only.
**Both `write` and `reconcile` gate on it** (review #4 — reconcile mutates too).

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

**Recovery** — because `StoreMode` is immutable (review #4), recovery = the worker
**reopens** the store on a retry, not a mere reload; success transitions back to `Writable`
and retires the banner.

**Mechanics:** the banner is a small **non-blocking, dismissible** notice over the board
area (never a modal; the rest of Lens stays usable), driven by `ReplicaState`.

---

## 6. Construction & startup order

- **App (live):** open the `BoardStoreWorker` on `data_dir/lens.db` **before the session
  actors start** (review #7 — minimizes `SQLITE_BUSY`), and give `open_db` a bounded
  **busy timeout** so a transient lock retries rather than fails. conn = `"lens-app"`. Wire
  `BoardReplica` into the `BoardView::mount` sites (`main.rs:110,165`).
- **Demo (fake):** in-memory worker (`:memory:`; `CONTROL_DDL` still makes the empty
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
The worker is async, so tests drive `run_until_parked` to settle worker replies.

- **Load renders persisted placements**; **new session placed + persists across reopen**.
- **Convergence / no double-place** with pinned conn → exactly one card row.
- **Group renders B-3 chrome via the real path** (migrated fixture test: seed group +
  members, assert `group_chrome_for_test`).
- **`.cached()` freeze regression (the real risk, review #10):** spawn a session →
  `run_until_parked` → the new card **renders and keeps animating** (reconcile fired, no
  freeze); seed a group → its **member cards render + tick** (closes the B-3
  `absolute_group` render-time member-read carryforward, `board/mod.rs:384`); assert
  **group-rollup freshness** when a member's cost changes.
- **State gating:** `Degraded`/`LoadFailed`/`Stale` replicas **refuse** `write` **and**
  `reconcile`; `LoadFailed` renders an empty **default-board** layout (not a panic);
  partial load (`skipped` non-empty) → `Degraded` + banner.
- **Tombstoned fleet key** stays absent with no reload/notify churn (review #9).
- **Reconcile idempotent / batched** — two reconciles → one row per session; a k-session
  batch issues one `PlaceSessions`.
- **Perf benchmark (MANDATORY):** release-mode worker round-trip (Load + batched
  PlaceSessions of N) stays within frame budget on realistic + stress fixtures.

Full `xtask gate` green.

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
