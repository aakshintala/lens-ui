# State-Model Engine P0 — Domain Types Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stand up the new `lens-core` crate and its `domain/` module — the full LOCKED §2 domain type set (branded ids, `SessionState`, `Item`/`ItemKind`, `BlockContext`, `Usage`/`Cost`, `StreamScratch`, and their supporting value types) as pure data + serde, no logic.

**Architecture:** `lens-core` is a new gpui-free crate depending on `lens-client`. P0 defines only the framework-neutral domain layer under `src/domain/`. The reducer (P1), persistence (P2), and actor/store (P3) build on these types in later phases/specs. The reuse boundary is deliberate: **reuse `lens-client`'s branded ids** (public newtypes with full serde) and its typed wire `SessionResourceObject`; **domain-own every other value/aggregate type**, because `lens-client`'s read wrappers (`TodoItem`, `PresenceViewer`, `SessionStatusValue`, …) are deserialize-only with private fields and no `Serialize` — unusable as a mutable, persistable view-model.

**Tech Stack:** Rust (edition 2024, workspace `rust-version = 1.91`), `serde` (derive) + `serde_json` (for `Value` payload fields and round-trip tests), `lens-client` (path dep).

## Global Constraints

- **Design source of truth:** `docs/design/app-architecture-and-state-model.md` §2 (LOCKED) + spec `docs/specs/2026-07-08-state-model-engine-design.md` §4 "P0".
- **`lens-core` has NO gpui dependency** — framework touch-points live in the future `lens-store` crate (spec D1/§3).
- **No logic in P0** — pure data + serde only. No threads, no SQLite, no reducer.
- **Production lint bar:** the crate opts into `lints.workspace = true` (workspace denies `unsafe_code`, `unused_must_use`, clippy `all`). Zero warnings.
- **`generated.rs` in `lens-client` is untouched** (codegen artifact).
- **Gate every task:** `cargo test -p lens-core` · `cargo clippy -p lens-core --all-targets` (zero warnings) · `cargo fmt --check`.
- **Grilling revisions honored** (memory `state-model-grilling-revisions`):
  - `BlockContext` is **pure attribution** `{ agent, depth, turn }` — the old `timestamp: f64` monotonic field is **dropped**.
  - The durable "when" is `Item.created_at: i64` (epoch **millis**) on the item **envelope**.
  - `SessionState.created_at: i64` is epoch **seconds** (distinct unit — keep the comments).
- **`presence` is RAM-only** — carried on `SessionState` in P0, but flagged for exclusion from the P2 persisted schema (not enforced in P0; P0 has no persistence).
- **Serde round-trip is the P0 gate** — every type derives `Serialize + Deserialize` and has a round-trip test. Churn-safe enums (status vocabularies) carry a `#[serde(other)] Unknown` variant.

---

## File Structure

```
crates/lens-core/
  Cargo.toml                 # NEW — deps: lens-client (path), serde, serde_json
  src/
    lib.rs                   # NEW — `pub mod domain;` + top-level re-exports
    domain/
      mod.rs                 # NEW — declares + re-exports submodules
      ids.rs                 # NEW — local branded_id! for 4 new ids; re-export lens-client's 9
      scalars.rs             # NEW — Role, ErrorSource, HostType, SessionLifecycle,
                             #        SessionStatusValue, ErrorInfo
      usage.rs               # NEW — Usage, ModelUsage, Cost, PresenceViewer
      controls.rs            # NEW — Todo, TodoStatus, SkillSummary, ModelOption,
                             #        SandboxStatus, Elicitation, ElicitationParams,
                             #        PendingUserMessage
      item.rs                # NEW — BlockContext, ContentBlock, Item, ItemKind, StreamScratch,
                             #        MessageAcc, ReasoningAcc
      session.rs             # NEW — SessionState (the aggregate)
```

Root workspace `Cargo.toml` already globs `members = ["crates/*"]`, so the crate is picked up automatically — no members edit needed.

---

## Type Inventory (the complete P0 surface)

Every type below is `#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]` unless noted. Ids additionally derive `Eq, Hash`.

| Type | Module | Notes |
|---|---|---|
| `ItemId`, `CallId`, `ResponseId`, `AgentId` | `ids` | NEW branded ids (local macro) |
| (re-export) `ConnectionId`, `SessionId`, `HostId`, `RunnerId`, `TerminalId`, `FileId`, `CommentId`, `PolicyId`, `ElicitationId` | `ids` | from `lens_client::ids` |
| `Role` | `scalars` | `User | Assistant` |
| `ErrorSource` | `scalars` | `Server | Client | Runner | Unknown` (`#[serde(other)] Unknown`) |
| `HostType` | `scalars` | `External | Managed` |
| `SessionLifecycle` | `scalars` | `Active | Slept | Deleted` |
| `SessionStatusValue` | `scalars` | `Idle | Launching | Running | Waiting | Failed | Unknown` (`#[serde(other)]`) |
| `ErrorInfo` | `scalars` | `{ code, message }` |
| `Usage` | `usage` | full token counts + `usage_by_model: BTreeMap<String, ModelUsage>` |
| `ModelUsage` | `usage` | per-model rollup |
| `Cost` | `usage` | `{ cumulative_usage: Usage, total_cost_usd: Option<f64> }` |
| `PresenceViewer` | `usage` | wire-faithful `{ user_id, joined_at, idle }` |
| `Todo`, `TodoStatus` | `controls` | `TodoStatus`: `Pending | InProgress | Completed | Unknown` |
| `SkillSummary` | `controls` | `{ name, description }` |
| `ModelOption` | `controls` | `{ id, label }` |
| `SandboxStatus` | `controls` | `{ stage, detail: Option<String> }` |
| `Elicitation`, `ElicitationParams` | `controls` | `Elicitation` carries `target_session_id` |
| `PendingUserMessage` | `controls` | optimistic bubble `{ pending_id, content, created_at }` |
| `BlockContext` | `item` | `{ agent, depth, turn }` — pure attribution |
| `ContentBlock` | `item` | `{ kind: String, text: Option<String>, data: Value }` |
| `Item` | `item` | envelope `{ id, seq, ctx, created_at, kind }` |
| `ItemKind` | `item` | the full 11-variant union (§2.3) |
| `StreamScratch`, `MessageAcc`, `ReasoningAcc` | `item` | RAM-only accumulators |
| `SessionState` | `session` | the aggregate view-model |

---

## Task 1: Crate skeleton + branded ids

**Files:**
- Create: `crates/lens-core/Cargo.toml`
- Create: `crates/lens-core/src/lib.rs`
- Create: `crates/lens-core/src/domain/mod.rs`
- Create: `crates/lens-core/src/domain/ids.rs`

**Interfaces:**
- Produces: `lens_core::domain::ids::{ItemId, CallId, ResponseId, AgentId}` (NEW) and re-exports `ConnectionId, SessionId, HostId, RunnerId, TerminalId, FileId, CommentId, PolicyId, ElicitationId` (from `lens_client::ids`). All are `#[serde(transparent)]` string newtypes with `new(impl Into<String>)`, `as_str()`, `Display`.

- [ ] **Step 1: Create the crate manifest**

`crates/lens-core/Cargo.toml`:

```toml
[package]
name = "lens-core"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
authors.workspace = true

[lints]
workspace = true

[dependencies]
lens-client = { path = "../lens-client" }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

> Note: match the exact `serde`/`serde_json` version syntax already used in `crates/lens-client/Cargo.toml`. If that manifest pins a specific version or uses `workspace = true` for these deps, mirror it verbatim (check before writing).

- [ ] **Step 2: Create `lib.rs`**

`crates/lens-core/src/lib.rs`:

```rust
//! `lens-core` — the framework-neutral state-model engine for one
//! `(connection, session)`. P0 defines the domain types (§2); later phases add
//! the reducer (§4), persistence (§6), and the actor (§8).

pub mod domain;
```

- [ ] **Step 3: Create `domain/mod.rs`**

`crates/lens-core/src/domain/mod.rs`:

```rust
//! §2 domain model — pure data + serde, no logic.

pub mod ids;
pub mod scalars;
pub mod usage;
pub mod controls;
pub mod item;
pub mod session;

pub use ids::*;
pub use scalars::*;
pub use usage::*;
pub use controls::*;
pub use item::*;
pub use session::*;
```

> The other submodules don't exist yet — this file will not compile until their tasks land. To keep Task 1 independently green, temporarily declare only `pub mod ids; pub use ids::*;` and add the rest as each task lands. (State this in the task; the final module list above is the end state.)

- [ ] **Step 4: Write the failing test for ids**

`crates/lens-core/src/domain/ids.rs`:

```rust
//! Branded ids. Reuses `lens-client`'s 9 ids (public newtypes, full serde) and
//! adds the 4 engine-local ones. `lens-client`'s `branded_id!` macro is not
//! exported, so we define a local one — trivial and keeps the crates decoupled.

use serde::{Deserialize, Serialize};

// Re-export the ids that already live in lens-client (§2.1 reuse boundary).
pub use lens_client::ids::{
    CommentId, ConnectionId, ElicitationId, FileId, HostId, PolicyId, RunnerId, SessionId,
    TerminalId,
};

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

// Engine-local ids not present in lens-client (§2.1). BridgeItemId is Bridge
// scope (§11) — out of this spec.
branded_id!(ItemId, CallId, ResponseId, AgentId);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_id_roundtrips_json_and_display() {
        let id = ItemId::new("item_abc");
        assert_eq!(id.as_str(), "item_abc");
        assert_eq!(id.to_string(), "item_abc");
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"item_abc\"");
        let back: ItemId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, id);
    }

    #[test]
    fn reexported_id_is_usable() {
        // Proves the re-export path compiles and the type is constructible here.
        let s = SessionId::new("conv_1");
        let json = serde_json::to_string(&s).unwrap();
        let back: SessionId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }
}
```

> Verify the `lens_client::ids::{…}` path is correct: `ids` is `pub mod ids;` in `lens-client/src/lib.rs`, so `lens_client::ids::SessionId` resolves. If any id name differs, fix the import list against `crates/lens-client/src/ids.rs`.

- [ ] **Step 5: Run the tests to verify they fail (compile error / undefined)**

Run: `cargo test -p lens-core`
Expected: builds the new crate; both tests pass. (If the `lens_client::ids` re-export path is wrong, this fails to compile — fix the path.)

- [ ] **Step 6: Verify gate**

Run: `cargo clippy -p lens-core --all-targets` (zero warnings) and `cargo fmt --check`.

- [ ] **Step 7: Commit**

```bash
git add crates/lens-core/Cargo.toml crates/lens-core/src/lib.rs crates/lens-core/src/domain/mod.rs crates/lens-core/src/domain/ids.rs
git commit -m "feat(lens-core): P0 crate skeleton + branded ids"
```

---

## Task 2: Scalar enums + `ErrorInfo`

**Files:**
- Create: `crates/lens-core/src/domain/scalars.rs`
- Modify: `crates/lens-core/src/domain/mod.rs` (add `pub mod scalars; pub use scalars::*;`)

**Interfaces:**
- Produces: `Role`, `ErrorSource`, `HostType`, `SessionLifecycle`, `SessionStatusValue`, `ErrorInfo`.

- [ ] **Step 1: Write the failing test + types**

`crates/lens-core/src/domain/scalars.rs`:

```rust
//! Scalar enums and small leaf structs (§2.2/§2.3/§2.5).

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

/// Origin of a persisted `Error` item (§2.3 `ItemKind::Error`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ErrorSource {
    Server,
    Client,
    Runner,
    /// Any source literal this version does not know.
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HostType {
    External,
    Managed,
}

/// Lens-local lifecycle (§2.2). Distinct from the server `archived` flag.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionLifecycle {
    Active,
    Slept,
    Deleted,
}

/// Canonical fine-grained status (§2.2). The full 5-state set is only observable
/// from SSE (`SessionStatusEvent`); the REST poll is coarse 3-state and is
/// normalized into this by the reducer (P1). `Unknown` covers dev0 churn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatusValue {
    Idle,
    Launching,
    Running,
    Waiting,
    Failed,
    #[serde(other)]
    Unknown,
}

/// Present iff `SessionState.status == Failed` (§2.5).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorInfo {
    pub code: String,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_serializes_lowercase_and_roundtrips() {
        assert_eq!(
            serde_json::to_string(&SessionStatusValue::Waiting).unwrap(),
            "\"waiting\""
        );
        let back: SessionStatusValue = serde_json::from_str("\"launching\"").unwrap();
        assert_eq!(back, SessionStatusValue::Launching);
    }

    #[test]
    fn status_unknown_literal_maps_to_unknown_variant() {
        let back: SessionStatusValue = serde_json::from_str("\"superseded\"").unwrap();
        assert_eq!(back, SessionStatusValue::Unknown);
    }

    #[test]
    fn error_source_unknown_is_churn_safe() {
        let back: ErrorSource = serde_json::from_str("\"gateway\"").unwrap();
        assert_eq!(back, ErrorSource::Unknown);
    }

    #[test]
    fn error_info_roundtrips() {
        let e = ErrorInfo { code: "rate_limited".into(), message: "slow down".into() };
        let json = serde_json::to_string(&e).unwrap();
        let back: ErrorInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn role_and_hosttype_and_lifecycle_roundtrip() {
        for r in [Role::User, Role::Assistant] {
            let back: Role = serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
            assert_eq!(back, r);
        }
        let back: HostType =
            serde_json::from_str(&serde_json::to_string(&HostType::Managed).unwrap()).unwrap();
        assert_eq!(back, HostType::Managed);
        let back: SessionLifecycle =
            serde_json::from_str(&serde_json::to_string(&SessionLifecycle::Slept).unwrap()).unwrap();
        assert_eq!(back, SessionLifecycle::Slept);
    }
}
```

- [ ] **Step 2: Register the module** — add to `crates/lens-core/src/domain/mod.rs`:

```rust
pub mod scalars;
pub use scalars::*;
```

- [ ] **Step 3: Run tests** — `cargo test -p lens-core`. Expected: PASS.

- [ ] **Step 4: Gate** — `cargo clippy -p lens-core --all-targets` + `cargo fmt --check`.

- [ ] **Step 5: Commit**

```bash
git add crates/lens-core/src/domain/scalars.rs crates/lens-core/src/domain/mod.rs
git commit -m "feat(lens-core): P0 scalar enums + ErrorInfo"
```

---

## Task 3: `Usage`, `ModelUsage`, `Cost`, `PresenceViewer`

**Files:**
- Create: `crates/lens-core/src/domain/usage.rs`
- Modify: `crates/lens-core/src/domain/mod.rs`

**Interfaces:**
- Produces: `Usage`, `ModelUsage`, `Cost`, `PresenceViewer`.
- Consumes: nothing from prior tasks.

- [ ] **Step 1: Write the types + test**

`crates/lens-core/src/domain/usage.rs`:

```rust
//! Token/cost accounting and presence (§2.5).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Per-model rollup (§2.5). Field set mirrors the pinned wire contract
/// (`lens_client::generated::ModelUsage`, generated.rs:3467) so no server data is
/// lost before P1/P2: cache buckets + per-model `total_cost_usd`. All optional —
/// "priced ⟺ key present" (a `None` cost means the model was unpriced), and the
/// harness may omit any token bucket.
///
/// NOT `Eq` — it holds `f64` (`total_cost_usd`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ModelUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub cache_creation_input_tokens: Option<u64>,
    pub cache_read_input_tokens: Option<u64>,
    pub total_cost_usd: Option<f64>,
}

/// NOT `Eq` — transitively holds `ModelUsage`'s `f64` via `usage_by_model`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub reasoning_tokens: Option<u64>,
    pub context_tokens: Option<u64>,
    /// Per-model rollup (0.2.0). Empty when the server reports no breakdown.
    pub usage_by_model: BTreeMap<String, ModelUsage>,
}

/// Accumulated client-side from `session.usage` events; the USD figure is
/// SERVER-computed (`total_cost_usd`) — Lens keeps no price table (§2.5).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Cost {
    pub cumulative_usage: Usage,
    pub total_cost_usd: Option<f64>,
}

/// `session.presence` — wire-faithful shape (§2.5). Transient/RAM-only; never
/// persisted. Carries NO display_name/is_owner/last_seen_at (those are derived
/// separately and joined by user_id).
///
/// P1 HANDOFF NOTE: the current `lens_client::stream::PresenceViewer` wrapper
/// exposes ONLY `user_id` (event.rs:146) — it drops `joined_at`/`idle` that the
/// generated contract (generated.rs:4220) carries. P1 cannot populate this domain
/// type from `ServerStreamEvent::Presence` until lens-client's stream wrapper is
/// widened (or P1 reads the generated type). Flagged for the P1 spec — NOT a P0 fix.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresenceViewer {
    pub user_id: String,
    /// ISO 8601 UTC; stable across reconnect within the leave-grace window.
    pub joined_at: String,
    pub idle: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_default_is_zeroed_and_roundtrips() {
        let u = Usage::default();
        assert_eq!(u.total_tokens, 0);
        assert!(u.usage_by_model.is_empty());
        let back: Usage = serde_json::from_str(&serde_json::to_string(&u).unwrap()).unwrap();
        assert_eq!(back, u);
    }

    #[test]
    fn usage_with_per_model_rollup_roundtrips() {
        let mut by_model = BTreeMap::new();
        by_model.insert(
            "claude-opus".to_string(),
            ModelUsage {
                input_tokens: Some(10),
                output_tokens: Some(20),
                total_tokens: Some(30),
                cache_creation_input_tokens: Some(2),
                cache_read_input_tokens: Some(8),
                total_cost_usd: Some(0.42),
            },
        );
        let u = Usage {
            input_tokens: 10,
            output_tokens: 20,
            total_tokens: 30,
            reasoning_tokens: Some(5),
            context_tokens: None,
            usage_by_model: by_model,
        };
        let back: Usage = serde_json::from_str(&serde_json::to_string(&u).unwrap()).unwrap();
        assert_eq!(back, u);
    }

    #[test]
    fn cost_roundtrips_with_and_without_usd() {
        let c = Cost { cumulative_usage: Usage::default(), total_cost_usd: Some(1.25) };
        let back: Cost = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        assert_eq!(back, c);
        let c0 = Cost::default();
        assert_eq!(c0.total_cost_usd, None);
    }

    #[test]
    fn presence_viewer_roundtrips() {
        let p = PresenceViewer {
            user_id: "u_1".into(),
            joined_at: "2026-07-08T00:00:00Z".into(),
            idle: false,
        };
        let back: PresenceViewer =
            serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(back, p);
    }
}
```

> `Cost` derives `PartialEq` but not `Eq` (`f64` has no `Eq`). Do NOT add `Eq` to `Cost` or `Usage`-containing aggregates that reach `Cost`.

- [ ] **Step 2: Register the module** in `mod.rs` (`pub mod usage; pub use usage::*;`).

- [ ] **Step 3: Run tests** — `cargo test -p lens-core`. Expected: PASS.

- [ ] **Step 4: Gate** — clippy + fmt.

- [ ] **Step 5: Commit**

```bash
git add crates/lens-core/src/domain/usage.rs crates/lens-core/src/domain/mod.rs
git commit -m "feat(lens-core): P0 Usage/ModelUsage/Cost/PresenceViewer"
```

---

## Task 4: Session sub-types (`controls.rs`)

**Files:**
- Create: `crates/lens-core/src/domain/controls.rs`
- Modify: `crates/lens-core/src/domain/mod.rs`

**Interfaces:**
- Consumes: `ElicitationId`, `SessionId` (from `ids`).
- Produces: `Todo`, `TodoStatus`, `SkillSummary`, `ModelOption`, `SandboxStatus`, `Elicitation`, `ElicitationParams`, `PendingUserMessage`.

> These are the "mirrored here but owned by their surface documents" types (§2.2). P0 defines minimal domain-owned shapes sufficient to carry state and round-trip; their rendering/action semantics belong to other docs and are grown when a consumer exists (YAGNI). Shapes are adapted from `lens-client`'s wire wrappers (`TodoItem`, `ElicitationParams`, `SkillRef`) — which are deserialize-only and cannot be reused directly.

- [ ] **Step 1: Write the types + test**

`crates/lens-core/src/domain/controls.rs`:

```rust
//! Session control/chrome sub-types (§2.2). Domain-owned mirrors of wire
//! wrappers; rendering/actions belong to their surface documents.

use crate::domain::ids::{ElicitationId, SessionId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    Pending,
    InProgress,
    Completed,
    #[serde(other)]
    Unknown,
}

/// The agent's live todos — rendered inline in chat (§2.2).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Todo {
    pub content: String,
    pub status: TodoStatus,
    pub active_form: String,
}

/// 0.2.0 chrome (§2.2).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillSummary {
    pub name: String,
    pub description: Option<String>,
}

/// Drives the model picker (§2.2 `model_options`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelOption {
    pub id: String,
    pub label: String,
}

/// Managed-sandbox launch progress (§2.2). `detail` set when `stage == "failed"`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxStatus {
    pub stage: String,
    pub detail: Option<String>,
}

/// Elicitation request parameters (mirrors `lens_client::stream::ElicitationParams`,
/// which is deserialize-only). Grown by the §4.3 elicitation surface.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ElicitationParams {
    pub mode: String,
    pub message: String,
    pub url: Option<String>,
    pub phase: Option<String>,
    pub policy_name: Option<String>,
    pub content_preview: Option<String>,
}

/// A pending elicitation prompt (§2.2, PLURAL). Carries `target_session_id` for
/// resolve routing (fan-out parents mirror multiple child prompts).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Elicitation {
    pub id: ElicitationId,
    pub target_session_id: SessionId,
    pub params: ElicitationParams,
}

/// Optimistic, pre-`consumed` user message (§7). RAM-only intent; carried on
/// `SessionState.pending_user`. `pending_id` is Lens-local until/unless the
/// server returns one (P3 live-verify item).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingUserMessage {
    pub pending_id: String,
    pub content: String,
    /// Epoch millis, injected-clock-stamped when the send is issued (P3).
    pub created_at: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn todo_roundtrips_and_unknown_status_is_churn_safe() {
        let t = Todo {
            content: "wire the reducer".into(),
            status: TodoStatus::InProgress,
            active_form: "wiring the reducer".into(),
        };
        let back: Todo = serde_json::from_str(&serde_json::to_string(&t).unwrap()).unwrap();
        assert_eq!(back, t);
        let unk: TodoStatus = serde_json::from_str("\"blocked\"").unwrap();
        assert_eq!(unk, TodoStatus::Unknown);
    }

    #[test]
    fn elicitation_roundtrips() {
        let e = Elicitation {
            id: ElicitationId::new("elic_1"),
            target_session_id: SessionId::new("conv_1"),
            params: ElicitationParams {
                mode: "url".into(),
                message: "approve?".into(),
                url: Some("https://x".into()),
                phase: None,
                policy_name: None,
                content_preview: None,
            },
        };
        let back: Elicitation = serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn pending_user_message_roundtrips() {
        let p = PendingUserMessage {
            pending_id: "pend_1".into(),
            content: "hello".into(),
            created_at: 1_700_000_000_000,
        };
        let back: PendingUserMessage =
            serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn skill_model_sandbox_roundtrip() {
        let s = SkillSummary { name: "grep".into(), description: None };
        assert_eq!(
            serde_json::from_str::<SkillSummary>(&serde_json::to_string(&s).unwrap()).unwrap(),
            s
        );
        let m = ModelOption { id: "opus".into(), label: "Opus 4.8".into() };
        assert_eq!(
            serde_json::from_str::<ModelOption>(&serde_json::to_string(&m).unwrap()).unwrap(),
            m
        );
        let sb = SandboxStatus { stage: "provisioning".into(), detail: None };
        assert_eq!(
            serde_json::from_str::<SandboxStatus>(&serde_json::to_string(&sb).unwrap()).unwrap(),
            sb
        );
    }
}
```

- [ ] **Step 2: Register the module** in `mod.rs` (`pub mod controls; pub use controls::*;`).

- [ ] **Step 3: Run tests** — `cargo test -p lens-core`. Expected: PASS.

- [ ] **Step 4: Gate** — clippy + fmt.

- [ ] **Step 5: Commit**

```bash
git add crates/lens-core/src/domain/controls.rs crates/lens-core/src/domain/mod.rs
git commit -m "feat(lens-core): P0 session control sub-types"
```

---

## Task 5: `BlockContext`, `ContentBlock`, `Item`, `ItemKind`, `StreamScratch`

**Files:**
- Create: `crates/lens-core/src/domain/item.rs`
- Modify: `crates/lens-core/src/domain/mod.rs`

**Interfaces:**
- Consumes: `ItemId`, `CallId`, `AgentId` (from `ids`); `Role`, `ErrorSource` (from `scalars`); `SessionResourceObject` (from `lens_client::generated`).
- Produces: `BlockContext`, `ContentBlock`, `Item`, `ItemKind`, `StreamScratch`, `MessageAcc`, `ReasoningAcc`.

> **Decision D-P0-1 (flag for review): `ResourceEvent` payload.** The design (§2.3) names `ResourceEvent { resource: SessionResourceObject }`. `lens_client::generated::SessionResourceObject` is a typed wire struct with full serde (`Serialize + Deserialize`), so this plan **reuses it** — faithful to the design and avoids inventing a shape for a payload whose rendering is workspace-doc-owned and deferred. Trade-off: couples `item.rs` to the `generated` codegen module. Alternative (if the reviewer prefers decoupling): domain-own a minimal `ResourceRef` and map at reduce time. Default = reuse; note it in the commit.

- [ ] **Step 1: Write the types + test**

`crates/lens-core/src/domain/item.rs`:

```rust
//! The canonical conversation unit (§2.3/§2.4) + transient stream accumulators
//! (§4.2). `Item` is the durable, reduced unit the transcript and disk hold.

use crate::domain::ids::{AgentId, CallId, ItemId};
use crate::domain::scalars::{ErrorSource, Role};
use lens_client::generated::SessionResourceObject;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// Attribution stamped onto every `Item` by the reducer (§2.4). Pure
/// attribution — the durable "when" lives on `Item.created_at` (grilling 2026-07-08).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockContext {
    /// "coder" | "coder.researcher"; None = root.
    pub agent: Option<String>,
    /// 0 = root, 1 = sub-agent, …
    pub depth: u32,
    /// Turn within the response.
    pub turn: u32,
}

/// One block of message content (§2.3 `Message.content`). `text` for text blocks;
/// `data` carries the opaque remainder for non-text blocks.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ContentBlock {
    pub kind: String,
    pub text: Option<String>,
    #[serde(default)]
    pub data: Value,
}

/// The durable, reduced conversation unit (§2.3).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Item {
    /// THE dedup/identity key. Persisted items carry only `id`, no seq.
    pub id: ItemId,
    /// SSE `sequence_number` when seen live; None for `GET /items`. Live-overlap
    /// dedup only — never a storage key.
    pub seq: Option<u64>,
    pub ctx: BlockContext,
    /// Epoch MILLIS, stamped by the reducer from an injected clock. The durable
    /// "when" (§2.3, replaces the dropped `BlockContext.timestamp`).
    pub created_at: i64,
    pub kind: ItemKind,
}

/// The typed item union (§2.3), mirroring omnigent conversation items.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ItemKind {
    Message {
        role: Role,
        content: Vec<ContentBlock>,
    },
    FunctionCall {
        call_id: CallId,
        name: String,
        arguments: Value,
        /// Wire enum: in_progress | completed | action_required | incomplete.
        status: String,
        agent_name: Option<String>,
    },
    FunctionCallOutput {
        call_id: CallId,
        output: String,
        arguments: Value,
    },
    Reasoning {
        full_text: String,
        summary_text: String,
        encrypted: bool,
    },
    /// web_search_call, mcp_call, …
    NativeTool {
        tool_type: String,
        data: Value,
    },
    Compaction {
        summary: String,
        token_count: Option<u64>,
    },
    SlashCommand {
        name: String,
        raw: String,
    },
    TerminalCommand {
        command: String,
    },
    /// Persisted error banner (mirrors response.error).
    Error {
        source: ErrorSource,
        code: String,
        message: String,
    },
    /// env | terminal | file (workspace doc). See Decision D-P0-1.
    ResourceEvent {
        resource: SessionResourceObject,
    },
    /// Switch-agent marker; `from` is SYNTHESIZED by the reducer (the wire event
    /// carries only agent_id/agent_name).
    AgentChanged {
        from: AgentId,
        to: AgentId,
        at: i64,
    },
}

// ── §4.2 transient accumulators (RAM-only) ──

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct StreamScratch {
    pub open_message: Option<MessageAcc>,
    pub open_reasoning: Option<ReasoningAcc>,
    pub unpaired_calls: HashMap<CallId, ItemId>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MessageAcc {
    /// 0.2.0: terminal-observed correlation.
    pub message_id: Option<String>,
    pub text: String,
    pub block_index: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ReasoningAcc {
    pub full_text: String,
    pub summary_text: String,
    pub encrypted: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ctx() -> BlockContext {
        BlockContext { agent: None, depth: 0, turn: 0 }
    }

    #[test]
    fn message_item_roundtrips() {
        let item = Item {
            id: ItemId::new("item_1"),
            seq: Some(3),
            ctx: ctx(),
            created_at: 1_700_000_000_000,
            kind: ItemKind::Message {
                role: Role::Assistant,
                content: vec![ContentBlock {
                    kind: "text".into(),
                    text: Some("hi".into()),
                    data: Value::Null,
                }],
            },
        };
        let back: Item = serde_json::from_str(&serde_json::to_string(&item).unwrap()).unwrap();
        assert_eq!(back, item);
    }

    #[test]
    fn item_created_at_survives_roundtrip_as_i64_millis() {
        // Grilling revision: durable "when" is created_at millis; no monotonic value.
        let item = Item {
            id: ItemId::new("item_2"),
            seq: None,
            ctx: ctx(),
            created_at: 1_700_000_123_456,
            kind: ItemKind::SlashCommand { name: "clear".into(), raw: "/clear".into() },
        };
        let back: Item = serde_json::from_str(&serde_json::to_string(&item).unwrap()).unwrap();
        assert_eq!(back.created_at, 1_700_000_123_456);
        assert_eq!(back, item);
    }

    #[test]
    fn every_itemkind_variant_roundtrips() {
        let kinds = vec![
            ItemKind::FunctionCall {
                call_id: CallId::new("call_1"),
                name: "read".into(),
                arguments: json!({"path": "a.rs"}),
                status: "in_progress".into(),
                agent_name: Some("coder".into()),
            },
            ItemKind::FunctionCallOutput {
                call_id: CallId::new("call_1"),
                output: "ok".into(),
                arguments: json!({"path": "a.rs"}),
            },
            ItemKind::Reasoning {
                full_text: "think".into(),
                summary_text: "t".into(),
                encrypted: false,
            },
            ItemKind::NativeTool { tool_type: "web_search_call".into(), data: json!({"q": "x"}) },
            ItemKind::Compaction { summary: "s".into(), token_count: Some(42) },
            ItemKind::TerminalCommand { command: "ls".into() },
            ItemKind::Error {
                source: ErrorSource::Server,
                code: "boom".into(),
                message: "kaboom".into(),
            },
            ItemKind::AgentChanged {
                from: AgentId::new("a"),
                to: AgentId::new("b"),
                at: 1_700_000_000_000,
            },
        ];
        for kind in kinds {
            let back: ItemKind =
                serde_json::from_str(&serde_json::to_string(&kind).unwrap()).unwrap();
            assert_eq!(back, kind);
        }
    }

    #[test]
    fn stream_scratch_default_is_empty_and_roundtrips() {
        let s = StreamScratch::default();
        assert!(s.open_message.is_none());
        assert!(s.unpaired_calls.is_empty());
        let back: StreamScratch =
            serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
    }
}
```

> The `#[serde(tag = "kind", ...)]` internally-tagged representation is a P0
> serde-shape choice (readable, self-describing). It does not need to match any
> wire tag (the reducer maps wire events → these). If a variant's fields would
> collide with the `kind` tag, switch to `#[serde(tag = "kind", content = "data")]`
> adjacently-tagged — but the current variants have no `kind` field, so internal
> tagging is fine.

- [ ] **Step 2: Register the module** in `mod.rs` (`pub mod item; pub use item::*;`).

- [ ] **Step 3: Run tests** — `cargo test -p lens-core`. Expected: PASS. If `lens_client::generated::SessionResourceObject`'s `deny_unknown_fields` or field set trips the round-trip, construct a minimal valid instance in a dedicated `resource_event_roundtrips` test using its public constructor/fields (inspect `crates/lens-client/src/generated.rs:7710`).

- [ ] **Step 4: Add the `ResourceEvent` round-trip test** — build a `SessionResourceObject` (check its public fields at `generated.rs:7710`; it derives `Serialize + Deserialize`), wrap in `ItemKind::ResourceEvent`, assert round-trip. If constructing it is awkward (all-private or builder-only), fall back to Decision D-P0-1's alternative (domain-own `ResourceRef`) and record that in the commit message.

- [ ] **Step 5: Gate** — clippy + fmt.

- [ ] **Step 6: Commit**

```bash
git add crates/lens-core/src/domain/item.rs crates/lens-core/src/domain/mod.rs
git commit -m "feat(lens-core): P0 Item/ItemKind union + BlockContext + StreamScratch"
```

---

## Task 6: `SessionState` aggregate + crate re-exports + full gate

**Files:**
- Create: `crates/lens-core/src/domain/session.rs`
- Modify: `crates/lens-core/src/domain/mod.rs` (final module list)
- Modify: `crates/lens-core/src/lib.rs` (top-level re-export of `domain`)

**Interfaces:**
- Consumes: every id + type from Tasks 1–5.
- Produces: `SessionState` — the per-session view-model aggregate.

- [ ] **Step 1: Write `SessionState` + test**

`crates/lens-core/src/domain/session.rs`:

```rust
//! `SessionState` — the per-session view-model (§2.2). Mirrors omnigent's
//! `SessionResponse` plus Lens-local fields. Pure data; the reducer (P1) is the
//! only writer (single-writer invariant, §8).

use crate::domain::controls::{Elicitation, ModelOption, SandboxStatus, SkillSummary, Todo};
use crate::domain::ids::{AgentId, ConnectionId, HostId, RunnerId, SessionId};
use crate::domain::item::{Item, StreamScratch};
use crate::domain::scalars::{ErrorInfo, HostType, SessionLifecycle, SessionStatusValue};
use crate::domain::usage::{Cost, PresenceViewer};
use crate::domain::controls::PendingUserMessage;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionState {
    // ── Identity & binding ──
    pub connection_id: ConnectionId,
    pub id: SessionId,
    pub agent_id: AgentId,
    pub agent_name: Option<String>,
    pub runner_id: Option<RunnerId>,
    pub parent_session_id: Option<SessionId>,

    // ── Status & lifecycle ──
    pub status: SessionStatusValue,
    pub last_task_error: Option<ErrorInfo>,
    /// Epoch SECONDS (distinct from Item.created_at millis).
    pub created_at: i64,

    // ── Model & controls ──
    pub llm_model: Option<String>,
    pub model_override: Option<String>,
    pub model_options: Option<Vec<ModelOption>>,
    pub reasoning_effort: Option<String>,
    pub collaboration_mode: Option<String>,
    pub context_window: Option<u64>,
    pub last_total_tokens: Option<u64>,
    pub cumulative_cost: Cost,

    // ── Workspace & host ──
    pub workspace: Option<String>,
    pub git_branch: Option<String>,
    pub host_type: HostType,
    pub host_id: Option<HostId>,
    pub sandbox_status: Option<SandboxStatus>,

    // ── Content ──
    pub items: Vec<Item>,
    pub todos: Vec<Todo>,
    pub skills: Vec<SkillSummary>,

    // ── Display & policy ──
    pub title: Option<String>,
    pub labels: BTreeMap<String, String>,
    pub permission_level: Option<u8>,
    pub pending_elicitations: Vec<Elicitation>,
    pub owner: Option<String>,

    // ── chrome: presence & co-viewers (RAM-only; excluded from P2 schema) ──
    pub presence: Vec<PresenceViewer>,

    // ── Lens-local transient (RAM only, never persisted) ──
    pub stream: StreamScratch,
    pub pending_user: Vec<PendingUserMessage>,

    // ── Lens-local persisted metadata ──
    pub archived: bool,
    pub lifecycle: SessionLifecycle,
    /// active-set LRU (epoch millis).
    pub last_focused_at: i64,
    /// reconcile cursor (typed client §7).
    pub last_seen_seq: Option<u64>,
}

impl SessionState {
    /// A fresh, empty session bound to `(connection, id)` with the given agent.
    /// Convenience constructor for the reducer/tests; all collections empty,
    /// status `Idle`, lifecycle `Active`.
    pub fn new(connection_id: ConnectionId, id: SessionId, agent_id: AgentId) -> Self {
        Self {
            connection_id,
            id,
            agent_id,
            agent_name: None,
            runner_id: None,
            parent_session_id: None,
            status: SessionStatusValue::Idle,
            last_task_error: None,
            created_at: 0,
            llm_model: None,
            model_override: None,
            model_options: None,
            reasoning_effort: None,
            collaboration_mode: None,
            context_window: None,
            last_total_tokens: None,
            cumulative_cost: Cost::default(),
            workspace: None,
            git_branch: None,
            host_type: HostType::External,
            host_id: None,
            sandbox_status: None,
            items: Vec::new(),
            todos: Vec::new(),
            skills: Vec::new(),
            title: None,
            labels: BTreeMap::new(),
            permission_level: None,
            pending_elicitations: Vec::new(),
            owner: None,
            presence: Vec::new(),
            stream: StreamScratch::default(),
            pending_user: Vec::new(),
            archived: false,
            lifecycle: SessionLifecycle::Active,
            last_focused_at: 0,
            last_seen_seq: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_session_is_idle_active_and_empty() {
        let s = SessionState::new(
            ConnectionId::new("conn_1"),
            SessionId::new("conv_1"),
            AgentId::new("agent_1"),
        );
        assert_eq!(s.status, SessionStatusValue::Idle);
        assert_eq!(s.lifecycle, SessionLifecycle::Active);
        assert!(s.items.is_empty());
        assert!(!s.archived);
    }

    #[test]
    fn empty_session_roundtrips() {
        let s = SessionState::new(
            ConnectionId::new("conn_1"),
            SessionId::new("conv_1"),
            AgentId::new("agent_1"),
        );
        let back: SessionState =
            serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn populated_session_roundtrips() {
        use crate::domain::item::{BlockContext, ContentBlock, Item, ItemKind};
        use crate::domain::scalars::Role;
        use crate::domain::ids::ItemId;

        let mut s = SessionState::new(
            ConnectionId::new("conn_1"),
            SessionId::new("conv_1"),
            AgentId::new("agent_1"),
        );
        s.status = SessionStatusValue::Running;
        s.title = Some("my session".into());
        s.labels.insert("env".into(), "prod".into());
        s.items.push(Item {
            id: ItemId::new("item_1"),
            seq: Some(1),
            ctx: BlockContext { agent: None, depth: 0, turn: 0 },
            created_at: 1_700_000_000_000,
            kind: ItemKind::Message {
                role: Role::User,
                content: vec![ContentBlock {
                    kind: "text".into(),
                    text: Some("hello".into()),
                    data: serde_json::Value::Null,
                }],
            },
        });
        let back: SessionState =
            serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
    }
}
```

> `SessionState` derives `PartialEq` but **not** `Eq` (it holds `Cost` → `f64`).

- [ ] **Step 2: Finalize `mod.rs`** — ensure the full module list is present:

```rust
//! §2 domain model — pure data + serde, no logic.

pub mod ids;
pub mod scalars;
pub mod usage;
pub mod controls;
pub mod item;
pub mod session;

pub use ids::*;
pub use scalars::*;
pub use usage::*;
pub use controls::*;
pub use item::*;
pub use session::*;
```

- [ ] **Step 3: Confirm `lib.rs` re-export** — `lib.rs` already has `pub mod domain;`. Optionally add `pub use domain::*;` for a flat top-level surface (matches `lens-client`'s style — verify against `crates/lens-client/src/lib.rs` and mirror). Prefer the flat re-export so consumers write `lens_core::SessionState`.

- [ ] **Step 4: Run the FULL crate test suite** — `cargo test -p lens-core`. Expected: all tasks' tests PASS.

- [ ] **Step 5: Full gate** —
  - `cargo clippy -p lens-core --all-targets` → zero warnings.
  - `cargo fmt --check` → clean.
  - `cargo build -p lens-core` → clean.
  - Confirm `git status` shows `crates/lens-client/src/generated.rs` untouched.

- [ ] **Step 6: Commit**

```bash
git add crates/lens-core/src/domain/session.rs crates/lens-core/src/domain/mod.rs crates/lens-core/src/lib.rs
git commit -m "feat(lens-core): P0 SessionState aggregate + domain re-exports"
```

---

## P0 Exit Criteria (the phase gate)

- [ ] `lens-core` crate exists, builds, and is a workspace member (auto via `crates/*` glob).
- [ ] The full §2 domain type set is defined (see Type Inventory) — pure data + serde, no logic.
- [ ] Every type serde round-trips (test-proven); churn-safe enums map unknown literals to `Unknown`.
- [ ] `Item` round-trips through `created_at` (epoch millis) with **no monotonic value** — grilling revision honored; `BlockContext` is `{ agent, depth, turn }` only.
- [ ] `SessionState::new(..)` produces an Idle/Active/empty session that round-trips.
- [ ] `presence` and the transient accumulators are carried but flagged RAM-only for P2.
- [ ] `cargo test -p lens-core` · `cargo clippy -p lens-core --all-targets` (zero warnings) · `cargo fmt --check` all green.
- [ ] `lens-client/src/generated.rs` untouched.
- [ ] **Cross-family review** at the phase seam (CLAUDE.md MANDATORY; `review-spend-policy` — one consolidated review of the whole P0 diff, a family other than the author's).

---

## Open decisions flagged for review

- **D-P0-1 — `ResourceEvent` payload** (Task 5): reuse `lens_client::generated::SessionResourceObject` (default, design-faithful) vs domain-own a `ResourceRef` (decouples from `generated`). Reviewer to confirm.
- **Flat top-level re-export** (Task 6 Step 3): `pub use domain::*;` in `lib.rs` — mirror `lens-client`'s convention.

## P1 handoff notes (from the P0 plan cross-family review)

- **`lens-client::stream::PresenceViewer` gap** (Task 3): the stream wrapper drops
  `joined_at`/`idle` (exposes only `user_id`), so P1 cannot fill the domain
  `PresenceViewer` from `ServerStreamEvent::Presence` until lens-client is widened
  or P1 reads `lens_client::generated::PresenceViewer`. Resolve in the P1 spec.
- **`ModelUsage` field set** is now wire-faithful (cache buckets + `total_cost_usd`,
  all optional) so P1's usage normalization can map it 1:1 from the wire.
```
