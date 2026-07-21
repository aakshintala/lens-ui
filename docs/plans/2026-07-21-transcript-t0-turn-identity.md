# T-0 Authoritative Turn Identity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the server `response_id` the single authoritative turn signal on every transcript `Item` (per-item identity) and as the session's live-turn signal (liveness), replacing the write-only `turn` counter.

**Architecture:** The fields already ride the wire but the hand-written `lens-client` types drop them, so T-0 first widens `lens-client` (Task 1), then threads `response_id` through the `lens-core` domain type + catch-up mapping + persistence (Task 2), then adds the live liveness scalar and per-item live stamping with its feed delta (Task 3). `lens-client` carries raw `String` ids; `lens-core` wraps them into the existing `ResponseId` branded newtype at the wire→domain boundary.

**Tech Stack:** Rust, gpui, rusqlite (SQLite), serde, `smallvec`. Tests are `cargo test`; the gate is `cargo run -p xtask -- gate` (fmt + workspace clippy `-D warnings` + tests; there is no `cargo xtask` alias in this repo). Byte-fixtures live in `docs/spikes/captures/2026-07-21-t0-verify/`.

**Design doc (authoritative):** `docs/specs/2026-07-21-transcript-t0-turn-identity-design.md`. Every task cites the design section it implements. Read it before starting.

## Global Constraints

- **UI never panics the process** — errors are modeled values; parsing gaps degrade to `None`, never `unwrap`/`expect` on wire data. (AGENTS.md)
- **Typed end-to-end** — no stringly-typed dispatch across the domain boundary; `response_id` becomes `ResponseId` on entering `lens-core`. (AGENTS.md)
- **`lens-client` never depends on `lens-core`** — the `reuse-only-ids` boundary: `lens-client` carries `Option<String>`; the conversion to `ResponseId` happens in `lens-core`'s `wire_to_domain_item`.
- **Clippy gate is workspace-wide** — `cargo clippy --workspace --all-targets -- -D warnings` MUST be clean before every commit; a red gate on pickup is resolved first.
- **Ground-truth discipline** — assertions about wire shape are byte-grounded against `docs/spikes/captures/2026-07-21-t0-verify/`, never from memory.
- **Additive persistence only** — `ALTER TABLE ... ADD COLUMN`; do NOT bump `SCHEMA_VERSION` (stays `3`, `schema.rs:5`); retain the legacy `turn` column.
- **Merge surface** — `runloop.rs`/`reduce/mod.rs` also change on `terminal-ws`; keep edits minimal and localized (design §9).

---

## File Structure

**Task 1 — lens-client widening**
- Modify: `crates/lens-client/src/stream/event.rs` — add `response_id: Option<String>` to `Item` variants (`event.rs:637-679`); read the key in `Item::from_value` (`event.rs:1058-1133`); change `ResponseEvent::InProgress` (`event.rs:572`) from a unit variant to carry `response_id`.
- Test: byte tests in `crates/lens-client/src/stream/event.rs` (or its existing test module) against the 2026-07-21 captures.

**Task 2 — Domain type + catch-up + persistence**
- Modify: `crates/lens-core/src/domain/item.rs:20` — `BlockContext.turn: u32` → `response_id: Option<ResponseId>`.
- Modify: `crates/lens-core/src/actor/runloop.rs:226-230` — catch-up `wire_to_domain_item` maps wire `response_id` (was `turn: 0`).
- Modify: `crates/lens-core/src/reduce/items.rs:17` — live-stamp site set to `response_id: None` placeholder (real value in Task 3), plus the `:365` test assertion.
- Modify: `crates/lens-core/src/persist/transcript.rs` — migration (`:41-62`), upsert write (`:146`), reconcile promotion (`:295-321`).
- Modify: `crates/lens-core/src/persist/map.rs:168-184` — `row_to_item` reads `response_id`, stops rehydrating `turn`.
- Test: existing `reduce`/`persist` test modules; new round-trip + catch-up mapping tests.

**Task 3 — Live liveness + per-item stamping**
- Modify: `crates/lens-core/src/domain/session.rs` — add `SessionState.active_response: Option<ResponseId>`.
- Modify: `crates/lens-core/src/reduce/update.rs` — add `StreamUpdate::ActiveResponseChanged(Option<ResponseId>)` (`Updates = SmallVec<[StreamUpdate; 2]>` at `:71`).
- Modify: `crates/lens-core/src/reduce/mod.rs` (+ `reduce/items.rs`) — set/clear `active_response`, emit the delta, stamp each live item's own wire `response_id`.
- Modify: `crates/lens-ui/src/card/model.rs:163` — add the ignore arm to `SessionCard::fold_detailed`'s exhaustive `match`.
- Test: live-stamping, set/clear, greedy-batch ordering, `SmallVec` budget bench.

---

## Task 1: lens-client widening

Implements design **§3**. Retain `response_id` on `stream::Item` (serves both catch-up `/items` and live `output_item.done`, since both decode through `Item`), and carry `response.in_progress`'s `response.id` on `ResponseEvent::InProgress`. `lens-client` uses `Option<String>` (no `lens-core` dependency).

**Files:**
- Modify: `crates/lens-client/src/stream/event.rs`
- Test: same file's test module (co-located `#[cfg(test)]`)

**Interfaces:**
- Consumes: the 2026-07-21 byte captures under `docs/spikes/captures/2026-07-21-t0-verify/`.
- Produces:
  - `stream::Item` variants (`Message`, `FunctionCall`, `FunctionCallOutput`, `Error`, `ResourceEvent`) each gain a public field `response_id: Option<String>`. (The `Other` catch-all is unchanged.)
  - An accessor `Item::response_id(&self) -> Option<&str>` returning the variant's id (`None` for `Other`), so `lens-core` reads it without matching every variant.
  - `ResponseEvent::InProgress { response_id: Option<String> }` (was a unit variant).

- [ ] **Step 1: Write the failing byte test for `Item::from_value` retention**

Add to the `event.rs` test module. Use a real captured item object; if a helper to load captures exists in the module follow its pattern, otherwise inline the minimal JSON matching the capture's shape (a `function_call` item carries `"response_id"`). Verify against the actual capture bytes first — do not invent the key casing.

```rust
#[test]
fn from_value_retains_response_id_on_message() {
    let v = serde_json::json!({
        "type": "message",
        "id": "item_abc",
        "response_id": "resp_bcb93365",
        // ...remaining required message fields per the capture...
    });
    let item = Item::from_value(&v).expect("parses");
    assert_eq!(item.response_id(), Some("resp_bcb93365"));
}

#[test]
fn from_value_response_id_absent_is_none() {
    let v = serde_json::json!({
        "type": "message",
        "id": "item_abc",
        // no response_id key
    });
    let item = Item::from_value(&v).expect("parses");
    assert_eq!(item.response_id(), None);
}
```

- [ ] **Step 2: Write the failing test for `ResponseEvent::InProgress` carrying the id**

```rust
#[test]
fn in_progress_carries_response_id() {
    // Shape from response.in_progress: { "response": { "id": "resp_..." } }
    let ev = /* parse a captured response.in_progress event into ResponseEvent */;
    match ev {
        ResponseEvent::InProgress { response_id } => {
            assert_eq!(response_id.as_deref(), Some("resp_37ba30e3"));
        }
        other => panic!("expected InProgress, got {other:?}"),
    }
}
```

Match the exact parse entry point the module already uses to build a `ResponseEvent` from bytes (find the existing `response.*` decode site in `event.rs`; do not invent one).

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p lens-client stream::event -- from_value_retains_response_id in_progress_carries`
Expected: FAIL — `no field response_id` / `no method response_id` / `InProgress` is a unit variant.

- [ ] **Step 4: Add the `response_id` field to the `Item` variants**

At `event.rs:637-679`, add `response_id: Option<String>` to each of `Message`, `FunctionCall`, `FunctionCallOutput`, `Error`, `ResourceEvent`. Leave `Other` unchanged.

- [ ] **Step 5: Read the key in `Item::from_value` and add the accessor**

At `event.rs:1058-1133`, in each variant's construction, populate the field from the wire object: `obj.get("response_id").and_then(|v| v.as_str()).map(str::to_owned)`. Then add the accessor near the `Item` impl:

```rust
impl Item {
    /// The server `response_id` this item belongs to, if the wire carried one.
    /// `None` for the `Other` catch-all and for pre-response_id wire rows.
    pub fn response_id(&self) -> Option<&str> {
        match self {
            Item::Message { response_id, .. }
            | Item::FunctionCall { response_id, .. }
            | Item::FunctionCallOutput { response_id, .. }
            | Item::Error { response_id, .. }
            | Item::ResourceEvent { response_id, .. } => response_id.as_deref(),
            Item::Other { .. } => None,
        }
    }
}
```

Match the exact variant field syntax already used (tuple vs struct variants); the design confirms these are the five id-bearing variants.

- [ ] **Step 6: Change `ResponseEvent::InProgress` to carry the id**

At `event.rs:572` change `InProgress,` to `InProgress { response_id: Option<String> },`. In the `response.*` decode path, build it as `ResponseEvent::InProgress { response_id: obj.pointer("/response/id").and_then(|v| v.as_str()).map(str::to_owned) }`. Fix every existing `match`/construction site of `ResponseEvent::InProgress` in `lens-client` to the new struct shape (compiler will list them).

- [ ] **Step 7: Run the tests to verify they pass**

Run: `cargo test -p lens-client stream::event`
Expected: PASS (new tests + existing suite green).

- [ ] **Step 8: Gate + commit**

```bash
cargo clippy -p lens-client --all-targets -- -D warnings && cargo fmt -p lens-client -- --check
git add crates/lens-client/src/stream/event.rs
git commit -m "feat(lens-client): retain response_id on stream::Item + ResponseEvent::InProgress"
```

---

## Task 2: Domain type + catch-up mapping + persistence

Implements design **§2.1, §4.1, §5**. Replace the write-only `BlockContext.turn: u32` with `response_id: Option<ResponseId>`, map the wire id on the catch-up path, and persist/restore/reconcile it — leaving the tree green (the live-path stamp is a `None` placeholder filled by Task 3).

**Files:**
- Modify: `crates/lens-core/src/domain/item.rs:20`
- Modify: `crates/lens-core/src/actor/runloop.rs:226-230`
- Modify: `crates/lens-core/src/reduce/items.rs` (`:17` stamp, `:365` test)
- Modify: `crates/lens-core/src/persist/transcript.rs` (`:41-62`, `:146`, `:295-321`)
- Modify: `crates/lens-core/src/persist/map.rs:168-184`
- Test: `reduce` + `persist` test modules

**Interfaces:**
- Consumes: `lens_client::stream::Item::response_id()` (Task 1); `ResponseId` (`domain/ids.rs:36`, `branded_id!`, transparent `String` newtype).
- Produces:
  - `BlockContext { agent, depth, response_id: Option<ResponseId> }` — the `turn` field is gone.
  - Persistence: nullable `response_id TEXT` column on `items`; catch-up rows carry the real id; reconcile promotes it.
  - Every other reader compiles because `turn` had no behavioral reader (design §2.1, verified).

- [ ] **Step 1: Write the failing catch-up mapping test**

In the `runloop`/`wire_to_domain_item` test module, assert the wire id lands on the domain item and absence maps to `None`.

```rust
#[test]
fn catch_up_maps_wire_response_id() {
    let wire = /* a lens_client::stream::Item::Message with response_id Some("resp_bcb93365") */;
    let item = wire_to_domain_item(&wire /*, existing args */);
    assert_eq!(item.ctx.response_id.as_deref(), Some("resp_bcb93365"));
}

#[test]
fn catch_up_absent_response_id_is_none() {
    let wire = /* an Item with response_id None */;
    let item = wire_to_domain_item(&wire /*, existing args */);
    assert_eq!(item.ctx.response_id, None);
}
```

`ResponseId` is a transparent `String` newtype; `as_deref()` works via its `Deref`/`AsRef` — if not, compare against `Some(&ResponseId::from("resp_bcb93365".to_string()))` using the constructor `branded_id!` generates.

- [ ] **Step 2: Write the failing persistence round-trip + reconcile-promotion tests**

In the `persist/transcript.rs` test module:

```rust
#[test]
fn response_id_round_trips() {
    let store = /* fresh TranscriptStore on a temp db */;
    let item = /* Item with ctx.response_id = Some(ResponseId::from("resp_abc")) */;
    store.upsert(&item).unwrap();
    let back = store.load_all().unwrap();
    assert_eq!(back[0].ctx.response_id.as_deref(), Some("resp_abc"));
}

#[test]
fn reconcile_promotes_response_id() {
    let store = /* store with a provisional row whose ctx.response_id = None */;
    // authoritative /items data arrives carrying resp_abc for the same item
    store.reconcile(/* authoritative item with response_id Some("resp_abc") */).unwrap();
    let back = store.load_all().unwrap();
    assert_eq!(back[0].ctx.response_id.as_deref(), Some("resp_abc"));
}

#[test]
fn legacy_turn_column_written_zero() {
    // older-binary degrade path SELECTs `turn`; it must stay NOT NULL and readable
    let store = /* store */;
    store.upsert(&item_with_response_id).unwrap();
    let turn: i64 = store.raw_query_scalar("SELECT turn FROM items LIMIT 1").unwrap();
    assert_eq!(turn, 0);
}
```

Adapt method names to the actual `TranscriptStore` API (`upsert`/`load_all`/`reconcile` are illustrative — use the real ones the file already exposes).

- [ ] **Step 3: Run to verify failure**

Run: `cargo test -p lens-core -- catch_up_maps response_id_round_trips reconcile_promotes legacy_turn_column`
Expected: FAIL — `no field response_id on BlockContext` (compile error is an acceptable red).

- [ ] **Step 4: Swap the `BlockContext` field**

At `item.rs:20`:

```rust
pub struct BlockContext {
    pub agent: Option<String>,
    pub depth: u32,
    pub response_id: Option<ResponseId>, // was: pub turn: u32
}
```

Ensure `ResponseId` is imported in `item.rs`.

- [ ] **Step 5: Fix the two reducer stamp/test sites to compile**

At `reduce/items.rs:17`, replace `turn: scratch.turn,` with `response_id: None,` (the live-path real value is Task 3). At `reduce/items.rs:365`, change `assert_eq!(s.items[0].ctx.turn, 0);` to `assert_eq!(s.items[0].ctx.response_id, None);`. Fix any other `BlockContext { .. }` literal the compiler flags (e.g. constructors that spelled out `turn`).

- [ ] **Step 6: Map the wire id on catch-up**

At `runloop.rs:226-230`, replace the hardcoded `turn: 0` construction:

```rust
ctx: BlockContext {
    agent: None,
    depth: 0,
    response_id: wire.response_id().map(|s| ResponseId::from(s.to_owned())),
},
```

Use the actual `wire` binding name and the actual `ResponseId` constructor `branded_id!` emits (`ResponseId::from(String)` or `ResponseId::new`). Empty string → `None` (never fabricate): guard with `.filter(|s| !s.is_empty())` before `.map` if the wire can send `""`.

- [ ] **Step 7: Persistence — migration, write, read, reconcile**

1. **Migration** (`transcript.rs:41-62`, following the existing `ALTER TABLE items ADD COLUMN` pattern): add `ALTER TABLE items ADD COLUMN response_id TEXT` (nullable, no default). Do NOT bump `SCHEMA_VERSION`. Keep the existing `turn` column.
2. **Upsert write** (`transcript.rs:146`): the row still writes the legacy `turn` column — write literal `0i64` there (design §5 decision: degrade-only, all-zero collapses old-binary grouping to one group). Add `response_id` to the write: bind `item.ctx.response_id.as_deref()` (a `&str`/`String`) into the new column.
3. **Read** (`map.rs:168-184`, `row_to_item`): read the `response_id` column into `ctx.response_id` (`Option<ResponseId>`, `NULL` → `None`); stop rehydrating `turn` into the (now-deleted) struct field — drop that read entirely.
4. **Reconcile promotion** (`transcript.rs:295-321`): the reconcile `UPDATE` currently sets id/kind/payload/`live_seq`/`provisional`/`call_id` but not `ctx`; add `response_id = ?` to the `SET` clause and bind the authoritative `response_id`, so a provisional row stamped `None` is corrected when `/items` arrives.

- [ ] **Step 8: Run the tests to verify they pass**

Run: `cargo test -p lens-core`
Expected: PASS (new tests + existing suite; the `:365` assertion now checks `response_id`).

- [ ] **Step 9: Gate + commit**

```bash
cargo run -p xtask -- gate
git add crates/lens-core/src/domain/item.rs crates/lens-core/src/actor/runloop.rs \
        crates/lens-core/src/reduce/items.rs crates/lens-core/src/persist/transcript.rs \
        crates/lens-core/src/persist/map.rs
git commit -m "feat(lens-core): response_id replaces turn on BlockContext; catch-up map + persistence"
```

---

## Task 3: Live liveness + per-item live stamping

Implements design **§2.2, §2.3, §4.2, §4.3**. Add the `active_response` scalar sourced from `response.in_progress` (the only working in-process liveness source), emit the `ActiveResponseChanged` feed delta, and stamp each live item with its own wire `response_id`. Success criterion is "the actor feed exposes the delta" — the transcript replica that consumes it is T-2.

**Files:**
- Modify: `crates/lens-core/src/domain/session.rs`
- Modify: `crates/lens-core/src/reduce/update.rs:71` (region)
- Modify: `crates/lens-core/src/reduce/mod.rs` + `crates/lens-core/src/reduce/items.rs`
- Modify: `crates/lens-ui/src/card/model.rs:163`
- Test: `reduce` test module; `SmallVec` budget bench

**Interfaces:**
- Consumes: `ResponseEvent::InProgress { response_id }` (Task 1); `BlockContext.response_id` (Task 2); `Updates = SmallVec<[StreamUpdate; 2]>` (`update.rs:71`).
- Produces:
  - `SessionState.active_response: Option<ResponseId>` — `Some` mid-turn, `None` idle/unknown.
  - `StreamUpdate::ActiveResponseChanged(Option<ResponseId>)` — a value-carrying delta matching `StatusChanged`/`ModelChanged`.
  - Live items created from `output_item.done` stamp their own wire `response_id`; reducer-synthesized items fall back to `active_response`.

- [ ] **Step 1: Write the failing set/clear + emit tests**

```rust
#[test]
fn in_progress_sets_active_and_emits() {
    let (state, updates) = reduce(state0, event_response_in_progress("resp_37ba30e3"));
    assert_eq!(state.active_response.as_deref(), Some("resp_37ba30e3"));
    assert!(updates.iter().any(|u|
        matches!(u, StreamUpdate::ActiveResponseChanged(Some(r)) if r.as_str() == "resp_37ba30e3")));
}

#[test]
fn terminal_response_clears_active_and_emits_none() {
    let (state, updates) = reduce(state_mid_turn, event_response_completed("resp_37ba30e3"));
    assert_eq!(state.active_response, None);
    assert!(updates.iter().any(|u| matches!(u, StreamUpdate::ActiveResponseChanged(None))));
}
```

Cover `completed`/`failed`/`incomplete`/`cancelled` as the terminal set (design §6: each carries the ending response's id).

- [ ] **Step 2: Write the failing per-item live-stamp test**

```rust
#[test]
fn output_item_done_stamps_own_response_id() {
    let (state, _) = reduce(state_mid_turn, event_output_item_done_with("resp_bcb93365"));
    assert_eq!(state.items.last().unwrap().ctx.response_id.as_deref(), Some("resp_bcb93365"));
}

#[test]
fn synthesized_item_falls_back_to_active_response() {
    // a finalized streaming accumulator has no wire item id → uses active_response
    let (state, _) = reduce(state_with_active("resp_abc"), event_finalize_accumulator());
    assert_eq!(state.items.last().unwrap().ctx.response_id.as_deref(), Some("resp_abc"));
}
```

- [ ] **Step 3: Write the failing greedy-batch ordering test**

```rust
#[test]
fn greedy_batch_active_item_none_settles_committed_item() {
    // Active(A) → item(A) → Active(None) in one batch may expose only None;
    // the committed item must still carry response_id A and read as settled.
    let (state, updates) = reduce_batch(state0, &[
        active_in_progress("resp_A"),
        output_item_done("resp_A"),
        response_completed("resp_A"),
    ]);
    assert_eq!(state.items.last().unwrap().ctx.response_id.as_deref(), Some("resp_A"));
    assert_eq!(state.active_response, None);
    assert!(matches!(updates.last(), Some(StreamUpdate::ActiveResponseChanged(None))));
}
```

- [ ] **Step 4: Run to verify failure**

Run: `cargo test -p lens-core -- in_progress_sets_active terminal_response_clears output_item_done_stamps synthesized_item_falls_back greedy_batch`
Expected: FAIL — `no field active_response` / `no variant ActiveResponseChanged`.

- [ ] **Step 5: Add `SessionState.active_response`**

In `session.rs`, add `pub active_response: Option<ResponseId>,` to `SessionState` (default `None` in its constructor/`Default`).

- [ ] **Step 6: Add the `ActiveResponseChanged` delta**

In `reduce/update.rs`, add to the `StreamUpdate` enum:

```rust
/// The session's live active response changed. `Some` on response.in_progress,
/// `None` on any terminal response.* (idle/unknown). Sourced from
/// response.in_progress.response.id — the only working in-process liveness source.
ActiveResponseChanged(Option<ResponseId>),
```

- [ ] **Step 7: Add the `lens-ui` ignore arm (keep the exhaustive match compiling)**

In `crates/lens-ui/src/card/model.rs`, `SessionCard::fold_detailed` (`:163`), add:

```rust
// Liveness delta is consumed by the T-2 transcript replica, not the summary card.
StreamUpdate::ActiveResponseChanged(_) => {}
```

- [ ] **Step 8: Wire set/clear/emit + live stamping in the reducer**

In `reduce/mod.rs` (and `reduce/items.rs`):
1. On `ResponseEvent::InProgress { response_id }`: if `Some(id)`, set `state.active_response = Some(ResponseId::from(id))` and push `StreamUpdate::ActiveResponseChanged(Some(..))`. If `None`, no-op (defensive; live-verified always present).
2. On terminal `response.completed/failed/incomplete/cancelled`: set `state.active_response = None` and push `ActiveResponseChanged(None)`.
3. At the `reduce/items.rs:17` stamp site (currently `response_id: None` from Task 2): for items built from `output_item.done`, stamp the item's own wire `response_id`; for reducer-synthesized finalized accumulators (no wire item), fall back to `state.active_response.clone()`.

Keep the `response.in_progress` path within the `SmallVec<[StreamUpdate; 2]>` inline budget (≤2 updates) — if it must emit `StatusChanged` + `ActiveResponseChanged`, that is exactly 2; anything more is a documented spill flagged by Step 9's bench.

- [ ] **Step 9: Add the `SmallVec` budget bench/assert**

Add a test (or criterion bench if the crate benches per-path) asserting the `response.in_progress` reduction emits ≤2 `StreamUpdate`s so `Updates` stays inline (no heap spill):

```rust
#[test]
fn in_progress_stays_within_smallvec_inline_budget() {
    let (_, updates) = reduce(state0, event_response_in_progress("resp_x"));
    assert!(updates.len() <= 2, "response.in_progress emitted {} updates; SmallVec inline cap is 2", updates.len());
}
```

- [ ] **Step 10: Run the tests to verify they pass**

Run: `cargo test -p lens-core && cargo test -p lens-ui`
Expected: PASS.

- [ ] **Step 11: Gate + commit**

```bash
cargo run -p xtask -- gate
git add crates/lens-core/src/domain/session.rs crates/lens-core/src/reduce/update.rs \
        crates/lens-core/src/reduce/mod.rs crates/lens-core/src/reduce/items.rs \
        crates/lens-ui/src/card/model.rs
git commit -m "feat(lens-core): active_response liveness + ActiveResponseChanged delta + live response_id stamping"
```

---

## Cross-family review + live rider (post-implementation)

Per project rules (`AGENTS.md` review-diversity + memory `whole-branch-review-needs-a-builder`):

- [ ] Cross-family whole-slice review of the T-0 diff by a non-authoring family, at least one reviewer that runs the gate. Adjudicate divergence by direct run.
- [ ] **Live rider** against real omnigent 0.5.1 (design §6, §10): drive two claude-sdk turns; confirm per-turn distinct `response_id`, and the cancel→retry mints a new `response_id` with `previous_response_id: null`. Capture is the proof, not the test.

---

## Self-Review (author checklist — completed)

**Spec coverage:**
- §2.1 `BlockContext` replace → Task 2 Step 4. §2.2 `active_response` → Task 3 Step 5. §2.3 `ActiveResponseChanged` + SmallVec budget → Task 3 Steps 6, 9.
- §3 lens-client widening (Item + from_value + InProgress) → Task 1.
- §4.1 catch-up mapping → Task 2 Step 6. §4.2 live set/clear/stamp → Task 3 Step 8. §4.3 feed delta + `SessionCard` ignore arm → Task 3 Steps 6-7.
- §4.4 reconnect degrade → no code (documented accept: `active_response = None` until next `in_progress`, which Task 3's clear-on-terminal already yields).
- §5 persistence (migration, legacy `turn`=0, write, read, reconcile promotion) → Task 2 Step 7.
- §6 turn semantics → covered by per-turn distinct id (tests) + §Cross-family live rider. §7 `created_at` → explicitly out of scope, no task (correct).
- §8 testing strategy → tests distributed across Tasks 1-3. §10 success criteria → all mapped.

**Not owned (correctly no task):** ViewBlock projection (T-1), real `created_at`/durations (T-6), `stream.turn` Ready-counter bug (Board handoff), transcript replica consumption (T-2).

**Type consistency:** `response_id: Option<ResponseId>` (domain) vs `Option<String>` (lens-client) with conversion at `wire_to_domain_item` — consistent across Tasks 1→2. `ActiveResponseChanged(Option<ResponseId>)` spelled identically in Task 3 Steps 1, 6, 7, 9. `Item::response_id()` accessor produced in Task 1, consumed in Task 2 Step 6.

**Placeholder scan:** No TBD/TODO. Test bodies use illustrative store/event constructors (`upsert`, `event_response_in_progress`, etc.) explicitly flagged to map onto the real API — this is deliberate, since fabricating exact bodies for unread code would mislead the executor (see plan preamble).
