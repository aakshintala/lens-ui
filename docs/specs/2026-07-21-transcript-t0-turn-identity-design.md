# T-0 — Authoritative turn identity (design)

**Date:** 2026-07-21 (rev 2 — cross-family reviewed + live-0.5.1-verified)
**Status:** Design — ready for implementation plan.
**Owner:** Lens design effort
**Type:** Implementation slice (build), transcript workstream **T-0** of T-0..T-7 —
the prerequisite surfaced by the T-1 cross-family review.
**Branch:** `lens-transript` (proceed now; see §9 merge coordination).

Makes the server **`response_id`** the single authoritative turn signal for the
transcript — on every `Item` (per-item identity) and as the session's live-turn
signal (liveness). Both are discarded today. This unblocks **T-1** (ViewBlock
projection), which groups and gates entirely on `response_id` with no heuristic.

Grounding: `docs/specs/2026-07-21-transcript-t1-viewblock-projection-design.md` §0/§6.2/§8;
live evidence memory [[t0-response-id-live-sourcing]] + captures
`docs/spikes/captures/2026-07-21-t0-verify/`.

---

## 0. What review + live verification changed (rev 2)

Rev 1 was cross-family reviewed (Grok 4.5 + GPT-5.6 via codex). Both **independently**
found rev 1's data-sourcing foundation false, and a **live 0.5.1 drive** (two real
claude-sdk turns, 2026-07-21) then corrected even the reviewers' proposed fix. The
empirical truth ([[t0-response-id-live-sourcing]]):

| Datum | `GET /items` (catch-up) | snapshot `?include_items` | live SSE |
|---|:--:|:--:|:--:|
| `response_id` (identity) | ✅ `turn_`/`resp_`/`conv_` | ✅ | ✅ `output_item.done.item.response_id` + `response.in_progress.response.id` |
| `created_at` (timestamp) | ❌ **null** | ✅ epoch **seconds** | ❌ absent |
| liveness (`active_response`) | — | ❌ `active_response_id` null **even mid-turn** | ✅ `response.in_progress.response.id` |
| `session.status.response_id` | — | — | ❌ **null** (running & idle) |

Three conclusions reshape the slice:

1. **The blocker is `lens-client`, not `lens-core`.** `lens_client::stream::Item`
   (`stream/event.rs:637-679`) carries neither `response_id` nor `created_at` on any
   variant; `Item::from_value` drops them; `wire_to_domain_item` (`runloop.rs:221`)
   can't map what it never receives. `response.in_progress`'s id is parsed as a **unit**
   `ResponseEvent::InProgress` — also dropped. **T-0 requires a lens-client widening
   step** (§3). Rev 1's "depends on nothing new" was false.
2. **Live liveness source is `response.in_progress.response.id` — only.** Both
   `session.status.response_id` and snapshot `active_response_id` are **null for
   in-process harnesses** (live-verified null even mid-turn). Rev 1 keyed on `Status`;
   the reviewers proposed snapshot `active_response_id`; **both are wrong in-process.**
3. **`created_at` is unobtainable on the catch-up path** (null on `/items`, absent from
   the live stream; present only on snapshot-embedded items, epoch **seconds**). Nothing
   in T-0/T-1 needs it (ordering is by store ordinal; only T-6's duration chip wants it).
   So rev 1's "map real `created_at` on catch-up" goal is **impossible and descopes** to
   a snapshot-sourced T-6 concern (§7).

---

## 1. Scope & boundaries

**T-0 owns** making `response_id` authoritative, across two crates:

1. **lens-client widening** (§3) — retain `response_id` on `stream::Item` + `from_value`
   (serves catch-up `/items` **and** live `output_item.done`); carry `response.in_progress`'s
   `response.id` on `ResponseEvent::InProgress`.
2. **Catch-up mapping** (§4.1) — map wire `response_id` onto domain `Item`s (stop
   hardcoding `turn:0`).
3. **Live identity + liveness** (§4.2) — stamp each item's own wire `response_id`; set
   `SessionState.active_response` from `response.in_progress`, clear on terminal `response.*`.
4. **Per-item identity type** (§2.1) — `response_id` on `BlockContext` (replacing the
   write-only `turn: u32`).
5. **Liveness exposure** (§4.3) — expose the active `response_id` via a feed delta.
6. **Persistence** (§5) — additive migration to persist/restore `response_id` + promote
   it in reconcile.

**T-0 does NOT own:**

| Concern | Why not T-0 | Where |
|---|---|---|
| `ViewBlock` projection / grouping / liveness gate | T-0 supplies the signal; T-1 consumes it | **T-1** |
| Real `created_at` / durations | Unobtainable on catch-up; needs a snapshot pass | **T-6** (§7) |
| `stream.turn` Ready-counter bump bug | Card/summary path, orthogonal | Board agent — `docs/handoffs/2026-07-21-turn-counter-non-completed-terminal-bug.md` |
| `response.completed.response.usage` retention | Per-turn chip data | T-6 |
| Focused **transcript** replica consumption of the liveness delta | The only current detailed-feed replica is `CardModel` (summary-only); the transcript replica is T-2's RowSource | **T-2** (§4.3) |
| Any rendering / gpui | T-0 is data | T-2+ |

---

## 2. Types

`ResponseId` **already exists** — `branded_id!(ItemId, CallId, ResponseId, …)` in
`domain/ids.rs` (transparent `String` newtype, full serde). T-0 adds no new type.

### 2.1 `BlockContext`: replace `turn` with `response_id`

```rust
// crates/lens-core/src/domain/item.rs
pub struct BlockContext {
    pub agent: Option<String>,
    pub depth: u32,
    pub response_id: Option<ResponseId>,   // was: pub turn: u32
}
```

**Replace, not add** — CONFIRMED by both reviewers: `ctx.turn` is write-only (stamped
`reduce/items.rs:17`, persisted `persist/transcript.rs`, test-asserted; transforms read
only `ctx.agent`/`ctx.depth`, `transforms.rs:20-48`). No behavioral reader. `Option`
because orphans, pre-`response_id` rows, or synthesized items may lack one — T-1's
sibling classification already treats "no agent `response_id`" as first-class.

> `state.stream.turn` (the `StreamState` **counter**, `reduce/mod.rs:136`) is a distinct
> field, feeds only the card Ready policy (`summary.rs:71`), and stays. Its non-`Completed`
> bump bug is the separate handoff — not T-0.

### 2.2 `SessionState`: the active-response scalar

```rust
// crates/lens-core/src/domain/session.rs
pub active_response: Option<ResponseId>,   // set from response.in_progress; None = idle/unknown
```

Source for the liveness signal (§4.3). **Not** the primary per-item stamp source — that
is each item's own wire `response_id` (§4.2, review finding #4).

### 2.3 `StreamUpdate`: the liveness delta

```rust
// crates/lens-core/src/reduce/update.rs
ActiveResponseChanged(Option<ResponseId>),
```

Dedicated value-carrying delta (matches `StatusChanged`/`ModelChanged`). **Budget note:**
`Updates` is `SmallVec<[StreamUpdate; 2]>` (`update.rs:13`); the `response.in_progress`
path must stay ≤2 updates or accept a documented spill — the plan pins this with a bench
check (codex finding).

---

## 3. lens-client widening (prerequisite step, same slice)

The fields are on the wire but the hand-written types drop them. T-0 widens:

1. **`stream::Item`** (`stream/event.rs:637-679`) — add `response_id: Option<ResponseId>`
   (or `String`) to the item variants that carry it (`Message`, `FunctionCall`,
   `FunctionCallOutput`, `Error`, `ResourceEvent`); **`Item::from_value`** reads the
   `response_id` key. This one change serves **both** catch-up (`/items`) and live
   (`output_item.done`), since both decode through `Item`.
2. **`ResponseEvent::InProgress`** (`stream/event.rs:572`, currently unit) — carry the
   `response.id` (the sole working liveness source). Minimal: a `{ response_id }` field.
3. **Not modeled:** `created_at` (unobtainable where it matters — §7); `session.status.response_id`
   (null in-process — do not wire it as a source).

This aligns with the deferred "lens-client modeling follow-on" (STATUS) and prior
capture-driven widenings (`plan3*`). It is byte-grounded by the 2026-07-21 captures.

---

## 4. Data flow

### 4.1 Catch-up (`/items` → domain `Item`)

`wire_to_domain_item` (`runloop.rs:221`) maps `wire.response_id` → `BlockContext.response_id`
(was `turn: 0`). `created_at` **stays clock/None** — the wire has none on this path (§7).
Absent/empty wire `response_id` → `None` (never fabricated).

### 4.2 Live (`output_item.done` / `response.in_progress` → domain)

- On `response.in_progress`, set `SessionState.active_response = Some(id)`; emit
  `ActiveResponseChanged(Some)`.
- On terminal `response.completed/failed/incomplete/cancelled`, clear
  `active_response = None`; emit `ActiveResponseChanged(None)`.
- When an item is created from `output_item.done` (`reduce/items.rs`), stamp
  **its own wire `response_id`** onto `BlockContext.response_id`. Reducer-**synthesized**
  items (finalized streaming accumulators, which have no wire item) fall back to
  `active_response`. (Review finding #4: prefer per-item wire id over the session scalar
  to avoid mis-stamping under null/reorder.)

### 4.3 Liveness out (feed → consumer)

`ActiveResponseChanged` deposits `active_response` into the foreground replica via the
existing value-carrying delta pattern. **Success criterion is "the actor feed exposes
the delta"** — the *transcript* replica that reads it is T-2's RowSource (not yet built);
the only current detailed-feed replica is `CardModel` (summary-only, `card/model.rs:163`),
whose exhaustive match T-0 must extend with a no-op/ignore arm so it compiles (codex
finding). Replica *consumption* for the projector is T-2.

**Ordering:** reduction precedes the terminal-prefix commit; a greedy-batch
`Active(A) → item(A) → Active(None)` may expose only `None`, which correctly presents the
committed item as settled (codex-verified sound). Pin with a batch regression test.

### 4.4 Reconnect-mid-turn degrade (documented, not solved)

For in-process harnesses no REST field carries the active response on reconnect (snapshot
`active_response_id` is null). T-0 accepts the degrade: `active_response = None` until the
next `response.in_progress`, so T-1 folds the live section until the stream re-establishes
it. A status-plus-last-item-`response_id` inference is a possible later refinement, not T-0.

---

## 5. Persistence

Additive, per prior art (`migrate_transcript_columns`, `transcript.rs:40-62` — `ALTER TABLE`
adds, not a `SCHEMA_VERSION` bump; both reviewers):

- Add nullable `response_id TEXT`; **retain the legacy `turn` column** (dropping it breaks
  the read-only-degrade path — older binaries still `SELECT turn`, `db.rs:27-35`).
- Write `ctx.response_id` on upsert; `row_to_item` (`map.rs:168-184`) reads it; old rows →
  `None` until catch-up backfill.
- **Promote authoritative metadata in reconcile** (codex finding): the reconcile branches
  (`transcript.rs:295-321`) currently update id/kind/payload but not `ctx`; T-0 must update
  `response_id` there, so a provisional live row stamped `None` is corrected when
  authoritative `/items` data arrives.

---

## 6. Turn semantics that fall out

- **Per-turn distinct `response_id`** — live-confirmed (turn-1 `resp_00b52ad7` ≠ turn-2
  `resp_bcb93365`). Each response is its own section in T-1, no counter.
- **Failed/incomplete/cancelled** — a retry gets a new `response_id` — **live-confirmed**
  for the cancelled case (2026-07-21, `captures/2026-07-21-t0-verify/interrupt-then-retry.stream.sse`):
  interrupting turn A (`resp_0099878e`) emitted `response.cancelled` carrying that id, then
  the retry (turn B) started a **new** `response.in_progress` = `resp_37ba30e3` with
  `previous_response_id: null` (independent response, not a linked continuation). Each
  `response.in_progress` allocates a fresh id per turn (scaffold: "allocates a `response_id`
  and runs `run_turn`"), so failed/incomplete generalize. **Bonus:** the terminal
  `response.cancelled`/`completed` event carries the ending response's `id`, so T-0's
  clear-on-terminal (§4.2) can identify *which* response ended.
- **Wake / disk paint** — `response_id` on every persisted item; `active_response = None`
  folds all sections.
- **User `turn_` vs agent `resp_` namespaces** — irrelevant to grouping (user messages are
  ordinal siblings, T-1 §6.2). T-0 maps whatever the wire supplies.

---

## 7. `created_at` — descoped to T-6 (evidence-forced)

Live 0.5.1: `created_at` is **null on `GET /items`** and **absent from the live stream**;
it exists **only** on snapshot-embedded items (epoch **seconds**). Therefore:

- T-0 does **not** attempt real `created_at` on catch-up or live — it is not on those wires.
  Catch-up keeps the clock stamp (or `None`); this is the best available and nothing in
  T-0/T-1 depends on it (ordering is by store ordinal, `transcript.rs:244`; the one
  catch-up timestamp consumer ignores the value, `reconcile.rs:60-64`).
- Real per-item timestamps require a **snapshot-embedded-items pass** with **seconds→millis**
  normalization (domain `Item.created_at` is millis, `item.rs:42`; wire is 10-digit seconds).
  That is a **T-6** prerequisite for the duration chip, recorded here so T-6 doesn't
  rediscover it.

---

## 8. Testing strategy

- **lens-client:** byte tests that `Item::from_value` retains `response_id` (message,
  function_call, function_call_output) from the 2026-07-21 captures; `ResponseEvent::InProgress`
  carries `response.id`.
- **Catch-up mapping (lens-core):** wire row with `response_id` → `Item` carries it (not
  `turn:0`); absent → `None`.
- **Live stamping:** `output_item.done` stamps the item's own `response_id`;
  `response.in_progress` sets `active_response` + emits `ActiveResponseChanged(Some)`;
  terminal `response.*` clears it + emits `(None)`; synthesized finalized accumulators fall
  back to `active_response`.
- **Delta/replica:** `ActiveResponseChanged` mirror-parity; greedy-batch ordering invariant
  (§4.3); `CardModel` ignore-arm compiles; `SmallVec` budget bench.
- **Persistence:** `response_id` round-trip; reconcile promotes `response_id` on a
  provisional→authoritative fold; additive migration + old-row `None`.
- `xtask gate` green (fmt/clippy/test, zero warnings). Add lens-client to the gate `-p` set
  if not already present.

---

## 9. Merge coordination

T-0 edits actor+reduce core (`runloop.rs`, `reduce/mod.rs`, `reduce/items.rs`,
`reduce/update.rs`, `domain/item.rs`, `domain/session.rs`, `persist/transcript.rs`,
`persist/map.rs`) plus `lens-client` (`stream/event.rs`). The `terminal-ws` branch is
concurrently rewriting the actor/engine → `runloop.rs`/`reduce/mod.rs` are a textual merge
surface. **Decision:** proceed on `lens-transript` now; whichever merges second reconciles.
Changes are logically independent (T-0 = identity; terminal = engine). The `stream.turn`
handoff similarly edits `reduce/mod.rs` arms — same low-risk textual overlap.

---

## 10. Success criteria

- **lens-client** retains `response_id` on `stream::Item`/`from_value` and on
  `ResponseEvent::InProgress` (byte-tested vs 2026-07-21 captures).
- `BlockContext.response_id: Option<ResponseId>` replaces `turn: u32`; `SessionState.active_response`
  + `StreamUpdate::ActiveResponseChanged` land.
- Catch-up maps wire `response_id`; live stamps each item's own `response_id`;
  `active_response` sourced from `response.in_progress`, cleared on terminal `response.*`;
  the actor feed exposes the delta (transcript replica consumption = T-2).
- `response_id` persists, restores, and is promoted in reconcile; migration is additive.
- `created_at` is explicitly out (§7); reconnect degrade documented (§4.4); failed-retry
  new-id flagged as a build-time live-rider check (§6).
- All §8 tests pass; `xtask gate` green. T-1 is buildable + testable end-to-end against a
  real `active_response` signal — no deferred sourcing dependency.
