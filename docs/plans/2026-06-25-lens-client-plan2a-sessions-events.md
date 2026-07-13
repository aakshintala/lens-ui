# lens-client Plan 2a — Sessions subservice + the `SessionEventInput` write path

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stand up the `Sessions` subservice and the load-bearing `POST /v1/sessions/{id}/events` write path — a hand-written typed `SessionEventInput` enum that serializes to omnigent's generalized `{type, data}` body, plus a typed `SendEventAck` response.

**Architecture:** First slice of the REST surface (Unit 4 / sub-unit 5b in `typed-client-implementation.md`). The crate exposes subservices off `Client` (`client.sessions().send_event(...)`). The `/events` body is a discriminated-on-`type`, free-`data` envelope (§6), so the client models the **subset Lens sends** as a typed Rust enum whose `to_json` builds the wire shape; the server's full 30-type dispatch table is pinned as a contract constant. Reuses the foundation's `http::decode_json` for typed non-2xx/parse handling. The session **read** endpoints (`GET /v1/sessions…`) are deliberately deferred to Plan 2b: their `200` responses are untyped (`{}`) in the contract and need a separate mirror-struct-vs-`serde_json::Value` decision.

**Tech Stack:** Rust (edition 2024, toolchain 1.91), `reqwest` (blocking), `serde`/`serde_json`. No async runtime (D2).

## Global Constraints

- Edition `2024`, `rust-version = "1.91"`; workspace lints apply (`unsafe_code = "deny"`, `clippy::all = "deny"`, `unused_must_use = "deny"`) — the crate already has `lints.workspace = true`.
- No `unsafe`. No async runtime, no `tokio`, no `flume` (D2). `reqwest::blocking` only.
- Default `cargo test` must pass with **no server running**; anything needing a live server is gated behind `--features live-tests` + `LENS_OMNIGENT_URL` (and `LENS_OMNIGENT_SESSION_ID` for the send test).
- `generated.rs` is machine-generated — never hand-edited. The typed `sessions::SessionEventInput` is a **hand-written** enum (the Lens-sent subset); it is distinct from `generated::SessionEventInput` (the raw stringly-typed wire container) and is the one re-exported at the crate root.
- Reuse `http::decode_json` for the 202 ack and error mapping; do not hand-roll status handling.
- **Ground-truth discipline (`.agents/principles.md`):** discriminators and the allowed-types set are pinned to omnigent `0.3.0.dev0` (`36b2a11c`) source. Exact values, verbatim from `omnigent/server/routes/sessions.py` + `omnigent/entities/conversation.py` (see the per-task constants). Never guess a discriminator.
- **Contract facts (verbatim from source):**
  - The 6 client-sent `type` discriminators: `message`, `function_call_output`, `approval`, `interrupt`, `compact`, `stop_session`. (`compact` is the control event — NOT `compaction`, which is a persisted item type.)
  - `message.data` = `{"role":"user","content":[<blocks>]}` (`role` required; client sends `"user"`). Content blocks are open objects, e.g. `{"type":"input_text","text":"…"}`.
  - `function_call_output.data` = `{"call_id": <string>, "output": <string>}` (both required strings).
  - `approval.data` = **flat** `{"elicitation_id": <string>, "action": "accept"|"decline"|"cancel", "content"?: <object>}` (NOT nested under `result`).
  - `interrupt`/`compact`/`stop_session`: `data` = `{}`.
  - `model_override` (`string|null`) and `tools` (`array<object>|null`) are top-level envelope fields, only meaningful for `message`; omit when `None`.
  - Response: HTTP `202` + JSON ack `{queued: bool, item_id?: str, pending_id?: str, denied?: bool, reason?: str, elicitation_id?: str}` (openapi declares it as empty `{}` — under-specified; model the real shape with serde defaults).
  - Full server `_ALLOWED_EVENT_TYPES` is the 30-element set pinned in Task 2.

---

### Task 1: The typed `SessionEventInput` enum + `to_json`

**Files:**
- Create: `crates/lens-client/src/sessions.rs`
- Modify: `crates/lens-client/src/lib.rs` (add `pub mod sessions;` + re-exports)

**Interfaces:**
- Consumes: `ids::ElicitationId`.
- Produces:
  - `sessions::ElicitationAction` enum (`Accept`/`Decline`/`Cancel`, serde-renamed lowercase).
  - `sessions::SessionEventInput` enum with variants `Message { content: Vec<serde_json::Value>, model_override: Option<String>, tools: Option<Vec<serde_json::Value>> }`, `FunctionCallOutput { call_id: String, output: String }`, `Approval { elicitation_id: ElicitationId, action: ElicitationAction, content: Option<serde_json::Map<String, serde_json::Value>> }`, `Interrupt`, `Compact`, `StopSession`.
  - `SessionEventInput::discriminator(&self) -> &'static str` and `SessionEventInput::to_json(&self) -> serde_json::Value`.

- [ ] **Step 1: Write the failing tests**

`crates/lens-client/src/sessions.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ElicitationId;
    use serde_json::json;

    #[test]
    fn message_serializes_role_user_and_content() {
        let evt = SessionEventInput::Message {
            content: vec![json!({"type": "input_text", "text": "Hello"})],
            model_override: None,
            tools: None,
        };
        assert_eq!(evt.discriminator(), "message");
        assert_eq!(
            evt.to_json(),
            json!({
                "type": "message",
                "data": {"role": "user", "content": [{"type": "input_text", "text": "Hello"}]}
            })
        );
    }

    #[test]
    fn message_includes_model_override_and_tools_when_present() {
        let evt = SessionEventInput::Message {
            content: vec![json!({"type": "input_text", "text": "hi"})],
            model_override: Some("anthropic/claude".into()),
            tools: Some(vec![json!({"type": "function", "function": {"name": "f"}})]),
        };
        let v = evt.to_json();
        assert_eq!(v["model_override"], json!("anthropic/claude"));
        assert_eq!(v["tools"], json!([{"type": "function", "function": {"name": "f"}}]));
    }

    #[test]
    fn function_call_output_carries_call_id_and_output() {
        let evt = SessionEventInput::FunctionCallOutput {
            call_id: "call_abc".into(),
            output: "{\"ok\":true}".into(),
        };
        assert_eq!(evt.discriminator(), "function_call_output");
        assert_eq!(
            evt.to_json(),
            json!({"type": "function_call_output", "data": {"call_id": "call_abc", "output": "{\"ok\":true}"}})
        );
    }

    #[test]
    fn approval_is_flat_with_elicitation_id_and_action() {
        let evt = SessionEventInput::Approval {
            elicitation_id: ElicitationId::new("elicit_1"),
            action: ElicitationAction::Accept,
            content: Some(serde_json::Map::from_iter([("choice".to_string(), json!("a"))])),
        };
        assert_eq!(evt.discriminator(), "approval");
        assert_eq!(
            evt.to_json(),
            json!({"type": "approval", "data": {"elicitation_id": "elicit_1", "action": "accept", "content": {"choice": "a"}}})
        );
    }

    #[test]
    fn approval_omits_content_when_none() {
        let evt = SessionEventInput::Approval {
            elicitation_id: ElicitationId::new("elicit_2"),
            action: ElicitationAction::Decline,
            content: None,
        };
        assert_eq!(
            evt.to_json(),
            json!({"type": "approval", "data": {"elicitation_id": "elicit_2", "action": "decline"}})
        );
    }

    #[test]
    fn control_events_send_empty_data() {
        for (evt, ty) in [
            (SessionEventInput::Interrupt, "interrupt"),
            (SessionEventInput::Compact, "compact"),
            (SessionEventInput::StopSession, "stop_session"),
        ] {
            assert_eq!(evt.discriminator(), ty);
            assert_eq!(evt.to_json(), json!({"type": ty, "data": {}}));
        }
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p lens-client sessions::`
Expected: FAIL — `cannot find type SessionEventInput` (module not declared / types absent).

- [ ] **Step 3: Prepend the implementation**

Prepend to `crates/lens-client/src/sessions.rs`:
```rust
//! The `Sessions` subservice and the generalized `/events` write path.
//!
//! `SessionEventInput` here is the **hand-written** typed enum for the subset of
//! events Lens sends — distinct from `crate::generated::SessionEventInput`, which
//! is the raw `{type, data, model_override, tools}` wire container. Discriminators
//! and payload shapes are pinned to omnigent 0.3.0.dev0 source
//! (`server/routes/sessions.py`, `entities/conversation.py`); never guess them.

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::ids::ElicitationId;

/// Consumer reply action for an `approval` event (MCP `ElicitResult` semantics).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ElicitationAction {
    Accept,
    Decline,
    Cancel,
}

impl ElicitationAction {
    fn as_str(self) -> &'static str {
        match self {
            ElicitationAction::Accept => "accept",
            ElicitationAction::Decline => "decline",
            ElicitationAction::Cancel => "cancel",
        }
    }
}

/// A client-submitted session event. Only the subset Lens sends is modeled;
/// the server accepts a larger dispatch table (pinned in `ALLOWED_EVENT_TYPES`).
#[derive(Clone, Debug, PartialEq)]
pub enum SessionEventInput {
    /// A user message. `content` is a list of open content blocks, e.g.
    /// `{"type":"input_text","text":"…"}`. `role` is always `"user"` on send.
    Message {
        content: Vec<Value>,
        model_override: Option<String>,
        tools: Option<Vec<Value>>,
    },
    /// A client-side tool result.
    FunctionCallOutput { call_id: String, output: String },
    /// Reply to an outstanding elicitation. Wire `data` is flat:
    /// `{elicitation_id, action, content?}`.
    Approval {
        elicitation_id: ElicitationId,
        action: ElicitationAction,
        content: Option<Map<String, Value>>,
    },
    /// Interrupt the active turn.
    Interrupt,
    /// Request context compaction (control event `"compact"`, not the
    /// `"compaction"` item type).
    Compact,
    /// Terminate the live session (reclaim the runner). Owner-gated server-side.
    StopSession,
}

impl SessionEventInput {
    /// The wire `type` discriminator.
    pub fn discriminator(&self) -> &'static str {
        match self {
            SessionEventInput::Message { .. } => "message",
            SessionEventInput::FunctionCallOutput { .. } => "function_call_output",
            SessionEventInput::Approval { .. } => "approval",
            SessionEventInput::Interrupt => "interrupt",
            SessionEventInput::Compact => "compact",
            SessionEventInput::StopSession => "stop_session",
        }
    }

    /// Serialize into the wire envelope: `{ "type": <discrim>, "data": <payload>, .. }`.
    pub fn to_json(&self) -> Value {
        let mut obj = Map::new();
        obj.insert("type".into(), json!(self.discriminator()));

        let data: Value = match self {
            SessionEventInput::Message { content, .. } => {
                json!({ "role": "user", "content": content })
            }
            SessionEventInput::FunctionCallOutput { call_id, output } => {
                json!({ "call_id": call_id, "output": output })
            }
            SessionEventInput::Approval { elicitation_id, action, content } => {
                let mut d = Map::new();
                d.insert("elicitation_id".into(), json!(elicitation_id.as_str()));
                d.insert("action".into(), json!(action.as_str()));
                if let Some(c) = content {
                    d.insert("content".into(), Value::Object(c.clone()));
                }
                Value::Object(d)
            }
            SessionEventInput::Interrupt
            | SessionEventInput::Compact
            | SessionEventInput::StopSession => json!({}),
        };
        obj.insert("data".into(), data);

        // `model_override` / `tools` are envelope-level and only meaningful for `message`.
        if let SessionEventInput::Message { model_override, tools, .. } = self {
            if let Some(m) = model_override {
                obj.insert("model_override".into(), json!(m));
            }
            if let Some(t) = tools {
                obj.insert("tools".into(), json!(t));
            }
        }

        Value::Object(obj)
    }
}
```

- [ ] **Step 4: Register the module + re-exports**

In `crates/lens-client/src/lib.rs`, add after the existing module declarations:
```rust
pub mod sessions;
```
and add to the re-export block (next to `pub use client::Client;`):
```rust
pub use sessions::{ElicitationAction, SessionEventInput};
```

- [ ] **Step 5: Run tests + clippy + fmt**

Run: `cargo test -p lens-client sessions:: && cargo clippy -p lens-client --all-targets -- -D warnings && cargo fmt -p lens-client --check`
Expected: PASS, no warnings. (Run `cargo fmt -p lens-client` first if formatting differs; `generated.rs` must not change.)

- [ ] **Step 6: Commit**

```bash
git add crates/lens-client/src/sessions.rs crates/lens-client/src/lib.rs
git commit -m "feat(lens-client): typed SessionEventInput enum + to_json (events write path)"
```

---

### Task 2: Pin the server's allowed-event-types contract

**Files:**
- Modify: `crates/lens-client/src/sessions.rs` (add the constant + tests)

**Interfaces:**
- Produces: `sessions::ALLOWED_EVENT_TYPES: [&str; 30]` — the full server dispatch table, pinned. A drift in this set (on re-vendor) requires a deliberate crate bump.

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` in `crates/lens-client/src/sessions.rs`:
```rust
    #[test]
    fn allowed_event_types_is_the_pinned_30() {
        // Pinned to omnigent 0.3.0.dev0 (sessions.py _ALLOWED_EVENT_TYPES,
        // = ITEM_TYPE_TO_DATA_CLS keys ∪ control/external extras). Sorted.
        assert_eq!(ALLOWED_EVENT_TYPES.len(), 30);
        let mut sorted = ALLOWED_EVENT_TYPES;
        sorted.sort_unstable();
        assert_eq!(sorted, ALLOWED_EVENT_TYPES, "keep ALLOWED_EVENT_TYPES sorted");
    }

    #[test]
    fn every_sent_discriminator_is_server_allowed() {
        for evt in [
            SessionEventInput::Message { content: vec![], model_override: None, tools: None },
            SessionEventInput::FunctionCallOutput { call_id: "c".into(), output: "o".into() },
            SessionEventInput::Approval {
                elicitation_id: crate::ids::ElicitationId::new("e"),
                action: ElicitationAction::Accept,
                content: None,
            },
            SessionEventInput::Interrupt,
            SessionEventInput::Compact,
            SessionEventInput::StopSession,
        ] {
            assert!(
                ALLOWED_EVENT_TYPES.contains(&evt.discriminator()),
                "{} not in ALLOWED_EVENT_TYPES",
                evt.discriminator()
            );
        }
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p lens-client sessions::tests::allowed`
Expected: FAIL — `cannot find value ALLOWED_EVENT_TYPES`.

- [ ] **Step 3: Add the constant**

Add to `crates/lens-client/src/sessions.rs` (after the `SessionEventInput` impl, before the test module):
```rust
/// The full set of `type` discriminators the server's `POST /events` route
/// accepts (`_ALLOWED_EVENT_TYPES`, omnigent 0.3.0.dev0). Lens only *sends* the
/// six modeled by `SessionEventInput`, but the contract test pins the whole set
/// so a re-vendor that adds/removes a type is a conscious change. Kept sorted.
pub const ALLOWED_EVENT_TYPES: [&str; 30] = [
    "approval",
    "compact",
    "compaction",
    "error",
    "external_assistant_message",
    "external_codex_collaboration_mode_change",
    "external_codex_subagent_start",
    "external_compaction_status",
    "external_conversation_item",
    "external_elicitation_resolved",
    "external_model_change",
    "external_output_reasoning_delta",
    "external_output_text_delta",
    "external_reasoning_effort_change",
    "external_session_interrupted",
    "external_session_status",
    "external_session_todos",
    "external_session_usage",
    "external_subagent_start",
    "function_call",
    "function_call_output",
    "interrupt",
    "mcp_elicitation",
    "message",
    "native_tool",
    "reasoning",
    "resource_event",
    "slash_command",
    "stop_session",
    "terminal_command",
];
```

- [ ] **Step 4: Run tests + clippy + fmt**

Run: `cargo test -p lens-client sessions:: && cargo clippy -p lens-client --all-targets -- -D warnings && cargo fmt -p lens-client --check`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/lens-client/src/sessions.rs
git commit -m "feat(lens-client): pin server ALLOWED_EVENT_TYPES contract (30 types)"
```

---

### Task 3: `SendEventAck` response type

**Files:**
- Modify: `crates/lens-client/src/sessions.rs` (add the type + tests)
- Modify: `crates/lens-client/src/lib.rs` (re-export)

**Interfaces:**
- Produces: `sessions::SendEventAck { queued: bool, item_id: Option<String>, pending_id: Option<String>, denied: bool, reason: Option<String>, elicitation_id: Option<String> }` (serde `Deserialize`, all fields `#[serde(default)]`).

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` in `crates/lens-client/src/sessions.rs`:
```rust
    #[test]
    fn ack_parses_queued_with_item_id() {
        let ack: SendEventAck =
            serde_json::from_str(r#"{"queued": true, "item_id": "item_42"}"#).unwrap();
        assert!(ack.queued);
        assert_eq!(ack.item_id.as_deref(), Some("item_42"));
        assert!(!ack.denied);
    }

    #[test]
    fn ack_parses_control_event_not_queued() {
        let ack: SendEventAck = serde_json::from_str(r#"{"queued": false}"#).unwrap();
        assert!(!ack.queued);
        assert_eq!(ack.item_id, None);
    }

    #[test]
    fn ack_parses_policy_denial() {
        let ack: SendEventAck =
            serde_json::from_str(r#"{"queued": false, "denied": true, "reason": "blocked"}"#)
                .unwrap();
        assert!(ack.denied);
        assert_eq!(ack.reason.as_deref(), Some("blocked"));
    }

    #[test]
    fn ack_tolerates_unknown_and_missing_fields() {
        // Empty body (openapi's under-specified `{}`) must still deserialize.
        let ack: SendEventAck = serde_json::from_str("{}").unwrap();
        assert!(!ack.queued);
        // Unknown extra fields are ignored, not an error.
        let ack2: SendEventAck =
            serde_json::from_str(r#"{"queued": true, "future_field": 1}"#).unwrap();
        assert!(ack2.queued);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p lens-client sessions::tests::ack`
Expected: FAIL — `cannot find type SendEventAck`.

- [ ] **Step 3: Add the type**

Add to `crates/lens-client/src/sessions.rs` (near the top, after the `use` lines):
```rust
/// Ack for `POST /v1/sessions/{id}/events` (HTTP 202). The openapi declares an
/// empty body, but the route always returns a small JSON ack — model it with
/// defaults so an empty or future-extended body still deserializes.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct SendEventAck {
    /// Whether the event was queued to the runner. Control events report `false`.
    #[serde(default)]
    pub queued: bool,
    /// Store item id for persisted item events (`message`, …). For
    /// `function_call_output` this echoes the `call_id`.
    #[serde(default)]
    pub item_id: Option<String>,
    /// Pending id for the native-terminal `message` bypass path.
    #[serde(default)]
    pub pending_id: Option<String>,
    /// Set when a policy denied a user `message`.
    #[serde(default)]
    pub denied: bool,
    /// Human-readable denial reason (paired with `denied`).
    #[serde(default)]
    pub reason: Option<String>,
    /// Elicitation id for the `mcp_elicitation` path.
    #[serde(default)]
    pub elicitation_id: Option<String>,
}
```

- [ ] **Step 4: Re-export**

In `crates/lens-client/src/lib.rs`, extend the sessions re-export:
```rust
pub use sessions::{ElicitationAction, SendEventAck, SessionEventInput};
```

- [ ] **Step 5: Run tests + clippy + fmt**

Run: `cargo test -p lens-client sessions:: && cargo clippy -p lens-client --all-targets -- -D warnings && cargo fmt -p lens-client --check`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/lens-client/src/sessions.rs crates/lens-client/src/lib.rs
git commit -m "feat(lens-client): SendEventAck typed response for /events"
```

---

### Task 4: `Sessions` subservice + `send_event` (live, gated)

**Files:**
- Modify: `crates/lens-client/src/sessions.rs` (add `Sessions` + `send_event`)
- Modify: `crates/lens-client/src/client.rs` (add `Client::sessions`; remove the now-used `#[allow(dead_code)]`)
- Modify: `crates/lens-client/src/lib.rs` (re-export `Sessions`)
- Create: `crates/lens-client/tests/live_send_event.rs`

**Interfaces:**
- Consumes: `client::Client` (its `pub(crate)` `conn()`/`http()`), `connection::Connection::url`, `ids::SessionId`, `http::decode_json`, `SessionEventInput`, `SendEventAck`.
- Produces:
  - `client::Client::sessions(&self) -> sessions::Sessions<'_>`.
  - `sessions::Sessions<'a>` with `send_event(&self, id: &SessionId, evt: &SessionEventInput) -> Result<SendEventAck>`.

- [ ] **Step 1: Implement `Sessions` + `send_event`**

Add to `crates/lens-client/src/sessions.rs` (after `SendEventAck`, before the test module). Add `use crate::client::Client; use crate::error::Result; use crate::http::decode_json; use crate::ids::SessionId;` to the imports at the top of the file:
```rust
/// The session subservice — borrows the `Client` for the duration of a call.
pub struct Sessions<'a> {
    client: &'a Client,
}

impl<'a> Sessions<'a> {
    pub(crate) fn new(client: &'a Client) -> Self {
        Self { client }
    }

    /// `POST /v1/sessions/{id}/events` — submit a typed event. Returns the
    /// server's ack (queued/item_id/denial). Blocking.
    pub fn send_event(&self, id: &SessionId, evt: &SessionEventInput) -> Result<SendEventAck> {
        let conn = self.client.conn();
        let url = conn.url(&format!("/v1/sessions/{id}/events"))?;
        let resp = conn
            .auth
            .apply(self.client.http().post(url).json(&evt.to_json()))
            .send()?;
        let status = resp.status().as_u16();
        let body = resp.text()?;
        decode_json("sessions/events", status, &body)
    }
}
```

- [ ] **Step 2: Add `Client::sessions` + drop the dead-code allow**

In `crates/lens-client/src/client.rs`, add a method to `impl Client` (next to `server_info`):
```rust
    /// The session subservice (`POST /events`, and read methods in later plans).
    pub fn sessions(&self) -> crate::sessions::Sessions<'_> {
        crate::sessions::Sessions::new(self)
    }
```
Then REMOVE the `// Consumed by Plan 2 (REST surface); allow until then.` comment and the `#[allow(dead_code)]` above `conn()` and `http()` — `send_event` now uses both, so the allow is no longer needed. Keep the `pub(crate) fn conn(...)` / `pub(crate) fn http(...)` bodies unchanged.

- [ ] **Step 3: Re-export `Sessions`**

In `crates/lens-client/src/lib.rs`:
```rust
pub use sessions::{ElicitationAction, SendEventAck, Sessions, SessionEventInput};
```

- [ ] **Step 4: Verify it compiles serverless + the allow is truly gone**

Run: `cargo test -p lens-client && cargo clippy -p lens-client --all-targets -- -D warnings && cargo fmt -p lens-client --check`
Expected: PASS — all prior unit tests still green; clippy clean with NO `#[allow(dead_code)]` on the client accessors (they're now used by `send_event`). `generated.rs` unchanged.

- [ ] **Step 5: Write the gated live test**

`crates/lens-client/tests/live_send_event.rs`:
```rust
//! Live test — requires a running omnigent server at $LENS_OMNIGENT_URL and an
//! existing session id in $LENS_OMNIGENT_SESSION_ID (a safe target; the test
//! sends an `Interrupt`, which is harmless on an idle session).
//! Run with: `LENS_OMNIGENT_URL=http://localhost:<port> LENS_OMNIGENT_SESSION_ID=<id> \
//!   cargo test -p lens-client --features live-tests --test live_send_event`
#![cfg(feature = "live-tests")]

use lens_client::ids::{ConnectionId, SessionId};
use lens_client::{Auth, Connection, SessionEventInput};

#[test]
fn interrupt_event_round_trips() {
    let base = std::env::var("LENS_OMNIGENT_URL")
        .expect("set LENS_OMNIGENT_URL")
        .parse()
        .expect("LENS_OMNIGENT_URL is not a valid URL");
    let sid = std::env::var("LENS_OMNIGENT_SESSION_ID")
        .expect("set LENS_OMNIGENT_SESSION_ID to a live session id");

    let conn = Connection::new(ConnectionId::new("live"), base, Auth::None);
    let client = lens_client::Client::new(conn).expect("handshake");
    let ack = client
        .sessions()
        .send_event(&SessionId::new(sid), &SessionEventInput::Interrupt)
        .expect("send_event should return a typed ack");
    // A control event reports queued=false; the point is the ack parsed.
    let _ = ack.queued;
}
```

- [ ] **Step 6: Run the serverless suite (gate dormant)**

Run: `cargo test -p lens-client`
Expected: PASS — the live test is compiled out.

- [ ] **Step 7: Run the live test against the pinned server**

Per `installing-omnigent-from-source`, ensure the daemon is up; `omnigent server status` for the port and a live session id, then:
```bash
LENS_OMNIGENT_URL=http://localhost:<port> LENS_OMNIGENT_SESSION_ID=<id> \
  cargo test -p lens-client --features live-tests --test live_send_event -- --nocapture
```
Expected: PASS — `interrupt_event_round_trips`.

- [ ] **Step 8: Commit**

```bash
git add crates/lens-client/src/sessions.rs crates/lens-client/src/client.rs crates/lens-client/src/lib.rs crates/lens-client/tests/live_send_event.rs
git commit -m "feat(lens-client): Sessions subservice + send_event (gated live test)"
```

---

## Self-review

- **Spec coverage (§6 + the write path):** the typed `SessionEventInput` (Lens-sent subset) ✓ (T1); `to_json` wire shapes verified per type against source — `message` role/content ✓, `function_call_output` call_id/output ✓, `approval` FLAT elicitation_id+action+content? ✓, control events `{}` ✓; full `_ALLOWED_EVENT_TYPES` pinned ✓ (T2); typed `SendEventAck` (real 202 ack, not the openapi `{}`) ✓ (T3); `Sessions` subservice + `send_event` over the foundation's `decode_json` ✓ (T4); D2 sync/blocking honored (no async) ✓; D3 default tests serverless + live gated ✓; the Plan-2 `#[allow(dead_code)]` seam on `Client::conn/http` is consumed and removed ✓.
- **Out of scope here (later 2x plans):** session READ (`list`/`get`/`items`/`child_sessions` — untyped `{}` responses, need the mirror-vs-Value decision), `create`/`patch`/`delete`/`fork`/`switch_agent`, the dedicated elicitation `resolve` endpoint, resources/terminals/comments, hosts/runners/agents, info/me. These are independent and don't block this slice.
- **Type consistency:** `SessionEventInput`/`ElicitationAction`/`SendEventAck`/`Sessions` defined in T1–T4 and re-exported incrementally in `lib.rs`; `discriminator()` strings used in T1 match the pinned set in T2; `send_event` consumes `to_json` (T1), `SendEventAck` (T3), and `http::decode_json` (foundation) with matching signatures; the hand-written `sessions::SessionEventInput` is kept distinct from `generated::SessionEventInput` (only the former is re-exported at the root).
- **Ground-truth:** every discriminator, payload shape, the 30-type set, and the ack fields are cited to omnigent 0.3.0.dev0 source (the research is recorded in the session ledger / `lens-client-foundation-gotchas` memory neighborhood), not inferred.

## Subsequent slices (to plan after this lands)

| Plan | Delivers | Gated on |
|---|---|---|
| 2b — Sessions read | `list`/`get`/`items`/`child_sessions`; resolves the untyped-`{}`-response strategy (hand-written mirrors incl. `ChildSessionSummary` vs `serde_json::Value`) | this slice (Sessions subservice) |
| 2c — Sessions lifecycle | `create` (multipart), `patch`, `delete`, `fork`, `switch_agent`, elicitation `resolve` | 2b |
| 2d — Resources | env-scoped fs/diff/search/shell, files, terminals REST, comments | 2b |
| 2e — Registries | hosts/runners, agents (read-only), policies, `info`/`me` | foundation |
