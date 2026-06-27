# lens-client Event Modeling (post-recapture) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fold the live event-recapture spike's byte findings into `lens-client`'s `ServerStreamEvent` taxonomy — promote three byte-verified families from `DEFERRED → Unknown` to typed `MODELED` variants, grow two under-modeled payloads consumers can't render without, and flip the `SCHEMA-DERIVED` flags on the families the spike byte-verified.

**Architecture:** `lens-client`'s stream taxonomy lives in one file, `crates/lens-client/src/stream/event.rs`. Each wire event maps through a private `Raw*` serde struct (which `#[serde(rename)]`s discarded fields to `_name`) into a public enum variant inside `SessionEvent::from_frame` / `ResponseEvent::from_frame`. Two parallel `&[&str]` lists — `MODELED_EVENT_TYPES` and `DEFERRED_EVENT_TYPES` — partition the pinned contract's discriminator set; the offline `taxonomy_drift` test asserts their disjoint union equals the contract exactly. Nested payloads use a public struct with private fields + getters (the `TodoItem` / `PresenceViewer` pattern); status-like strings use a `#[serde(rename_all=...)]` enum with a `#[serde(other)] Unknown` arm.

**Tech Stack:** Rust (edition 2024), `serde` / `serde_json`, blocking (no tokio). Tests are inline `#[cfg(test)] mod tests` in `event.rs`.

**Ground truth:** every test in this plan uses bytes copied verbatim from the spike corpus `docs/spikes/captures/2026-06-26-live-recapture/` (findings: `docs/spikes/2026-06-26-live-event-recapture.md`). Do not invent payloads.

## Global Constraints

- **No `Value` to consumers** — every exposed field is typed; discarded wire fields are parsed into `_name`-prefixed `Raw*` struct fields, never surfaced.
- **Nested payloads** = public struct, private fields, pub `&self` getters (match `TodoItem` at `event.rs:146-161`). Status strings = enum with `#[serde(rename_all)]` + `#[serde(other)] Unknown`.
- **`generated.rs` is never touched.**
- **Degrade-don't-panic:** a `session.*`/`response.*` type whose body fails to deserialize returns `None` from `from_frame` → `parse_event` yields `Unknown`. Never `unwrap`/`expect` on wire data.
- **Comments explain why / the non-obvious; never narrate code** (AGENTS.md). Cite the corpus file in byte-verified test comments.
- **Every task ends green:** `cargo test -p lens-client && cargo clippy -p lens-client --all-targets -- -D warnings && cargo fmt -p lens-client -- --check`. The offline `taxonomy_drift` test must stay green after every list move.
- **Promoting a type to `MODELED` requires three coordinated edits**, all in the same task: (a) a typed enum variant + match arm, (b) move its string from `DEFERRED_EVENT_TYPES` to `MODELED_EVENT_TYPES` (alphabetical order), (c) a byte-grounded test. Skipping (b) leaves `taxonomy_drift` green but lets the live stream see the type as `Unknown` undetected; skipping (a) fails `taxonomy_drift` (MODELED with no arm still works, but the variant must exist to be useful).
- **Scope boundary — explicitly NOT in this plan** (still environmentally unverified per the spike, keep their `SCHEMA-DERIVED`/`DEFERRED` flags): `turn.*`, `response.created`, `response.queued`, `response.retry`, `response.client_task.cancel`, `response.output_file.done`, `response.heartbeat` (byte-seen but a low-value keepalive — left `DEFERRED` deliberately), `session.collaboration_mode`, `response.reasoning_summary_text.delta`, `response.compaction.completed`, `response.compaction.failed`, `response.incomplete`, `session.terminal_pending`, `session.model_options`, `session.sandbox_status`.

---

## File Structure

- `crates/lens-client/src/stream/event.rs` — **all** taxonomy changes (enums, `Raw*` structs, match arms, the two `*_EVENT_TYPES` lists, inline tests). Single file by design; do not split.
- `crates/lens-client/tests/taxonomy_drift.rs` — **read-only** here; it must keep passing (no edits expected — the contract set is unchanged, only the MODELED/DEFERRED partition moves).
- `docs/design/typed-client.md` — §7 doc reconciliation (Task 7): `session.terminal.activity` is an SSE event (not WS/Plan-7); record the still-blocked families.

---

### Task 1: Promote `session.agent_changed` to a typed variant

**Files:**
- Modify: `crates/lens-client/src/stream/event.rs` (SessionEvent enum, Raw structs, `SessionEvent::from_frame`, both `*_EVENT_TYPES` lists, tests)

**Interfaces:**
- Produces: `SessionEvent::AgentChanged { agent_id: String, agent_name: String }`
- Bytes (corpus `agent-switched.sse`): `{"sequence_number": null, "type": "session.agent_changed", "conversation_id": "...", "agent_id": "ag_...", "agent_name": "debby"}`

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` block in `event.rs`:

```rust
#[test]
fn bytes_session_agent_changed() {
    // Byte-verified: docs/spikes/captures/2026-06-26-live-recapture/agent-switched.sse
    let ev = parse_event(&frame(
        "session.agent_changed",
        r#"{"sequence_number":null,"type":"session.agent_changed","conversation_id":"conv_2a9","agent_id":"ag_2e9","agent_name":"debby"}"#,
    ));
    assert_eq!(
        ev,
        ServerStreamEvent::Session(SessionEvent::AgentChanged {
            agent_id: "ag_2e9".into(),
            agent_name: "debby".into(),
        })
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p lens-client bytes_session_agent_changed`
Expected: FAIL — `no variant ... AgentChanged` (compile error).

- [ ] **Step 3: Add the variant, Raw struct, match arm, and list move**

In `SessionEvent` (after the `Skills` variant, before the closing `}` at ~line 110), add:

```rust
    AgentChanged {
        agent_id: String,
        agent_name: String,
    },
```

Add a `Raw*` struct near the other session raws (after `RawSessionSandboxStatus`, ~line 345):

```rust
#[derive(Deserialize)]
struct RawAgentChanged {
    agent_id: String,
    agent_name: String,
    #[serde(rename = "conversation_id")]
    _conversation_id: String,
}
```

Add the match arm in `SessionEvent::from_frame` (before `_ => return None,` at ~line 672):

```rust
            "session.agent_changed" => {
                let r: RawAgentChanged = serde_json::from_str(d).ok()?;
                SessionEvent::AgentChanged {
                    agent_id: r.agent_id,
                    agent_name: r.agent_name,
                }
            }
```

In `MODELED_EVENT_TYPES`, add `"session.agent_changed",` in alphabetical position (between `"response.reasoning_text.delta"`'s session block start and `"session.changed_files.invalidated"` — i.e. as the first `session.*` entry). In `DEFERRED_EVENT_TYPES`, delete the `"session.agent_changed",` line.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p lens-client`
Expected: PASS — `bytes_session_agent_changed` and `accounted_event_types_match_pinned_contract` both green (the contract union is unchanged; the partition moved).

- [ ] **Step 5: Lint + format**

Run: `cargo clippy -p lens-client --all-targets -- -D warnings && cargo fmt -p lens-client`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/lens-client/src/stream/event.rs
git commit -m "feat(lens-client): type session.agent_changed (byte-verified, was DEFERRED)"
```

---

### Task 2: Promote `session.created` (child-spawn) to a typed variant

**Files:**
- Modify: `crates/lens-client/src/stream/event.rs`

**Interfaces:**
- Produces: `SessionEvent::Created { child_session_id: String, agent_id: String, parent_session_id: String }`
- Bytes (corpus `polly-child-session.sse`): `{"sequence_number": null, "type": "session.created", "conversation_id": "...", "child_session_id": "conv_...", "agent_id": "ag_...", "parent_session_id": "conv_..."}`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn bytes_session_created_child() {
    // Byte-verified: docs/spikes/captures/2026-06-26-live-recapture/polly-child-session.sse
    let ev = parse_event(&frame(
        "session.created",
        r#"{"sequence_number":null,"type":"session.created","conversation_id":"conv_parent","child_session_id":"conv_child","agent_id":"ag_b","parent_session_id":"conv_parent"}"#,
    ));
    assert_eq!(
        ev,
        ServerStreamEvent::Session(SessionEvent::Created {
            child_session_id: "conv_child".into(),
            agent_id: "ag_b".into(),
            parent_session_id: "conv_parent".into(),
        })
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p lens-client bytes_session_created_child`
Expected: FAIL — `no variant ... Created`.

- [ ] **Step 3: Add variant, Raw struct, match arm, list move**

Variant (after `AgentChanged`):

```rust
    Created {
        child_session_id: String,
        agent_id: String,
        parent_session_id: String,
    },
```

Raw struct (near the other session raws):

```rust
#[derive(Deserialize)]
struct RawSessionCreated {
    child_session_id: String,
    agent_id: String,
    parent_session_id: String,
    #[serde(rename = "conversation_id")]
    _conversation_id: String,
}
```

Match arm (in `SessionEvent::from_frame`):

```rust
            "session.created" => {
                let r: RawSessionCreated = serde_json::from_str(d).ok()?;
                SessionEvent::Created {
                    child_session_id: r.child_session_id,
                    agent_id: r.agent_id,
                    parent_session_id: r.parent_session_id,
                }
            }
```

Move `"session.created"` from `DEFERRED_EVENT_TYPES` to `MODELED_EVENT_TYPES` (alphabetical — after `"session.changed_files.invalidated"` / `"session.child_session.updated"`, before `"session.heartbeat"`).

- [ ] **Step 4: Run tests**

Run: `cargo test -p lens-client`
Expected: PASS (incl. `taxonomy_drift`).

- [ ] **Step 5: Lint + format**

Run: `cargo clippy -p lens-client --all-targets -- -D warnings && cargo fmt -p lens-client`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/lens-client/src/stream/event.rs
git commit -m "feat(lens-client): type session.created child-spawn (byte-verified, was DEFERRED)"
```

---

### Task 3: Promote `session.resource.deleted` to a typed variant

**Files:**
- Modify: `crates/lens-client/src/stream/event.rs`

**Interfaces:**
- Produces: `SessionEvent::ResourceDeleted { resource_id: String, resource_type: String }`
- Bytes (corpus `agent-switched.sse`): `{"sequence_number": null, "type": "session.resource.deleted", "resource_id": "terminal_tui_main", "resource_type": "terminal", "session_id": "conv_..."}`
- Note: `ResourceCreated` stays a unit variant (separate concern; not byte-grown here).

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn bytes_session_resource_deleted() {
    // Byte-verified: docs/spikes/captures/2026-06-26-live-recapture/agent-switched.sse
    let ev = parse_event(&frame(
        "session.resource.deleted",
        r#"{"sequence_number":null,"type":"session.resource.deleted","resource_id":"terminal_tui_main","resource_type":"terminal","session_id":"conv_2a9"}"#,
    ));
    assert_eq!(
        ev,
        ServerStreamEvent::Session(SessionEvent::ResourceDeleted {
            resource_id: "terminal_tui_main".into(),
            resource_type: "terminal".into(),
        })
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p lens-client bytes_session_resource_deleted`
Expected: FAIL — `no variant ... ResourceDeleted`.

- [ ] **Step 3: Add variant, Raw struct, match arm, list move**

Variant (after `ResourceCreated` at ~line 67):

```rust
    ResourceDeleted {
        resource_id: String,
        resource_type: String,
    },
```

Raw struct:

```rust
#[derive(Deserialize)]
struct RawResourceDeleted {
    resource_id: String,
    resource_type: String,
    #[serde(rename = "session_id")]
    _session_id: String,
}
```

Match arm (place it next to `"session.resource.created"`):

```rust
            "session.resource.deleted" => {
                let r: RawResourceDeleted = serde_json::from_str(d).ok()?;
                SessionEvent::ResourceDeleted {
                    resource_id: r.resource_id,
                    resource_type: r.resource_type,
                }
            }
```

Move `"session.resource.deleted"` from `DEFERRED_EVENT_TYPES` to `MODELED_EVENT_TYPES` (after `"session.resource.created"`).

- [ ] **Step 4: Run tests**

Run: `cargo test -p lens-client`
Expected: PASS.

- [ ] **Step 5: Lint + format**

Run: `cargo clippy -p lens-client --all-targets -- -D warnings && cargo fmt -p lens-client`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/lens-client/src/stream/event.rs
git commit -m "feat(lens-client): type session.resource.deleted (byte-verified, was DEFERRED)"
```

---

### Task 4: Grow `session.child_session.updated` to expose the `child{}` object

**Files:**
- Modify: `crates/lens-client/src/stream/event.rs` (variant, new `ChildSession` struct + `ChildTaskStatus` enum, `RawChildSessionUpdated`, match arm, update existing test, mod re-export)
- Modify: `crates/lens-client/src/stream/mod.rs` (re-export `ChildSession`, `ChildTaskStatus`)

**Interfaces:**
- Consumes: nothing from prior tasks.
- Produces: `SessionEvent::ChildSessionUpdated { child_session_id: String, child: ChildSession }` where `ChildSession` exposes `id()/title()/tool()/session_name() -> &str`, `busy() -> bool`, `current_task_status() -> ChildTaskStatus`; `enum ChildTaskStatus { Launching, InProgress, Completed, Unknown }`.
- Bytes (corpus `polly-child-session.sse`): `child` = `{"id":"conv_...","title":"claude_code:spike-hello-file","tool":"claude_code","session_name":"spike-hello-file","busy":false,"current_task_status":"launching"}`. Status progression observed: `launching → in_progress → completed`.

- [ ] **Step 1: Write the failing test (and fix the existing stale one)**

Replace the existing `schema_child_session_updated` test (it asserts the old flat shape with `child:{}`) with the byte-grounded version:

```rust
#[test]
fn bytes_child_session_updated() {
    // Byte-verified: docs/spikes/captures/2026-06-26-live-recapture/polly-child-session.sse
    let ev = parse_event(&frame(
        "session.child_session.updated",
        r#"{"sequence_number":null,"type":"session.child_session.updated","conversation_id":"conv_parent","child_session_id":"conv_child","child":{"id":"conv_child","title":"claude_code:spike-hello-file","tool":"claude_code","session_name":"spike-hello-file","busy":false,"current_task_status":"launching"}}"#,
    ));
    let ServerStreamEvent::Session(SessionEvent::ChildSessionUpdated { child_session_id, child }) = ev
    else {
        panic!("expected ChildSessionUpdated, got {ev:?}");
    };
    assert_eq!(child_session_id, "conv_child");
    assert_eq!(child.id(), "conv_child");
    assert_eq!(child.title(), "claude_code:spike-hello-file");
    assert_eq!(child.tool(), "claude_code");
    assert_eq!(child.session_name(), "spike-hello-file");
    assert!(!child.busy());
    assert_eq!(child.current_task_status(), ChildTaskStatus::Launching);
}

#[test]
fn child_task_status_unknown_for_novel_value() {
    // dev0 churn safety: an unknown status string degrades, never panics.
    let ev = parse_event(&frame(
        "session.child_session.updated",
        r#"{"sequence_number":null,"type":"session.child_session.updated","conversation_id":"c","child_session_id":"cc","child":{"id":"cc","title":"t","tool":"claude_code","session_name":"n","busy":true,"current_task_status":"some_future_state"}}"#,
    ));
    let ServerStreamEvent::Session(SessionEvent::ChildSessionUpdated { child, .. }) = ev else {
        panic!("expected ChildSessionUpdated");
    };
    assert_eq!(child.current_task_status(), ChildTaskStatus::Unknown);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p lens-client bytes_child_session_updated child_task_status_unknown`
Expected: FAIL — `ChildSession`/`ChildTaskStatus` undefined; `ChildSessionUpdated` has no `child` field.

- [ ] **Step 3: Add `ChildTaskStatus`, `ChildSession`, grow the variant + raw + arm**

Add the status enum near `TodoItemStatus` (~line 136):

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChildTaskStatus {
    Launching,
    InProgress,
    Completed,
    /// Any status this crate version does not know (dev0 churn safety).
    #[serde(other)]
    Unknown,
}
```

Add the public struct (getters; mirrors `TodoItem`) near `TodoItem` (~line 146):

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct ChildSession {
    id: String,
    title: String,
    tool: String,
    session_name: String,
    busy: bool,
    current_task_status: ChildTaskStatus,
}
impl ChildSession {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn title(&self) -> &str {
        &self.title
    }
    pub fn tool(&self) -> &str {
        &self.tool
    }
    pub fn session_name(&self) -> &str {
        &self.session_name
    }
    pub fn busy(&self) -> bool {
        self.busy
    }
    pub fn current_task_status(&self) -> ChildTaskStatus {
        self.current_task_status
    }
}
```

Grow the variant (replace the existing `ChildSessionUpdated { child_session_id }` at ~line 79; also delete its `// SCHEMA-DERIVED` comment):

```rust
    ChildSessionUpdated {
        child_session_id: String,
        child: ChildSession,
    },
```

Replace `RawChildSessionUpdated` (~line 288) — parse `child` into a typed raw instead of discarding it:

```rust
#[derive(Deserialize)]
struct RawChild {
    id: String,
    title: String,
    tool: String,
    session_name: String,
    busy: bool,
    current_task_status: ChildTaskStatus,
}
#[derive(Deserialize)]
struct RawChildSessionUpdated {
    child_session_id: String,
    child: RawChild,
    #[serde(rename = "conversation_id")]
    _conversation_id: String,
}
```

Update the match arm (~line 609; drop its `// SCHEMA-DERIVED` comment):

```rust
            "session.child_session.updated" => {
                let r: RawChildSessionUpdated = serde_json::from_str(d).ok()?;
                SessionEvent::ChildSessionUpdated {
                    child_session_id: r.child_session_id,
                    child: ChildSession {
                        id: r.child.id,
                        title: r.child.title,
                        tool: r.child.tool,
                        session_name: r.child.session_name,
                        busy: r.child.busy,
                        current_task_status: r.child.current_task_status,
                    },
                }
            }
```

- [ ] **Step 4: Re-export the new public types**

In `crates/lens-client/src/stream/mod.rs`, add `ChildSession, ChildTaskStatus` to the `pub use event::{...}` list (alphabetical):

```rust
pub use event::{
    ChildSession, ChildTaskStatus, DEFERRED_EVENT_TYPES, DisconnectReason, Item,
    MODELED_EVENT_TYPES, MessageContentBlock, PresenceViewer, ResponseEvent, ServerStreamEvent,
    SessionEvent, SessionStatusValue,
};
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p lens-client`
Expected: PASS (new tests + the rest). No `taxonomy_drift` change (`session.child_session.updated` was already `MODELED`).

- [ ] **Step 6: Lint + format**

Run: `cargo clippy -p lens-client --all-targets -- -D warnings && cargo fmt -p lens-client`
Expected: clean (the previously-discarded `_child` warning is gone).

- [ ] **Step 7: Commit**

```bash
git add crates/lens-client/src/stream/event.rs crates/lens-client/src/stream/mod.rs
git commit -m "feat(lens-client): expose child{} on child_session.updated (byte-verified)"
```

---

### Task 5: Grow `response.elicitation_request` to expose `params`

**Files:**
- Modify: `crates/lens-client/src/stream/event.rs` (variant, `ElicitationParams` struct, `RawElicitationParams`/`RawElicitationRequest`, match arm, test)
- Modify: `crates/lens-client/src/stream/mod.rs` (re-export `ElicitationParams`)

**Interfaces:**
- Produces: `ResponseEvent::ElicitationRequest { elicitation_id: String, params: ElicitationParams }` where `ElicitationParams` exposes `message()/policy_name()/phase()/content_preview()/url()/mode() -> &str`.
- Bytes (corpus `elicitation-request.sse`): `params` = `{"mode":"url","message":"approve_file_ops: Agent wants to call sys_os_write('/tmp/spike_elicit.txt'). Approve?","requestedSchema":{},"url":"/approve/conv_.../elicit_...","phase":"tool_call","policy_name":"approve_file_ops","content_preview":"{\"path\": ...}","target_session_id":null}`. (`requestedSchema` is `{}` here — discard; `target_session_id` is nullable — discard for v1, it's the parent-mirror hint.)

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn bytes_elicitation_request_params() {
    // Byte-verified: docs/spikes/captures/2026-06-26-live-recapture/elicitation-request.sse
    let ev = parse_event(&frame(
        "response.elicitation_request",
        r#"{"sequence_number":null,"type":"response.elicitation_request","elicitation_id":"elicit_17f","method":"elicitation/create","params":{"mode":"url","message":"approve_file_ops: Agent wants to call sys_os_write('/tmp/spike_elicit.txt'). Approve?","requestedSchema":{},"url":"/approve/conv_78/elicit_17f","phase":"tool_call","policy_name":"approve_file_ops","content_preview":"{\"path\": \"/tmp/spike_elicit.txt\"}","target_session_id":null}}"#,
    ));
    let ServerStreamEvent::Response(ResponseEvent::ElicitationRequest { elicitation_id, params }) = ev
    else {
        panic!("expected ElicitationRequest, got {ev:?}");
    };
    assert_eq!(elicitation_id, "elicit_17f");
    assert_eq!(params.policy_name(), "approve_file_ops");
    assert_eq!(params.phase(), "tool_call");
    assert_eq!(params.mode(), "url");
    assert!(params.message().contains("Approve?"));
    assert!(params.content_preview().contains("spike_elicit.txt"));
    assert_eq!(params.url(), "/approve/conv_78/elicit_17f");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p lens-client bytes_elicitation_request_params`
Expected: FAIL — `ElicitationParams` undefined; variant has no `params` field.

- [ ] **Step 3: Add `ElicitationParams`, replace the raws, grow the variant + arm**

Add the public struct (getters) near the other public payload structs:

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct ElicitationParams {
    mode: String,
    message: String,
    url: String,
    phase: String,
    policy_name: String,
    content_preview: String,
}
impl ElicitationParams {
    pub fn mode(&self) -> &str {
        &self.mode
    }
    pub fn message(&self) -> &str {
        &self.message
    }
    pub fn url(&self) -> &str {
        &self.url
    }
    pub fn phase(&self) -> &str {
        &self.phase
    }
    pub fn policy_name(&self) -> &str {
        &self.policy_name
    }
    pub fn content_preview(&self) -> &str {
        &self.content_preview
    }
}
```

Replace `RawElicitationParams` (~line 273) — capture the surfaced fields (and tolerate omissions: native-harness elicitations may omit `phase`/`policy_name`, so `#[serde(default)]` them):

```rust
#[derive(Deserialize)]
struct RawElicitationParams {
    #[serde(default)]
    mode: String,
    message: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    phase: String,
    #[serde(default)]
    policy_name: String,
    #[serde(default)]
    content_preview: String,
}
```

(`RawElicitationRequest` at ~line 277 keeps `elicitation_id` + `#[serde(rename="params")] params: RawElicitationParams` — but rename the field from `_params` to `params` since it is now used.)

```rust
#[derive(Deserialize)]
struct RawElicitationRequest {
    elicitation_id: String,
    params: RawElicitationParams,
}
```

Grow the variant (~line 401; drop its `// SCHEMA-DERIVED` comment):

```rust
    ElicitationRequest {
        elicitation_id: String,
        params: ElicitationParams,
    },
```

Update the match arm (~line 736; drop its `// SCHEMA-DERIVED` comment):

```rust
            "response.elicitation_request" => {
                let r: RawElicitationRequest = serde_json::from_str(d).ok()?;
                ResponseEvent::ElicitationRequest {
                    elicitation_id: r.elicitation_id,
                    params: ElicitationParams {
                        mode: r.params.mode,
                        message: r.params.message,
                        url: r.params.url,
                        phase: r.params.phase,
                        policy_name: r.params.policy_name,
                        content_preview: r.params.content_preview,
                    },
                }
            }
```

- [ ] **Step 4: Re-export + fix any existing elicitation test**

Add `ElicitationParams` to the `pub use event::{...}` in `mod.rs`. If an existing test constructs `ElicitationRequest { elicitation_id: ... }` without `params` (search: `grep -n "ElicitationRequest {" crates/lens-client/src/stream/event.rs`), update it to the byte-grounded form above or pattern-match with `..`.

- [ ] **Step 5: Run tests**

Run: `cargo test -p lens-client`
Expected: PASS.

- [ ] **Step 6: Lint + format**

Run: `cargo clippy -p lens-client --all-targets -- -D warnings && cargo fmt -p lens-client`
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add crates/lens-client/src/stream/event.rs crates/lens-client/src/stream/mod.rs
git commit -m "feat(lens-client): expose elicitation params (byte-verified)"
```

---

### Task 6: Flip `SCHEMA-DERIVED` flags on the byte-verified families

**Files:**
- Modify: `crates/lens-client/src/stream/event.rs` (comments + test comments only — no behavior change)

**Interfaces:** none (comment-only task; pure documentation hygiene so the next reader trusts which variants are byte-grounded vs still-guessed).

The spike byte-verified these families that carry a `// SCHEMA-DERIVED (not byte-verified — re-capture at config-time)` comment on their **variant** and/or **match arm**. Remove the comment on *exactly* these (and only these), on both the enum-variant site and the `from_frame` match-arm site:

- `SessionEvent`: `TerminalActivity`, `Model`, `Todos`, `ReasoningEffort`, `Skills`, `ChildSessionUpdated` (already de-flagged in Task 4 — verify), plus the three promoted in Tasks 1–3 (no flag to remove, they were `DEFERRED`).
- `ResponseEvent`: `Failed`, `Cancelled`, `ReasoningTextDelta`, `CompactionInProgress`, `ElicitationRequest` (de-flagged in Task 5 — verify), `ElicitationResolved`, `Error`.

**Leave the `SCHEMA-DERIVED` comment intact** on the still-unverified (per Global Constraints scope boundary): `SessionEvent::{TerminalPending, ModelOptions, SandboxStatus}`, `ResponseEvent::{Incomplete, ReasoningSummaryTextDelta, CompactionCompleted, CompactionFailed}`, and the synthetic `ReasoningClosed` (keep its "NOT BYTE-VERIFIED" note — claude folds reasoning; it remains synthetic).

- [ ] **Step 1: Rename the stale `schema_*` tests to `bytes_*` for the de-flagged families**

For each now-byte-verified family with an existing `schema_*` test (e.g. `schema_session_todos`, `schema_session_model`, `schema_terminal_activity`, `schema_response_cancelled`, `schema_elicitation_resolved`, `schema_response_error`, `schema_reasoning_text_delta` — confirm names with `grep -n "fn schema_" crates/lens-client/src/stream/event.rs`), rename the function to `bytes_<same>` and replace its `// SCHEMA-DERIVED.` comment with `// Byte-verified: docs/spikes/captures/2026-06-26-live-recapture/<file>.sse`. Pick the corpus file from the findings doc's taxonomy table. Do **not** change the assertion bodies (the shapes already match the wire).

- [ ] **Step 2: Remove the variant + match-arm `SCHEMA-DERIVED` comments**

Delete the `// SCHEMA-DERIVED (not byte-verified — re-capture at config-time)` line above each de-flagged variant and match arm listed above. Leave the still-unverified ones.

- [ ] **Step 3: Run the full suite**

Run: `cargo test -p lens-client`
Expected: PASS (comment/name changes only).

- [ ] **Step 4: Lint + format**

Run: `cargo clippy -p lens-client --all-targets -- -D warnings && cargo fmt -p lens-client`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add crates/lens-client/src/stream/event.rs
git commit -m "docs(lens-client): flip SCHEMA-DERIVED flags to byte-verified for recaptured families"
```

---

### Task 7: Reconcile typed-client §7 + whole-crate verification

**Files:**
- Modify: `docs/design/typed-client.md` (§7 event taxonomy / classification)

**Interfaces:** none (doc + verification).

- [ ] **Step 1: Update typed-client §7**

In `docs/design/typed-client.md` §7, make these edits (search for the relevant lines):
- `session.terminal.activity`: note it is delivered on the **SSE stream** (byte-verified 2026-06-26), not via the WS terminal attach — drop any "Plan 7 / WS-only" framing for *activity* (terminal *content* may still be WS).
- Mark byte-verified (cite `docs/spikes/2026-06-26-live-event-recapture.md`): `agent_changed`, `session.created` (child), `resource.deleted`, `child_session.updated` (now carries `child{}`), `elicitation_request` (now carries `params`), plus `reasoning_text.delta`, `model`, `reasoning_effort`, `todos`, `cancelled`, `interrupted`, `compaction.in_progress`.
- Record the still-blocked families and *why* (one line): `turn.*` (codex-native only), `response.created`/`queued` (openai-scaffold / runner-deferred), `reasoning_summary_text.delta` (codex), `compaction.completed` (needs a configured `llm_model`).

- [ ] **Step 2: Whole-crate verification**

Run each and confirm:

```bash
cargo test -p lens-client
cargo clippy -p lens-client --all-targets -- -D warnings
cargo fmt -p lens-client -- --check
cargo run -p xtask -- drift
git status --porcelain crates/lens-client/src/generated.rs   # expect: empty (untouched)
```

Expected: all tests pass; clippy/fmt clean; `xtask drift` green; `generated.rs` shows no diff.

- [ ] **Step 3: Commit**

```bash
git add docs/design/typed-client.md
git commit -m "docs(typed-client): reconcile §7 with live recapture (terminal.activity is SSE; blocked families)"
```

---

## Notes for the executor

- **Cross-family review** (AGENTS.md / CLAUDE.md): this is taxonomy/shape work — route one consolidated review through `cursor-delegate` to a family other than the author's (`gpt-5.5` or `gemini-3.5`) at the end, per `[[review-spend-policy]]`. The byte-grounded tests are the ground truth a reviewer checks against.
- **A live re-verify is optional** but cheap if a server is up: `cargo test -p lens-client --features live-tests live_taxonomy` against a running `0.3.0.dev0` confirms the promoted types arrive typed (not `Unknown`). Not required for completion (the offline `taxonomy_drift` + byte tests cover it).
- **If `taxonomy_drift` fails** with "unaccounted" after a list move, you deleted from one list without adding to the other. If "listed as both", you added without deleting.
