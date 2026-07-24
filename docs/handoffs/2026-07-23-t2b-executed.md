# Transcript T-2b (disk-windowing) — executed

**Date:** 2026-07-23  **Branch:** `t2b-disk-windowing` @ `ac9f7da` — **UNMERGED** (user set the
boundary: "I want to see the demo before the merge, so stop before merge").

## What shipped

`FocusedTranscript` (`crates/lens-ui/src/focused/`) is now a **bounded, byte-budgeted,
LRU-evicted resident window** over the disk-backed transcript, replacing the O(whole-transcript)
in-memory replica. Cold focus and RAM are now O(window), not O(history).

- **T1** (`crates/lens-core/src/persist/`) byte-accurate read primitives:
  `ReadRange::{All, Tail{byte_budget}, Backward{before,byte_budget}, Span{from,through}, Delta, One}`;
  newest-first accumulate-and-break byte budgets; `RangeRead.rows: Vec<(ordinal, byte_len, Item)>`
  with `byte_len` via `length(CAST(payload AS BLOB))`.
- **T2** resident window: `BTreeMap<i64,Item>` keyed by ordinal + cursors
  `resident_lo/resident_hi/known_committed/last_rendered_ordinal/resident_bytes`; `Span` in-band
  reconcile (de-ghost); window-invariant `WorkSection.run_anchor: ItemId` + replica-side sticky
  `section_anchor` map.
- **T3** LRU eviction (`evict_if_over_cap`: pop_first when following, else pop_last), RowStore
  entity GC, forward-delta gating on `following`, `Priority::Page` coalescer slot.
- **T4** scroll-back UX: `RowKind::LoadOlder` sentinel (iff `resident_lo>0`), `should_page_older`
  + `page_in_flight` single-in-flight guard, `jump_to_latest` tail-reload, prepend anchor
  preservation via **identity re-pin** (capture the RowId at the scroll anchor before rebuild,
  `scroll_to` its new index after — supersedes an insert-count-arithmetic dead-end).
- **T5** dropped the `rusqlite` dep from lens-ui (`PersistError::is_busy()`/`synthetic_busy()`).
- **T6** D19 regression locks (`run_catchup` never full-reconciles; focused never enqueues
  `ReadRange::All`), `xtask focused-seed` generator, `xtask focused-sweep` out-of-gate latency/RAM
  harness, and the `LENS_DEMO_FOCUSED=1` demo.

## Verification

- **Full `xtask gate` GREEN** @ `ac9f7da`: clippy `-D warnings`, fmt, tests (lens-core 310 +
  lens-ui 197), benches compile, no API drift.
- **Real-window probe** (`focused_scroll_probe`, sandbox-disabled) exit 0 — initial-bottom,
  stick-to-bottom-while-following, finalize-anchor-stable, backward-prepend-anchor, C2
  prepend-with-eviction anchor.
- **Demo renders**: `docs/evidence/t2b-focused-demo.png` — the 2000-item windowed transcript in
  the focused `#chat-slot`. Run: `LENS_DEMO_FOCUSED=1 cargo run -p lens-app --features demo`.
- **Sweep** (`.superpowers/sdd/focused-sweep-results.md`): 1k/10k/50k → cold-focus
  12.8/27.8/33.1 ms; resident 0.71/7.09/12.58 MB — all under the 24 MB cap. **Byte-budget consts
  VALIDATED, no tuning** (TAIL=8MiB, PAGE=4MiB, RESIDENT_CAP=24MiB).

## Reviews

Each task got a grok-4.5 cross-family per-task review. The **end-of-workstream codex (gpt-5.6)
whole-branch review found a Critical every prior review + the probe + all tests missed**: the
incremental live-tail reproject silently drops a Delta/One committing into the SETTLED band when
`live_section_lo == None` (a committed item arriving while no response is active). Controller
verified all 9 findings and:

- **Fixed** F1 (Critical, settled-band drop → force full reproject via `touched_settled`),
  F2 (upsert displaced-id byte-accounting drift), F3 (empty All/Tail stale band), F4
  (`known_committed`/`last_rendered` regression → `.max()`), F7 (LoadOlder-anchor yank at ordinal 0
  → successor fallback) — each with a non-vacuous regression test (F1 empirically revert-verified).
- The F1 fix **regressed** (newly-materialized appends yanked a scrolled-up viewport) — fixed by
  **generalizing the identity re-pin to every reproject**; caught by re-running the real-window
  probe. The probe's "paused-not-yanked" contract was untestable (gpui walls off synthetic scroll)
  and had only passed vacuously via the F1 bug — removed, replaced by the headless
  `append_while_scrolled_up_preserves_anchor` test.
- **Rejected** F8 (unreachable: `focus_generation` immutable per replica).
- **Deferred** F5 (reconcile-Span capture race, mitigated by the reconcile-epoch overlap guard)
  and F6 (sticky-anchor erase = intended window-invariant behavior).
- Grok cross-family review of the fix wave: **SHIP**. One Minor perf note (N1: universal re-pin
  runs O(n) `position` per reproject — safe at bounded window sizes).

## Open (documented, non-blocking)

- **F5** reconcile-Span capture race — follow-up (epoch-guard mitigates today).
- **N1** re-pin O(n) — optimize only if profiling flags the hot path.
- **M2** thin orphan-ToolSpan test; byte-accurate-FFI not triggered.

## Next

**User reviews the demo, then merges `t2b-disk-windowing` → `main` + push** (solo integration
workflow: no PR; gate = tests pass + zero warnings/dead-code). Do NOT merge before that review.

Ledger: `.superpowers/sdd/progress.md`. Codex triage: `.superpowers/sdd/codex-review-triage.md`.
