# Handoff — state-model P3-3a grilled → write the plan — 2026-07-10

## TL;DR for the next session

P3-3 was **sliced into 3a (lifecycle core) / 3b (recovery semantics)**, and **P3-3a
was grilled to shared understanding**. The decisions are committed to the spec and the
LOCKED design docs are amended. **Nothing is blocking.** Start a **fresh session** and
run `writing-plans` for **P3-3a** from spec §2.3, then execute subagent-driven (same
shape as P3-1/P3-2).

- **Spec (SSOT):** `docs/superpowers/specs/2026-07-08-state-model-engine-design.md`
  **§2.3 (D19–D22)** — authoritative. Also §4 P3 (slice note), §7.1 (amendment tracking).
- **Amended design docs:** `app-architecture-and-state-model.md` (§3.4, §4.1, §6.3, §8),
  `typed-client.md` (§7 + Bootstrap). All carry dated `2026-07-10, P3-3a` amendment blocks.
- **Commit:** `9d39232` on `main`, **docs-only, NOT pushed** (one ahead of origin).
- **Memory:** `state-model-p3-3a-grilling` (decisions), plus `state-model-p3-grilling`,
  `state-model-p3-1-actor-foundation`, `composer-send-recovery-and-history`.
- **Builds on:** P3-1 (`crates/lens-core/src/actor/`, `crates/lens-store/`) + P3-2
  (D16/D18, merged `d5df2a1..51b10af`).

## What P3-3a delivers (D19–D22, spec §2.3)

- **D19 — sole-fetcher forward catch-up + transport-only reader.** Reconcile = bounded
  wake-load (control scalars + `next_ordinal`) + **unbounded actor-owned forward
  catch-up** (`Sessions::items(after=frontier_item_id, order=asc)` until
  `has_more==false`; `frontier` = newest item on disk). Runs **on the actor thread**,
  mode-switched (drain events/commands → one bounded blocking page → `Stop`/`Sleep`
  check → repeat); live events arriving during catch-up are **buffered then drained**
  on completion (keeps ordinals contiguous). The **`lens-client` reader goes
  transport-only** — delete the `items()` fetch + `items_to_replay` from `reconnect()`
  and `bootstrap()`, shrink `Reopen` 3→2 (`open_stream`, `snapshot`), delete
  `HttpReopener::items()`/`items_to_replay()`. Subtractive, but a **MANDATORY
  cross-family-review seam** (hardened crate).
- **D20 — actor holds a small pruned working set, NOT an 8 MB byte-window.** Disk is
  canonical for finalized items: `reduce → write-through → emit StreamUpdate → prune`.
  Far-back re-fire = **blind idempotent disk upsert-by-id** (event carries the full
  item; no RAM lookup). The ~8 MB render window is a **deferred replica concern**. 3a
  therefore **drops actor-side eviction / byte-accounting** in favor of
  prune-after-write-through. `Rebased` drops its item payload.
- **D21 — sleep = `SessionCommand::Sleep`, wake = respawn, trigger external.**
  `is_quiesced()` = pure `transient_work_outstanding()` ∧ `transport==Connected` ∧
  `!reconcile_in_flight`. Sleep processed in-loop (re-check → flush [`lifecycle=Slept`,
  `last_seen_seq`] → best-effort `stop_session` → stop → registry `Slept`). Wake
  respawns from disk. 3a builds a **skeletal `FleetScheduler` seam** + a deterministic
  round-trip test (injected `Clock`, mock `Reopen`, temp `TranscriptStore`) — no
  wall-clock, no UI. §9 timer/LRU/focus deferred.
- **D22 — never-seen-huge first-attach deferred whole** (snapshot-tail-paint +
  negative-ordinal scroll-back); `ordinal` is already `i64` so no migration later.
- **D15 rider** (still UNFIXED): `fold_snapshot` (`reduce/snapshot.rs`) never sets
  `state.created_at` (add `state.created_at = snap.created_at()`); P2 upsert
  (`persist/control.rs:~102`) needs the first-non-zero guard
  (`CASE WHEN sessions.created_at != 0 THEN sessions.created_at ELSE excluded... END`).

## Task order (build catch-up BEFORE deleting reader replay — else broken intermediate)

1. **D15** — `created_at` P1 fold + P2 first-non-zero guard (independent, small).
2. **Pure predicate** — `SessionState::transient_work_outstanding()` + actor
   `is_quiesced()` (unit-testable, no actor thread). Verify: `is_quiesced`/
   `transient_work_outstanding`/`sleep`/`wake` do **not** exist yet (only the
   `runloop.rs:141` quiescence-gate comment); `transport`/`reconcile_in_flight`
   already actor-owned from P3-2.
3. **Actor forward catch-up + prune-working-set + `Rebased`-drops-items** (mode-switched
   loop, live-buffer-then-drain). Uses existing `Sessions::items(id, &ItemsPage)` +
   `has_more`; new `TranscriptStore` "frontier" query (max ordinal + its `item_id`).
4. **Reader → transport-only** (subtractive `lens-client`) — **review seam**.
5. **`Sleep`** command + ordering + **wake respawn** from disk.
6. **`FleetScheduler` skeletal seam** + deterministic round-trip test + **gated D17
   live-verify** (post-`stop_session` effects durably re-fetchable on wake).
7. **Docs** (STATUS/handoff/progress).

## Gotchas / non-obvious

- **`native ⇏ pending_id`** carries over from P3-2 — irrelevant to 3a's catch-up but
  don't regress the reconcile keying.
- **`/items` persisted rows have no `seq`** — the tail/frontier is delimited by
  **`item_id` overlap**, not sequence. `live_seq` (`TranscriptStore` column) is NULL
  for disk/reconcile rows.
- **Catch-up execution = actor-thread mode-switched** (worker-thread + third channel is
  a deferred, localized upgrade — do NOT build it in 3a).
- **Blast radius warning:** D20 touches merged **P3-1** code — the actor currently holds
  `items: Vec<Arc<Item>>` and `Rebased` carries items. 3a includes a *contained
  revision* of that (prune after write-through; `Rebased` drops items; baseline items
  from disk on promote — the disk-load path is deferred). Low-risk (no renderer consumes
  it yet) but deliberate. Keep the P1 pure-reducer contract intact — the actor prunes,
  the reducer still mutates a small `state.items`.
- **The D17 live-verify rider** is the only live-server dependency in 3a; batch it into
  one gated run (`installing-omnigent-from-source`, pinned 0.4.0). Not spec-blocking.

## P3-3b (later — its own grilling+plan)

Held-bubble resume (401/Parse/ContractMismatch bubbles have no resume-resend path),
`SendLost` re-derivation (variant exists, unproduced — naive diff false-positives on
landed sends), command-path `Auth 403`/`NotFound` §9 escalation, parked-feeder drain /
outcome-channel wedge policy. Coupled to composer send-recovery + input-history
(memory `composer-send-recovery-and-history`).
