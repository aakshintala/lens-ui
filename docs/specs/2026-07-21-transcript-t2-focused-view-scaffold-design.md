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
context after spawn (`store.rs:64-70`). It must **retain a per-session reader factory**
(`data_dir` + `conn_id` + `session_id`) so the replica can open its reader. `focus_session`
gains access to that context.

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
- Read transactions are short; a `Mutex` (if used) is never locked on the gpui thread.

### 3.4 Two disk-read paths: forward-delta (fast) + reconcile re-read (correctness)

- **Forward-delta (live growth):** on `TranscriptAdvanced{ord}` with `ord >
  last_rendered`, enqueue a `(last_rendered, ord]` read; id-keyed upsert; advance
  `last_rendered`.
- **Reconcile re-read (below-watermark changes):** provisional reconcile rewrites/deletes
  rows at **existing** ordinals (§0). So on a **reconcile episode** — detected by
  `TransportChanged.reconcile_in_flight` transitioning **true→false** (routed to the
  replica, §9) — enqueue a **full resident-range re-read** and **id-keyed reconcile**
  against the RowStore: changed ids update in place, folded-away provisional rows are
  removed, new ids inserted. This is O(resident) but only on the (rare) reconcile
  episode; **T-2b** bounds it to the resident tail. Upsert-by-id makes it flash-free.

The one small write-side addition to `TranscriptStore`/`SqliteTranscriptStore` is the
ranged/transactional read primitive backing the reader (shared by T-2 and T-2b).

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
- Occupies a synthetic `RowId` outside the item-id space (or renders as an inter-row
  separator — plan detail).

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
(staged §6) · `markers: Vec<(RowId, Marker)>` · `focus_generation: u64` · the reader
worker handle + `session_id`.

**Batch fold rules:**

| Frame | Replica action |
|---|---|
| `Rebased(scalars)` | Update status/title/active-response scalars **only**. Never clear `items` (append-only would remount every row). Baseline read was enqueued at **create** (§2). |
| `ScratchChanged(s)` | `self.scratch = s`. If an accumulator that was open is now cleared, **stage** its last presentation into `pending_finalize` keyed by its RowId (§6) — do **not** drop the row. Re-project. |
| `TranscriptAdvanced{ord}` | If `ord > last_rendered`: enqueue forward-delta `(last_rendered, ord]` read (gen G). |
| `ActiveResponseChanged(r)` | `self.active_response = r`. Re-project (grouping flips). |
| `Reconnected{gap}` | If `gap != Some(0)`, inject a `ReconnectBreak` marker at the tail. |
| `TransportChanged{reconcile_in_flight}` | Drive the debounced `syncing…` indicator (§9); on true→false, enqueue the reconcile re-read (§3.4). |

**Projection** runs **on input change** (any of `items`/`scratch`/`active_response`
mutated), not per frame: `project_all(&items, &scratch, active_response.as_ref())` (or
`project_filtered(.., splice_reasoning=false)` for the History-view/`hide_reasoning`
caller, §8) → materialize owned presentations → upsert (§6).

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
re-enters the entity to render its owned presentation. `ListState::splice`/`reset` is
called whenever order/count/height changes (spike `NOTES.md:196-203`).

**Stable `RowId` assignment (identity across finalize).** A `RowId` is assigned **when an
accumulator opens**, reused when it finalizes:

- **Message** — `message_id` if present, else a **session-monotonic synthetic id** (the
  `None` case). Because `finalize_message` derives the item id from `message_id` (§0),
  the committed row's id **equals** the streaming RowId for keyed messages; for the
  synthetic case the replica maps its synthetic RowId → the finalized `local_id`.
- **Reasoning** — needs an id (`ReasoningAcc` has none): **add one** (§4) so streaming and
  finalized reasoning share a RowId; else a synthetic-id map as for messages.
- `Item` → store id · `ToolSpan{call}` → call's store id · `WorkSection{response_id}` →
  from `response_id` · markers → synthetic ids.

**Staged retirement (no absent frame).** Finalize is **not** atomic: `ScratchChanged`
clears the accumulator synchronously; the disk row arrives on a later async read (§0).
So on the clearing `ScratchChanged`, the replica **keeps rendering the last accumulator
presentation** for that RowId (stashed in `pending_finalize`) instead of dropping the
row. When the forward-delta read delivers the committed row for that RowId, the replica
**swaps the presentation in place** (same RowId, same `Entity`) and clears the
`pending_finalize` entry. Batch routing (§3.1) lets the replica see scratch-clear +
watermark as one episode, minimizing the staged interval. The row is **never absent**;
its `EntityId` never changes.

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
   the anchor (no remount/absent row to jump from). `ListState::splice/reset` on
   order/count/height change.
4. **New-session jump** — open lands at bottom (fresh `ListState` + `Bottom`).

`list()` gives **render virtualization** (windowed *painting*) for free — distinct from
T-2b's **disk windowing** (bounded resident *set*). Per-frame work is O(visible) because
projection is cached off-frame (§6), not recomputed per paint.

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
- **Reconnect semantics** — `Some(0)` → no marker; `None` and `Some(N>0)` → exactly one
  marker; a gap while unfocused produces none (narrowed criterion).
- **Concurrent reader/writer** — reader tolerates `SQLITE_BUSY` under the busy timeout
  while the actor writes.
- **`ListState` invalidation** — order/count mutation calls `splice`/`reset`; no stale
  window.
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
```
