# Handoff — Transcript T-2 design-locked, ready for `writing-plans`

**Written:** 2026-07-22 · **Branch:** `lens-transript` (UNMERGED, not pushed this session)
**HEAD:** `a8e1b5e` · **Spec:** `docs/specs/2026-07-21-transcript-t2-focused-view-scaffold-design.md` (rev 4, **architecture-locked**)
**Next action:** start a **fresh session**, invoke **`writing-plans`** against the spec.
**Memory:** [[transcript-workstream-decomposition]] · [[t1-viewblock-projection-executed]] · [[state-model-d23-disk-render]] · [[transcript-virtualization-spike-2026-07]] · [[omnigent-two-id-space-reconciliation]] · [[premature-layer-boundary-binding]]

## TL;DR

T-2 (focused view scaffold + live disk-sourced surface — the **first real consumer** of
T-1's `Vec<ViewBlock>`) was brainstormed → design-locked over this session. The design
survived **three GPT-5.6/codex review rounds** (each REWORK→fixes); all mechanical/plumbing
findings are closed, the three hard design decisions are resolved with the user, and their
round-3 contract refinements are folded in. The spec is **architecture-locked, deliberately
not "review-green"** — the residual items are mechanism/contract detail that the **plan +
TDD** nail better than more prose (path **B**, user-chosen). **Do not re-run the design
review; write the plan.**

## Workstream reorg landed this session (STATUS + SPEC-GAPS updated)

- **T-2** tightened to the consumer machinery (below). **T-2b** split out (byte-budgeted
  windowed baseline + scroll-back paging + bounded-tail reconcile) — **next after T-2, NOT
  deferred**. **Live in-progress tool-tail** feed extension moved to **T-4**. Polymorphic
  `ContentTab` protocol deferred to terminal-UI-integration (SPEC-GAPS cross-spec-risks;
  shell §7.2). Order: **T-2 → T-2b → T-3 → T-4 → T-5 → T-6 → T-7**.

## The locked design (read the spec §13 + §5/§6 for the real detail)

- **D-3 → A′ (two-level, group-from-birth):** every turn's work is a `WorkSection` from
  birth — Level-1 entity per `response_id`, Level-2 child entities; expanded when live **or**
  latest-settled-until-next-user (§4), collapsed otherwise; **finalize flips a render flag,
  nothing remounts** (structural flash-free). Needs a **T-1 amendment** (first build task).
- **D-1 → z:** settled sections cached; only the live section re-projects per delta via a
  **per-`response_id` projection** (not `project_all`); coarse invalidate-all-settled on the
  rare reconcile. O(live turn) steady state; clears the frame budget without T-2b.
- **D-2 → ii:** reducer emits `Retired { acc_id, disposition = Finalizing { item_id } |
  Discarded }`; `Finalizing` stages by `item_id` until the disk row swaps in place;
  `Discarded` (reconnect + terminal Failed/Incomplete/Cancelled) drops with no ghost.

## Skeleton decisions (rev 2, still standing)

Feed is single-consumer → **one poller fans out via a `WeakEntity<FleetStore>` batch
dispatch** to card + a store-side `FocusedTranscript` replica (installed **before**
`Promote`). Replica opens a **read-only `TranscriptReader`** (busy_timeout, no DDL) on a
**dedicated serialized reader worker** with a bounded coalescing queue + Retryable/Fatal
states + focus-generation gating; one transactional `(ordinal, Item) + watermark` read.
Native `list()`/`ListAlignment::Bottom`; **`splice` for live changes, `reset` only for
new-session** (reset=bottom-follow would yank a paused reader). `ReconnectBreak` = UI-only
marker with `{after_ordinal, seq}` anchor on `gap != Some(0)`.

## lens-core footprint the plan must schedule (each cross-family reviewed)

1. **T-1 amendment** (`reduce/view.rs`) — the first task; response-keyed uniform grouping
   (merge non-consecutive runs), stamp live `StreamingReasoning` with `response_id`, keep
   `active_response` as a projection input, update ≥4 of T-1's 21 tests. (T-1 spec §5.3
   already annotated superseded.)
2. `StreamUpdate::Reconnected { gap }` (`reduce/update.rs`, `snapshot.rs`).
3. `Retired { acc_id, disposition }` from finalize/discard/terminal paths (`reduce/items.rs`,
   `folds.rs:221` must retire scratch on Failed/Incomplete/Cancelled).
4. `id` on `ReasoningAcc` threaded to `finalize_reasoning` (`domain/item.rs`, `reduce/items.rs`).
5. `TranscriptReader` read-only opener + `busy_timeout` + transactional ranged read
   (`persist/`), public on `TranscriptReader` only.
6. Route `ActorOutcome::TransportChanged.reconcile_in_flight` to the replica (`fleet/poller.rs`
   discards it today), + a per-session **reconcile epoch** retained in `FleetStore` for the
   focus-mid-reconcile case.
7. `FleetStore` retains a per-session **reader factory** (`data_dir`/`conn_id`/`session_id` —
   discarded today, `store.rs:64-70`).

## Open mechanism items the PLAN resolves (not spec prose)

- `response_id` vs `(response_id, run)` section identity — confirm against real transcripts
  (recommendation: `response_id`, merge runs).
- Exact `Retired` payload + which reducer sites emit it.
- The per-`response_id` live-projection index shape (so `ScratchChanged` is O(live)).
- Silent **re-fire** below-watermark update (§3.4 partial-1) — actor emits a lightweight
  below-watermark-changed signal, or accept bounded staleness until next reconcile. Decide.

## Process / rules for the planning session

- **Delegation (CLAUDE.md):** default subagent work → `cursor-delegate` on `composer-2.5`;
  Opus subagent only for architecture/security/synthesis; **gpt-5.6 review ONLY via codex**
  (`codex exec -s read-only ... < /dev/null` — the `< /dev/null` avoids the stdin hang).
  Every non-trivial change gets ≥1 cross-family review.
- **Reviews saved** this session: `…/scratchpad/t2-codex-review.txt` (round 1),
  `t2-codex-rereview.txt` (round 2), `t2-codex-review3.txt` (round 3). The scratchpad is
  session-specific — copy out if a later session needs them; the findings are all folded
  into the spec, so this is optional.
- **Verify green anytime:** `cargo run -p xtask -- gate` (fmt + workspace clippy -D warnings
  + tests + drift; **no** `cargo xtask` alias). T-1's tests: `cargo test -p lens-core reduce::view`.
- **Merge coordination:** `terminal-ws` concurrently touches `reduce/` — T-2's touches
  (`update.rs` `Reconnected{gap}`, `view.rs` amendment, `persist/`) are small; second-to-merge
  reconciles.

## Commits this session (all docs, on `lens-transript`, UNMERGED/unpushed)

`b04380e` design+reorg · `5284892` rev 2 (round-1 fixes) · `c420e8c` status · `e54f2ac` rev 3
mechanical (round-2) · `b1bccc4` design-locked D-1/D-2/D-3 · `a8e1b5e` rev 4 (round-3 fixes).
No code changed. Nothing merged or pushed (user's call — [[commit-when-finished]]).
