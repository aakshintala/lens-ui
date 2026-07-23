# T-2b — Disk windowing, scroll-back paging & bounded-tail reconcile — design

**Date:** 2026-07-22
**Workstream:** `lens-ui` transcript fan-out → T-2b (disk-scale)
**Depends on:** T-2 (focused replica, landed on `main` `60425d2`)
**Status:** design — awaiting user review before planning

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
caller** (only the Board replica + tests). This is sound because omnigent items are
append-only/immutable ([[state-model-d23-disk-render]]): supersession, compaction, and
`/clear` all *append* past the frontier. So the spike's "reconcile bounded-tail, NEVER
full history" contract was already implemented by P3-3's D19 design. T-2b adds one
optional regression test to lock that invariant.

## 2. Goals / non-goals

**Goals**
- `FocusedTranscript` holds a **bounded, byte-budgeted, LRU-evicted resident window**, not
  the whole transcript.
- **Scroll-back paging**: scrolling up loads older pages from disk; a top sentinel marks
  more history above; a turn straddling the window top renders as a partial section.
- **Scoped reconcile re-read**: reconcile-epoch-settle re-reads only the resident band,
  not `ReadRange::All`.
- Carried Minors: **M1** (RowStore entity GC) and **M4** (drop `rusqlite` from lens-ui).
- **Seeded demo** as the permanent visual + latency/RAM acceptance rig.
- Multi-day sessions are correct and smooth; the spike's latency/RAM numbers hold in-product.

**Non-goals**
- No content rendering changes (stubs stay; T-3/T-4 own content).
- No actor / network changes. `/items` pagination already ships in lens-client and is the
  actor's concern; the replica reads only from disk.
- No new collapse/section semantics — grouping is unchanged; it simply operates over the
  resident window.

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

SQL shapes (all wrapped in the existing single-txn `read_range`, returning
`RangeRead { rows, skipped, watermark }`):

- **`Tail`**: `SELECT <cols>, length(payload) FROM items ORDER BY ordinal DESC` → accumulate
  `length(payload)`, break once the budget is exceeded (always yield ≥1 row). Rows returned
  ascending.
- **`Backward`**: same, with `WHERE ordinal < ?before`.
- **`Span`**: `SELECT <cols> FROM items WHERE ordinal >= ?from AND ordinal <= ?through ORDER
  BY ordinal`.

`watermark` (newest non-provisional ordinal) is returned by every variant, as today.

### 3.1 Delta (upsert) vs Span (replace) — why both

`Delta` upserts (insert-or-update by id; **never removes**) — correct for forward growth.
`Span` **replaces in range** (its rows are the *complete* truth for `[from,through]`; any
resident row in that ordinal band not present is dropped). Replace is required because a
reconcile can **delete** a row, and upsert cannot subtract:

> Live appends provisional `fc_live` at ordinal 5. Actor catch-up folds the tool call into
> its durable store row `msg_store` (already at ordinal 0, a **different id** — the
> two-id-space hazard, [[omnigent-two-id-space-reconciliation]]); the fold **deletes**
> ordinal 5. A tail re-read now simply *omits* ordinal 5. A `Delta` upsert adds `msg_store`
> but leaves `fc_live` resident → **ghost**. `Span` drops resident ordinals in-band absent
> from the read → no ghost.

This is exactly why today's reconcile uses `ReadRange::All` + full `replace_read_rows`
(de-ghosts by construction). `Span` is that same replace, **scoped to the resident band**.
The distinction is additive-vs-subtractive and is **orthogonal to the substrate key** — it
holds whether `items` is a `Vec` deduped by id or an ordinal-keyed map.

### 3.2 M4 (lens-core half)

- `PersistError::is_busy(&self) -> bool` — encapsulates the
  `Sqlite(DatabaseBusy|DatabaseLocked)` check that lens-ui currently open-codes.
- A rusqlite-free `PersistError::synthetic_busy()` constructor gated on `test-util` (or
  `cfg(test)` re-exported), so lens-ui test fakes inject busy without depending on
  `rusqlite`.

## 4. Resident window (`FocusedTranscript`)

`items` stops being the whole transcript and becomes a **contiguous, byte-budgeted `[lo,
hi]` span in ordinal space, anchored on the viewport.**

### 4.1 Substrate

Replace `items: Vec<Item>` + parallel `item_ordinals: HashMap<ItemId,i64>` with an
**ordinal-keyed `BTreeMap<i64, Item>`** (identity for render stays `ItemId`; keep
`item_ordinals: HashMap<ItemId,i64>` as the reverse index the marker/GC code needs). The
BTreeMap gives range-delete (`Span` replace + eviction) and range-scan (`Backward` prepend)
directly. Projection consumes `items.values()` (already ascending).

New state:
- `resident_lo: i64`, `resident_hi: i64` — the resident ordinal band.
- `resident_bytes: usize` + per-row `length(payload)` (carried on the read; store a
  `HashMap<ItemId,usize>` or fold into the item).
- consts: `TAIL_BUDGET_BYTES` (~8 MiB), `PAGE_BUDGET_BYTES` (~2–4 MiB),
  `RESIDENT_CAP_BYTES` (~24 MiB). Named, tunable; seeded from the spike.

### 4.2 Load directions

- **Baseline**: `Tail{ TAIL_BUDGET_BYTES }` → `items` = tail rows; `resident_lo` = min
  ordinal read (or `watermark+1` if empty); `resident_hi` = watermark.
- **Scroll up**: `Backward{ before: resident_lo, PAGE_BUDGET_BYTES }` → **prepend**; lower
  `resident_lo`.
- **Scroll down / follow**: re-extend `resident_hi` toward `watermark` via the existing
  forward `Delta{ after: resident_hi, through: min(resident_hi+page, watermark) }`.
- **Reconcile-epoch-settle**: `Span{ from: resident_lo, through: watermark }` +
  replace-in-band (replaces `ReadRange::All`).

### 4.3 LRU eviction

After any load, if `resident_bytes > RESIDENT_CAP_BYTES`, **evict the end farther from the
viewport** (drop rows from `items` + `item_ordinals` + `resident_bytes`, GC their RowStore
entities — see §6). Only ends are trimmed, so the band stays **contiguous**. Move
`resident_lo`/`resident_hi` accordingly.

Never evict rows carrying live/pending state (a `pending_finalize` tail is scratch/AccId-
keyed, not a disk item, so it is never in `items`; a just-committed disk row near the tail
is protected because following keeps the tail resident). The evicted band is cheaply
re-loadable (`Backward` for the top, `Delta`/`Tail` for the bottom).

### 4.4 Forward-delta gating when the tail is evicted (subtle case)

If the user scrolled up and LRU evicted the live tail (`resident_hi < watermark`), an
incoming `TranscriptAdvanced{ord}` must **not** pull those rows into the window — they are
off-screen below. It advances the known `watermark` and bumps the "↓ N new" count only.
Follow-to-bottom re-loads the tail (`Tail` or `Delta{ after: resident_hi }`), symmetric to
backward paging. So `TranscriptAdvanced`'s delta-read is gated on **"is `resident_hi` at
the tail / are we following."** This is inherent to LRU + live tail.

### 4.5 What is unchanged

`live_section_start`, `settled_structure_len`, `compute_expansion_flags`,
`latest_settled_before_next_user`, `reproject`, staged finalize, collapse timing — all
already operate over `self.items` and become O(window) for free. The live RAM tail
(scratch, AccId-keyed) is unchanged and remains split from the disk window at the watermark.

## 5. Scroll-back UX & anchor (`focused/view.rs`)

- **Top sentinel** row (`RowKind::LoadOlder`) rendered at the top of the list whenever
  `resident_lo > 0` (more history above). Scroll-near-top triggers a `Backward` page; a
  **single-in-flight guard** prevents duplicate requests.
- **Partial section at the boundary**: a turn whose `response_id` starts above `resident_lo`
  renders as a partial rail with only its resident children; more attach as pages load.
  `group_work_section` needs no change — it groups whatever is in `items`; the sentinel sits
  above it.
- **Scroll anchor (the real-window risk)**: prepending to the bottom-aligned `ListState`
  must keep visible content stationary. Proven by **extending `focused_scroll_probe`** (real
  GPUI window, sandbox-disabled, honest `process::exit` codes — [[t2-real-window-probe-sandbox]],
  [[gpui-list-scroll-and-realwindow-probe-gotchas]]): `scroll_by/scroll_to` don't fire the
  scroll handler, and `visible_range` is pre-scroll while `is_scrolled` is post-scroll, so
  the decision must be extracted + unit-tested and the *paint* proven in the probe. Not
  `#[gpui::test]` (NoopTextSystem false-greens paint).

## 6. M1 — RowStore entity GC

`entities: HashMap<RowId, Entity<RowState>>` never removes entries when rows leave
`order`/`structure`; with paging + eviction this grows unbounded and churns. Add a sweep
after reproject/eviction that **retains exactly**:

- everything in `order`, ∪
- `{ chip_id, rail_id }` for every live `SectionKey` in `sections` (collapsed keeps a
  dormant rail; expanded keeps a dormant chip — both must survive), ∪
- `StreamTail(acc)` for every `acc` in `pending_finalize`, ∪
- markers present in `structure`.

Everything else is dropped. Correctness-sensitive → **sabotage-verified** test (a naive
"retain `order` only" must visibly fail: collapse a section, GC, re-expand — the rail must
not remount). Follows [[false-green-probe-drives-production-path]] discipline.

## 7. M4 — drop `rusqlite` from lens-ui

- `focused/reader.rs::is_sqlite_busy` → `err.is_busy()` (§3.2).
- Test fakes inject busy via `PersistError::synthetic_busy()`.
- Remove `rusqlite` (and `bundled`) from `crates/lens-ui/Cargo.toml` (dev-dep + dep).
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
  1k/10k/50k items. Confirms the spike numbers in-product; flags if a budget const needs
  tuning. Cold-first-focus latency is explicitly measured (the spike flagged it as
  warm-cache-only unmeasured).

## 9. Task decomposition (one spec → ordered plan)

1. **T1 — read primitives + M4 lens-core half.** `Tail`/`Backward`/`Span` on
   `SqliteTranscriptReader` (byte-budget iterate-and-break; `Span` inclusive-range);
   `PersistError::is_busy` + `synthetic_busy`. lens-core unit tests (byte-budget break,
   span replace-set, empty file, provisional excluded from watermark).
2. **T2 — replica resident-window model.** `BTreeMap` substrate + `resident_lo/hi/bytes`;
   baseline `Tail`; `Backward` prepend; scoped reconcile via `Span` + replace-in-band;
   forward `Delta` re-extend. Replica unit tests (prepend, span-replace de-ghosts, watermark
   tracking, generation guard preserved).
3. **T3 — LRU eviction + forward-delta gating + M1 GC.** Contiguous-band eviction; the
   §4.4 tail-evicted gate; entity GC sweep (§6). Sabotage-verified GC + eviction tests.
4. **T4 — view: sentinel + anchor + partial section.** `RowKind::LoadOlder`, scroll-near-top
   trigger + single-in-flight guard, partial-section render; **extend `focused_scroll_probe`**
   for prepend-anchor stability (real window). Extract the scroll decision for unit tests.
5. **T5 — M4 lens-ui dep-drop.** Wire `is_busy`, swap fakes to `synthetic_busy`, remove the
   `rusqlite` dep, verify `cargo tree`.
6. **T6 — seeded demo + sweep + D19 regression test.** Seed generator, demo wiring,
   out-of-gate latency/RAM sweep; one actor test asserting no full-history transcript
   reconcile path exists.
- **End:** whole-branch cross-family review (grok-4.5 per-task during build; codex reserved
  for the end-of-workstream pass per the standing credit directive).

Order: T1 → T2 → T3 → T4; T5 anywhere after T1; T6 last. T3 depends on T2; T4 depends on
T2/T3.

## 10. Risks & mitigations

- **Scroll anchor on prepend** (highest) — bottom-aligned `ListState` + splice-at-front. May
  need explicit scroll compensation for inserted height. Mitigation: real-window probe first
  (T4), extract the decision for unit tests, prove paint. Prior gpui scroll gotchas apply.
- **Forward-delta / follow interaction with eviction** (§4.4) — the most stateful bit.
  Mitigation: explicit "following ⇔ `resident_hi == watermark`" invariant + tests for
  advance-while-scrolled-up and follow-to-bottom-reloads-tail.
- **GC over-eviction** (dropping a dormant chip/rail or a pending tail → remount/flash or
  orphan). Mitigation: precise retain-set (§6) + sabotage test.
- **Byte-budget const tuning** — defaults from a warm-cache spike; the T6 sweep validates
  cold + in-product and is the tuning gate.

## 11. Open questions

None blocking. Byte-budget constants are seeded from the spike and finalized by the T6
sweep. `ReadRange::All` is retained (harmless; tiny sessions/tests) rather than removed.
