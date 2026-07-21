# T-0 — Authoritative turn identity (design)

**Date:** 2026-07-21
**Status:** Design — ready for implementation plan.
**Owner:** Lens design effort
**Type:** Implementation slice (build), transcript workstream **T-0** of T-0..T-7 —
the prerequisite surfaced by the T-1 cross-family review.
**Branch:** `lens-transript` (proceed now; see §8 merge coordination).

Makes the server **`response_id`** the single authoritative turn signal for the
transcript — on every `Item` (per-item identity) and as the session's live-turn
signal (liveness). Today both are discarded. This unblocks **T-1** (ViewBlock
projection), which groups and gates entirely on `response_id` with no heuristic.

Grounding: `docs/specs/2026-07-21-transcript-t1-viewblock-projection-design.md` §0, §6.2, §8.

---

## 0. Why this slice exists

The T-1 review (Grok 4.5 + GPT-5.6) proved that `ctx.turn`/`scratch.turn` are
unusable as a turn signal for disk-sourced history — the transcript's steady state:

- `wire_to_domain_item` (`crates/lens-core/src/actor/runloop.rs:221-233`) stamps every
  catch-up `/items` row `turn: 0` + fetch-time `created_at`. Disk history therefore has
  **no turn boundaries and no real timestamps**.
- `scratch.turn` (the live `state.stream.turn` counter) is RAM-only, defaults to 0, is
  never restored on wake, and never bumps on failed/incomplete/cancelled responses.

The fix is already on the wire and currently **discarded**, not a contract limit:

- `/items`' `ConversationItem` carries a **required** `response_id` and `created_at`
  (`vendor/omnigent-0.5.1/openapi.json:877-965`; codegen already models them —
  `crates/lens-client/src/generated.rs:1450`).
- The live stream carries the active `response_id` on `SessionEvent::Status`
  (`crates/lens-client/src/stream/event.rs:51-54`).

T-0 stops discarding both and makes `response_id` authoritative end-to-end.

---

## 1. Scope & boundaries

**T-0 owns** making `response_id` authoritative:

1. **Catch-up mapping** — map wire `response_id` + `created_at` onto domain `Item`s
   (stop hardcoding `turn:0` + fetch time).
2. **Live stamping** — stamp the active `response_id` (from `SessionEvent::Status`)
   onto items created during a live turn.
3. **Per-item identity** — carry `response_id` on `BlockContext` (replacing the
   write-only `turn: u32`).
4. **Liveness exposure** — expose the session's active `response_id` through the actor
   feed to the foreground replica, so the projector (T-1) can gate on it.
5. **Persistence** — migrate the transcript store to persist/restore `response_id`.

**T-0 does NOT own:**

| Concern | Why not T-0 | Where |
|---|---|---|
| `ViewBlock` projection / grouping / liveness gate | T-0 supplies the signal; T-1 consumes it | **T-1** |
| The `stream.turn` Ready-counter bump bug (non-`Completed` terminals) | Card/summary Ready-policy path, orthogonal subsystem | Board agent — `docs/handoffs/2026-07-21-turn-counter-non-completed-terminal-bug.md` |
| `response.completed.response.usage` (model/tokens/cost) retention | Per-turn chip data | T-6 |
| Any rendering / gpui | T-0 is lens-core data | T-2+ |

---

## 2. Types

`ResponseId` **already exists** — `branded_id!(ItemId, CallId, ResponseId, …)` in
`crates/lens-core/src/domain/ids.rs` (transparent `String` newtype, full serde,
`new`/`as_str`/`Display`). T-0 introduces no new type; it adds two fields and one delta.

### 2.1 `BlockContext`: replace `turn` with `response_id`

```rust
// crates/lens-core/src/domain/item.rs
pub struct BlockContext {
    pub agent: Option<String>,
    pub depth: u32,
    pub response_id: Option<ResponseId>,   // was: pub turn: u32
}
```

**Replace, not add.** `ctx.turn` is **write-only today** — stamped in
`reduce/items.rs:17`, persisted in `persist/transcript.rs`, asserted in tests, and
read by **no behavioral consumer** (verified: consumers read `ctx.agent` and
`ctx.depth` only — `reduce/transforms.rs`). T-1 is its first real reader. Leaving a
vestigial `turn: 0` field beside `response_id` is a known trap (catch-up would silently
carry the broken value). `Option` because a pre-`response_id` item, an orphan, or a
malformed wire row may legitimately lack one; T-1's sibling/section classification
already treats "no agent `response_id`" as a first-class case.

> `state.stream.turn` (the `StreamState` **counter**, `reduce/mod.rs:136`) is a
> **different field** and stays. It feeds the card Ready policy (`actor/summary.rs:71`),
> a live-only summary concern; the transcript never reads it. Its non-`Completed`
> bump bug is the separate handoff above — explicitly not T-0.

### 2.2 `SessionState`: the active-response scalar

```rust
// crates/lens-core/src/domain/session.rs
pub struct SessionState {
    // …
    pub active_response: Option<ResponseId>,   // the session's live turn; None = idle
}
```

Held on `SessionState` (not `StreamScratch`) because it is the source for **both**
jobs — the value stamped onto new live items (§3.2) and the liveness signal exposed to
the replica (§3.3). One datum, two uses.

### 2.3 `StreamUpdate`: the liveness delta

```rust
// crates/lens-core/src/reduce/update.rs
ActiveResponseChanged(Option<ResponseId>),
```

A dedicated value-carrying delta, matching the existing pattern (`StatusChanged`,
`ModelChanged`, …): each delta deposits its just-reduced value into the foreground
replica. Dedicated rather than folded into `StatusChanged` because the reducer already
emits `StatusChanged` from multiple sites and the liveness transition is its own
concern; a targeted delta keeps the replica-mirror mapping 1:1 and testable.

---

## 3. Data flow

### 3.1 Catch-up (disk `/items` → domain `Item`)

`wire_to_domain_item` (`actor/runloop.rs:221`) stops hardcoding. It maps:

- `wire.response_id` → `BlockContext.response_id` (was `turn: 0`).
- `wire.created_at` → `Item.created_at` (was `clock.now_millis()`).

Both wire fields are `required` on `ConversationItem` (§0). Where a row's
`response_id` is absent or empty at the wire level, `BlockContext.response_id = None`
(the `Option` absorbs it — never a fabricated id).

**Behavioral-change callout (`created_at`):** disk items currently stamp fetch time;
switching to the wire value changes the timestamp — and therefore any fetch-time-derived
ordering — for existing/replayed sessions. This is the correct fix (real creation time
is what T-6's duration chip and any time display need), but it is a semantic change, not
a pure add. The plan must (a) confirm no current consumer depends on the fetch-time
value as an ordering key, and (b) cover it with a golden/replay test. Ordering itself is
by store ordinal (unchanged); this touches the *timestamp*, not the sequence.

### 3.2 Live (stream → domain `Item`)

`SessionEvent::Status` carries the active `response_id` (`event.rs:51-54`,
`Option<String>`). On Status, the reducer sets `SessionState.active_response` and emits
`ActiveResponseChanged`. When an item is created during the turn (`reduce/items.rs`
`push_item`), it stamps `BlockContext.response_id = active_response.clone()`.

**Nullable-live:** the wire `response_id` on Status is `Option` and can be `null`
(queued / between turns). `null` → `active_response = None` → items created in that
window carry `response_id: None`, and T-1 reads `None` as "idle" for its liveness gate.
This is correct, not a gap.

### 3.3 Liveness out (replica → projector)

`ActiveResponseChanged` mirrors `SessionState.active_response` into the foreground
replica exactly as every other scalar delta does. T-1's `project()` reads it as
`active_response: Option<&ResponseId>` at render time. This is the piece the T-1 spec
(§8) deferred to T-2 as "a T-2 actor-feed sourcing dependency"; T-0 owns it so T-1 is
testable end-to-end on landing (§7).

### 3.4 Persistence

`persist/transcript.rs` currently persists `ctx.turn` as an `INTEGER` column
(`transcript.rs:131,146,176,189`). Migrate to persist/restore `response_id` as `TEXT`:

- Schema bump (add `response_id TEXT`; drop or ignore the `turn` column per the store's
  migration convention — follow the existing schema-version/degrade pattern).
- Write `item.ctx.response_id` on upsert; read it back into `BlockContext.response_id`
  on restore.
- The plan pins whether this is an additive migration + backfill or a version bump with
  a fresh column, consistent with how P2 handled prior schema changes
  (`state-model-p2-persistence` — persisted-enum-serde-`Other` rule and schema-degrade).

---

## 4. Turn semantics that fall out (no extra code)

Keying per-item identity on `response_id` gives the correct behavior the counter never
could, **for free**:

- **Failed / incomplete / cancelled turns** — the server issues a **new `response_id`**
  for the retry, so each attempt is naturally its own turn/section in T-1. There is no
  counter to "bump," and no end-reason special-casing. (The separate `stream.turn`
  counter bug — handoff §8 — is about the *card Ready glance*, not this.)
- **Wake / disk-only paint** — `response_id` is on every persisted item, so turn
  boundaries survive a cold restart with no RAM state. `active_response = None` on a
  disk-only paint folds all sections (T-1 §5.3).
- **User-input items carry a distinct id namespace** (`turn_`/task vs agent `resp_`) —
  irrelevant to grouping: user messages are ordinal-positioned siblings, never inside a
  section (T-1 §6.2). T-0 maps whatever the wire supplies; it does not filter by
  namespace.

---

## 5. Key resolutions

### 5.1 Replace `turn`, don't add
`ctx.turn` has no behavioral reader (§2.1) → replacing is low-blast-radius and avoids a
vestigial broken field. Cost is contained to `reduce/items.rs` (the stamp),
`persist/transcript.rs` (the column), and test fixtures.

### 5.2 One datum for stamp + liveness
The active `response_id` from Status is the same value T-1 stamps onto live items and
gates on. Holding it once on `SessionState` (not duplicating into scratch) keeps a
single source of truth and makes the liveness delta a straight mirror.

### 5.3 Liveness is T-0, not T-2
The signal T-0 exists to make authoritative *is* the liveness signal. Deferring its
exposure (as the T-1 spec did) would ship T-1 with an untestable gate and split one
concern across two slices. T-0 exposes it; the T-1 spec §8 note is superseded.

### 5.4 `stream.turn` counter bug is out
It lives in the card/summary Ready path (a board-surface consumer), is orthogonal to
transcript turn identity, and bundling it drags T-0 into the summary subsystem. Handed
off (§1, §8).

---

## 6. Dependencies

- **Blocks:** T-1 (and transitively T-2..T-7) — all transcript grouping/liveness.
- **Depends on:** nothing new. `ResponseId` exists; the wire fields are modeled
  (`generated.rs`); Status already parses `response_id` (`event.rs`).
- **Contract confidence:** 0.5.1 openapi marks `response_id` + `created_at` `required`;
  codegen reflects it. A cheap live byte-check that `/items` rows actually *populate*
  `response_id` (not merely schema-present) is worthwhile insurance during the build —
  it supersedes the old T-1-spec prereq "re-capture 0.5.1 /items to pin field shape,"
  which the openapi + codegen already answer at the schema level.

---

## 7. Testing strategy

Matches the `reduce/` idiom (inline construction, hand-asserted; existing golden/replay
harness for the disk path).

- **Catch-up mapping (`reduce`/`runloop`):** a wire row with `response_id` + `created_at`
  produces an `Item` carrying both (not `turn:0`/fetch-time); a row with absent/empty
  `response_id` → `BlockContext.response_id = None`.
- **Live stamping (`reduce`):** `Status{response_id: Some}` sets `active_response` and
  emits `ActiveResponseChanged`; a subsequent `push_item` stamps that id;
  `Status{response_id: null}` → `active_response = None` and items stamp `None`.
- **Liveness mirror:** `ActiveResponseChanged` deposits into the foreground replica
  (mirror-parity test alongside the other scalar deltas).
- **Persistence round-trip:** persist an item with `response_id` and restore it;
  schema-degrade/older-DB path per the store convention.
- **Golden/replay (disk path):** a hermetic `/items` catch-up replay asserts real
  `created_at` + `response_id` land (guards the §3.1 behavioral change; watch the
  fresh-disk history/delta split trap — `state-model-golden-replay-gotchas`).
- `xtask gate` green (fmt/clippy/test, zero warnings/dead code).

---

## 8. Merge coordination

T-0 edits the actor+reduce core — `runloop.rs` (`wire_to_domain_item`), `reduce/mod.rs`
(Status arm), `reduce/items.rs` (stamp), `reduce/update.rs` (delta), `domain/item.rs`
(`BlockContext`), `domain/session.rs`, `persist/transcript.rs`. The `terminal-ws` branch
is concurrently rewriting the actor/engine, so `runloop.rs`/`reduce/mod.rs` are a
textual merge surface.

**Decision:** proceed on `lens-transript` now; whichever branch merges second reconciles
the actor/reduce edits. The changes are logically independent (T-0 = per-item
`response_id` identity + liveness; terminal = engine/command-stream), so the collision is
textual, not a design conflict. The `stream.turn` handoff (§5.4) similarly edits
`reduce/mod.rs` arms — same low-risk textual overlap, flagged in that doc.

---

## 9. Success criteria

- `BlockContext` carries `response_id: Option<ResponseId>` (no `turn: u32`);
  `SessionState.active_response` + `StreamUpdate::ActiveResponseChanged` land.
- Catch-up maps wire `response_id` + `created_at` (no `turn:0`/fetch-time); live turns
  stamp the active `response_id`; the active signal reaches the foreground replica.
- `response_id` persists and restores through the transcript store.
- All §7 tests pass, including the golden/replay guard for the `created_at` change;
  `xtask gate` green.
- T-1 can be built and unit/integration-tested end-to-end against a real
  `active_response` signal — no deferred sourcing dependency.
