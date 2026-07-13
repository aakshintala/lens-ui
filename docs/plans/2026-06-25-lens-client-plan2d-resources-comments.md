# lens-client Plan 2d — Resources, terminals, comments, session meta

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans. Steps use checkbox (`- [ ]`).

**Goal:** The environment-scoped resource surface (filesystem/diff/search/shell/files), terminals REST, line comments, and session metadata (labels/owner/permissions) on the `Sessions` subservice.

**Architecture:** Extends Plans 2a–2c; reuses `Client::get_json`/`send_json`/`send_multipart` and the typed-wrapper read pattern (private fields + typed getters, no `Value` to consumers). Request bodies use **generated** types (`AddCommentRequest`, `UpdateCommentRequest`, `SendCommentsRequest`, `GrantPermissionRequest`). Read wrappers are modeled with the getters a near-term consumer needs; where the full server shape isn't yet pinned, the wrapper is real and minimal with a cited growth path (⚠ notes) — add getters as consumers demand, mining field names from omnigent source, never by leaking `Value`.

**Tech Stack:** Rust (edition 2024), `reqwest` blocking (+ multipart from 2c), `serde`. No async (D2).

## Global Constraints

- Plans 2a–2c constraints apply. No `Value` in public read signatures; reuse helpers + `decode_json`; generated.rs never hand-edited; live tests gated.
- **Ground truth (omnigent `0.3.0.dev0`, `36b2a11c`):**
  - `GET …/resources` → `SessionResourcePaginatedList` (**generated**). `GET …/resources/{resource_id}` → untyped object.
  - Environments: `GET …/resources/environments`, `…/{env_id}`, `…/{env_id}/changes` → untyped.
  - Filesystem: `GET …/{env_id}/filesystem`, `…/filesystem/{relative_path}` (read; PUT/PATCH/DELETE also exist — Lens read-only for now). `GET …/{env_id}/diff/{relative_path}` → `{before, after}` strings (NOT unified diff).
  - **Search is GET** (`sessions.py:16330`): `…/{env_id}/search?q=<req>&include=<globs>&exclude=<globs>&limit=<≤500>` → `{object:"list", data:[FilesystemEntry], has_more}`. `FilesystemEntry` keys (`runner/app.py:14548-14556`): `id`, `object` (="session.environment.filesystem.entry"), `name`, `path`, `type`, `bytes`, `modified_at`.
  - `POST …/{env_id}/shell` → untyped one-shot command result.
  - Files: `GET/POST …/resources/files` (POST = multipart `Body_upload_session_file_…`), `GET …/files/{file_id}`, `GET …/files/{file_id}/content`.
  - Terminals: `GET/POST …/resources/terminals`, `DELETE …/terminals/{terminal_id}`, `POST …/terminals/{terminal_id}/transfer`. (WS attach is Plan 4, NOT here.)
  - Comments: `POST …/comments` (`AddCommentRequest`), `PATCH …/comments/{comment_id}` (`UpdateCommentRequest`), `DELETE …/comments/{comment_id}`, `POST …/comments/send` (`SendCommentsRequest`). All return bare `object`.
  - `GET …/labels` (GET only) → `SessionLabelsResponse {id, labels}` (**generated**); mutate via `PATCH /sessions/{id}` (Plan 2c). `GET …/owner` → untyped. Permissions: `GET …/permissions` (untyped) / `PUT …/permissions` (`GrantPermissionRequest`, levels 1–3) / `DELETE …/permissions/{target_user_id}`.
  - Generated request types present: `AddCommentRequest`, `UpdateCommentRequest`, `SendCommentsRequest`, `GrantPermissionRequest`, `SessionLabelsResponse`, `SessionResourcePaginatedList`. (Verify `UpdateCommentRequest`/`SendCommentsRequest` exist in `generated.rs`; if a name differs, use the actual generated name.)

---

### Task 1: Filesystem search + `FilesystemEntry` (fully-pinned shape)

**Files:** Create `crates/lens-client/src/resources.rs`; modify `sessions.rs` (add a `resources()` accessor or fold methods onto `Sessions`), `lib.rs`. (Choose: put resource methods on `Sessions` directly — simplest — or a nested `Resources<'a>` subservice. This plan adds them to `Sessions` for brevity; if it grows large, refactor to `Sessions::resources()` later.)

**Interfaces:** Produces `sessions::FilesystemEntry` (getters: `id`, `name`, `path`, `entry_type`, `bytes`, `modified_at`), `sessions::FilesystemList { data: Vec<FilesystemEntry>, has_more: bool }`, and `Sessions::search(&self, id: &SessionId, env_id: &str, query: &SearchQuery) -> Result<FilesystemList>`.

- [ ] **Step 1: Failing test**:
```rust
    #[test]
    fn filesystem_list_parses() {
        let body = r#"{"object":"list","has_more":false,"data":[
            {"id":"e1","object":"session.environment.filesystem.entry","name":"main.rs",
             "path":"src/main.rs","type":"file","bytes":1024,"modified_at":1719331200}]}"#;
        let l: FilesystemList = serde_json::from_str(body).unwrap();
        assert_eq!(l.data[0].path(), "src/main.rs");
        assert_eq!(l.data[0].entry_type(), "file");
        assert_eq!(l.data[0].bytes(), Some(1024));
    }

    #[test]
    fn search_query_builds() {
        let q = SearchQuery::new("fn main").include("*.rs").limit(100);
        let pairs = q.to_query();
        assert!(pairs.contains(&("q", "fn main".to_string())));
        assert!(pairs.contains(&("include", "*.rs".to_string())));
        assert!(pairs.contains(&("limit", "100".to_string())));
    }
```
- [ ] **Step 2: Run** → FAIL.
- [ ] **Step 3: Implement** in `sessions.rs` (or `resources.rs`):
```rust
/// A filesystem entry (`runner/app.py:14548-14556`).
#[derive(Clone, Debug, serde::Deserialize)]
pub struct FilesystemEntry {
    id: String,
    name: String,
    path: String,
    #[serde(rename = "type")]
    entry_type: String,
    #[serde(default)]
    bytes: Option<u64>,
    #[serde(default)]
    modified_at: Option<i64>,
}
impl FilesystemEntry {
    pub fn id(&self) -> &str { &self.id }
    pub fn name(&self) -> &str { &self.name }
    pub fn path(&self) -> &str { &self.path }
    pub fn entry_type(&self) -> &str { &self.entry_type }
    pub fn bytes(&self) -> Option<u64> { self.bytes }
    pub fn modified_at(&self) -> Option<i64> { self.modified_at }
}

/// `{object:"list", data:[FilesystemEntry], has_more}` envelope.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct FilesystemList {
    pub data: Vec<FilesystemEntry>,
    #[serde(default)]
    pub has_more: bool,
}

/// Query for environment search (`q` required; `include`/`exclude` globs; `limit` ≤ 500).
#[derive(Clone, Debug)]
pub struct SearchQuery {
    q: String,
    include: Option<String>,
    exclude: Option<String>,
    limit: Option<u32>,
}
impl SearchQuery {
    pub fn new(q: impl Into<String>) -> Self { Self { q: q.into(), include: None, exclude: None, limit: None } }
    pub fn include(mut self, g: impl Into<String>) -> Self { self.include = Some(g.into()); self }
    pub fn exclude(mut self, g: impl Into<String>) -> Self { self.exclude = Some(g.into()); self }
    pub fn limit(mut self, n: u32) -> Self { self.limit = Some(n.min(500)); self }
    fn to_query(&self) -> Vec<(&'static str, String)> {
        let mut v = vec![("q", self.q.clone())];
        if let Some(g) = &self.include { v.push(("include", g.clone())); }
        if let Some(g) = &self.exclude { v.push(("exclude", g.clone())); }
        if let Some(n) = self.limit { v.push(("limit", n.to_string())); }
        v
    }
}
```
and to `impl<'a> Sessions<'a>`:
```rust
    /// `GET …/resources/environments/{env_id}/search` — server-side fs search.
    pub fn search(&self, id: &SessionId, env_id: &str, query: &SearchQuery) -> Result<FilesystemList> {
        self.client.get_json(
            &format!("/v1/sessions/{id}/resources/environments/{env_id}/search"),
            &query.to_query(),
        )
    }
```
- [ ] **Step 4: Re-export. Step 5: Verify. Step 6: Commit** `git commit -m "feat(lens-client): env search + FilesystemEntry"`.

---

### Task 2: Filesystem listing, file read, diff

**Interfaces:** `Sessions::list_filesystem(&self, id, env_id) -> Result<FilesystemList>`; `Sessions::read_file(&self, id, env_id, relative_path) -> Result<FileContent>`; `Sessions::diff(&self, id, env_id, relative_path) -> Result<FileDiff>` where `FileDiff { before: String, after: String }` (typed getters).

- [ ] **Step 1: Failing test** for the one pinned shape:
```rust
    #[test]
    fn file_diff_parses_before_after() {
        let d: FileDiff = serde_json::from_str(r#"{"before":"a\n","after":"b\n"}"#).unwrap();
        assert_eq!(d.before(), "a\n");
        assert_eq!(d.after(), "b\n");
    }
```
- [ ] **Step 2: Run** → FAIL. **Step 3: Implement**:
```rust
/// `GET …/diff/{relative_path}` — `{before, after}` (NOT a unified diff).
#[derive(Clone, Debug, serde::Deserialize)]
pub struct FileDiff { before: String, after: String }
impl FileDiff {
    pub fn before(&self) -> &str { &self.before }
    pub fn after(&self) -> &str { &self.after }
}

/// `GET …/filesystem/{relative_path}` — file read. ⚠ Mine the exact key names
/// (content vs base64, encoding, size) from the runner source when wiring the
/// editor; start with a `content()` getter over the field the runner returns.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct FileContent {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    encoding: Option<String>,
}
impl FileContent {
    pub fn content(&self) -> Option<&str> { self.content.as_deref() }
    pub fn encoding(&self) -> Option<&str> { self.encoding.as_deref() }
}
```
and methods (note: `relative_path` segments must be URL-encoded — use a tiny percent-encode or rely on `reqwest`'s path being pre-joined; verify the server expects a single `{relative_path}` catch-all):
```rust
    pub fn list_filesystem(&self, id: &SessionId, env_id: &str) -> Result<FilesystemList> {
        self.client.get_json(&format!("/v1/sessions/{id}/resources/environments/{env_id}/filesystem"), &[])
    }
    pub fn read_file(&self, id: &SessionId, env_id: &str, relative_path: &str) -> Result<FileContent> {
        self.client.get_json(&format!("/v1/sessions/{id}/resources/environments/{env_id}/filesystem/{relative_path}"), &[])
    }
    pub fn diff(&self, id: &SessionId, env_id: &str, relative_path: &str) -> Result<FileDiff> {
        self.client.get_json(&format!("/v1/sessions/{id}/resources/environments/{env_id}/diff/{relative_path}"), &[])
    }
```
> ⚠ `FileContent`'s field names are unverified — confirm against the runner filesystem-read handler before relying on `content()`; expand getters as the editor needs them. `relative_path` URL-encoding: confirm whether the server route is a `{relative_path:path}` catch-all (slashes allowed) and encode accordingly.

- [ ] **Step 4: Re-export. Step 5: Verify. Step 6: Commit** `git commit -m "feat(lens-client): filesystem list/read/diff"`.

---

### Task 3: Changes list, shell, resources list/get, environments

**Interfaces:** `Sessions::changed_files(id, env_id) -> Result<FilesystemList>` (changed-files list — same entry shape; ⚠ verify); `Sessions::shell(id, env_id, command) -> Result<ShellResult>`; `Sessions::resources(id) -> Result<generated::SessionResourcePaginatedList>`; `Sessions::resource(id, resource_id) -> Result<ResourceObject>`; `Sessions::environments(id) -> Result<Vec<ResourceObject>>` (⚠ envelope shape); `Sessions::environment(id, env_id) -> Result<ResourceObject>`.

- [ ] **Step 1: Implement** (these are mostly untyped — model minimal real wrappers with id/type getters; grow as consumers need). Key code:
```rust
/// One-shot shell result (`POST …/shell`). ⚠ Confirm field names (stdout/stderr/
/// exit_code) against the runner shell handler; these are the conventional keys.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct ShellResult {
    #[serde(default)]
    stdout: String,
    #[serde(default)]
    stderr: String,
    #[serde(default)]
    exit_code: Option<i64>,
}
impl ShellResult {
    pub fn stdout(&self) -> &str { &self.stdout }
    pub fn stderr(&self) -> &str { &self.stderr }
    pub fn exit_code(&self) -> Option<i64> { self.exit_code }
}

/// A generic session resource (environment/terminal/file). Untyped server-side;
/// expose id/object now, grow typed getters as the resource UI needs them.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct ResourceObject {
    id: String,
    #[serde(default, rename = "object")]
    object: String,
}
impl ResourceObject {
    pub fn id(&self) -> &str { &self.id }
    pub fn object(&self) -> &str { &self.object }
}
```
Methods (shell body shape — ⚠ confirm; conventional `{command}`):
```rust
    pub fn shell(&self, id: &SessionId, env_id: &str, command: &str) -> Result<ShellResult> {
        let body = serde_json::json!({ "command": command });
        self.client.send_json(reqwest::Method::POST,
            &format!("/v1/sessions/{id}/resources/environments/{env_id}/shell"), &[], Some(&body))
    }
    pub fn resources(&self, id: &SessionId) -> Result<crate::generated::SessionResourcePaginatedList> {
        self.client.get_json(&format!("/v1/sessions/{id}/resources"), &[])
    }
    pub fn resource(&self, id: &SessionId, resource_id: &str) -> Result<ResourceObject> {
        self.client.get_json(&format!("/v1/sessions/{id}/resources/{resource_id}"), &[])
    }
    pub fn environment(&self, id: &SessionId, env_id: &str) -> Result<ResourceObject> {
        self.client.get_json(&format!("/v1/sessions/{id}/resources/environments/{env_id}"), &[])
    }
```
> ⚠ `environments(id)` (the list) — confirm whether the envelope is `{data:[...]}` or a bare array, then model `environments(&self, id) -> Result<Vec<ResourceObject>>` accordingly. `shell` body and `ShellResult`/`changed_files` shapes are conventional-but-unverified — confirm against the runner handlers; the `command` POST body may carry more (timeout, cwd).

- [ ] **Step 2: Re-export. Step 3: Verify. Step 4: Commit** `git commit -m "feat(lens-client): shell + resources/environments reads"`.

---

### Task 4: Files — list / upload / metadata / content

**Interfaces:** `Sessions::files(id) -> Result<FilesList>`; `Sessions::upload_file(id, bytes, filename, mime) -> Result<FileResource>`; `Sessions::file(id, file_id) -> Result<FileResource>`; `Sessions::file_content(id, file_id) -> Result<Vec<u8>>` (raw bytes — content endpoints return the file body, not JSON; this is the one non-JSON read).

- [ ] **Step 1: Implement** — file metadata wrapper (⚠ mine fields: id, filename, bytes, mime, created_at) + the upload multipart (`Body_upload_session_file_…` field name — verify, conventionally `file`):
```rust
#[derive(Clone, Debug, serde::Deserialize)]
pub struct FileResource {
    id: FileId,
    #[serde(default)]
    filename: Option<String>,
    #[serde(default)]
    bytes: Option<u64>,
}
impl FileResource {
    pub fn id(&self) -> &FileId { &self.id }
    pub fn filename(&self) -> Option<&str> { self.filename.as_deref() }
    pub fn bytes(&self) -> Option<u64> { self.bytes }
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct FilesList { pub data: Vec<FileResource>, #[serde(default)] pub has_more: bool }
```
(import `crate::ids::FileId`.) `file_content` returns raw bytes — add a small `Client` helper that returns `Result<Vec<u8>>` instead of decode_json:
```rust
    // in client.rs:
    pub(crate) fn get_bytes(&self, path: &str) -> crate::error::Result<Vec<u8>> {
        let url = self.conn().url(path)?;
        let resp = self.conn().auth.apply(self.http().get(url)).send()?;
        let status = resp.status().as_u16();
        if !(200..=299).contains(&status) {
            return Err(crate::http::check_status(path, status).unwrap_err());
        }
        Ok(resp.bytes()?.to_vec())
    }
```
and `Sessions` methods:
```rust
    pub fn files(&self, id: &SessionId) -> Result<FilesList> {
        self.client.get_json(&format!("/v1/sessions/{id}/resources/files"), &[])
    }
    pub fn upload_file(&self, id: &SessionId, bytes: Vec<u8>, filename: &str, mime: &str) -> Result<FileResource> {
        let part = reqwest::blocking::multipart::Part::bytes(bytes)
            .file_name(filename.to_string())
            .mime_str(mime).map_err(crate::error::ClientError::Network)?;
        let form = reqwest::blocking::multipart::Form::new().part("file", part); // ⚠ verify field name
        self.client.send_multipart(reqwest::Method::POST, &format!("/v1/sessions/{id}/resources/files"), form)
    }
    pub fn file(&self, id: &SessionId, file_id: &FileId) -> Result<FileResource> {
        self.client.get_json(&format!("/v1/sessions/{id}/resources/files/{file_id}"), &[])
    }
    pub fn file_content(&self, id: &SessionId, file_id: &FileId) -> Result<Vec<u8>> {
        self.client.get_bytes(&format!("/v1/sessions/{id}/resources/files/{file_id}/content"))
    }
```
- [ ] **Step 2: Re-export. Step 3: Verify. Step 4: Commit** `git commit -m "feat(lens-client): file resources list/upload/metadata/content"`.
> ⚠ Verify the upload multipart field name and `FileResource` field names against `Body_upload_session_file_…` / the files handler.

---

### Task 5: Terminals — list / create / delete / transfer

**Interfaces:** `Sessions::terminals(id) -> Result<Vec<ResourceObject>>` (⚠ envelope); `Sessions::create_terminal(id, opts) -> Result<ResourceObject>`; `Sessions::delete_terminal(id, terminal_id) -> Result<()>`; `Sessions::transfer_terminal(id, terminal_id, target_session_id) -> Result<()>`. Use `TerminalId` from `ids`.

- [ ] **Step 1: Implement** — terminals are untyped objects; reuse `ResourceObject`. For `delete`/`transfer` returning bare object/no useful body, deserialize into `serde_json::Value` internally but return `()` (don't leak it):
```rust
    pub fn create_terminal(&self, id: &SessionId, opts: &serde_json::Value) -> Result<ResourceObject> {
        // opts e.g. {"launch_args": [...]}; ⚠ confirm create body shape.
        self.client.send_json(reqwest::Method::POST, &format!("/v1/sessions/{id}/resources/terminals"), &[], Some(opts))
    }
    pub fn delete_terminal(&self, id: &SessionId, terminal_id: &crate::ids::TerminalId) -> Result<()> {
        let _: serde_json::Value = self.client.send_json::<serde_json::Value, ()>(
            reqwest::Method::DELETE, &format!("/v1/sessions/{id}/resources/terminals/{terminal_id}"), &[], None)?;
        Ok(())
    }
    pub fn transfer_terminal(&self, id: &SessionId, terminal_id: &crate::ids::TerminalId, target: &SessionId) -> Result<()> {
        let body = serde_json::json!({ "target_session_id": target.as_str() }); // ⚠ confirm body key
        let _: serde_json::Value = self.client.send_json(
            reqwest::Method::POST, &format!("/v1/sessions/{id}/resources/terminals/{terminal_id}/transfer"), &[], Some(&body))?;
        Ok(())
    }
```
- [ ] **Step 2: Re-export. Step 3: Verify. Step 4: Commit** `git commit -m "feat(lens-client): terminals REST (list/create/delete/transfer)"`.
> ⚠ Confirm: terminals list envelope; create-terminal body; transfer body key (`target_session_id`?). WS attach is Plan 4, not here.

---

### Task 6: Comments — add / edit / delete / send

**Interfaces:** `Sessions::add_comment(id, req: &generated::AddCommentRequest) -> Result<CommentObject>`; `edit_comment(id, comment_id, req: &generated::UpdateCommentRequest) -> Result<CommentObject>`; `delete_comment(id, comment_id) -> Result<()>`; `send_comments(id, req: &generated::SendCommentsRequest) -> Result<()>`. Use `CommentId`.

- [ ] **Step 1: Implement** — comments return bare `object`; model a minimal `CommentObject` (id getter) and use the generated request types:
```rust
#[derive(Clone, Debug, serde::Deserialize)]
pub struct CommentObject {
    #[serde(default)]
    id: Option<String>,
}
impl CommentObject { pub fn id(&self) -> Option<&str> { self.id.as_deref() } }
```
```rust
    pub fn add_comment(&self, id: &SessionId, req: &crate::generated::AddCommentRequest) -> Result<CommentObject> {
        self.client.send_json(reqwest::Method::POST, &format!("/v1/sessions/{id}/comments"), &[], Some(req))
    }
    pub fn edit_comment(&self, id: &SessionId, comment_id: &crate::ids::CommentId, req: &crate::generated::UpdateCommentRequest) -> Result<CommentObject> {
        self.client.send_json(reqwest::Method::PATCH, &format!("/v1/sessions/{id}/comments/{comment_id}"), &[], Some(req))
    }
    pub fn delete_comment(&self, id: &SessionId, comment_id: &crate::ids::CommentId) -> Result<()> {
        let _: serde_json::Value = self.client.send_json::<serde_json::Value, ()>(
            reqwest::Method::DELETE, &format!("/v1/sessions/{id}/comments/{comment_id}"), &[], None)?;
        Ok(())
    }
    pub fn send_comments(&self, id: &SessionId, req: &crate::generated::SendCommentsRequest) -> Result<()> {
        let _: serde_json::Value = self.client.send_json(
            reqwest::Method::POST, &format!("/v1/sessions/{id}/comments/send"), &[], Some(req))?;
        Ok(())
    }
```
- [ ] **Step 2: Re-export. Step 3: Verify** (confirm `UpdateCommentRequest`/`SendCommentsRequest` exist in `generated.rs`; if the generated name differs, use it). **Step 4: Commit** `git commit -m "feat(lens-client): line comments add/edit/delete/send"`.

---

### Task 7: Session meta — labels / owner / permissions

**Interfaces:** `Sessions::labels(id) -> Result<generated::SessionLabelsResponse>`; `Sessions::owner(id) -> Result<OwnerInfo>`; `Sessions::permissions(id) -> Result<PermissionsInfo>`; `Sessions::grant_permission(id, req: &generated::GrantPermissionRequest) -> Result<()>`; `Sessions::revoke_permission(id, target_user_id) -> Result<()>`.

- [ ] **Step 1: Implement** — labels uses the generated response; owner/permissions are untyped (minimal wrappers, grow later):
```rust
#[derive(Clone, Debug, serde::Deserialize)]
pub struct OwnerInfo { #[serde(default)] user_id: Option<String> }
impl OwnerInfo { pub fn user_id(&self) -> Option<&str> { self.user_id.as_deref() } }

#[derive(Clone, Debug, serde::Deserialize)]
pub struct PermissionsInfo {
    // ⚠ Untyped server-side; model the grants once the sharing UI consumes them
    // (e.g. a map of user_id -> level). Start minimal.
    #[serde(default)]
    public_level: Option<i64>,
}
impl PermissionsInfo { pub fn public_level(&self) -> Option<i64> { self.public_level } }
```
```rust
    pub fn labels(&self, id: &SessionId) -> Result<crate::generated::SessionLabelsResponse> {
        self.client.get_json(&format!("/v1/sessions/{id}/labels"), &[])
    }
    pub fn owner(&self, id: &SessionId) -> Result<OwnerInfo> {
        self.client.get_json(&format!("/v1/sessions/{id}/owner"), &[])
    }
    pub fn permissions(&self, id: &SessionId) -> Result<PermissionsInfo> {
        self.client.get_json(&format!("/v1/sessions/{id}/permissions"), &[])
    }
    /// Grant levels 1–3 only (read/edit/manage); owner (4) is not grantable (server 403s).
    pub fn grant_permission(&self, id: &SessionId, req: &crate::generated::GrantPermissionRequest) -> Result<()> {
        let _: serde_json::Value = self.client.send_json(
            reqwest::Method::PUT, &format!("/v1/sessions/{id}/permissions"), &[], Some(req))?;
        Ok(())
    }
    pub fn revoke_permission(&self, id: &SessionId, target_user_id: &str) -> Result<()> {
        let _: serde_json::Value = self.client.send_json::<serde_json::Value, ()>(
            reqwest::Method::DELETE, &format!("/v1/sessions/{id}/permissions/{target_user_id}"), &[], None)?;
        Ok(())
    }
```
- [ ] **Step 2: Re-export `OwnerInfo`, `PermissionsInfo`. Step 3: Verify. Step 4: Commit** `git commit -m "feat(lens-client): session labels/owner/permissions"`.
> Out of scope here: `POST …/mcp` (a JSON-RPC 2.0 proxy, not a session read — model separately if/when Lens needs in-app MCP tool calls). `…/policies` (session-scoped) is grouped with server policies in Plan 2e.

---

## Self-review

- **Spec coverage:** search (+FilesystemEntry, fully pinned) ✓; filesystem list/read/diff ✓; changes/shell/resources/environments ✓; files list/upload/metadata/content (raw bytes) ✓; terminals list/create/delete/transfer ✓; comments add/edit/delete/send ✓; labels/owner/permissions ✓. mcp + session policies deliberately routed elsewhere.
- **Grounded vs ⚠-to-verify:** `FilesystemEntry`, `FileDiff`, search query, labels/comments/permissions request types are pinned (source/generated). `FileContent`/`ShellResult`/`ResourceObject`/`FileResource`/terminal+transfer bodies + several multipart field names are **real minimal wrappers marked ⚠** — the implementer confirms exact field/key names against omnigent source as each consumer lands. This is the lazy-accessor growth path, not placeholders: every method compiles and returns a typed wrapper; no `Value` reaches a consumer.
- **Type consistency:** all reuse `get_json`/`send_json`/`send_multipart`/`get_bytes`, foundation ids (`FileId`/`TerminalId`/`CommentId`/`SessionId`), and generated request types by their `generated::` path.

## Next

Plan 2e (registries: agents/hosts/runners/policies/info/me). After 2b–2e land, revisit `items()` + the SSE taxonomy in Plan 3, and resolve all ⚠ field-name verifications with golden-response captures as the state-model layer begins consuming these reads.
