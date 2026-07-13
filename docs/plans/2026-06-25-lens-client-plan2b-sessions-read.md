# lens-client Plan 2b — Sessions read surface

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the session **read** endpoints to the `Sessions` subservice — `get` (snapshot), `list` (fleet poll), and `child_sessions` — establishing the crate's typed-read pattern over omnigent's untyped (`{}`) responses.

**Architecture:** These responses are **not typed in `openapi.json`** (they're open `{}`); their real shapes live in omnigent's Pydantic models (cited per type below). lens-client is the typed seam: each read returns a **strict Rust type with private fields + public typed getters** — consumers never receive `serde_json::Value` and never parse JSON. New fields are added as a private field + a getter in lens-client (the single edit site). A small private request helper (`Client::get_json`) keeps each method to one line. `items()` (polymorphic conversation items) is deliberately deferred to Plan 3, where the typed item union is modeled.

**Tech Stack:** Rust (edition 2024), `reqwest` (blocking), `serde`/`serde_json`. No async (D2). Builds on Plan 2a (`Sessions` subservice) and the foundation (`http::decode_json`, `ids`, `Connection`).

## Global Constraints

- All constraints from Plan 2a apply (edition 2024, blocking, no async/tokio, `cargo test` serverless, live tests gated behind `--features live-tests` + `LENS_OMNIGENT_URL`, reuse `http::decode_json`, `generated.rs` never hand-edited).
- **No `serde_json::Value` in any public read signature.** Public reads return strict types; field access is via typed getters on lens-client types.
- **Ground truth (omnigent `0.3.0.dev0`, `36b2a11c`), verbatim:**
  - REST status enum is **3-value**: `"idle" | "running" | "failed"` (`schemas.py:1604` `SessionResponse`, `:1869` `SessionListItem`). The SSE 5-value enum (`launching`/`waiting`) is Plan 3; REST collapses `waiting`→`running` server-side (`sessions.py:1792-1811`).
  - Liveness is **top-level booleans** `runner_online: bool|None`, `host_online: bool|None` (NOT a nested object), plus always-present `host_resumable: bool` (`sessions.py:19074-19098`, `schemas.py:1642`). `include_liveness` defaults `true`.
  - `SessionResponse` (snapshot) core: `id`, `status`, `agent_id`, `agent_name?`, `archived` (default false), `created_at` (epoch **seconds**, int), `labels` (`dict[str,str]`), `runner_online?`, `host_online?`, `host_resumable`. **No `kind`, no `silent`, no `updated_at`** on the snapshot.
  - `SessionListItem` = snapshot minus liveness/items/`host_resumable`, **plus `updated_at`** (epoch seconds). Wrapped in `PaginatedList { data, first_id, last_id, has_more }`, serialized `exclude_none` (unset optionals omitted, not null).
  - `ChildSessionSummary` (`schemas.py:558-664`): `id`(req), `object`(="child_session"), `parent_session_id`(req), `title?`, `tool?`, `session_name?`, `kind`(="sub_agent"), `created_at`(req int), `updated_at`(req int), `agent_id?`, `agent_name?`, `current_task_id?`, `current_task_status?`, `busy`(default false), `labels`(dict), `last_task_error?`(`dict[str,str]`), `last_message_preview?`, `pending_elicitations_count`(default 0).
  - List query params: `limit`, `after?`, `before?`, `agent_id?`, `agent_name?`, `order?`, `sort_by?`, `search_query?`, `include_archived`, `kind` (`default|sub_agent|any`). Snapshot params: `include_items?`, `include_liveness?`(default true), `refresh_state?`. child_sessions params: `limit`, `after?`, `before?`, `order?`.

---

### Task 1: REST status enum + private request helper

**Files:**
- Modify: `crates/lens-client/src/sessions.rs` (add `SessionStatus`)
- Modify: `crates/lens-client/src/client.rs` (add `pub(crate) get_json`)
- Modify: `crates/lens-client/src/lib.rs` (re-export `SessionStatus`)

**Interfaces:**
- Produces: `sessions::SessionStatus` (`Idle`/`Running`/`Failed`, serde-renamed lowercase, `Deserialize`); `client::Client::get_json<T: DeserializeOwned>(&self, path: &str, query: &[(&str, String)]) -> Result<T>`.

- [ ] **Step 1: Failing tests** — add to `sessions.rs` test module:
```rust
    #[test]
    fn session_status_deserializes_rest_values() {
        use serde_json::json;
        assert_eq!(serde_json::from_value::<SessionStatus>(json!("idle")).unwrap(), SessionStatus::Idle);
        assert_eq!(serde_json::from_value::<SessionStatus>(json!("running")).unwrap(), SessionStatus::Running);
        assert_eq!(serde_json::from_value::<SessionStatus>(json!("failed")).unwrap(), SessionStatus::Failed);
        // "waiting" is collapsed to "running" server-side and never reaches REST; reject it.
        assert!(serde_json::from_value::<SessionStatus>(json!("waiting")).is_err());
    }
```

- [ ] **Step 2: Run** `cargo test -p lens-client sessions::tests::session_status` → FAIL (`SessionStatus` missing).

- [ ] **Step 3: Implement** — add to `sessions.rs` (near the top types):
```rust
/// Session status as reported by the REST surface (snapshot + list). Only three
/// values reach REST; the server collapses `waiting`→`running` and never emits
/// `launching` on parents (`sessions.py:1792-1811`). The richer 5-value SSE
/// status (`SessionStatusEvent`) is modeled separately in the streaming plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatus {
    Idle,
    Running,
    Failed,
}
```

- [ ] **Step 4: Add the request helper** — in `client.rs`, add to `impl Client` (uses the existing `pub(crate)` `conn`/`http` + `http::decode_json`):
```rust
    /// Issue a GET expecting a JSON body, mapping status → typed errors. Internal
    /// building block for the typed REST methods. `query` pairs are appended as-is.
    pub(crate) fn get_json<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> crate::error::Result<T> {
        let url = self.conn().url(path)?;
        let resp = self
            .conn()
            .auth
            .apply(self.http().get(url).query(query))
            .send()?;
        let status = resp.status().as_u16();
        let body = resp.text()?;
        crate::http::decode_json(path, status, &body)
    }
```

- [ ] **Step 5: Re-export** — in `lib.rs`, extend the sessions re-export with `SessionStatus`.

- [ ] **Step 6: Verify** `cargo test -p lens-client && cargo clippy -p lens-client --all-targets -- -D warnings && cargo fmt -p lens-client --check` → PASS. (Note: `get_json` is `pub(crate)` and unused until Task 3; if clippy flags it, that is expected — Task 3 lands in the same plan. If you split commits, add `#[allow(dead_code)]` here with a `// used by Sessions read methods (Task 3)` note and remove it in Task 3. Prefer committing Tasks 1+3 close together.)

- [ ] **Step 7: Commit** `git add crates/lens-client/src/sessions.rs crates/lens-client/src/client.rs crates/lens-client/src/lib.rs && git commit -m "feat(lens-client): REST SessionStatus + Client::get_json helper"`

---

### Task 2: `SessionSnapshot` read type

**Files:** Modify `crates/lens-client/src/sessions.rs`, `lib.rs` (re-export).

**Interfaces:**
- Produces: `sessions::SessionSnapshot` — private serde fields, public typed getters: `id() -> &SessionId`, `status() -> SessionStatus`, `agent_id() -> &str`, `agent_name() -> Option<&str>`, `archived() -> bool`, `created_at() -> i64`, `labels() -> &BTreeMap<String, String>`, `runner_online() -> Option<bool>`, `host_online() -> Option<bool>`, `host_resumable() -> bool`.

- [ ] **Step 1: Failing test** — add to test module:
```rust
    #[test]
    fn session_snapshot_parses_core_fields_and_liveness() {
        let body = r#"{
            "id": "sess_1", "status": "running", "agent_id": "ag_1",
            "agent_name": "Builder", "archived": false, "created_at": 1719331200,
            "labels": {"env": "prod"}, "runner_online": true, "host_online": null,
            "host_resumable": false, "extra_unmodeled_field": 99
        }"#;
        let s: SessionSnapshot = serde_json::from_str(body).unwrap();
        assert_eq!(s.id().as_str(), "sess_1");
        assert_eq!(s.status(), SessionStatus::Running);
        assert_eq!(s.agent_id(), "ag_1");
        assert_eq!(s.agent_name(), Some("Builder"));
        assert!(!s.archived());
        assert_eq!(s.created_at(), 1719331200);
        assert_eq!(s.labels().get("env").map(String::as_str), Some("prod"));
        assert_eq!(s.runner_online(), Some(true));
        assert_eq!(s.host_online(), None);
        assert!(!s.host_resumable());
    }
```

- [ ] **Step 2: Run** → FAIL (`SessionSnapshot` missing).

- [ ] **Step 3: Implement** — add to `sessions.rs` (ensure `use crate::ids::SessionId; use std::collections::BTreeMap;` at the top):
```rust
/// A session snapshot (`GET /v1/sessions/{id}`). Mirrors the CORE fields of
/// omnigent's `SessionResponse` (`schemas.py:1601-1642`); unmodeled fields are
/// ignored. Fields are private — access is via the typed getters, so the wire
/// shape stays an lens-client implementation detail (single edit site for drift).
#[derive(Clone, Debug, serde::Deserialize)]
pub struct SessionSnapshot {
    id: SessionId,
    status: SessionStatus,
    agent_id: String,
    #[serde(default)]
    agent_name: Option<String>,
    #[serde(default)]
    archived: bool,
    created_at: i64,
    #[serde(default)]
    labels: BTreeMap<String, String>,
    #[serde(default)]
    runner_online: Option<bool>,
    #[serde(default)]
    host_online: Option<bool>,
    #[serde(default)]
    host_resumable: bool,
}

impl SessionSnapshot {
    pub fn id(&self) -> &SessionId { &self.id }
    pub fn status(&self) -> SessionStatus { self.status }
    pub fn agent_id(&self) -> &str { &self.agent_id }
    pub fn agent_name(&self) -> Option<&str> { self.agent_name.as_deref() }
    pub fn archived(&self) -> bool { self.archived }
    /// Creation time, epoch **seconds**.
    pub fn created_at(&self) -> i64 { self.created_at }
    pub fn labels(&self) -> &BTreeMap<String, String> { &self.labels }
    /// `Some` only when the snapshot was fetched with `include_liveness` (default true).
    pub fn runner_online(&self) -> Option<bool> { self.runner_online }
    pub fn host_online(&self) -> Option<bool> { self.host_online }
    pub fn host_resumable(&self) -> bool { self.host_resumable }
}
```

- [ ] **Step 4: Re-export** `SessionSnapshot` in `lib.rs`.
- [ ] **Step 5: Verify** tests + clippy + fmt PASS.
- [ ] **Step 6: Commit** `git commit -m "feat(lens-client): SessionSnapshot typed read type"` (stage sessions.rs + lib.rs).

---

### Task 3: `Sessions::get` + `GetOpts`

**Files:** Modify `crates/lens-client/src/sessions.rs`; create `crates/lens-client/tests/live_sessions_read.rs`.

**Interfaces:**
- Produces: `sessions::GetOpts { include_items: bool, include_liveness: bool, refresh_state: bool }` (with `Default` = all false except `include_liveness: true`); `Sessions::get(&self, id: &SessionId, opts: GetOpts) -> Result<SessionSnapshot>`.

- [ ] **Step 1: Failing test (query building is pure)** — add a helper + test:
```rust
    #[test]
    fn get_opts_builds_expected_query() {
        let q = GetOpts::default().to_query();
        assert!(q.contains(&("include_liveness", "true".to_string())));
        assert!(q.contains(&("include_items", "false".to_string())));
        assert!(q.contains(&("refresh_state", "false".to_string())));
    }
```

- [ ] **Step 2: Run** → FAIL (`GetOpts` missing).

- [ ] **Step 3: Implement** — add to `sessions.rs`:
```rust
/// Options for `Sessions::get`. Defaults: liveness on, items off, no refresh.
#[derive(Clone, Copy, Debug)]
pub struct GetOpts {
    pub include_items: bool,
    pub include_liveness: bool,
    pub refresh_state: bool,
}

impl Default for GetOpts {
    fn default() -> Self {
        Self { include_items: false, include_liveness: true, refresh_state: false }
    }
}

impl GetOpts {
    fn to_query(self) -> Vec<(&'static str, String)> {
        vec![
            ("include_items", self.include_items.to_string()),
            ("include_liveness", self.include_liveness.to_string()),
            ("refresh_state", self.refresh_state.to_string()),
        ]
    }
}
```
and add to `impl<'a> Sessions<'a>`:
```rust
    /// `GET /v1/sessions/{id}` — the session snapshot. Blocking.
    pub fn get(&self, id: &SessionId, opts: GetOpts) -> Result<SessionSnapshot> {
        self.client.get_json(&format!("/v1/sessions/{id}"), &opts.to_query())
    }
```

- [ ] **Step 4: Gated live test** — `crates/lens-client/tests/live_sessions_read.rs`:
```rust
//! Live read tests. Require $LENS_OMNIGENT_URL and $LENS_OMNIGENT_SESSION_ID.
#![cfg(feature = "live-tests")]

use lens_client::ids::{ConnectionId, SessionId};
use lens_client::sessions::GetOpts;
use lens_client::{Auth, Connection};

fn client() -> lens_client::Client {
    let base = std::env::var("LENS_OMNIGENT_URL").expect("LENS_OMNIGENT_URL").parse().unwrap();
    lens_client::Client::new(Connection::new(ConnectionId::new("live"), base, Auth::None)).expect("handshake")
}

#[test]
fn get_snapshot_parses() {
    let sid = SessionId::new(std::env::var("LENS_OMNIGENT_SESSION_ID").expect("LENS_OMNIGENT_SESSION_ID"));
    let snap = client().sessions().get(&sid, GetOpts::default()).expect("snapshot");
    assert_eq!(snap.id().as_str(), sid.as_str());
    let _ = snap.status();
}
```

- [ ] **Step 5: Verify** `cargo test -p lens-client` (serverless PASS; live compiled out) + clippy `--all-targets` + fmt. Then optionally the live run with both env vars set.
- [ ] **Step 6: Commit** `git commit -m "feat(lens-client): Sessions::get snapshot (gated live test)"` (stage sessions.rs + the new test file).

---

### Task 4: `Sessions::list` + `SessionSummary` + `SessionFilter`

**Files:** Modify `crates/lens-client/src/sessions.rs`, `lib.rs`, `tests/live_sessions_read.rs`.

**Interfaces:**
- Produces: `sessions::SessionSummary` (typed getters: `id`, `status`, `agent_id`, `agent_name`, `archived`, `created_at`, `updated_at`, `labels`); `sessions::SessionList { data: Vec<SessionSummary>, first_id: Option<String>, last_id: Option<String>, has_more: bool }`; `sessions::SessionKind` (`Default`/`SubAgent`/`Any`); `sessions::SessionFilter` (builder of query pairs); `Sessions::list(&self, filter: &SessionFilter) -> Result<SessionList>`.

- [ ] **Step 1: Failing tests**
```rust
    #[test]
    fn session_list_parses_paginated_envelope() {
        let body = r#"{"object":"list","data":[
            {"id":"s1","status":"idle","agent_id":"ag","agent_name":null,"archived":false,
             "created_at":1,"updated_at":2,"labels":{}}],
            "first_id":"s1","last_id":"s1","has_more":false}"#;
        let list: SessionList = serde_json::from_str(body).unwrap();
        assert_eq!(list.data.len(), 1);
        assert_eq!(list.data[0].id().as_str(), "s1");
        assert_eq!(list.data[0].updated_at(), 2);
        assert!(!list.has_more);
    }

    #[test]
    fn session_filter_builds_query() {
        let f = SessionFilter::new().kind(SessionKind::SubAgent).include_archived(true).search("foo").limit(50);
        let q = f.to_query();
        assert!(q.contains(&("kind", "sub_agent".to_string())));
        assert!(q.contains(&("include_archived", "true".to_string())));
        assert!(q.contains(&("search_query", "foo".to_string())));
        assert!(q.contains(&("limit", "50".to_string())));
    }
```

- [ ] **Step 2: Run** → FAIL.

- [ ] **Step 3: Implement** — add to `sessions.rs`:
```rust
/// One element of `GET /v1/sessions` (omnigent `SessionListItem`, `schemas.py:1866-1885`).
/// Like a snapshot minus liveness, plus `updated_at`.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct SessionSummary {
    id: SessionId,
    status: SessionStatus,
    agent_id: String,
    #[serde(default)]
    agent_name: Option<String>,
    #[serde(default)]
    archived: bool,
    created_at: i64,
    updated_at: i64,
    #[serde(default)]
    labels: BTreeMap<String, String>,
}

impl SessionSummary {
    pub fn id(&self) -> &SessionId { &self.id }
    pub fn status(&self) -> SessionStatus { self.status }
    pub fn agent_id(&self) -> &str { &self.agent_id }
    pub fn agent_name(&self) -> Option<&str> { self.agent_name.as_deref() }
    pub fn archived(&self) -> bool { self.archived }
    pub fn created_at(&self) -> i64 { self.created_at }
    pub fn updated_at(&self) -> i64 { self.updated_at }
    pub fn labels(&self) -> &BTreeMap<String, String> { &self.labels }
}

/// `GET /v1/sessions` — a `PaginatedList` of summaries.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct SessionList {
    pub data: Vec<SessionSummary>,
    #[serde(default)]
    pub first_id: Option<String>,
    #[serde(default)]
    pub last_id: Option<String>,
    #[serde(default)]
    pub has_more: bool,
}

/// `kind` filter for the fleet poll.
#[derive(Clone, Copy, Debug)]
pub enum SessionKind { Default, SubAgent, Any }

impl SessionKind {
    fn as_str(self) -> &'static str {
        match self { SessionKind::Default => "default", SessionKind::SubAgent => "sub_agent", SessionKind::Any => "any" }
    }
}

/// Query filter for `Sessions::list`. All fields optional; unset → omitted.
#[derive(Clone, Debug, Default)]
pub struct SessionFilter {
    limit: Option<u32>,
    after: Option<String>,
    before: Option<String>,
    agent_id: Option<String>,
    agent_name: Option<String>,
    search_query: Option<String>,
    include_archived: Option<bool>,
    kind: Option<SessionKind>,
}

impl SessionFilter {
    pub fn new() -> Self { Self::default() }
    pub fn limit(mut self, n: u32) -> Self { self.limit = Some(n); self }
    pub fn after(mut self, c: impl Into<String>) -> Self { self.after = Some(c.into()); self }
    pub fn before(mut self, c: impl Into<String>) -> Self { self.before = Some(c.into()); self }
    pub fn agent_id(mut self, v: impl Into<String>) -> Self { self.agent_id = Some(v.into()); self }
    pub fn agent_name(mut self, v: impl Into<String>) -> Self { self.agent_name = Some(v.into()); self }
    pub fn search(mut self, v: impl Into<String>) -> Self { self.search_query = Some(v.into()); self }
    pub fn include_archived(mut self, v: bool) -> Self { self.include_archived = Some(v); self }
    pub fn kind(mut self, k: SessionKind) -> Self { self.kind = Some(k); self }

    fn to_query(&self) -> Vec<(&'static str, String)> {
        let mut q = Vec::new();
        if let Some(n) = self.limit { q.push(("limit", n.to_string())); }
        if let Some(v) = &self.after { q.push(("after", v.clone())); }
        if let Some(v) = &self.before { q.push(("before", v.clone())); }
        if let Some(v) = &self.agent_id { q.push(("agent_id", v.clone())); }
        if let Some(v) = &self.agent_name { q.push(("agent_name", v.clone())); }
        if let Some(v) = &self.search_query { q.push(("search_query", v.clone())); }
        if let Some(v) = self.include_archived { q.push(("include_archived", v.to_string())); }
        if let Some(k) = self.kind { q.push(("kind", k.as_str().to_string())); }
        q
    }
}
```
and to `impl<'a> Sessions<'a>`:
```rust
    /// `GET /v1/sessions` — fleet poll. Blocking.
    pub fn list(&self, filter: &SessionFilter) -> Result<SessionList> {
        self.client.get_json("/v1/sessions", &filter.to_query())
    }
```

- [ ] **Step 4: Re-export** `SessionSummary`, `SessionList`, `SessionKind`, `SessionFilter` in `lib.rs`.
- [ ] **Step 5: Gated live test** — append to `live_sessions_read.rs`:
```rust
#[test]
fn list_sessions_parses() {
    use lens_client::sessions::SessionFilter;
    let list = client().sessions().list(&SessionFilter::new().limit(5)).expect("list");
    // Envelope parsed; data may be empty on a fresh server.
    let _ = list.has_more;
}
```
- [ ] **Step 6: Verify** + **Step 7: Commit** `git commit -m "feat(lens-client): Sessions::list fleet poll + SessionFilter"`.

---

### Task 5: `ChildSessionSummary` mirror + `Sessions::child_sessions`

**Files:** Modify `crates/lens-client/src/sessions.rs`, `lib.rs`, `tests/live_sessions_read.rs`.

**Interfaces:**
- Produces: `sessions::ChildSessionSummary` (full mirror; getters for all fields; the event-partial fields are `Option`); `Sessions::child_sessions(&self, id: &SessionId, page: &SessionFilter) -> Result<SessionList>`... **no** — child sessions return their own list. Produces `sessions::ChildSessionList { data: Vec<ChildSessionSummary>, first_id, last_id, has_more }` and `Sessions::child_sessions(&self, id: &SessionId, limit: Option<u32>, after: Option<&str>) -> Result<ChildSessionList>`.

- [ ] **Step 1: Failing test**
```rust
    #[test]
    fn child_session_summary_parses_full_and_partial() {
        // Full (GET) shape.
        let full = r#"{"id":"c1","object":"child_session","parent_session_id":"p1",
            "title":"T","tool":"task","session_name":"sn","kind":"sub_agent",
            "created_at":1,"updated_at":2,"busy":true,"labels":{},"current_task_status":"running",
            "pending_elicitations_count":3}"#;
        let c: ChildSessionSummary = serde_json::from_str(full).unwrap();
        assert_eq!(c.id().as_str(), "c1");
        assert_eq!(c.parent_session_id(), "p1");
        assert!(c.busy());
        assert_eq!(c.pending_elicitations_count(), 3);
        assert_eq!(c.current_task_status(), Some("running"));

        // Partial (event delta) shape — most fields absent; required-on-full
        // fields that events omit must default, not error.
        let partial = r#"{"id":"c1","busy":false,"current_task_status":"launching"}"#;
        let p: ChildSessionSummary = serde_json::from_str(partial).unwrap();
        assert_eq!(p.id().as_str(), "c1");
        assert_eq!(p.parent_session_id(), "");
        assert_eq!(p.created_at(), 0);
    }
```

- [ ] **Step 2: Run** → FAIL.

- [ ] **Step 3: Implement** — add to `sessions.rs`. Note: the GET carries the full model, but the SSE `session.child_session.updated` event carries a sparse partial; to let the SAME struct parse both (events merge into the cached row), `parent_session_id`/`created_at`/`updated_at` use `#[serde(default)]` even though they are required on the full GET. The contract test pins the full shape; the partial is tolerated:
```rust
/// Mirror of omnigent `ChildSessionSummary` (`schemas.py:558-664`). Not in
/// `openapi.json` `components` — hand-written from source and contract-tested.
/// The live `session.child_session.updated` event carries a PARTIAL of this
/// shape, so fields the event may omit default rather than error (the state
/// model merges present fields over the cached child row).
#[derive(Clone, Debug, serde::Deserialize)]
pub struct ChildSessionSummary {
    id: SessionId,
    #[serde(default)]
    parent_session_id: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    tool: Option<String>,
    #[serde(default)]
    session_name: Option<String>,
    #[serde(default)]
    agent_id: Option<String>,
    #[serde(default)]
    agent_name: Option<String>,
    #[serde(default)]
    current_task_id: Option<String>,
    #[serde(default)]
    current_task_status: Option<String>,
    #[serde(default)]
    busy: bool,
    #[serde(default)]
    created_at: i64,
    #[serde(default)]
    updated_at: i64,
    #[serde(default)]
    labels: BTreeMap<String, String>,
    #[serde(default)]
    last_task_error: Option<BTreeMap<String, String>>,
    #[serde(default)]
    last_message_preview: Option<String>,
    #[serde(default)]
    pending_elicitations_count: i64,
}

impl ChildSessionSummary {
    pub fn id(&self) -> &SessionId { &self.id }
    pub fn parent_session_id(&self) -> &str { &self.parent_session_id }
    pub fn title(&self) -> Option<&str> { self.title.as_deref() }
    pub fn tool(&self) -> Option<&str> { self.tool.as_deref() }
    pub fn session_name(&self) -> Option<&str> { self.session_name.as_deref() }
    pub fn agent_id(&self) -> Option<&str> { self.agent_id.as_deref() }
    pub fn agent_name(&self) -> Option<&str> { self.agent_name.as_deref() }
    pub fn current_task_id(&self) -> Option<&str> { self.current_task_id.as_deref() }
    pub fn current_task_status(&self) -> Option<&str> { self.current_task_status.as_deref() }
    pub fn busy(&self) -> bool { self.busy }
    pub fn created_at(&self) -> i64 { self.created_at }
    pub fn updated_at(&self) -> i64 { self.updated_at }
    pub fn labels(&self) -> &BTreeMap<String, String> { &self.labels }
    pub fn last_task_error(&self) -> Option<&BTreeMap<String, String>> { self.last_task_error.as_ref() }
    pub fn last_message_preview(&self) -> Option<&str> { self.last_message_preview.as_deref() }
    pub fn pending_elicitations_count(&self) -> i64 { self.pending_elicitations_count }
}

/// `GET /v1/sessions/{id}/child_sessions` — paginated child summaries.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct ChildSessionList {
    pub data: Vec<ChildSessionSummary>,
    #[serde(default)]
    pub first_id: Option<String>,
    #[serde(default)]
    pub last_id: Option<String>,
    #[serde(default)]
    pub has_more: bool,
}
```
and to `impl<'a> Sessions<'a>`:
```rust
    /// `GET /v1/sessions/{id}/child_sessions` — list sub-sessions. Blocking.
    pub fn child_sessions(
        &self,
        id: &SessionId,
        limit: Option<u32>,
        after: Option<&str>,
    ) -> Result<ChildSessionList> {
        let mut q = Vec::new();
        if let Some(n) = limit { q.push(("limit", n.to_string())); }
        if let Some(a) = after { q.push(("after", a.to_string())); }
        self.client.get_json(&format!("/v1/sessions/{id}/child_sessions"), &q)
    }
```

- [ ] **Step 4: Re-export** `ChildSessionSummary`, `ChildSessionList` in `lib.rs`.
- [ ] **Step 5: Gated live test** — append:
```rust
#[test]
fn child_sessions_parses() {
    let sid = lens_client::ids::SessionId::new(std::env::var("LENS_OMNIGENT_SESSION_ID").expect("session id"));
    let list = client().sessions().child_sessions(&sid, Some(10), None).expect("child_sessions");
    let _ = list.has_more;
}
```
- [ ] **Step 6: Verify** + **Step 7: Commit** `git commit -m "feat(lens-client): ChildSessionSummary mirror + child_sessions"`.

---

## Self-review

- **Spec coverage:** `get` (snapshot, core fields + liveness booleans) ✓; `list` (PaginatedList of summaries + filter) ✓; `child_sessions` (full mirror, partial-tolerant) ✓; REST 3-value `SessionStatus` ✓; the typed-read pattern (private fields + getters, no `Value` leaked) established ✓; `get_json` helper reused ✓.
- **Deliberately deferred:** `items()` — conversation items are the polymorphic union modeled by Plan 3's SSE taxonomy; returning them now would either leak `Value` (violates the no-JSON-to-consumers rule) or duplicate Plan 3. `items()` lands in Plan 3 atop the typed item enum. `kind`/`silent`/`updated_at` absences are recorded (not response fields / list-only).
- **Type consistency:** every read type uses `SessionId` (foundation), `SessionStatus` (Task 1), `BTreeMap<String,String>` for labels; `get_json` signature matches its uses; `SessionFilter`/`GetOpts` `to_query` return `Vec<(&'static str, String)>` matching `get_json`'s `&[(&str, String)]`.
- **Ground truth:** status enum, liveness booleans, snapshot/list field sets, and the `ChildSessionSummary` full+partial shapes are cited to omnigent `0.3.0.dev0` source.

## Next

Plan 2c (sessions lifecycle: create/patch/delete/fork/switch-agent/elicitation resolve) builds on this `Sessions` subservice and reuses `SessionSnapshot` (patch/fork/switch-agent all return a snapshot).
