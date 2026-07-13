# lens-client Plan 2e — Registries (agents / hosts / runners / policies / me)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans. Steps use checkbox (`- [ ]`).

**Goal:** The server-level registry surface — agents (read-only), hosts (read + directories + filesystem), runners (list/status/launch), policies (server-wide + session-scoped + registry + evaluate), and `/v1/me` — closing out the §3 REST surface.

**Architecture:** Extends Plans 2a–2d; registry methods hang off `Client` directly (a new `registries.rs` with `impl Client` blocks), session-scoped policies off `Sessions`. Reuses `get_json`/`send_json`, the typed-wrapper read pattern (no `Value` to consumers), generated request types (`LaunchRunnerRequest`, `CreateDirectoryRequest`, `CreateDefaultPolicyRequest`, `CreateSessionPolicyRequest`, `UpdateSessionPolicyRequest`, `AgentObject`). `RunnerStatus` and `Me` have pinned shapes; hosts/policies are minimal wrappers grown lazily.

**Tech Stack:** Rust (edition 2024), `reqwest` blocking, `serde`. No async (D2).

## Global Constraints

- Plans 2a–2d constraints apply. No `Value` in public read signatures; reuse helpers; generated.rs never hand-edited; live tests gated.
- **Ground truth (omnigent `0.3.0.dev0`, `36b2a11c`):**
  - `GET /v1/agents` → `PaginatedList` of agent objects (read-only; NO REST CRUD). `AgentObject` is **generated**.
  - `GET /v1/hosts`, `GET /v1/hosts/{host_id}` → untyped objects (read-only; NO POST/DELETE hosts). `POST /v1/hosts/{host_id}/directories` (`CreateDirectoryRequest`, generated). `GET /v1/hosts/{host_id}/filesystem[/{path}]` → browse.
  - `POST /v1/hosts/{host_id}/runners` (`LaunchRunnerRequest {session_id, workspace, git?}`, generated). `GET /v1/runners` → list. `GET /v1/runners/{runner_id}/status` → `{runner_id: str, online: bool, error?: str}` (`runner_tunnel.py:234-265`).
  - Policies: `GET/POST /v1/policies` (`CreateDefaultPolicyRequest`), `GET/PATCH/DELETE /v1/policies/{policy_id}`, `GET /v1/policy-registry`. Session-scoped: `GET/POST /v1/sessions/{id}/policies` (`CreateSessionPolicyRequest`), `GET/DELETE /v1/sessions/{id}/policies/{policy_id}`, `POST /v1/sessions/{id}/policies/evaluate`.
  - `GET /v1/me` → `200 {"user_id": str|null}`; `401 {"user_id": null, "login_url": str}` (`app.py:1566-1592`). (`/v1/info`, `/api/version`, `/health` already in the foundation.)
  - Permission/grant levels and `/v1/info` semantics are already modeled (foundation + Plan 2d).

---

### Task 1: `registries.rs` scaffold + `RunnerStatus` + runners

**Files:** Create `crates/lens-client/src/registries.rs`; modify `lib.rs` (add `pub mod registries;` + re-exports).

**Interfaces:** Produces `registries::RunnerStatus { runner_id: RunnerId, online: bool, error: Option<String> }` (typed getters) and `Client::list_runners(&self) -> Result<RunnerList>`, `Client::runner_status(&self, runner_id: &RunnerId) -> Result<RunnerStatus>`.

- [ ] **Step 1: Failing test** — in `registries.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runner_status_parses_online_and_offline() {
        let on: RunnerStatus = serde_json::from_str(r#"{"runner_id":"r1","online":true}"#).unwrap();
        assert_eq!(on.runner_id().as_str(), "r1");
        assert!(on.online());
        assert_eq!(on.error(), None);
        let off: RunnerStatus =
            serde_json::from_str(r#"{"runner_id":"r1","online":false,"error":"exited 1"}"#).unwrap();
        assert!(!off.online());
        assert_eq!(off.error(), Some("exited 1"));
    }
}
```
- [ ] **Step 2: Run** `cargo test -p lens-client registries::` → FAIL.
- [ ] **Step 3: Implement** — prepend to `registries.rs`:
```rust
//! Server-level registries (agents, hosts, runners, policies) + `/v1/me`.
//! Methods hang off `Client`. Read responses are typed wrappers (no `Value`
//! reaches consumers); request bodies use generated types where available.

use crate::client::Client;
use crate::error::Result;
use crate::ids::{HostId, RunnerId};

/// `GET /v1/runners/{id}/status` (`runner_tunnel.py:234-265`).
#[derive(Clone, Debug, serde::Deserialize)]
pub struct RunnerStatus {
    runner_id: RunnerId,
    #[serde(default)]
    online: bool,
    #[serde(default)]
    error: Option<String>,
}
impl RunnerStatus {
    pub fn runner_id(&self) -> &RunnerId { &self.runner_id }
    pub fn online(&self) -> bool { self.online }
    pub fn error(&self) -> Option<&str> { self.error.as_deref() }
}

/// `GET /v1/runners` — runner list. ⚠ Confirm the envelope (`{data:[...]}` vs
/// bare array); model the element fields as the runner UI needs them.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct RunnerList {
    #[serde(default)]
    pub data: Vec<serde_json::Value>, // ⚠ promote to a typed RunnerSummary when consumed
    #[serde(default)]
    pub has_more: bool,
}

impl Client {
    pub fn list_runners(&self) -> Result<RunnerList> {
        self.get_json("/v1/runners", &[])
    }
    pub fn runner_status(&self, runner_id: &RunnerId) -> Result<RunnerStatus> {
        self.get_json(&format!("/v1/runners/{runner_id}/status"), &[])
    }
}
```
> ⚠ `RunnerList.data` is the one place this plan tolerates `Vec<Value>` *internally* — but it is `pub`, which would leak `Value` to consumers. Before merge, either (a) make `data` private + add a typed `RunnerSummary` with getters, or (b) keep the list endpoint deferred until a consumer needs it and ship only `runner_status` (fully typed). Pick (a) if the fleet UI needs the list now; otherwise (b). Do not ship a `pub Vec<Value>`.

- [ ] **Step 4: Re-export `RunnerStatus`, `RunnerList` (or omit per the ⚠). Step 5: Verify** tests + clippy + fmt. **Step 6: Commit** `git commit -m "feat(lens-client): registries scaffold + runner status/list"`.

---

### Task 2: `Me` (`/v1/me`)

**Interfaces:** Produces `registries::Me { user_id: Option<String> }` (getter) and `Client::me(&self) -> Result<Me>`.

- [ ] **Step 1: Failing test**:
```rust
    #[test]
    fn me_parses_user_id() {
        let m: Me = serde_json::from_str(r#"{"user_id":"u_42"}"#).unwrap();
        assert_eq!(m.user_id(), Some("u_42"));
        let anon: Me = serde_json::from_str(r#"{"user_id":null}"#).unwrap();
        assert_eq!(anon.user_id(), None);
    }
```
- [ ] **Step 2: Run** → FAIL. **Step 3: Implement** in `registries.rs`:
```rust
/// `GET /v1/me` — auth identity (`app.py:1566-1592`). 200 carries `user_id`
/// (null when accounts are off); a 401 (OIDC unauthenticated) is surfaced as
/// `ClientError::Auth` by `decode_json`, so this type models the 200 body only.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct Me {
    #[serde(default)]
    user_id: Option<String>,
}
impl Me {
    pub fn user_id(&self) -> Option<&str> { self.user_id.as_deref() }
}

impl Client {
    pub fn me(&self) -> Result<Me> {
        self.get_json("/v1/me", &[])
    }
}
```
- [ ] **Step 4: Re-export `Me`. Step 5: Verify. Step 6: Commit** `git commit -m "feat(lens-client): /v1/me identity"`.
> Note: a 401 from `/v1/me` becomes `ClientError::Auth { status: 401 }` (foundation `decode_json`), losing the `login_url` in the 401 body. If the login chrome needs that URL, add a dedicated `me_or_login()` that reads `status`+`text` and returns an enum `MeResult::Authed(Me) | NeedsLogin { login_url }`. Flagged for the auth UI; out of scope for the basic getter.

---

### Task 3: Agents (read-only)

**Interfaces:** Produces `registries::AgentList { data: Vec<generated::AgentObject>, has_more, first_id, last_id }` and `Client::list_agents(&self) -> Result<AgentList>`.

- [ ] **Step 1: Implement** — agents list is a `PaginatedList` whose elements are the generated `AgentObject`; hand-model the envelope to get typed elements (the generated `PaginatedList` may type `data` as untyped):
```rust
/// `GET /v1/agents` — agent registry (read-only; authoring is filesystem YAML +
/// bundle upload). Elements are the generated `AgentObject`.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct AgentList {
    #[serde(default)]
    pub data: Vec<crate::generated::AgentObject>,
    #[serde(default)]
    pub first_id: Option<String>,
    #[serde(default)]
    pub last_id: Option<String>,
    #[serde(default)]
    pub has_more: bool,
}

impl Client {
    pub fn list_agents(&self) -> Result<AgentList> {
        self.get_json("/v1/agents", &[])
    }
}
```
- [ ] **Step 2: Failing/passing test** — a golden parse using the real `AgentObject` field set (read its required fields from `generated.rs` and build a minimal valid JSON; e.g. if `AgentObject` requires `id`/`name`, `{"data":[{"id":"ag","name":"A", …}],"has_more":false}`). Assert `list.data[0].id == "ag"`.
- [ ] **Step 3: Verify** (this confirms `generated::AgentObject` derives `Deserialize` — typify emits it). **Step 4: Re-export `AgentList`. Step 5: Commit** `git commit -m "feat(lens-client): list agents"`.
> ⚠ Build the test JSON from `AgentObject`'s ACTUAL required fields in `generated.rs` (don't guess the field set). If `AgentObject` has non-defaulted required fields beyond `id`, include them.

---

### Task 4: Hosts (read + directories + filesystem)

**Interfaces:** `Client::list_hosts(&self) -> Result<HostList>`; `Client::host(&self, host_id: &HostId) -> Result<HostObject>`; `Client::create_directory(&self, host_id: &HostId, req: &generated::CreateDirectoryRequest) -> Result<HostObject>`; `Client::host_filesystem(&self, host_id: &HostId, path: Option<&str>) -> Result<FilesystemList>` (reuse `sessions::FilesystemList` from Plan 2d, or move it to a shared module).

- [ ] **Step 1: Implement** — hosts are untyped; minimal `HostObject` (id getter), grow later:
```rust
#[derive(Clone, Debug, serde::Deserialize)]
pub struct HostObject {
    id: HostId,
    #[serde(default, rename = "object")]
    object: String,
    // ⚠ grow getters (name, online, provider, …) as the host-picker UI needs them.
}
impl HostObject {
    pub fn id(&self) -> &HostId { &self.id }
    pub fn object(&self) -> &str { &self.object }
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct HostList {
    #[serde(default)]
    pub data: Vec<HostObject>,
    #[serde(default)]
    pub has_more: bool,
}

impl Client {
    pub fn list_hosts(&self) -> Result<HostList> { self.get_json("/v1/hosts", &[]) }
    pub fn host(&self, host_id: &HostId) -> Result<HostObject> {
        self.get_json(&format!("/v1/hosts/{host_id}"), &[])
    }
    pub fn create_directory(&self, host_id: &HostId, req: &crate::generated::CreateDirectoryRequest) -> Result<HostObject> {
        self.send_json(reqwest::Method::POST, &format!("/v1/hosts/{host_id}/directories"), &[], Some(req))
    }
    pub fn host_filesystem(&self, host_id: &HostId, path: Option<&str>) -> Result<crate::sessions::FilesystemList> {
        let p = match path {
            Some(p) => format!("/v1/hosts/{host_id}/filesystem/{p}"),
            None => format!("/v1/hosts/{host_id}/filesystem"),
        };
        self.get_json(&p, &[])
    }
}
```
- [ ] **Step 2: Re-export `HostObject`, `HostList`. Step 3: Verify** (confirm `host_filesystem` returns the Plan-2d `FilesystemList` shape; if host fs differs from env fs, model a separate type). **Step 4: Commit** `git commit -m "feat(lens-client): hosts read + directories + filesystem"`.
> ⚠ `HostList`/`HostObject`/`create_directory` response shapes are untyped — confirm the list envelope and grow `HostObject` getters from source as the new-session host-picker is built. `host_filesystem` reuses `FilesystemList`; verify the host fs entry shape matches the env fs entry shape.

---

### Task 5: Launch runner

**Interfaces:** `Client::launch_runner(&self, host_id: &HostId, req: &generated::LaunchRunnerRequest) -> Result<RunnerLaunchResult>`.

- [ ] **Step 1: Implement** — minimal typed result (⚠ grow):
```rust
#[derive(Clone, Debug, serde::Deserialize)]
pub struct RunnerLaunchResult {
    #[serde(default)]
    runner_id: Option<RunnerId>,
    // ⚠ confirm the launch response shape (runner_id? status?) from sessions/host source.
}
impl RunnerLaunchResult {
    pub fn runner_id(&self) -> Option<&RunnerId> { self.runner_id.as_ref() }
}

impl Client {
    /// `POST /v1/hosts/{id}/runners` — launch a runner. `req` = generated
    /// `LaunchRunnerRequest {session_id, workspace, git?}`.
    pub fn launch_runner(&self, host_id: &HostId, req: &crate::generated::LaunchRunnerRequest) -> Result<RunnerLaunchResult> {
        self.send_json(reqwest::Method::POST, &format!("/v1/hosts/{host_id}/runners"), &[], Some(req))
    }
}
```
- [ ] **Step 2: Re-export. Step 3: Verify. Step 4: Commit** `git commit -m "feat(lens-client): launch runner"`.

---

### Task 6: Policies — server-wide + registry + session-scoped

**Interfaces:**
- On `Client`: `list_policies() -> Result<PolicyList>`, `create_policy(&generated::CreateDefaultPolicyRequest) -> Result<PolicyObject>`, `policy(&PolicyId) -> Result<PolicyObject>`, `delete_policy(&PolicyId) -> Result<()>`, `policy_registry() -> Result<PolicyRegistry>`.
- On `Sessions` (in `sessions.rs`): `policies(&SessionId) -> Result<PolicyList>`, `create_policy(&SessionId, &generated::CreateSessionPolicyRequest) -> Result<PolicyObject>`, `session_policy(&SessionId, &PolicyId) -> Result<PolicyObject>`, `delete_policy(&SessionId, &PolicyId) -> Result<()>`, `evaluate_policy(&SessionId, &serde_json::Value) -> Result<PolicyEvaluation>`.

- [ ] **Step 1: Implement** — policies are untyped; minimal wrappers (`PolicyObject` id getter, `PolicyList`, `PolicyRegistry`, `PolicyEvaluation`), grow as the policy UI lands. Use `PolicyId` from `ids`. Example (server-wide in `registries.rs`):
```rust
use crate::ids::PolicyId;

#[derive(Clone, Debug, serde::Deserialize)]
pub struct PolicyObject { id: PolicyId }
impl PolicyObject { pub fn id(&self) -> &PolicyId { &self.id } }

#[derive(Clone, Debug, serde::Deserialize)]
pub struct PolicyList { #[serde(default)] pub data: Vec<PolicyObject>, #[serde(default)] pub has_more: bool }

#[derive(Clone, Debug, serde::Deserialize)]
pub struct PolicyRegistry { /* ⚠ catalog shape — model when the policy authoring UI consumes it */ }

#[derive(Clone, Debug, serde::Deserialize)]
pub struct PolicyEvaluation { /* ⚠ evaluate result shape — model when consumed */ }

impl Client {
    pub fn list_policies(&self) -> Result<PolicyList> { self.get_json("/v1/policies", &[]) }
    pub fn create_policy(&self, req: &crate::generated::CreateDefaultPolicyRequest) -> Result<PolicyObject> {
        self.send_json(reqwest::Method::POST, "/v1/policies", &[], Some(req))
    }
    pub fn policy(&self, id: &PolicyId) -> Result<PolicyObject> { self.get_json(&format!("/v1/policies/{id}"), &[]) }
    pub fn delete_policy(&self, id: &PolicyId) -> Result<()> {
        let _: serde_json::Value = self.send_json::<serde_json::Value, ()>(reqwest::Method::DELETE, &format!("/v1/policies/{id}"), &[], None)?;
        Ok(())
    }
    pub fn policy_registry(&self) -> Result<PolicyRegistry> { self.get_json("/v1/policy-registry", &[]) }
}
```
Session-scoped (in `sessions.rs`, reuse `registries::{PolicyObject, PolicyList, PolicyEvaluation}` or move them to a shared `policies` module):
```rust
    pub fn policies(&self, id: &SessionId) -> Result<crate::registries::PolicyList> {
        self.client.get_json(&format!("/v1/sessions/{id}/policies"), &[])
    }
    pub fn create_policy(&self, id: &SessionId, req: &crate::generated::CreateSessionPolicyRequest) -> Result<crate::registries::PolicyObject> {
        self.client.send_json(reqwest::Method::POST, &format!("/v1/sessions/{id}/policies"), &[], Some(req))
    }
    pub fn session_policy(&self, id: &SessionId, policy_id: &crate::ids::PolicyId) -> Result<crate::registries::PolicyObject> {
        self.client.get_json(&format!("/v1/sessions/{id}/policies/{policy_id}"), &[])
    }
    pub fn delete_policy(&self, id: &SessionId, policy_id: &crate::ids::PolicyId) -> Result<()> {
        let _: serde_json::Value = self.client.send_json::<serde_json::Value, ()>(
            reqwest::Method::DELETE, &format!("/v1/sessions/{id}/policies/{policy_id}"), &[], None)?;
        Ok(())
    }
    pub fn evaluate_policy(&self, id: &SessionId, input: &serde_json::Value) -> Result<crate::registries::PolicyEvaluation> {
        self.client.send_json(reqwest::Method::POST, &format!("/v1/sessions/{id}/policies/evaluate"), &[], Some(input))
    }
```
- [ ] **Step 2: Implement empty-struct caveat** — `PolicyRegistry`/`PolicyEvaluation` as empty structs won't expose anything; that's intentional (model fields when a consumer lands). If clippy objects to empty structs, add a single `raw_present: ()` or derive only what's needed; prefer `#[derive(Deserialize)] pub struct PolicyRegistry {}` (valid). Add a golden parse test for `PolicyObject` (`{"id":"pol_1"}` → id == "pol_1").
- [ ] **Step 3: Re-export the policy types. Step 4: Verify. Step 5: Commit** `git commit -m "feat(lens-client): server + session policies, policy registry"`.
> ⚠ `PolicyObject`/`PolicyRegistry`/`PolicyEvaluation` shapes are untyped — grow getters/fields from source as the policy UI is built. `evaluate_policy` input is a `Value` *request* (hypothetical-input payload the caller constructs), not a leaked read.

---

## Self-review

- **Spec coverage:** agents list ✓; hosts list/get/directories/filesystem ✓; runners list/status/launch ✓; policies server-wide (list/create/get/delete/registry) + session-scoped (list/create/get/delete/evaluate) ✓; `/v1/me` ✓. `/v1/info`, `/api/version`, `/health` already in the foundation; hosts/agents correctly read-only (no CRUD invented).
- **Grounded vs ⚠:** `RunnerStatus` and `Me` are pinned; `AgentObject`/request bodies are generated. Hosts/policies/runner-list/launch-result are real minimal wrappers marked ⚠ for field growth from source — never a `pub Vec<Value>` shipped (the `RunnerList.data` caveat is called out explicitly with a resolve-before-merge instruction).
- **Type consistency:** registry methods on `Client` via `get_json`/`send_json`; session policies on `Sessions`; ids (`HostId`/`RunnerId`/`PolicyId`) from the foundation; generated request types by `generated::` path.

## REST surface complete (2a–2e)

After 2e lands, the §3 HTTP surface is modeled end-to-end (writes typed via generated/hand types; reads typed via wrappers with no `Value` reaching consumers). Remaining lens-client work is the **streaming layer**:
- **Plan 3 — SSE taxonomy + reader thread + reconnect**, which also delivers the typed conversation-item union that upgrades `Sessions::items()` (deferred from 2b).
- **Plan 4 — WS terminal attach + verification consolidation** (`xtask drift`/`live-test`, golden-SSE captures, resolving the 2b–2e ⚠ field verifications).

**Checkpoint before Plan 3:** reassess whether to keep tracking `0.3.0.dev0` or wait for a `0.3.0` release tag — the streaming taxonomy is where re-vendor churn is expensive (see STATUS).
