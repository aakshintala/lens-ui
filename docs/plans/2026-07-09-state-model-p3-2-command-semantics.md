# State-model P3-2: Command Semantics Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land D16 optimistic-send reconcile and D18 path-keyed error mapping on the P3-1 `ActiveSession` actor, plus the three deferred P3-1 review items (M2/M1/Nit), so command outcomes and stream-terminal lifecycle are typed, testable, and ready for P3-3 quiesce.

**Architecture:** Extend the existing off-thread actor (`crossbeam::Select` over events + commands) with a `Send` command that POSTs via lens-client, stamps server ack ids onto a restructured `PendingUserMessage`, and reconciles the optimistic bubble by a fixed (1)→(2)→(3) precedence on both live `session.input.consumed` and reconnect replay. Split §13.1 into Table A (stream `Disconnected{reason}` → park/stop) and Table B (`ClientError` on command/REST → command outcome + introspection ring). Introduce actor-owned, never-persisted `reconcile_in_flight` + transport state so P3-3's `is_quiesced()` can treat parked sessions as non-quiesced.

**Tech Stack:** Rust (edition 2024, rust-version 1.91), existing `crossbeam-channel` / `async-channel` / `lens-client` / `lens-core` / `lens-store` stack from P3-1. No new workspace deps expected.

## Context / spec refs

| Source | What it locks |
| --- | --- |
| Spec `docs/specs/2026-07-08-state-model-engine-design.md` §2.2 D16 (≈L210–231), D18 (≈L260–282); §4 P3(b) (≈L433–470); §7.1 §13.1 amendment row | Authoritative D16/D18 decisions |
| Design doc `docs/design/app-architecture-and-state-model.md` §13.1 (≈L1190–1238) | Already-amended two-table wording (verify, don't silently re-decide) |
| Handoff `docs/handoffs/2026-07-09-state-model-p3-1-execution-and-p3-2-next.md` | Scope, file touchpoints, P3-3 ordering, deferred M2/M1/Nit |
| Ledger `.superpowers/sdd/progress.md` ("STATE-MODEL P3-1") | What exists; deferred items in detail |
| P3-1 plan `docs/plans/2026-07-09-state-model-p3-1-actor-foundation.md` | Format template + foundation contracts this plan extends |

**Fresh finding (2026-07-09, not yet in committed docs — supersedes the handoff's open D16 live-verify rider):** Verified live against omnigent 0.4.0 (pinned commit `31669e1b`) and by reading `server/routes/sessions.py:19368-19379` (bare dict return, no `response_model` coercion):

- POST `/v1/sessions/{id}/events` ack is a **non-empty** body. Live example (HTTP 202): `{"queued":true,"item_id":"msg_f51e3a3a97524ae78a21b7d408b69ff3"}`.
- Exactly **one** id is populated per message POST: **non-native session → `item_id`** (persisted store id); **healthy native terminal → `pending_id`** (optimistic bubble id).
- Therefore reconcile precedence **(1) native-by-`pending_id`** and **(2) store-by-`item_id`** are the **common paths** — not dead code. `#[serde(default)]` does not mask them. Precedence **(3) content/ordinal match is defensive-only**.
- **GOTCHA (must not be coded away):** native ⇏ `pending_id`. A native session whose terminal is down fails the ensure-probe and the server returns the failure-turn's **`item_id`** (not `pending_id`). Reconcile MUST key on *whichever id is present* via the (1)→(2)→(3) chain and must **NEVER** hardcode `native ⇒ pending_id`.

**Conflict note (handoff vs fresh finding):** The handoff still says "do the D16 live-verify rider first." That rider is **resolved** by the finding above. This plan records the result as Task 1 (fixture + doc note) and does **not** re-open a live omnigent spin as a gate. If an executor wants belt-and-suspenders confirmation, it is optional and non-blocking.

**Conflict note (spec §4 P3(b) rule 3 vs D16):** §4 P3(b) rule 3 said "no client-supplied correlation id… do not design assuming one." D16 **explicitly supersedes** that framing (spec L225–227): a *client-supplied* id still doesn't exist, but the *server-returned* ack id does. Plan follows D16.

**Prerequisite gaps found while reading code (not called out as separate tasks in the handoff, but load-bearing for D16):**

1. `SessionEvent::InputConsumed` today carries only `{ item_id, item_type }` (`lens-client/src/stream/event.rs:73–76`, raw parse L314–318) — **`cleared_pending_id` is on the wire and in `generated.rs` but not plumbed** into the typed event. Native by-id live reconcile (precedence 1) needs it.
2. Hand-written `SessionSnapshot` (`sessions.rs:79–146`) does **not** yet model `pending_inputs` (present in `generated.rs` / openapi). Reconnect native re-hydrate (spec §4 P3(b) native path) needs it.

## Global Constraints

- Edition **2024**, `rust-version = "1.91"`; crates under `crates/` set `lints.workspace = true`.
- `crates/lens-client/src/generated.rs` is **NEVER** hand-edited.
- **No `serde_json::Value` leaks to consumers** — typed wrappers only.
- **`lens-core` has NO gpui dependency.** All gpui touch-points stay in `lens-store`.
- Each task lands **green**: `cargo fmt --check` + `cargo clippy --all-targets` (0 warnings) + `cargo test`, scoped to the touched crate(s).
- **No foreground blocking** — POST/`send_event` and all I/O run on the actor OS thread (or a helper it owns); foreground only applies value-carrying deltas.
- **Value-carrying-completeness rule:** every `SessionState` field the reducer/actor writes MUST emit a `StreamUpdate` delta carrying that field's new value, else the gpui replica silently misses it. Any new `PendingUserMessage` / pending-user field added here needs a carrying delta + an `apply` arm.
- **OUT OF SCOPE (P3-3 — do not implement):** D17 quiesce/sleep/wake, D11 byte-window eviction, blocking `GET /items` tail-pagination lift. Reconnect reconcile uses items already folded into `state.items` via bootstrap/`SnapshotRestored` / live stream — **not** a new REST list call.
- **MANDATORY cross-family review** at the temporal send/reconcile path (Tasks 5–6) and the error-mapping lifecycle transitions (Tasks 7–8).

---

## File Structure

**Modified `crates/lens-client`:**
- `src/stream/event.rs` — plumb `cleared_pending_id: Option<String>` onto `SessionEvent::InputConsumed` + raw parse + tests.
- `src/sessions.rs` — model `pending_inputs` on `SessionSnapshot` (typed `{pending_id, content}` rows) + accessors; keep `SendEventAck` as-is (`:697`).

**Modified `crates/lens-core`:**
- `src/domain/controls.rs` — restructure `PendingUserMessage` (`:71`): keep Lens-local `pending_id`; add `server_pending_id` / `store_item_id`.
- `src/reduce/update.rs` — add `PendingUserChanged(Vec<PendingUserMessage>)` (value-carrying); comment reserved `CollaborationModeChanged` / `TitleChanged`.
- `src/reduce/{folds,mod,items,snapshot}.rs` — InputConsumed reconcile hook; M1 ScratchChanged completeness; snapshot `pending_inputs` re-hydrate; TitleChanged emit if snapshot title fold gains a live producer (or comment-only).
- `src/actor/mod.rs` + `runloop.rs` — `SessionCommand::Send{…}`; M2 Demote guard; park/stop transport; `reconcile_in_flight`; command outcome + introspection ring; inject a send capability (`SessionApi` trait or equivalent).
- New: `src/actor/reconcile.rs` (or inline in runloop if small) — (1)→(2)→(3) precedence helpers.
- New: `src/actor/errors.rs` (or `outcome.rs`) — Table A/B mapping + `ActorOutcome` ring.
- `src/actor/summary.rs` — only if Summary needs a parked/needs-attention flag tweak (likely already covered by Failed status).

**Modified `crates/lens-store`:**
- `src/lib.rs` — `apply` arm for `PendingUserChanged` (+ any new lifecycle markers if introduced).

**Docs (light, in the task that needs them):**
- Confirm design-doc §13.1 matches D18 (already looks amended); if drift, sync. Record live-verify finding in STATUS / handoff note only if the executor is asked to — **not** a blocker for code tasks.

---

## Task 1: Record D16 live-verify result + ack fixture (de-risk)

Lock the fresh finding into a regression fixture so `SendEventAck` deser cannot silently regress to "empty body ⇒ all None." No production behavior change beyond a test + a short comment on `SendEventAck`.

**Files:**
- Modify: `crates/lens-client/src/sessions.rs` (`SendEventAck` doc comment near `:693–717`; tests near `:1900`)
- Test: `crates/lens-client/src/sessions.rs` (`#[cfg(test)]`)

**Interfaces:**
- Consumes: existing `SendEventAck` (`queued` / `item_id` / `pending_id` / `denied` / `reason` / `elicitation_id`).
- Produces: documented contract + fixtures proving (a) non-native `item_id`-only ack and (b) native `pending_id`-only ack deserialize; empty `{}` still defaults to all-`None` (defensive).

- [ ] **Step 1: Write failing tests for the two common ack shapes**

```rust
#[test]
fn ack_non_native_message_carries_item_id_only() {
    // Live 2026-07-09 vs omnigent 0.4.0 @ 31669e1b (HTTP 202).
    let ack: SendEventAck = serde_json::from_str(
        r#"{"queued":true,"item_id":"msg_f51e3a3a97524ae78a21b7d408b69ff3"}"#,
    )
    .unwrap();
    assert!(ack.queued);
    assert_eq!(
        ack.item_id.as_deref(),
        Some("msg_f51e3a3a97524ae78a21b7d408b69ff3")
    );
    assert_eq!(ack.pending_id, None, "exactly one id populated per message POST");
}

#[test]
fn ack_native_healthy_terminal_carries_pending_id_only() {
    // Shape confirmed by server route + live-verify; bytes may be synthetic if
    // the live capture was non-native-only — keep the invariant explicit.
    let ack: SendEventAck = serde_json::from_str(
        r#"{"queued":true,"pending_id":"pending_a1b2c3"}"#,
    )
    .unwrap();
    assert_eq!(ack.pending_id.as_deref(), Some("pending_a1b2c3"));
    assert_eq!(ack.item_id, None, "exactly one id populated per message POST");
}
```

Run: `cargo test -p lens-client ack_non_native_message_carries_item_id_only ack_native_healthy_terminal_carries_pending_id_only`
Expected: PASS if deser already works (likely — existing `ack_parses_queued_with_item_id`); if a test fails, fix deser before proceeding. The point of this task is the **invariant comment + dual-shape fixtures**, not new fields.

- [ ] **Step 2: Document the gotcha on `SendEventAck`**

Above the struct (`sessions.rs:693`):

```rust
/// Ack for `POST /v1/sessions/{id}/events` (HTTP 202).
///
/// Live-verified 2026-07-09 (omnigent 0.4.0 @ 31669e1b; route
/// `server/routes/sessions.py` bare dict return): body is NON-empty; exactly ONE
/// of `item_id` (non-native / native-with-terminal-down) or `pending_id`
/// (healthy native terminal) is populated per message POST. Do NOT assume
/// `native ⇒ pending_id` — a native session whose ensure-probe fails returns
/// `item_id`. `#[serde(default)]` remains so an empty/future body still parses.
```

- [ ] **Step 3: Gate**

Run: `cargo fmt -p lens-client && cargo clippy -p lens-client --all-targets && cargo test -p lens-client`
Expected: fmt clean, 0 warnings, all tests pass; `git diff --stat crates/lens-client/src/generated.rs` empty.

- [ ] **Step 4: Commit**

```bash
git add crates/lens-client/src/sessions.rs
git commit -m "test(lens-client): lock SendEventAck live shapes — one of item_id|pending_id (D16)"
```

**External dependency:** none (finding already done). Optional live re-confirm is non-blocking.

---

## Task 2: Plumb `cleared_pending_id` on `InputConsumed` (lens-client prerequisite)

Native live reconcile (D16 precedence 1) keys off `cleared_pending_id` from `session.input.consumed`. Wire field exists (`generated.rs` / fixtures already show `"cleared_pending_id": null`); typed `SessionEvent::InputConsumed` drops it today.

**Files:**
- Modify: `crates/lens-client/src/stream/event.rs` (`SessionEvent::InputConsumed` ≈`:73–76`; `RawInputConsumedData` ≈`:314–318`; parse arm ≈`:750–754`; test `input_consumed_reads_nested_data` ≈`:1151`)
- Test: same module

**Interfaces:**
- Consumes: wire `data.cleared_pending_id: Option<String>`.
- Produces:
```rust
SessionEvent::InputConsumed {
    item_id: String,
    item_type: String,
    cleared_pending_id: Option<String>, // NEW
}
```

- [ ] **Step 1: Failing test — present + absent**

```rust
#[test]
fn input_consumed_carries_cleared_pending_id_when_present() {
    let ev = parse_event(&frame(
        "session.input.consumed",
        r#"{"data":{"item_id":"msg_1","type":"message","data":{},"cleared_pending_id":"pending_a1b2c3"}}"#,
    ));
    assert_eq!(
        ev,
        ServerStreamEvent::Session(SessionEvent::InputConsumed {
            item_id: "msg_1".into(),
            item_type: "message".into(),
            cleared_pending_id: Some("pending_a1b2c3".into()),
        })
    );
}

#[test]
fn input_consumed_cleared_pending_id_defaults_none() {
    let ev = parse_event(&frame(
        "session.input.consumed",
        r#"{"data":{"item_id":"msg_1","type":"message","data":{}}}"#,
    ));
    let ServerStreamEvent::Session(SessionEvent::InputConsumed {
        cleared_pending_id,
        ..
    }) = ev
    else {
        panic!("expected InputConsumed");
    };
    assert_eq!(cleared_pending_id, None);
}
```

Run: `cargo test -p lens-client input_consumed_carries_cleared_pending_id` → Expected: FAIL (field missing / struct mismatch).

- [ ] **Step 2: Minimal plumb**

```rust
// SessionEvent variant
InputConsumed {
    item_id: String,
    item_type: String,
    cleared_pending_id: Option<String>,
}

// RawInputConsumedData
#[derive(Deserialize)]
struct RawInputConsumedData {
    item_id: String,
    #[serde(rename = "type")]
    item_type: String,
    #[serde(default)]
    cleared_pending_id: Option<String>,
}

// parse arm
SessionEvent::InputConsumed {
    item_id: r.data.item_id,
    item_type: r.data.item_type,
    cleared_pending_id: r.data.cleared_pending_id,
}
```

Update every existing `InputConsumed { … }` construction in tests/fixtures helpers to include `cleared_pending_id: None` (or `Some` where asserting).

- [ ] **Step 3: Gate + commit**

Run: `cargo fmt -p lens-client && cargo clippy -p lens-client --all-targets && cargo test -p lens-client`
Also: `cargo test -p lens-core` (folds match `InputConsumed` — will fail compile until Task 6 wires reconcile; **if** lens-core match arms are non-exhaustive on fields, fix the pattern to `InputConsumed { .. }` temporarily or land Task 6's fold in the same PR wave — prefer keeping this task lens-client-only by using `..` in lens-core's current ignore arm at `folds.rs:132`).

```bash
git add crates/lens-client/src/stream/event.rs
git commit -m "feat(lens-client): plumb cleared_pending_id on session.input.consumed (D16 prereq)"
```

---

## Task 3: Model `pending_inputs` on `SessionSnapshot` (lens-client prerequisite)

Reconnect native path (spec §4 P3(b) / D16): snapshot `pending_inputs: [{pending_id, content}]` re-hydrates still-pending bubbles. Hand-written snapshot currently skips the wire field.

**Files:**
- Modify: `crates/lens-client/src/sessions.rs` (`SessionSnapshot` ≈`:79–146` + `impl` accessors)
- Test: `crates/lens-client/src/sessions.rs` tests (snapshot deser fixtures)

**Interfaces:**
- Produces:
```rust
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct PendingInput {
    pub pending_id: String,
    pub content: String, // wire may be richer later; start with string-or-extract — verify against openapi/generated
}

// on SessionSnapshot:
#[serde(default, deserialize_with = "de_null_default")]
pending_inputs: Vec<PendingInput>,

pub fn pending_inputs(&self) -> &[PendingInput] { &self.pending_inputs }
```

> **Verify against ground truth before coding the `content` type:** `generated.rs` documents `{"pending_id","content"}` — confirm whether `content` is a plain string or a content-block array. If the generated schema says string, use `String`; if blocks, mirror the minimal shape needed for content-match (defensive path 3) without pulling `serde_json::Value` into lens-core (extract text in lens-client or pass `String` of joined text). Cite the chosen schema line in the commit message.

- [ ] **Step 1: Failing deser test**

```rust
#[test]
fn snapshot_parses_pending_inputs() {
    let raw = r#"{
      "id":"conv_1","status":"idle","agent_id":"ag",
      "created_at":0,"harness":"claude-native",
      "pending_inputs":[{"pending_id":"pending_a1","content":"hello"}]
    }"#;
    let s: SessionSnapshot = serde_json::from_str(raw).unwrap();
    assert_eq!(s.pending_inputs().len(), 1);
    assert_eq!(s.pending_inputs()[0].pending_id, "pending_a1");
}

#[test]
fn snapshot_pending_inputs_null_is_empty() {
    let raw = r#"{
      "id":"conv_1","status":"idle","agent_id":"ag",
      "created_at":0,"harness":"x","pending_inputs":null
    }"#;
    let s: SessionSnapshot = serde_json::from_str(raw).unwrap();
    assert!(s.pending_inputs().is_empty());
}
```

Run → Expected: FAIL (unknown field ignored today ⇒ empty / no accessor).

- [ ] **Step 2: Implement field + accessor + null tolerance**

Use existing `de_null_default` pattern (same file) so explicit `null` does not fail deser.

- [ ] **Step 3: Gate + commit**

Run: `cargo fmt -p lens-client && cargo clippy -p lens-client --all-targets && cargo test -p lens-client`

```bash
git add crates/lens-client/src/sessions.rs
git commit -m "feat(lens-client): SessionSnapshot.pending_inputs for native reconnect rehydrate (D16)"
```

---

## Task 4: Restructure `PendingUserMessage` + value-carrying `PendingUserChanged` (D16 shape)

Restructure the domain type and give the replica a carrying delta **before** wiring send/reconcile, so later tasks cannot forget the completeness rule.

**Files:**
- Modify: `crates/lens-core/src/domain/controls.rs` (`PendingUserMessage` ≈`:67–76` + roundtrip test ≈`:114`)
- Modify: `crates/lens-core/src/reduce/update.rs` (add variant)
- Modify: `crates/lens-store/src/lib.rs` (`apply` match — must stay exhaustive)
- Modify: any persist/map sites that construct `PendingUserMessage` (grep)
- Test: `controls.rs` + `lens-store` apply test

**Interfaces:**
- Produces:
```rust
pub struct PendingUserMessage {
    /// Lens-local id — addresses the optimistic bubble for rollback/UI.
    pub pending_id: String,
    /// ← `SendEventAck.pending_id`, stamped at POST-return (native healthy path).
    pub server_pending_id: Option<String>,
    /// ← `SendEventAck.item_id`, stamped at POST-return (non-native / native-down).
    pub store_item_id: Option<String>,
    pub content: String,
    pub created_at: i64,
}

// StreamUpdate
PendingUserChanged(Vec<PendingUserMessage>),
```

- [ ] **Step 1: Failing roundtrip + apply tests**

```rust
#[test]
fn pending_user_message_carries_server_ids() {
    let p = PendingUserMessage {
        pending_id: "lens_pend_1".into(),
        server_pending_id: Some("pending_a1".into()),
        store_item_id: None,
        content: "hi".into(),
        created_at: 1,
    };
    let back: PendingUserMessage =
        serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
    assert_eq!(back, p);
}

// lens-store
#[test]
fn apply_pending_user_changed_is_copy_assignment() {
    let mut s = state();
    let bubble = PendingUserMessage { /* … */ };
    apply(&mut s, StreamUpdate::PendingUserChanged(vec![bubble.clone()]));
    assert_eq!(s.pending_user, vec![bubble]);
}
```

Run → Expected: FAIL (fields / variant missing).

- [ ] **Step 2: Implement struct + variant + apply arm**

```rust
PendingUserChanged(v) => state.pending_user = v,
```

Update `pending_user_message_roundtrips` and every constructor. Default new fields to `None` at optimistic-insert time (pre-ack).

- [ ] **Step 3: Gate**

Run: `cargo fmt -p lens-core -p lens-store && cargo clippy -p lens-core -p lens-store --all-targets && cargo test -p lens-core && cargo test -p lens-store`

- [ ] **Step 4: Commit**

```bash
git add crates/lens-core/src/domain/controls.rs crates/lens-core/src/reduce/update.rs crates/lens-store/src/lib.rs
git commit -m "feat(lens-core): PendingUserMessage server ids + PendingUserChanged delta (D16)"
```

**Value-carrying callout:** `pending_user` mutations from here on **must** emit `PendingUserChanged(state.pending_user.clone())` (or the post-mutation vec). No silent field writes.

---

## Task 5: Sweep P3-1 deferred items (M2 load-bearing, M1, Nit)

Fold the whole-branch deferred set into the command surface before Send lands — especially **M2**, which can silently kill the actor on `Demote`.

> **Priority split — M2 is load-bearing, M1 is optional.** M2 is a real crash (silent actor-thread death) and MUST land. The Nit is a free comment. **M1, by contrast, self-heals** (attribution lags one delta, then corrects) — yet the fix touches the hottest reduce arms (`Completed` turn-bump, `AgentChanged`, `OutputItemDone` FunctionCall), adding regression surface to load-bearing paths for a cosmetic lag. Treat M1 as **optional within this task**: land it only if the Step-3 regression tests are green and the cross-family reviewer is comfortable; otherwise **defer M1 to its own change** and keep this task to M2 + Nit. Do not let M1 churn block the M2 fix from merging.

**Files:**
- Modify: `crates/lens-core/src/actor/runloop.rs` (Demote / Summary emit ≈`:139–181`)
- Modify: `crates/lens-core/src/reduce/mod.rs` (Completed / OutputItemDone ≈`:90–131`)
- Modify: `crates/lens-core/src/reduce/folds.rs` (`AgentChanged` ≈`:157–171`)
- Modify: `crates/lens-core/src/reduce/update.rs` (Nit comments on reserved variants)
- Test: `actor/runloop.rs`, `reduce/mod.rs` / `folds.rs`

**Interfaces:**
- M2 produces: Demote on a Detailed-only handle is **non-fatal** (recommended: treat missing summary consumer as non-fatal — `send_blocking` Err in Summary mode logs/pushes introspection and **continues**, same as "bridge gone" only when *both* channels are dead; **or** reject `Demote` when spawned via `spawn_actor` by tracking `summary_attached: bool`). Prefer **non-fatal missing summary consumer** so `spawn_actor` stays a thin wrapper and Demote is a no-op emit rather than a landmine.
- M1 produces: every arm that writes `stream.current_agent` or `stream.turn` also emits `ScratchChanged(Arc::new(state.stream.clone()))` when those fields change (even if open_message/open_reasoning unchanged).
- Nit: doc-comment on `CollaborationModeChanged` / `TitleChanged`: "reserved — no live SSE producer in P3; title mutates via `SnapshotRestored`/`Rebased` only unless a future fold emits this delta."

- [ ] **Step 1: Failing M2 test — Demote on `spawn_actor` must not kill the thread**

```rust
#[test]
fn demote_on_detailed_only_handle_does_not_kill_actor() {
    let (ev_tx, ev_rx) = crossbeam_channel::bounded(64);
    let (up_tx, up_rx) = async_channel::bounded(64);
    // spawn_actor drops the summary receiver (P3-1).
    let handle = spawn_actor(fresh_state(), ev_rx, up_tx, test_stores(), test_clock());

    handle.commands.send(SessionCommand::Demote).unwrap();
    ev_tx.send(status_running_event()).unwrap();

    // Must still emit on the Detailed channel OR survive in Summary-with-no-consumer.
    // Acceptance: actor accepts another Stop and joins; does not panic/poison.
    let _ = up_rx.try_recv(); // may be empty if mode flipped to Summary
    handle.stop_and_join(); // must return promptly
}
```

Run → Expected: FAIL today (actor dies on `summaries.send_blocking` Err → `return`).

- [ ] **Step 2: Fix M2 in the Summary emit path**

```rust
OutputMode::Summary => {
    // Missing summary consumer (Detailed-only spawn) is non-fatal: Demote
    // becomes a mode flip without a listener. Only exit when the Detailed
    // bridge is also gone (checked on Detailed sends / Promote Rebased).
    let _ = output
        .summaries
        .send_blocking(SummaryUpdate::from_state(&state));
}
```

(If introspection ring from Task 8 exists already, push an `ActorOutcome::SummaryConsumerGone` instead of `let _ =`; otherwise leave a `TODO(P3-2 Task 8)` comment and keep non-fatal.)

- [ ] **Step 3: Failing M1 tests — AgentChanged / Completed / FunctionCall attribution emit ScratchChanged**

```rust
#[test]
fn agent_changed_emits_scratch_when_current_agent_updates() {
    // reduce SessionEvent::AgentChanged; assert updates contain ScratchChanged
    // whose Arc.current_agent matches state.stream.current_agent.
}

#[test]
fn completed_always_emits_scratch_after_turn_bump() {
    // even with no open_message/open_reasoning, turn bump must carry ScratchChanged
    // so replica turn stays in sync.
}
```

Implement: remove the `if had_open_*` guard around ScratchChanged on Completed (or emit a second ScratchChanged dedicated to turn); on AgentChanged fold after setting `current_agent`, push `ScratchChanged`; on OutputItemDone FunctionCall agent_name path, emit ScratchChanged even when message preview was not cleared.

- [ ] **Step 4: Nit comments on reserved variants**

In `update.rs` above `CollaborationModeChanged` / `TitleChanged`, add the clarifying comments. No producer required unless this plan's snapshot title path wants a live delta — snapshot already rebases (I1); **do not** invent a SSE producer.

- [ ] **Step 5: Gate + commit**

Run: `cargo fmt -p lens-core && cargo clippy -p lens-core --all-targets && cargo test -p lens-core`

```bash
git add crates/lens-core/src/actor/runloop.rs crates/lens-core/src/reduce
git commit -m "fix(lens-core): Demote-safe summary emit (M2) + ScratchChanged completeness (M1)"
```

---

## Task 6: `SessionCommand::Send` — optimistic bubble, POST, stamp ack ids, rollback (D16 send half)

**REVIEW SEAM** (temporal/stateful). Introduce the send command: push optimistic `pending_user` → POST `Sessions::send_event` on the actor thread → stamp `server_pending_id`/`store_item_id` from ack → emit `PendingUserChanged`. On `ClientError::Network` (and denied ack), roll the bubble back.

**Files:**
- Modify: `crates/lens-core/src/actor/mod.rs`, `runloop.rs`
- Create: `crates/lens-core/src/actor/api.rs` (thin `SessionApi` trait — keeps tests free of real HTTP)
- Modify: `crates/lens-store/src/lib.rs` only if apply already done in Task 4
- Test: `crates/lens-core/src/actor/runloop.rs` (scripted `SessionApi`)

**Interfaces:**
```rust
pub enum SessionCommand {
    Stop,
    Promote,
    Demote,
    /// Optimistic user message. `content` is plain text for P3-2; actor wraps
    /// into `SessionEventInput::Message { content: [{type:input_text,text}], .. }`.
    Send {
        text: String,
        model_override: Option<String>,
    },
}

/// Actor-thread HTTP surface (lens-client Sessions subset).
pub trait SessionApi: Send {
    fn send_event(
        &self,
        id: &SessionId,
        evt: &SessionEventInput,
    ) -> Result<SendEventAck, ClientError>;
}

// spawn_actor* gains `api: Arc<dyn SessionApi>` (or Box). Tests use a mock.
```

Command-outcome channel (minimal for this task; Task 8 deepens Table B):

```rust
pub enum CommandOutcome {
    SendAccepted { lens_pending_id: String, ack: SendEventAck },
    SendDenied { lens_pending_id: String, reason: Option<String> },
    SendFailed { lens_pending_id: String, error: String }, // display; typed map in Task 8
}
// ActorHandle gains `outcomes: async_channel::Receiver<CommandOutcome>` (or a third sender passed in).
```

- [ ] **Step 1: Failing tests — optimistic insert, ack stamp, network rollback**

```rust
#[test]
fn send_inserts_optimistic_bubble_then_stamps_item_id_from_ack() {
    let api = MockApi::succeed_with_ack(SendEventAck {
        queued: true,
        item_id: Some("msg_42".into()),
        pending_id: None,
        ..Default::default()
    });
    let (ev_rx, /*…*/) = /* spawn with api */;
    handle.commands.send(SessionCommand::Send {
        text: "hello".into(),
        model_override: None,
    }).unwrap();

    let u = up_rx.recv_blocking().unwrap();
    // First emit: PendingUserChanged with lens-local pending_id, server ids None
    // OR single emit after ack — pick one sequencing and test it:
    // Recommended sequencing (matches UX): emit optimistic immediately, then
    // emit again after stamp (two PendingUserChanged). Assert final bubble has
    // store_item_id == Some("msg_42") and server_pending_id == None.
}

#[test]
fn send_network_error_rolls_back_optimistic_bubble() {
    let api = MockApi::fail(ClientError::Network(/*…*/));
    // after Send: pending_user empty again; outcome SendFailed; no stream teardown
}

#[test]
fn send_denied_ack_rolls_back_and_reports_reason() {
    let api = MockApi::succeed_with_ack(SendEventAck {
        queued: false,
        denied: true,
        reason: Some("policy".into()),
        ..Default::default()
    });
    // bubble removed; outcome SendDenied
}

#[test]
fn send_stamps_whichever_id_is_present_never_assumes_native() {
    // Case A: pending_id only → server_pending_id set, store_item_id None
    // Case B: item_id only → store_item_id set, server_pending_id None
    // No harness/native flag consulted when stamping.
}
```

Run → Expected: FAIL (`Send` not defined).

- [ ] **Step 2: Implement Send arm**

Pseudocode for the command arm:

```rust
SessionCommand::Send { text, model_override } => {
    let lens_pending_id = new_lens_pending_id(); // ulid/uuid/counter — document choice
    let bubble = PendingUserMessage {
        pending_id: lens_pending_id.clone(),
        server_pending_id: None,
        store_item_id: None,
        content: text.clone(),
        created_at: clock.now_millis(),
    };
    state.pending_user.push(bubble);
    emit_pending(&output, &state); // PendingUserChanged

    let evt = SessionEventInput::Message {
        content: vec![json!({"type":"input_text","text": text})],
        model_override,
        tools: None,
    };
    match api.send_event(&state.id, &evt) {
        Ok(ack) if ack.denied => {
            rollback_pending(&mut state, &lens_pending_id);
            emit_pending(&output, &state);
            let _ = outcomes.send_blocking(CommandOutcome::SendDenied { .. });
        }
        Ok(ack) => {
            if let Some(p) = state.pending_user.iter_mut().find(|p| p.pending_id == lens_pending_id) {
                // Stamp whichever id is present — NEVER branch on harness/native.
                p.server_pending_id = ack.pending_id.clone();
                p.store_item_id = ack.item_id.clone();
            }
            emit_pending(&output, &state);
            let _ = outcomes.send_blocking(CommandOutcome::SendAccepted { .. });
        }
        Err(ClientError::Network(_)) => {
            rollback_pending(&mut state, &lens_pending_id);
            emit_pending(&output, &state);
            let _ = outcomes.send_blocking(CommandOutcome::SendFailed { .. });
        }
        Err(e) => {
            // Full Table B in Task 8; for now: no stream teardown, map to SendFailed,
            // rollback only when D16 says so (Network). Other errors: keep or drop?
            // Spec Table B: Auth/NotFound escalate differently — leave bubble +
            // outcome for Task 8 to refine; document the temporary policy in code.
            let _ = outcomes.send_blocking(CommandOutcome::SendFailed { .. });
        }
    }
}
```

> Persist: `pending_user` is RAM-only intent (controls.rs doc; persist map already skips it). Do **not** write it through to SQLite.

- [ ] **Step 3: Gate + commit + review**

Run: `cargo fmt -p lens-core && cargo clippy -p lens-core --all-targets && cargo test -p lens-core`

```bash
git add crates/lens-core/src/actor
git commit -m "feat(lens-core): SessionCommand::Send optimistic bubble + ack stamp + network rollback (D16)"
```

**MANDATORY cross-family review** of this diff (temporal send path, ack stamping gotcha, rollback).

---

## Task 7: Reconcile precedence (1)→(2)→(3) — live consumed + reconnect replay (D16 reconcile half)

**REVIEW SEAM.** Drop the optimistic bubble when the server accepts the input, using the same precedence for live `InputConsumed` and reconnect/`SnapshotRestored` replay. Sits in the tail-bounded scope (bubbles are always at the tail — D12).

**Files:**
- Create: `crates/lens-core/src/actor/reconcile.rs` (pure helpers — unit-testable without the run-loop)
- Modify: `crates/lens-core/src/reduce/folds.rs` — the `InputConsumed { .. }` arm (`:132`) stops returning empty and calls the reconcile helper.

  **Placement decision (fixed — do not re-deliberate):** reconcile lives in the **pure reducer**, calling pure helpers in `reconcile.rs`. The reducer is already the single writer of `state.pending_user`, so doing by-id drops here (a) keeps one writer — no actor/reduce race, (b) colocates the value-carrying `PendingUserChanged` emit with the mutation, and (c) stays pure/deterministic/total because the helpers are pure (P1 contract holds). The helpers are shared by two reduce call sites — the `folds.rs` `InputConsumed` arm (live) and `snapshot.rs` `fold_snapshot` (reconnect). The actor does **not** post-process reconcile; it only forwards the emitted delta. (Cost is O(pending_user.len) per signal — bubbles are few and at the tail, D12.)
- Modify: `crates/lens-core/src/reduce/snapshot.rs` (keep `pending_user` on gap — already tested; add re-hydrate/drop against `pending_inputs` + items)
- Test: `reconcile.rs` unit matrix + reduce/actor integration

**Interfaces:**
```rust
/// Drop at most one matching bubble. Returns true if a bubble was removed.
pub fn reconcile_pending_user(
    pending: &mut Vec<PendingUserMessage>,
    signal: ReconcileSignal<'_>,
) -> bool;

pub enum ReconcileSignal<'a> {
    /// Live or replayed consumed event.
    Consumed {
        cleared_pending_id: Option<&'a str>, // server pending id
        item_id: &'a str,                    // store id of persisted message
        content: Option<&'a str>,            // for defensive (3); may be None on live event
    },
    /// Post-reconnect / snapshot: still-pending server ids + trailing user items.
    Snapshot {
        pending_inputs: &'a [PendingInput],
        trailing_user_item_ids_and_text: &'a [(String, String)],
    },
}
```

The two signals have **different shapes** and must be specified separately — do not fuse them into one branch. `Consumed` (live/replayed event) *drops* a bubble; `Snapshot` (reconnect) *keeps or drops* each bubble against the server's own pending list. Same three id-keys, different decision.

**Signal A — `Consumed` (one event → drop at most one bubble). Precedence, cite spec D16 L219–224:**

1. `server_pending_id.is_some() && server_pending_id == cleared_pending_id` → drop (native by-id).
2. else `store_item_id.is_some() && store_item_id == item_id` → drop (store by-id).
3. else, **only if the signal carries `content`** (Some) — content/ordinal match → drop the FIFO-oldest unmatched bubble with equal content. A *live* `session.input.consumed` event carries **no** message content (`content: None`), so path-3 is inert on live events: an unmatched bubble simply survives until the next snapshot reconcile picks it up via Signal B. Path-3 exists for replayed/enriched signals, not the live wire.

Walk 1→2→3 **per bubble**; stop at the first key that is `Some` and matches. Emit `PendingUserChanged(pending.clone())` iff a bubble was removed.

**Signal B — `Snapshot` (whole `pending_inputs` list + trailing user items → keep/drop each bubble). Decision table, cite spec §4 P3(b):**

| Bubble state at reconnect | Action |
| --- | --- |
| `server_pending_id` present in `snapshot.pending_inputs[].pending_id` | **keep** — server still has it pending (re-hydrate/leave as-is) |
| `server_pending_id` absent from `pending_inputs`, but `store_item_id` matches a replayed trailing user item id | **drop** — it landed as a persisted item |
| both server ids absent from `pending_inputs`/items, content matches a trailing user item | **drop** — defensive floor (path-3 equivalent) |
| none of the above (in neither `pending_inputs` nor items) | **drop as lost** — the send never persisted server-side. The reducer just drops it + emits `PendingUserChanged` (the bubble vanishes). An *explicit* "lost" surface (toast/outcome) can't originate in the pure reducer — if wanted, the **actor** diffs `pending_user` before/after the reconcile batch and pushes `ActorOutcome::SendLost` to its ring. **Deferred to Task 9** (out-of-band UX polish, not required for correctness). |

Rule 1 (§4 P3(b)): a reconnect with a gap must **not** blanket-clear `pending_user` — only the table above removes bubbles, per-bubble.

**GOTCHA reminder in code comment:** never consult the harness to pick a key; walk the id-keys per bubble. A native-terminal-down send arrives with `store_item_id` set and `server_pending_id` None — path 2 handles it with no `is_native` branch.

- [ ] **Step 1: Pure-helper failing matrix**

```rust
#[test]
fn precedence_server_pending_id_wins() { /* cleared_pending_id match drops; item_id ignored */ }

#[test]
fn precedence_store_item_id_when_no_server_pending() { /* item_id match drops */ }

#[test]
fn precedence_content_match_is_defensive_floor() {
    // both server ids None; content matches trailing user item → drop
}

#[test]
fn native_down_uses_item_id_not_harness_flag() {
    // bubble has store_item_id Some, server_pending_id None (native terminal down
    // ack shape) → path (2) drops on item_id. No "is_native" parameter exists.
}

#[test]
fn reconnected_with_gap_does_not_clear_pending_user() {
    // existing snapshot test must keep passing (spec §4 P3(b) rule 1)
}

#[test]
fn snapshot_pending_inputs_keeps_still_pending_bubbles() {
    // server_pending_id present in pending_inputs → keep
    // server_pending_id absent + store_item_id matches an item → drop
}
```

- [ ] **Step 2: Wire InputConsumed in folds**

Replace the ignore arm (`folds.rs:132`):

```rust
SessionEvent::InputConsumed {
    item_id,
    item_type: _,
    cleared_pending_id,
} => {
    let mut pending = std::mem::take(&mut state.pending_user);
    let changed = reconcile_pending_user(
        &mut pending,
        ReconcileSignal::Consumed {
            cleared_pending_id: cleared_pending_id.as_deref(),
            item_id,
            content: None, // live event payload not required for (1)/(2)
        },
    );
    state.pending_user = pending;
    if changed {
        smallvec![StreamUpdate::PendingUserChanged(state.pending_user.clone())]
    } else {
        smallvec![]
    }
}
```

- [ ] **Step 3: Wire snapshot/reconnect**

After `fold_snapshot` chrome folds, run snapshot reconcile using `snap.pending_inputs()` + trailing user message texts from `snap.items()` / `state.items`. Emit `PendingUserChanged` if the vec changed. On `Reconnected { gap: Some(g) } if g != 0`, do **not** clear pending_user (already true); reconcile runs when `SnapshotRestored` items land in the same bootstrap batch (actor already rebases after SnapshotRestored — I1).

- [ ] **Step 4: Actor integration test — Send → consumed clears bubble**

Script: mock API returns `item_id=msg_1`; push `InputConsumed { item_id: msg_1, cleared_pending_id: None }`; assert `PendingUserChanged` empty vec (or absent bubble).

- [ ] **Step 5: Gate + commit + review**

Run: `cargo fmt -p lens-core -p lens-client && cargo clippy -p lens-core -p lens-client --all-targets && cargo test -p lens-core && cargo test -p lens-client`

```bash
git add crates/lens-core/src/actor/reconcile.rs crates/lens-core/src/reduce crates/lens-core/src/actor
git commit -m "feat(lens-core): pending_user reconcile precedence (1) pending_id (2) item_id (3) content (D16)"
```

**MANDATORY cross-family review** of reconcile helpers + fold/snapshot wiring.

**External dependency:** none beyond Task 1's recorded finding. Reconnect path uses snapshot items already in-engine — **not** P3-3 `GET /items` pagination.

---

## Task 8: D18 Table A — `Disconnected{reason}` → park / stop + `reconcile_in_flight`

**REVIEW SEAM** (lifecycle). Map terminal stream disconnects to actor lifecycle. Introduce actor-owned transport + `reconcile_in_flight` (never persisted) so P3-3 `is_quiesced()` can require `transport == Connected && !reconcile_in_flight`.

**Files:**
- Create: `crates/lens-core/src/actor/transport.rs` (or section in `errors.rs`)
- Modify: `crates/lens-core/src/actor/runloop.rs` (event arm on `Disconnected` / `Reconnecting` / `Reconnected`)
- Modify: `crates/lens-core/src/reduce/mod.rs` if `Disconnected` needs to carry reason into a value-carrying delta for the UI banner
- Modify: `crates/lens-store/src/lib.rs` if a new `Disconnected(DisconnectReason)` / `TransportChanged` delta is added
- Test: actor lifecycle tests with scripted `ServerStreamEvent::Disconnected { reason }`

**Interfaces:**
```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActorTransport {
    Connected,
    Reconnecting,
    /// Recoverable terminal — actor + state resident; awaiting re-auth / user retry.
    Parked { reason: ParkReason },
    // Stopped is "thread exited" — not a resident state.
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParkReason {
    Unauthorized,
    SessionFailed,
    RetriesExhausted,
}

// Actor-owned (NOT on SessionState, NOT persisted):
// transport: ActorTransport,
// reconcile_in_flight: bool,  // true from Disconnected/Reconnecting until post-reconnect reconcile completes
```

**Table A (cite design §13.1 / spec D18):**

| `DisconnectReason` | action |
| --- | --- |
| `Unauthorized` / `SessionFailed` / `RetriesExhausted` | **park** — stop selecting on events (or drop event rx), keep command rx alive for Stop / future Resume (Resume wiring can be a stub command that is P3-3/§9); state resident |
| `Forbidden` | **stop** — break run-loop (registry removal is §9; emit outcome `StoppedRemoved`) |
| `NotFound` | **stop** — break; mark local tombstone intent via outcome `StoppedTombstone` (disk lifecycle write can be `SessionLifecycle::Deleted` if cheap; else outcome-only + TODO for P3-3 wake/tombstone schema) |

Also: on `Reconnecting` set `reconcile_in_flight = true`, `transport = Reconnecting`. On `Reconnected` after pending reconcile completes, clear `reconcile_in_flight`, `transport = Connected`. A **parked** session has `transport != Connected` ⇒ will not auto-sleep in P3-3.

- [ ] **Step 1: Failing lifecycle tests**

```rust
#[test]
fn disconnect_unauthorized_parks_actor_still_accepts_stop() {
    // send Disconnected { Unauthorized }; assert outcome Parked; send Stop; join ok
    // further events ignored / rx dropped
}

#[test]
fn disconnect_forbidden_stops_actor_thread() {
    // Disconnected { Forbidden } → thread exits; join without explicit Stop
}

#[test]
fn disconnect_not_found_stops_with_tombstone_outcome() { /* … */ }

#[test]
fn reconnecting_sets_reconcile_in_flight() {
    // expose via test-only introspection snapshot or outcome ring
}
```

- [ ] **Step 2: Value-carrying reason for UI (if needed)**

Today `StreamUpdate::Disconnected` is a marker (`update.rs:56`; reduce L154). If the banner needs the reason, change to `Disconnected(DisconnectReason)` and update `apply` (noop or store on a replica-visible field). **Prefer carrying the reason** — completeness rule / UI. Re-export `DisconnectReason` from lens-client (already public).

- [ ] **Step 3: Implement park/stop in run-loop**

On `Disconnected` after reduce/emit: match reason → set transport / push outcome / `break` or continue in parked mode (Select only on commands). Do not clear `pending_user` on park.

- [ ] **Step 4: Gate + commit + review**

Run: `cargo fmt -p lens-core -p lens-store && cargo clippy -p lens-core -p lens-store --all-targets && cargo test -p lens-core && cargo test -p lens-store`

```bash
git add crates/lens-core/src/actor crates/lens-core/src/reduce crates/lens-store/src/lib.rs
git commit -m "feat(lens-core): D18 Table A park/stop on Disconnected + reconcile_in_flight"
```

**MANDATORY cross-family review** of park/stop transitions.

---

## Task 9: D18 Table B — `ClientError` → command outcome + introspection ring

Map command/REST errors to **command outcomes** (never stream teardown). Give swallowed persist errors (`let _ =` in `persist_write_through`, runloop ≈`:207–223`) a home on a bounded introspection ring.

**Files:**
- Create: `crates/lens-core/src/actor/outcome.rs`
- Modify: `crates/lens-core/src/actor/runloop.rs` (persist + Send error paths from Task 6)
- Modify: design-doc §13.1 only if drift vs implementation (verify first — doc already has Table B ≈L1223–1238)
- Test: outcome mapping unit tests + persist-failure ring test

**Interfaces:**
```rust
pub enum ActorOutcome {
    Command(CommandOutcome),
    PersistError { where_: &'static str, message: String },
    Parked { reason: ParkReason },
    StoppedRemoved,
    StoppedTombstone,
    SummaryConsumerGone, // from M2
    /// Optimistic bubble that never persisted server-side (snapshot reconcile
    /// found it in neither `pending_inputs` nor items). Actor-diffed, not from
    /// the pure reducer. UX-polish only — see Task 7 Signal B "drop as lost".
    SendLost { lens_pending_id: String },
    // …
}

pub struct OutcomeRing { /* bounded VecDeque, e.g. 64; push never blocks emit */ }

// Table B mapping (command path only):
// Network on send → rollback (Task 6) + transient marker
// Auth{401} → prompt re-auth outcome; keep session
// Auth — note: ClientError::Auth { status } only; 403 may arrive as Server{403} —
//   map status explicitly
// NotFound → tombstone outcome (command-scoped; do not tear stream unless policy says so)
// Server{status,body} → 5xx transient / other-4xx denied
// ThreadSpawn → fatal at stream-open (actor never started) — document at spawn site
// Parse / ContractMismatch → decode-drift marker
// Drop phantom Ws row (no variant) — comment only
```

- [ ] **Step 1: Failing mapping tests**

```rust
#[test]
fn table_b_server_5xx_is_transient() {
    assert!(matches!(
        map_client_error(&ClientError::Server { status: 503, body: json!({}) }),
        Mapped::Transient { .. }
    ));
}

#[test]
fn table_b_server_4xx_is_denied() { /* 400/422 → Denied */ }

#[test]
fn persist_error_lands_on_ring_without_blocking_emit() {
    // TranscriptStore stub returns Err; actor still emits StreamUpdate; ring has PersistError
}
```

- [ ] **Step 2: Implement ring + replace `let _ =` persist**

```rust
if let Err(e) = stores.transcript.upsert_item(...) {
    outcomes.push(ActorOutcome::PersistError {
        where_: "transcript.upsert_item",
        message: e.to_string(),
    });
}
```

Wire Task 6's non-Network Send errors through `map_client_error`. Refine rollback policy per Table B (Network rolls back; denied ack rolls back; Auth keeps bubble held — match design §13.1).

- [ ] **Step 3: `ThreadSpawn` documentation at stream-open**

Wherever P3 attaches a stream (may be outside this crate today), add a comment + outcome path: `ClientError::ThreadSpawn` ⇒ session never becomes Active. If no attach site exists yet in lens-core, put the mapping fn + a unit test and a `TODO(attach)` reference in `outcome.rs`.

- [ ] **Step 4: Verify design-doc §13.1**

Read `docs/design/app-architecture-and-state-model.md` §13.1. If it already matches D18 (it appears to as of 2026-07-09), **no edit**. If drift (e.g. missing Network→rollback), amend in the same commit. Spec §7.1 listed the amendment — treat as "confirm done."

- [ ] **Step 5: Gate + commit + review**

Run: `cargo fmt -p lens-core && cargo clippy -p lens-core --all-targets && cargo test -p lens-core`

```bash
git add crates/lens-core/src/actor docs/design/app-architecture-and-state-model.md
git commit -m "feat(lens-core): D18 Table B ClientError command outcomes + persist introspection ring"
```

**MANDATORY cross-family review** of Table B mapping + ring (error/lifecycle seam).

---

## Task 10: End-to-end command-interleaving matrix + P3-2 gate

Close the P3 gate slice that belongs to command semantics (spec §4 P3 gate — command-interleaving matrix; sleep/wake stays P3-3).

**Files:**
- Test: `crates/lens-core/src/actor/runloop.rs` or `crates/lens-core/tests/command_matrix.rs`
- Docs: brief STATUS bullet only if the executor is updating STATUS this wave

**Matrix (scripted mock API + crossbeam events, no live server):**

| Case | Setup | Assert |
| --- | --- | --- |
| Send while idle | no open turn | bubble→ack stamp→consumed clears |
| Send while running | status Running + scratch open | same reconcile; no stream teardown on Network fail |
| Send while reconnecting | `reconcile_in_flight` true | bubble retained across Reconnected gap; clears on SnapshotRestored reconcile |
| Demote then Send | Summary mode | Send still works; summary consumer optional (M2) |
| Disconnect park then Stop | Unauthorized | park outcome; Stop joins |
| Stop during in-flight Send | mock API blocks then returns | actor joins within ~request-timeout, not indefinitely (risk 5a) |
| Persist fail during Send batch | stub Err | ring entry; deltas still emitted |

- [ ] **Step 1: Write the matrix tests (fail if gaps)**
- [ ] **Step 2: Fix any gaps found**
- [ ] **Step 3: Full P3-2 gate**

```bash
cargo fmt --check
cargo clippy -p lens-client -p lens-core -p lens-store --all-targets -- -D warnings
cargo test -p lens-client
cargo test -p lens-core
cargo test -p lens-store
# confirm generated.rs untouched
git diff --stat crates/lens-client/src/generated.rs
```

- [ ] **Step 4: Commit**

```bash
git add crates/lens-core
git commit -m "test(lens-core): P3-2 command-interleaving matrix (D16/D18)"
```

---

## Self-Review Checklist (run before handoff)

**1. Spec coverage:**
- D16 PendingUserMessage restructure → Task 4.
- D16 send + ack stamp + Network rollback → Task 6.
- D16 reconcile (1)(2)(3) live + reconnect → Task 7 (prereqs Tasks 2–3).
- D16 live-verify rider → Task 1 (**resolved** finding recorded; supersedes handoff "do live-verify first").
- D18 Table A park/stop + `reconcile_in_flight` → Task 8.
- D18 Table B + introspection ring → Task 9.
- P3-1 deferred M2/M1/Nit → Task 5.
- **Explicitly out of scope:** D17 sleep/wake/`is_quiesced` full predicate, D11 eviction, `GET /items` pagination (P3-3). This plan only **prepares** transport/`reconcile_in_flight` for P3-3.

**2. Placeholder scan:** Resume-from-park command is stubbed (park keeps command rx; no auto-restream) — intentional §9/P3-3. Tombstone disk schema may be outcome-only if `SessionLifecycle::Deleted` write is not yet wired — flag in Task 8, don't pretend it's full §3.1.

**3. Type consistency:** `PendingUserChanged` defined Task 4 ↔ apply Task 4 ↔ emit Tasks 6–7. `cleared_pending_id` Task 2 ↔ reconcile Task 7. `pending_inputs` Task 3 ↔ snapshot reconcile Task 7. `SessionCommand::Send` Task 6 ↔ matrix Task 10. `ActorTransport`/`reconcile_in_flight` Task 8 ↔ consumed later by P3-3 `is_quiesced()`.

**4. Value-carrying-completeness:** New fields `server_pending_id`/`store_item_id` live inside `PendingUserMessage` carried by `PendingUserChanged` (whole vec). `Disconnected(reason)` if changed to carry reason. No new `SessionState` scalar without a delta.

**5. Open verification points for the executor** (don't guess):
- Exact wire type of `pending_inputs[].content` (string vs content blocks) — Task 3; cite `generated.rs` / openapi.
- Whether `ClientError::Auth` distinguishes 403 vs using `Server { status: 403 }` — Task 9; read `lens-client` HTTP decode paths.
- `cx`/gpui untouched this plan — if a foreground outcome bridge is needed, it is a thin `async_channel` drain analogous to `spawn_apply_bridge` and can wait until a UI consumer exists (ring is actor-side first).

---

## Risks & open questions

1. **Native ⇏ `pending_id` gotcha** — highest logic risk. Any code that branches stamp/reconcile on `harness == *native*` will mis-handle terminal-down natives. Tests in Tasks 6–7 must include the `item_id`-only native-down shape explicitly.
2. **`InputConsumed` / `pending_inputs` were unplumbed** — handoff assumed ack ids were the main gap; live reconcile also needed lens-client field plumbing (Tasks 2–3). If openapi `content` shape is richer than `String`, content-match (3) may be weaker than ideal — acceptable because (3) is defensive-only post-live-verify.
3. **Handoff vs fresh finding** — handoff still lists live-verify as step 1; this plan treats it as resolved. If a reviewer rejects the uncommitted finding, re-run live verify before merging Task 1's comment as gospel.
4. **Park without Resume** — P3-2 parks but does not re-open `Sessions::stream`. UI/§9 must not assume auto-retry. Document on `Parked` outcome.
5. **Send serialization on actor thread** — `send_event` is blocking HTTP on the actor OS thread (same as intended P3 model). A slow POST stalls event drain for that session only; do not move POST to the gpui foreground. If latency becomes an issue, a later plan can add an out-of-band worker — out of scope here.

5a. **Blocked-`Select` responsiveness (Task 6 review-seam concern).** While the actor thread is parked inside `api.send_event(...)`, its `crossbeam::Select` services **nothing** — incoming stream events **and every command, including `Stop`**, queue until the POST returns or its HTTP timeout fires. So a `Stop` issued during a slow/hung send is not honored until the POST unblocks. This is bounded by the lens-client request timeout (verify it is set — a `None` timeout makes `Stop` hang indefinitely on a black-hole server). Acceptable for P3-2 (bounded, per-session), but: (a) the send call MUST run under a finite timeout, and (b) the interleaving matrix (Task 10) should include a "Stop during in-flight Send" case asserting the actor joins within ~timeout. If this latency proves unacceptable, the out-of-band worker in risk 5 is the fix — do not paper over it by moving Stop to a side channel that bypasses the single-consumer invariant.

8a. **`SessionApi` injection ripple (Task 6 wiring, enumerate before coding).** Adding `api: Arc<dyn SessionApi + Send>` to the actor changes the P3-1 spawn surface. Touch list: `spawn_actor` and `spawn_actor_dual` signatures (`actor/mod.rs`); the struct/tuple the run-loop closure captures; **every existing call site** — the walking-skeleton bridge test setup, all `#[gpui::test]`/actor unit tests that call `spawn_actor*`, and any lens-store integration harness. Mirror the P3-1 clock facts: the trait object is **`Send` but not necessarily `Sync`** (owned + moved to the OS thread), so it is `Box<dyn SessionApi + Send>` or `Arc<dyn SessionApi + Send + Sync>` — pick `Box` unless a handle needs to clone it (matches the `Box<dyn Clock + Send>` precedent). Tests inject a scripted mock; production injects a thin adapter over `lens_client::Sessions`. Grep `spawn_actor` before starting Task 6 to get the exact call-site count.
6. **Spec §4 P3(b) vs D16 supersession** — content/ordinal match remains as floor; do not delete it when implementing by-id paths.
7. **Design-doc §13.1 already amended** — Task 9 should verify, not blindly rewrite; avoid conflicting amendments.
8. **P3-3 contract** — keep `reconcile_in_flight` and `ActorTransport` off `SessionState` / SQLite so sleep/wake does not persist transient transport. A parked session must remain `transport != Connected`.
