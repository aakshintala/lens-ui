# T-2 — Focused view scaffold + live disk-sourced surface (design)

**Date:** 2026-07-21
**Status:** DESIGN — awaiting user review, then `writing-plans`.
**Owner:** Lens design effort
**Type:** Implementation slice (build), transcript workstream T-2 of T-0..T-7 (+ T-2b).

Implements `docs/design/conversation-transcript.md` §16 (the scrolling surface) +
§17 (edge states: disk-paint → reconcile, historical hydration) — the **first real
consumer** of the T-1 `Vec<ViewBlock>` projection. Mounts a focused transcript into
the shell's `#chat-slot`, backed by a store-side replica that reads finalized items
from disk (D23) and splices the live tail from the actor's scratch, rendered through
gpui's native `list()`.

This is an **implementation decomposition** of an already-complete product design. It
does not reopen product questions; it resolves the lens-ui/gpui specifics and the
actor-feed consumption the render surface needs.

Sibling slices (STATUS "transcript fan-out"): **T-0** authoritative turn identity ✅ ·
**T-1** pure ViewBlock projection ✅ · **T-2b** disk windowing + scroll-back paging +
bounded-tail reconcile (**next after T-2**) · T-3 content/markdown · T-4 tool spans +
resource markers (**+ live in-progress tool-tail feed extension**) · T-5 sub-agent
spans · T-6 turn lifecycle + `WorkSectionMeta` · T-7 composer & live turn.

---

## 0. What the code-map established (2026-07-21)

Before scoping, a read-only exploration mapped the *actual* current state (memories
were stale). Load-bearing facts:

- **The feed is single-consumer.** `(feed_tx, feed_rx) = async_channel::bounded(64)` is
  created in `FleetStore::spawn_live_session` (`crates/lens-ui/src/fleet/store.rs:171`)
  and `feed_rx` is **moved into** `spawn_session_poller` (`store.rs:202`,
  `fleet/poller.rs:10`). There is **no broadcast/tee**; a second `async_channel`
  receiver would *steal* frames, not fan them out. So a second detailed consumer must
  be reached **through the existing poller**, not via a new channel.
- **On focus, the actor says "read from disk yourself."** `SessionCommand::Promote`
  (`actor/runloop.rs:518`) emits `Detailed(Rebased(scalars_baseline(state)))` — and
  `scalars_baseline` **clears `items`** (`runloop.rs:1141`, D23) — then flips
  `output.mode = Detailed`. `Rebased` carries **no items**; the baseline transcript
  comes from disk.
- **The three deltas T-2 needs already exist** (`reduce/update.rs`): `TranscriptAdvanced
  { committed_ordinal }` (`:20`, = highest ordinal on disk inclusive, `next_ordinal-1`),
  `ScratchChanged(Arc<StreamScratch>)` (`:26`), `ActiveResponseChanged(Option<ResponseId>)`
  (T-0). `ItemAppended/Updated` are **deleted** (D23). The card poller currently
  **no-ops** `TranscriptAdvanced`/`ActiveResponseChanged` with comments reserving them
  for "the T-2 transcript replica" (`card/model.rs:236,244`).
- **The transcript DB is a per-session file**, `{data_dir}/{session_id}.db`, WAL
  (`fleet/live.rs:71`, `open_stores`). The actor owns an exclusive write `Connection`
  (`SqliteTranscriptStore { conn: Connection }`, `persist/transcript.rs:17`;
  `Connection` is `!Sync`). WAL permits a **second read connection** to the same file.
- **`TranscriptStore` has no ranged read** — only full-table `load_items`
  (`persist/transcript.rs:253`) and `store_frontier`. No `WHERE ordinal < ? LIMIT`,
  no byte-budget, no bounded-tail reconcile.
- **`GET /items` is already paginated** (`after`/`before`/`limit`/`order` cursor,
  `lens-client/src/sessions.rs:472,1292`). The "Bucket-C blocking dep" the earlier
  handoff flagged on T-2 is effectively satisfied on the network side.
- **`ContentTab` is an inert marker.** `pub trait ContentTab {}` — no methods, nothing
  bounds on it; the real mount seam is the concrete `TabHandle { view: AnyView, title,
  focus_handle }` (`slot/mod.rs:6,8`). `#chat-slot` renders a literal `"chat"` string
  (`board/mod.rs:266`).
- **The RowSource machinery exists in the spike** (`spikes/transcript-virtual/`):
  `RowSource` trait, id-keyed retained `RowStore { order: Vec<RowId>, entities:
  HashMap<RowId, Entity<RowState>> }`, `finalize_handoff(UpsertById|ClearRecreate)`,
  and native `list(list_state, closure)` with `ListState::new(n, ListAlignment::Bottom,
  OVERDRAW)`. It is fixture-driven; no production disk-sourced upsert.

---

## 1. Scope & boundaries

**T-2 owns:** the focused transcript **surface** — mount it, feed a store-side replica
the detailed frames, source finalized rows from disk and the live tail from scratch,
project through T-1, render through native `list()`, and satisfy the four §16 scroll
contracts. It renders **every** `ViewBlock` variant, using **stub** content for the
blocks T-3/T-4 own (message/reasoning markdown, tool-span archetypes) — the stubs are
replaced, not extended around.

**T-2 does NOT own** (each → its slice):

| Concern | Why not T-2 | Slice |
|---|---|---|
| Byte-budgeted **windowed baseline** (don't load-all on open) | Scale; small/medium sessions work on load-all | **T-2b** |
| **Scroll-back paging** (load *older* items on scroll-up) | Scale; forward-delta suffices for the live surface | **T-2b** |
| **Bounded-tail reconcile** (scope reconcile to the resident tail) | Scale; full-history reconcile is a >1s stall only on multi-day sessions | **T-2b** |
| Rich **message/reasoning** content (markdown, safe-prefix) | Its own vendor+patch effort | T-3 |
| **Tool-span** archetypes / native tools / resource-marker render | Its own render effort | T-4 |
| **Live in-progress tool-tail** (actor shipping above-watermark working items) | A lens-core *feed extension*; in-progress tool spans first matter where tool-span render lives | **T-4** |
| `WorkSectionMeta` chip (duration/model/tokens/cost) | Needs per-turn data T-1/T-2 can't supply | T-6 |
| Composer / interrupt / elicitation dock | The chat closer | T-7 |
| Polymorphic **`ContentTab`** mount protocol | Needs a *second* real UI surface to design against; deferred to terminal-UI-integration (SPEC-GAPS cross-spec-risks) | future |

---

## 2. Architecture — the data flow

```
focus_session(id)  (fleet/store.rs)
  ├─ Demote(prev), Promote(id)           → actor(id) flips to Detailed, emits Rebased(scalars)
  └─ create FocusedTranscript replica    (store-side entity; opens a 2nd READ conn to {id}.db)
        │
        ▼  baseline (async, off-UI-thread): load_items → id-keyed upsert into RowStore
        │
   ┌────┴─────────────── the ONE poller (per session) dispatches each ActorFeed frame ──┐
   │  Summary(u)                 → SessionCard.fold_summary  (chrome; unchanged)         │
   │  Detailed(Rebased scalars)  → replica: refresh scalars ONLY (never clear items)     │
   │  Detailed(ScratchChanged)   → replica: live tail (StreamingMessage/Reasoning)       │
   │  Detailed(TranscriptAdvanced{committed_ordinal})                                    │
   │                             → replica: forward-delta read (last_rendered, ord] →    │
   │                               id-keyed upsert (flash-free finalize handoff)         │
   │  Detailed(ActiveResponseChanged(r)) → replica: set active_response (liveness)       │
   │  Detailed(Reconnected{gap})         → replica: inject ReconnectBreak marker if gap  │
   └─────────────────────────────────────────────────────────────────────────────────────┘
        │
        ▼  render (per frame):
     project_all/project_filtered(&items, &scratch, active_response) → Vec<ViewBlock>   (T-1)
        → map each block to a stable row id → list()/ListState/ListAlignment::Bottom
        → each retained RowState entity renders STUB content from its ViewBlock
```

Disk is the source of truth for finalized rows; RAM scratch is the source for the live
tail above the watermark. The two never overlap (D23 split-at-the-watermark).

---

## 3. Forced decisions (the code leaves no real alternative)

### 3.1 Feed fan-out = one poller, two sinks — dispatch centralizes in `FleetStore`

There is no broadcast on the feed and the receiver is already owned by the poller, so
the poller stays the **sole** consumer and **fans out**: `Summary` → the card;
`Detailed` → the card's existing `fold_detailed` (chrome scalars) **and**, when this
session is focused, the `FocusedTranscript` replica.

The replica is created *after* the poller (on focus), so the poller cannot capture it
at spawn. Resolution: the poller folds each frame through a **`FleetStore` method**
(e.g. `fold_session_feed(session_id, frame, cx)`) that owns the routing —
`FleetStore` gains `focused_transcript: Option<(SessionId, Entity<FocusedTranscript>)>`
and routes `Detailed` frames to it when `session_id` matches. This centralizes
dispatch and keeps the poller a thin pump. (Alternatives — a broadcast channel, or
recreating the actor with a second sender — are rejected: the former is new plumbing
for one consumer, the latter is impossible post-spawn since the actor holds the sole
`Sender`.)

### 3.2 The replica lives store-side (fleet layer), not in the view

`FocusedTranscript` is a gpui `Entity` owned by `FleetStore`, created on `Promote`,
dropped on `Demote` — the same ownership/lifecycle as `SessionCard`. The `#chat-slot`
`ContentTab` is a **pure renderer** that reads the replica entity. Rationale: the
poller (in the fleet layer) drives the replica; if the view owned it, the poller would
need a handle *into* a view — a backwards seam. Store owns data, view renders it
(consistent with the shipped card pattern and the state-model "SessionStore is a gpui
replica" decision).

### 3.3 The replica opens its own read connection; all reads off the UI thread

The actor keeps its exclusive write `Connection`. The replica opens a **second
`TranscriptStore` read handle** to `{data_dir}/{session_id}.db` (WAL → concurrent
reads are safe while the actor writes). Every disk read (baseline + forward-delta)
runs in a background task; results are applied to the RowStore on the UI thread. The
replica holds the read handle behind a `Send` guard usable from the background task
(mechanism — dedicated reader task vs `Mutex<Connection>` vs per-read open — is a plan
detail; `Connection` is `Send` but `!Sync`).

The read handle is the `TranscriptStore` **trait**, so the fake fleet (tests) injects
an in-memory implementation and the replica is exercisable without a real DB.

### 3.4 One small store primitive is in-scope: a forward-delta ranged read

`TranscriptStore` gains **one** method — read `(after_ordinal, through_ordinal]` in
ordinal order (`WHERE ordinal > ? AND ordinal <= ? ORDER BY ordinal`). This is what
makes D23's "replica reads `(last_rendered, committed_ordinal]`" real; without it the
replica re-scans the whole table on every commit. It is **not** T-2b: T-2b is the
byte-budgeted *window* (bounded resident set), *backward* scroll-back paging, and
*bounded-tail reconcile*. The forward-delta read is a forward tail-growth primitive
both slices use.

T-2 baseline load = full `load_items`. T-2b swaps that baseline for a byte-budgeted
tail window and adds the backward page primitive.

### 3.5 `ReconnectBreak` is a replica-injected synthetic marker

No `ReconnectBreak` exists anywhere, and by design it has **no backing item** (why T-1
deferred it). It is a **UI-only** marker the replica injects into its row order — not
an `Item`, not a projection output. Trigger: `StreamUpdate::Reconnected` carrying a
**real gap**. The reducer computes a reconnect `gap: Option<u64>` (`snapshot.rs:98`)
but `StreamUpdate::Reconnected` may not currently carry it — if not, **widen the
variant to `Reconnected { gap: Option<u64> }`** (a minor, additive lens-core touch;
cross-family reviewed). The marker occupies a synthetic `RowId` (markers get ids
outside the item-id space) or renders as an inter-row separator (plan detail). `↻`
appears **only on a real gap** (§17), never on a clean reconnect.

---

## 4. Home & module layout

New module tree in **lens-ui** (`crates/lens-ui/src/`):

- `focused/mod.rs` — `FocusedTranscript` replica entity (state + feed folding).
- `focused/rowsource.rs` — production `RowSource` + `RowStore` lifted from the spike
  (id-keyed retained entities, upsert-not-recreate).
- `focused/view.rs` — the gpui `Render` surface: `list()` wiring, scroll contracts,
  stub row renderers. Constructed via `focused_transcript_tab(replica, cx) -> TabHandle`.
- `slot/mod.rs` — add the `focused_transcript_tab` factory; **`ContentTab` untouched**.

The **one** lens-core touch: the forward-delta ranged read on the `TranscriptStore`
trait + its `SqliteTranscriptStore` impl (`persist/`), and — if needed — widening
`StreamUpdate::Reconnected` to carry `gap` (`reduce/update.rs` + emit site).

---

## 5. The `FocusedTranscript` replica

**State:**

- `items: Vec<Item>` — the resident finalized transcript (T-2: the whole thing; T-2b:
  a windowed tail). Canonical for projection input.
- `scratch: Arc<StreamScratch>` — the latest live tail (from `ScratchChanged`).
- `active_response: Option<ResponseId>` — liveness (from `ActiveResponseChanged`).
- `last_rendered_ordinal: i64` — high-water mark of resident finalized items.
- `rows: RowStore` — id-keyed retained `Entity<RowState>` (identity across `list()`
  recycle + finalize handoff).
- `markers: Vec<(RowId, Marker)>` — synthetic UI-only rows (e.g. `ReconnectBreak`).
- the `TranscriptStore` read handle + `session_id`/`conn_id`.

**Fold rules (the detailed frames):**

| Frame | Replica action |
|---|---|
| `Rebased(scalars)` | Update status/title/active-response scalars **only**. Never clear `items` (append-only → clearing would remount every row = the D23 anti-pattern). Independent of the baseline load, which is kicked at replica **creation** (§2 — the read handle + session id are known then; disk already holds the committed prefix for an existing session). |
| `ScratchChanged(s)` | `self.scratch = s`; re-render (live tail changes). |
| `TranscriptAdvanced{ord}` | If `ord > last_rendered_ordinal`: background forward-delta read `(last_rendered, ord]` → on UI thread, id-keyed **upsert** into `rows`, extend `items`, set `last_rendered = ord`. |
| `ActiveResponseChanged(r)` | `self.active_response = r`; re-render (a turn went live/settled → grouping flips). |
| `Reconnected{gap}` | If `gap` is a real gap, inject a `ReconnectBreak` marker at the current tail. |

**Projection at render:** `project_all(&self.items, &self.scratch, self.active_response
.as_ref())` (or `project_filtered(.., splice_reasoning=false)` for the History-view
caller running `hide_reasoning` — see §8). The result is a transient `Vec<ViewBlock>`
whose lifetime is the `render` call; the `list()` closure indexes it.

---

## 6. RowSource, row identity, and the flash-free finalize (the crux)

**Impedance mismatch.** Projection is borrow-only and transient (`Vec<ViewBlock<'a>>`,
`'a` = the resident `items`). The `RowStore` is retained id-keyed entities so `list()`
recycle and (T-3's) markdown state survive across frames. T-2 bridges them:

1. Each `render`, project into a local `Vec<ViewBlock>`.
2. Map each block to a **stable `RowId`**:
   - `Item(it)` → the item's store id.
   - `ToolSpan { call, .. }` → the call item's store id.
   - `WorkSection { response_id, .. }` → derived from `response_id`.
   - `StreamingMessage(acc)` → `acc.message_id`; `StreamingReasoning(acc)` → the
     reasoning stream id.
   - injected markers → their synthetic `RowId`.
3. **id-keyed upsert**: a row id already in `RowStore` reuses its `Entity<RowState>`;
   a new id mints one. **Never clear-and-recreate** (spike-proven: clear-recreate
   remounts the viewport; upsert preserves entity id, markdown init, and bottom-pin —
   D23 MANDATORY).
4. The retained entity renders **stub** content from the current `ViewBlock` (T-3/T-4
   replace the stubs).

**The flash-free finalize hazard (correctness-critical).** A streaming message renders
as `StreamingMessage(acc)` keyed by `acc.message_id`. When it finalizes, the item is
committed to disk and read back as `Item`, keyed by its **store id**. If
`message_id != store_id`, the upsert sees the streaming row *disappear* and a *new*
committed row *appear* → remount → flash + scroll-jump — the exact failure the upsert
exists to prevent. Per [[omnigent-two-id-space-reconciliation]] the live SSE id and the
store id **can** differ (proven for scaffold `fc_*` function-calls; **unverified for
messages**).

**Resolution + build order:**
1. **First, byte-verify** whether the streaming `message_id` equals the persisted item
   id for the target harnesses (reuse the golden SSE captures / a live rider). If they
   match, use `message_id` directly as the row id and the hazard evaporates.
2. If they differ, the replica adopts the store id for the provisional streaming row on
   finalize, using the **existing** id reconciliation (`reduce/reconcile.rs`, the D16/D19
   item-id reconcile that already maps live↔store ids) — the streaming row keeps its
   `Entity`, only its `RowId` key is rekeyed.
3. Either way: a **mandatory test** asserts the `EntityId` of the message row is
   identical before and after finalize (the spike's negative control was `EntityId
   86v1→31v3` on clear-recreate; the invariant is *no* remount).

---

## 7. The scroll surface — the four §16 contracts

Native `list()` / `ListState` / `ListAlignment::Bottom` (spike verdict
[[transcript-virtualization-spike-2026-07]], 7/7). T-2 implements:

1. **Stick-to-bottom, don't yank.** `ListAlignment::Bottom` auto-follows while pinned
   (`logical_scroll_top()` reads `item_ix == count` at bottom). The moment the user
   scrolls up, auto-follow **pauses** (detect via the logical anchor moving off the
   tail). Resume on scroll-to-bottom or pill click.
2. **`↓ N new · jump to latest` pill** — shown **only when scrolled up**; `N` counts
   rows appended since auto-follow paused. Click → scroll to bottom + resume.
3. **Scroll anchoring** on finalize / above-viewport height change — `list()`
   compensates above-viewport reflow (spike 1b go/no-go **held**); the id-keyed upsert
   (§6) is what makes the anchor hold (no remount to jump from).
4. **New-session jump** — opening a session lands at the bottom (`ListAlignment::Bottom`
   + fresh `ListState`).

`list()` handles **render virtualization** (windowing of *painting*) for free — this is
distinct from T-2b's **disk windowing** (bounding the resident *item set*). T-2 paints
windowed over a fully-resident `items`.

---

## 8. Surface reuse — Chat column vs History view

The same `FocusedTranscript` + `list()` surface backs both the **Chat column** (T-7
adds the composer) and the read-only **History view** (§18, no composer). The only
projection difference: the History view for archived/sleeping sessions runs the
Stage-1 `hide_reasoning` filter and therefore **must** call `project_filtered(..,
splice_reasoning=false)`, or live reasoning would leak past the filter (T-1 spec §5.2,
the `splice_reasoning` seam). T-2 wires the Chat-column caller (unfiltered); the
History-view caller is a thin variant. No separate "history renderer" (§17).

---

## 9. Edge states (§17)

- **Disk-paint → reconcile.** On focus, the replica paints from SQLite instantly
  (baseline `load_items`), then the actor's transport-only reconnect + forward
  catch-up (`GET /items`, existing) advances the watermark; the replica reads forward
  and upserts. Content is flash-free (id-keyed; §6). A **debounced `syncing…`
  indicator** shows only if reconcile takes >~150 ms (drive off the existing
  `ActorOutcome::TransportChanged { reconcile_in_flight }` the poller already handles
  for the card overlay).
- **Empty session.** Clean empty state; the composer (T-7) will dock below. T-2 shows
  the empty transcript.
- **Historical hydration.** Items from `GET /items` land on disk via the actor and
  reach the replica through the *same* `TranscriptAdvanced` → forward-read path as live
  — one projection, no separate path.

---

## 10. Testing strategy

- **Replica fold unit tests** (fake fleet + in-memory `TranscriptStore`): each detailed
  frame drives the documented state change; `Rebased` refreshes scalars without
  touching `items`; `TranscriptAdvanced` triggers exactly one forward-delta read of the
  right range; out-of-order / duplicate `TranscriptAdvanced` (ord ≤ high-water) is a
  no-op.
- **Flash-free finalize (MANDATORY, §6):** a streaming message → finalize sequence
  asserts the message row's `EntityId` is **unchanged** across the transition
  (real-window harness, per [[gpui-test-noop-text-system]] / [[terminal-realwindow-harness-pitfalls]]
  — `#[gpui::test]` fakes the text system and would false-green paint/identity; the run
  is the only proof).
- **Scroll contracts:** stick-to-bottom pins at the tail; scroll-up pauses auto-follow
  and shows the pill with the correct `N`; pill click resumes at bottom; append while
  scrolled-up does **not** yank. (Real-window harness; the virtualization spike proved
  the primitives — these test *our* pill/pause state machine over them.)
- **ReconnectBreak:** `Reconnected{gap: Some}` injects exactly one marker at the tail;
  `gap: None` injects none.
- **Poller fan-out:** one `Detailed` frame updates both the card chrome and the focused
  replica; an unfocused session's frame never touches a replica.
- **Forward-delta read primitive** (lens-core): ranged read returns `(after, through]`
  in ordinal order; empty range → empty; respects `UNIQUE(ordinal)`.

---

## 11. Dependencies

- **On T-0** (done) — `active_response` liveness + `ActiveResponseChanged`.
- **On T-1** (done) — `project_all`/`project_filtered`/`group_work_section`, the
  `ViewBlock` enum, the `splice_reasoning` seam.
- **On the RowSource spike** — lifted from `spikes/transcript-virtual/` to production.
- **`GET /items` pagination** — already in lens-client (no new work).
- **Blocks:** T-2b (swaps baseline→window, adds scroll-back + bounded reconcile), T-3
  (fills message/reasoning stubs), T-4 (fills tool-span stubs + the live-tool-tail feed
  extension).
- **Coordination:** `terminal-ws` concurrently touches `reduce/`; T-2's lens-core touch
  is small (one ranged-read method + maybe the `Reconnected` widening) — second-to-merge
  reconciles.

---

## 12. Success criteria

- Focusing a session mounts a live transcript in `#chat-slot`; blurring tears it down.
- Finalized rows come from disk (D23 split-at-watermark); the live tail (streaming
  message + reasoning) comes from scratch; the two never overlap or duplicate.
- The streaming→finalized handoff is **flash-free** (row `EntityId` unchanged) — proven
  in a real-window run.
- All four §16 scroll contracts hold; the `↓ N new` pill pauses/resumes correctly.
- Every `ViewBlock` variant renders (stubs for T-3/T-4 content); no variant panics or
  is dropped.
- `ReconnectBreak` appears only on a real gap.
- `xtask gate` green (fmt/clippy -D warnings/tests/drift).
- No byte-budgeted windowing, scroll-back paging, or bounded-tail reconcile leaks into
  T-2 (those are T-2b); `ContentTab` is left an inert marker.
```
