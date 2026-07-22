# Handoff — Transcript T-2 plan-complete, ready to EXECUTE (subagent-driven)

**Written:** 2026-07-22 · **Branch:** `lens-transript` (UNMERGED, not pushed this session)
**HEAD:** `b3ba61f` · **Plan:** `docs/plans/2026-07-22-transcript-t2-focused-view-scaffold.md` (15 tasks)
**Spec:** `docs/specs/2026-07-21-transcript-t2-focused-view-scaffold-design.md` (rev 4 + **D-3 refinement** §13)
**Next action:** start a **fresh session**, invoke **`superpowers:subagent-driven-development`**, execute the plan **task-by-task starting at Task 1**.
**Memory:** [[transcript-workstream-decomposition]] · [[t1-viewblock-projection-executed]] · [[t2-per-run-sections-refinement]] · [[state-model-d23-disk-render]] · [[transcript-virtualization-spike-2026-07]] · [[omnigent-two-id-space-reconciliation]] · [[gpui-test-noop-text-system]] · [[terminal-realwindow-harness-pitfalls]] · [[board-b4a-design]]

## TL;DR

T-2 (focused view scaffold + live disk-sourced surface — first real consumer of T-1's
`Vec<ViewBlock>`) is **design-locked and now fully planned**. The 15-task plan was authored
this session against the rev-4 spec, with all lens-core sites read verbatim and the four
spec-deferred mechanism items resolved. One "locked" decision was **refined with the user
during planning** (D-3 → per-run sections; see below). Execution mode chosen: **subagent-driven**
(fresh subagent per task, composer builds + codex/Opus cross-family review between tasks).
**Do not re-plan; execute Task 1 first.**

## The four spec-deferred mechanism items — RESOLVED in the plan

1. **Section identity → per contiguous run, keyed `(response_id, run_index)`** (NOT one-per-response
   merge). Confirmed by real data (`docs/spikes/captures/2026-06-26-live-recapture/claude-native-todos.sse`
   has assistant messages interleaved between tool-call runs of one response). See D-3 refinement below.
2. **`Retired { acc_id, disposition }`** — `Finalizing { item_id }` on `Completed`; `Discarded` on
   `Failed`/`Incomplete`/`Cancelled` (`folds.rs:221` — currently does NOT retire scratch) **and** reconnect
   gap≠Some(0) (`snapshot.rs:98`). Each accumulator gets a stable `acc_id` minted at open (Task 3).
3. **Live re-projection index = `live_section_start: usize`** — the live turn's items are contiguous at
   the `items` tail; recompute on `ActiveResponseChanged`; re-project `&items[live_section_start..]` + scratch.
4. **Silent re-fire → precise `TranscriptRewritten { ordinal }` signal** (user-steered, NOT a reconnect
   proxy). The actor→replica contract now announces **every** below-watermark write it performs — a
   **three-signal model**: `TranscriptAdvanced` (append) / `TranscriptRewritten` (in-place re-fire, detected
   in `commit_terminal_prefix` as `stored_ord != requested`) / reconcile-epoch coarse re-read. No known gap
   left open.

## D-3 refinement (user-approved this session) — per-run sections, NOT merge-and-hoist

The round-3 "one section per `response_id`, merge non-consecutive runs" framing was reconsidered when
planning surfaced its cost: merging + hoisting interleaved assistant messages **below** the collapsed work
**reorders mid-turn narration**, decoupling it from the work it describes. **Resolution: keep per contiguous
run grouping** (as original T-1) → interleaved turns render **multiple chips in chronological order** with
the messages between them. A′'s flash-free core is **unchanged**: the section entity is keyed by
`(response_id, run_index)` (finalize-stable — `run_index` doesn't shift when a run's streaming tail settles;
only the rare coarse reconcile re-keys), and the **collapse flag is derived per `response_id`** so §4 timing
folds a whole turn's runs together. **This made Task 1 SIMPLER** (closer to original T-1: drop the
flat-when-live branch + stamp `run_index` + fold the live `StreamingReasoning` into its run — no merge logic).
Recorded in spec §13/D-3 + plan Global-Constraints decision #1 + Tasks 1/11/12. Cost: per-run chips (a
`WorkSectionMeta` per-run-vs-per-turn question → **T-6**).

## Plan structure (15 tasks, two phases)

- **Phase A — lens-core (Tasks 1–6, HIGH-CONFIDENCE, verbatim code):** T-1 amendment (per-run + `run_index`
  + `StreamingReasoning{response_id}`) · `StreamUpdate::Reconnected{gap}` · `AccId` minted at accumulator open ·
  `Retired{acc_id,disposition}` + terminal/reconnect scratch retirement · `TranscriptRewritten{ordinal}` ·
  read-only `TranscriptReader` + transactional `(ordinal,Item)+watermark` ranged read. Each has inline TDD +
  gate + a specific codex review prompt.
- **Phase B — lens-ui (Tasks 7–15, SPIKE-REFERENCED, needs real-window iteration):** FleetStore reader-factory +
  reconcile-epoch · poller fan-out via `WeakEntity` + route `reconcile_in_flight` · replica skeleton +
  `live_section_start` · serialized reader worker (coalesce/priority/Retryable-Fatal/focus-gen) · production
  `RowStore` (owned presentations) · **Task 12 = the crux** (two-level retained entities + staged finalize,
  MANDATORY real-window test + Opus review) · `list()` surface + 4 scroll contracts + mount in `#chat-slot` ·
  `ReconnectBreak` anchor · `syncing…` debounce + release perf gate. gpui glue lifts
  `spikes/transcript-virtual/src/` — cited per task; the run is the only proof ([[gpui-test-noop-text-system]],
  [[terminal-realwindow-harness-pitfalls]]).

## Process / rules for the execution session

- **Subagent-driven** (`superpowers:subagent-driven-development`): fresh subagent per task, two-stage review.
- **Delegation (CLAUDE.md):** builds → `cursor-delegate` on `composer-2.5`; **every lens-core change gets ≥1
  cross-family review** — gpt-5.6 ONLY via `codex exec -s read-only … < /dev/null` (the `< /dev/null` avoids
  the stdin hang); Opus subagent for Task 12's staged-finalize architecture + the final §12 synthesis review.
  Delegated gates MUST include `cargo fmt --check`.
- **Gate:** `cargo run -p xtask -- gate` (fmt + workspace clippy `-D warnings` + tests + drift). NO `cargo xtask`
  alias. Verify T-1 tests still green after Task 1: `cargo test -p lens-core reduce::view`.
- **Merge coordination:** `terminal-ws` concurrently touches `reduce/` — T-2's `update.rs`/`snapshot.rs`/`folds.rs`
  touches are small; second-to-merge reconciles.
- **Scope fences (do NOT let leak in):** windowed baseline / scroll-back paging / bounded-tail reconcile → T-2b;
  rich content → T-3/T-4 (T-2 renders **stubs**); live tool-tail → T-4; `WorkSectionMeta`/composer → T-6/T-7;
  `ContentTab` stays an inert marker.

## Commits this session (docs only, on `lens-transript`, UNMERGED/unpushed)

`b3ba61f` T-2 plan (15 tasks) + D-3 refinement to per-run sections · (this handoff + STATUS update to follow).
No code changed. Nothing merged or pushed (user's call — [[commit-when-finished]]).
