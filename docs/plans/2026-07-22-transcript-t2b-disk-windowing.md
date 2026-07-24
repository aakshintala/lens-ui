# Transcript T-2b — Disk windowing, scroll-back paging & bounded-tail reconcile — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn `FocusedTranscript` from an O(whole-transcript) replica into a bounded, byte-budgeted, LRU-evicted resident window with disk scroll-back paging and a scoped (band-only) reconcile re-read, so multi-day sessions stay correct and smooth.

**Architecture:** Replica-side only. New `TranscriptReader::read_range` byte-budget variants (`Tail`/`Backward`/`Span`) load newest-first up to a byte budget; `FocusedTranscript.items` becomes an ordinal-keyed `BTreeMap` band anchored on the viewport with three explicit cursors (`resident_lo/hi`, `known_committed`). Reconcile re-reads only the resident band via `Span` (replace-in-range, so a folded/deleted provisional de-ghosts). Section identity moves from a window-relative `run_index` counter to a window-invariant `run_anchor: ItemId` so paging older history in never renumbers a visible section. LRU eviction + a RowStore entity GC bound RAM.

**Tech Stack:** Rust, gpui `list()`, rusqlite (lens-core only after M4), `BTreeMap`/`HashMap`, TDD with `#[gpui::test]` + a real-window `focused_scroll_probe`.

**Design source:** `docs/specs/2026-07-22-transcript-t2b-disk-windowing-design.md` (rev 2, grok-4.5-reviewed).

## Global Constraints

- **No actor / network changes.** The replica reads only from disk; `/items` pagination is the actor's concern and already ships. (spec §2 non-goals)
- **No content-rendering changes.** Row stubs stay; T-3/T-4 own content. Only section *identity* changes, never collapse/grouping semantics. (spec §2, §5)
- **`ReadRange::All` is retained** (harmless; tiny sessions + tests). Production baseline/reconcile stop using it. (spec §11)
- **Byte-budget consts are named + tunable**, seeded from the spike, finalized by the T6 sweep: `TAIL_BUDGET_BYTES` ≈ 8 MiB, `PAGE_BUDGET_BYTES` ≈ 4 MiB, `RESIDENT_CAP_BYTES` ≈ 24 MiB. (spec §4.1)
- **Every byte-budget read iterates newest-first and breaks on accumulated `length(payload)`** — never a SQL running-sum window (that scans the whole table). Always yield ≥1 row. (spec §3)
- **BTreeMap migration is exhaustive**: every `usize`-index consumer of `items` moves to ordinal-native in the same task; leaving one as a vec index is a bug (and fixes the latent `*ordinal as usize` index in `upsert_read_rows`). (spec §4.1)
- **Review diversity (MANDATORY, per CLAUDE.md):** each non-trivial task gets ≥1 cross-family review — grok-4.5 per-task via `cursor-delegate`; codex (`codex exec -s read-only`, gpt-5.6) reserved for the end-of-workstream whole-branch pass. `composer-2.5` authors, so it can't review.
- **Gate = tests pass + zero warnings/dead-code + `cargo fmt --check`.** New production crates/bins get added to the xtask gate's explicit `-p` lists; never pipe the gate through `tail`. (memory: xtask-gate-scope)
- **Ordinals are append-only/immutable** (omnigent items never rewrite in place; supersession/compaction/`/clear` append past the frontier), which is why `Span` replace-in-band and `Backward` prepend are sound. (spec §1)

---

## File structure

**lens-core (T1, T5-core, T6-regression):**
- `crates/lens-core/src/persist/mod.rs` — `ReadRange` variants, `RangeRead.rows` 3-tuple, `PersistError::is_busy` + `synthetic_busy`.
- `crates/lens-core/src/persist/transcript.rs` — `read_range` SQL for `Tail`/`Backward`/`Span`; byte-budget row loop.
- `crates/lens-core/src/persist/map.rs` — `row_to_ordinal_len_item` mapper.
- `crates/lens-core/src/reduce/view.rs` — `ViewBlock::WorkSection.run_anchor` (T-1 amendment).

**lens-ui replica (T2, T3, T4, T5):**
- `crates/lens-ui/src/focused/mod.rs` — `BTreeMap` substrate, cursors, `apply_read` match, load directions, LRU eviction, forward-delta gating, `set_following`, GC call sites.
- `crates/lens-ui/src/focused/rowsource.rs` — `RowId::Section`/`SectionRail` → `ItemId`, `SectionKey.run_anchor`, `RowKind::LoadOlder`, entity GC sweep.
- `crates/lens-ui/src/focused/reader.rs` — `Priority::Page`, coalescer slot, `is_busy` swap.
- `crates/lens-ui/src/focused/view.rs` — top sentinel, scroll-near-top trigger, partial section, `jump_to_latest` tail-reload, FollowMode→`following`.
- `crates/lens-ui/src/bin/focused_scroll_probe.rs` + `crates/lens-ui/tests/focused_scroll_realwindow.rs` — prepend-anchor stability.
- `crates/lens-ui/Cargo.toml` — drop `rusqlite` dep + dev-dep (T5).

**Demo / sweep (T6):**
- `crates/xtask/src/main.rs` — seed generator subcommand + out-of-gate latency/RAM sweep.
- lens-app demo wiring (`--features demo`, `#chat-slot`) — locate at task start.

---

## Task 1 — Read primitives + `RangeRead` byte-len + M4 lens-core half

**Files:**
- Modify: `crates/lens-core/src/persist/mod.rs:164-180` (`ReadRange`, `RangeRead`), `:42-52` (`PersistError`)
- Modify: `crates/lens-core/src/persist/transcript.rs:45-83` (`read_range`)
- Modify: `crates/lens-core/src/persist/map.rs:195-199` (add `row_to_ordinal_len_item`)
- Modify: `crates/lens-core/Cargo.toml` (add `test-util` feature)
- Test: inline `#[cfg(test)]` in `transcript.rs` and `mod.rs`

**Interfaces:**
- Produces (consumed by T2/T3/T4/T5):
  ```rust
  pub enum ReadRange {
      All,                                          // unchanged
      Delta { after: i64, through: i64 },           // unchanged (upsert)
      One   { ordinal: i64 },                       // unchanged
      Tail     { byte_budget: usize },              // NEW — newest rows up to budget, ascending
      Backward { before: i64, byte_budget: usize }, // NEW — rows `ordinal < before` up to budget, ascending
      Span     { from: i64, through: i64 },         // NEW — inclusive [from,through] ascending; REPLACE-in-range
  }
  pub struct RangeRead {
      pub rows: Vec<(i64, usize, Item)>,  // (ordinal, payload_len_bytes, item) — usize is NEW
      pub skipped: Vec<SkippedRow>,
      pub watermark: Option<i64>,         // newest NON-provisional ordinal (unchanged)
  }
  impl PersistError {
      pub fn is_busy(&self) -> bool;                        // == self.is_transient()
      #[cfg(any(test, feature = "test-util"))]
      pub fn synthetic_busy() -> Self;                      // rusqlite-free constructor for downstream fakes
  }
  ```

- [ ] **Step 1: Write failing test — `Tail` breaks on byte budget.** In `transcript.rs` tests, seed a store with several rows of known `length(payload)`, then:
  ```rust
  let read = reader.read_range(ReadRange::Tail { byte_budget: SMALL }).unwrap();
  // newest rows only, ascending by ordinal, at least 1, total payload_len ~≤ budget (last row may overshoot)
  assert!(read.rows.windows(2).all(|w| w[0].0 < w[1].0));  // ascending ordinals
  assert!(!read.rows.is_empty());
  assert_eq!(read.rows.last().unwrap().0, max_ordinal);    // includes the newest row
  assert_eq!(read.watermark, Some(max_nonprovisional_ordinal));
  ```
- [ ] **Step 2: Run — expect FAIL** (variant + 3-tuple don't exist). `cargo test -p lens-core --lib persist::transcript`
- [ ] **Step 3: Add the variants + 3-tuple.** In `mod.rs`: add `Tail`/`Backward`/`Span` to `ReadRange`; change `RangeRead.rows` to `Vec<(i64, usize, Item)>`.
- [ ] **Step 4: Add the byte-len mapper.** In `map.rs` add:
  ```rust
  /// Reader row: ordinal @0, length(payload) @1, then row_to_item columns @2..
  pub(crate) fn row_to_ordinal_len_item(r: &rusqlite::Row) -> rusqlite::Result<(i64, usize, Item)> {
      Ok((r.get(0)?, r.get::<_, i64>(1)? as usize, row_to_item_at_offset(r, 2)?))
  }
  ```
- [ ] **Step 5: Rewrite `read_range`.** Change `SELECT_PREFIX` to prepend the byte length so every arm yields the 3-tuple and `item_id` is now column 2:
  ```rust
  const SELECT_PREFIX: &str =
      "SELECT ordinal, length(payload), item_id, live_seq, kind, payload, agent, depth, created_at, response_id FROM items";
  ```
  - `All`/`Delta`/`One`: same SQL suffixes, but `collect_skipping(&mut rows, /*id_col*/ 2, row_to_ordinal_len_item)`.
  - `Tail`: `"{SELECT_PREFIX} ORDER BY ordinal DESC"`; hand-roll the row loop — accumulate `col1` payload len, push rows until the running sum **exceeds** `byte_budget` (always keep ≥1), then `rows.reverse()` to ascending. Skipped rows recorded as elsewhere.
  - `Backward`: same as `Tail` with `WHERE ordinal < ?1` bound to `before`.
  - `Span`: `"{SELECT_PREFIX} WHERE ordinal >= ?1 AND ordinal <= ?2 ORDER BY ordinal"`, `collect_skipping(.., 2, row_to_ordinal_len_item)` — no budget.
  - `watermark` sub-query unchanged.
- [ ] **Step 6: Run the `Tail` test — expect PASS.**
- [ ] **Step 7: Write + pass `Backward` and `Span` tests.** `Backward { before }` returns only `ordinal < before`, ascending, budget-bounded. `Span { from, through }` returns exactly the inclusive band ascending, no budget. Add: a `Backward` with `before` below all ordinals returns empty (0 rows is allowed for `Backward`, unlike `Tail`). Run: PASS.
- [ ] **Step 8: `PersistError::is_busy` + `synthetic_busy`.** Add to `impl PersistError` (`mod.rs`):
  ```rust
  pub fn is_busy(&self) -> bool { self.is_transient() }
  #[cfg(any(test, feature = "test-util"))]
  pub fn synthetic_busy() -> Self {
      PersistError::Sqlite(rusqlite::Error::SqliteFailure(
          rusqlite::ffi::Error::new(5 /* SQLITE_BUSY */), None))
  }
  ```
  Add `test-util = []` to `crates/lens-core/Cargo.toml` `[features]`. Test: `assert!(PersistError::synthetic_busy().is_busy()); assert!(!PersistError::ReadOnly.is_busy());`
- [ ] **Step 9: Update all `RangeRead` construction call sites to the 3-tuple.** In `transcript.rs` `read_range_delta_returns_ordinals_and_watermark_in_one_txn` and any `RangeRead { rows: vec![(ord, item)] }` → `vec![(ord, len, item)]`. (reader.rs / mod.rs test sites are updated in T2/T5, but lens-core must compile now — grep `RangeRead {` and `(i64, Item)` inside lens-core.) Run: `cargo test -p lens-core` green, zero warnings.
- [ ] **Step 10: Commit.**
  ```bash
  git add crates/lens-core
  git commit -m "feat(persist): Tail/Backward/Span read ranges + byte-len RangeRead + is_busy"
  ```
- [ ] **Step 11: Cross-family review** the T1 diff via `cursor-delegate` on grok-4.5 (byte-budget off-by-one, column-offset correctness, watermark unchanged). Address findings, re-run gate.

---

## Task 2 — Replica resident-window model + BTreeMap migration + `run_anchor` identity + following seam

This is the substrate everything downstream builds on. It compiles green and de-ghosts, but eviction/gating/sentinel are T3/T4.

**Files:**
- Modify: `crates/lens-core/src/reduce/view.rs:18-22, 155-200` (`ViewBlock::WorkSection.run_anchor`, `group_work_section`)
- Modify: `crates/lens-ui/src/focused/rowsource.rs:17-47, 593-615` (`RowId::Section`/`SectionRail` → `ItemId`, `SectionKey.run_anchor`, `materialize_work_section`)
- Modify: `crates/lens-ui/src/focused/mod.rs:39-152` (fields/constructors), `:302-344` (`apply_read`), `:414-424, 519-560, 610-646` (ordinal-native migration)
- Test: inline in all three modules

**Interfaces:**
- Consumes: T1 `ReadRange::{Tail,Backward,Span}`, `RangeRead.rows: Vec<(i64, usize, Item)>`.
- Produces (consumed by T3/T4):
  ```rust
  // reduce/view.rs
  ViewBlock::WorkSection { response_id: &'a ResponseId, run_anchor: &'a ItemId, blocks: Vec<ViewBlock<'a>> }
  // rowsource.rs
  pub enum RowId { Section(ResponseId, ItemId), SectionRail(ResponseId, ItemId), Work(ItemId), Sibling(ItemId), StreamTail(AccId), Marker(u64) }
  pub struct SectionKey { pub response_id: ResponseId, pub run_anchor: ItemId }
  // focused/mod.rs — FocusedTranscript
  fn set_following(&mut self, following: bool);   // thin seam; T4 drives it
  // cursors (private): resident_lo, resident_hi, known_committed, last_rendered_ordinal, resident_bytes, following, live_section_lo
  ```

### 2A — `run_anchor` section identity (T-1 amendment)

- [ ] **Step 1: Failing test — interleaved run does not renumber under truncation.** In `reduce/view.rs` tests, adapt `interleaved_message_keeps_two_runs_in_order_with_run_index` (line ~728): assert each `WorkSection.run_anchor` equals its first child block's `ItemId`, and that projecting only the *later* run in isolation yields the **same** `run_anchor` as when both runs are present (a counter would differ).
- [ ] **Step 2: Run — FAIL** (`run_anchor` field absent).
- [ ] **Step 3: Change `ViewBlock::WorkSection`** to `run_anchor: &'a ItemId`. Add a helper:
  ```rust
  fn anchor_of<'a>(vb: &ViewBlock<'a>) -> Option<&'a ItemId> {
      match vb { ViewBlock::Item(i) => Some(&i.id), ViewBlock::ToolSpan { call, .. } => Some(&call.id), _ => None }
  }
  ```
  In `group_work_section`, drop `run_counts`; in `flush`, stamp `run_anchor` = the first `run` block whose `anchor_of` is `Some` (a work run always starts with an `Item`/`ToolSpan`). Update the `panic!` matcher at ~489 and any `run_index` reads in tests.
- [ ] **Step 4: Run — PASS.** `cargo test -p lens-core --lib reduce::view`
- [ ] **Step 5: Propagate to rowsource.** `SectionKey { response_id, run_anchor: ItemId }`; `RowId::Section(ResponseId, ItemId)` + `SectionRail(ResponseId, ItemId)`; `chip_id`/`rail_id` clone `run_anchor`. `materialize_work_section` takes `run_anchor: &ItemId` and stamps the key. Update chip/rail stub text (`section {resp} @{anchor}`), the `RowId::Section(resp, run_index)` match at rowsource.rs:879, and `legacy_section_rail_marker_seq` test helper (now keyed by anchor — or delete if it no longer type-checks; it only guards the Marker/rail namespace which still holds).
- [ ] **Step 6: Run rowsource tests — PASS**, zero warnings.
- [ ] **Step 7: Non-churn test (the C4 guarantee).** New `#[gpui::test]` in `rowsource.rs`: materialize an interleaved two-run response truncated to the later run; then materialize again with the earlier run prepended; assert the upper section's `chip_id`/`rail_id` `EntityId`s are unchanged across the two materializes (naive counter must fail this). Run: PASS.
- [ ] **Step 8: Commit.** `git commit -am "feat(view): window-invariant run_anchor section identity (T-1 amendment)"`

### 2B — BTreeMap substrate + cursors

- [ ] **Step 9: Failing test — baseline `Tail` sets the cursors.** In `focused/mod.rs` tests, add `range_read`/`seed_resident_replica` overloads for the 3-tuple, then:
  ```rust
  r.apply_read(1, ReadRange::Tail { byte_budget: TAIL_BUDGET_BYTES }, tail_read(rows, watermark), cx);
  assert_eq!(r.resident_lo_for_test(), min_ordinal);
  assert_eq!(r.resident_hi_for_test(), max_ordinal);            // provisional included, NOT watermark
  assert_eq!(r.known_committed_for_test(), watermark);
  assert_eq!(r.items_len_for_test(), rows.len());
  ```
  Add `#[cfg(test)]` accessors `resident_lo_for_test`/`resident_hi_for_test`/`known_committed_for_test`.
- [ ] **Step 10: Run — FAIL.**
- [ ] **Step 11: Migrate the struct.** In `FocusedTranscript`:
  ```rust
  items: BTreeMap<i64, Item>,          // was Vec<Item>
  item_bytes: HashMap<ItemId, usize>,  // NEW — eviction accounting
  resident_lo: i64,                    // NEW
  resident_hi: i64,                    // NEW
  known_committed: i64,                // NEW
  resident_bytes: usize,               // NEW
  following: bool,                     // NEW (default true)
  live_section_lo: Option<i64>,        // was live_section_start: usize
  // item_ordinals, last_rendered_ordinal, everything else kept
  ```
  Add the three `const *_BYTES` (module top). Init in both constructors (`resident_lo/hi = -1`, `known_committed = -1`, `resident_bytes = 0`, `following = true`, `live_section_lo = None`, `items: BTreeMap::new()`).
- [ ] **Step 12: Change the baseline enqueue.** `new_with_reader` (mod.rs:112): `enqueue_read(ReadRange::Tail { byte_budget: TAIL_BUDGET_BYTES }, Priority::Baseline, focus_generation)` — was `ReadRange::All`.
- [ ] **Step 13: Ordinal-native migration (exhaustive — Global Constraint).** Convert every `items`-as-`Vec` consumer:
  - `recompute_live_section_start` → `recompute_live_section_lo`: find the min ordinal whose `ctx.response_id == active` via `items.iter()` (BTreeMap is ordered); store `Option<i64>` (None ⇒ no active/live band empty). Slices `items[..start]` / `items[start..]` become `items.range(..lo)` / `items.range(lo..)` (or full range when `None`).
  - `recompute_settled_prefix` (:551): slice via `self.items.range(..lo)` where `lo = live_section_lo.unwrap_or(i64::MAX)`; collect `&Item` refs in ordinal order.
  - `reproject` full/live-tail (:570-596): full path iterates `self.items.values()`; live-tail slice via `self.items.range(lo..)`.
  - `compute_expansion_flags` / `latest_settled_before_next_user` (:503-548): iterate `self.items` ordered; the `rposition`/index comparisons become **ordinal** comparisons (compare map keys, not vec indices).
  - `replace_read_rows` (:610): rebuild `items` as `BTreeMap` from `(ordinal, item)`, rebuild `item_ordinals` + `item_bytes` + `resident_bytes` from the 3-tuple lens.
  - `upsert_read_rows` (:620) → ordinal-keyed insert `items.insert(*ordinal, item.clone())` (this **fixes** the latent `*ordinal as usize` vec-index bug). Update `item_ordinals` + `item_bytes` + `resident_bytes` deltas.
  - `seed_item` test helper + `live_slice_len_for_test` → ordinal-keyed.
- [ ] **Step 14: `apply_read` exhaustive match + cursor updates (spec §4.2).** Replace `full_replace = matches!(range, All)` with `full_replace = matches!(range, All | Span { .. } | Tail { .. })` and extend both `match range` blocks to cover every variant:
  - `All` / `Tail`: `replace_read_rows`; `resident_lo = min read ordinal`, `resident_hi = max read ordinal` (provisional included), `known_committed = watermark`, `last_rendered_ordinal = watermark`.
  - `Span { from, through }`: **replace-in-band** — drop resident rows in `[from,through]` absent from the read (`items.retain`/`range` + re-insert), then insert read rows; `full_replace` path (full reproject + `settled_structure_len = 0`); advance cursors to cover the read.
  - `Delta { through, .. }`: `upsert_read_rows`; `resident_hi = max(resident_hi, through)`, `last_rendered_ordinal = max(.., through)`, `known_committed = max(.., watermark)`.
  - `Backward`: **prepend** — insert read rows, lower `resident_lo` to min read ordinal; `full_replace = false` (partial reproject is fine; identity is anchor-stable).
  - `One`: unchanged.
  The `Span`-as-`All` gate ensures RowStore structure fully invalidates (no ghost `Work` rows). (spec §4.2 grok I5)
- [ ] **Step 15: Run Step-9 test — PASS.** Then run the whole `focused` suite; fix ordinal-native fallout.
- [ ] **Step 16: `Span` de-ghost test (the fc_live@5 case, spec §3.1).** New `#[gpui::test]`: seed a windowed replica with a provisional `fc_live` at ordinal 5 (above watermark 0) and `msg_store` at 0 resident; apply `Span { from: resident_lo, through: resident_hi }` whose rows **omit** ordinal 5 (the fold deleted it). Assert `items` no longer contains `fc_live` and no ghost row survives in `rows.order()`. Confirms `Span` spans the resident band by ordinal (not watermark). Run: PASS.
- [ ] **Step 17: `Backward` prepend test.** Seed a tail band `[10,20]`; apply `Backward { before: 10 }` returning `[5,9]`; assert `resident_lo == 5`, `resident_hi == 20`, ordering preserved, section anchors unchanged. Run: PASS.
- [ ] **Step 18: Reconcile re-read uses `Span`, not `All`.** Change `on_reconcile_epoch_settled` (:295-300) to `enqueue_read(ReadRange::Span { from: self.resident_lo, through: self.resident_hi }, Priority::Reconcile, ..)`. Add the thin seam `pub fn set_following(&mut self, following: bool)` (sets the flag; T4 drives it). Update the generation-guard test to the new variant. Run: PASS, zero warnings.
- [ ] **Step 19: Commit.** `git commit -am "feat(focused): BTreeMap resident window, cursors, Span reconcile, following seam"`
- [ ] **Step 20: Cross-family review** T2 diff (grok-4.5 via cursor-delegate): focus on the `apply_read` cursor arithmetic (`resident_hi` vs `watermark`), `Span` replace-in-band drop correctness, and that no `items`-as-vec-index consumer was missed. Address, re-gate.

---

## Task 3 — LRU eviction + forward-delta gating + `Priority::Page` + M1 entity GC

Depends on T2 (substrate + `following`).

**Files:**
- Modify: `crates/lens-ui/src/focused/reader.rs:17-23, 141-204` (`Priority::Page`, coalescer slot)
- Modify: `crates/lens-ui/src/focused/mod.rs` (eviction after load; `TranscriptAdvanced` gating; GC call sites)
- Modify: `crates/lens-ui/src/focused/rowsource.rs` (add `gc_entities`)
- Modify: `crates/lens-ui/src/focused/view.rs:70-76` (pill math → ordinal-based)

**Interfaces:**
- Consumes: T2 cursors + `following`.
- Produces (consumed by T4):
  ```rust
  pub enum Priority { Baseline, Delta, Reconcile, Rewrite, Page }   // Page NEW
  // rowsource.rs
  impl RowStore { pub(crate) fn gc_entities(&mut self, live_stream_tails: &HashSet<AccId>); }
  ```

### 3A — `Priority::Page` coalescer slot

- [ ] **Step 1: Failing test — two `Backward` pages coalesce keeping lowest `before`.** In `reader.rs` tests, insert two `Backward` targets (`before: 20`, `before: 10`) at `Priority::Page`; `pop_highest` yields one with `before: 10`.
- [ ] **Step 2: Run — FAIL.**
- [ ] **Step 3: Add `Page` to `Priority`; add `page: Option<ReadTarget>` to `TargetCoalescer`.** `is_empty` includes it. `insert` `Page` arm keeps the lowest `Backward.before` (coalesce). `pop_highest` order: `rewrites → reconcile → baseline → page → delta`. (spec §5: map `Tail`→Baseline, `Span`→Reconcile, `Backward`→Page.)
- [ ] **Step 4: Run — PASS.** Add a test that a `Reconcile`-priority `Span` still jumps ahead of a pending `Delta` (existing `reconcile_jumps_ahead_of_pending_delta`, retarget to `Span`).
- [ ] **Step 5: Commit.** `git commit -am "feat(reader): Priority::Page coalescing slot for Backward pages"`

### 3B — LRU eviction

- [ ] **Step 6: Failing test — resident_bytes held under cap, band stays contiguous.** Seed rows summing `> RESIDENT_CAP_BYTES` via successive loads while `following = true`; assert `resident_bytes <= RESIDENT_CAP_BYTES` after the last load, `items` keys are contiguous (no holes), and the newest ordinal (tail) is still resident (top evicted).
- [ ] **Step 7: Run — FAIL.**
- [ ] **Step 8: Add `evict_if_over_cap`** called at the end of `apply_read` (after row application + `resident_bytes` update, before/with reproject). Rule (spec §4.3): if `resident_bytes > RESIDENT_CAP_BYTES`, trim the end **farther from the viewport** — `following ⇒` drop from `resident_lo` upward (advance `resident_lo`); else drop from `resident_hi` downward (lower `resident_hi`) — one row at a time until under cap, updating `items` + `item_ordinals` + `item_bytes` + `resident_bytes`. Only band ends are trimmed (stays contiguous). `StreamTail`/pending rows live in scratch, never in `items`, so they're never evicted. GC the dropped rows' RowStore entities (call `gc_entities` — 3C).
- [ ] **Step 9: Run — PASS.** Add a test: while `following = false` (scrolled up), eviction drops from the bottom (`resident_hi` lowers), top stays. Run: PASS.

### 3C — M1 entity GC

- [ ] **Step 10: Sabotage test — collapsed child survives GC→expand (spec §6, follows [[false-green-probe-drives-production-path]]).** Materialize a section, collapse it (children in `SectionNode.children` but not in `order`), run `gc_entities`, re-expand; assert each child's **`EntityId` is unchanged**. A naive "retain `order` only" must fail this test.
- [ ] **Step 11: Run — FAIL** (`gc_entities` absent).
- [ ] **Step 12: Implement `RowStore::gc_entities(&mut self, live_stream_tails: &HashSet<AccId>)`.** Build the retain set:
  - every `RowId` in `order`, ∪
  - for every `SectionKey` in `sections`: `chip_id`, `rail_id`, **and every child in `SectionNode.children`** (grok C2 — collapsed children aren't in `order`), ∪
  - `chip_id`/`rail_id` for every `SectionKey` in `pending_tail_section.values()`, ∪
  - `RowId::StreamTail(acc)` for every `acc` in `live_stream_tails`, ∪
  - every `Marker` id present in `structure`.
  `self.entities.retain(|id, _| retain.contains(id))`.
- [ ] **Step 13: Run — PASS.**
- [ ] **Step 14: Wire GC into the replica with the ordering constraint.** In `reproject` (mod.rs), call `self.rows.gc_entities(&pending_accs)` **only after `overlay_pending_finalize`** (else a mid-finalize tail is transiently unreferenced), where `pending_accs: HashSet<AccId> = self.pending_finalize.keys().cloned().collect()`. Also call after eviction. Add a test: a mid-finalize staged tail survives a reproject-with-GC (its `StreamTail` entity retained via `pending_finalize`). Run: PASS.

### 3D — Forward-delta gating + ordinal pill math

- [ ] **Step 15: Failing test — advance-while-scrolled-up does not load, pill counts committed-but-unresident.** With `following = false` and `resident_hi < known_committed`, fold `TranscriptAdvanced { committed_ordinal: hi + 10 }`; assert **no** forward `Delta` was enqueued (inspect the test reader's read log / no cursor jump) and `known_committed` advanced to `hi + 10` while `resident_hi` is unchanged.
- [ ] **Step 16: Run — FAIL.**
- [ ] **Step 17: Gate `TranscriptAdvanced` (mod.rs:213-226).** Enqueue the forward `Delta { after: resident_hi, through: min(resident_hi + page, ord) }` **only when `self.following`** (⇔ `resident_hi == known_committed` at the tail); otherwise just `self.known_committed = max(self.known_committed, ord)` and `cx.notify()`. (spec §4.4 grok C3)
- [ ] **Step 18: Ordinal pill math (view.rs:70-76).** `new_since_pause` while paused returns `known_committed − resident_hi` (committed-but-unresident), read from the replica, instead of `row_count − rows_at_pause` (which stays 0 when rows aren't loaded). Keep the old row-delta path for the loaded case only if both are needed; prefer the ordinal count as the single source. Update `follow_mode_pill_n_counts_rows_since_pause` accordingly.
- [ ] **Step 19: Run — PASS**, whole `focused` suite green, zero warnings.
- [ ] **Step 20: Commit.** `git commit -am "feat(focused): LRU eviction, entity GC, forward-delta gating + ordinal pill"`
- [ ] **Step 21: Cross-family review** T3 diff (grok-4.5): eviction direction vs `following`, GC retain-set completeness (collapsed children, pending tails), gating invariant `following ⇔ resident_hi == known_committed`. Address, re-gate.

---

## Task 4 — View: top sentinel + scroll anchor + partial section + FollowMode wiring

Depends on T2/T3. The prepend-anchor paint is proven in the real-window probe.

**Files:**
- Modify: `crates/lens-ui/src/focused/rowsource.rs:60-72` (`RowKind::LoadOlder`)
- Modify: `crates/lens-ui/src/focused/mod.rs` (top-sentinel structure entry, `page_in_flight` guard, `set_following` wiring)
- Modify: `crates/lens-ui/src/focused/view.rs:93-165` (scroll-near-top trigger, `jump_to_latest` tail-reload, FollowMode→`following`, `kind_tag`, stub renderer)
- Modify: `crates/lens-ui/src/bin/focused_scroll_probe.rs` + `crates/lens-ui/tests/focused_scroll_realwindow.rs`

**Interfaces:**
- Consumes: T2 `set_following`, cursors; T3 `Priority::Page`.
- Produces: `RowKind::LoadOlder`; the extracted scroll-near-top decision fn (unit-testable).

- [ ] **Step 1: Failing test — top sentinel present iff `resident_lo > 0`.** New `#[gpui::test]`: with `resident_lo > 0`, `rows.order()` first entry is a `RowKind::LoadOlder`; with `resident_lo == 0` (or absent when `-1`), no sentinel. (Add a `#[cfg(test)]` accessor if needed.)
- [ ] **Step 2: Run — FAIL.**
- [ ] **Step 3: Add `RowKind::LoadOlder`** to the enum; extend `kind_tag` (view.rs:153) and `render_stub_row`/`stub_renderer_covers_every_row_kind`. In the replica's reproject, push a `StructureEntry::Marker`-like top sentinel (or a dedicated `StructureEntry::LoadOlder`) at index 0 whenever `resident_lo > 0`; ensure `rebuild_flat_order` emits it first.
- [ ] **Step 4: Run — PASS.**
- [ ] **Step 5: Failing test — scroll-near-top enqueues one `Backward` page, guarded single-in-flight.** Extract the decision into a pure fn:
  ```rust
  fn should_page_older(visible_start: usize, resident_lo: i64, page_in_flight: bool) -> bool
  // true iff resident_lo > 0 && !page_in_flight && visible_start <= NEAR_TOP_ROWS
  ```
  Test both branches; and that a second trigger while `page_in_flight` returns false (no duplicate read).
- [ ] **Step 6: Run — FAIL → implement `should_page_older` + `page_in_flight: bool` on the replica** (set true on enqueue of the `Backward` page, cleared in `apply_read` for `Backward`). Wire the scroll handler (view.rs:176) to call it and enqueue `Backward { before: resident_lo, byte_budget: PAGE_BUDGET_BYTES }` at `Priority::Page`. Run: PASS.
- [ ] **Step 7: `jump_to_latest` reloads the tail (spec §4.4).** Change `jump_to_latest` (view.rs:102-111) to first `replica.set_following(true)` **and** enqueue a tail reload (`Tail { TAIL_BUDGET_BYTES }` or `Delta { after: resident_hi }`) so eviction-dropped tail rows come back, then `scroll_to` the true latest. Test: under an evicted tail (`resident_hi < known_committed`), `jump_to_latest` enqueues a tail read and sets `following = true`. Run: PASS.
- [ ] **Step 8: FollowMode → `following` plumbing.** In `on_scroll_event` / `set_follow_mode` (view.rs:56-100), drive `replica.set_following(mode == Following)` so the T3 gating uses the real signal. Test: scrolling up sets `following = false` on the replica; returning to bottom sets it true. Run: PASS.
- [ ] **Step 9: Partial-section boundary note + orphan test.** With a run's earlier blocks above `resident_lo`, it renders as a partial rail with only resident children (anchor stable → no churn). Add a test that a `ToolSpan` whose `output` is resident but `call` is above `resident_lo` renders as a transient orphan (documented boundary UX, covered by the sentinel) without panicking `pair_tool_spans` (view.rs:35-82). Run: PASS.
- [ ] **Step 10: Real-window prepend-anchor probe (the highest risk, spec §5/§10).** Extend `focused_scroll_probe.rs`: scroll up to trigger a `Backward` prepend, then assert visible content is **stationary** (paint-level) after the splice-at-front on the `ListAlignment::Bottom` list. Follow [[t2-real-window-probe-sandbox]] (sandbox disabled) + [[gpui-list-scroll-and-realwindow-probe-gotchas]] (`scroll_by/scroll_to` don't fire the handler; `visible_range` pre- vs `is_scrolled` post-scroll; honest `process::exit`). Extract the scroll-compensation decision for a unit test; prove the paint in the probe (not `#[gpui::test]` — NoopTextSystem false-greens paint). Run via `xtask` real-window gate (`!`-prefixed if needed). If prepend shifts content, add explicit scroll compensation for inserted height.
- [ ] **Step 11: Run — probe PASS** (honest exit code), whole `focused` suite green, zero warnings.
- [ ] **Step 12: Commit.** `git commit -am "feat(focused/view): LoadOlder sentinel, backward paging, prepend anchor, follow wiring"`
- [ ] **Step 13: Cross-family review** T4 diff (grok-4.5): sentinel placement in `rebuild_flat_order`, single-in-flight guard correctness, `jump_to_latest` reload ordering, and the probe's paint assertion (does it own production state? — must emit the reducer's canonical signals, not hand-author them, per [[false-green-probe-drives-production-path]]). Address, re-gate.

---

## Task 5 — M4: drop `rusqlite` from lens-ui

Can run any time after T1.

**Files:**
- Modify: `crates/lens-ui/src/focused/reader.rs:225-241` (swap `is_sqlite_busy` → `err.is_busy()`), test fakes (:375-382, :716-735)
- Modify: `crates/lens-ui/Cargo.toml:22, 46-47` (remove `rusqlite` dep + dev-dep; add `lens-core` `test-util` dev-feature)

- [ ] **Step 1: Swap the busy check.** In `reader.rs`, delete `is_sqlite_busy` and use `err.is_busy()` in `classify_persist_error`. Update the two `is_sqlite_busy(...)` test assertions (:723,:728) to `err.is_busy()`.
- [ ] **Step 2: Swap test fakes to `synthetic_busy`.** `FakeReader`'s `ScriptedOutcome::Busy` arm (:375-382) returns `PersistError::synthetic_busy()` instead of hand-building `rusqlite::Error::SqliteFailure`.
- [ ] **Step 3: Remove the dep.** Delete `rusqlite = { version = "0.32" }` (line 22) and the `[dev-dependencies]` `rusqlite` (line 47) from `crates/lens-ui/Cargo.toml`; add `test-util` to the `lens-core` dev-dependency features so `synthetic_busy` is reachable.
- [ ] **Step 4: Verify no transitive dep.** Run `cargo tree -p lens-ui | grep rusqlite` — expect **empty** (only lens-core pulls rusqlite). Run `cargo test -p lens-ui` green, zero warnings.
- [ ] **Step 5: Commit.** `git commit -am "refactor(lens-ui): drop rusqlite dep via PersistError::is_busy/synthetic_busy"`

---

## Task 6 — Seeded demo + latency/RAM sweep + D19 regression test

Last task. Demo mount = `crates/lens-app/src/main.rs` (`#[cfg(feature = "demo")]`); the `#chat-slot` lives in `crates/lens-ui/src/board/mod.rs:984-999` fed by `chat_tab`, and `crates/lens-ui/src/slot/mod.rs` already mounts a `FocusedTranscript` via `mount_focused_transcript_view`. Mirror the `lens-terminal-demo` pattern.

**Files:**
- Create/Modify: `crates/xtask/src/main.rs` (seed generator subcommand + `focused-sweep`)
- Modify: `crates/lens-app/src/main.rs` (`#[cfg(feature = "demo")]` entry — build a seeded `ReaderFactory`)
- Modify: `crates/lens-ui/src/board/mod.rs:984-999` (`#chat-slot` → `chat_tab`) + `crates/lens-ui/src/slot/mod.rs:44` (already mounts via `mount_focused_transcript_view`) — install the seeded focused replica as `chat_tab`
- Test: inline regression test (actor + replica invariants)

- [ ] **Step 1: D19 regression test (split, spec §9 T6 / grok I9 — not a weak grep).**
  - **(a)** In lens-core actor tests: assert `run_catchup` (runloop.rs:319) never calls `TranscriptStore::reconcile(&[Item])` — a compile-time/structural guard (e.g. a test double whose `reconcile` panics, driven through a catch-up cycle), not a source grep.
  - **(b)** In lens-ui: assert focused `new`/`on_reconcile_epoch_settled` never enqueue `ReadRange::All` outside `cfg(test)` — via the test reader's read log after a baseline + a reconcile-settle, asserting the ranges are `Tail`/`Span`, never `All`.
- [ ] **Step 2: Run — expect PASS** (T2 already replaced `All` with `Tail`/`Span`; this locks it). If it fails, a production `All` leaked — fix.
- [ ] **Step 3: Seed generator.** Add an `xtask` subcommand that writes a seeded `<session>.db` (via `SqliteTranscriptStore::upsert_item`) with thousands of items, **bimodal** payload sizes (small markers + large dumps) like the spike, into the demo data dir. Parameterize item count (1k/10k/50k).
- [ ] **Step 4: Demo wiring.** Under `--features demo`, build a `ReaderFactory` pointing at the seeded db and mount a `FocusedTranscript` view in `#chat-slot` (today empty — no reader factory seeded). Permanent visual-acceptance rig, like the terminal demo. Manually run it once (`/run` or the demo bin) to confirm the transcript renders + scrolls.
- [ ] **Step 5: Out-of-gate latency/RAM sweep.** Add an `xtask focused-sweep` (like terminal `rss-sweep`, **not** in the CI gate): seed → measure **cold-first-focus** latency (explicitly, per the spike's unmeasured warm-cache gap), backward-page latency, and assert `resident_bytes ≤ RESIDENT_CAP_BYTES` at 1k/10k/50k items. Emit a report. This is the tuning gate for the byte-budget consts — adjust `TAIL/PAGE/RESIDENT_*_BYTES` if the numbers demand it, and record the final values.
- [ ] **Step 6: Add new bins/crates to the xtask gate `-p` lists** (memory: xtask-gate-scope) if the seed/sweep introduce a production target. Run the full gate: green, zero warnings, `fmt --check`.
- [ ] **Step 7: Commit.** `git commit -am "feat(demo): seeded focused transcript demo + latency/RAM sweep + D19 regression"`

---

## End of workstream

- [ ] **Whole-branch cross-family review** — codex (`codex exec -s read-only`, gpt-5.6) reserved for this end-of-workstream pass per the standing credit directive (memory: review-spend-policy, codex-as-reviewer). Include a **gate-running** reviewer (memory: whole-branch-review-needs-a-builder) — the resident-window substrate is a new seam sharing `apply_read`/coalescer with existing paths. Capture stdout (codex isn't truncated the way cursor-delegate is).
- [ ] **Converge on fixes** (memory: self-grill-needs-diff-review — cross-family review the fix diff), re-run the gate + the real-window probe + the out-of-gate sweep.
- [ ] **Update `docs/STATUS.md`** (+ handoff if multi-session) per memory: end-of-session-status-update. Persist durable learnings (byte-budget final consts, prepend-anchor gotchas, GC retain-set) to memory.
- [ ] **Merge to `main` + push** (solo integration workflow: no PR; gate = tests pass + zero warnings/dead-code).

---

## Self-review notes (spec coverage)

- §3 read primitives + `RangeRead` byte-len → T1. §3.1 `Span` vs `Delta` de-ghost → T2 Step 16. §3.2 M4 lens-core half → T1 Step 8.
- §4 resident window + cursors + BTreeMap migration → T2. §4.2 `apply_read` `Span`-as-`All` → T2 Step 14. §4.3 LRU → T3 3B. §4.4 following/pill/jump gating → T3 3D + T4 Step 7. §4.5 following seam → T2 Step 18 (declared), T4 Step 8 (driven).
- §5 scroll-back UX: sentinel/partial → T4; `run_anchor` (C4) → T2 2A; prepend anchor → T4 Step 10.
- §6 M1 GC → T3 3C. §7 M4 dep-drop → T5. §8 demo + sweep → T6 Steps 3-5. §9 D19 regression → T6 Steps 1-2.
- §10 risks map to: T4 Step 10 (anchor), T3 3D (following/eviction), T2 2A + Step 7 (identity churn), T3 3C (GC), T6 Step 5 (const tuning).
