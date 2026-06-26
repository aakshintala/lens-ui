# lens-client Plan 2c — Sessions lifecycle (create/patch/delete/fork/switch-agent/elicitation)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans. Steps use checkbox (`- [ ]`).

**Goal:** The session **mutation** surface — create (JSON + multipart bundle), patch, delete, fork, switch-agent (+ bundle upload), and elicitation fetch/resolve — on the `Sessions` subservice.

**Architecture:** Extends Plans 2a/2b. Writes use the **generated** request types where they exist (`UpdateSessionRequest`, `SessionForkRequest`, `SessionSwitchAgentRequest`, `ElicitationResult`, `SessionGitOptions`); two responses are hand-modeled (`ConversationDeleted`, `CreatedSessionResponse`) and one request (`CreateSessionRequest`, the JSON create body, which omnigent does not expose in `openapi.json` components). `patch`/`fork`/`switch-agent` all return a `SessionSnapshot` (Plan 2b). A `Client::send_json` helper (POST/PATCH/DELETE with optional JSON body) keeps methods to a few lines; multipart needs the `reqwest` `multipart` feature.

**Tech Stack:** Rust (edition 2024), `reqwest` blocking (+ `multipart`), `serde`. No async (D2).

## Global Constraints

- Plan 2a/2b constraints apply. No `serde_json::Value` in public signatures; reuse `http::decode_json`; `generated.rs` never hand-edited; live tests gated.
- **Ground truth (omnigent `0.3.0.dev0`, `36b2a11c`):**
  - `POST /v1/sessions` is content-type-split (`sessions.py:12605-12639`): `application/json` body = `SessionCreateRequest` → returns full `SessionResponse` (snapshot); `multipart/form-data` (fields `metadata` JSON-string=`SessionCreateMetadata` + `bundle` file, BOTH required, `sessions.py:12926-12934`) → returns `CreatedSessionResponse {session_id, agent_id, agent_name}` (`schemas.py:1289-1291`).
  - Minimal JSON create body: `{"agent_id": "<ag_…>"}` (`schemas.py:1142-1155`); other fields default. JSON extras: `host_type: "external"|"managed"` (default `"external"`), `host_id?`, `workspace?`, `git: {branch_name (req), base_branch?}`, `initial_items?`.
  - `PATCH /v1/sessions/{id}` body `UpdateSessionRequest` → full `SessionResponse` (`sessions.py:13613-13632`).
  - `DELETE /v1/sessions/{id}?delete_branch=<bool>` → `ConversationDeleted {id, object:"conversation.deleted", deleted:true}` (`schemas.py:531-544`).
  - `POST /v1/sessions/{source_id}/fork` (path param is **`source_id`**) body `SessionForkRequest` → new `SessionResponse` (status idle) (`sessions.py:13990-14019`).
  - `POST /v1/sessions/{id}/switch-agent` body `SessionSwitchAgentRequest {agent_id (req)}` → full `SessionResponse` (`sessions.py:14181-14206`).
  - `PUT /v1/sessions/{id}/agent` multipart (`Body_update_session_agent_…`) → `AgentObject` (bundle **storage only**, idempotent, does NOT fire `session.agent_changed`).
  - `GET /v1/sessions/{id}/elicitations/{elicitation_id}` → untyped `{}` pending state. `POST …/resolve` body `ElicitationResult {action: accept|decline|cancel, content?}` → 202 ack.
  - Generated types present (see `generated.rs`): `UpdateSessionRequest`, `SessionForkRequest`, `SessionSwitchAgentRequest`, `ElicitationResult`, `SessionGitOptions`, `AgentObject`.

---

### Task 1: `send_json` helper + `reqwest` multipart feature

**Files:** Modify `crates/lens-client/src/client.rs`, `crates/lens-client/Cargo.toml`.

**Interfaces:** Produces `Client::send_json<T, B>(&self, method, path, query, body: Option<&B>) -> Result<T>` and `Client::send_multipart<T>(&self, method, path, form) -> Result<T>`.

- [ ] **Step 1** — In `Cargo.toml`, add `"multipart"` to the `reqwest` features list (now `["blocking", "json", "rustls-tls", "multipart"]`).
- [ ] **Step 2** — Add to `impl Client` in `client.rs`:
```rust
    /// Send a request with an optional JSON body, mapping status → typed errors.
    /// `body: None::<&()>` for verbs without a body.
    pub(crate) fn send_json<T, B>(
        &self,
        method: reqwest::Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<&B>,
    ) -> crate::error::Result<T>
    where
        T: serde::de::DeserializeOwned,
        B: serde::Serialize,
    {
        let url = self.conn().url(path)?;
        let mut rb = self.http().request(method, url).query(query);
        if let Some(b) = body {
            rb = rb.json(b);
        }
        let resp = self.conn().auth.apply(rb).send()?;
        let status = resp.status().as_u16();
        let text = resp.text()?;
        crate::http::decode_json(path, status, &text)
    }

    /// Send a multipart/form-data request (bundle uploads).
    pub(crate) fn send_multipart<T: serde::de::DeserializeOwned>(
        &self,
        method: reqwest::Method,
        path: &str,
        form: reqwest::blocking::multipart::Form,
    ) -> crate::error::Result<T> {
        let url = self.conn().url(path)?;
        let rb = self.http().request(method, url).multipart(form);
        let resp = self.conn().auth.apply(rb).send()?;
        let status = resp.status().as_u16();
        let text = resp.text()?;
        crate::http::decode_json(path, status, &text)
    }
```
- [ ] **Step 3** — Verify `cargo build -p lens-client` (multipart feature resolves). The helpers are `pub(crate)`; if clippy flags them unused before Task 2, commit Tasks 1+2 together (or temporary `#[allow(dead_code)]` removed in Task 2).
- [ ] **Step 4: Commit** `git commit -m "feat(lens-client): send_json/send_multipart helpers + reqwest multipart"` (stage client.rs + Cargo.toml).

---

### Task 2: `Sessions::create` (JSON path)

**Files:** Modify `sessions.rs`, `lib.rs`.

**Interfaces:** Produces `sessions::HostType` (`External`/`Managed`), `sessions::CreateSessionRequest` (serde `Serialize`: `agent_id` required + optional `host_type`, `host_id`, `workspace`, `git`, `initial_items`), and `Sessions::create(&self, req: &CreateSessionRequest) -> Result<SessionSnapshot>`.

- [ ] **Step 1: Failing test** (serialization is the testable surface):
```rust
    #[test]
    fn create_request_serializes_minimal_and_full() {
        use serde_json::json;
        let min = CreateSessionRequest::new("ag_1");
        assert_eq!(serde_json::to_value(&min).unwrap(), json!({"agent_id": "ag_1"}));

        let full = CreateSessionRequest::new("ag_1")
            .host_type(HostType::Managed)
            .host_id("host_9")
            .git("feature/x", Some("main"));
        let v = serde_json::to_value(&full).unwrap();
        assert_eq!(v["agent_id"], json!("ag_1"));
        assert_eq!(v["host_type"], json!("managed"));
        assert_eq!(v["host_id"], json!("host_9"));
        assert_eq!(v["git"], json!({"branch_name": "feature/x", "base_branch": "main"}));
    }
```
- [ ] **Step 2: Run** → FAIL.
- [ ] **Step 3: Implement** in `sessions.rs` (reuse `generated::SessionGitOptions` for the git body, or hand-model inline — here inline for a clean owned builder):
```rust
/// Host placement for a new session.
#[derive(Clone, Copy, Debug, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HostType { External, Managed }

/// JSON body for `POST /v1/sessions` (omnigent `SessionCreateRequest`,
/// `schemas.py:1038-1155`). Not in `openapi.json` components — hand-written.
/// Only `agent_id` is required; unset fields are omitted (server defaults apply).
#[derive(Clone, Debug, serde::Serialize)]
pub struct CreateSessionRequest {
    agent_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    host_type: Option<HostType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    host_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    workspace: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    git: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    initial_items: Option<Vec<serde_json::Value>>,
}

impl CreateSessionRequest {
    pub fn new(agent_id: impl Into<String>) -> Self {
        Self { agent_id: agent_id.into(), host_type: None, host_id: None, workspace: None, git: None, initial_items: None }
    }
    pub fn host_type(mut self, h: HostType) -> Self { self.host_type = Some(h); self }
    pub fn host_id(mut self, v: impl Into<String>) -> Self { self.host_id = Some(v.into()); self }
    pub fn workspace(mut self, v: impl Into<String>) -> Self { self.workspace = Some(v.into()); self }
    pub fn git(mut self, branch_name: impl Into<String>, base_branch: Option<&str>) -> Self {
        let mut g = serde_json::Map::new();
        g.insert("branch_name".into(), serde_json::json!(branch_name.into()));
        if let Some(b) = base_branch { g.insert("base_branch".into(), serde_json::json!(b)); }
        self.git = Some(serde_json::Value::Object(g));
        self
    }
    /// `initial_items` are `SessionEventInput`-shaped; build via `SessionEventInput::to_json`.
    pub fn initial_items(mut self, items: Vec<serde_json::Value>) -> Self { self.initial_items = Some(items); self }
}
```
and to `impl<'a> Sessions<'a>`:
```rust
    /// `POST /v1/sessions` (JSON) — create a session against an existing agent.
    pub fn create(&self, req: &CreateSessionRequest) -> Result<SessionSnapshot> {
        self.client.send_json(reqwest::Method::POST, "/v1/sessions", &[], Some(req))
    }
```
- [ ] **Step 4: Re-export** `HostType`, `CreateSessionRequest`. **Step 5: Verify. Step 6: Commit** `git commit -m "feat(lens-client): Sessions::create (JSON)"`.

> **Note on `initial_items`:** the field is `Vec<serde_json::Value>` because the values are `SessionEventInput` wire shapes produced by `SessionEventInput::to_json` (Plan 2a) — a typed input the caller already holds, not raw JSON they parse. This is the one place a `Value` appears in a *request builder* (not a read), and it is fed by the typed enum.

---

### Task 3: `Sessions::create_with_bundle` (multipart)

**Files:** Modify `sessions.rs`, `lib.rs`.

**Interfaces:** Produces `sessions::CreatedSessionResponse { session_id: SessionId, agent_id: String, agent_name: String }` (typed getters) and `Sessions::create_with_bundle(&self, metadata: &serde_json::Value, bundle: Vec<u8>, bundle_filename: &str) -> Result<CreatedSessionResponse>`.

- [ ] **Step 1: Failing test** (response deser):
```rust
    #[test]
    fn created_session_response_parses() {
        let r: CreatedSessionResponse =
            serde_json::from_str(r#"{"session_id":"s1","agent_id":"ag","agent_name":"A"}"#).unwrap();
        assert_eq!(r.session_id().as_str(), "s1");
        assert_eq!(r.agent_name(), "A");
    }
```
- [ ] **Step 2: Run** → FAIL.
- [ ] **Step 3: Implement** in `sessions.rs`:
```rust
/// Response of multipart `POST /v1/sessions` (omnigent `CreatedSessionResponse`,
/// `schemas.py:1289-1291`). Lighter than a snapshot.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct CreatedSessionResponse {
    session_id: SessionId,
    agent_id: String,
    agent_name: String,
}
impl CreatedSessionResponse {
    pub fn session_id(&self) -> &SessionId { &self.session_id }
    pub fn agent_id(&self) -> &str { &self.agent_id }
    pub fn agent_name(&self) -> &str { &self.agent_name }
}
```
and to `impl<'a> Sessions<'a>`:
```rust
    /// `POST /v1/sessions` (multipart) — create from an agent `bundle` (.tar.gz)
    /// with a JSON `metadata` part (omnigent `SessionCreateMetadata`).
    pub fn create_with_bundle(
        &self,
        metadata: &serde_json::Value,
        bundle: Vec<u8>,
        bundle_filename: &str,
    ) -> Result<CreatedSessionResponse> {
        let form = reqwest::blocking::multipart::Form::new()
            .text("metadata", serde_json::to_string(metadata)?)
            .part(
                "bundle",
                reqwest::blocking::multipart::Part::bytes(bundle)
                    .file_name(bundle_filename.to_string())
                    .mime_str("application/gzip")
                    .map_err(crate::error::ClientError::Network)?,
            );
        self.client.send_multipart(reqwest::Method::POST, "/v1/sessions", form)
    }
```
- [ ] **Step 4: Re-export. Step 5: Verify. Step 6: Commit** `git commit -m "feat(lens-client): Sessions::create_with_bundle (multipart)"`.

> `SessionCreateMetadata` fields (`schemas.py:1222-1274`, `extra=forbid`): `title?`, `labels`, `reasoning_effort?`, `host_id?`, `workspace?`, `terminal_launch_args?`, `parent_session_id?`. The `metadata` arg is a `Value` the caller builds from these keys; consider a typed `SessionCreateMetadata` builder in a later pass if callers proliferate.

---

### Task 4: `patch` / `delete`

**Files:** Modify `sessions.rs`, `lib.rs`, a new `tests/live_sessions_lifecycle.rs`.

**Interfaces:** Produces `sessions::ConversationDeleted { id, object, deleted }` (getters) and `Sessions::patch(&self, id: &SessionId, req: &generated::UpdateSessionRequest) -> Result<SessionSnapshot>`, `Sessions::delete(&self, id: &SessionId, delete_branch: bool) -> Result<ConversationDeleted>`.

- [ ] **Step 1: Failing test**:
```rust
    #[test]
    fn conversation_deleted_parses() {
        let d: ConversationDeleted =
            serde_json::from_str(r#"{"id":"s1","object":"conversation.deleted","deleted":true}"#).unwrap();
        assert_eq!(d.id().as_str(), "s1");
        assert!(d.deleted());
    }
```
- [ ] **Step 2: Run** → FAIL.
- [ ] **Step 3: Implement**:
```rust
/// Response of `DELETE /v1/sessions/{id}` (omnigent `ConversationDeleted`).
#[derive(Clone, Debug, serde::Deserialize)]
pub struct ConversationDeleted {
    id: SessionId,
    #[serde(default)]
    object: String,
    #[serde(default)]
    deleted: bool,
}
impl ConversationDeleted {
    pub fn id(&self) -> &SessionId { &self.id }
    pub fn object(&self) -> &str { &self.object }
    pub fn deleted(&self) -> bool { self.deleted }
}
```
and to `impl<'a> Sessions<'a>` (uses the generated request type):
```rust
    /// `PATCH /v1/sessions/{id}` — update mutable session fields. Returns the
    /// updated snapshot. Build `req` from `lens_client::generated::UpdateSessionRequest`
    /// (fields: `runner_id`, `archived`, `silent`, `labels`, `model_override`,
    /// `reasoning_effort`, `collaboration_mode`, `terminal_launch_args`, …).
    pub fn patch(
        &self,
        id: &SessionId,
        req: &crate::generated::UpdateSessionRequest,
    ) -> Result<SessionSnapshot> {
        self.client.send_json(reqwest::Method::PATCH, &format!("/v1/sessions/{id}"), &[], Some(req))
    }

    /// `DELETE /v1/sessions/{id}` — delete; `delete_branch` cleans the worktree.
    pub fn delete(&self, id: &SessionId, delete_branch: bool) -> Result<ConversationDeleted> {
        self.client.send_json::<ConversationDeleted, ()>(
            reqwest::Method::DELETE,
            &format!("/v1/sessions/{id}"),
            &[("delete_branch", delete_branch.to_string())],
            None,
        )
    }
```
- [ ] **Step 4: Re-export** `ConversationDeleted`. **Step 5: Verify** (clippy will confirm `generated::UpdateSessionRequest` derives `Serialize` — typify emits Serialize on all schemas). **Step 6: Commit** `git commit -m "feat(lens-client): Sessions::patch + delete"`.

---

### Task 5: `fork` / `switch_agent` / `put_agent` (bundle)

**Files:** Modify `sessions.rs`, `lib.rs`.

**Interfaces:** Produces `Sessions::fork(&self, source: &SessionId, req: &generated::SessionForkRequest) -> Result<SessionSnapshot>`, `Sessions::switch_agent(&self, id: &SessionId, req: &generated::SessionSwitchAgentRequest) -> Result<SessionSnapshot>`, `Sessions::put_agent(&self, id: &SessionId, bundle: Vec<u8>, bundle_filename: &str) -> Result<generated::AgentObject>`.

- [ ] **Step 1: Implement** (these are thin wrappers over generated types — the testable surface is the live round-trip; no new serde types to unit-test). Add to `impl<'a> Sessions<'a>`:
```rust
    /// `POST /v1/sessions/{source_id}/fork` — clone the conversation onto a new
    /// session. Returns the new (idle) snapshot.
    pub fn fork(
        &self,
        source: &SessionId,
        req: &crate::generated::SessionForkRequest,
    ) -> Result<SessionSnapshot> {
        self.client.send_json(reqwest::Method::POST, &format!("/v1/sessions/{source}/fork"), &[], Some(req))
    }

    /// `POST /v1/sessions/{id}/switch-agent` — switch the bound agent (fires
    /// `session.agent_changed`). Returns the updated snapshot. `req.agent_id` required.
    pub fn switch_agent(
        &self,
        id: &SessionId,
        req: &crate::generated::SessionSwitchAgentRequest,
    ) -> Result<SessionSnapshot> {
        self.client.send_json(reqwest::Method::POST, &format!("/v1/sessions/{id}/switch-agent"), &[], Some(req))
    }

    /// `PUT /v1/sessions/{id}/agent` — store/replace the agent bundle (storage
    /// only; does NOT fire `session.agent_changed`). Returns the stored `AgentObject`.
    pub fn put_agent(
        &self,
        id: &SessionId,
        bundle: Vec<u8>,
        bundle_filename: &str,
    ) -> Result<crate::generated::AgentObject> {
        let form = reqwest::blocking::multipart::Form::new().part(
            "bundle",
            reqwest::blocking::multipart::Part::bytes(bundle)
                .file_name(bundle_filename.to_string())
                .mime_str("application/gzip")
                .map_err(crate::error::ClientError::Network)?,
        );
        self.client.send_multipart(reqwest::Method::PUT, &format!("/v1/sessions/{id}/agent"), form)
    }
```
> **⚠ Implementer:** confirm the multipart field name for `PUT /agent` against `Body_update_session_agent_v1_sessions__session_id__agent_put` in `generated.rs` / `openapi.json` — it may be `bundle` or another key (`file`/`agent`). Use the exact field name the schema declares; adjust `.part("<name>", …)` accordingly. This is a contract detail to verify, not guess.

- [ ] **Step 2: Verify** (compile + clippy; generated types confirm Serialize/Deserialize). **Step 3: Commit** `git commit -m "feat(lens-client): Sessions fork/switch_agent/put_agent"`.

---

### Task 6: Elicitations — `get` / `resolve`

**Files:** Modify `sessions.rs`, `lib.rs`, `tests/live_sessions_lifecycle.rs`.

**Interfaces:** Produces `Sessions::elicitation(&self, sid: &SessionId, eid: &ElicitationId) -> Result<ElicitationState>` (typed wrapper over the untyped pending state — expose the fields a consumer needs as typed getters; start minimal) and `Sessions::resolve_elicitation(&self, sid: &SessionId, eid: &ElicitationId, result: &generated::ElicitationResult) -> Result<SendEventAck>` (reuse Plan 2a's `SendEventAck` for the 202 ack).

- [ ] **Step 1: Implement** — the pending-elicitation GET returns an untyped object; model a minimal typed `ElicitationState` (grow getters as the elicitation UI needs them; never leak `Value`). Add to `sessions.rs`:
```rust
use crate::ids::ElicitationId;

/// Pending elicitation state (`GET …/elicitations/{id}`). The response is
/// untyped server-side; expose typed getters for the fields the elicitation UI
/// consumes. Start with the correlation id + raw-typed status; extend as needed.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct ElicitationState {
    #[serde(default)]
    elicitation_id: Option<String>,
    #[serde(default)]
    status: Option<String>,
}
impl ElicitationState {
    pub fn elicitation_id(&self) -> Option<&str> { self.elicitation_id.as_deref() }
    pub fn status(&self) -> Option<&str> { self.status.as_deref() }
}
```
and to `impl<'a> Sessions<'a>`:
```rust
    /// `GET /v1/sessions/{sid}/elicitations/{eid}` — deep-linkable pending state.
    pub fn elicitation(&self, sid: &SessionId, eid: &ElicitationId) -> Result<ElicitationState> {
        self.client.get_json(&format!("/v1/sessions/{sid}/elicitations/{eid}"), &[])
    }

    /// `POST …/elicitations/{eid}/resolve` — RESTful resolve (preferred over the
    /// `approval` event when an elicitation_id is on hand). Body is the generated
    /// `ElicitationResult {action, content?}`.
    pub fn resolve_elicitation(
        &self,
        sid: &SessionId,
        eid: &ElicitationId,
        result: &crate::generated::ElicitationResult,
    ) -> Result<crate::sessions::SendEventAck> {
        self.client.send_json(
            reqwest::Method::POST,
            &format!("/v1/sessions/{sid}/elicitations/{eid}/resolve"),
            &[],
            Some(result),
        )
    }
```
> **⚠ Implementer:** the `…/resolve` 202 ack shape may differ from `SendEventAck`; if it is a distinct shape, model a small `ElicitationResolveAck` instead. Verify against a live resolve response. The GET elicitation getters (`elicitation_id`/`status`) are a starting set — add typed getters for `action`/`requested_schema`/etc. when the elicitation UI consumes them, mining the field names from omnigent source.

- [ ] **Step 2: Re-export** `ElicitationState`. **Step 3: Verify. Step 4: Commit** `git commit -m "feat(lens-client): elicitation get + resolve"`.

---

## Self-review

- **Spec coverage:** create (JSON ✓ + multipart bundle ✓), patch ✓, delete ✓, fork ✓, switch-agent ✓, put_agent bundle ✓, elicitation get ✓ + resolve ✓. `patch`/`fork`/`switch_agent` return `SessionSnapshot` (2b); create-JSON returns snapshot, create-multipart returns `CreatedSessionResponse`; delete returns `ConversationDeleted`.
- **Generated vs hand-written:** generated request types reused (`UpdateSessionRequest`, `SessionForkRequest`, `SessionSwitchAgentRequest`, `ElicitationResult`, `AgentObject`); hand-modeled where omnigent doesn't expose a component (`CreateSessionRequest`, `ConversationDeleted`, `CreatedSessionResponse`, `ElicitationState`).
- **Open verifications flagged (not guessed):** the `PUT /agent` multipart field name, and the `…/resolve` ack shape — both marked ⚠ for the implementer to confirm against the contract/live, per ground-truth discipline.
- **No-`Value`-to-consumers:** honored on all responses (typed wrappers/getters). The two `Value` appearances are in *request builders* (`initial_items`, multipart `metadata`), fed by typed inputs — never returned to a consumer to parse.

## Next

Plan 2d (resources/terminals/comments) and Plan 2e (registries) reuse `get_json`/`send_json`. Live lifecycle tests (`tests/live_sessions_lifecycle.rs`) should create→patch→delete a throwaway session to exercise the round trip end-to-end against the pinned server.
