# lens-client Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stand up the `lens-client` crate foundation — branded types, error model, connection/auth, the typify codegen pipeline, and a blocking HTTP `Client` that handshakes + passes the contract gate against a live omnigent server.

**Architecture:** A single Rust crate that is the typed seam over omnigent's HTTP/SSE/WS contract. This plan covers the non-streaming foundation only: synchronous/blocking `reqwest`, no async runtime. Wire types are generated once from the vendored `openapi.json` via a `typify`-based `xtask` and committed to `generated.rs`; a hand-written layer wraps them.

**Tech Stack:** Rust (edition 2024, toolchain 1.91.1), `reqwest` (blocking), `serde`/`serde_json`, `thiserror`, `url`, `typify` (in `xtask`).

## Global Constraints

- Edition `2024`, `rust-version = "1.91"`; workspace lints apply (`unsafe_code = "deny"`, `clippy::all = "deny"`, `unused_must_use = "deny"`) — copy `lints.workspace = true` into the crate.
- No `unsafe`. No async runtime, no `tokio`, no `flume` (decision D2, `typed-client-implementation.md`).
- Ground truth: `vendor/omnigent-0.3.0.dev0/openapi.json`, pin `36b2a11c` (ADR-0001). Never trust the spec's stale `info.version` (`0.1.0`).
- `PINNED_OMNIGENT_VERSION = "0.3.0.dev0"`.
- `generated.rs` is machine-generated — never hand-edited; tweaks live in wrapper modules.
- Default `cargo test` must pass with **no server running**; anything needing a live server is gated behind `--features live-tests` + `LENS_OMNIGENT_URL`.
- Contract endpoints (verbatim, `typed-client.md` §3/§8): `GET /api/version` → `{"version": "<semver>"}`; `GET /v1/info` → `{accounts_enabled, login_url, needs_setup, databricks_features, managed_sandboxes_enabled, sandbox_provider}` (no version field); `GET /health` liveness only.

---

### Task 1: Crate skeleton + error model

**Files:**
- Create: `crates/lens-client/Cargo.toml`
- Create: `crates/lens-client/src/lib.rs`
- Create: `crates/lens-client/src/error.rs`

**Interfaces:**
- Produces: `lens_client::error::ClientError` (enum), `lens_client::error::Result<T> = std::result::Result<T, ClientError>`, `lens_client::PINNED_OMNIGENT_VERSION: &str`.

- [ ] **Step 1: Create the crate manifest**

`crates/lens-client/Cargo.toml`:
```toml
[package]
name = "lens-client"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
authors.workspace = true

[lints]
workspace = true

[dependencies]
reqwest = { version = "0.12", default-features = false, features = ["blocking", "json", "rustls-tls"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
url = { version = "2", features = ["serde"] }

[features]
live-tests = []
```

- [ ] **Step 2: Write the failing test for the error surface**

`crates/lens-client/src/error.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract_mismatch_displays_expected_and_actual() {
        let e = ClientError::ContractMismatch { expected: "0.3.0.dev0", actual: "0.2.0".into() };
        let s = e.to_string();
        assert!(s.contains("0.3.0.dev0"), "got: {s}");
        assert!(s.contains("0.2.0"), "got: {s}");
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p lens-client error::`
Expected: FAIL — `cannot find type ClientError`.

- [ ] **Step 4: Implement the error model**

Prepend to `crates/lens-client/src/error.rs`:
```rust
use thiserror::Error;

pub type Result<T> = std::result::Result<T, ClientError>;

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("auth failed (status {status})")]
    Auth { status: u16 },

    #[error("not found: {what}")]
    NotFound { what: String },

    #[error("server error (status {status}): {body}")]
    Server { status: u16, body: serde_json::Value },

    #[error("contract mismatch: expected {expected}, server reports {actual}")]
    ContractMismatch { expected: &'static str, actual: String },

    #[error("parse error: {0}")]
    Parse(#[from] serde_json::Error),
}
```

- [ ] **Step 5: Create the crate root**

`crates/lens-client/src/lib.rs`:
```rust
//! `lens-client` — the typed seam over omnigent's HTTP/SSE/WS contract.
//! See `docs/design/typed-client.md` (contract) and
//! `docs/design/typed-client-implementation.md` (build decisions).

pub mod error;

/// The omnigent contract version this crate is pinned to (ADR-0001).
pub const PINNED_OMNIGENT_VERSION: &str = "0.3.0.dev0";

pub use error::{ClientError, Result};
```

- [ ] **Step 6: Run tests + clippy to verify pass**

Run: `cargo test -p lens-client && cargo clippy -p lens-client -- -D warnings`
Expected: PASS, no clippy warnings.

- [ ] **Step 7: Commit**

```bash
git add crates/lens-client/Cargo.toml crates/lens-client/src/lib.rs crates/lens-client/src/error.rs
git commit -m "feat(lens-client): crate skeleton + error model"
```

---

### Task 2: Branded id newtypes

**Files:**
- Create: `crates/lens-client/src/ids.rs`
- Modify: `crates/lens-client/src/lib.rs` (add `pub mod ids;`)

**Interfaces:**
- Produces: `lens_client::ids::{SessionId, ElicitationId, HostId, RunnerId, TerminalId, FileId, CommentId, PolicyId, ConnectionId}` — each a `String` newtype with `new`, `as_str`, `Display`, `Serialize`/`Deserialize`, `Clone`, `Eq`, `Hash`.

- [ ] **Step 1: Write the failing test**

`crates/lens-client/src/ids.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_roundtrip_json_and_display() {
        let s = SessionId::new("sess_abc");
        assert_eq!(s.as_str(), "sess_abc");
        assert_eq!(s.to_string(), "sess_abc");
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(json, "\"sess_abc\"");
        let back: SessionId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn distinct_id_types_do_not_unify() {
        // Compile-time guarantee: this block must not compile if uncommented.
        // let _: SessionId = HostId::new("h"); // <- type error by construction
        assert_ne!(
            std::any::TypeId::of::<SessionId>(),
            std::any::TypeId::of::<HostId>()
        );
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p lens-client ids::`
Expected: FAIL — `cannot find type SessionId`.

- [ ] **Step 3: Implement the branded-id macro + types**

Prepend to `crates/lens-client/src/ids.rs`:
```rust
use serde::{Deserialize, Serialize};

macro_rules! branded_id {
    ($($name:ident),+ $(,)?) => {
        $(
            #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
            #[serde(transparent)]
            pub struct $name(String);

            impl $name {
                pub fn new(s: impl Into<String>) -> Self { Self(s.into()) }
                pub fn as_str(&self) -> &str { &self.0 }
            }

            impl std::fmt::Display for $name {
                fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    f.write_str(&self.0)
                }
            }
        )+
    };
}

branded_id!(
    SessionId, ElicitationId, HostId, RunnerId,
    TerminalId, FileId, CommentId, PolicyId, ConnectionId,
);
```

- [ ] **Step 4: Register the module**

In `crates/lens-client/src/lib.rs`, add after `pub mod error;`:
```rust
pub mod ids;
```

- [ ] **Step 5: Run tests + clippy**

Run: `cargo test -p lens-client ids:: && cargo clippy -p lens-client -- -D warnings`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/lens-client/src/ids.rs crates/lens-client/src/lib.rs
git commit -m "feat(lens-client): branded id newtypes"
```

---

### Task 3: Connection + auth

**Files:**
- Create: `crates/lens-client/src/connection.rs`
- Modify: `crates/lens-client/src/lib.rs` (add `pub mod connection;`)

**Interfaces:**
- Consumes: `ids::ConnectionId`.
- Produces:
  - `connection::Auth` enum: `None`, `Bearer { token: String }`, `Cookie { value: String }`, `ForwardedEmail { email: String }`.
  - `connection::Auth::apply(&self, rb: reqwest::blocking::RequestBuilder) -> reqwest::blocking::RequestBuilder`.
  - `connection::Connection { id: ConnectionId, base_url: url::Url, auth: Auth }` with `Connection::new(id, base_url, auth)` and `fn url(&self, path: &str) -> Result<url::Url>` (joins a `/`-rooted path onto `base_url`).

- [ ] **Step 1: Write the failing tests**

`crates/lens-client/src/connection.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ConnectionId;

    #[test]
    fn url_joins_rooted_path() {
        let c = Connection::new(
            ConnectionId::new("c1"),
            "http://localhost:8000".parse().unwrap(),
            Auth::None,
        );
        let u = c.url("/v1/sessions").unwrap();
        assert_eq!(u.as_str(), "http://localhost:8000/v1/sessions");
    }

    #[test]
    fn bearer_auth_sets_authorization_header() {
        // We assert the RequestBuilder carries the header by building a request.
        let client = reqwest::blocking::Client::new();
        let rb = client.get("http://localhost:8000/health");
        let rb = Auth::Bearer { token: "tok123".into() }.apply(rb);
        let req = rb.build().unwrap();
        assert_eq!(
            req.headers().get("authorization").unwrap(),
            "Bearer tok123"
        );
    }

    #[test]
    fn forwarded_email_sets_header() {
        let client = reqwest::blocking::Client::new();
        let rb = client.get("http://localhost:8000/health");
        let rb = Auth::ForwardedEmail { email: "a@b.com".into() }.apply(rb);
        let req = rb.build().unwrap();
        assert_eq!(req.headers().get("x-forwarded-email").unwrap(), "a@b.com");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p lens-client connection::`
Expected: FAIL — `cannot find type Connection`.

- [ ] **Step 3: Implement connection + auth**

Prepend to `crates/lens-client/src/connection.rs`:
```rust
use crate::error::{ClientError, Result};
use crate::ids::ConnectionId;
use reqwest::blocking::RequestBuilder;

#[derive(Clone, Debug)]
pub enum Auth {
    None,
    Bearer { token: String },
    Cookie { value: String },
    ForwardedEmail { email: String },
}

impl Auth {
    pub fn apply(&self, rb: RequestBuilder) -> RequestBuilder {
        match self {
            Auth::None => rb,
            Auth::Bearer { token } => rb.bearer_auth(token),
            Auth::Cookie { value } => rb.header(reqwest::header::COOKIE, value),
            Auth::ForwardedEmail { email } => rb.header("X-Forwarded-Email", email),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Connection {
    pub id: ConnectionId,
    pub base_url: url::Url,
    pub auth: Auth,
}

impl Connection {
    pub fn new(id: ConnectionId, base_url: url::Url, auth: Auth) -> Self {
        Self { id, base_url, auth }
    }

    /// Join a `/`-rooted absolute path onto the connection's base URL.
    pub fn url(&self, path: &str) -> Result<url::Url> {
        self.base_url
            .join(path)
            .map_err(|e| ClientError::NotFound { what: format!("bad url {path}: {e}") })
    }
}
```

- [ ] **Step 4: Register the module**

In `crates/lens-client/src/lib.rs`, add:
```rust
pub mod connection;
```

- [ ] **Step 5: Run tests + clippy**

Run: `cargo test -p lens-client connection:: && cargo clippy -p lens-client -- -D warnings`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/lens-client/src/connection.rs crates/lens-client/src/lib.rs
git commit -m "feat(lens-client): connection + auth injection"
```

---

### Task 4: `xtask codegen` — typify scaffold

**Files:**
- Create: `crates/xtask/Cargo.toml`
- Create: `crates/xtask/src/main.rs`
- Create (generated, committed): `crates/lens-client/src/generated.rs`
- Modify: `crates/lens-client/src/lib.rs` (add `pub mod generated;`)

**Interfaces:**
- Produces: a binary runnable via `cargo run -p xtask -- codegen` that reads `vendor/omnigent-0.3.0.dev0/openapi.json`, extracts `components/schemas`, runs `typify`, and writes `crates/lens-client/src/generated.rs`. The generated module exposes serde structs/enums for every schema (exact names follow the spec's schema keys, e.g. `SessionObject`, `ResponseObject`, `SessionUsage`).

- [ ] **Step 1: Create the xtask manifest**

`crates/xtask/Cargo.toml`:
```toml
[package]
name = "xtask"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
authors.workspace = true

# xtask is tooling, not shipped — it does not opt into workspace production lints.

[dependencies]
typify = "0.4"
serde_json = "1"
schemars = "0.8"
prettyplease = "0.2"
syn = "2"
quote = "1"
anyhow = "1"
```

- [ ] **Step 2: Write the codegen xtask**

`crates/xtask/src/main.rs`:
```rust
use anyhow::{bail, Context, Result};
use std::path::PathBuf;

const SPEC: &str = "vendor/omnigent-0.3.0.dev0/openapi.json";
const OUT: &str = "crates/lens-client/src/generated.rs";

fn main() -> Result<()> {
    let cmd = std::env::args().nth(1).unwrap_or_default();
    match cmd.as_str() {
        "codegen" => codegen(),
        other => bail!("unknown xtask command: {other:?} (expected: codegen)"),
    }
}

fn codegen() -> Result<()> {
    let root = workspace_root()?;
    let spec_path = root.join(SPEC);
    let raw = std::fs::read_to_string(&spec_path)
        .with_context(|| format!("read {}", spec_path.display()))?;
    let doc: serde_json::Value = serde_json::from_str(&raw)?;

    let schemas = doc
        .get("components")
        .and_then(|c| c.get("schemas"))
        .and_then(|s| s.as_object())
        .context("openapi.json has no components.schemas")?;

    let mut settings = typify::TypeSpaceSettings::default();
    settings.with_derive("PartialEq".into());
    let mut type_space = typify::TypeSpace::new(&settings);

    for (name, schema) in schemas {
        let schema: schemars::schema::Schema = serde_json::from_value(schema.clone())
            .with_context(|| format!("schema {name} is not valid JSON Schema"))?;
        type_space
            .add_type_with_name(&schema, Some(name.clone()))
            .with_context(|| format!("typify failed on schema {name}"))?;
    }

    let tokens = type_space.to_stream();
    let file: syn::File = syn::parse2(tokens).context("parse generated tokens")?;
    let pretty = prettyplease::unparse(&file);

    let header = "// @generated by `cargo run -p xtask -- codegen` from \
vendor/omnigent-0.3.0.dev0/openapi.json — DO NOT EDIT BY HAND.\n\
#![allow(clippy::all)]\n\n";
    std::fs::write(root.join(OUT), format!("{header}{pretty}"))
        .with_context(|| format!("write {OUT}"))?;

    println!("wrote {OUT} ({} schemas)", schemas.len());
    Ok(())
}

fn workspace_root() -> Result<PathBuf> {
    // xtask is invoked from the workspace root via `cargo run -p xtask`.
    Ok(std::env::current_dir()?)
}
```

- [ ] **Step 3: Run the generator**

Run: `cargo run -p xtask -- codegen`
Expected: prints `wrote crates/lens-client/src/generated.rs (<N> schemas)` and the file exists.

> If `typify` rejects a specific schema (3.2-isms, unusual `$ref`s), note the failing schema name in the task report and skip it with a `// SKIPPED: <name> — <reason>` comment in a `SKIPPED.md` next to the spec; do **not** hand-edit `generated.rs`. This is the D1 "validate typify maps cleanly" risk — surface it, don't paper over it.

- [ ] **Step 4: Register the generated module + verify it compiles**

In `crates/lens-client/src/lib.rs`, add:
```rust
pub mod generated;
```

Run: `cargo build -p lens-client`
Expected: compiles. (Generated code is `#![allow(clippy::all)]`d.)

- [ ] **Step 5: Commit**

```bash
git add crates/xtask/Cargo.toml crates/xtask/src/main.rs crates/lens-client/src/generated.rs crates/lens-client/src/lib.rs
git commit -m "feat(xtask): typify codegen + committed generated wire types"
```

---

### Task 5: HTTP core + contract-gate logic (unit-tested, no server)

**Files:**
- Create: `crates/lens-client/src/http.rs`
- Create: `crates/lens-client/src/info.rs`
- Modify: `crates/lens-client/src/lib.rs`

**Interfaces:**
- Consumes: `connection::Connection`, `error::{ClientError, Result}`, `PINNED_OMNIGENT_VERSION`.
- Produces:
  - `info::VersionResponse { version: String }` and `info::ServerInfo { accounts_enabled: bool, login_url: Option<String>, needs_setup: bool, databricks_features: serde_json::Value, managed_sandboxes_enabled: bool, sandbox_provider: Option<String> }` (serde `Deserialize`).
  - `http::check_contract(expected: &str, actual: &str) -> Result<()>` — `Ok(())` on exact match, else `ClientError::ContractMismatch`.

- [ ] **Step 1: Write the failing test for the gate logic**

`crates/lens-client/src/http.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract_gate_accepts_exact_match() {
        assert!(check_contract("0.3.0.dev0", "0.3.0.dev0").is_ok());
    }

    #[test]
    fn contract_gate_rejects_mismatch() {
        let err = check_contract("0.3.0.dev0", "0.2.0").unwrap_err();
        match err {
            crate::error::ClientError::ContractMismatch { expected, actual } => {
                assert_eq!(expected, "0.3.0.dev0");
                assert_eq!(actual, "0.2.0");
            }
            other => panic!("wrong error: {other:?}"),
        }
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p lens-client http::`
Expected: FAIL — `cannot find function check_contract`.

- [ ] **Step 3: Implement the gate logic**

Prepend to `crates/lens-client/src/http.rs`:
```rust
use crate::error::{ClientError, Result};

/// Exact-match contract gate. Coarse on dev0 (the version string is identical
/// across commits — see typed-client-implementation.md D4); real drift
/// detection is the startup taxonomy diff + `xtask drift`, planned later.
pub fn check_contract(expected: &'static str, actual: &str) -> Result<()> {
    if expected == actual {
        Ok(())
    } else {
        Err(ClientError::ContractMismatch { expected, actual: actual.to_string() })
    }
}
```

- [ ] **Step 4: Implement the info response types**

`crates/lens-client/src/info.rs`:
```rust
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct VersionResponse {
    pub version: String,
}

/// `GET /v1/info` — unauthenticated capability/auth probe (no version field).
#[derive(Debug, Clone, Deserialize)]
pub struct ServerInfo {
    #[serde(default)]
    pub accounts_enabled: bool,
    #[serde(default)]
    pub login_url: Option<String>,
    #[serde(default)]
    pub needs_setup: bool,
    #[serde(default)]
    pub databricks_features: serde_json::Value,
    #[serde(default)]
    pub managed_sandboxes_enabled: bool,
    #[serde(default)]
    pub sandbox_provider: Option<String>,
}
```

- [ ] **Step 5: Register modules**

In `crates/lens-client/src/lib.rs`, add:
```rust
pub mod http;
pub mod info;
```

- [ ] **Step 6: Run tests + clippy**

Run: `cargo test -p lens-client && cargo clippy -p lens-client -- -D warnings`
Expected: PASS, no warnings.

- [ ] **Step 7: Commit**

```bash
git add crates/lens-client/src/http.rs crates/lens-client/src/info.rs crates/lens-client/src/lib.rs
git commit -m "feat(lens-client): contract-gate logic + info response types"
```

---

### Task 6: `Client::new` handshake (live, gated) + ready ladder

**Files:**
- Create: `crates/lens-client/src/client.rs`
- Modify: `crates/lens-client/src/lib.rs`
- Create: `crates/lens-client/tests/live_handshake.rs`

**Interfaces:**
- Consumes: `connection::Connection`, `info::{VersionResponse, ServerInfo}`, `http::check_contract`, `PINNED_OMNIGENT_VERSION`.
- Produces:
  - `client::Client { conn: Connection, http: reqwest::blocking::Client, info: ServerInfo }`.
  - `client::Client::new(conn: Connection) -> Result<Client>` — runs the ready ladder: `GET /health` → `GET /api/version` (gate vs `PINNED_OMNIGENT_VERSION`) → `GET /v1/info` (captured into `info`).
  - `client::Client::server_info(&self) -> &ServerInfo`.

- [ ] **Step 1: Implement the client + handshake**

`crates/lens-client/src/client.rs`:
```rust
use crate::connection::Connection;
use crate::error::{ClientError, Result};
use crate::http::check_contract;
use crate::info::{ServerInfo, VersionResponse};
use crate::PINNED_OMNIGENT_VERSION;

pub struct Client {
    conn: Connection,
    http: reqwest::blocking::Client,
    info: ServerInfo,
}

impl Client {
    /// Handshake + contract gate. Ready ladder: /health → /api/version → /v1/info.
    pub fn new(conn: Connection) -> Result<Self> {
        let http = reqwest::blocking::Client::builder()
            .build()
            .map_err(ClientError::Network)?;

        // 1. liveness
        let health = conn.auth.apply(http.get(conn.url("/health")?)).send()?;
        if !health.status().is_success() {
            return Err(ClientError::Server {
                status: health.status().as_u16(),
                body: serde_json::json!({ "stage": "health" }),
            });
        }

        // 2. contract gate
        let ver: VersionResponse = conn
            .auth
            .apply(http.get(conn.url("/api/version")?))
            .send()?
            .json()?;
        check_contract(PINNED_OMNIGENT_VERSION, &ver.version)?;

        // 3. capabilities
        let info: ServerInfo = conn
            .auth
            .apply(http.get(conn.url("/v1/info")?))
            .send()?
            .json()?;

        Ok(Self { conn, http, info })
    }

    pub fn server_info(&self) -> &ServerInfo {
        &self.info
    }

    /// Internal accessors for later REST modules.
    pub(crate) fn conn(&self) -> &Connection { &self.conn }
    pub(crate) fn http(&self) -> &reqwest::blocking::Client { &self.http }
}
```

- [ ] **Step 2: Register module + re-export**

In `crates/lens-client/src/lib.rs`, add:
```rust
pub mod client;
pub use client::Client;
pub use connection::{Auth, Connection};
```

- [ ] **Step 3: Verify it compiles (no server needed)**

Run: `cargo build -p lens-client && cargo clippy -p lens-client -- -D warnings`
Expected: compiles, no warnings.

- [ ] **Step 4: Write the gated live handshake test**

`crates/lens-client/tests/live_handshake.rs`:
```rust
//! Live test — requires a running omnigent server at $LENS_OMNIGENT_URL.
//! Run with: `LENS_OMNIGENT_URL=http://localhost:<port> cargo test -p lens-client --features live-tests --test live_handshake`
#![cfg(feature = "live-tests")]

use lens_client::{Auth, Connection};
use lens_client::ids::ConnectionId;

fn base_url() -> url::Url {
    std::env::var("LENS_OMNIGENT_URL")
        .expect("set LENS_OMNIGENT_URL to the running server (omnigent server status)")
        .parse()
        .expect("LENS_OMNIGENT_URL is not a valid URL")
}

#[test]
fn handshake_succeeds_against_pinned_server() {
    let conn = Connection::new(ConnectionId::new("live"), base_url(), Auth::None);
    let client = lens_client::Client::new(conn).expect("handshake should pass the contract gate");
    // /v1/info is reachable and parsed; accounts_enabled is a real bool either way.
    let _ = client.server_info().accounts_enabled;
}
```

Add `url` to dev-deps in `crates/lens-client/Cargo.toml`:
```toml
[dev-dependencies]
url = "2"
```

- [ ] **Step 5: Run the default suite (no server) to confirm the gate is dormant**

Run: `cargo test -p lens-client`
Expected: PASS — the live test is compiled out (no `live-tests` feature).

- [ ] **Step 6: Run the live test against the pinned server**

Per `installing-omnigent-from-source`, ensure the daemon is up, then:
Run: `omnigent server status` (note the port), then
`LENS_OMNIGENT_URL=http://localhost:<port> cargo test -p lens-client --features live-tests --test live_handshake -- --nocapture`
Expected: PASS — `handshake_succeeds_against_pinned_server`.

- [ ] **Step 7: Commit**

```bash
git add crates/lens-client/src/client.rs crates/lens-client/src/lib.rs crates/lens-client/Cargo.toml crates/lens-client/tests/live_handshake.rs
git commit -m "feat(lens-client): Client::new handshake + contract gate (gated live test)"
```

---

## Self-review

- **Spec coverage (Units 1–3 of `typed-client-implementation.md` §4):** scaffold/error/ids/connection (Tasks 1–3) ✓; codegen via typify one-shot committed (Task 4, D1) ✓; HTTP core + contract gate + ready ladder + ServerInfo capture (Tasks 5–6, §8) ✓; D2 sync/blocking honored (no async anywhere) ✓; D3 default tests serverless + live gated (Task 6) ✓; D4 coarse-gate comment recorded in code (Task 5) ✓.
- **Out of scope here (next plan, post-codegen):** REST methods (§3), `SessionEventInput` write path (§6), SSE taxonomy + reader thread (§4), reconnect (§7), WS terminal (§5), `xtask drift`/`live-test`, golden-SSE captures (§9). These need `generated.rs` to exist first — that's why they're phased.
- **Type consistency:** `ClientError` variants used in Tasks 5–6 all defined in Task 1; `check_contract` signature matches between Task 5 def and Task 6 use; `Connection::url`/`Auth::apply` signatures consistent between Tasks 3 and 6; branded `ConnectionId` from Task 2 used in Tasks 3/6.
- **Risk flagged, not hidden:** Task 4 Step 3 surfaces typify-incompatible schemas explicitly rather than silently editing generated output.

## Subsequent phases (to plan after this lands)

| Plan | Units | Delivers | Gated on |
|---|---|---|---|
| 2 — REST surface | 4 (5a–5e) | typed methods for all of §3 incl. `SessionEventInput` write path | `generated.rs` from Task 4 |
| 3 — Streaming | 5–6 | SSE parser + `ServerStreamEvent` taxonomy + blocking reader thread + reconnect (three-bucket) | Plan 2 (snapshot/items for reconnect) |
| 4 — Terminal + verification | 7–8 | WS terminal attach; `xtask drift`/`live-test`; golden-SSE contract tests | Plan 3 |
