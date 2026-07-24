# T-2b — Disk windowing, scroll-back paging & bounded-tail reconcile — design

**Date:** 2026-07-22
**Workstream:** `lens-ui` transcript fan-out → T-2b (disk-scale)
**Depends on:** T-2 (focused replica, landed on `main` `60425d2`)
**Status:** design rev 2 — grok-4.5 cross-family review folded (4 Critical + 5 Important +
Minors; review at `scratchpad/t2b-grok-review.md`). Awaiting user review before planning.

---

## 1. Context & problem

T-2 built the focused transcript replica (`FocusedTranscript`, `crates/lens-ui/src/focused/`)
at **small scale**: the baseline read is `ReadRange::All`, so `FocusedTranscript.items:
Vec<Item>` holds the **entire** transcript, and every downstream path is O(transcript):

- baseline `ReadRange::All` clones the whole transcript into `items`;
- `on_reconcile_epoch_settled` → `ReadRange::All` re-reads the whole transcript;
- `reproject` / `recompute_settled_prefix` / `compute_expansion_flags` iterate all of `items`.

The [[large-transcript-latency-spike-2026-07]] measured the cost: full-history work is a
**>1s stall** on multi-day sessions (370×–3100× slower than a bounded tail). The **entire
O(transcript) cost collapses to O(resident-window)** the moment `items` becomes a bounded
window instead of the full transcript — because the projection/collapse code already
operates over `self.items`.

**Scope is replica-side, and that is not a punt.** The actor write-side is *already*
bounded: `run_catchup` (`actor/runloop.rs:319`) is forward-only from `store_frontier()`,
fetching `/items?after=<frontier>` in fixed pages (D19) and never re-reading history it
holds on disk. The full-history `TranscriptStore::reconcile(&[Item])` has **no actor
caller** (grep: only the Board replica's board-layout reconcile + `spikes/large-transcript`
+ `transcript.rs` tests). This is sound because omnigent items are append-only/immutable
([[state-model-d23-disk-render]]): supersession, compaction, and `/clear` all *append* past
the frontier. So the spike's "reconcile bounded-tail, NEVER full history" contract was
already implemented by P3-3's D19 design. T-2b adds a regression test to lock the invariant.

## 2. Goals / non-goals

**Goals**
- `FocusedTranscript` holds a **bounded, byte-budgeted, LRU-evicted resident window**, not
  the whole transcript.
- **Scroll-back paging**: scrolling up loads older pages from disk; a top sentinel marks
  more history above; a turn straddling the window top renders as a partial section with a
  **window-invariant** identity.
- **Scoped reconcile re-read**: reconcile-epoch-settle re-reads only the resident band,
  not `ReadRange::All`, and de-ghosts correctly.
- Carried Minors: **M1** (RowStore entity GC) and **M4** (drop `rusqlite` from lens-ui).
- **Seeded demo** as the permanent visual + latency/RAM acceptance rig.
- Multi-day sessions are correct and smooth; the spike's latency/RAM numbers hold in-product.

**Non-goals**
- No content rendering changes (stubs stay; T-3/T-4 own content).
- No actor / network changes. `/items` pagination already ships in lens-client and is the
  actor's concern; the replica reads only from disk.
- No new collapse semantics — grouping is unchanged; only section *identity* changes (§5).

## 3. Read primitives (lens-core `TranscriptReader`)

New `ReadRange` variants. The byte-budget variants iterate newest-first and break on
accumulated `length(payload)` — **not** a SQL running-sum window (which scans the whole
table); per the spike.

```rust
pub enum ReadRange {
    All,                                    // kept for tests / tiny sessions; leaves prod baseline+reconcile
    Delta { after: i64, through: i64 },     // forward live growth — UPSERT, unchanged
    One   { ordinal: i64 },                 // TranscriptRewritten re-read, unchanged
    Tail     { byte_budget: usize },        // NEW — baseline: newest rows up to budget
    Backward { before: i64, byte_budget: usize }, // NEW — scroll-back page: rows `< before` up to budget
    Span     { from: i64, through: i64 },   // NEW — scoped reconcile re-read, inclusive; REPLACE-in-range
}
```

SQL shapes (all wrapped in the existing single-txn `read_range`):

- **`Tail`**: `SELECT <cols>, length(payload) FROM items ORDER BY ordinal DESC` → accumulate
  `length(payload)`, break once the budget is exceeded (always yield ≥1 row). Rows ascending.
- **`Backward`**: same, with `WHERE ordinal < ?before`.
- **`Span`**: `SELECT <cols>, length(payload) FROM items WHERE ordinal >= ?from AND ordinal
  <= ?through ORDER BY ordinal`.

**`RangeRead` carries per-row byte length (grok M11).** The whole byte-budget + eviction
accounting depends on knowing each resident row's payload size without re-hitting disk. So:

```rust
pub struct RangeRead {
    pub rows: Vec<(i64, usize, Item)>,  // (ordinal, payload_len_bytes, item)  — usize is NEW
    pub skipped: Vec<SkippedRow>,
    pub watermark: Option<i64>,          // newest NON-provisional ordinal
}
```

`watermark` is returned by every variant, as today. All existing call sites (T-2 apply
paths, `read_range` tests) update to the 3-tuple.

### 3.1 Delta (upsert) vs Span (replace) — why both, and Span's true band

`Delta` upserts (insert-or-update by id; **never removes**) — correct for forward growth.
`Span` **replaces in range** (its rows are the *complete* truth for `[from,through]`; any
resident row in that ordinal band not present is dropped). Replace is required because a
reconcile can **delete** a row, and upsert cannot subtract:

> Live appends provisional `fc_live` at ordinal 5. Actor catch-up folds the tool call into
> its durable store row `msg_store` (already at ordinal 0, a **different id** — the
> two-id-space hazard, [[omnigent-two-id-space-reconciliation]]); the fold **deletes**
> ordinal 5 (`transcript.rs:366–383`, golden test `reconcile_when_store_id_already_present_
> deletes_provisional`). A tail re-read now simply *omits* ordinal 5. A `Delta` upsert adds
> `msg_store` but leaves `fc_live` resident → **ghost** (`upsert_read_rows`, `mod.rs:620–630`
> never removes absent ids). `Span` drops resident ordinals in-band absent from the read.

**Critical (grok C1): the reconcile Span must span the resident band by ordinal, NOT the
watermark.** The watermark is the newest *non-provisional* ordinal; the deleted provisional
ghost sits **above** it (in the example watermark = 0 while the ghost is at ordinal 5). So
reconcile is `Span { from: resident_lo, through: resident_hi }`, where `resident_hi` is the
true max resident ordinal (provisional included) — a **separate cursor** from `watermark`
(§4). Replace-drop covers `[resident_lo, resident_hi]`. Add a unit test that replays the
fc_live@5 / msg_store@0 fold under a **windowed** replica and asserts no ghost.

The distinction is additive-vs-subtractive and is **orthogonal to the substrate key** — it
holds whether `items` is a `Vec` deduped by id or an ordinal-keyed map.

### 3.2 M4 (lens-core half)

- `PersistError::is_busy(&self) -> bool` — defined **in terms of the existing
  `is_transient`** (which already matches `DatabaseBusy|DatabaseLocked`,
  `persist/mod.rs:43–51`), not a divergent predicate. It encapsulates the check lens-ui
  currently open-codes (`reader.rs:233–240`).
- A rusqlite-free `PersistError::synthetic_busy()` constructor gated on `test-util` (or
  `cfg(test)` re-exported), so lens-ui test fakes inject busy without depending on
  `rusqlite`.

## 4. Resident window (`FocusedTranscript`)

`items` stops being the whole transcript and becomes a **contiguous, byte-budgeted band in
ordinal space, anchored on the viewport.**

### 4.1 Substrate + cursors + migration checklist

Replace `items: Vec<Item>` with an **ordinal-keyed `BTreeMap<i64, Item>`** (identity for
render stays `ItemId`; keep `item_ordinals: HashMap<ItemId,i64>` as the reverse index the
marker/GC code needs, plus `item_bytes: HashMap<ItemId,usize>` for eviction accounting).
Range-delete gives `Span` replace + eviction; range-scan gives `Backward` prepend directly.

**Cursors — kept explicitly distinct (grok C1/C3):**
- `resident_lo: i64` / `resident_hi: i64` — the loaded band (min/max resident ordinal,
  provisional included). `resident_hi` is what reconcile `Span` and de-ghosting use.
- `known_committed: i64` — the newest committed ordinal the actor has told us about
  (watermark from reads + advanced by `TranscriptAdvanced`). **May advance while
  `resident_hi` lags** (tail evicted / paused). Drives the "↓ N new" count.
- `last_rendered_ordinal: i64` — the forward `Delta` cursor: last ordinal actually applied
  into `items`. Must **not** jump ahead of `resident_hi` without a load.
- `resident_bytes: usize` — running sum of `item_bytes`; the eviction trigger.

**BTreeMap migration is an explicit T2 checklist, not "for free" (grok I6).** Every current
`usize`-index consumer of `items` moves to ordinal-native:
- `live_section_start: usize` + `items.iter().position` → an ordinal cursor
  (`live_section_lo: Option<i64>`); the `items[..lo]` / `items[lo..]` slices become
  BTreeMap range scans (`range(..lo)` / `range(lo..)`).
- `latest_settled_before_next_user` compares **vec indices** today (`mod.rs:519–548`) →
  compare **ordinals**.
- `upsert_read_rows` uses `*ordinal as usize` as a **vec index** (`mod.rs:623–628`) — already
  latently wrong for sparse ordinals; goes ordinal-native (BTreeMap insert by key). The
  migration *fixes* this latent bug (a window starting at `resident_lo > 0` would break the
  index arithmetic today).
- `seed_item` / tests move to ordinal keys.

consts: `TAIL_BUDGET_BYTES` (~8 MiB), `PAGE_BUDGET_BYTES` (~2–4 MiB), `RESIDENT_CAP_BYTES`
(~24 MiB). Named, tunable; seeded from the spike, finalized by the T6 sweep.

### 4.2 Load directions & the `apply_read` contract

- **Baseline**: `Tail{ TAIL_BUDGET_BYTES }` → `items` = tail rows; `resident_lo` = min
  ordinal read; `resident_hi` = **max ordinal read** (provisional included, *not* watermark);
  `known_committed = watermark`.
- **Scroll up**: `Backward{ before: resident_lo, PAGE_BUDGET_BYTES }` → **prepend**; lower
  `resident_lo`.
- **Scroll down / follow**: re-extend `resident_hi` toward `known_committed` via forward
  `Delta{ after: resident_hi, through: min(resident_hi+page, known_committed) }`.
- **Reconcile-epoch-settle**: `Span{ from: resident_lo, through: resident_hi }` (§3.1).

**`apply_read` must treat `Span` like `All` for projection (grok I5).** Today `full_replace
= matches!(range, ReadRange::All)` gates `reproject(true)` + `settled_structure_len = 0`
(`mod.rs:313–341`). `Span` replaces-in-band on the substrate **and** must trigger full
settled invalidation + `reproject(true)`; otherwise RowStore structure keeps ghost `Work`
rows until a coincidental full path. Extend the `match range` arm exhaustively so every new
variant (Tail/Backward/Span) sets `resident_hi`/`known_committed`/`last_rendered` correctly.

### 4.3 LRU eviction

After any load, if `resident_bytes > RESIDENT_CAP_BYTES`, **evict the end farther from the
viewport** (drop rows from `items` + `item_ordinals` + `item_bytes` + `resident_bytes`, GC
their RowStore entities — §6). Only ends are trimmed, so the band stays **contiguous**;
update `resident_lo`/`resident_hi`. Never evict rows carrying live/pending state (a
`pending_finalize` tail is scratch/AccId-keyed, never in `items`; a just-committed disk row
near the tail is protected because following keeps the tail resident). The evicted band is
cheaply re-loadable (`Backward` for the top, `Tail`/`Delta` for the bottom).

### 4.4 Following, the "↓ N new" pill, and forward-delta gating (grok C3)

If the user scrolled up and LRU evicted the live tail (`resident_hi < known_committed`), an
incoming `TranscriptAdvanced{ord}` must **not** pull those rows into the window — they are
off-screen below. It advances `known_committed` only. Concretely:

- **Following signal**: a thin replica flag `following: bool` (grok I8), set by the view
  from `FollowMode` (§4.5 seam). `TranscriptAdvanced` enqueues the forward `Delta` **only
  when `following` (⇔ `resident_hi` is at the tail)**; otherwise it just bumps
  `known_committed`.
- **Pill count is ordinal-based, not row-count** (today's `row_count − rows_at_pause`,
  `view.rs:70–76`, would stay 0 when rows aren't loaded): while paused/gated the pill shows
  `known_committed − resident_hi` (committed-but-unresident).
- **`jump_to_latest` reloads the tail**: it must enqueue `Tail` (or `Delta{ after:
  resident_hi }`) and set `following = true` **before/with** the scroll — today it only
  `scroll_to` + sets Following (`view.rs:102–111`), which under eviction would jump to the
  resident bottom, not the true latest.
- **Marker anchoring** (`reinsert_markers`/`entry_repr`, `rowsource.rs:437–479`) keys on
  `item_ordinals`; a reconnect marker must anchor at `resident_hi` (a resident ordinal), not
  a gated `known_committed` that has no resident row, or `entry_repr` degrades to `i64::MAX`.

### 4.5 Following seam (T2/T3 vs T4)

Promote a thin `replica.set_following(bool)` (or `wants_live_tail`) into **T2/T3** so §4.4
gating is unit-testable at the replica against the real signal; **T4** wires scroll/jump to
drive it. T3 tests set the flag directly; T4 owns the FollowMode→flag plumbing.

### 4.6 What is unchanged

`reproject`, staged finalize, collapse timing, the live RAM tail (scratch, AccId-keyed) —
unchanged in behavior; they become O(window) once the substrate is bounded and the API is
ordinal-native (§4.1).

## 5. Scroll-back UX, section identity & anchor (`focused/view.rs`, `reduce/view.rs`, `rowsource.rs`)

- **Top sentinel** row (`RowKind::LoadOlder`) at the top of the list whenever `resident_lo >
  0`. Scroll-near-top triggers a `Backward` page; a **single-in-flight guard** + the reader
  `Priority::Page` slot (grok I7 — the coalescer currently has no `Backward`/`Tail`/`Span`
  slot; add `Priority::Page` coalescing keep-lowest-`before`, map `Tail`→Baseline,
  `Span`→Reconcile) prevent duplicate/interleaved reads.

- **Section identity is anchored to the run's first-child `ItemId`, not a window-relative
  counter (grok C4 — a T-1 amendment).** Today `run_index` counts prior runs of the same
  `response_id` *in the passed blocks* (`reduce/view.rs:155–184`); if the window holds only
  the later run of an interleaved response and a `Backward` page then loads the earlier run,
  the visible section renumbers 0→1 → a **different `SectionKey`** → chip/rail entities
  remount, expansion identity lost. Fix — make section identity **window-invariant**:

  - `SectionKey { response_id: ResponseId, run_anchor: ItemId }` where `run_anchor` = the
    `ItemId` of the run's **first block** (disk-stable; a run's members are the same items
    regardless of what's above, so its first-child id never changes with the window).
  - `RowId::Section(ResponseId, ItemId)` / `RowId::SectionRail(ResponseId, ItemId)` follow.
  - `group_work_section` stamps `run_anchor` instead of computing a counter. Display ordering
    still follows chronological block order; only the *key* changes.

  This is the same class of T-1 amendment T-2 already made (response-keyed grouping). Add a
  test: interleaved two-run response truncated to the later run → page in the earlier run →
  the upper section's chrome EntityIds **do not churn**.

- **Partial section at the boundary**: a turn whose earlier blocks are above `resident_lo`
  renders as a partial rail with only its resident children; more attach as pages load — now
  safe, because the `run_anchor` is stable. `pair_tool_spans` (`view.rs:35–82`) assumes
  call+output co-reside; an output-only-in-window row is a transient orphan until the call's
  page loads — acceptable boundary UX, covered by the sentinel; note it explicitly.

- **Scroll anchor (the real-window risk)**: prepending to the bottom-aligned `ListState`
  must keep visible content stationary. Proven by **extending `focused_scroll_probe`** (real
  GPUI window, sandbox-disabled, honest `process::exit` codes — [[t2-real-window-probe-sandbox]],
  [[gpui-list-scroll-and-realwindow-probe-gotchas]]): `scroll_by/scroll_to` don't fire the
  scroll handler, and `visible_range` is pre-scroll while `is_scrolled` is post-scroll, so
  the decision is extracted + unit-tested and the *paint* proven in the probe (not
  `#[gpui::test]` — NoopTextSystem false-greens paint).

## 6. M1 — RowStore entity GC

`entities: HashMap<RowId, Entity<RowState>>` never removes entries when rows leave
`order`/`structure`; with paging + eviction this grows unbounded and churns. Add a sweep
after reproject/eviction that **retains exactly**:

- everything in `order`, ∪
- for every live `SectionKey` in `sections`: its `chip_id`, its `rail_id`, **and every
  `child ∈ section.children`** (grok C2 — collapsed sections keep children in
  `SectionNode.children` but **not** in `order`, `rowsource.rs:502–511`; dropping them yields
  blank rows on expand, `view.rs:183–188`, + a remount flash on the next full materialize),
- chip/rail for every `SectionKey` in `pending_tail_section.values()` (a scratch-cleared
  finalize can need the section chrome reachable only via that map, `rowsource.rs:328–343`),
- `StreamTail(acc)` for every `acc` in `pending_finalize`, ∪
- markers present in `structure`.

Everything else is dropped. **Ordering constraint**: GC runs only *after*
`overlay_pending_finalize` has re-attached structure (else a mid-finalize tail is
transiently unreferenced). Correctness-sensitive → **sabotage-verified** test: collapse a
section → GC → re-expand → the children's **EntityIds** must be unchanged (a naive "retain
`order` only" must visibly fail). Follows [[false-green-probe-drives-production-path]].

## 7. M4 — drop `rusqlite` from lens-ui

- `focused/reader.rs::is_sqlite_busy` → `err.is_busy()` (§3.2).
- Test fakes inject busy via `PersistError::synthetic_busy()`.
- Remove `rusqlite` (and `bundled`) from `crates/lens-ui/Cargo.toml` (dep + dev-dep).
- Verify `cargo tree -p lens-ui | grep rusqlite` is empty (only lens-core pulls it).

## 8. Demo & acceptance (last task)

- **Seeded** (not live — live needs omnigent) large `.db`: thousands of items, bimodal
  sizes (dumps + markers) like the spike. Generated by an `xtask` (or seeded at demo
  startup) into the demo data dir; a `ReaderFactory` points at it.
- `lens-app --features demo` mounts a `FocusedTranscript` in `#chat-slot` fed by the seeded
  reader (today the pane is empty — no reader factory seeded). Permanent visual-acceptance
  rig (like the terminal demo).
- **Out-of-gate latency/RAM sweep** (like terminal `rss-sweep`): seed → measure baseline
  focus latency, backward-page latency, and that `resident_bytes` holds ≤ `RESIDENT_CAP` at
  1k/10k/50k items. **Cold-first-focus latency measured explicitly** (the spike flagged it
  as warm-cache-only unmeasured). Confirms the spike numbers in-product; the tuning gate for
  the byte-budget consts.

## 9. Task decomposition (one spec → ordered plan)

1. **T1 — read primitives + `RangeRead` byte-len + M4 lens-core half.** `Tail`/`Backward`/
   `Span` on `SqliteTranscriptReader` (byte-budget iterate-and-break; `Span` inclusive
   replace-band); `RangeRead.rows` → `(i64, usize, Item)` + update T-2 call sites;
   `PersistError::is_busy` (via `is_transient`) + `synthetic_busy`. lens-core unit tests.
2. **T2 — replica resident-window model + BTreeMap migration + following seam.** BTreeMap
   substrate; the `resident_lo/hi` + `known_committed` + `last_rendered` + `item_bytes`
   cursors; ordinal-native migration of the §4.1 index consumers; baseline `Tail`; `Backward`
   prepend; scoped reconcile via `Span`→full-reproject (§4.2); the C4 `run_anchor` section
   identity (T-1 amendment: `SectionKey`/`RowId::Section` → `ItemId`, `group_work_section`
   stamps anchor); thin `replica.set_following`. Replica + reduce unit tests (span de-ghosts
   the fc_live@5 case; prepend; run_anchor stable under truncation; generation guard).
3. **T3 — LRU eviction + forward-delta gating + `Priority::Page` + M1 GC.** Contiguous-band
   eviction; §4.4 gating (uses the T2 `following` flag) + ordinal-based pill math;
   `Priority::Page` coalescing; entity GC sweep (§6). Sabotage-verified GC (children survive
   collapse→GC→expand) + eviction + gated-advance tests.
4. **T4 — view: sentinel + anchor + partial section + FollowMode wiring.** `RowKind::
   LoadOlder`, scroll-near-top trigger + single-in-flight guard, partial-section render,
   `jump_to_latest` tail-reload, FollowMode→`following` plumbing; **extend
   `focused_scroll_probe`** for prepend-anchor stability (real window). Extract the scroll
   decision for unit tests.
5. **T5 — M4 lens-ui dep-drop.** Wire `is_busy`, swap fakes to `synthetic_busy`, remove the
   `rusqlite` dep, verify `cargo tree`.
6. **T6 — seeded demo + sweep + D19 regression test (split).** Seed generator, demo wiring,
   out-of-gate latency/RAM sweep; regression asserts **(a)** actor catch-up never calls
   `TranscriptStore::reconcile(&[Item])` and **(b)** focused `new`/`on_reconcile_epoch_settled`
   never enqueue `ReadRange::All` outside `cfg(test)` (grok I9 — not a weak grep false-green).
- **End:** whole-branch cross-family review (grok-4.5 per-task during build; codex reserved
  for the end-of-workstream pass per the standing credit directive).

Order: T1 → T2 → T3 → T4; T5 after T1; T6 last. T3 depends on T2 (substrate + `following`
seam); T4 depends on T2/T3.

## 10. Risks & mitigations

- **Scroll anchor on prepend** (highest) — bottom-aligned `ListState` + splice-at-front. May
  need explicit scroll compensation for inserted height. Mitigation: real-window probe first
  (T4), extract the decision for unit tests, prove paint.
- **Following / eviction interaction** (§4.4) — the most stateful bit. Mitigation: explicit
  `following ⇔ resident_hi == known_committed` invariant + the three-cursor split; tests for
  advance-while-scrolled-up (pill counts, no load) and follow-to-bottom-reloads-tail.
- **Section-identity churn under truncation** (§5/C4) — mitigated by the `run_anchor` ItemId
  key + the truncated-interleaved-run non-churn test.
- **GC over-/under-eviction** — dropping a collapsed child, dormant chip/rail, or pending
  tail → blank/remount or orphan. Mitigation: precise retain-set (§6) + sabotage test +
  the post-`overlay_pending_finalize` ordering constraint.
- **Byte-budget const tuning** — defaults from a warm-cache spike; the T6 sweep validates
  cold + in-product and is the tuning gate.

## 11. Open questions

None blocking. Byte-budget constants are seeded from the spike and finalized by the T6
sweep. `ReadRange::All` is retained (harmless; tiny sessions/tests) rather than removed. The
C4 section-identity approach is **decided**: window-invariant `run_anchor: ItemId` key.
