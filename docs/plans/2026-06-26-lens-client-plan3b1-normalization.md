# lens-client Plan 3b-1 — SSE stream normalization (§7a) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the pure §7a normalization layer between the Plan 3a SSE parser and the consumer — `OutputItemDone` re-fire suppression and synthetic `ReasoningClosed` — so the state model receives a deduped, bracket-closed event stream.

**Architecture:** A stateful `Normalizer` sits in the reader thread (`stream/reader.rs`) between `parse_event` and `tx.send`, so the crate normalizes *before* handing events to the state model (typed-client.md §7a). It is a pure transform — `push(ServerStreamEvent) -> Vec<ServerStreamEvent>` (one event in yields zero, one, or two out) plus `flush()` at EOF — with no I/O, unit-tested against the golden fixtures from Plan 3a (`tests/fixtures/sse/happy_path.stream.sse`) and synthetic frames. Reconnect (§7) and the typed `items()`/snapshot reads it needs are **Plan 3b-2** — out of scope here.

**Tech Stack:** Rust (edition 2024), `std::collections::HashSet`. No new dependencies. Builds directly on the Plan 3a `stream::event` taxonomy and `stream::reader` bridge.

## Global Constraints

- **MANDATORY** No `serde_json::Value` exposed to consumers. The normalizer only moves and constructs already-typed `ServerStreamEvent` values; it introduces no new wire-parsing. (AGENTS.md typed-end-to-end.)
- **MANDATORY** The UI never panics the process — the normalizer is total: every event maps to a `Vec` of typed events, no `unwrap`/`panic` on event content. (AGENTS.md.)
- **MANDATORY** No I/O on the gpui foreground thread — normalization runs on the existing Plan 3a reader OS thread; the crate's public methods stay blocking `fn`. (`.agents/rust-ui.md`.)
- **MANDATORY** `generated.rs` stays untouched. Run `cargo clippy --all-targets` + `cargo fmt` clean before every commit.
- **MANDATORY** Ground-truth discipline — dedup keying is byte-grounded in `docs/spikes/captures/2026-06-26-sse/` (the `function_call` `in_progress`→`completed` pair, same `call_id`, differing `id`/`status`). `ReasoningClosed`'s text accumulation has **no bytes on this box** (claude-sdk emits no `reasoning_text.delta`); mark it `// NOT BYTE-VERIFIED (claude-sdk folds reasoning into output_text — re-capture at config-time)`.
- **Pinned semantics (decided 2026-06-26):** dedup **suppresses literal re-fires only** — a second event with the same `(kind, call_id, status)` — so the captured `in_progress`→`completed` progression is preserved as two events (it relaxes §7a's "exactly once" wording; Task 4 updates the doc). `ReasoningClosed` is built and flagged, not deferred.
- Pin: omnigent `0.3.0.dev0` (`36b2a11c`). The live `live_stream` test (`--features live-tests`) is updated in Task 4 to assert the normalized stream still flows.

**Scope of 3b-1.** The two byte-/recon-grounded §7a guarantees that need no new endpoints: (1) `OutputItemDone` re-fire suppression for `function_call`/`function_call_output`, (2) synthetic `ReasoningClosed`. **Out of scope (Plan 3b-2):** the `Reconnected { gap }` ordering guarantee and the entire §7 reconnect protocol (snapshot + `GET /items` merge-by-id + sequence-dedup), and the typed `Sessions::items()` + session snapshot reads that reconnect consumes.

---

### Task 1: `Normalizer` skeleton + pass-through transparency + `ReasoningClosed` variant

Stand up the stateful transform and the synthetic enum variant it will emit. This task delivers only the identity behavior: every event not yet special-cased passes through unchanged, in order. §7a's transparency guarantee ("no ordering changes beyond the dedup and synthetic events") is locked here as a test before any mutation logic lands.

**Files:**
- Create: `crates/lens-client/src/stream/normalize.rs`
- Modify: `crates/lens-client/src/stream/mod.rs` (add `pub(crate) mod normalize;`)
- Modify: `crates/lens-client/src/stream/event.rs` (add `ReasoningClosed` variant to `ResponseEvent`)

**Interfaces:**
- Consumes: `ServerStreamEvent`, `ResponseEvent`, `SessionEvent`, `Item` (Plan 3a, `stream::event`).
- Produces:
  - `pub(crate) struct Normalizer` with `fn default() -> Self`, `pub(crate) fn push(&mut self, ev: ServerStreamEvent) -> Vec<ServerStreamEvent>`, and `pub(crate) fn flush(&mut self) -> Vec<ServerStreamEvent>`.
  - New variant `ResponseEvent::ReasoningClosed { full_text: String, summary_text: String }` (synthetic; never produced by `parse_event`, only by the `Normalizer` in Task 3).

- [ ] **Step 1: Add the `ReasoningClosed` variant**

In `crates/lens-client/src/stream/event.rs`, in the `pub enum ResponseEvent` block, add immediately after the `ReasoningStarted,` variant:

```rust
    /// SYNTHETIC (typed-client.md §7a) — emitted by `stream::normalize::Normalizer`,
    /// never by `parse_event`. The SSE stream has no reasoning-end frame; the crate
    /// closes the bracket on the first `OutputTextDelta`/`Completed` after
    /// `ReasoningStarted`. `full_text`/`summary_text` accumulate the reasoning deltas
    /// so the renderer need not re-accumulate.
    /// NOT BYTE-VERIFIED (claude-sdk folds reasoning into output_text — re-capture at config-time)
    ReasoningClosed {
        full_text: String,
        summary_text: String,
    },
```

- [ ] **Step 2: Write the failing test**

Create `crates/lens-client/src/stream/normalize.rs`:

```rust
//! §7a normalization: the pure, stateful transform between the SSE parser and
//! the consumer. Two guarantees, nothing more (typed-client.md §7a):
//!   1. `OutputItemDone` re-fire suppression — a second event with the same
//!      `(kind, call_id, status)` is dropped (claude-sdk double-fires). The
//!      captured `in_progress`→`completed` progression is preserved (status
//!      differs), so this is NOT a collapse to one event per call_id.
//!   2. Synthetic `ReasoningClosed` — the stream has no reasoning-end frame;
//!      the bracket closes on the first `OutputTextDelta`/`Completed` after a
//!      `ReasoningStarted`.
//! Everything else passes through unchanged, in order. No text accumulation,
//! call/result pairing, or reordering beyond the above — that is the state
//! model's job. Lives on the Plan 3a reader thread; never blocks the foreground.

use super::event::{ResponseEvent, ServerStreamEvent};

#[cfg(test)]
mod tests {
    use super::super::event::{Item, SessionEvent, SessionStatusValue};
    use super::*;

    fn status(s: SessionStatusValue) -> ServerStreamEvent {
        ServerStreamEvent::Session(SessionEvent::Status { status: s, response_id: None })
    }

    #[test]
    fn unrelated_events_pass_through_unchanged_and_in_order() {
        let mut n = Normalizer::default();
        let a = status(SessionStatusValue::Running);
        let b = ServerStreamEvent::Response(ResponseEvent::InProgress);
        let c = ServerStreamEvent::Unknown { event_type: "x.y".into() };
        let mut out = Vec::new();
        out.extend(n.push(a.clone()));
        out.extend(n.push(b.clone()));
        out.extend(n.push(c.clone()));
        assert_eq!(out, vec![a, b, c]);
    }

    #[test]
    fn a_lone_output_item_passes_through() {
        let mut n = Normalizer::default();
        let ev = ServerStreamEvent::Response(ResponseEvent::OutputItemDone {
            item: Item::FunctionCallOutput {
                id: "fco_1".into(),
                call_id: "toolu_1".into(),
                output: "ok".into(),
            },
        });
        assert_eq!(n.push(ev.clone()), vec![ev]);
    }

    #[test]
    fn flush_on_empty_state_yields_nothing() {
        let mut n = Normalizer::default();
        assert!(n.flush().is_empty());
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p lens-client --lib stream::normalize 2>&1 | head -20`
Expected: FAIL — `cannot find struct Normalizer` / `cannot find function default`.

- [ ] **Step 4: Implement the skeleton**

Add to `crates/lens-client/src/stream/normalize.rs`, above the `#[cfg(test)]` block:

```rust
use std::collections::HashSet;

#[derive(Default)]
pub(crate) struct Normalizer {
    /// Keys of `OutputItemDone` items already emitted: `(kind, call_id, status)`.
    /// A repeat with an identical key is a literal re-fire and is dropped.
    seen_items: HashSet<(&'static str, String, String)>,
    /// `Some` while a reasoning bracket is open (between `ReasoningStarted` and
    /// its synthetic close). Accumulates the reasoning/summary deltas.
    reasoning: Option<ReasoningAccum>,
}

#[derive(Default)]
struct ReasoningAccum {
    full_text: String,
    summary_text: String,
}

impl Normalizer {
    /// Transform one parsed event into zero, one, or two normalized events.
    /// Total — never panics on event content. Task 2 adds suppression; Task 3
    /// adds the reasoning close. For now, identity.
    pub(crate) fn push(&mut self, ev: ServerStreamEvent) -> Vec<ServerStreamEvent> {
        vec![ev]
    }

    /// Flush any open synthetic state at stream EOF. Task 3 emits a trailing
    /// `ReasoningClosed` for a reasoning bracket the stream ended without closing.
    pub(crate) fn flush(&mut self) -> Vec<ServerStreamEvent> {
        Vec::new()
    }
}
```

Add to `crates/lens-client/src/stream/mod.rs`, next to the other `mod` lines (keep it crate-private — the consumer never names the normalizer):

```rust
pub(crate) mod normalize;
```

> The `seen_items` / `reasoning` fields are unused until Tasks 2–3. Add `#[allow(dead_code)]` on the two struct fields *only if* clippy blocks the commit; remove the allow in Task 2/3 when they are read. Prefer landing Tasks 1–3 together if the dead-code warning is noisy.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p lens-client --lib stream::normalize`
Expected: PASS (3 tests).

- [ ] **Step 6: Lint + commit**

```bash
cargo fmt -p lens-client && cargo clippy -p lens-client --all-targets -- -D warnings
git add crates/lens-client/src/stream/normalize.rs crates/lens-client/src/stream/mod.rs crates/lens-client/src/stream/event.rs
git commit -m "feat(lens-client): Normalizer skeleton + ReasoningClosed variant"
```

---

### Task 2: `OutputItemDone` re-fire suppression

Implement §7a dedup with the pinned semantics: suppress a second `OutputItemDone` whose `(kind, call_id, status)` was already emitted. The captured `function_call` `in_progress`→`completed` pair (same `call_id`, differing `status`) is **preserved as two events**; a literal duplicate is dropped. `function_call_output` has no `status` field on `Item` (Plan 3a modeled `{id, call_id, output}`), so it keys on `(kind, call_id, "")`.

**Files:**
- Modify: `crates/lens-client/src/stream/normalize.rs`

**Interfaces:**
- Consumes: `Item::FunctionCall { call_id, status, .. }`, `Item::FunctionCallOutput { call_id, .. }` (Plan 3a).
- Produces: no signature change — `push` now returns `vec![]` for a suppressed re-fire.

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` in `normalize.rs`:

```rust
    fn fn_call(call_id: &str, status: &str, item_id: &str) -> ServerStreamEvent {
        ServerStreamEvent::Response(ResponseEvent::OutputItemDone {
            item: Item::FunctionCall {
                id: item_id.into(),
                call_id: call_id.into(),
                name: "sys_os_shell".into(),
                arguments: "{}".into(),
                status: status.into(),
                agent: None,
            },
        })
    }
    fn fn_out(call_id: &str, item_id: &str) -> ServerStreamEvent {
        ServerStreamEvent::Response(ResponseEvent::OutputItemDone {
            item: Item::FunctionCallOutput {
                id: item_id.into(),
                call_id: call_id.into(),
                output: "ok".into(),
            },
        })
    }

    #[test]
    fn literal_function_call_refire_is_suppressed() {
        let mut n = Normalizer::default();
        let first = fn_call("toolu_1", "completed", "fc_a");
        assert_eq!(n.push(first.clone()), vec![first]);
        // Identical (kind, call_id, status) — dropped, even with a different item id.
        assert!(n.push(fn_call("toolu_1", "completed", "fc_b")).is_empty());
    }

    #[test]
    fn in_progress_then_completed_is_preserved_as_two_events() {
        // Byte-grounded: the captured happy-path turn fires the same call_id once
        // in_progress, once completed (differing status) — both survive.
        let mut n = Normalizer::default();
        let ip = fn_call("toolu_1", "in_progress", "fc_a");
        let done = fn_call("toolu_1", "completed", "fc_b");
        assert_eq!(n.push(ip.clone()), vec![ip]);
        assert_eq!(n.push(done.clone()), vec![done]);
    }

    #[test]
    fn literal_function_call_output_refire_is_suppressed() {
        let mut n = Normalizer::default();
        let first = fn_out("toolu_1", "fco_a");
        assert_eq!(n.push(first.clone()), vec![first]);
        assert!(n.push(fn_out("toolu_1", "fco_b")).is_empty());
    }

    #[test]
    fn distinct_call_ids_are_independent() {
        let mut n = Normalizer::default();
        assert_eq!(n.push(fn_call("toolu_1", "completed", "a")).len(), 1);
        assert_eq!(n.push(fn_call("toolu_2", "completed", "b")).len(), 1);
    }

    #[test]
    fn non_dedup_items_are_never_suppressed() {
        // A message item has no call_id key — two messages both pass through.
        let mut n = Normalizer::default();
        let msg = ServerStreamEvent::Response(ResponseEvent::OutputItemDone {
            item: Item::Message { id: "m1".into(), role: "assistant".into(), content: vec![] },
        });
        assert_eq!(n.push(msg.clone()), vec![msg.clone()]);
        assert_eq!(n.push(msg.clone()), vec![msg]);
    }

    #[test]
    fn happy_path_fixture_preserves_both_function_call_events() {
        // The fixture has the same call_id as in_progress AND completed function_call,
        // plus one function_call_output — all three survive (no literal re-fire).
        let bytes = include_bytes!("../../tests/fixtures/sse/happy_path.stream.sse");
        let mut p = super::super::sse::SseParser::default();
        let mut frames = p.push(bytes);
        frames.extend(p.finish());
        let mut n = Normalizer::default();
        let mut out = Vec::new();
        for f in &frames {
            out.extend(n.push(super::super::event::parse_event(f)));
        }
        let fc = out.iter().filter(|e| matches!(e,
            ServerStreamEvent::Response(ResponseEvent::OutputItemDone { item: Item::FunctionCall { .. } }))).count();
        let fco = out.iter().filter(|e| matches!(e,
            ServerStreamEvent::Response(ResponseEvent::OutputItemDone { item: Item::FunctionCallOutput { .. } }))).count();
        assert_eq!(fc, 2, "in_progress + completed function_call both preserved");
        assert_eq!(fco, 1, "single function_call_output preserved");
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p lens-client --lib stream::normalize 2>&1 | head -25`
Expected: FAIL — `literal_function_call_refire_is_suppressed` (the skeleton passes the duplicate through).

> If `happy_path_fixture_preserves_both_function_call_events` reports `fc != 2`, read the fixture (`crates/lens-client/tests/fixtures/sse/happy_path.stream.sse`) and confirm the actual `function_call` count — do not change the suppression to fit a guessed number; the fixture is ground truth.

- [ ] **Step 3: Implement suppression in `push`**

Replace the body of `Normalizer::push` in `normalize.rs`:

```rust
    pub(crate) fn push(&mut self, ev: ServerStreamEvent) -> Vec<ServerStreamEvent> {
        if let ServerStreamEvent::Response(ResponseEvent::OutputItemDone { item }) = &ev {
            use super::event::Item;
            let key = match item {
                Item::FunctionCall { call_id, status, .. } => {
                    Some(("function_call", call_id.clone(), status.clone()))
                }
                // `Item::FunctionCallOutput` carries no status field (Plan 3a) —
                // key on call_id alone via a constant status slot.
                Item::FunctionCallOutput { call_id, .. } => {
                    Some(("function_call_output", call_id.clone(), String::new()))
                }
                _ => None, // messages/errors/other have no dedup key
            };
            if let Some(key) = key {
                if !self.seen_items.insert(key) {
                    return Vec::new(); // literal re-fire — drop
                }
            }
        }
        vec![ev]
    }
```

(If Task 1 added `#[allow(dead_code)]` on `seen_items`, remove it now — the field is read.)

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p lens-client --lib stream::normalize`
Expected: PASS (all Task 1 + Task 2 tests).

- [ ] **Step 5: Lint + commit**

```bash
cargo fmt -p lens-client && cargo clippy -p lens-client --all-targets -- -D warnings
git add crates/lens-client/src/stream/normalize.rs
git commit -m "feat(lens-client): OutputItemDone re-fire suppression (preserves in_progress→completed)"
```

---

### Task 3: Synthetic `ReasoningClosed` + delta accumulation

Emit `ReasoningClosed { full_text, summary_text }` when the first `OutputTextDelta` or `Completed` arrives after a `ReasoningStarted`, carrying the accumulated `reasoning_text`/`reasoning_summary_text` deltas. The close event precedes the triggering event in the output. `ReasoningStarted` and the deltas still pass through (the renderer streams them live); the crate only *adds* the synthetic close. The trigger is byte-grounded (the happy-path fixture has `reasoning.started` immediately followed by `output_text.delta`); the text accumulation is not (no delta frames on this box) — flagged accordingly.

**Files:**
- Modify: `crates/lens-client/src/stream/normalize.rs`

**Interfaces:**
- Consumes: `ResponseEvent::ReasoningStarted`, `ReasoningTextDelta { delta }`, `ReasoningSummaryTextDelta { delta }`, `OutputTextDelta { .. }`, `Completed` (Plan 3a); emits `ResponseEvent::ReasoningClosed { full_text, summary_text }` (Task 1).
- Produces: no signature change — `push` now returns two events (`[ReasoningClosed, trigger]`) when closing a bracket; `flush` returns `[ReasoningClosed]` for an unclosed bracket at EOF.

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` in `normalize.rs`:

```rust
    fn rdelta(d: &str) -> ServerStreamEvent {
        ServerStreamEvent::Response(ResponseEvent::ReasoningTextDelta { delta: d.into() })
    }
    fn sdelta(d: &str) -> ServerStreamEvent {
        ServerStreamEvent::Response(ResponseEvent::ReasoningSummaryTextDelta { delta: d.into() })
    }
    fn text_delta() -> ServerStreamEvent {
        ServerStreamEvent::Response(ResponseEvent::OutputTextDelta {
            delta: "Hi".into(), message_id: None, index: None, last: None,
        })
    }

    #[test]
    fn reasoning_closes_on_first_output_text_delta_with_accumulated_text() {
        let mut n = Normalizer::default();
        assert_eq!(n.push(ServerStreamEvent::Response(ResponseEvent::ReasoningStarted)),
                   vec![ServerStreamEvent::Response(ResponseEvent::ReasoningStarted)]);
        assert_eq!(n.push(rdelta("be")), vec![rdelta("be")]); // passes through + accumulates
        assert_eq!(n.push(rdelta("cause")), vec![rdelta("cause")]);
        assert_eq!(n.push(sdelta("sum")), vec![sdelta("sum")]);
        // First output_text.delta closes the bracket: [ReasoningClosed, the delta].
        let out = n.push(text_delta());
        assert_eq!(out, vec![
            ServerStreamEvent::Response(ResponseEvent::ReasoningClosed {
                full_text: "because".into(), summary_text: "sum".into(),
            }),
            text_delta(),
        ]);
    }

    #[test]
    fn reasoning_closes_on_completed() {
        let mut n = Normalizer::default();
        n.push(ServerStreamEvent::Response(ResponseEvent::ReasoningStarted));
        let completed = ServerStreamEvent::Response(ResponseEvent::Completed);
        let out = n.push(completed.clone());
        assert_eq!(out, vec![
            ServerStreamEvent::Response(ResponseEvent::ReasoningClosed {
                full_text: String::new(), summary_text: String::new(),
            }),
            completed,
        ]);
    }

    #[test]
    fn only_one_close_per_reasoning_bracket() {
        let mut n = Normalizer::default();
        n.push(ServerStreamEvent::Response(ResponseEvent::ReasoningStarted));
        assert_eq!(n.push(text_delta()).len(), 2); // close + delta
        assert_eq!(n.push(text_delta()), vec![text_delta()]); // no second close
        assert_eq!(n.push(ServerStreamEvent::Response(ResponseEvent::Completed)),
                   vec![ServerStreamEvent::Response(ResponseEvent::Completed)]);
    }

    #[test]
    fn output_text_delta_without_open_reasoning_is_untouched() {
        let mut n = Normalizer::default();
        assert_eq!(n.push(text_delta()), vec![text_delta()]);
    }

    #[test]
    fn flush_closes_a_dangling_reasoning_bracket() {
        let mut n = Normalizer::default();
        n.push(ServerStreamEvent::Response(ResponseEvent::ReasoningStarted));
        n.push(rdelta("x"));
        assert_eq!(n.flush(), vec![
            ServerStreamEvent::Response(ResponseEvent::ReasoningClosed {
                full_text: "x".into(), summary_text: String::new(),
            }),
        ]);
        assert!(n.flush().is_empty()); // idempotent — already closed
    }

    #[test]
    fn happy_path_fixture_synthesizes_one_reasoning_closed() {
        // reasoning.started (line 22) is immediately followed by output_text.delta
        // (line 25): the trigger is byte-grounded. full_text is empty (no delta
        // frames on claude-sdk). Exactly one ReasoningClosed for the turn.
        let bytes = include_bytes!("../../tests/fixtures/sse/happy_path.stream.sse");
        let mut p = super::super::sse::SseParser::default();
        let mut frames = p.push(bytes);
        frames.extend(p.finish());
        let mut n = Normalizer::default();
        let mut out = Vec::new();
        for f in &frames {
            out.extend(n.push(super::super::event::parse_event(f)));
        }
        out.extend(n.flush());
        let closes = out.iter().filter(|e| matches!(e,
            ServerStreamEvent::Response(ResponseEvent::ReasoningClosed { .. }))).count();
        assert_eq!(closes, 1);
        // The close lands before the first output_text.delta.
        let close_idx = out.iter().position(|e| matches!(e,
            ServerStreamEvent::Response(ResponseEvent::ReasoningClosed { .. }))).unwrap();
        let first_text_idx = out.iter().position(|e| matches!(e,
            ServerStreamEvent::Response(ResponseEvent::OutputTextDelta { .. }))).unwrap();
        assert!(close_idx < first_text_idx);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p lens-client --lib stream::normalize 2>&1 | head -25`
Expected: FAIL — `reasoning_closes_on_first_output_text_delta_with_accumulated_text` (skeleton never emits `ReasoningClosed`).

- [ ] **Step 3: Implement the reasoning bracket**

In `Normalizer::push` (`normalize.rs`), add reasoning handling. The full method, with the Task 2 suppression intact:

```rust
    pub(crate) fn push(&mut self, ev: ServerStreamEvent) -> Vec<ServerStreamEvent> {
        use super::event::Item;
        match &ev {
            // ── reasoning bracket ────────────────────────────────────────────
            ServerStreamEvent::Response(ResponseEvent::ReasoningStarted) => {
                self.reasoning = Some(ReasoningAccum::default());
                return vec![ev];
            }
            ServerStreamEvent::Response(ResponseEvent::ReasoningTextDelta { delta }) => {
                if let Some(acc) = self.reasoning.as_mut() {
                    acc.full_text.push_str(delta);
                }
                return vec![ev];
            }
            ServerStreamEvent::Response(ResponseEvent::ReasoningSummaryTextDelta { delta }) => {
                if let Some(acc) = self.reasoning.as_mut() {
                    acc.summary_text.push_str(delta);
                }
                return vec![ev];
            }
            ServerStreamEvent::Response(ResponseEvent::OutputTextDelta { .. })
            | ServerStreamEvent::Response(ResponseEvent::Completed) => {
                if let Some(close) = self.close_reasoning() {
                    return vec![close, ev];
                }
                return vec![ev];
            }
            // ── OutputItemDone re-fire suppression (Task 2) ──────────────────
            ServerStreamEvent::Response(ResponseEvent::OutputItemDone { item }) => {
                let key = match item {
                    Item::FunctionCall { call_id, status, .. } => {
                        Some(("function_call", call_id.clone(), status.clone()))
                    }
                    Item::FunctionCallOutput { call_id, .. } => {
                        Some(("function_call_output", call_id.clone(), String::new()))
                    }
                    _ => None,
                };
                if let Some(key) = key {
                    if !self.seen_items.insert(key) {
                        return Vec::new();
                    }
                }
                vec![ev]
            }
            _ => vec![ev],
        }
    }

    /// Take the open reasoning bracket (if any) and build its synthetic close.
    fn close_reasoning(&mut self) -> Option<ServerStreamEvent> {
        let acc = self.reasoning.take()?;
        Some(ServerStreamEvent::Response(ResponseEvent::ReasoningClosed {
            full_text: acc.full_text,
            summary_text: acc.summary_text,
        }))
    }
```

Replace the body of `Normalizer::flush`:

```rust
    pub(crate) fn flush(&mut self) -> Vec<ServerStreamEvent> {
        self.close_reasoning().into_iter().collect()
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p lens-client --lib stream::normalize`
Expected: PASS (all Task 1–3 tests).

- [ ] **Step 5: Full crate test sweep**

Run: `cargo test -p lens-client --lib 2>&1 | tail -5`
Expected: PASS — the Plan 3a suite (85 tests) plus the new normalize tests; no regressions.

- [ ] **Step 6: Lint + commit**

```bash
cargo fmt -p lens-client && cargo clippy -p lens-client --all-targets -- -D warnings
git add crates/lens-client/src/stream/normalize.rs
git commit -m "feat(lens-client): synthetic ReasoningClosed + delta accumulation"
```

---

### Task 4: Wire the `Normalizer` into the reader thread + update §7a doc + live test

Thread every parsed event through the `Normalizer` before `tx.send`, and flush at EOF — so `EventStream` consumers receive the normalized stream (§7a: the crate normalizes before the state model sees events). Update typed-client.md §7a to match the pinned semantics (this is the ground-truth-discipline step). Extend the live test to assert the normalized stream still flows.

**Files:**
- Modify: `crates/lens-client/src/stream/reader.rs` (`run`)
- Modify: `docs/design/typed-client.md` (§7a bullets)
- Modify: `crates/lens-client/tests/live_stream.rs` (assertion note)

**Interfaces:**
- Consumes: `Normalizer` (Tasks 1–3), `parse_event` (Plan 3a), `SseParser` (Plan 3a).
- Produces: no public signature change — `EventStream::recv`/`try_recv` now yield normalized events.

- [ ] **Step 1: Thread the normalizer through `run`**

In `crates/lens-client/src/stream/reader.rs`, update the imports and `run` function. Replace the `use super::...` lines and the whole `fn run`:

```rust
use super::event::parse_event;
use super::event::ServerStreamEvent;
use super::normalize::Normalizer;
use super::sse::SseParser;

// ... EventStream unchanged ...

fn run(mut resp: reqwest::blocking::Response, tx: mpsc::Sender<ServerStreamEvent>) {
    let mut parser = SseParser::default();
    let mut normalizer = Normalizer::default();
    let mut buf = [0u8; 8192];
    loop {
        match resp.read(&mut buf) {
            Ok(0) => break, // server closed the stream
            Ok(n) => {
                for frame in parser.push(&buf[..n]) {
                    for ev in normalizer.push(parse_event(&frame)) {
                        if tx.send(ev).is_err() {
                            return; // consumer dropped EventStream — stop reading
                        }
                    }
                }
            }
            Err(_) => break, // network error: close the channel (Plan 3b-2 reconnects)
        }
    }
    for frame in parser.finish() {
        for ev in normalizer.push(parse_event(&frame)) {
            let _ = tx.send(ev);
        }
    }
    // Close any reasoning bracket the stream ended without closing (§7a).
    for ev in normalizer.flush() {
        let _ = tx.send(ev);
    }
}
```

> Keep the existing `use super::event::{ServerStreamEvent, parse_event};` style if `reader.rs` already groups them — match the file's current import grouping rather than splitting as shown. The functional change is: construct a `Normalizer`, route each `parse_event` result through `normalizer.push`, and `normalizer.flush()` after `parser.finish()`.

- [ ] **Step 2: Build + run the full suite**

Run: `cargo test -p lens-client --lib 2>&1 | tail -5`
Expected: PASS — no regressions; reader still compiles with the normalizer in the loop.

- [ ] **Step 3: Update §7a in the design doc**

In `docs/design/typed-client.md`, replace the first three bullets of **§7a Normalization guarantees** (the `ToolCall` dedup, `ToolResult` dedup, and `ReasoningClosed` bullets) with the pinned semantics:

```markdown
- **`OutputItemDone` re-fire suppression** — a second `output_item.done` whose
  `(kind, call_id, status)` was already emitted is dropped (claude-sdk's MCP path
  double-fires identical items). This is **literal-duplicate suppression, not a
  collapse to one event per `call_id`**: the captured `function_call`
  `in_progress`→`completed` progression (same `call_id`, differing `status`) is
  preserved as two events so the state model keeps the "tool starting" signal.
  (Earlier drafts said "each `call_id` appears exactly once"; relaxed 2026-06-26
  per the golden-SSE bytes — see `docs/spikes/2026-06-26-golden-sse-capture.md`.)
- **`ReasoningClosed` synthesis (synthetic event)** — the SSE stream has no
  explicit reasoning-end event. The crate emits `ReasoningClosed` when the first
  `OutputTextDelta` or `Completed` arrives after a `ReasoningStarted`, carrying
  the accumulated `full_text` + `summary_text` so the renderer need not
  re-accumulate. The state model treats reasoning as a proper open/close bracket
  without tracking implicit state. **NOT byte-verified**: claude-sdk (the only
  harness on the capture box) folds reasoning into `output_text` and emits no
  `reasoning_text.delta` frames, so the close *trigger* is byte-grounded but the
  text accumulation is schema-derived — re-capture at config-time.
```

(Leave the `Reconnected { gap }` bullet and the "No text accumulation…" closing bullet unchanged — `Reconnected` is Plan 3b-2.)

- [ ] **Step 4: Note the normalized stream in the live test**

In `crates/lens-client/tests/live_stream.rs`, add a comment above the drain loop documenting that the stream is now normalized, and (optionally) count `ReasoningClosed` for visibility. Minimal change — add after the `let mut saw_unknown` line:

```rust
    // Plan 3b-1: the stream is now normalized (re-fire dedup + synthetic
    // ReasoningClosed). Surface any ReasoningClosed for visibility; claude-sdk
    // folds reasoning into output_text, so this is typically empty-text.
    let mut saw_reasoning_closed = 0usize;
```

And in the `match stream.recv()` arms, add before the catch-all `Some(_) => {}`:

```rust
            Some(ServerStreamEvent::Response(ResponseEvent::ReasoningClosed { .. })) => {
                saw_reasoning_closed += 1;
            }
```

And after the loop, alongside the existing `saw_unknown` reporting:

```rust
    eprintln!("normalized stream: {saw_reasoning_closed} ReasoningClosed event(s)");
```

> This requires `ResponseEvent` in scope — the test already imports `use lens_client::stream::{ResponseEvent, ServerStreamEvent};`. If it only imports `ServerStreamEvent`, add `ResponseEvent` to that `use`.

- [ ] **Step 5: Run the live test against the pinned server**

Warm a session, then:

```bash
omnigent run --harness claude-sdk --server http://127.0.0.1:6767 -p "hi" </dev/null
# grab the new idle claude-sdk session id (GET /v1/sessions)
LENS_OMNIGENT_URL=http://127.0.0.1:6767 LENS_OMNIGENT_SESSION_ID=<conv_…> \
  cargo test -p lens-client --features live-tests --test live_stream -- --nocapture
```

Expected: PASS — `response.completed` still observed through the normalized stream; prints the `ReasoningClosed` count and any unmodeled events. (If no live server is available, record that Step 5 was not run — do not claim it passed.)

- [ ] **Step 6: Lint + commit**

```bash
cargo fmt -p lens-client && cargo clippy -p lens-client --all-targets -- -D warnings
git add crates/lens-client/src/stream/reader.rs docs/design/typed-client.md crates/lens-client/tests/live_stream.rs
git commit -m "feat(lens-client): normalize SSE in reader thread + §7a doc update"
```

---

## Out of scope for 3b-1 (Plan 3b-2)

- **No-replay reconnect (§7):** disconnect detection at the `Err(_)`/`Ok(0)` seam, backoff retry (~7s), snapshot restore (bucket B), `GET /items` merge-by-id (bucket A), sequence-dedup of the live overlap (bucket C), stop-immediately on 401/403/404/`status=failed`.
- **`Reconnected { gap }` synthetic event** + the "precedes all replayed history" ordering guarantee (§7a).
- **Typed `Sessions::items()`** (→ the `Item` union) and the **typed session snapshot** (`GET /v1/sessions/{id}` with `include_items`/`include_liveness`) that reconnect consumes — both deferred from 2a–2e, folded into 3b-2.
- **Deferred 3a Minors** that belong with the reconnect/reader rework: `try_recv` idle-vs-closed liveness signal; reqwest read-timeout vs reader-thread leak on a hard hang. (The redundant-`serde(default)`-on-`Option` minor is a cleanup that can ride either plan.)

## Self-Review notes

- **Spec coverage (§7a):** `ToolCall`/`ToolResult` dedup → Task 2 (pinned to literal-re-fire suppression; the `in_progress`→`completed` preservation is byte-grounded); `ReasoningClosed` synthesis → Tasks 1+3 (flagged not-byte-verified); transparency ("no ordering changes beyond the above") → Task 1 pass-through test; "the crate normalizes before handing to the state model" → Task 4 reader wiring. `Reconnected { gap }` → explicitly deferred to 3b-2.
- **No-Value rule:** the normalizer constructs/forwards only typed `ServerStreamEvent`s; no `serde_json::Value`, no new wire parsing. ✓
- **Never-panic:** `push`/`flush` are total `match`es over typed events; no `unwrap` on content; `HashSet::insert` and `Option::take` cannot panic. ✓
- **Type consistency:** `Normalizer::{push,flush}`, `ReasoningAccum`, `ReasoningClosed { full_text, summary_text }`, the `(kind, call_id, status)` key, and `Item::FunctionCall{call_id,status}` / `Item::FunctionCallOutput{call_id}` field names match Plan 3a's `event.rs` exactly. ✓
- **Process:** temporal/stateful logic = composer-2.5's weak spot (`composer-delegation-profile`) → **per-task cross-family review** (gpt-5.5/gemini-3.5), per STATUS's note that review returns for Plan 3. Mind Cursor-credit cost (`review-spend-policy`).
