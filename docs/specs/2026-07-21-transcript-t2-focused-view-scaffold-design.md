# T-2 — Focused view scaffold + live disk-sourced surface (design)

**Date:** 2026-07-21
**Status:** REVISED (rev 2) after GPT-5.6 (codex) cross-family review — awaiting user
review, then `writing-plans`.
**Owner:** Lens design effort
**Type:** Implementation slice (build), transcript workstream T-2 of T-0..T-7 (+ T-2b).

Implements `docs/design/conversation-transcript.md` §16 (the scrolling surface) +
§17 (edge states: disk-paint → reconcile, historical hydration) — the **first real
consumer** of the T-1 `Vec<ViewBlock>` projection. Mounts a focused transcript into
the shell's `#chat-slot`, backed by a store-side replica that reads finalized items
from disk and splices the live tail from the actor's scratch, rendered through gpui's
native `list()`.

This is an **implementation decomposition** of an already-complete product design. It
does not reopen product questions; it resolves the lens-ui/gpui specifics and the
actor-feed consumption the render surface needs.

Sibling slices: **T-0** authoritative turn identity ✅ · **T-1** pure ViewBlock
projection ✅ · **T-2b** disk windowing + scroll-back paging + bounded-tail reconcile
(**next after T-2**) · T-3 content/markdown · T-4 tool spans + resource markers
(**+ live in-progress tool-tail feed extension**) · T-5 sub-agent spans · T-6 turn
lifecycle + `WorkSectionMeta` · T-7 composer & live turn.

---

## 0. What the code-map + review established (2026-07-21)

A read-only exploration mapped the actual code; the GPT-5.6 review (rev-2 below)
verified the load-bearing claims and broke several first-draft assumptions. Facts the
design now rests on, with citations:

- **The feed is single-consumer.** `(feed_tx, feed_rx) = async_channel::bounded(64)` is
  created in `FleetStore::spawn_live_session` (`fleet/store.rs:171`) and `feed_rx` is
  **moved into** `spawn_session_poller` (`store.rs:202`, `poller.rs:10`). No
  broadcast/tee; a cloned receiver would *steal* frames. A second detailed consumer must
  be reached **through the poller**. ✅ verified.
- **On focus the actor says "read from disk yourself."** `Promote` (`runloop.rs:518`)
  emits `Detailed(Rebased(scalars_baseline))` — `scalars_baseline` **clears `items`**
  (`runloop.rs:1141`, D23) — then flips `output.mode = Detailed`. ✅ verified.
- **The forward watermark exists but is NOT the whole disk-change story.**
  `TranscriptAdvanced { committed_ordinal }` (`reduce/update.rs:20`) is emitted after a
  terminal-status commit, `committed_ordinal = next_ordinal-1` (`runloop.rs:663,1188`).
  **But disk mutates below the watermark**: `reconcile_store_item`
  (`persist/transcript.rs:286-340`) does `UPDATE items SET item_id/kind/payload/response_id
  WHERE ordinal=?` and `DELETE … WHERE ordinal=? AND provisional=1` at **existing**
  ordinals during catch-up (`runloop.rs:379-416`); message reconciliation rewrites
  content at the same id/ordinal (`transcript.rs:611+`); a re-fire of an already-persisted
  id updates in place and emits **no** `TranscriptAdvanced` (`runloop.rs:2234` test).
  So a **forward-delta-only** model with `ord ≤ last_rendered ⇒ no-op` **misses
  authoritative below-watermark changes** — §3.4 adds a reconcile-range re-read. ⚠ this
  broke the first draft.
- **The finalize handoff is NOT atomic across scratch→disk.** On the canonical Message /
  `Completed`, the reducer clears `open_message` and pushes `ScratchChanged`
  **synchronously** (`reduce/mod.rs:118-145`), while the committed row reaches the
  replica only via an **async** disk read on `TranscriptAdvanced`. Naively dropping the
  streaming row on `ScratchChanged` leaves a frame with the row **absent** → flash.
  §6 stages the retirement. ⚠ this broke the first draft.
- **`finalize_message` derives the item id FROM `message_id`** (`reduce/items.rs:112-130`,
  falling back to a synthesized `local_id` when `message_id` is `None`). So for keyed
  messages the streaming id **equals** the finalized item id **by construction** — no
  byte-verification needed. The gaps are the `None` fallback and that **`ReasoningAcc`
  carries no id** (`domain/item.rs:129-142`). ✅ verified (simplifies the first draft).
- **`reduce/reconcile.rs` is pending-USER reconciliation, not transcript-id** — the
  first draft mis-cited it; item reconciliation lives in `persist/transcript.rs`. ✅
  corrected.
- **The transcript DB is per-session**, `{data_dir}/{session_id}.db`, WAL
  (`fleet/live.rs:71`). The actor owns an exclusive write `Connection` (`!Sync`,
  `persist/transcript.rs:17`). A second reader is WAL-compatible **but**
  `SqliteTranscriptStore::open` runs **DDL/migrations/metadata writes** (`persist/db.rs:36-67`)
  and sets **no busy timeout** — so it is not a safe read handle as-is (§3.3). ✅ verified.
- **`load_items` returns items WITHOUT ordinals** (`transcript.rs:253-260`); the frontier
  is a **separate** query (`transcript.rs:263-283`). Reading them independently observes
  two snapshots → §3.3 adds one transactional `(ordinal, Item) + watermark` primitive.
  ✅ verified.
- **`gpui::list()`'s render closure is `'static`** (gpui-0.2.2 `elements/list.rs:21-30`);
  the spike captures an entity and re-enters it, with `RowState` owning **cloned** text
  (`spikes/transcript-virtual/src/rowsource.rs:13-37`). A render-local **borrowed**
  `Vec<ViewBlock>` cannot be captured. §6 projects into **owned** presentations. ⚠ this
  broke the first draft's "no clone in the tree."
- **`GET /items` is already paginated** (`lens-client/src/sessions.rs:472,1292`).
- **`ContentTab` is an inert marker**; the mount seam is the concrete `TabHandle`
  (`slot/mod.rs:6,8`); `#chat-slot` renders a literal `"chat"` (`board/mod.rs:266`). ✅
  verified. Second WAL reader compatible once opened correctly. ✅ verified.
- **`ActorOutcome::TransportChanged` carries `reconcile_in_flight`** (`actor/outcome.rs:17`)
  but the poller **discards** it with `..` (`poller.rs:94-105`). §9 routes it. ✅ verified.

---

## 1. Scope & boundaries

**T-2 owns** the focused transcript **surface**: mount it; feed a store-side replica the
detailed frames; source finalized rows from disk (baseline + forward-delta + **reconcile
re-read**) and the live tail from scratch; project through T-1 into **owned row
presentations**; render through native `list()`; satisfy the four §16 scroll contracts;
meet the frame-budget perf gate. It renders **every** `ViewBlock` variant, using **stub**
content for T-3/T-4-owned blocks (the stubs are replaced, not extended around).

**T-2 does NOT own** (each → its slice):

| Concern | Why not T-2 | Slice |
|---|---|---|
| Byte-budgeted **windowed baseline** (don't load-all on open; bound resident RAM) | Scale; small/medium sessions work on load-all | **T-2b** |
| **Scroll-back paging** (load *older* items on scroll-up) | Scale | **T-2b** |
| **Bounded-tail** scoping of the reconcile re-read | Scale (T-2 re-reads the whole resident set on reconcile — correct but O(N); rare event) | **T-2b** |
| Rich message/reasoning content, tool-span archetypes | Own render efforts | T-3, T-4 |
| **Live in-progress tool-tail** (actor ships above-watermark working items) | A lens-core feed extension; belongs with tool-span render | **T-4** |
| `WorkSectionMeta` chip, composer/interrupt/elicitation | Later slices | T-6, T-7 |
| Polymorphic `ContentTab` protocol | Needs a 2nd real surface | future (SPEC-GAPS) |

**In T-2 for correctness (NOT deferred):** the reconcile-range re-read (below-watermark
changes, §3.4), the transactional read-only reader + busy timeout (§3.3), the staged
finalize handoff (§6), the per-frame-bounded owned-presentation render (§6/§8).

---

## 2. Architecture — the data flow

```
focus_session(id)  (fleet/store.rs)
  ├─ install FocusedTranscript replica FIRST (store-side; retains reader factory: data_dir+conn_id+id)
  │     └─ open READ-ONLY reader (busy_timeout) to {id}.db on a dedicated reader worker
  │     └─ enqueue baseline read (focus-generation G)  ── serialized through the reader worker ──┐
  ├─ Demote(prev), Promote(id)  → actor(id): Detailed, emits Rebased(scalars)                    │
  └─ poller drains each ActorFeed BATCH → FleetStore.fold_session_feed(id, batch, cx):           │
        Summary(u)                 → SessionCard.fold_summary  (chrome; unchanged)               │
        Detailed(Rebased scalars)  → replica: refresh scalars ONLY (never clear items)           │
        Detailed(ScratchChanged)   → replica: update live tail; STAGE finalize retirement        │
        Detailed(TranscriptAdvanced{ord}) → replica: enqueue forward-delta read (ord, G)         │
        Detailed(ActiveResponseChanged(r)) → replica: set active_response                        │
        Detailed(Reconnected{gap})  → replica: if gap != Some(0), inject ReconnectBreak marker   │
        (TransportChanged reconcile_in_flight true→false) → replica: enqueue reconcile re-read    │
        │                                                                                        │
        ▼  reader worker applies each result on the UI thread (drop if focus-generation != G):   │
     (ordinal, Item, watermark) rows → build owned RowPresentation → id-keyed upsert into RowStore┘
        │
        ▼  project on INPUT CHANGE (items/scratch/active_response), NOT per frame:
     project_all/project_filtered(&items, &scratch, active_response) → Vec<ViewBlock>  (T-1, borrow-only)
        → materialize OWNED RowPresentation per block → upsert retained Entity<RowState>
        → ListState::splice/reset on order/count/height change
     render: list(state, closure)  — closure is 'static: captures the entity + owned RowId order snapshot
```

Disk is authoritative for finalized rows; RAM scratch is the live tail; the staged
handoff (§6) bridges them without an absent frame.

---

## 3. Decisions

### 3.1 Feed fan-out = one poller draining BATCHES through `FleetStore`

The feed is single-consumer, so the poller stays sole receiver and **routes each drained
batch** (not frame-by-frame) via a `FleetStore` method — `fold_session_feed(session_id,
batch, cx)` — that fans `Summary` → the card and `Detailed` → the card's `fold_detailed`
(chrome) **and**, when focused, the `FocusedTranscript` replica. The poller captures a
**`WeakEntity<FleetStore>`** (a strong capture would cycle task↔entity). Batch routing
lets the replica recognize *scratch-clear + watermark in one batch* as a single finalize
episode (§6). No broadcast channel; recreating the actor with a second sender is
impossible post-spawn (it holds the sole `Sender`).

### 3.2 The replica lives store-side (fleet layer), installed before `Promote`

`FocusedTranscript` is a gpui `Entity` owned by `FleetStore`, **created on focus before
`Promote` is sent** (so it is ready for the `Rebased` + first frames), dropped on
`Demote`. `#chat-slot`'s `ContentTab` is a **pure renderer** of the replica. Store owns
data, view renders it (the shipped card pattern).

**Missing plumbing to add:** `FleetStore` currently discards `data_dir`/connection
context after spawn (`store.rs:64-70`). It must **retain, per session**, (a) a **reader
factory** (`data_dir` + `conn_id` + `session_id`) so the replica can open its reader, and
(b) the **current reconcile epoch/state** (§3.4, Imp-4) so a replica installed
mid-reconcile is seeded correctly rather than missing the falling edge. `focus_session`
gains access to both.

### 3.3 A dedicated reader worker with a read-only handle + transactional primitive

The actor keeps its exclusive write `Connection`. The replica does **all** disk reads on
**one dedicated background reader worker** (serialized — never independent spawns) owning
a `Box<dyn TranscriptReader + Send>`:

- **New `TranscriptReader` interface** — read/query-only, separate from the write
  `TranscriptStore`. Opened via a **read-only opener** that does **no** DDL/migration/
  metadata writes (unlike `SqliteTranscriptStore::open`) and sets a **bounded
  `busy_timeout`** (WAL readers can still see `SQLITE_BUSY`; the default handler is null).
- **One transactional read primitive** returning `Vec<(ordinal, Item)>` **plus the exact
  snapshot watermark**, in a single transaction — so items and frontier are one snapshot
  (`load_items` returns no ordinals and the frontier is a separate query → two snapshots,
  a race). Two shapes: forward-delta `(after, through]` and full-resident (T-2) /
  windowed (T-2b) baseline + reconcile re-read.
- **Focus-generation token `G`.** Every read is tagged with the focus generation; a
  result whose `G` ≠ the current focus is dropped, so a stale read from a prior focus
  can't land on the new session's rows.
- **Bounded request queue + typed error states** (`.agents/rust-ui.md:7` bounded-channel
  rule). The worker takes a **bounded latest-target queue**: forward-watermark targets
  **coalesce continuously** (only the highest pending `through` survives), reconcile
  re-reads take **priority**, and the baseline is the first target. Each read result is
  **`Retryable` (`SQLITE_BUSY` past the busy timeout) or `Fatal`**: a `Retryable` failure
  re-enqueues the same target with backoff; a `Fatal` failure surfaces an error state to
  the surface (not a silent blank). A read that fails while a row is in `pending_finalize`
  (§6) must **recover** — the staged presentation is retried/kept, never orphaned into a
  permanent ghost.
- Read transactions are short; a `Mutex` (if used) is never locked on the gpui thread.
- **The public read primitive lives on `TranscriptReader` only** (read-only); the write
  `TranscriptStore` shares nothing public with it beyond a **private** SQL row-decoder /
  query helper. (Rev-2 §3.4 wrongly called it a "write-side addition to `TranscriptStore`".)

### 3.4 Two disk-read paths: forward-delta (fast) + reconcile re-read (correctness)

- **Forward-delta (live growth):** on `TranscriptAdvanced{ord}` with `ord >
  last_rendered`, enqueue a `(last_rendered, ord]` read; id-keyed upsert; advance
  `last_rendered`.
- **Reconcile re-read (below-watermark changes):** provisional reconcile rewrites/deletes
  rows at **existing** ordinals (§0). Trigger via a **per-session reconcile epoch held in
  `FleetStore`** (seeded into the replica at creation), **not** only a locally-observed
  `reconcile_in_flight` true→false edge — a replica **created mid-reconcile sees only the
  falling edge's `false`** and would take a baseline over half-reconciled disk (Imp-4). So:
  `FleetStore` retains the current reconcile epoch/state; the replica is seeded from it;
  **completion of any epoch that overlapped the replica's baseline forces a re-read.** The
  re-read is a **full resident-range re-read** + **id-keyed reconcile** against the
  RowStore (changed ids update in place, folded-away provisional rows removed, new ids
  inserted). O(resident), only on the (rare) reconcile episode; **T-2b** bounds it to the
  tail. Upsert-by-id keeps it flash-free.
- **Known gap — silent in-place updates (re-fire):** a re-fire of an already-persisted id
  updates an existing ordinal and emits **no** `TranscriptAdvanced` (`runloop.rs:2234`
  test); the scaffold emits two `output_item.done` for one `fc_*` id (D23). These are
  invisible to the forward-delta path and are only corrected at the next reconcile episode.
  **Fix (rev-3 decision needed, small):** either (a) the actor emits a lightweight
  below-watermark-changed signal on the in-place-update path, or (b) accept bounded
  staleness until the next reconcile. (a) is preferred if cheap; flagged for the
  disk-change-signal-completeness discussion.

The read primitive backing the reader is public on **`TranscriptReader`** (§3.3), sharing
only a private decoder with the write store; both T-2 and T-2b consume it.

### 3.5 `ReconnectBreak` = replica-injected synthetic marker (gap ≠ Some(0))

No `ReconnectBreak` exists; by design it has no backing item (why T-1 deferred it). It is
a **UI-only** marker injected into the row order — not an `Item`, not projection output.

- **Widen** `StreamUpdate::Reconnected` to `Reconnected { gap: Option<u64> }` (it carries
  none today; `reduce/update.rs:62`). Minor additive lens-core change.
- **Condition:** inject on `gap != Some(0)` — matching the reducer, which treats every
  value **except `Some(0)`** as a discontinuity (`reduce/snapshot.rs:98-111`); `None` is a
  discontinuity too (the first draft's "None ⇒ no marker" was backwards).
- **Lifecycle (honest limitation):** markers live in the ephemeral focused replica and are
  **lost on `Demote`**; a gap while unfocused (Summary mode delivers no detailed frames)
  is never marked. **T-2's success criterion is narrowed to "gaps observed while
  continuously focused."** A durable per-session discontinuity ledger (survives defocus)
  is deferred to **T-6** (turn-lifecycle/reconnect-break render owner). Never persisted as
  an `Item`.
- **Temporal anchor (not "the tail").** A marker carries `{ after_ordinal, seq }` — the
  resident ordinal it follows plus a monotonic sequence — so **every full reprojection
  re-inserts it deterministically** at the same position (Imp-5). Storing only
  `(RowId, Marker)` and injecting "at the tail" lets it float to the newest tail or vanish
  during order reconciliation. It occupies a synthetic `RowId` outside the item-id space,
  merged into projected order by `after_ordinal`.

---

## 4. Home & module layout

New module tree in **lens-ui** (`crates/lens-ui/src/`):

- `focused/mod.rs` — `FocusedTranscript` replica (state, batch folding, staged finalize).
- `focused/reader.rs` — the dedicated reader worker + `TranscriptReader` client + focus-
  generation gating.
- `focused/rowsource.rs` — production `RowSource`/`RowStore` (id-keyed retained entities;
  **owned `RowPresentation`**; `ListState::splice/reset` discipline) lifted from the spike.
- `focused/view.rs` — the gpui `Render` surface: `list()` wiring, scroll contracts, stub
  row renderers. Built via `focused_transcript_tab(replica, cx) -> TabHandle`.
- `slot/mod.rs` — add `focused_transcript_tab`; **`ContentTab` untouched**.
- `fleet/store.rs`, `fleet/poller.rs` — retain the reader factory; `fold_session_feed`
  batch routing via `WeakEntity`.

**lens-core touches (small but real):** the transactional/ranged read primitive +
read-only opener + `busy_timeout` (`persist/`); widen `StreamUpdate::Reconnected` with
`gap`; **add an id to `ReasoningAcc`** threaded to `finalize_reasoning` for stable
reasoning identity (mirroring `finalize_message`'s `message_id` — confirm the exact
threading against `finalize_reasoning` in planning). Each cross-family reviewed.

---

## 5. The `FocusedTranscript` replica

**State:** `items: Vec<Item>` (resident finalized transcript; ordinal-keyed) ·
`scratch: Arc<StreamScratch>` · `active_response: Option<ResponseId>` ·
`last_rendered_ordinal: i64` · `rows: RowStore` (id-keyed retained `Entity<RowState>`
holding **owned** `RowPresentation`) · `pending_finalize: HashMap<RowId, RowPresentation>`
(staged §6) · `markers: Vec<Marker>` where `Marker { after_ordinal, seq, kind }` (§3.5
anchor) · `focus_generation: u64` · the reader worker handle + `session_id`.

**Batch fold rules:**

| Frame | Replica action |
|---|---|
| `Rebased(scalars)` | Update status/title/active-response scalars **only**. Never clear `items` (append-only would remount every row). Baseline read was enqueued at **create** (§2). |
| `ScratchChanged(s)` | `self.scratch = s`. If an accumulator that was open is now cleared **because it finalized** (not abandoned — see ⚠ below), **stage** its last presentation into `pending_finalize` keyed by its RowId (§6) — do **not** drop the row. Re-project. |
| `TranscriptAdvanced{ord}` | If `ord > last_rendered`: enqueue forward-delta `(last_rendered, ord]` read (gen G). |
| `ActiveResponseChanged(r)` | `self.active_response = r`. Re-project (grouping flips). |
| `Reconnected{gap}` | If `gap != Some(0)`, inject a `ReconnectBreak` marker anchored at the current tail ordinal (§3.5). |
| `TransportChanged{reconcile_in_flight}` | Drive the debounced `syncing…` indicator (§9); on the reconcile-epoch edge, enqueue the reconcile re-read (§3.4). |

> ⚠ **Finalize-vs-abandon (design decision D-2, §13).** A cleared accumulator can mean
> *finalized* (a `Message`/`Completed` committed → a disk row is coming) **or** *abandoned*
> (a reconnect discontinuity clears scratch **without** committing — `reduce/snapshot.rs:98`).
> Staging both produces a **permanent ghost row** in the abandon case. Distinguishing them
> is unresolved and drives the row-identity model (§13). Do **not** implement the
> `ScratchChanged` staging rule until D-2 lands.

**Projection** runs **on input change** (any of `items`/`scratch`/`active_response`
mutated), not per frame — the full staged pipeline (a bare `project_all` never groups):

```
chat:    group_work_section( project_all(&items, &scratch, ar),               ar )
history: group_work_section( project_filtered(hide_reasoning(&items), &scratch, ar, false), ar )
```

`ar = active_response.as_ref()`. `group_work_section` (Stage 3) is what emits
`WorkSection`; `project_*` alone is Stage 2 only (`view.rs:93,110`). Then materialize owned
presentations → upsert (§6). **Per-frame cost of this is design decision D-1 (§13).**

---

## 6. RowSource, owned presentations, and the staged finalize (the crux)

**Why owned, not borrowed.** `project_*` returns `Vec<ViewBlock<'a>>` borrowing `items`/
`scratch`; `gpui::list()`'s closure is **`'static`** and cannot capture a render-local
borrowed `Vec`, and a `'static Entity<RowState>` cannot retain a borrowed `ViewBlock`
(§0). So T-1's borrow-only projection stays a pure fn, and **T-2 materializes each block
into an owned, minimal `RowPresentation`** (kind + text/flags the stub renderer needs —
not the whole `Item`). The bounded per-row copy is accepted; **"zero clone in the render
tree" is not a workable invariant** and is dropped. Projection runs on input change; the
`list()` closure captures only the entity + an owned `Vec<RowId>` order snapshot and
re-enters the entity to render its owned presentation.

**`splice`, not `reset`, for live changes (New-Crit-3).** `ListState::reset` sets
`logical_scroll_top = None`, which under `ListAlignment::Bottom` means **bottom-follow** —
so a routine `reset` on any change **yanks a paused (scrolled-up) reader to latest**,
violating §16 "don't yank" (gpui `list.rs:241`). Therefore: **`reset` is reserved for
initial mount / new-session replacement**; all live changes use **minimal `splice`
diffs**, and every content-mutated row whose height may change is explicitly invalidated.
Tested while scrolled up (contract 1).

> ⚠ **The row-identity model below is UNDER REVISION pending design decision D-3 (§13).**
> The GPT-5.6 re-review showed the flat-top-level-row identity story is wrong for agent
> work: finalized reasoning and tool spans do **not** stay top-level rows — Stage 3 folds
> them into a `WorkSection` keyed by `response_id` (`view.rs:148`). So a streaming
> reasoning row is **not** a stable top-level RowId that survives into a finalized
> top-level row; it becomes a **child of a grouped section**. The mapping is therefore
> row→(section child), not row→row. D-3 settles how retained entities key onto the grouped
> projection. The keyed-**message** case (below) is unaffected and correct.

**Stable `RowId` — the keyed-message case (correct, D-3-independent).** A message row's
`RowId` is assigned when the accumulator opens (its `message_id`) and reused on finalize:
because `finalize_message` derives the item id **from** `message_id` (§0), the committed
row's id **equals** the streaming RowId. The `None`-message and reasoning/tool cases are
D-3 (they need either an accumulator-id correlation or the section-child model).

**Staged retirement (no absent frame).** Finalize is **not** atomic: `ScratchChanged`
clears the accumulator synchronously; the disk row arrives on a later async read (§0).
So on a **finalizing** clear (not an abandoning one — D-2), the replica **keeps rendering
the last accumulator presentation** (stashed in `pending_finalize`) instead of dropping
the row; when the forward-delta read delivers the committed row for that RowId, it
**swaps in place** (same `Entity`) and clears the staged entry. Batch routing (§3.1) lets
the replica see scratch-clear + watermark as one episode, minimizing the interval. The row
is **never absent**; its `EntityId` never changes. (Exact keying depends on D-2 + D-3.)

**Test (MANDATORY, real-window harness — `#[gpui::test]` fakes the text system and
false-greens paint/identity, per [[gpui-test-noop-text-system]]/[[terminal-realwindow-harness-pitfalls]]):**
a streaming→finalize sequence asserts, **on every intervening paint**, that the message
row's `EntityId` is unchanged, the row is **present** (row count never dips), content is
correct, and `ListOffset` (bottom-pin) holds. Endpoint-only `EntityId` equality is
insufficient.

---

## 7. The scroll surface — the four §16 contracts

Native `list()` / `ListState` / `ListAlignment::Bottom` (spike verdict
[[transcript-virtualization-spike-2026-07]], 7/7):

1. **Stick-to-bottom, don't yank** — `ListAlignment::Bottom` auto-follows while pinned;
   scroll-up **pauses** auto-follow (logical anchor off the tail); resume on
   scroll-to-bottom / pill.
2. **`↓ N new · jump to latest` pill** — only when scrolled up; `N` = rows appended since
   pause; click → bottom + resume.
3. **Scroll anchoring** on finalize / above-viewport height change — `list()` compensates
   above-viewport reflow (spike 1b held); the id-keyed upsert + staged finalize (§6) keep
   the anchor (no remount/absent row to jump from). Live changes go through **`splice`**
   (never `reset` — §6, New-Crit-3); content-mutated rows are height-invalidated.
4. **New-session jump** — open lands at bottom (fresh `ListState` + `Bottom`) — the one
   place `reset` is used.

`list()` gives **render virtualization** (windowed *painting*) for free — distinct from
T-2b's **disk windowing** (bounded resident *set*). Whether per-frame/per-delta work is
actually O(visible) rather than O(resident) is **design decision D-1 (§13)** — the
off-frame projection still re-runs the whole staged pipeline on every scratch delta today.

---

## 8. Surface reuse — Chat column vs History view

The same replica + `list()` backs the **Chat column** (T-7 adds the composer) and the
read-only **History view** (§18, no composer). Only projection differs: the History view
runs Stage-1 `hide_reasoning` and therefore calls `project_filtered(.., splice_reasoning
=false)`, or live reasoning leaks past the filter (T-1 §5.2). No separate history
renderer (§17).

---

## 9. Edge states (§17)

- **Disk-paint → reconcile.** On focus, the replica paints from SQLite instantly
  (baseline read), then the actor's transport-only reconnect + forward catch-up advances
  the watermark (forward-delta) and reconciles provisional rows (reconcile re-read, §3.4).
  Flash-free (id-keyed). **Debounced `syncing…`** shows only if reconcile takes >~150 ms —
  driven off `ActorOutcome::TransportChanged.reconcile_in_flight`, which the poller
  **currently discards** (`poller.rs:94`) and must now route to the replica; the 150 ms
  debounce + cancellation is tested.
- **Empty session** — clean empty state; composer (T-7) docks below.
- **Historical hydration** — `GET /items` items land on disk via the actor and reach the
  replica through the same forward-delta/reconcile paths — one projection (§17).

---

## 10. Testing strategy

Beyond the §6 mandatory finalize test:

- **Replica batch-fold units** (fake fleet + in-memory `TranscriptReader`): each frame
  drives the documented state change; `Rebased` refreshes scalars without touching
  `items`; `TranscriptAdvanced` enqueues exactly one `(after, through]` read; a stale
  `ord ≤ last_rendered` is a no-op **on the forward path** but below-watermark changes are
  caught by the reconcile path.
- **Baseline/delta ordering** — a delta arriving before baseline installs must not
  regress; baseline + deltas serialize through the reader worker; watermark targets
  coalesce until baseline lands.
- **Below-watermark reconcile** — a provisional row rewritten/rekeyed/deleted at an
  existing ordinal (via `reconcile_store_item`) is reflected after the reconcile re-read;
  a tool-id rekey maps old→new RowId without remount.
- **Stale-read gating** — a read completing after a focus switch (generation changed) is
  dropped and cannot land on the new session's rows.
- **Focus-mid-reconcile** (Imp-4) — a replica installed while `reconcile_in_flight` is
  already `true` still performs the re-read on epoch completion (seeded from `FleetStore`),
  not just on a locally-observed falling edge; its baseline never retains stale
  below-watermark rows.
- **Re-fire / silent in-place update** (partial-1) — a duplicate `output_item.done` for a
  persisted id that emits no `TranscriptAdvanced` is nonetheless reflected (via whichever
  D-of-§3.4 fix lands); assert no stale row survives a reconcile.
- **Unkeyed finalize** (D-3) — a `None`-`message_id` streaming message finalizes to a
  `local_id` row with no absent frame / no remount (once D-3 fixes the correlation).
- **Reconnect-discard, no ghost** (D-2) — a reconnect discontinuity that clears scratch
  **without** committing must **not** leave a permanent staged `pending_finalize` row.
- **Marker position persistence** (Imp-5) — a `ReconnectBreak` survives N full reprojections
  at its `after_ordinal` anchor; it neither floats to the tail nor vanishes.
- **Paused-scroll not yanked** (New-Crit-3) — a live change while scrolled up uses `splice`
  and does **not** jump the viewport to bottom; only new-session `reset` re-pins.
- **Reconnect semantics** — `Some(0)` → no marker; `None` and `Some(N>0)` → exactly one
  marker; a gap while unfocused produces none (narrowed criterion).
- **Concurrent reader/writer** — reader tolerates `SQLITE_BUSY` (retryable) under the busy
  timeout while the actor writes; a `Fatal` read surfaces an error state, not a blank.
- **Poller fan-out** — one `Detailed` batch updates card chrome + focused replica; an
  unfocused session's batch never touches a replica.
- **`syncing…` debounce** — shows only >150 ms; cancels if reconcile finishes sooner.
- **Perf gate (release-mode benchmark)** — steady-state re-project + upsert + paint at
  realistic transcript sizes stays within the frame budget (`.agents/performance.md`
  8.3 ms/11.1 ms; `.agents/rust-ui.md` allocation-light). Proves the off-frame projection
  actually bounds per-frame cost to O(visible).

Real-window harness for identity/paint/scroll (the run is the only proof); in-memory
`TranscriptReader` for fold logic.

---

## 11. Dependencies

- **On T-0/T-1** (done) — liveness + the projection API + `splice_reasoning` seam.
- **Lifts** the RowSource spike to production (owned presentations, not fixtures).
- **`GET /items` pagination** — already in lens-client.
- **New lens-core surface T-2 introduces** (each cross-family reviewed): the
  transactional/ranged `TranscriptReader` read primitive + read-only opener +
  `busy_timeout`; `StreamUpdate::Reconnected { gap }`; an id on `ReasoningAcc` threaded to
  `finalize_reasoning`; routing `reconcile_in_flight` to the focused replica.
- **Blocks:** T-2b (windowed baseline + scroll-back + bounded reconcile scoping), T-3
  (message/reasoning stubs), T-4 (tool-span stubs + live-tool-tail feed extension).
- **Coordination:** `terminal-ws` concurrently touches `reduce/` — T-2's `reduce/update.rs`
  touch (`Reconnected { gap }`) and `persist/` additions are small; second-to-merge
  reconciles.

---

## 12. Success criteria

- Focusing mounts a live transcript in `#chat-slot`; blurring tears it down; the replica
  is installed **before** `Promote`.
- Finalized rows come from disk; the live tail from scratch; the **staged finalize** shows
  **no absent frame and no remount** (row `EntityId` stable, row count never dips) —
  proven on every intervening paint in a real-window run.
- Below-watermark provisional changes (rewrite/rekey/delete) are reflected via the
  reconcile re-read; forward growth via forward-delta; both flash-free.
- Reads are serialized through one worker, opened **read-only with a busy timeout**,
  transactional `(ordinal, Item) + watermark`, and **focus-generation-gated**.
- All four §16 scroll contracts hold; `↓ N new` pauses/resumes; `ListState` invalidation
  correct.
- Steady-state render meets the frame budget (release benchmark); per-frame cost is
  O(visible), not O(resident).
- `ReconnectBreak` appears on `gap != Some(0)` while continuously focused; never persisted.
- Every `ViewBlock` variant renders (stubs for T-3/T-4); none panics or is dropped.
- `xtask gate` green.
- No byte-budgeted windowing / scroll-back paging / bounded-reconcile-scoping leaks into
  T-2 (those are T-2b); `ContentTab` left an inert marker.

---

## 13. Open design decisions (blocking plan-readiness)

The mechanical review findings are folded into §§3–10 above. Three decisions remain — they
are coupled (D-3 is the spine; D-1 and D-2 hang off it) and are **not** mechanical:

### D-3 — How do retained row entities key onto a projection that *regroups*?

**The tension.** T-1's projection is not a stable flat list: while a turn is live its work
streams as flat top-level blocks (`StreamingReasoning`, `ToolSpan`, …); when it settles,
`group_work_section` **folds that run into one `WorkSection`** keyed by `response_id`
(`view.rs:148`). So the *same* content changes shape — top-level rows → children of a
grouped section — exactly at finalize. The rev-2 "assign a stable top-level RowId per
streaming block, reuse it on the finalized row" is therefore wrong for agent work:
reasoning/tool spans don't stay top-level rows. (Keyed **messages** are unaffected — they
stay top-level siblings and `finalize_message` preserves the id.)

**Candidate models** (to discuss): **(A)** retained entities key on **two levels** — a
stable entity per `WorkSection` (`response_id`) that itself owns a stable child list keyed
by child id; live turn = flat children promoted to top level, settled = same children under
a collapsed section, no remount either way. **(B)** the live turn renders flat top-level
rows, and finalize *does* remount into a section — but only the **latest** turn (bounded,
one-time), accepting a single reflow at collapse (which §4 already treats as expected).
**(C)** don't group in T-2 at all — render everything flat, defer `WorkSection` rendering to
T-6 (which owns collapse/meta anyway), so T-2's identity model is genuinely flat.

### D-2 — Finalize vs abandonment of a cleared accumulator

A cleared accumulator means *finalized* (disk row coming) or *abandoned* (reconnect
discontinuity clears scratch with no commit, `snapshot.rs:98`). The replica must
distinguish them or it stages a permanent ghost row. Options: **(i)** infer from the batch
(§3.1 batch routing — finalize co-occurs with an item/watermark; abandon co-occurs with
`Reconnected`); **(ii)** reducer-owned typed retirement (`Finalizing` vs `Discarded`) with
stable accumulator ids (a lens-core change, and the natural home for the id-correlation D-3
also needs). (ii) is cleaner but larger.

### D-1 — Per-frame/per-delta projection cost

Even off-frame, re-running the full staged pipeline (`project_all` + `group_work_section` +
materialize) on **every scratch delta** is O(resident); streaming deltas are frequent, and
the frame budget is 8.3/11.1 ms (`.agents/performance.md`). Options: **(x)** accept
O(resident) in T-2, bounded by the T-2b resident window (i.e. T-2 is correct-but-unshippable
until T-2b caps N — make that explicit); **(y)** incremental projection — only the live
tail re-projects per delta, settled sections are cached; **(z)** a hybrid keyed on D-3's
section model (a settled `WorkSection` entity is immutable ⇒ never re-projected). Ties
directly to D-3.

**Recommendation to open discussion:** D-3→**(A)** (two-level section entities) makes D-1→**(z)**
(settled sections cached/immutable) and D-2→**(ii)** natural and mutually reinforcing — but
(A) is the most up-front modelling. The cheaper path is D-3→**(C)** (flat in T-2, group in
T-6) + D-1→**(x)** (bounded by T-2b) + D-2→**(i)** (batch inference) — smaller T-2, but it
pushes grouping semantics into T-6 and leans on T-2b for the perf gate. This is the crux to
decide before the plan.
