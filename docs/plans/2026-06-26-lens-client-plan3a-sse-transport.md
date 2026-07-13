# lens-client Plan 3a — SSE transport + typed event taxonomy Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stand up the live SSE event stream in `lens-client` — a pure SSE frame parser, the typed `ServerStreamEvent` taxonomy modeled from captured bytes, and a blocking reader-thread → `mpsc` → poller bridge exposed as `Sessions::stream()`.

**Architecture:** A dedicated blocking OS thread holds the `reqwest::blocking` response body, runs the pure SSE frame parser over the byte stream, deserializes each frame into a typed `ServerStreamEvent`, and pushes it down a `std::sync::mpsc` channel. `EventStream` is the consumer handle (`recv`/`try_recv`). No async runtime (typed-client-implementation.md D2). The frame parser and the event-deserialization layer are **pure functions over bytes**, unit-tested against golden fixtures captured from the live pinned server — the same "pure core + serverless tests" pattern as `http.rs`. Forward-compatibility on the churning dev0 contract is built in: any unmodeled event type deserializes to a non-panicking `Unknown` variant.

**Tech Stack:** Rust (edition 2024), `reqwest::blocking` (already a dep), `serde`/`serde_json`, `std::thread` + `std::sync::mpsc`. No new dependencies.

## Global Constraints

- **MANDATORY** No `serde_json::Value` exposed to consumers. The escape-hatch `Unknown` variant carries only `event_type: String` publicly; any retained raw payload is private (introspection-only). (AGENTS.md typed-end-to-end.)
- **MANDATORY** The UI never panics the process — unmodeled/garbled events become typed values (`Unknown`), never `unwrap`/`panic`. (AGENTS.md.)
- **MANDATORY** No I/O on the gpui foreground thread — all stream reads on a dedicated OS thread. The crate's public methods are blocking `fn`, not `async fn` (typed-client-implementation.md D2). (`.agents/rust-ui.md`.)
- **MANDATORY** `generated.rs` stays untouched — hand-write the event types (the SSE event schemas are under-modeled in openapi; model from bytes per typed-client.md §9). Run `cargo clippy --all-targets` + `cargo fmt` clean before every commit.
- **MANDATORY** Ground-truth discipline — event shapes come from the captured bytes in `docs/spikes/captures/2026-06-26-sse/`, not memory. Where a family is uncaptured (env-blocked, per the spike), model from the openapi schema and **mark it `// SCHEMA-DERIVED (not byte-verified)`**.
- Pin: omnigent `0.3.0.dev0` (`36b2a11c`). Live tests are `#[cfg(feature = "live-tests")]`, run via `LENS_OMNIGENT_URL=… cargo test -p lens-client --features live-tests`.

**Scope of 3a (byte-grounded foundation).** This plan models and golden-tests the event families we captured from bytes (status, usage, presence, heartbeat, resource.created, input.consumed, changed_files.invalidated, interrupted; response in_progress/completed/output_text.delta/reasoning.started/output_item.done; the Item union for message/function_call/function_call_output/error). The **normalization layer** (§7a dedup, synthetic `ReasoningClosed`) and the **no-replay reconnect protocol** (§7 three-bucket) are **Plan 3b**. The remaining schema-derived variants (reasoning deltas, elicitation, compaction, child_session, terminal, turn events, response failed/incomplete/cancelled) are added with schema-shaped tests in 3a Task 6 and re-verified from bytes at config-time capture.

---

### Task 1: SSE frame parser (pure)

Parse the `event: …\ndata: …\n\n` wire framing into raw `(event, data)` frames. Pure over a byte buffer so it is unit-testable against the captured `.sse` fixtures with no live server. The streamer (Task 5) feeds it incrementally; the parser must handle frame boundaries split across reads.

**Files:**
- Create: `crates/lens-client/src/stream/mod.rs`
- Create: `crates/lens-client/src/stream/sse.rs`
- Modify: `crates/lens-client/src/lib.rs` (add `pub mod stream;` and re-exports)
- Create: `crates/lens-client/tests/fixtures/sse/happy_path.stream.sse` (copied from `docs/spikes/captures/2026-06-26-sse/happy_path.stream.sse`)

**Interfaces:**
- Produces: `pub(crate) struct SseFrame { pub event: String, pub data: String }` and `pub(crate) struct SseParser` with `fn new() -> Self`, `fn push(&mut self, bytes: &[u8]) -> Vec<SseFrame>` (returns frames completed by this chunk), and `fn finish(&mut self) -> Vec<SseFrame>` (flush any trailing complete frame at EOF).

- [ ] **Step 1: Copy the golden fixture**

```bash
mkdir -p crates/lens-client/tests/fixtures/sse
cp docs/spikes/captures/2026-06-26-sse/happy_path.stream.sse crates/lens-client/tests/fixtures/sse/happy_path.stream.sse
```

- [ ] **Step 2: Write the failing test**

Create `crates/lens-client/src/stream/sse.rs`:

```rust
//! Pure SSE wire-framing parser (`event: …\ndata: …\n\n`). No I/O — the reader
//! thread (stream::reader) feeds byte chunks in; this splits them into frames.
//! Live-tail, no-replay (transport spike §4): framing must wait for a full
//! `\n\n`-terminated frame, never match on a raw substring.

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SseFrame {
    pub event: String,
    pub data: String,
}

#[derive(Default)]
pub(crate) struct SseParser {
    buf: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_single_frame() {
        let mut p = SseParser::default();
        let frames = p.push(b"event: session.status\ndata: {\"status\":\"idle\"}\n\n");
        assert_eq!(
            frames,
            vec![SseFrame {
                event: "session.status".into(),
                data: "{\"status\":\"idle\"}".into()
            }]
        );
    }

    #[test]
    fn handles_a_frame_split_across_two_chunks() {
        let mut p = SseParser::default();
        assert!(p.push(b"event: response.completed\ndata: {\"a\":1}").is_empty());
        let frames = p.push(b"\n\n");
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].event, "response.completed");
        assert_eq!(frames[0].data, "{\"a\":1}");
    }

    #[test]
    fn parses_the_full_happy_path_fixture() {
        let bytes = include_bytes!("../../tests/fixtures/sse/happy_path.stream.sse");
        let mut p = SseParser::default();
        let mut frames = p.push(bytes);
        frames.extend(p.finish());
        // The captured happy-path turn has 25 frames (13 distinct event types).
        assert_eq!(frames.len(), 25);
        assert_eq!(frames[0].event, "session.heartbeat");
        assert!(frames.iter().any(|f| f.event == "response.completed"));
        // Every frame parsed a non-empty event name and JSON-object data.
        assert!(frames.iter().all(|f| !f.event.is_empty() && f.data.starts_with('{')));
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p lens-client --lib stream::sse 2>&1 | head -20`
Expected: FAIL — `no method named push found for struct SseParser`.

- [ ] **Step 4: Implement the parser**

Add to `crates/lens-client/src/stream/sse.rs` (above the `#[cfg(test)]` block):

```rust
impl SseParser {
    /// Feed a byte chunk; return any frames completed (`\n\n`-terminated) by it.
    /// A trailing partial frame stays buffered for the next chunk.
    pub(crate) fn push(&mut self, bytes: &[u8]) -> Vec<SseFrame> {
        // Bytes are UTF-8 SSE text; lossy is safe — control framing is ASCII.
        self.buf.push_str(&String::from_utf8_lossy(bytes));
        let mut out = Vec::new();
        while let Some(idx) = self.buf.find("\n\n") {
            let block: String = self.buf.drain(..idx + 2).collect();
            if let Some(frame) = parse_block(block.trim_end_matches('\n')) {
                out.push(frame);
            }
        }
        out
    }

    /// Flush a trailing complete frame at EOF (server closed without a final `\n\n`).
    pub(crate) fn finish(&mut self) -> Vec<SseFrame> {
        let rest = std::mem::take(&mut self.buf);
        parse_block(rest.trim()).into_iter().collect()
    }
}

/// Parse one `event:`/`data:` block. Multiple `data:` lines join with `\n`
/// (SSE spec). Returns None for comment-only/empty blocks (e.g. `:` keepalives).
fn parse_block(block: &str) -> Option<SseFrame> {
    let mut event = String::new();
    let mut data: Vec<&str> = Vec::new();
    for line in block.lines() {
        if let Some(v) = line.strip_prefix("event:") {
            event = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("data:") {
            data.push(v.strip_prefix(' ').unwrap_or(v));
        }
    }
    if event.is_empty() && data.is_empty() {
        return None;
    }
    Some(SseFrame {
        event,
        data: data.join("\n"),
    })
}
```

- [ ] **Step 5: Wire the module**

Create `crates/lens-client/src/stream/mod.rs`:

```rust
//! Live SSE event stream: pure frame parser (`sse`), typed event taxonomy
//! (`event`), and the blocking reader-thread bridge (`reader`).
pub(crate) mod sse;
```

Add to `crates/lens-client/src/lib.rs` next to the other `pub mod`/`mod` lines:

```rust
pub mod stream;
```

(If `lib.rs` uses `mod stream;` privately, keep it private and re-export the public types in later tasks. Check the existing module-visibility convention in `lib.rs` and match it.)

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p lens-client --lib stream::sse -- --nocapture`
Expected: PASS (3 tests). If `parses_the_full_happy_path_fixture` asserts the wrong count, read the fixture and set the count to the actual number of `\n\n`-separated frames — do not change the parser to fit a guessed number.

- [ ] **Step 7: Lint + commit**

```bash
cargo fmt -p lens-client && cargo clippy -p lens-client --all-targets -- -D warnings
git add crates/lens-client/src/stream/ crates/lens-client/src/lib.rs crates/lens-client/tests/fixtures/sse/
git commit -m "feat(lens-client): pure SSE frame parser"
```

---

### Task 2: `ServerStreamEvent` skeleton + `Unknown` fallback

The top-level typed event and its forward-compatible fallback. Deserialize a raw `SseFrame` into a `ServerStreamEvent`; any unmodeled `event:` type becomes `Unknown` (never a panic, never a parse error). Subsequent tasks fill in the modeled families.

**Files:**
- Create: `crates/lens-client/src/stream/event.rs`
- Modify: `crates/lens-client/src/stream/mod.rs` (add `pub mod event;`)

**Interfaces:**
- Consumes: `SseFrame` (Task 1).
- Produces:
  - `pub enum ServerStreamEvent { Session(SessionEvent), Response(ResponseEvent), Unknown { event_type: String } }` (the `Turn`/`Synthetic` arms land in Task 6 / Plan 3b).
  - `pub(crate) fn parse_event(frame: &SseFrame) -> ServerStreamEvent` — total, never fails: a modeled type that fails to deserialize ALSO degrades to `Unknown` (dev0 churn safety) rather than erroring.

- [ ] **Step 1: Write the failing test**

Create `crates/lens-client/src/stream/event.rs`:

```rust
//! The typed SSE event taxonomy, modeled from captured bytes
//! (docs/spikes/captures/2026-06-26-sse/). `parse_event` is total: an unknown
//! or unparseable event degrades to `Unknown` so the reader thread never panics
//! on dev0 contract churn (AGENTS.md: the UI never panics).

use super::sse::SseFrame;

#[derive(Debug, Clone, PartialEq)]
pub enum ServerStreamEvent {
    Session(SessionEvent),
    Response(ResponseEvent),
    /// Forward-compat escape hatch for an event type this crate version does not
    /// model. Carries only the wire `type` (no `Value` to consumers); the raw
    /// payload is dropped. The contract test (Plan 3c) alarms when a live stream
    /// produces `Unknown`, signaling a needed crate bump.
    Unknown { event_type: String },
}

#[derive(Debug, Clone, PartialEq)]
pub enum SessionEvent {} // filled in Task 3

#[derive(Debug, Clone, PartialEq)]
pub enum ResponseEvent {} // filled in Task 4

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(event: &str, data: &str) -> SseFrame {
        SseFrame { event: event.into(), data: data.into() }
    }

    #[test]
    fn unmodeled_event_type_degrades_to_unknown() {
        let ev = parse_event(&frame("session.brand_new_2027", "{}"));
        assert_eq!(ev, ServerStreamEvent::Unknown { event_type: "session.brand_new_2027".into() });
    }

    #[test]
    fn garbage_data_on_unknown_type_still_does_not_panic() {
        let ev = parse_event(&frame("totally.unknown", "not json{{"));
        assert!(matches!(ev, ServerStreamEvent::Unknown { .. }));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p lens-client --lib stream::event 2>&1 | head -20`
Expected: FAIL — `cannot find function parse_event`.

- [ ] **Step 3: Implement `parse_event` skeleton**

Add to `crates/lens-client/src/stream/event.rs`:

```rust
/// Total: maps a raw frame to a typed event, degrading to `Unknown` on any
/// unmodeled type or deserialization failure. Modeled-family dispatch is added
/// by Tasks 3–4 (each returns `Some(event)` or `None` → fall through to Unknown).
pub(crate) fn parse_event(frame: &SseFrame) -> ServerStreamEvent {
    if let Some(ev) = SessionEvent::from_frame(frame) {
        return ServerStreamEvent::Session(ev);
    }
    if let Some(ev) = ResponseEvent::from_frame(frame) {
        return ServerStreamEvent::Response(ev);
    }
    ServerStreamEvent::Unknown { event_type: frame.event.clone() }
}

impl SessionEvent {
    fn from_frame(_frame: &SseFrame) -> Option<Self> { None } // Task 3 fills this
}
impl ResponseEvent {
    fn from_frame(_frame: &SseFrame) -> Option<Self> { None } // Task 4 fills this
}
```

Add to `crates/lens-client/src/stream/mod.rs`:

```rust
pub mod event;
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p lens-client --lib stream::event`
Expected: PASS (2 tests).

> Note: an empty `enum SessionEvent {}` cannot be constructed; `from_frame` returning `None` compiles because it never names a variant. Task 3 replaces the empty enum with real variants.

- [ ] **Step 5: Lint + commit**

```bash
cargo fmt -p lens-client && cargo clippy -p lens-client --all-targets -- -D warnings
git add crates/lens-client/src/stream/event.rs crates/lens-client/src/stream/mod.rs
git commit -m "feat(lens-client): ServerStreamEvent skeleton + Unknown fallback"
```

---

### Task 3: `SessionEvent` family (byte-modeled)

Model the captured `session.*` chrome events. **Bytes correct the design (typed-client.md §10) in three places** — encode the byte shapes, not the sketch:
- `session.changed_files.invalidated` carries `{session_id, environment_id}` — **NO `paths`** (the §10 `paths` field is wrong).
- `session.interrupted` carries `{data:{requested_at, response_id}}` — not bare.
- `session.input.consumed` nests its fields under `data:{item_id, type, data}`.
- `session.status.status` is a lowercase string (`idle`/`running`/…); `session.heartbeat` carries `server_time` (nullable) in addition to `sequence_number`.

**Files:**
- Modify: `crates/lens-client/src/stream/event.rs`
- Create: `crates/lens-client/tests/fixtures/sse/interrupt.stream.sse` (from `docs/spikes/captures/2026-06-26-sse/interrupt.stream.sse`)

**Interfaces:**
- Produces (public):

```rust
pub enum SessionEvent {
    Status { status: SessionStatusValue, response_id: Option<String> },
    Usage { context_tokens: Option<i64>, context_window: Option<i64>, total_cost_usd: Option<f64> },
    Presence { viewers: Vec<PresenceViewer> },
    Heartbeat { sequence_number: Option<i64>, server_time: Option<String> },
    ResourceCreated,
    InputConsumed { item_id: String, item_type: String },
    ChangedFilesInvalidated { environment_id: String },
    Interrupted { requested_at: Option<i64> },
}
pub enum SessionStatusValue { Idle, Launching, Running, Waiting, Failed, Unknown }
pub struct PresenceViewer { /* private fields + getters; see step */ }
```

- [ ] **Step 1: Copy the interrupt fixture**

```bash
cp docs/spikes/captures/2026-06-26-sse/interrupt.stream.sse crates/lens-client/tests/fixtures/sse/interrupt.stream.sse
```

- [ ] **Step 2: Write the failing tests**

Replace the empty `enum SessionEvent {}` test block and add to the `#[cfg(test)] mod tests` in `event.rs`:

```rust
    #[test]
    fn status_running_from_bytes() {
        let ev = parse_event(&frame(
            "session.status",
            r#"{"conversation_id":"c","status":"running","response_id":null,"error":null}"#,
        ));
        assert_eq!(
            ev,
            ServerStreamEvent::Session(SessionEvent::Status {
                status: SessionStatusValue::Running,
                response_id: None,
            })
        );
    }

    #[test]
    fn unknown_status_string_is_not_a_panic() {
        let ev = parse_event(&frame("session.status", r#"{"status":"hibernating"}"#));
        assert_eq!(
            ev,
            ServerStreamEvent::Session(SessionEvent::Status {
                status: SessionStatusValue::Unknown,
                response_id: None,
            })
        );
    }

    #[test]
    fn changed_files_invalidated_has_no_paths_field() {
        // Byte-verified: payload is {session_id, environment_id}; the design's
        // `paths` field does not exist on the wire.
        let ev = parse_event(&frame(
            "session.changed_files.invalidated",
            r#"{"sequence_number":null,"session_id":"c","environment_id":"default"}"#,
        ));
        assert_eq!(
            ev,
            ServerStreamEvent::Session(SessionEvent::ChangedFilesInvalidated {
                environment_id: "default".into(),
            })
        );
    }

    #[test]
    fn input_consumed_reads_nested_data() {
        let ev = parse_event(&frame(
            "session.input.consumed",
            r#"{"data":{"item_id":"msg_1","type":"message","data":{}}}"#,
        ));
        assert_eq!(
            ev,
            ServerStreamEvent::Session(SessionEvent::InputConsumed {
                item_id: "msg_1".into(),
                item_type: "message".into(),
            })
        );
    }

    #[test]
    fn interrupted_carries_requested_at() {
        let ev = parse_event(&frame(
            "session.interrupted",
            r#"{"data":{"requested_at":1782502914,"response_id":null}}"#,
        ));
        assert_eq!(
            ev,
            ServerStreamEvent::Session(SessionEvent::Interrupted { requested_at: Some(1782502914) })
        );
    }

    #[test]
    fn interrupt_fixture_yields_a_session_interrupted_event() {
        let bytes = include_bytes!("../../tests/fixtures/sse/interrupt.stream.sse");
        let mut p = super::super::sse::SseParser::default();
        let mut frames = p.push(bytes);
        frames.extend(p.finish());
        assert!(frames.iter().map(parse_event).any(|e| matches!(
            e,
            ServerStreamEvent::Session(SessionEvent::Interrupted { .. })
        )));
    }
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p lens-client --lib stream::event 2>&1 | head -25`
Expected: FAIL — `SessionEvent` has no variant `Status`, etc.

- [ ] **Step 4: Implement `SessionEvent`**

Replace the empty `SessionEvent` enum and its `from_frame` stub in `event.rs`:

```rust
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq)]
pub enum SessionEvent {
    Status { status: SessionStatusValue, response_id: Option<String> },
    Usage { context_tokens: Option<i64>, context_window: Option<i64>, total_cost_usd: Option<f64> },
    Presence { viewers: Vec<PresenceViewer> },
    Heartbeat { sequence_number: Option<i64>, server_time: Option<String> },
    ResourceCreated,
    InputConsumed { item_id: String, item_type: String },
    ChangedFilesInvalidated { environment_id: String },
    Interrupted { requested_at: Option<i64> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatusValue {
    Idle,
    Launching,
    Running,
    Waiting,
    Failed,
    /// Any status literal this crate version does not know (dev0 churn safety).
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresenceViewer {
    user_id: Option<String>,
}
impl PresenceViewer {
    pub fn user_id(&self) -> Option<&str> { self.user_id.as_deref() }
}

// Internal raw shapes (private; never exposed) used only to deserialize.
#[derive(Deserialize)]
struct RawStatus { status: SessionStatusValue, #[serde(default)] response_id: Option<String> }
#[derive(Deserialize)]
struct RawUsage { #[serde(default)] context_tokens: Option<i64>, #[serde(default)] context_window: Option<i64>, #[serde(default)] total_cost_usd: Option<f64> }
#[derive(Deserialize)]
struct RawPresence { #[serde(default)] viewers: Vec<RawViewer> }
#[derive(Deserialize)]
struct RawViewer { #[serde(default)] user_id: Option<String> }
#[derive(Deserialize)]
struct RawHeartbeat { #[serde(default)] sequence_number: Option<i64>, #[serde(default)] server_time: Option<String> }
#[derive(Deserialize)]
struct RawChangedFiles { environment_id: String }
#[derive(Deserialize)]
struct RawInputConsumed { data: RawInputConsumedData }
#[derive(Deserialize)]
struct RawInputConsumedData { item_id: String, #[serde(rename = "type")] item_type: String }
#[derive(Deserialize)]
struct RawInterrupted { #[serde(default)] data: Option<RawInterruptedData> }
#[derive(Deserialize)]
struct RawInterruptedData { #[serde(default)] requested_at: Option<i64> }

impl SessionEvent {
    fn from_frame(frame: &SseFrame) -> Option<Self> {
        // Returns None on a non-session.* type → parse_event falls through.
        // A modeled type that fails to deserialize maps to Unknown at the
        // parse_event layer is NOT what we want here; instead we surface a safe
        // default so the chrome event is not silently dropped. We do that by
        // returning Some with best-effort fields, falling back to Unknown status
        // / empty collections (serde `default`). A hard parse failure on a
        // session.* type returns None (→ Unknown) — acceptable, it is logged.
        let d = &frame.data;
        Some(match frame.event.as_str() {
            "session.status" => {
                let r: RawStatus = serde_json::from_str(d).ok()?;
                SessionEvent::Status { status: r.status, response_id: r.response_id }
            }
            "session.usage" => {
                let r: RawUsage = serde_json::from_str(d).ok()?;
                SessionEvent::Usage { context_tokens: r.context_tokens, context_window: r.context_window, total_cost_usd: r.total_cost_usd }
            }
            "session.presence" => {
                let r: RawPresence = serde_json::from_str(d).ok()?;
                SessionEvent::Presence { viewers: r.viewers.into_iter().map(|v| PresenceViewer { user_id: v.user_id }).collect() }
            }
            "session.heartbeat" => {
                let r: RawHeartbeat = serde_json::from_str(d).ok()?;
                SessionEvent::Heartbeat { sequence_number: r.sequence_number, server_time: r.server_time }
            }
            "session.resource.created" => SessionEvent::ResourceCreated,
            "session.input.consumed" => {
                let r: RawInputConsumed = serde_json::from_str(d).ok()?;
                SessionEvent::InputConsumed { item_id: r.data.item_id, item_type: r.data.item_type }
            }
            "session.changed_files.invalidated" => {
                let r: RawChangedFiles = serde_json::from_str(d).ok()?;
                SessionEvent::ChangedFilesInvalidated { environment_id: r.environment_id }
            }
            "session.interrupted" => {
                let r: RawInterrupted = serde_json::from_str(d).ok()?;
                SessionEvent::Interrupted { requested_at: r.data.and_then(|x| x.requested_at) }
            }
            _ => return None,
        })
    }
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p lens-client --lib stream::event`
Expected: PASS (all SessionEvent tests + the Task 2 tests).

- [ ] **Step 6: Lint + commit**

```bash
cargo fmt -p lens-client && cargo clippy -p lens-client --all-targets -- -D warnings
git add crates/lens-client/src/stream/event.rs crates/lens-client/tests/fixtures/sse/interrupt.stream.sse
git commit -m "feat(lens-client): SessionEvent family from bytes (changed_files no-paths fix)"
```

---

### Task 4: `ResponseEvent` family + `Item` union (byte-modeled)

Model the captured `response.*` events and the conversation-`Item` union from `output_item.done`. **Byte facts to encode:** `function_call.arguments` is a JSON **string** (keep it as `String`; do not parse — the state model decides); the `function_call.agent` field is `resp_…` while `status:"in_progress"` and the agent **name** when `completed` (expose the raw string as `agent`, document the wart); `response.completed.response.output` is `[]`.

**Files:**
- Modify: `crates/lens-client/src/stream/event.rs`

**Interfaces:**
- Produces (public):

```rust
pub enum ResponseEvent {
    InProgress,
    Completed,
    OutputTextDelta { delta: String, message_id: Option<String>, index: Option<usize>, last: Option<bool> },
    ReasoningStarted,
    OutputItemDone { item: Item },
}
pub enum Item {
    Message { id: String, role: String, content: Vec<MessageContentBlock> },
    FunctionCall { id: String, call_id: String, name: String, arguments: String, status: String, agent: Option<String> },
    FunctionCallOutput { id: String, call_id: String, output: String },
    Error { id: String, source: Option<String>, code: Option<String>, message: Option<String> },
    Other { item_type: String },  // forward-compat for unmodeled item types
}
pub struct MessageContentBlock { /* private fields + getters */ }
```

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` in `event.rs`:

```rust
    #[test]
    fn output_text_delta_from_bytes() {
        let ev = parse_event(&frame(
            "response.output_text.delta",
            r#"{"sequence_number":4,"delta":"Hello","message_id":null,"index":null,"final":null}"#,
        ));
        assert_eq!(
            ev,
            ServerStreamEvent::Response(ResponseEvent::OutputTextDelta {
                delta: "Hello".into(), message_id: None, index: None, last: None,
            })
        );
    }

    #[test]
    fn output_item_done_function_call_keeps_arguments_as_string() {
        let ev = parse_event(&frame(
            "response.output_item.done",
            r#"{"item":{"id":"fc_1","type":"function_call","status":"completed","name":"sys_os_shell","arguments":"{\"command\":\"pwd\"}","call_id":"toolu_1","agent":"claude-sdk"}}"#,
        ));
        match ev {
            ServerStreamEvent::Response(ResponseEvent::OutputItemDone {
                item: Item::FunctionCall { name, arguments, call_id, agent, .. },
            }) => {
                assert_eq!(name, "sys_os_shell");
                assert_eq!(arguments, r#"{"command":"pwd"}"#); // raw JSON string, unparsed
                assert_eq!(call_id, "toolu_1");
                assert_eq!(agent.as_deref(), Some("claude-sdk"));
            }
            other => panic!("wrong event: {other:?}"),
        }
    }

    #[test]
    fn output_item_done_message_and_output() {
        let m = parse_event(&frame(
            "response.output_item.done",
            r#"{"item":{"id":"msg_1","type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":"hi"}]}}"#,
        ));
        assert!(matches!(m, ServerStreamEvent::Response(ResponseEvent::OutputItemDone { item: Item::Message { .. } })));
        let o = parse_event(&frame(
            "response.output_item.done",
            r#"{"item":{"id":"fco_1","type":"function_call_output","call_id":"toolu_1","output":"/work"}}"#,
        ));
        assert!(matches!(o, ServerStreamEvent::Response(ResponseEvent::OutputItemDone { item: Item::FunctionCallOutput { .. } })));
    }

    #[test]
    fn error_item_from_bytes() {
        let ev = parse_event(&frame(
            "response.output_item.done",
            r#"{"item":{"id":"err_1","type":"error","status":"completed","data":{"source":"execution","code":"RuntimeError","message":"boom"}}}"#,
        ));
        match ev {
            ServerStreamEvent::Response(ResponseEvent::OutputItemDone {
                item: Item::Error { code, message, source, .. },
            }) => {
                assert_eq!(code.as_deref(), Some("RuntimeError"));
                assert_eq!(message.as_deref(), Some("boom"));
                assert_eq!(source.as_deref(), Some("execution"));
            }
            other => panic!("wrong event: {other:?}"),
        }
    }

    #[test]
    fn unmodeled_item_type_becomes_other_not_panic() {
        let ev = parse_event(&frame(
            "response.output_item.done",
            r#"{"item":{"id":"x","type":"native_tool","kind":"web_search_call"}}"#,
        ));
        assert!(matches!(
            ev,
            ServerStreamEvent::Response(ResponseEvent::OutputItemDone { item: Item::Other { .. } })
        ));
    }

    #[test]
    fn happy_path_fixture_full_event_coverage() {
        let bytes = include_bytes!("../../tests/fixtures/sse/happy_path.stream.sse");
        let mut p = super::super::sse::SseParser::default();
        let mut frames = p.push(bytes);
        frames.extend(p.finish());
        let events: Vec<_> = frames.iter().map(parse_event).collect();
        // No event in the captured happy-path turn falls through to Unknown.
        let unknowns: Vec<_> = events.iter().filter_map(|e| match e {
            ServerStreamEvent::Unknown { event_type } => Some(event_type.clone()),
            _ => None,
        }).collect();
        assert!(unknowns.is_empty(), "unmodeled captured events: {unknowns:?}");
        // The item union is exercised: function_call, message, function_call_output all present.
        let has = |pred: fn(&Item) -> bool| events.iter().any(|e| matches!(e,
            ServerStreamEvent::Response(ResponseEvent::OutputItemDone { item }) if pred(item)));
        assert!(has(|i| matches!(i, Item::FunctionCall { .. })));
        assert!(has(|i| matches!(i, Item::Message { .. })));
        assert!(has(|i| matches!(i, Item::FunctionCallOutput { .. })));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p lens-client --lib stream::event 2>&1 | head -25`
Expected: FAIL — `ResponseEvent` has no variant `OutputTextDelta`, etc.

- [ ] **Step 3: Implement `ResponseEvent` + `Item`**

Replace the empty `ResponseEvent` enum and its `from_frame` stub in `event.rs`:

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum ResponseEvent {
    InProgress,
    Completed,
    OutputTextDelta { delta: String, message_id: Option<String>, index: Option<usize>, last: Option<bool> },
    ReasoningStarted,
    OutputItemDone { item: Item },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Message { id: String, role: String, content: Vec<MessageContentBlock> },
    /// `arguments` is the raw JSON string as it arrives on the wire (unparsed —
    /// the state model owns parsing). `agent` is a wire wart: it is the
    /// `resp_…` response id while `status == "in_progress"`, and the agent name
    /// once `completed`. Exposed verbatim; consumers must not assume a name.
    FunctionCall { id: String, call_id: String, name: String, arguments: String, status: String, agent: Option<String> },
    FunctionCallOutput { id: String, call_id: String, output: String },
    Error { id: String, source: Option<String>, code: Option<String>, message: Option<String> },
    /// Forward-compat for item types not yet modeled (native_tool, reasoning,
    /// compaction, slash_command, terminal_command, resource_event) — added in
    /// Task 6 / at config-time capture.
    Other { item_type: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct MessageContentBlock {
    block_type: String,
    text: Option<String>,
}
impl MessageContentBlock {
    pub fn block_type(&self) -> &str { &self.block_type }
    pub fn text(&self) -> Option<&str> { self.text.as_deref() }
}

#[derive(Deserialize)]
struct RawTextDelta { delta: String, #[serde(default)] message_id: Option<String>, #[serde(default)] index: Option<usize>, #[serde(default, rename = "final")] last: Option<bool> }
#[derive(Deserialize)]
struct RawItemEnvelope { item: serde_json::Value }
#[derive(Deserialize)]
struct RawContentBlock { #[serde(rename = "type")] block_type: String, #[serde(default)] text: Option<String> }
#[derive(Deserialize)]
struct RawErrorData { #[serde(default)] source: Option<String>, #[serde(default)] code: Option<String>, #[serde(default)] message: Option<String> }

impl ResponseEvent {
    fn from_frame(frame: &SseFrame) -> Option<Self> {
        let d = &frame.data;
        Some(match frame.event.as_str() {
            "response.in_progress" => ResponseEvent::InProgress,
            "response.completed" => ResponseEvent::Completed,
            "response.reasoning.started" => ResponseEvent::ReasoningStarted,
            "response.output_text.delta" => {
                let r: RawTextDelta = serde_json::from_str(d).ok()?;
                ResponseEvent::OutputTextDelta { delta: r.delta, message_id: r.message_id, index: r.index, last: r.last }
            }
            "response.output_item.done" => {
                let env: RawItemEnvelope = serde_json::from_str(d).ok()?;
                ResponseEvent::OutputItemDone { item: Item::from_value(env.item) }
            }
            _ => return None,
        })
    }
}

impl Item {
    /// Total over a wire item object; unmodeled `type`s map to `Other`.
    fn from_value(v: serde_json::Value) -> Self {
        let id = v.get("id").and_then(|x| x.as_str()).unwrap_or_default().to_string();
        let item_type = v.get("type").and_then(|x| x.as_str()).unwrap_or_default().to_string();
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or_default().to_string();
        let so = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_string);
        match item_type.as_str() {
            "message" => {
                let content = v.get("content")
                    .and_then(|c| serde_json::from_value::<Vec<RawContentBlock>>(c.clone()).ok())
                    .unwrap_or_default()
                    .into_iter().map(|b| MessageContentBlock { block_type: b.block_type, text: b.text }).collect();
                Item::Message { id, role: s("role"), content }
            }
            "function_call" => Item::FunctionCall {
                id, call_id: s("call_id"), name: s("name"), arguments: s("arguments"), status: s("status"), agent: so("agent"),
            },
            "function_call_output" => Item::FunctionCallOutput { id, call_id: s("call_id"), output: s("output") },
            "error" => {
                let data = v.get("data").and_then(|x| serde_json::from_value::<RawErrorData>(x.clone()).ok()).unwrap_or(RawErrorData { source: None, code: None, message: None });
                Item::Error { id, source: data.source, code: data.code, message: data.message }
            }
            other => Item::Other { item_type: other.to_string() },
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p lens-client --lib stream::event`
Expected: PASS (all ResponseEvent/Item tests + earlier tests).

> If `happy_path_fixture_full_event_coverage` reports unmodeled events, they are real captured types this task did not model — verify against the fixture and either model them here or accept them as `Unknown`/`Other` and adjust the assertion with a documented allow-list comment. Do not weaken the no-Unknown intent silently.

- [ ] **Step 5: Lint + commit**

```bash
cargo fmt -p lens-client && cargo clippy -p lens-client --all-targets -- -D warnings
git add crates/lens-client/src/stream/event.rs
git commit -m "feat(lens-client): ResponseEvent + Item union from bytes"
```

---

### Task 5: `EventStream` reader thread + `Sessions::stream()`

The blocking OS thread that holds the `reqwest::blocking` response body, feeds the parser, and pushes typed events down an `mpsc` channel. `EventStream` is the consumer handle. This is the Arbor-pattern bridge (typed-client.md §4): thread → channel → poller.

**Files:**
- Create: `crates/lens-client/src/stream/reader.rs`
- Modify: `crates/lens-client/src/stream/mod.rs` (add `pub mod reader;` + re-exports)
- Modify: `crates/lens-client/src/sessions.rs` (add `Sessions::stream`)
- Modify: `crates/lens-client/src/client.rs` (add a `pub(crate) fn http_stream_get` if a streaming GET helper is cleaner than reusing `http()`)
- Create: `crates/lens-client/tests/live_stream.rs` (live, feature-gated)

**Interfaces:**
- Consumes: `Client` (`conn`, `http`), `SseParser` (Task 1), `parse_event` (Task 2).
- Produces:
  - `pub struct EventStream` with `pub fn recv(&self) -> Option<ServerStreamEvent>` (blocks until the next event or the stream closes → `None`) and `pub fn try_recv(&self) -> Option<ServerStreamEvent>` (non-blocking drain for the gpui poller).
  - `Sessions::stream(&self, id: &SessionId) -> Result<EventStream>` — opens `GET /v1/sessions/{id}/stream` and spawns the reader thread.

- [ ] **Step 1: Implement the reader + EventStream**

Create `crates/lens-client/src/stream/reader.rs`:

```rust
//! The SSE reader thread: holds the blocking reqwest body, feeds the pure
//! parser, deserializes typed events, and pushes them down an mpsc channel.
//! One thread per active session (typed-client.md §4); the gpui poller drains
//! via `try_recv` off `cx.background_spawn`. Never blocks the foreground thread.

use std::io::Read;
use std::sync::mpsc;
use std::thread::JoinHandle;

use super::event::{parse_event, ServerStreamEvent};
use super::sse::SseParser;

pub struct EventStream {
    rx: mpsc::Receiver<ServerStreamEvent>,
    _handle: JoinHandle<()>,
}

impl EventStream {
    /// Spawn the reader thread over an open blocking response body.
    pub(crate) fn spawn(resp: reqwest::blocking::Response) -> Self {
        let (tx, rx) = mpsc::channel();
        let handle = std::thread::Builder::new()
            .name("lens-sse-reader".into())
            .spawn(move || run(resp, tx))
            .expect("spawn SSE reader thread");
        EventStream { rx, _handle: handle }
    }

    /// Block until the next event, or `None` when the stream closes.
    pub fn recv(&self) -> Option<ServerStreamEvent> {
        self.rx.recv().ok()
    }

    /// Non-blocking drain for the UI poller. `None` when no event is queued
    /// (including after the stream has closed).
    pub fn try_recv(&self) -> Option<ServerStreamEvent> {
        self.rx.try_recv().ok()
    }
}

fn run(mut resp: reqwest::blocking::Response, tx: mpsc::Sender<ServerStreamEvent>) {
    let mut parser = SseParser::default();
    let mut buf = [0u8; 8192];
    loop {
        match resp.read(&mut buf) {
            Ok(0) => break, // server closed the stream
            Ok(n) => {
                for frame in parser.push(&buf[..n]) {
                    if tx.send(parse_event(&frame)).is_err() {
                        return; // consumer dropped EventStream — stop reading
                    }
                }
            }
            Err(_) => break, // network error: close the channel (Plan 3b reconnects)
        }
    }
    for frame in parser.finish() {
        let _ = tx.send(parse_event(&frame));
    }
}
```

- [ ] **Step 2: Add `Sessions::stream`**

In `crates/lens-client/src/sessions.rs`, add (matching the existing `Sessions` accessor style — it holds `&Client`):

```rust
    /// Open the live SSE event stream for a session. Live-tail, no-replay:
    /// the caller must subscribe BEFORE posting the message that should be
    /// observed (transport spike §4). Returns an `EventStream` whose reader
    /// thread is already running.
    pub fn stream(&self, id: &crate::ids::SessionId) -> crate::error::Result<crate::stream::EventStream> {
        let url = self.client.conn().url(&format!("/v1/sessions/{id}/stream"))?;
        let resp = self.client.conn().auth.apply(self.client.http().get(url)).send()?;
        let status = resp.status().as_u16();
        if !(200..=299).contains(&status) {
            return Err(crate::http::check_status("v1/sessions/stream", status).unwrap_err());
        }
        Ok(crate::stream::EventStream::spawn(resp))
    }
```

> Check how `Sessions` names its `&Client` field (e.g. `self.client` vs `self.0`) in `sessions.rs` and match it. Reuse the existing `conn()`/`http()` accessors on `Client`.

- [ ] **Step 3: Export the public types**

In `crates/lens-client/src/stream/mod.rs`:

```rust
pub mod reader;
pub use event::{Item, MessageContentBlock, PresenceViewer, ResponseEvent, ServerStreamEvent, SessionEvent, SessionStatusValue};
pub use reader::EventStream;
```

Ensure `lib.rs` re-exports these at the crate root if that is the existing convention for the typed surface (match how `SessionEventInput`, `Auth`, `Connection` are re-exported).

- [ ] **Step 4: Build to verify it compiles**

Run: `cargo build -p lens-client`
Expected: clean build.

- [ ] **Step 5: Write the live test**

Create `crates/lens-client/tests/live_stream.rs`:

```rust
//! Live test — requires a running omnigent server at $LENS_OMNIGENT_URL and an
//! idle, runner-backed session id in $LENS_OMNIGENT_SESSION_ID (claude-sdk).
//! Subscribe-first: opens the stream, posts a message, asserts typed events flow.
//! Run: LENS_OMNIGENT_URL=… LENS_OMNIGENT_SESSION_ID=… \
//!   cargo test -p lens-client --features live-tests --test live_stream -- --nocapture
#![cfg(feature = "live-tests")]

use lens_client::ids::{ConnectionId, SessionId};
use lens_client::stream::{ResponseEvent, ServerStreamEvent};
use lens_client::{Auth, Connection, SessionEventInput};

#[test]
fn live_stream_yields_typed_events() {
    let base = std::env::var("LENS_OMNIGENT_URL").expect("set LENS_OMNIGENT_URL").parse().unwrap();
    let sid = SessionId::new(std::env::var("LENS_OMNIGENT_SESSION_ID").expect("set LENS_OMNIGENT_SESSION_ID"));
    let client = lens_client::Client::new(Connection::new(ConnectionId::new("live"), base, Auth::None)).unwrap();

    // Subscribe FIRST (no-replay), then drive a turn.
    let stream = client.sessions().stream(&sid).expect("open stream");
    client.sessions().send_event(&sid, &SessionEventInput::Message {
        content: vec![serde_json::json!({"type":"input_text","text":"Say hello in one word."})],
        model_override: None,
        tools: None,
    }).expect("post message");

    // Drain until a terminal response event or a timeout.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    let mut saw_completed = false;
    let mut saw_unknown: Vec<String> = Vec::new();
    while std::time::Instant::now() < deadline {
        match stream.recv() {
            Some(ServerStreamEvent::Response(ResponseEvent::Completed)) => { saw_completed = true; break; }
            Some(ServerStreamEvent::Unknown { event_type }) => saw_unknown.push(event_type),
            Some(_) => {}
            None => break,
        }
    }
    assert!(saw_completed, "never observed response.completed");
    // Surface (do not hard-fail) any unmodeled live events — feeds Plan 3c drift.
    if !saw_unknown.is_empty() {
        eprintln!("UNMODELED live events (model these / Plan 3c): {saw_unknown:?}");
    }
}
```

- [ ] **Step 6: Run the live test against the pinned server**

Warm a session first (per the spike rig):

```bash
omnigent run --harness claude-sdk --server http://127.0.0.1:6767 -p "hi" </dev/null
# grab the new idle claude-sdk session id from: omnigent server status / GET /v1/sessions
LENS_OMNIGENT_URL=http://127.0.0.1:6767 LENS_OMNIGENT_SESSION_ID=<conv_…> \
  cargo test -p lens-client --features live-tests --test live_stream -- --nocapture
```

Expected: PASS — prints any UNMODELED live events, asserts `response.completed` observed.

- [ ] **Step 7: Lint + commit**

```bash
cargo fmt -p lens-client && cargo clippy -p lens-client --all-targets -- -D warnings
git add crates/lens-client/src/stream/ crates/lens-client/src/sessions.rs crates/lens-client/src/client.rs crates/lens-client/tests/live_stream.rs
git commit -m "feat(lens-client): EventStream reader thread + Sessions::stream"
```

---

### Task 6: Schema-derived variants (env-blocked families) + drift inventory

Add the event/item variants the spike could not capture from bytes (single-harness box) so the typed surface is *complete* and the reader thread never returns `Unknown` for a known-but-uncaptured type. Each is modeled from the openapi schema and **flagged `// SCHEMA-DERIVED (not byte-verified — re-capture at config-time)`**. Tests use schema-shaped JSON, not golden fixtures.

**Files:**
- Modify: `crates/lens-client/src/stream/event.rs`

**Interfaces:**
- Adds to `ResponseEvent`: `Failed`, `Incomplete`, `Cancelled`, `ReasoningTextDelta { delta }`, `ReasoningSummaryTextDelta { delta }`, `CompactionInProgress`, `CompactionCompleted { total_tokens: Option<i64> }`, `CompactionFailed`, `Error { source, tool_name, code, message }`, `ElicitationRequest { elicitation_id }`, `ElicitationResolved { elicitation_id }`.
- Adds to `SessionEvent`: `ChildSessionUpdated { child_id: String }`, `TerminalActivity { terminal_id: String }`, `TerminalPending { terminal_id: String }`, `Model { model: String }`, `Todos`, `ReasoningEffort { effort: String }`, `ModelOptions`, `SandboxStatus`, `Skills`.

- [ ] **Step 1: Write schema-shaped tests**

Add to the `#[cfg(test)] mod tests`, each annotated as schema-derived:

```rust
    #[test]
    fn schema_reasoning_text_delta() {
        // SCHEMA-DERIVED: ReasoningTextDeltaEvent {delta, sequence_number, type}.
        let ev = parse_event(&frame("response.reasoning_text.delta", r#"{"delta":"because","sequence_number":5}"#));
        assert_eq!(ev, ServerStreamEvent::Response(ResponseEvent::ReasoningTextDelta { delta: "because".into() }));
    }

    #[test]
    fn schema_reasoning_summary_text_delta() {
        // SCHEMA-DERIVED.
        let ev = parse_event(&frame("response.reasoning_summary_text.delta", r#"{"delta":"sum"}"#));
        assert_eq!(ev, ServerStreamEvent::Response(ResponseEvent::ReasoningSummaryTextDelta { delta: "sum".into() }));
    }

    #[test]
    fn schema_response_failed_carries_status() {
        // SCHEMA-DERIVED: response.failed mirrors response.completed (response obj).
        let ev = parse_event(&frame("response.failed", r#"{"response":{"status":"failed"}}"#));
        assert_eq!(ev, ServerStreamEvent::Response(ResponseEvent::Failed));
    }

    #[test]
    fn schema_child_session_updated() {
        // SCHEMA-DERIVED: session.child_session.updated — child id under data.
        let ev = parse_event(&frame("session.child_session.updated", r#"{"data":{"child_session_id":"conv_child"}}"#));
        assert_eq!(ev, ServerStreamEvent::Session(SessionEvent::ChildSessionUpdated { child_id: "conv_child".into() }));
    }
```

(Add one test per new variant family in the same form; keep each minimal.)

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p lens-client --lib stream::event 2>&1 | head -20`
Expected: FAIL — variants not defined.

- [ ] **Step 3: Implement the schema-derived variants**

Extend the `ResponseEvent`/`SessionEvent` enums and their `from_frame` match arms with the new types, each preceded by `// SCHEMA-DERIVED (not byte-verified — re-capture at config-time)`. Read the exact `type` consts and field names from `vendor/omnigent-0.3.0.dev0/openapi.json` (e.g. `ReasoningTextDeltaEvent` → `response.reasoning_text.delta`; `response.failed`/`response.incomplete`/`response.cancelled`; the `session.child_session.updated`/`session.terminal.activity`/`session.terminal_pending` consts). Do not invent field names — copy them from the schema.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p lens-client --lib stream::event`
Expected: PASS.

- [ ] **Step 5: Record the drift inventory**

Append to `docs/spikes/2026-06-26-golden-sse-capture.md` a short "Schema-derived variants pending byte-verification" list naming each variant added here, so the config-time re-capture has a checklist.

- [ ] **Step 6: Lint + commit**

```bash
cargo fmt -p lens-client && cargo clippy -p lens-client --all-targets -- -D warnings
git add crates/lens-client/src/stream/event.rs docs/spikes/2026-06-26-golden-sse-capture.md
git commit -m "feat(lens-client): schema-derived SSE variants (flagged, pending byte-verify)"
```

---

## Out of scope for 3a (next plans)

- **Plan 3b — normalization + no-replay reconnect:** §7a dedup (`ToolCall`/`ToolResult` by `call_id`), synthetic `ReasoningClosed`, the `Reconnected { gap }` ordering guarantee, and the §7 three-bucket reconnect protocol (snapshot + `/items` merge-by-id + sequence-dedup on the `response.*` overlap). The reader thread's `Err(_) => break` is the seam where 3b attaches reconnect.
- **Plan 3c — contract-drift CI (outstanding B6):** the live `live_stream` test's `Unknown`/`Other` surfacing becomes a gated alarm; `xtask drift` diffs the captured taxonomy vs the server.
- **WS terminal attach (§5):** `tungstenite` binary PTY stream — separate plan.

## Self-Review notes

- **Spec coverage:** §4 SSE stream → Tasks 1+5; §10 `ServerStreamEvent`/`SessionEvent`/`ResponseEvent`/`Item` → Tasks 2–4+6; the three byte-corrections (changed_files no-paths, interrupted payload, input.consumed nesting) → Task 3; the function_call `arguments`-as-string + `agent` wart → Task 4. §7/§7a deferred to 3b (stated). Reasoning deltas + child_session/terminal/elicitation/compaction → Task 6 schema-derived (stated, flagged).
- **No-Value rule:** the only `serde_json::Value` use is internal (`RawItemEnvelope.item`, error `data`) inside `Item::from_value`/`from_frame`; never exposed. `Unknown`/`Other` carry only `String`. ✓
- **Type consistency:** `ServerStreamEvent`/`SessionEvent`/`ResponseEvent`/`Item`/`EventStream`/`SessionStatusValue` names are used identically across Tasks 2–6 and the live test. `Sessions::stream` returns `crate::stream::EventStream`. ✓
- **Never-panic:** `parse_event` is total; reader thread maps all read errors to channel close; no `unwrap` on wire data. ✓
