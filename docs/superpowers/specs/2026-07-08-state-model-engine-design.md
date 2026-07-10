# State-model engine — implementation spec

**Date:** 2026-07-08
**Component:** the per-session state-model engine (`lens-core` + `lens-store`)
**Consumes:** `lens-client` (typed omnigent client, feature-complete on `main`)
**Design source of truth:** `docs/design/app-architecture-and-state-model.md`
(LOCKED sections §2, §3, §4, §6, §7, §8, §13). This spec does **not** re-decide
that architecture — it records how to *build* it, phased, with the per-phase
implementation plans deferred.

---

## 1. Scope

Builds the complete state-model machinery for **one** `(connection, session)`:

```
EventStream ─▶ reduce(&mut SessionState) ─▶ canonical state
                 ├─▶ SessionPersistence write-through (SQLite)
                 └─▶ StreamUpdate ─▶ SessionStore replica (gpui) ─▶ notify
UI action  ─▶ SessionCommand ─▶ ActiveSession actor
```

**In scope** (per-session engine): §2 domain types, §3 lifecycle transitions,
§4 pure reducer + render transforms, §6 persistence, §7 command flow, §8
`ActiveSession` actor + `SessionStore` replica + the two-direction seam, §13.1
error→state mapping.

**Out of scope** — the app-level orchestration that manages *many* engines,
carved into their own specs (each builds ON this one):

- §5/§9 multi-connection `AppState`/`ConnectionApp`/registry/navigation → the
  **immediate next spec**. This spec exposes the seam (a `SessionHandle`-shaped
  handle + a coarse-update apply path) but does not spec the registry internals.
- §10 cross-session list poll → with §9.
- §11 Bridge router, §12.3 Concierge → separate features (own consumer surfaces).

Rationale for the cut is in §7 Deferred below and the design discussion: the
per-session engine is the load-bearing, highest-risk, independently-verifiable
unit — it is exactly what `lens-client` was hardened for. §9/§10 wrap engine
types and would inherit the engine's contract-stability risk if specced before
the engine contract is proven against real bytes.

---

## 2. Decisions

- **D1 — Two crates.** `lens-core` (framework-neutral: domain, reducer,
  transforms, persistence, `ActiveSession` actor, seam types) + `lens-store`
  (gpui: `SessionStore = Entity<SessionState>` replica + the `cx.spawn` drain
  bridge). Localizes the design's §14 *three* framework touch-points to one
  small crate and keeps the pure engine testable without pulling gpui into every
  test. The actor is "plain off-thread Rust, not a gpui entity" (§14), so it
  lives in `lens-core`.
- **D2 — Scope = per-session engine.** §9 registry / §10 poll = **named seam
  only**, next spec. Bridge/Concierge deferred.
- **D3 — Single-writer, re-stated from LOCKED §8.** For each Active session,
  canonical `SessionState` is owned and `&mut`-mutated by exactly one
  `ActiveSession` actor off the foreground thread; every mutation flows through
  `reduce()` in that actor's run-loop and nowhere else. `SessionStore` is a
  read-only foreground replica: it applies self-contained `StreamUpdate`s and
  notifies, never reduces, never originates state. Not re-litigated here (memory
  `state-model-single-writer-decision`).
- **D4 — Persistence, two-tier (grilling revision).** `SessionPersistence` trait
  splits into a **`ControlStore`** (one `lens.db`: connections/sessions/
  cost_samples/meta) + a per-session **`TranscriptStore`** (one SQLite file per
  `(connection, session)`, actor-owned WAL connection — items only). `rusqlite`,
  portable (jsonb-mappable) schema per LOCKED §6.1/§6.2. The split makes actor
  writes contention-free by construction and retention/tombstone a file op;
  transcripts are re-fetchable via `GET /items` so isolating them is safe.
  Backing-store swap is behind the trait.
- **D5 — Blocking OS thread, no tokio** on the core data path — matches
  `lens-client` D2 (the typed client's `EventStream` is a blocking reader).
- **D6 — `StreamUpdate` drafted at P1, ratified at the P3 skeleton.** The
  `StreamUpdate` enum (the reducer's output per LOCKED §4.1 —
  `reduce() -> SmallVec<[StreamUpdate; 2]>`) is *drafted* empirically by the P1
  reducer against the golden SSE corpus: P1 freezes the **semantic deltas** (which
  events produce which state changes). Its **apply-side representation** may still
  refine when the P3 walking skeleton exercises `StreamUpdate::apply` on the gpui
  replica (and P2 write-through) — so the contract is *ratified* at the P3
  skeleton, not P1 exit. The de-risking still holds: P0/P1 land without waiting on
  P3, and P2/P3 plans are written against the P1 draft and reconciled at the
  skeleton. (Reviewer note, gpt-5.5: an early hard freeze would hide the apply/
  write-through coupling — hence draft-then-ratify, not freeze-at-P1.)
- **D7 — Walking skeleton = P3 task 1.** The off-thread→foreground handoff
  (blocking OS thread → channel → gpui `Entity` → `cx.notify`) is proven by a
  minimal end-to-end slice before the real reducer volume lands in P3 — no
  separate throwaway spike (the pattern is well-trodden in gpui/Zed; the risk is
  our specific blocking-thread↔executor shape + backpressure, which the skeleton
  exercises).

### 2.1 P3 grilling refinements (2026-07-09) — D8–D14

Resolved in a focused grilling pass over the actor↔replica seam before writing
the P3 plan. These are authoritative for P3; where one amends a LOCKED design
section it is flagged **→ design-doc amendment** and listed in §7.1.

- **D8 — `StreamUpdate` ratified as value-carrying deltas (option A), `items:
  Vec<Arc<Item>>`.** Ratifies D6. Each variant carries its just-reduced value
  (`ItemAppended(Arc<Item>)`, `ItemUpdated { index, item: Arc<Item> }`,
  `StatusChanged(SessionStatusValue)`, …); `SessionStore::apply` is **pure
  copy-assignment** — it deposits the already-reduced value into the named field,
  never re-derives, never runs `reduce` on the foreground — O(1)/delta. `items`
  becomes `Vec<Arc<Item>>` so item **bodies are shared** between the actor's
  canonical state and the replica: the replica's "copy" is a pointer spine +
  small scalars, *not* a second copy of transcript bytes. **Rejected B**
  (whole-state snapshot swap): a plain-`Vec` deep clone is O(n²) over a streaming
  turn and would force an `im::Vector`/`Arc<Vec>` restructure to be viable.
  Supersedes the P1 marker-only draft in `reduce/update.rs`.
- **D9 — `Rebased(Box<SessionState>)` bulk variant at actor attach; no
  remove/clear variant.** The reducer only ever **appends or updates-in-place**
  (verified `reduce/items.rs:183` — id-hit → `ItemUpdated { index }`, id-miss →
  push → `ItemAppended`); it never removes/clears/truncates items. So steady-state
  `StreamUpdate` needs only those two item variants. The one exception is the
  **baseline at attach**: when an actor attaches/promotes to a full replica it
  emits `Rebased(Box<SessionState>)` **once** (disk-painted baseline: identity +
  scalars + resident item window); the replica does `*state = baseline`, then all
  subsequent deltas are incremental A. This keeps **index-based `ItemUpdated`
  sound** (the replica is a faithful mirror *from the rebase forward*) and is the
  one place a whole-state swap is correct **and** cheap (once/attach, not per
  event). Distinct from `SnapshotRestored` (scalar-only mid-session reconnect
  chrome).
- **D10 — Fidelity is focus-scoped; the actor is dual-mode.** A **full** replica
  (with items) fed by full `StreamUpdate`s exists **only for focused sessions
  (≤ ~10)**. Background-warm Active sessions get only a coarse **`SummaryUpdate`**
  feed (status/title/tokens/needs-attention/sub-agent-active), emitted **by the
  actor** at within-turn **ms–s** cadence — not the per-token full delta stream.
  (Same `SummaryUpdate` *type* as the §10 poll uses for Slept sessions, but here
  the actor is the producer — two producers, one type.) The actor supports two
  output modes (`Detailed | Summary`); **promote** (on focus) emits a `Rebased`
  baseline then `Detailed` deltas; **demote** (on blur) drops the full items and
  reverts to `Summary`. **This spec builds the actor's dual-mode capability + the
  promote/demote primitive; the trigger *policy* (focus set, active-set LRU) is
  §9 (seam only).** → **design-doc amendment §9:** the current wording ("every
  Active session's actor emits `StreamUpdate`s into [a full] replica") is
  render-scoping, not memory-scoping; full-fidelity duplication must be bounded by
  the **focus** count, not the **warm** count.
- **D11 — Byte-windowed in-RAM transcript (retention).** Canonical `items` in the
  actor is a **byte-budgeted tail** (target ~8 MB, count backstop), not the full
  history. Real sessions reach **~600 MiB / 10k–100k items** (multi-day,
  auto-compacted); items are **bimodal** (100 B markers vs 200 KB dumps), so the
  window is sized by **bytes, not item count**. Full history lives in the
  per-session `TranscriptStore` file; older items **lazy-load on scroll-back**
  (indexed by `ordinal`, paged, off-thread, prefetch one page ahead) and the
  resident tail evicts. `TranscriptStore` grows a **windowed/paged load +
  hydrate-older** primitive. Keeps fleet RAM flat (~240 MB @ 30 warm × ~8 MB)
  regardless of session age. → **design-doc amendment §6/§15:** promotes "disk
  retention" from a deferred tunable to a **designed P3 seam** (thresholds still
  tunable). **[Refined by D20 (§2.3): the ~8 MB byte-window is a *replica*/render
  concern, not an actor tail — the actor holds a small pruned working set and disk
  is canonical for finalized items. 3a builds prune-after-write-through, not
  actor-side eviction.]**
- **D12 — Large-transcript latency spike (new P3 task, sequenced FIRST).**
  Throwaway harness against a synthetic **~500 MiB / ~100k-item** transcript,
  measuring: (1) windowed page-load (scroll-back) — expect ~1–10 ms/page;
  (2) cold hydrate (Slept→focus) — expect ~5–20 ms; (3) **`reconcile` at scale +
  its correct scope** — the real unknown: naïve reconcile-by-id over 100k rows is
  O(transcript), so it likely must bound to the reconnect **tail** (since
  `last_seen_seq`), which entangles the server `GET /items` **pagination** contract
  deferred from plan 3b-2b. Sequenced **before** the sleep/wake wiring because it
  sets the `reconcile` contract wake/reconnect depend on. Same throwaway-spike
  calibration as the framework spikes.
- **D13 — Actor ingest = crossbeam `Select` over two typed channels.** The actor
  must block-wait on **both** the `EventStream` receiver and the `SessionCommand`
  receiver (commands serviced promptly even during heartbeat-only idle).
  `std::mpsc` can't select; a busy-poll (rejected) wastes a wakeup on every idle
  warm session; a forwarder thread adds a per-event context-switch tax (~1–5µs) +
  2-stage backpressure. **Chosen:** swap lens-client's reader channel
  `std::sync::mpsc::sync_channel → crossbeam_channel::bounded`, expose a
  `receiver()` accessor, and the actor `Select`s over `(events, commands)` — zero
  busy-poll, zero extra thread, and lens-client stays a **pristine typed event
  source** (no consumer-type leakage; complexity lives in the new lens-core code).
  It is `tokio::select!` for the blocking world — recovers the one ergonomic tokio
  would have given without reintroducing an async runtime beside gpui or
  re-async-ifying hardened `reqwest::blocking` lens-client. **Cost:** a localized
  edit to feature-complete/hardened lens-client under the backpressure +
  `stop()`/drop-unblock semantics → re-verify those + **MANDATORY cross-family
  review** of the diff (temporal/stateful, `[[composer-delegation-profile]]`).
  Adds `crossbeam-channel` as a workspace dep. Perf: 1-hop, parks when idle;
  equivalent to the merged-mailbox alternative on the hot path (select bookkeeping
  ~100–300 ns/event ≪ 1.36µs reduce); **none of this touches the 120fps foreground
  apply path** (a separate channel).
- **D14 — Design-memo rationale correction (§8).** The two-copy (actor + replica)
  justification must read **"off-thread to decouple N warm *background* streams
  from the gpui foreground executor"** — NOT "off-thread because reduce is
  expensive." `reduce` is **1.36µs/turn** (P1 bench); the load-bearing reason is
  that N mostly-idle warm sessions must advance + persist without waking the UI
  thread per event, and gpui entities are foreground-mutation-only, so canonical
  state cannot itself be the entity. → **design-doc amendment §8** + memory
  `state-model-single-writer-decision`: without this a future reader correctly
  notices reduce is trivial and concludes the whole actor layer is pointless.

### 2.2 P3 grilling refinements (2026-07-09, session 2) — D15–D18

Closes the four branches left open after D8–D14 (the `/grilling` resume). Each is
spec-decidable now; where one rests on a live-server observation it is flagged as
a **live-verify rider** (not spec-blocking) and batched into the single P3 live
run (§4 P3 gate).

- **D15 — `created_at` is an immutable first-non-zero stamp; also fold it from the
  snapshot.** Closes the P2-deferred clobber (memory `state-model-p2-persistence`).
  Two complementary fixes: **(1, P2 SQL)** the `sessions` upsert stops doing
  `created_at=excluded.created_at` unconditionally and becomes first-non-zero-wins:
  `created_at = CASE WHEN sessions.created_at != 0 THEN sessions.created_at ELSE
  excluded.created_at END` — an immutable creation stamp is never overwritten once
  set, and an actor writing fresh state (`created_at = 0`, pre-bootstrap) can never
  clobber a good value the §10 list-poll wrote. **(2, P1 reduce — a genuine defect
  found this session)** `fold_snapshot` (`reduce/snapshot.rs:18`) folds ~25 fields
  but **never sets `state.created_at`**, so within this engine the actor's
  `created_at` is *always 0*; add `state.created_at = snap.created_at();` (accessor
  exists, epoch **seconds** per §2 `session.rs:27`). The guard makes disk *safe*;
  the fold makes the actor-written value *correct*. Guard belongs to P2, fold to P1.
- **D16 — Optimistic-send reconcile is keyed by server ack ids, with content-match
  as the defensive floor.** Ratifies the §4 P3(b) collision. **Finding:**
  `SendEventAck` (`lens-client sessions.rs:697`) **already models `pending_id`
  (native bypass) and `item_id` (persisted store id, non-native)**, and
  `send_event` returns the full ack — so a server-authoritative correlation id is
  plumbed *today*. Restructure `PendingUserMessage` (`domain/controls.rs:71`) to
  separate Lens-local from server ids: keep `pending_id: String` (Lens-local,
  addresses the bubble for rollback/UI) and add `server_pending_id: Option<String>`
  (native, ← `SendEventAck.pending_id`) + `store_item_id: Option<String>`
  (non-native, ← `SendEventAck.item_id`), both stamped at POST-return. **Reconcile
  precedence (identical for the live `consumed` stream and the reconnect replay):**
  (1) `server_pending_id` present → native by-id (live `cleared_pending_id` /
  snapshot `pending_inputs[].pending_id`); (2) `store_item_id` present → non-native
  by-id (replayed `GET /items` item whose `id ==` it → drop bubble); (3) ack empty
  → the §4 P3(b) content/ordinal match. Sits inside D12's tail-bounded reconcile
  scope (bubbles are always at the tail). **Supersedes** the §4 P3(b) rule-3 framing
  ("no correlation id exists, do not design assuming one"): a *client-supplied* id
  still doesn't exist, but the *server-returned* ack id does — carry the slot.
  **Live-verify rider:** confirm the ack populates `pending_id`/`item_id` at runtime
  (`#[serde(default)]` masks an empty body as `None`, and no POST-ack body is in the
  corpus). If confirmed, (1)/(2) are the common path and (3) is defensive-only; if
  not, (3) carries it and nothing is lost.
- **D17 — `is_quiesced` = a pure-core predicate ∧ an actor transport conjunct;
  sleep is flush-first with a re-check guard.** **Predicate split:** the six
  content clauses are a pure `SessionState::transient_work_outstanding()` in
  lens-core (unit-testable, no actor) — quiesced needs `status ==
  SessionStatusValue::Idle` ∧ `stream.open_message/open_reasoning` both `None` ∧
  `stream.unpaired_calls.is_empty()` ∧ `pending_user.is_empty()` ∧
  `pending_elicitations.is_empty()` ∧ `!terminal_pending`. The §3.2 **"unreconciled
  reconnect"** condition has **no field** (verified: reducer records no
  reconnect phase — it is transport, not content), so the actor's `is_quiesced()` =
  that pure predicate ∧ `transport == Connected` ∧ `!reconcile_in_flight`, where
  `reconcile_in_flight` is an **actor-owned, never-persisted** flag (true from
  `Disconnected`/`Reconnecting` until the post-reconnect reconcile completes).
  **"Pinned" is NOT in the predicate** — it is held-by-intent, a §9 scheduler gate
  ("don't *call* `sleep()`"), not a transient-work condition. **"Recent terminal
  activity" is subsumed by the scheduler's ~10-min idle timer** — no separate
  cooldown timestamp (recent terminal activity ⇒ not idle recently ⇒ timer hasn't
  elapsed). **Sleep ordering** (`sleep()` on the actor): (1) **re-check
  `is_quiesced()` atomically, abort-and-stay-Active if false** — the actor is
  single-threaded, so this closes the scheduler-check→`sleep()` TOCTOU; (2) **flush
  durable** — final transcript upsert committed + control write `lifecycle=Slept`,
  `last_seen_seq`, `last_focused_at`; (3) **best-effort `stop_session`** —
  fire-and-forget, timeout-bounded, outcome → introspection ring, **never blocks the
  flush**; (4) stop actor + close stream; (5) drop heavy RAM. Flush-first (not
  stop-then-flush) is safe because the predicate already guaranteed terminal state,
  so `stop_session` yields no meaningful transcript deltas. **Live-verify rider:**
  confirm post-`stop_session` server effects are durably re-fetchable on wake
  (`GET /items`/snapshot) — the design breaks only if some effect is live-stream-only
  and never persisted; that is the one thing the bytes must rule out.
  **[Implemented/refined by D21 (§2.3): sleep = `SessionCommand::Sleep` (in-loop
  re-check), wake = respawn-from-disk, trigger is the external §9 scheduler; 3a
  ships a skeletal `FleetScheduler` seam + a deterministic round-trip test.]**
- **D18 — §13.1 splits into two path-keyed tables; recoverable disconnects park,
  terminal ones stop.** **Finding:** `Disconnected { reason: DisconnectReason }`
  (`lens-client stream/event.rs`) already carries a 5-variant reason
  (`Unauthorized|Forbidden|NotFound|SessionFailed|RetriesExhausted`), each
  pre-annotated with intent — so auth/notfound/failed on the **stream path** arrive
  *folded into the terminal event*, distinct from the same conditions on the
  **command/REST path** (which return `ClientError`). The design §13.1 table
  conflates both paths in one flat list; split it. **Table A — terminal
  `Disconnected{reason}` → actor lifecycle:** `Unauthorized` / `SessionFailed` /
  `RetriesExhausted` → **park** (close stream, keep actor + state resident, await
  re-auth/user-retry via `Sessions::stream`); `Forbidden` → **stop** + remove from
  registry; `NotFound` → **stop** + local read-only tombstone. A parked session is
  **not** quiesced (`transport != Connected`) so it will not auto-sleep — it holds
  RAM until the user acts; any force-reclaim of piled-up parked sessions is **§9
  policy**, not this engine. **Table B — `ClientError` on command/REST → command
  outcome** (fills three gaps in the design table): `Server { status, body }`
  (**absent** from the design table) → 5xx = transient (log/marker/retry-eligible),
  other-4xx = denied/bug (surface, no retry); `ThreadSpawn` (**absent**) → fatal at
  stream-open, actor never starts, session can't go Active; `Ws` in the design table
  → **no such `ClientError` variant** (WS terminal deferred, no `terminal.rs`) — drop
  or mark forward-looking; `Network`/`Parse`/`Auth`/`NotFound`/`ContractMismatch`
  scope to command outcome (e.g. `Network` on `send` → roll back the optimistic
  bubble per D16), **not** stream teardown. → **design-doc amendment §13.1** (§7.1).

### 2.3 P3-3a grilling refinements (2026-07-10) — D19–D22

Resolved in a grilling pass over the P3-3 *lifecycle* seam (wake/sleep, reconcile
scope, transcript retention) before writing the P3-3a plan. **These slice P3-3
and materially revise D8/D9/D11 and the D14 rationale** — where a refinement
supersedes an earlier decision it says so, and the change is listed in §7.1.

- **D19 — Reconcile splits into a bounded wake-load and an unbounded actor-owned
  forward catch-up; the actor is the *sole* `/items` fetcher, and the reader goes
  transport-only.** "Reconcile" was conflating three concerns — durable
  completeness (disk), the resident RAM window, and what blocks the wake hot-path.
  Split them:
  - **Bounded wake path:** on respawn the actor loads control scalars +
    `next_ordinal` from disk and opens the stream. Bounded by construction — no
    full-history read (the Task 0 spike's requirement).
  - **Unbounded forward catch-up:** the actor pages `Sessions::items(after =
    frontier_item_id, order = asc)` until `has_more == false`, appending to
    `TranscriptStore` at contiguous ordinals. `frontier = newest item on disk`
    (empty ⇒ from oldest). Gap-proportional ("takes as long as it takes") but
    **off the hot path** and disk-targeted. Runs **on the actor thread**,
    mode-switched (drain events/commands → one bounded blocking page → `Stop`/`Sleep`
    check → repeat), consistent with P3-2's bounded-blocking `send_event` precedent;
    live events arriving during catch-up are **buffered then drained** on completion
    so ordinals stay contiguous. Triggered on `Reconnected`, wake-respawn, and
    first-attach. A worker-thread variant is a deferred, localized upgrade.
  - **Sole-fetcher / transport-only reader.** Two fetchers from opposite ends do
    not compose: a reader tail-replay (newest-N, backward) punches a hole ahead of
    the gap and poisons the actor's `frontier` (disk-max becomes the top of a hole →
    forward-paging skips the middle → silent loss). Resolution: the **actor owns all
    `/items`** (forward-only, contiguity-preserving); the `lens-client` reader is
    demoted to **transport recovery only** (backoff → re-open stream → snapshot
    chrome → synthetic `Reconnecting`/`Reconnected`/`SnapshotRestored`), and its
    item-replay is **deleted**. Durable, disk-backed, ordinal-assigning gap-fill is
    structurally a *stateful-consumer* job the stateless client cannot own. → **amends
    the 3b-2b "reader owns item recovery end-to-end" decision** (typed-client §7);
    `lens-client` change is subtractive (delete `items()`/`items_to_replay`, shrink
    `Reopen` 3→2 methods) → **MANDATORY cross-family review** despite being deletion.
  - **Never-seen-huge session deferred whole.** Forward-only catch-up is only bad
    for a first attach to a large session Lens has *never* persisted (dawn-of-time
    load, oldest-first). That case — snapshot-tail-paint + scroll-back-backfill on
    **negative/anchored ordinals** — is deferred entirely to the composer/UI work.
    The `ordinal` column is already `i64` (negatives representable today), so the
    door is open with **no migration**. 3a must not foreclose negative-ordinal
    prepend; nothing in the single-forward-behavior does.

- **D20 — The actor holds a *small pruned working set*, NOT an 8 MB byte-windowed
  tail; disk is canonical for finalized items; the render window is a deferred
  *replica* concern. (Corrects D8/D9/D11 and the D14 "30 × 8 MB" figure — a category
  error.)** Walking the reduce access pattern: streaming accumulation lives in
  `StreamScratch`; tool pairing uses `unpaired_calls`; ordinal assignment needs one
  number; reconcile dedup is a disk upsert-by-id. So a **finalized** item's only jobs
  are write-through + emit `StreamUpdate`; after that the actor has no reason to
  retain it. The actor keeps **scratch + scalars + a same-turn working set +
  `next_ordinal` + `frontier`** (turn-bounded, not 8 MB); a far-back re-fire that
  misses the window is a **blind idempotent disk upsert-by-id** (the event carries
  the full item), not a RAM lookup. The ~8 MB figure was conflating the actor's
  *reduce working set* (small) with the *render window* (live-follow) — the latter is
  a **focused-replica** concern (live tail = actor→replica RAM; scroll-back = disk),
  deferred with the viewport/UI work. **3a therefore drops actor-side byte-window /
  eviction / byte-accounting** in favor of **prune-after-write-through**. Blast radius
  is contained to the actor + this rationale correction: the P1 pure-reducer contract
  is kept (`reduce` still mutates a small `state.items` the actor prunes); `Rebased`
  drops its item payload (baseline items come from disk on promote — deferred).

- **D21 — Sleep is a command; wake is a respawn; the trigger is external.**
  Implements D17. `is_quiesced()` = pure `transient_work_outstanding()` ∧
  actor-owned `transport == Connected` ∧ `!reconcile_in_flight` (the transport
  conjuncts already exist from P3-2). **Sleep = `SessionCommand::Sleep`** processed
  in the actor's `Select` loop (re-check quiescence in-loop → flush durable
  [`lifecycle = Slept`, `last_seen_seq`] → best-effort fire-and-forget
  `stop_session` → stop actor → registry `Slept`) — single-writer, closes the
  scheduler→sleep TOCTOU. **Wake = respawn** a fresh actor from disk (a slept actor
  is gone; wake can't be a command). The **trigger** (10-min timer, focus-set/LRU,
  force-reclaim) is the **§9 scheduler**, deferred; 3a builds a **skeletal
  `FleetScheduler` seam** (spawn-on-wake / `Sleep`-command entry points) driven by a
  **deterministic round-trip test** (injected `Clock`, mock `Reopen`, temp
  `TranscriptStore`) — no wall-clock timer, no UI.

- **D22 — P3-3 is sliced into 3a (lifecycle core) and 3b (recovery semantics).**
  3a and 3b have different risk profiles; 3b's held-bubble/`SendLost` logic depends
  on 3a's wake/reconcile path existing.
  - **P3-3a — lifecycle core (this section):** the D19 catch-up + transport-only
    reader, the D20 working-set actor, the D21 sleep/wake + scheduler seam, plus the
    **D15 rider** (`created_at` P1 fold + P2 first-non-zero guard — still unfixed).
    Ships the auto-sleep/wake round-trip green. **Task order** (avoids a broken
    intermediate — build catch-up *before* deleting reader replay): (1) D15;
    (2) pure `transient_work_outstanding()`/`is_quiesced()`; (3) actor forward
    catch-up + prune-working-set + `Rebased`-drops-items; (4) reader → transport-only
    (**review seam**); (5) `Sleep` + wake respawn; (6) `FleetScheduler` skeletal
    seam + deterministic round-trip test + gated D17 live-verify; (7) docs.
  - **P3-3b — recovery semantics (deferred to its own grilling+plan):** held-bubble
    resume (401/Parse/ContractMismatch bubbles have no resume-resend path today),
    `SendLost` re-derivation (variant exists, unproduced — naive diff false-positives
    on landed sends), command-path `Auth 403`/`NotFound` §9 escalation (deferred in
    P3-2), and the parked-feeder drain / outcome-channel wedge policy. Coupled to the
    deferred composer send-recovery + input-history feature (memory
    `composer-send-recovery-and-history`).

---

## 3. Workspace layout

```
crates/
  lens-client/     # existing — typed omnigent client
  lens-core/       # NEW — gpui-free; depends on lens-client
    src/
      domain/      # §2: SessionState, Item/ItemKind, BlockContext, ids, Usage/Cost, StreamScratch
      reduce/      # §4: reduce(), scratch accumulation, folds, transforms (§4.3)
      persist/     # §6: SessionPersistence trait → ControlStore (lens.db) +
                   #     per-session TranscriptStore (rusqlite/WAL) + schema
      actor/       # §8/§7: ActiveSession, SessionCommand, command semantics
      lib.rs       # StreamUpdate / SessionCommand seam types re-exported
  lens-store/      # NEW — gpui; depends on gpui + lens-core
    src/
      lib.rs       # SessionStore (Entity<SessionState> replica) + cx.spawn drain bridge
  xtask/           # existing
```

`lens-core` has **no gpui dependency** — the framework touch-points are all in
`lens-store`. Branded ids reuse `lens-client`'s (`ConnectionId`, `SessionId`,
…) and add engine-local ones (`ItemId`); a session's engine identity is the
composite `(ConnectionId, SessionId)` (persistence PK), but the *registry* that
holds many of them is out of scope (D2).

---

## 4. Build order (each phase lands independently, green)

Strict dependency order **P0 → P1 → P2 → P3** (the actor write-throughs to
persistence, so persistence precedes the actor).

### P0 — Domain types (`lens-core/domain`, §2)
Branded ids, `SessionState`, `Item` + `ItemKind` enum — the full LOCKED **§2.3**
union: message | function_call | function_call_output | reasoning | native_tool |
compaction | slash_command | terminal_command | **error** | resource_event |
agent_changed. `BlockContext { agent, depth, turn }` — **pure attribution**; the
durable "when" is `Item.created_at: i64` (epoch millis) on the item **envelope**,
stamped from an injected clock at reduce time (§2.3/§2.4, grilling revision — the
old `BlockContext.timestamp: f64` monotonic field is **dropped**: no consumer,
non-round-trippable). `Usage`/`Cost`/`ErrorInfo`/`PresenceViewer`, `StreamScratch`
(§4.2). Pure data + serde (payloads jsonb-mappable per §6.2). No logic.
**`presence` is RAM-only — never a persisted column** (§2.5/§6.2); carry it on
`SessionState` but exclude it from the P2 schema. **Gate:** serde round-trips;
`Item` round-trips through `created_at` (no monotonic value); fmt · clippy · tests.

> **Doc correction (resolved 2026-07-08):** the LOCKED §2.3 `ItemKind` includes
> `Error { source, code, message }` ("persisted error banner") but the §6.2
> schema `kind`-column comment originally omitted `error`. **Now fixed in the
> design §6.2** (the `items.kind` comment lists `error`); the P2 `item_kind`
> enum must include it.

### P1 — Pure reducer + render transforms (`lens-core/reduce`, §4) — *contract-proving*
`reduce(&mut SessionState, &ServerStreamEvent) -> SmallVec<[StreamUpdate; 2]>`:
text accumulation (`OutputTextDelta`→`MessageAcc`→finalized `Message` on
`ResponseCompleted`); tool pairing by `call_id`; reasoning bracketing (open →
delta → synthetic `ReasoningClosed`); `BlockContext` attribution stamped at item
creation; identity/ordering/**dedup by `id`** (persisted items carry no
`sequence_number`; `seq` is an SSE-only overlap hint); session-field folds
(status/usage/todos/model/model_options/reasoning_effort/collaboration_mode/
skills/elicitation/child_session/presence/sandbox_status/terminal_pending/
agent_changed); `AgentChanged` item insertion (synthesize `from` from prior
state); the `SnapshotRestored` fold (bootstrap + reconnect chrome — **scalar
restore only, no transcript side-effects**). Plus §4.3 render transforms
(`hide_reasoning`, `flatten_sub_agents`, `merge_text_for_display`,
`only_agent`/`by_depth`, `with_agent_changed_markers`) as pure fns over
`&[Item]`.

**P1 also owns normalization** (flagged from Plan 4, memory
`plan4-pre-consumer-hardening`): the two status vocabularies (`SessionStatusValue`
6-val live vs `SessionStatus` 3-val snapshot) and the two usage representations
are normalized here into `SessionState`'s canonical fields.

No threads, no gpui, no SQLite. `reduce()` stays **pure** (§4.1 "does no I/O"), so
the reduce-time `Item.created_at` epoch (§2.3, grilling revision — replaces the
dropped `BlockContext.timestamp`) comes from an **injected clock** (a `Clock`
seam / passed-in instant), never a direct wall-clock read — otherwise replay is
non-deterministic.

**TDD against the golden SSE corpus** (`docs/spikes/captures/2026-06-26-sse/` +
`…-live-recapture/`) for the wire-event arms. The **reconnect/bootstrap arms are
crate-synthetic** (`Reconnecting`/`Reconnected{gap}`/`SnapshotRestored` are
lens-client §7 synthetics, NOT in the wire corpus) → tested with **hand-authored
synthetic-event fixtures**. Required cases: gap-cleared `StreamScratch` on
`Reconnected{gap != Some(0)}` (§4.2); `SnapshotRestored` scalar-only fold with
**no transcript side-effects** (§4.1); replayed `GET /items` dedup; duplicate-`id`
dedup against hydrated items. **Gate:** reducer deterministic/replayable under the
fixed clock (replay twice → identical `SessionState`); fmt · clippy · tests.
**The reducer-emitted `StreamUpdate` semantic deltas are drafted at this phase's
exit (D6); ratified at the P3 skeleton.**

### P2 — Persistence (`lens-core/persist`, §6) — *two-tier (grilling revision)*
`SessionPersistence` trait, **factored into two roles** over the §6.2 two-file
schema (§6.1):

- **`ControlStore`** (one `lens.db`): `connections`/`sessions`/`cost_samples`/
  `meta`. Session-field folds write here through a **single serialized
  control-plane writer** (low volume). `error` is in the `sessions.status` /
  transcript `item_kind` vocabularies as applicable (P0 doc correction, now fixed
  in §6.2).
- **`TranscriptStore`** (one SQLite file per `(connection_id, session_id)`,
  path `transcripts/<connection_id>/<conv_id>.db`): only that session's `items`.
  Write-through upsert **by `item_id`** into the file the **actor owns** (its own
  WAL write connection → zero cross-actor contention). The file's own `meta`
  carries `schema_version` + `(connection_id, session_id)` (self-describing).

**Impl:** `rusqlite` (blocking, matches D5) + WAL both tiers. `meta.schema_version`
migration gate in **each** file (unknown future version → read-only-degraded,
never corrupted). In-progress `StreamScratch` and `presence` are RAM-only, never
persisted (§2.5/§4.2). P2 exposes **load / upsert / reconcile primitives** on
`TranscriptStore` (reconcile keyed by item `id`, since disk may lag the server)
plus control-plane load/upsert; the **wake wiring that calls them is P3** (a
lifecycle/actor concern, §6.3). Retention/pruning/tombstone become **file ops**
on the transcript file (§15 open q; still deferred). **Gate:** temp-db
write-through + reconcile-primitive tests **on both stores**; per-file
schema_version gating test; open/close transcript file across the Active
lifecycle; fmt · clippy.

### P3 — Actor + store + commands (`lens-core/actor` + `lens-store`, §8/§7/§13.1)

> **Sliced (§2.3 D22).** P3 executed as **P3-1** (actor foundation — DONE),
> **P3-2** (command semantics D16/D18 — DONE), and **P3-3**, itself split into
> **P3-3a lifecycle core** (catch-up + transport-only reader D19, working-set actor
> D20, sleep/wake D21, D15 rider) and **P3-3b recovery semantics** (held-bubble
> resume, `SendLost`, cmd-path escalation, drain policy). The (a)/(c) prose below
> predates the D19/D20 refinements — where it says "byte-windowed `items` tail" or
> "disk-paint the byte-windowed tail," read **§2.3 D20** (the actor holds a *small
> pruned working set*, disk is canonical, the render window is a deferred replica
> concern) and **D19** (reconcile = bounded wake-load + unbounded forward catch-up;
> the reader no longer replays items).

**Task 0 = large-transcript latency spike (D12), sequenced FIRST.** Throwaway
harness vs a synthetic ~500 MiB / ~100k-item transcript file: page-load, cold
hydrate, and `reconcile`-at-scale + scope. Runs before the wake wiring because it
fixes the `reconcile` contract (bounded tail vs full history) that (c) depends on.

**Task 1 = walking skeleton (D7), ratifies D8/D9.** One fake event → `reduce` →
value-carrying `StreamUpdate` (D8) over a bounded channel → `SessionStore` replica
applies (`apply` = pure copy-assignment) → `cx.notify` → observed on the
foreground; plus a `Rebased` baseline (D9) at attach. Proves the
blocking-thread↔`cx.spawn` handoff + backpressure shape and ratifies the
value-carrying-delta + `Arc<Item>` representation end-to-end.

Then, in three parts:

**(a) Actor run-loop.** `ActiveSession` on its OS thread **waits on two inputs via
crossbeam `Select` (D13)** — the `EventStream` receiver (now
`crossbeam_channel::Receiver<ServerStreamEvent>`, exposed by `lens-client`) and
the `SessionCommand` receiver — block-until-either-ready, so commands are serviced
even during heartbeat-only idle, with no busy-poll and no forwarder thread. Per
event: reduce → persist write-through (byte-windowed `items` tail, D11) → emit
`StreamUpdate` (`Detailed` mode, focused) **or** `SummaryUpdate` (`Summary` mode,
background — D10). `SessionStore` replica applies (`StreamUpdate::apply` = pure
copy-assignment, D8 — no parse/reduce/IO on the foreground) with `cx.observe`
granularity; bounded-channel backpressure + delta coalescing (greedy `try_recv`
drain of all pending updates before one `cx.notify`). **Promote/demote** (D10):
on focus emit `Rebased` + switch to `Detailed`; on blur drop items + revert to
`Summary`. The trigger *policy* (focus, active-set) is §9.

**(b) Command semantics (§7).** `SessionCommand` inbound — **send** (optimistic
actor-owned `pending_user`, FIFO reconcile on `session.input.consumed`, rollback
on POST failure), interrupt, compact, approve, stop_session, fork, switch_agent;
bootstrap + reconnect wiring (the actor consumes the crate-synthetic
`Reconnecting`/`Reconnected`/`SnapshotRestored`/`Disconnected` lifecycle from
`lens-client` §7); §13.1 `ClientError`→app-state mapping.

> **Optimistic-send × reconnect reconcile (found in review — the one collision
> §7's FIFO leaves open).** §7's FIFO reconcile assumes client posts and
> `session.input.consumed` events stay ordered — but a reconnect **gap** can drop
> the `consumed` event, which otherwise duplicates the optimistic bubble (the
> replayed `GET /items` re-adds the message while the un-reconciled `pending_user`
> entry still shows) or orphans it. Rules:
> 1. **Do not clear `pending_user` on `Reconnected { gap != Some(0) }`** — unlike
>    `StreamScratch` (§4.2). It is user intent; resolve it against the reconnect
>    replay, never by dropping it.
> 2. **Reconcile `pending_user` against the reconnect replay, not only the live
>    `consumed` stream, and split by session type:**
>    - **Native-terminal** (claude-native / codex-native): the message is *not*
>      persisted at POST time, so trust the `SnapshotRestored` snapshot's
>      `pending_inputs` (`{pending_id, content}[]`) — present ⇒ still pending
>      (keep/re-hydrate); absent from both `pending_inputs` and the replayed items
>      ⇒ lost (re-send or drop-with-marker). Live clears use the consumed event's
>      `cleared_pending_id` **by id**.
>    - **Non-native**: the message *is* persisted at POST time, so match ordered
>      un-reconciled `pending_user` (role=user, `created_by == me`) against the
>      ordered trailing user-message items in the replay not already canonical;
>      content validates. Drop the optimistic bubble on match (kills the dup).
> 3. **No client-supplied correlation id exists on the message input** (only the
>    elicitation path has one) — do not design assuming one. Reconcile-repopulating
>    `pending_user` from `pending_inputs` is **not** a §4.1 transcript side-effect
>    (it pushes no canonical `Item`), but call it out so the `SnapshotRestored`
>    "scalar-only" wording isn't read as forbidding it.
>
> **P3 verification item (not spec-blocking):** confirm against a live native
> session whether `POST /events` **returns** the `pending_id`, letting Lens stamp
> the optimistic bubble at POST time for fully by-id native reconcile. Add to the
> P3 gate.

**(c) Session lifecycle mechanics (§3).** The engine owns the *mechanics*: an
`is_quiesced` predicate (strict — no open scratch / pending tools / pending user /
unreconciled reconnect / non-`idle` status / live terminal, §3.2), **sleep**
(flush persistence → best-effort `stop_session` → stop actor → drop heavy RAM,
§3.4/§6.3), **wake** (disk-paint the **byte-windowed** tail, D11 → fresh stream
bootstrap → **tail-scoped** reconcile-by-id per the D12 spike's `reconcile`
contract, not a full-history diff, calling the P2 primitives), and
`SessionLifecycle = Active | Slept | Deleted` +
tombstone state (§3.1). The **trigger/scheduler** that *fires* auto-sleep (the
~10min timer, the active-set) is the **§9 seam**, deferred — the engine exposes
`is_quiesced()` + `sleep()`/`wake()` for that scheduler to call.

**Gate:** scripted-mock actor tests (reuse the `lens-client` `Reopen`-style seam
for deterministic reconnect/bootstrap without a server) + the walking-skeleton
integration; a **command-interleaving matrix** — send/interrupt/stop exercised
while the stream is idle, running, and reconnecting; sleep/wake round-trip
(quiesce → sleep → flush asserted → wake → reconcile); **no foreground blocking**
(all I/O off-thread — AGENTS.md MANDATORY); fmt · clippy · tests.

**Batched live-verify run (D16/D17, not spec-blocking).** One gated live session
against a pinned 0.4.0 server (`installing-omnigent-from-source`) confirms the
three riders together — cheaper than scattering them, same real session:
1. **D16:** `POST /events` populates `SendEventAck.pending_id` (native `message`)
   and `.item_id` (non-native `message`) at runtime → picks the primary reconcile
   path (id-match vs content-match fallback).
2. **D17:** post-`stop_session` server effects on an already-idle session are
   durably re-fetchable on wake (`GET /items`/snapshot) → validates flush-first.
3. The pre-existing §4 P3(b) check (does native `POST /events` return `pending_id`
   for at-POST-time by-id native reconcile) — same observation as (1).

---

## 5. Local verification

- **Per phase:** `cargo test -p lens-core` (P0–P2) / `-p lens-store` (P3),
  `cargo clippy --all-targets`, `cargo fmt --check`. `generated.rs` untouched.
- **P1 corpus:** the reducer replays the captured `.stream.sse` corpora; add a
  determinism test (replay twice → identical `SessionState`).
- **P3 skeleton:** a gated integration example/harness exercising the full
  off-thread→foreground path.
- **Live:** a gated `--features live-tests` run driving one real session through
  the engine end-to-end is deferred to after P3 lands (needs a running pinned
  0.4.0 server; `installing-omnigent-from-source` skill). Unit + corpus coverage
  is the phase gate; live is confirmation.
- **Review:** cross-family diversity review at each phase seam (P1 reducer and P3
  actor are load-bearing); consolidate where cheap (`review-spend-policy`). P1 and
  P3 warrant an Opus synthesis pass given they set the seam contracts.
- **Perf:** benchmark-or-it's-not-done on the hot paths (AGENTS.md) — reducer
  throughput (corpus/frame budget) and `StreamUpdate::apply` cost. The 120fps /
  90fps-regression contract applies to the foreground apply path.

---

## 6. Seam contracts (what this engine exposes)

- **Up to the UI (§13.2):** `SessionStore` read/observe access — `items` (through
  §4.3 transforms), status/usage/model/todos/presence/cost/sandbox scalars,
  `pending_elicitations`. Surfaces never receive `&mut SessionState`.
- **Down to `lens-client`:** the actor consumes `ServerStreamEvent` (incl. the
  synthetic lifecycle) and issues `SessionEventInput` commands + the REST
  fork/switch-agent endpoints.
- **Up to the UI, coarse (D10):** a **`SummaryUpdate`** — a type *distinct* from
  `StreamUpdate`, carrying **only coarse card-summary fields** (status/title/
  last_total_tokens/host_id/needs-attention/sub-agent-active). **Two producers:**
  (i) the **actor** emits it for a background-warm Active session in `Summary` mode
  (within-turn ms–s cadence, D10) instead of the full delta stream; (ii) the §10
  list-poll applies it to a **Slept** session's store **without an actor and
  without touching the reducer** (not a backdoor reduce path). `apply` is
  copy-assignment of coarse scalars only. A **focused** Active session is fed
  `Detailed` `StreamUpdate`s, not `SummaryUpdate`, and its live stream is
  authoritative for any field (§10). The allowed-field set is finalized by the
  §9/§10 spec; the type + invariant are committed here.
- **To the future §9 registry (named, not specced):** a `SessionHandle`-shaped
  handle `{ SessionStore replica, Option<ActiveSessionHandle> }`. The registry
  owns the **focus/active-set policy** that drives the actor's promote/demote
  (D10) and the `Detailed`↔`Summary` mode switch; this spec exposes the actor
  capability + the `Rebased` promote primitive, not the trigger.

---

## 7. Deferred / recorded (clean seams)

- **§9 registry / §10 poll** → immediate next spec (seam named in §6 above).
- **§11 Bridge, §12.3 Concierge** → separate features.
- **WS terminal byte stream** (§13.2) — direct from the typed-client WS client,
  not through the reducer; and `lens-client` has no `terminal.rs`/`tungstenite`
  yet (known build-order deferral). This engine carries only the
  `terminal.activity`/`terminal_pending` *notifications*.
- **Presence broadcast** (§12.1) — receive-only in v1.
- **`client_os_*` inbound bidirectional tools** (§7) — reserved extension.
- **Disk retention/pruning policy**, **auto-sleep threshold**, **poll cadence**
  (§15 open questions) — tune in the verification pass; the schema supports either.
- **`lens-client` residuals the reducer will eventually want** (memory
  `plan4-pre-consumer-hardening`): `last_task_error` type-ambiguity, minimal
  wrappers to grow with golden captures — resolve as the reducer consumes them.

### 7.1 Design-doc amendments required (from D8–D14)

These edit LOCKED sections of `app-architecture-and-state-model.md`; do them
deliberately when the P3 plan is written so the design source stays the truth.

- **§8 (D14):** rewrite the two-copy rationale — "off-thread to decouple N warm
  *background* streams from the gpui foreground executor," not "reduce is
  expensive" (reduce = 1.36µs). Also mirror in memory
  `state-model-single-writer-decision`.
- **§9 (D10):** the replica is full-fidelity **only when focused**; background-warm
  Active sessions get a coarse `SummaryUpdate` feed from the actor, not a full
  `StreamUpdate` replica. Duplication bounded by focus count, not warm count. Add
  the actor `Detailed | Summary` dual-mode + promote/demote.
- **§8 replica contract (D8/D9):** `StreamUpdate` is **value-carrying**;
  `apply` = pure copy-assignment; add the one-shot `Rebased(Box<SessionState>)`
  baseline at attach; `items: Vec<Arc<Item>>`.
- **§6/§15 (D11):** transcript retention is a **byte-windowed** resident tail +
  paged `TranscriptStore` load, a designed seam — not solely a deferred tunable.
- **§13.1 (D18):** restructure the single error/lifecycle table into **two
  path-keyed tables** — Table A (stream terminal `Disconnected{reason}` → actor
  park/stop lifecycle) and Table B (`ClientError` on command/REST → command
  outcome). Add the missing `Server{status,body}` (4xx/5xx split) and `ThreadSpawn`
  (fatal stream-open) rows; drop or mark-forward-looking the phantom `Ws` row.

**From D19–D22 (§2.3, P3-3a — 2026-07-10):**

- **typed-client §7 + §7 Bootstrap (D19):** the reader is **transport-only** —
  reconnect/bootstrap no longer fetch/replay `GET /items`; delete the item-replay
  steps + the three-bucket "bucket A" replay; amend the "reader owns item recovery
  end-to-end" decision → "the stateful consumer (actor) owns durable item recovery
  via forward catch-up." `Reopen` shrinks 3→2 methods.
- **app-arch §4.1 (D19/D20):** the `SnapshotRestored` fold stays scalar-only, but
  the accompanying "replayed `GET /items` history" is gone — the actor catches up
  itself; the reducer mutates a **small pruned working set**, not a full resident
  `items`; far-back re-fires are disk upserts-by-id.
- **app-arch §6.3/§15 + §8 (D20, supersedes the 2026-07-09 D11/D8/D9 blocks):** the
  ~8 MB byte-window is a **replica/render** concern, **not** an actor tail; the actor
  holds a small pruned working set and **disk is canonical** for finalized items;
  fleet RAM is per-actor-small + one focused replica, not 30 × 8 MB. `Rebased` drops
  its item payload (baseline items from disk on promote).
- **app-arch §3 (D21):** sleep = `SessionCommand::Sleep` (in-loop re-check), wake =
  respawn-from-disk, external §9 trigger; `is_quiesced` conjuncts made explicit.

### 7.2 New dependencies / cross-crate touches (from D13)

- **`crossbeam-channel`** — new workspace dependency (actor `Select`).
- **`lens-client` reader channel swap** (`std::sync::mpsc::sync_channel →
  crossbeam_channel::bounded`) + a `receiver()` accessor on `EventStream`. First
  P3 modification of hardened/feature-complete `lens-client`: re-verify `stop()` +
  drop-unblocks-blocked-sender under crossbeam; **MANDATORY cross-family review**
  of the diff (temporal/stateful).

---

## 8. Reversibility

- The two-crate split is cheap to collapse or re-cut — `lens-store` is thin.
- `StreamUpdate`/`SessionCommand` are the only cross-layer contracts *this spec
  owns* (`SummaryUpdate` belongs to the §9/§10 spec): `StreamUpdate` drafted at P1
  / ratified at the P3 skeleton (D6), `SessionCommand` at P3 — versionable if they
  must change.
- SQLite is behind `SessionPersistence` — a backing-store swap is a trait impl,
  not a rewrite (D4).
- Every phase lands green and independently, so a phase can be revised without
  unwinding its predecessors (the `lens-client` per-plan precedent).
