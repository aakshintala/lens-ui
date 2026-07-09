# State-Model Engine P2 — Persistence (`lens-core/persist`, §6) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

> **Opus pre-build review APPLIED (2026-07-08, verdict SHIP-WITH-FIXES → resolved).**
> Column-index mapping (INSERT ?1..?34 ↔ params ↔ `SESSION_COLUMNS` ↔ `row_to_session`
> get(0..30)) and the `reconcile` negative-ordinal park were **verified correct** (no
> off-by-one). 9 findings applied, marked `REVIEW#n` in-code: **#1** corrupt/garbled
> `schema_version` now degrades (`VersionState::Unreadable`) instead of hard-failing the
> open (was blocking reads too — broke §6.3); **#2** WAL flip + DDL now run ONLY after the
> mode is resolved (was mutating a future-version file before the gate); **#3** WAL via
> `execute_batch` (avoids `pragma_update` `ExecuteReturnedResults`); **#4** dead `from_json`
> import dropped (zero-warning gate); **#5** `PRAGMA foreign_keys=ON` on the RW path (FK was
> decorative); **#6** unsigned columns read/written through `i64` uniformly (loads stay
> total); **#7** tests added — `tombstoned_at` preservation, bare-token (Bridge-contract)
> assertions, corrupt-version degrade; **#8/#9** doc notes on the decode-error taxonomy +
> `upsert_item` fresh-ordinal precondition. Unused `PersistError::SchemaTooNew` removed (the
> degraded state is `StoreMode::ReadOnlyDegraded` + `PersistError::ReadOnly`).

**Goal:** Build the two-tier local persistence layer for one `(connection, session)` — a control-plane **`ControlStore`** (`lens.db`: connections / sessions / cost_samples / meta) and a per-session **`TranscriptStore`** (`transcripts/<conn>/<conv>.db`: items + self-describing meta) — over `rusqlite`/WAL, exposing **load / upsert / reconcile-by-item-id** primitives. Storage only; the wake/actor wiring that *calls* these is P3.

**Architecture:** `lens-core` gains a `persist` module. Two role **traits** (`ControlStore`, `TranscriptStore`) name the SessionPersistence abstraction (§6.1 "a backing-store swap is a trait impl"); `SqliteControlStore` / `SqliteTranscriptStore` are the v1 blocking implementations. The schema is portable, denormalized, and a **stable Bridge read contract** (§6.1). Persisted state is the durable subset of `SessionState`/`Item`; in-progress accumulators (`StreamScratch`), `presence`, and `pending_user` are **RAM-only, never a column**. Each SQLite file carries `meta.schema_version` and is version-gated on open (unknown future version ⇒ **read-only-degraded**, never corrupted). `reconcile(&[Item])` makes a transcript file match server truth by `item_id` (disk may lag: compaction rewrote history, items edited, new items committed while sleeping).

**Tech Stack:** Rust (edition 2024, workspace `rust-version = 1.91`), `rusqlite` with the `bundled` feature (compiles SQLite from source → zero system dependency, portable per §6.1), `serde`/`serde_json` (json columns), `lens-core` domain types (P0/P1), `criterion` (existing dev-dep — persistence throughput bench per AGENTS.md), `tempfile` (new dev-dep — temp-db tests).

## Global Constraints

- **Design source of truth:** `docs/design/app-architecture-and-state-model.md` §6 (LOCKED) + spec `docs/superpowers/specs/2026-07-08-state-model-engine-design.md` §4 "P2".
- **`lens-core` has NO gpui dependency.** No threads, no actor in P2 — blocking storage primitives only.
- **The UI never panics** (AGENTS.md MANDATORY): every load path is total over decodable disk data. Corrupt/unknown-shape rows degrade (skip-with-log-shape / read-only-degraded), never `panic!`/`unwrap` on stored data. `expect`/`unwrap` is allowed **only** on our own in-memory serialization of our own enums (documented invariants), never on bytes read back from disk.
- **Schema is a portable, denormalized Bridge read contract** (§6.1): standard SQL types, JSON payloads in `TEXT` columns that map to Postgres `jsonb`, text ids, epoch integer timestamps, a stable `items.kind` enum vocabulary, denormalized `BlockContext` columns (`agent`/`depth`/`turn`). No SQLite-only feature on the critical path.
- **RAM-only, never persisted** (§2.5/§4.2): `StreamScratch` (`SessionState.stream`), `presence`, `pending_user`. Excluded from every schema.
- **WAL on both tiers** (§6.1): `PRAGMA journal_mode=WAL` on open. Readers never block the writer.
- **Per-file `schema_version` migration gate** (§6.3): `SCHEMA_VERSION` known → open read-write; unknown *future* version → open **read-only-degraded** (writes return `PersistError::SchemaTooNew`, reads still work). Never silent corruption.
- **Production lint bar:** `lints.workspace = true`. Zero warnings.
- **`generated.rs` in `lens-client` is untouched.** No `lens-client` change at all in P2.
- **Gate every task:** `cargo test -p lens-core` · `cargo clippy -p lens-core --all-targets` (zero warnings) · `cargo fmt --check`.
- **Review workflow (this branch):** build = composer-2.5 (cursor-delegate); **all reviews = Opus (`Agent` tool)** — Codex/gpt-5.5 and non-Composer Cursor models are out of credits this session. Opus-reviewing-composer still satisfies the MANDATORY cross-family review-diversity rule (different family). The plan itself gets one **Opus review before build** (the decisions block below is the primary target).

---

## Decisions (REVIEW THESE FIRST — Opus pre-build review target)

These resolve drift between the P0/P1 `SessionState`/`Item` as built and the §6.2
schema *sketch*. §6.2 is explicitly a "sketch"; §6.1's constraints (portable,
denormalized, Bridge-readable, RAM-only exclusions) are the binding contract.

- **D-P2-1 — Two role traits, no umbrella trait.** The "abstract `SessionPersistence`
  trait" (§6.1) is realized as the `persist` module + **two** role traits
  (`ControlStore`, `TranscriptStore`), not a single god-trait — they have disjoint
  lifetimes (app-lifetime vs per-session, §6.1) and disjoint methods. YAGNI on an
  umbrella. SQLite impls are `SqliteControlStore` / `SqliteTranscriptStore`.
- **D-P2-2 — Cost persists as a lossless companion json + denormalized Bridge
  projections.** §6.2's `sessions` columns (`cumulative_cost REAL`,
  `last_total_tokens INTEGER`, `usage_by_model TEXT`) are a **lossy projection** of
  the P0 `Cost { cumulative_usage: Usage, total_cost_usd }` — `Usage.input_tokens /
  output_tokens / reasoning_tokens / context_tokens` have no column. To keep reload
  exact **and** Bridge-readable: write the three denormalized columns (Bridge read
  contract) **plus** a lossless `cost_json TEXT` (full `Cost`, Lens's own reload
  source). Load reads `cost_json`. Added column, flagged.
- **D-P2-3 — `terminal_pending` is persisted** (`terminal_pending INTEGER NOT NULL
  DEFAULT 0`). P1's `session.rs:46` declares it a "RAM+persisted scalar" but §6.2
  omits the column. The §6.2 sketch is extensible; honor P1's contract. Added column,
  flagged.
- **D-P2-4 — Store-managed columns are preserved across upsert, not sourced from
  `SessionState`.** `pinned` (§9 registry), `last_status` (§2.2 coarse-poll guard),
  `tombstoned_at` (P3 server-delete lifecycle) have **no `SessionState` field** in
  P0/P1. `upsert_session` sets them to their DEFAULT on INSERT and **omits them from
  the `ON CONFLICT DO UPDATE SET` clause** so a later fold cannot clobber a P3/§9
  write. `updated_at` is store-managed and **always** written (= injected `now_ms`).
- **D-P2-5 — Live-stream chrome is re-derived on wake, not persisted.** `model_options`,
  `sandbox_status`, `pending_elicitations` are transient bootstrap chrome (re-emitted
  by the fresh stream on wake, §6.3) — not columned in §6.2 and **not** persisted.
  `load_session` returns them empty/`None`; the P3 wake reconcile refills from the
  live tail. (Contrast `todos`/`skills`, which *are* columned and persisted.)
- **D-P2-6 — `load_session` returns a disk-snapshot `SessionState`, items empty.**
  `items` live in the transcript file (§6.2), not `lens.db`. `ControlStore::load_session`
  fills persisted scalars/collections and leaves `items = vec![]`, RAM-only fields at
  their `Default`/empty. The caller (P3 wake) paints this, then loads items from the
  `TranscriptStore` and reconciles against the live stream. This is the "disk may lag,
  reconcile repairs" model (§6.3) — `load_session` is deliberately *not* a full session.
- **D-P2-7 — `ordinal` is store-assigned, not on the domain `Item`.** `Item` carries
  no ordinal; it is the item's index in `SessionState.items`. `upsert_item(ordinal,
  item)` takes it explicitly (the actor passes the append position); `reconcile(items)`
  reassigns `ordinal = slice index` for the whole set. `UNIQUE(ordinal)` per §6.2.
- **D-P2-8 — Enum columns store the bare serde string; `#[serde(other)]` makes reload
  churn-safe.** `status`/`host_type`/`lifecycle` are stored as their lowercase serde
  token (`"waiting"`, not `"\"waiting\""`) for Bridge. Reload via
  `serde_json::from_value(Value::String(col))` — `SessionStatusValue`/`ErrorSource`
  already carry `#[serde(other)] Unknown`, so an unrecognized stored token degrades,
  never errors.
- **D-P2-9 — `payload` stores the full tagged `ItemKind` json; `kind` column is the
  denormalized index.** `items.payload = serde_json::to_string(&item.kind)` (a
  `{"kind":"…",…}` object, jsonb-mappable). `items.kind` duplicates the discriminant
  as a bare token for Bridge's stable enum. Load deserializes `ItemKind` from `payload`
  alone; `kind` is a redundant read-contract projection.

---

## File Structure

```
crates/lens-core/
  Cargo.toml                 # MODIFY — add rusqlite (bundled) dep; add tempfile dev-dep; add [[bench]]
  benches/
    persist_throughput.rs    # NEW — criterion: transcript upsert + load throughput (I/O-bound baseline)
  src/
    lib.rs                   # MODIFY — `pub mod persist;` + re-exports
    persist/
      mod.rs                 # NEW — module root: PersistError/Result, StoreMode, ConnectionRecord,
                             #       the two role traits (ControlStore, TranscriptStore), re-exports
      db.rs                  # NEW — shared open_db(path, ddl, version) → (Connection, StoreMode);
                             #       WAL pragma; meta table; schema_version read/gate helpers
      schema.rs              # NEW — SQL DDL constants + SCHEMA_VERSION + items.kind vocabulary check
      map.rs                 # NEW — SessionState↔row + Item↔row mapping (json/enum column helpers)
      control.rs             # NEW — SqliteControlStore: connections/sessions/cost_samples
      transcript.rs          # NEW — SqliteTranscriptStore: items + reconcile + self-describing meta
```

Root workspace `Cargo.toml` globs `members = ["crates/*"]` — no members edit.

---

## Task 1: Crate wiring + shared open/version-gate (`persist/mod.rs`, `persist/db.rs`, `persist/schema.rs`)

**Files:**
- Modify: `crates/lens-core/Cargo.toml`
- Create: `crates/lens-core/src/persist/mod.rs`
- Create: `crates/lens-core/src/persist/db.rs`
- Create: `crates/lens-core/src/persist/schema.rs`
- Modify: `crates/lens-core/src/lib.rs`

**Interfaces:**
- Produces:
  - `persist::PersistError` (enum) + `persist::Result<T> = std::result::Result<T, PersistError>`
  - `persist::StoreMode { ReadWrite, ReadOnlyDegraded }`
  - `persist::db::open_db(path: &Path, ddl: &str, current_version: u32) -> Result<(rusqlite::Connection, StoreMode)>`
  - `persist::db::VersionState { Fresh, Known(u32), Unreadable }`
  - `persist::db::read_schema_version(conn: &rusqlite::Connection) -> Result<VersionState>`
  - `persist::schema::SCHEMA_VERSION: u32` (= `1`)
  - `persist::schema::{CONTROL_DDL, TRANSCRIPT_DDL}: &str`

- [ ] **Step 1: Add deps to `crates/lens-core/Cargo.toml`**

Under `[dependencies]` add:

```toml
rusqlite = { version = "0.32", features = ["bundled"] }
```

Under `[dev-dependencies]` add:

```toml
tempfile = "3"
```

(Note: any `0.3x` rusqlite with the `bundled` feature is acceptable if `0.32` fails to resolve — the API surface used here is stable across those. `bundled` is REQUIRED — it vendors SQLite so there is no system-lib dependency, per §6.1 portability.)

- [ ] **Step 2: Write `persist/schema.rs` (DDL constants + version)**

```rust
//! Portable SQL DDL for the two tiers (§6.2) + the schema version. The schema is
//! a STABLE, DENORMALIZED read contract (§6.1) — Bridge reads these tables.

/// Bumped only on a breaking schema change; gates per-file migration (§6.3).
pub const SCHEMA_VERSION: u32 = 1;

/// `lens.db` — control plane (one file). `meta` is created by `db::open_db`.
/// P2 additions vs §6.2 sketch: `cost_json` (D-P2-2), `terminal_pending` (D-P2-3).
pub const CONTROL_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS connections (
  id          TEXT PRIMARY KEY,
  base_url    TEXT NOT NULL,
  auth_kind   TEXT NOT NULL,
  label       TEXT,
  server_info TEXT,
  created_at  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS sessions (
  connection_id     TEXT NOT NULL REFERENCES connections(id),
  id                TEXT NOT NULL,
  agent_id          TEXT NOT NULL,
  agent_name        TEXT,
  runner_id         TEXT,
  parent_session_id TEXT,
  status            TEXT NOT NULL,
  last_task_error   TEXT,
  llm_model         TEXT,
  model_override    TEXT,
  reasoning_effort  TEXT,
  collaboration_mode TEXT,
  context_window    INTEGER,
  last_total_tokens INTEGER,
  cumulative_cost   REAL,
  usage_by_model    TEXT,
  cost_json         TEXT,
  workspace         TEXT,
  git_branch        TEXT,
  host_type         TEXT NOT NULL,
  host_id           TEXT,
  title             TEXT,
  labels            TEXT,
  permission_level  INTEGER,
  owner             TEXT,
  todos             TEXT,
  skills            TEXT,
  terminal_pending  INTEGER NOT NULL DEFAULT 0,
  created_at        INTEGER NOT NULL,
  archived          INTEGER NOT NULL DEFAULT 0,
  lifecycle         TEXT NOT NULL DEFAULT 'active',
  pinned            INTEGER NOT NULL DEFAULT 0,
  tombstoned_at     INTEGER,
  last_focused_at   INTEGER,
  last_status       TEXT,
  last_seen_seq     INTEGER,
  updated_at        INTEGER NOT NULL,
  PRIMARY KEY (connection_id, id)
);

CREATE TABLE IF NOT EXISTS cost_samples (
  connection_id  TEXT NOT NULL,
  session_id     TEXT NOT NULL,
  sampled_at     INTEGER NOT NULL,
  total_cost_usd REAL NOT NULL,
  PRIMARY KEY (connection_id, session_id, sampled_at)
);
"#;

/// Per-session transcript file. `meta` (created by `db::open_db`) additionally
/// carries `connection_id` + `session_id` so the file is self-describing (§6.2).
pub const TRANSCRIPT_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS items (
  item_id    TEXT NOT NULL,
  live_seq   INTEGER,
  ordinal    INTEGER NOT NULL,
  kind       TEXT NOT NULL,
  payload    TEXT NOT NULL,
  agent      TEXT,
  depth      INTEGER NOT NULL DEFAULT 0,
  turn       INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL,
  PRIMARY KEY (item_id),
  UNIQUE (ordinal)
);
"#;
```

- [ ] **Step 3: Write `persist/mod.rs` (error type, mode, module wiring)**

```rust
//! §6 local persistence: the `SessionPersistence` abstraction as two role traits
//! (D-P2-1) over a portable, Bridge-readable SQLite schema (§6.1). Storage
//! primitives only — the wake/actor wiring that calls them is P3.

pub mod control;
pub mod db;
pub mod map;
pub mod schema;
pub mod transcript;

use crate::domain::ids::{ConnectionId, SessionId};
use crate::domain::item::Item;
use crate::domain::session::SessionState;
use thiserror::Error;

pub use control::SqliteControlStore;
pub use transcript::SqliteTranscriptStore;

pub type Result<T> = std::result::Result<T, PersistError>;

#[derive(Debug, Error)]
pub enum PersistError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("encode/decode error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("write refused: store opened read-only-degraded (schema newer than this build, §6.3)")]
    ReadOnly,

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// How a file was opened after the schema-version gate (§6.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreMode {
    /// `meta.schema_version` == this build's `SCHEMA_VERSION` (or fresh file).
    ReadWrite,
    /// `meta.schema_version` > this build's — reads allowed, writes refused.
    ReadOnlyDegraded,
}

/// A `connections` row (§6.2). No P0 domain owner yet (§9 registry scope), so the
/// persist layer owns this record type for P2.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectionRecord {
    pub id: ConnectionId,
    pub base_url: String,
    /// none|bearer|cookie|forwarded_email
    pub auth_kind: String,
    pub label: Option<String>,
    /// json from GET /v1/info; stored verbatim.
    pub server_info: Option<String>,
    pub created_at: i64,
}

/// Control-plane role (`lens.db`, app-lifetime). D-P2-1.
pub trait ControlStore {
    fn mode(&self) -> StoreMode;
    fn upsert_connection(&self, c: &ConnectionRecord) -> Result<()>;
    fn load_connections(&self) -> Result<Vec<ConnectionRecord>>;
    /// `now_ms` stamps `updated_at` (store-managed, D-P2-4). Preserves the
    /// store-managed columns (`pinned`/`last_status`/`tombstoned_at`) on update.
    fn upsert_session(&self, s: &SessionState, now_ms: i64) -> Result<()>;
    /// Disk snapshot: items empty, RAM-only fields defaulted (D-P2-6).
    fn load_session(&self, conn: &ConnectionId, id: &SessionId) -> Result<Option<SessionState>>;
    fn list_sessions(&self, conn: &ConnectionId) -> Result<Vec<SessionState>>;
    fn insert_cost_sample(
        &self,
        conn: &ConnectionId,
        id: &SessionId,
        sampled_at: i64,
        total_cost_usd: f64,
    ) -> Result<()>;
    /// Ordered `(sampled_at, total_cost_usd)` in `[since, until]` (inclusive).
    fn cost_samples_in(
        &self,
        conn: &ConnectionId,
        id: &SessionId,
        since: i64,
        until: i64,
    ) -> Result<Vec<(i64, f64)>>;
}

/// Per-session transcript role (one file per `(connection, session)`). D-P2-1.
pub trait TranscriptStore {
    fn mode(&self) -> StoreMode;
    /// The `(connection_id, session_id)` from the file's self-describing meta.
    fn identity(&self) -> Result<(ConnectionId, SessionId)>;
    /// Write-through one finalized item at its canonical `ordinal` (D-P2-7).
    fn upsert_item(&self, ordinal: i64, item: &Item) -> Result<()>;
    /// All items ordered by `ordinal`.
    fn load_items(&self) -> Result<Vec<Item>>;
    /// Make the file match server truth by `item_id`: upsert each at `ordinal =
    /// index`, delete rows whose id is absent (§6.3 reconcile-by-id).
    fn reconcile(&self, items: &[Item]) -> Result<()>;
}
```

Add `thiserror = "2"` to `[dependencies]` in `crates/lens-core/Cargo.toml` (matches `lens-client`).

- [ ] **Step 4: Write `persist/db.rs` (shared open + WAL + version gate) — TDD**

Write the failing test first in `db.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::persist::schema::SCHEMA_VERSION;
    use tempfile::tempdir;

    const DDL: &str = "CREATE TABLE IF NOT EXISTS t (a INTEGER);";

    #[test]
    fn fresh_file_opens_read_write_at_current_version() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("x.db");
        let (conn, mode) = open_db(&path, DDL, SCHEMA_VERSION).unwrap();
        assert_eq!(mode, StoreMode::ReadWrite);
        assert_eq!(read_schema_version(&conn).unwrap(), VersionState::Known(SCHEMA_VERSION));
        // WAL is on.
        let jm: String = conn.query_row("PRAGMA journal_mode;", [], |r| r.get(0)).unwrap();
        assert_eq!(jm.to_lowercase(), "wal");
    }

    #[test]
    fn future_version_opens_read_only_degraded() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("x.db");
        // Create at current version, then bump meta to a future version.
        {
            let (conn, _) = open_db(&path, DDL, SCHEMA_VERSION).unwrap();
            conn.execute(
                "UPDATE meta SET value = ?1 WHERE key = 'schema_version'",
                [(SCHEMA_VERSION + 1).to_string()],
            )
            .unwrap();
        }
        let (_conn, mode) = open_db(&path, DDL, SCHEMA_VERSION).unwrap();
        assert_eq!(mode, StoreMode::ReadOnlyDegraded);
    }

    #[test]
    fn corrupt_version_cell_degrades_never_hard_errors() {
        // REVIEW#1: a garbled schema_version must NOT fail the open (which would
        // block reads too) — §6.3 "never corrupted → read-only-degraded".
        let dir = tempdir().unwrap();
        let path = dir.path().join("x.db");
        {
            let (conn, _) = open_db(&path, DDL, SCHEMA_VERSION).unwrap();
            conn.execute("UPDATE meta SET value = 'not-a-number' WHERE key = 'schema_version'", [])
                .unwrap();
        }
        let (conn, mode) = open_db(&path, DDL, SCHEMA_VERSION).unwrap();
        assert_eq!(mode, StoreMode::ReadOnlyDegraded);
        // Reads still work in degraded mode.
        assert!(conn.query_row("SELECT COUNT(*) FROM meta", [], |r| r.get::<_, i64>(0)).is_ok());
    }
}
```

Run: `cargo test -p lens-core persist::db -- --nocapture`
Expected: FAIL (`open_db` not defined).

- [ ] **Step 5: Implement `persist/db.rs`**

```rust
//! Shared file open: the `meta` version-carrier, the schema-version gate (§6.3),
//! and — ONLY on the read-write path — WAL + DDL. Both tiers open through here so
//! the gate is written once.
//!
//! REVIEW#1/#2 ordering invariant: we must decide the mode BEFORE mutating the
//! file (WAL flip / DDL). The one pre-gate write is `CREATE TABLE IF NOT EXISTS
//! meta` — benign because `meta (key TEXT PRIMARY KEY, value TEXT)` is the STABLE
//! version-carrier by contract (same shape across every schema version), so on a
//! future-version file it already exists and the statement is a no-op.

use crate::persist::{Result, StoreMode};
use rusqlite::Connection;
use std::path::Path;

/// The three states of a file's `meta.schema_version` cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VersionState {
    /// No `schema_version` row — a brand-new file.
    Fresh,
    /// A parseable version integer.
    Known(u32),
    /// A present-but-garbled cell (non-numeric). Treated as "do not understand" →
    /// degraded, never a hard open failure (§6.3).
    Unreadable,
}

/// Open (creating parent dirs + file) `path` and gate on `schema_version`:
/// - `Fresh`            → run `ddl`, stamp `schema_version = current`, `ReadWrite`.
/// - `Known(current)`   → run `ddl` (idempotent), `ReadWrite`.
/// - `Known(< current)` → run `ddl` forward + re-stamp, `ReadWrite`.
/// - `Known(> current)` → `ReadOnlyDegraded` (never migrate down; no file write).
/// - `Unreadable`       → `ReadOnlyDegraded` (never corrupt; no file write).
///
/// WAL + `PRAGMA foreign_keys=ON` are enabled only on the `ReadWrite` branches
/// (§6.1); a degraded file is opened for reads without touching its header.
pub fn open_db(path: &Path, ddl: &str, current_version: u32) -> Result<(Connection, StoreMode)> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(path)?;
    // The only pre-gate write — see the ordering invariant above.
    conn.execute_batch("CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT);")?;

    // Decide the mode BEFORE any WAL/DDL mutation.
    let (mode, stamp): (StoreMode, Option<u32>) = match read_schema_version(&conn)? {
        VersionState::Fresh => (StoreMode::ReadWrite, Some(current_version)),
        VersionState::Known(v) if v == current_version => (StoreMode::ReadWrite, None),
        VersionState::Known(v) if v < current_version => (StoreMode::ReadWrite, Some(current_version)),
        VersionState::Known(_) | VersionState::Unreadable => (StoreMode::ReadOnlyDegraded, None),
    };

    if mode == StoreMode::ReadWrite {
        // execute_batch discards PRAGMA's returned row (REVIEW#3: pragma_update can
        // surface ExecuteReturnedResults for journal_mode).
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        conn.execute_batch(ddl)?;
        if let Some(v) = stamp {
            conn.execute(
                "INSERT INTO meta (key, value) VALUES ('schema_version', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                [v.to_string()],
            )?;
        }
    }
    Ok((conn, mode))
}

/// Classify the stored `meta.schema_version` cell. Requires `meta` to exist.
/// `QueryReturnedNoRows` → `Fresh`; a real query error (locked db, I/O) propagates;
/// a present-but-unparseable cell → `Unreadable` (NOT an error — REVIEW#1).
pub fn read_schema_version(conn: &Connection) -> Result<VersionState> {
    let cell: Option<String> = match conn.query_row(
        "SELECT value FROM meta WHERE key = 'schema_version'",
        [],
        |r| r.get::<_, String>(0),
    ) {
        Ok(s) => Some(s),
        Err(rusqlite::Error::QueryReturnedNoRows) => None,
        Err(e) => return Err(e.into()),
    };
    Ok(match cell {
        None => VersionState::Fresh,
        Some(s) => match s.parse::<u32>() {
            Ok(v) => VersionState::Known(v),
            Err(_) => VersionState::Unreadable,
        },
    })
}
```

- [ ] **Step 6: Wire `lib.rs`**

Add to `crates/lens-core/src/lib.rs`:

```rust
pub mod persist;
```

and extend the re-export line:

```rust
pub use persist::{ControlStore, PersistError, StoreMode, TranscriptStore};
```

- [ ] **Step 7: Run tests + gate**

Run: `cargo test -p lens-core persist::db`
Expected: PASS (2 tests).
Run: `cargo clippy -p lens-core --all-targets` (zero warnings) and `cargo fmt`.

- [ ] **Step 8: Commit**

```bash
git add crates/lens-core/Cargo.toml crates/lens-core/src/lib.rs crates/lens-core/src/persist/
git commit -m "feat(lens-core): P2 task 1 — persist scaffold, shared open + schema-version gate"
```

---

## Task 2: Column mapping helpers (`persist/map.rs`)

**Files:**
- Create: `crates/lens-core/src/persist/map.rs`
- Test: inline `#[cfg(test)]` in `map.rs`

**Interfaces:**
- Consumes: P0 domain (`Item`, `ItemKind`, `SessionState`, enums).
- Produces:
  - `map::enum_token<T: Serialize>(v: &T) -> Result<String>` (bare serde string for a string-serializing enum)
  - `map::from_token<T: DeserializeOwned>(s: String) -> Result<T>`
  - `map::json_string<T: Serialize>(v: &T) -> Result<String>`
  - `map::from_json<T: DeserializeOwned>(s: &str) -> Result<T>`
  - `map::item_kind_token(k: &ItemKind) -> &'static str` (the `items.kind` vocabulary, D-P2-9)

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::item::ItemKind;
    use crate::domain::scalars::SessionStatusValue;

    #[test]
    fn enum_token_is_bare_string_and_roundtrips_with_churn_safety() {
        let t = enum_token(&SessionStatusValue::Waiting).unwrap();
        assert_eq!(t, "waiting"); // NOT "\"waiting\""
        let back: SessionStatusValue = from_token(t).unwrap();
        assert_eq!(back, SessionStatusValue::Waiting);
        // Unknown stored token degrades, never errors (D-P2-8).
        let back: SessionStatusValue = from_token("superseded".to_string()).unwrap();
        assert_eq!(back, SessionStatusValue::Unknown);
    }

    #[test]
    fn item_kind_token_matches_schema_vocabulary() {
        assert_eq!(
            item_kind_token(&ItemKind::TerminalCommand { command: "ls".into() }),
            "terminal_command"
        );
        assert_eq!(
            item_kind_token(&ItemKind::Reasoning {
                full_text: String::new(),
                summary_text: String::new(),
                encrypted: false,
            }),
            "reasoning"
        );
    }
}
```

Run: `cargo test -p lens-core persist::map`
Expected: FAIL (module not defined).

- [ ] **Step 2: Implement `persist/map.rs`**

```rust
//! Column mapping: bare enum tokens (D-P2-8), json columns (D-P2-9), and the
//! `items.kind` vocabulary. Serialization of our OWN enums cannot fail on a
//! string-serializing type — the `expect` invariants below are never external data.

use crate::domain::item::ItemKind;
use crate::persist::{PersistError, Result};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

/// A string-serializing enum → its bare token (`"waiting"`), for a Bridge column.
pub fn enum_token<T: Serialize>(v: &T) -> Result<String> {
    match serde_json::to_value(v)? {
        Value::String(s) => Ok(s),
        other => Err(PersistError::Json(serde::ser::Error::custom(format!(
            "expected a string-serializing enum, got {other}"
        )))),
    }
}

/// A stored bare token → the enum (churn-safe via the enum's `#[serde(other)]`).
pub fn from_token<T: DeserializeOwned>(s: String) -> Result<T> {
    Ok(serde_json::from_value(Value::String(s))?)
}

/// Any serde type → a json `TEXT` column value.
pub fn json_string<T: Serialize>(v: &T) -> Result<String> {
    Ok(serde_json::to_string(v)?)
}

/// A json `TEXT` column value → the serde type.
pub fn from_json<T: DeserializeOwned>(s: &str) -> Result<T> {
    Ok(serde_json::from_str(s)?)
}

/// The stable `items.kind` vocabulary (§6.2 / D-P2-9). Matches `ItemKind`'s
/// snake_case serde tags exactly.
pub fn item_kind_token(k: &ItemKind) -> &'static str {
    match k {
        ItemKind::Message { .. } => "message",
        ItemKind::FunctionCall { .. } => "function_call",
        ItemKind::FunctionCallOutput { .. } => "function_call_output",
        ItemKind::Reasoning { .. } => "reasoning",
        ItemKind::NativeTool { .. } => "native_tool",
        ItemKind::Compaction { .. } => "compaction",
        ItemKind::SlashCommand { .. } => "slash_command",
        ItemKind::TerminalCommand { .. } => "terminal_command",
        ItemKind::Error { .. } => "error",
        ItemKind::ResourceEvent { .. } => "resource_event",
        ItemKind::AgentChanged { .. } => "agent_changed",
    }
}
```

Need `use serde::ser::Error as _;` in scope for `serde::ser::Error::custom` — add it to the `use` block: `use serde::ser::Error as _;`.

- [ ] **Step 3: Run tests + gate**

Run: `cargo test -p lens-core persist::map`
Expected: PASS.
Run: `cargo clippy -p lens-core --all-targets` and `cargo fmt`.

- [ ] **Step 4: Commit**

```bash
git add crates/lens-core/src/persist/map.rs
git commit -m "feat(lens-core): P2 task 2 — column mapping helpers (enum tokens, json cols, kind vocab)"
```

---

## Task 3: `SqliteControlStore` — connections (`persist/control.rs`)

**Files:**
- Create: `crates/lens-core/src/persist/control.rs`
- Test: inline `#[cfg(test)]` in `control.rs`

**Interfaces:**
- Consumes: `db::open_db`, `schema::{CONTROL_DDL, SCHEMA_VERSION}`, `ConnectionRecord`, `ControlStore` trait.
- Produces:
  - `SqliteControlStore::open(path: &Path) -> Result<Self>`
  - `impl ControlStore for SqliteControlStore` (this task: `mode`, `upsert_connection`, `load_connections`; sessions/cost in Tasks 4–5)

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ids::ConnectionId;
    use crate::persist::{ConnectionRecord, ControlStore};
    use tempfile::tempdir;

    fn store() -> (tempfile::TempDir, SqliteControlStore) {
        let dir = tempdir().unwrap();
        let s = SqliteControlStore::open(&dir.path().join("lens.db")).unwrap();
        (dir, s)
    }

    #[test]
    fn connection_upsert_then_load_roundtrips() {
        let (_d, s) = store();
        let c = ConnectionRecord {
            id: ConnectionId::new("conn_1"),
            base_url: "http://localhost:8080".into(),
            auth_kind: "bearer".into(),
            label: Some("Local".into()),
            server_info: Some(r#"{"version":"0.4.0"}"#.into()),
            created_at: 1_700_000_000,
        };
        s.upsert_connection(&c).unwrap();
        // Upsert again with a changed label — no duplicate row, label updated.
        let c2 = ConnectionRecord { label: Some("Local dev".into()), ..c.clone() };
        s.upsert_connection(&c2).unwrap();
        let loaded = s.load_connections().unwrap();
        assert_eq!(loaded, vec![c2]);
    }
}
```

Run: `cargo test -p lens-core persist::control`
Expected: FAIL (module/`SqliteControlStore` not defined).

- [ ] **Step 2: Implement the `open` + connections half of `persist/control.rs`**

```rust
//! `SqliteControlStore` — the control-plane role (`lens.db`): connections,
//! sessions, cost_samples (§6.2). One blocking `rusqlite::Connection`; P3 wraps
//! it in the serialized control-plane writer.

use crate::domain::ids::{ConnectionId, SessionId};
use crate::domain::session::SessionState;
use crate::persist::db::open_db;
use crate::persist::schema::{CONTROL_DDL, SCHEMA_VERSION};
use crate::persist::{ConnectionRecord, ControlStore, PersistError, Result, StoreMode};
use rusqlite::Connection;
use std::path::Path;

pub struct SqliteControlStore {
    conn: Connection,
    mode: StoreMode,
}

impl SqliteControlStore {
    /// Open (creating) the control-plane db at `path`, version-gated (§6.3).
    pub fn open(path: &Path) -> Result<Self> {
        let (conn, mode) = open_db(path, CONTROL_DDL, SCHEMA_VERSION)?;
        Ok(Self { conn, mode })
    }

    fn guard_write(&self) -> Result<()> {
        match self.mode {
            StoreMode::ReadWrite => Ok(()),
            StoreMode::ReadOnlyDegraded => Err(PersistError::ReadOnly),
        }
    }
}

impl ControlStore for SqliteControlStore {
    fn mode(&self) -> StoreMode {
        self.mode
    }

    fn upsert_connection(&self, c: &ConnectionRecord) -> Result<()> {
        self.guard_write()?;
        self.conn.execute(
            "INSERT INTO connections (id, base_url, auth_kind, label, server_info, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET
               base_url=excluded.base_url, auth_kind=excluded.auth_kind,
               label=excluded.label, server_info=excluded.server_info",
            rusqlite::params![
                c.id.as_str(),
                c.base_url,
                c.auth_kind,
                c.label,
                c.server_info,
                c.created_at,
            ],
        )?;
        Ok(())
    }

    fn load_connections(&self) -> Result<Vec<ConnectionRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, base_url, auth_kind, label, server_info, created_at
             FROM connections ORDER BY created_at, id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(ConnectionRecord {
                id: ConnectionId::new(r.get::<_, String>(0)?),
                base_url: r.get(1)?,
                auth_kind: r.get(2)?,
                label: r.get(3)?,
                server_info: r.get(4)?,
                created_at: r.get(5)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
    }

    // sessions + cost_samples: Tasks 4–5.
    fn upsert_session(&self, _s: &SessionState, _now_ms: i64) -> Result<()> {
        unimplemented!("Task 4")
    }
    fn load_session(&self, _conn: &ConnectionId, _id: &SessionId) -> Result<Option<SessionState>> {
        unimplemented!("Task 4")
    }
    fn list_sessions(&self, _conn: &ConnectionId) -> Result<Vec<SessionState>> {
        unimplemented!("Task 4")
    }
    fn insert_cost_sample(
        &self,
        _conn: &ConnectionId,
        _id: &SessionId,
        _sampled_at: i64,
        _total_cost_usd: f64,
    ) -> Result<()> {
        unimplemented!("Task 5")
    }
    fn cost_samples_in(
        &self,
        _conn: &ConnectionId,
        _id: &SessionId,
        _since: i64,
        _until: i64,
    ) -> Result<Vec<(i64, f64)>> {
        unimplemented!("Task 5")
    }
}
```

> Note: the `unimplemented!` stubs exist only so the trait compiles between tasks. They are filled in Tasks 4–5 and MUST be gone (no `unimplemented!`/`todo!` in `lens-core`) before the branch review. Clippy allows `unimplemented!`; the final gate (Task 7) greps for it.

- [ ] **Step 3: Register the module**

`persist/mod.rs` already has `pub mod control;` (Task 1 Step 3). No change.

- [ ] **Step 4: Run tests + gate**

Run: `cargo test -p lens-core persist::control`
Expected: PASS (1 test).
Run: `cargo clippy -p lens-core --all-targets` and `cargo fmt`.

- [ ] **Step 5: Commit**

```bash
git add crates/lens-core/src/persist/control.rs
git commit -m "feat(lens-core): P2 task 3 — SqliteControlStore open + connections upsert/load"
```

---

## Task 4: `SqliteControlStore` — sessions (`persist/control.rs`, `persist/map.rs`)

**Files:**
- Modify: `crates/lens-core/src/persist/control.rs` (replace the 3 session stubs)
- Modify: `crates/lens-core/src/persist/map.rs` (add `session_to_params` + `row_to_session`)

**Interfaces:**
- Consumes: `map::{enum_token, from_token, json_string, from_json}`, the `Cost`/`Usage` types.
- Produces:
  - `impl ControlStore::{upsert_session, load_session, list_sessions}` for `SqliteControlStore`
  - `map::SESSION_COLUMNS: &str` (the shared SELECT column list, so load/list share one parser)
  - `map::row_to_session(r: &rusqlite::Row) -> rusqlite::Result<SessionState>`

- [ ] **Step 1: Write the failing tests (in `control.rs`)**

```rust
    #[test]
    fn session_upsert_then_load_roundtrips_persisted_fields() {
        use crate::domain::ids::{AgentId, ConnectionId, SessionId};
        use crate::domain::item::{BlockContext, ContentBlock, Item, ItemKind};
        use crate::domain::scalars::{Role, SessionStatusValue};
        use crate::domain::usage::{Cost, ModelUsage, Usage};
        use std::collections::BTreeMap;

        let (_d, s) = store();
        // A connection row must exist (FK).
        s.upsert_connection(&ConnectionRecord {
            id: ConnectionId::new("conn_1"),
            base_url: "u".into(),
            auth_kind: "none".into(),
            label: None,
            server_info: None,
            created_at: 1,
        })
        .unwrap();

        let mut st = SessionState::new(
            ConnectionId::new("conn_1"),
            SessionId::new("conv_1"),
            AgentId::new("agent_1"),
        );
        st.status = SessionStatusValue::Running;
        st.title = Some("t".into());
        st.labels.insert("env".into(), "prod".into());
        st.terminal_pending = true;
        st.last_total_tokens = Some(1234);
        st.context_window = Some(200_000);
        let mut by_model = BTreeMap::new();
        by_model.insert("opus".to_string(), ModelUsage { input_tokens: Some(3), ..Default::default() });
        st.cumulative_cost = Cost {
            cumulative_usage: Usage { input_tokens: 3, output_tokens: 4, total_tokens: 7, usage_by_model: by_model, ..Default::default() },
            total_cost_usd: Some(0.5),
        };
        // items are NOT persisted here (they live in the transcript file, D-P2-6).
        st.items.push(Item {
            id: crate::domain::ids::ItemId::new("item_1"),
            seq: None,
            ctx: BlockContext { agent: None, depth: 0, turn: 0 },
            created_at: 1,
            kind: ItemKind::Message { role: Role::User, content: vec![ContentBlock { kind: "text".into(), text: Some("x".into()), data: serde_json::Value::Null }] },
        });

        s.upsert_session(&st, 1_700_000_000_000).unwrap();
        let loaded = s
            .load_session(&ConnectionId::new("conn_1"), &SessionId::new("conv_1"))
            .unwrap()
            .expect("row present");

        // Persisted fields survive; items are empty on load (D-P2-6).
        assert_eq!(loaded.status, SessionStatusValue::Running);
        assert_eq!(loaded.title.as_deref(), Some("t"));
        assert_eq!(loaded.labels.get("env").map(String::as_str), Some("prod"));
        assert!(loaded.terminal_pending);
        assert_eq!(loaded.last_total_tokens, Some(1234));
        // Cost is lossless via cost_json (D-P2-2).
        assert_eq!(loaded.cumulative_cost, st.cumulative_cost);
        assert!(loaded.items.is_empty());
        // RAM-only fields are defaulted on load.
        assert!(loaded.presence.is_empty());
        assert!(loaded.pending_user.is_empty());
    }

    #[test]
    fn upsert_preserves_store_managed_columns() {
        use crate::domain::ids::{AgentId, ConnectionId, SessionId};

        let (_d, s) = store();
        s.upsert_connection(&ConnectionRecord {
            id: ConnectionId::new("conn_1"), base_url: "u".into(), auth_kind: "none".into(),
            label: None, server_info: None, created_at: 1,
        }).unwrap();
        let st = SessionState::new(ConnectionId::new("conn_1"), SessionId::new("conv_1"), AgentId::new("a"));
        s.upsert_session(&st, 10).unwrap();
        // Simulate P3/§9 writes to ALL THREE store-managed columns (D-P2-4).
        s.conn.execute(
            "UPDATE sessions SET pinned = 1, last_status = 'waiting', tombstoned_at = 999 WHERE id = 'conv_1'",
            [],
        ).unwrap();
        // A later reducer fold re-upserts the session — must NOT clobber them.
        s.upsert_session(&st, 20).unwrap();
        let pinned: i64 = s.conn.query_row("SELECT pinned FROM sessions WHERE id='conv_1'", [], |r| r.get(0)).unwrap();
        let last_status: Option<String> = s.conn.query_row("SELECT last_status FROM sessions WHERE id='conv_1'", [], |r| r.get(0)).unwrap();
        let tombstoned_at: Option<i64> = s.conn.query_row("SELECT tombstoned_at FROM sessions WHERE id='conv_1'", [], |r| r.get(0)).unwrap();
        let updated_at: i64 = s.conn.query_row("SELECT updated_at FROM sessions WHERE id='conv_1'", [], |r| r.get(0)).unwrap();
        assert_eq!(pinned, 1);
        assert_eq!(last_status.as_deref(), Some("waiting"));
        assert_eq!(tombstoned_at, Some(999));
        assert_eq!(updated_at, 20); // store-managed, always written
    }

    #[test]
    fn enum_columns_store_bare_unquoted_tokens_for_bridge() {
        // D-P2-8/D-P2-9: Bridge reads `status`/`host_type`/`lifecycle`/`items.kind`
        // as bare tokens, NOT json-quoted. Pin the raw cell (roundtrip tests alone
        // would pass even if a stray-quoted token were stored).
        use crate::domain::ids::{AgentId, ConnectionId, SessionId};
        use crate::domain::scalars::SessionStatusValue;
        let (_d, s) = store();
        s.upsert_connection(&ConnectionRecord {
            id: ConnectionId::new("conn_1"), base_url: "u".into(), auth_kind: "none".into(),
            label: None, server_info: None, created_at: 1,
        }).unwrap();
        let mut st = SessionState::new(ConnectionId::new("conn_1"), SessionId::new("conv_1"), AgentId::new("a"));
        st.status = SessionStatusValue::Waiting;
        s.upsert_session(&st, 1).unwrap();
        let status: String = s.conn.query_row("SELECT status FROM sessions WHERE id='conv_1'", [], |r| r.get(0)).unwrap();
        let host_type: String = s.conn.query_row("SELECT host_type FROM sessions WHERE id='conv_1'", [], |r| r.get(0)).unwrap();
        let lifecycle: String = s.conn.query_row("SELECT lifecycle FROM sessions WHERE id='conv_1'", [], |r| r.get(0)).unwrap();
        assert_eq!(status, "waiting");     // not "\"waiting\""
        assert_eq!(host_type, "external");
        assert_eq!(lifecycle, "active");
    }
```

Run: `cargo test -p lens-core persist::control`
Expected: FAIL (`unimplemented!` panics).

- [ ] **Step 2: Add `row_to_session` + `SESSION_COLUMNS` to `persist/map.rs`**

```rust
use crate::domain::ids::{AgentId, ConnectionId, HostId, RunnerId, SessionId};
use crate::domain::scalars::{ErrorInfo, HostType, SessionLifecycle, SessionStatusValue};
use crate::domain::session::SessionState;
use crate::domain::usage::Cost;
use std::collections::BTreeMap;

/// The `sessions` SELECT column list — shared by `load_session` + `list_sessions`
/// so both feed one `row_to_session`. Order MUST match `row_to_session`'s `get(n)`.
pub const SESSION_COLUMNS: &str = "connection_id, id, agent_id, agent_name, runner_id, \
    parent_session_id, status, last_task_error, llm_model, model_override, reasoning_effort, \
    collaboration_mode, context_window, last_total_tokens, cost_json, workspace, git_branch, \
    host_type, host_id, title, labels, permission_level, owner, todos, skills, terminal_pending, \
    created_at, archived, lifecycle, last_focused_at, last_seen_seq";

/// Reconstruct a disk-snapshot `SessionState` (items empty; RAM-only fields
/// defaulted — D-P2-6). Total over decodable rows (never panics on disk data).
pub fn row_to_session(r: &rusqlite::Row) -> rusqlite::Result<SessionState> {
    // Lift a decode error out of a rusqlite row closure. NOTE (REVIEW#8): a
    // serde_json/enum decode failure is surfaced as `rusqlite::Error::
    // FromSqlConversionFailure`, which `?` then converts to `PersistError::Sqlite`
    // — NOT `PersistError::Json`. Totality is preserved (never a panic); callers
    // must not rely on the `Json` variant to distinguish a decode failure here.
    fn to_sql_err<E: std::error::Error + Send + Sync + 'static>(e: E) -> rusqlite::Error {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    }

    let connection_id = ConnectionId::new(r.get::<_, String>(0)?);
    let id = SessionId::new(r.get::<_, String>(1)?);
    let agent_id = AgentId::new(r.get::<_, String>(2)?);
    let mut st = SessionState::new(connection_id, id, agent_id);

    st.agent_name = r.get(3)?;
    st.runner_id = r.get::<_, Option<String>>(4)?.map(RunnerId::new);
    st.parent_session_id = r.get::<_, Option<String>>(5)?.map(SessionId::new);
    st.status = from_token::<SessionStatusValue>(r.get::<_, String>(6)?).map_err(to_sql_err)?;
    st.last_task_error = match r.get::<_, Option<String>>(7)? {
        Some(j) => Some(from_json::<ErrorInfo>(&j).map_err(to_sql_err)?),
        None => None,
    };
    st.llm_model = r.get(8)?;
    st.model_override = r.get(9)?;
    st.reasoning_effort = r.get(10)?;
    st.collaboration_mode = r.get(11)?;
    // REVIEW#6: read unsigned columns through i64 uniformly (like last_seen_seq /
    // permission_level) so a high-bit value loads (rusqlite's u64 FromSql errors
    // on > i64::MAX) — keeps loads total.
    st.context_window = r.get::<_, Option<i64>>(12)?.map(|v| v as u64);
    st.last_total_tokens = r.get::<_, Option<i64>>(13)?.map(|v| v as u64);
    st.cumulative_cost = match r.get::<_, Option<String>>(14)? {
        Some(j) => from_json::<Cost>(&j).map_err(to_sql_err)?,
        None => Cost::default(),
    };
    st.workspace = r.get(15)?;
    st.git_branch = r.get(16)?;
    st.host_type = from_token::<HostType>(r.get::<_, String>(17)?).map_err(to_sql_err)?;
    st.host_id = r.get::<_, Option<String>>(18)?.map(HostId::new);
    st.title = r.get(19)?;
    st.labels = match r.get::<_, Option<String>>(20)? {
        Some(j) => from_json::<BTreeMap<String, String>>(&j).map_err(to_sql_err)?,
        None => BTreeMap::new(),
    };
    st.permission_level = r.get::<_, Option<i64>>(21)?.map(|v| v as u8);
    st.owner = r.get(22)?;
    st.todos = match r.get::<_, Option<String>>(23)? {
        Some(j) => from_json(&j).map_err(to_sql_err)?,
        None => Vec::new(),
    };
    st.skills = match r.get::<_, Option<String>>(24)? {
        Some(j) => from_json(&j).map_err(to_sql_err)?,
        None => Vec::new(),
    };
    st.terminal_pending = r.get::<_, i64>(25)? != 0;
    st.created_at = r.get(26)?;
    st.archived = r.get::<_, i64>(27)? != 0;
    st.lifecycle = from_token::<SessionLifecycle>(r.get::<_, String>(28)?).map_err(to_sql_err)?;
    st.last_focused_at = r.get(29)?;
    st.last_seen_seq = r.get::<_, Option<i64>>(30)?.map(|v| v as u64);
    // items, presence, stream, pending_user, model_options, sandbox_status,
    // pending_elicitations: NOT persisted (D-P2-5/D-P2-6) — left at `new()` defaults.
    Ok(st)
}
```

- [ ] **Step 3: Implement the three session methods in `control.rs`** (replace the stubs)

```rust
    fn upsert_session(&self, s: &SessionState, now_ms: i64) -> Result<()> {
        use crate::persist::map::{enum_token, json_string};
        self.guard_write()?;

        let status = enum_token(&s.status)?;
        let host_type = enum_token(&s.host_type)?;
        let lifecycle = enum_token(&s.lifecycle)?;
        let last_task_error = s.last_task_error.as_ref().map(json_string).transpose()?;
        let cost_json = json_string(&s.cumulative_cost)?;
        let usage_by_model = json_string(&s.cumulative_cost.cumulative_usage.usage_by_model)?;
        let labels = json_string(&s.labels)?;
        let todos = json_string(&s.todos)?;
        let skills = json_string(&s.skills)?;

        // INSERT sets store-managed columns to their defaults; ON CONFLICT UPDATE
        // OMITS pinned/last_status/tombstoned_at so a P3/§9 write survives (D-P2-4).
        // updated_at is store-managed and always written.
        self.conn.execute(
            "INSERT INTO sessions (
               connection_id, id, agent_id, agent_name, runner_id, parent_session_id,
               status, last_task_error, llm_model, model_override, reasoning_effort,
               collaboration_mode, context_window, last_total_tokens, cumulative_cost,
               usage_by_model, cost_json, workspace, git_branch, host_type, host_id,
               title, labels, permission_level, owner, todos, skills, terminal_pending,
               created_at, archived, lifecycle, last_focused_at, last_seen_seq, updated_at
             ) VALUES (
               ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17,
               ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32, ?33, ?34
             )
             ON CONFLICT(connection_id, id) DO UPDATE SET
               agent_id=excluded.agent_id, agent_name=excluded.agent_name,
               runner_id=excluded.runner_id, parent_session_id=excluded.parent_session_id,
               status=excluded.status, last_task_error=excluded.last_task_error,
               llm_model=excluded.llm_model, model_override=excluded.model_override,
               reasoning_effort=excluded.reasoning_effort, collaboration_mode=excluded.collaboration_mode,
               context_window=excluded.context_window, last_total_tokens=excluded.last_total_tokens,
               cumulative_cost=excluded.cumulative_cost, usage_by_model=excluded.usage_by_model,
               cost_json=excluded.cost_json, workspace=excluded.workspace, git_branch=excluded.git_branch,
               host_type=excluded.host_type, host_id=excluded.host_id, title=excluded.title,
               labels=excluded.labels, permission_level=excluded.permission_level, owner=excluded.owner,
               todos=excluded.todos, skills=excluded.skills, terminal_pending=excluded.terminal_pending,
               created_at=excluded.created_at, archived=excluded.archived, lifecycle=excluded.lifecycle,
               last_focused_at=excluded.last_focused_at, last_seen_seq=excluded.last_seen_seq,
               updated_at=excluded.updated_at",
            rusqlite::params![
                s.connection_id.as_str(), s.id.as_str(), s.agent_id.as_str(), s.agent_name,
                s.runner_id.as_ref().map(|v| v.as_str()),
                s.parent_session_id.as_ref().map(|v| v.as_str()),
                status, last_task_error, s.llm_model, s.model_override, s.reasoning_effort,
                s.collaboration_mode, s.context_window.map(|v| v as i64),
                s.last_total_tokens.map(|v| v as i64),
                s.cumulative_cost.total_cost_usd, usage_by_model, cost_json, s.workspace,
                s.git_branch, host_type, s.host_id.as_ref().map(|v| v.as_str()), s.title, labels,
                s.permission_level, s.owner, todos, skills, s.terminal_pending as i64,
                s.created_at, s.archived as i64, lifecycle, s.last_focused_at,
                s.last_seen_seq.map(|v| v as i64), now_ms,
            ],
        )?;
        Ok(())
    }

    fn load_session(&self, conn: &ConnectionId, id: &SessionId) -> Result<Option<SessionState>> {
        use crate::persist::map::{SESSION_COLUMNS, row_to_session};
        let sql = format!(
            "SELECT {SESSION_COLUMNS} FROM sessions WHERE connection_id = ?1 AND id = ?2"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let mut rows = stmt.query(rusqlite::params![conn.as_str(), id.as_str()])?;
        match rows.next()? {
            Some(r) => Ok(Some(row_to_session(r)?)),
            None => Ok(None),
        }
    }

    fn list_sessions(&self, conn: &ConnectionId) -> Result<Vec<SessionState>> {
        use crate::persist::map::{SESSION_COLUMNS, row_to_session};
        let sql = format!(
            "SELECT {SESSION_COLUMNS} FROM sessions WHERE connection_id = ?1 \
             ORDER BY last_focused_at DESC, id"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params![conn.as_str()], |r| row_to_session(r))?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
    }
```

- [ ] **Step 4: Run tests + gate**

Run: `cargo test -p lens-core persist::control`
Expected: PASS (3 tests total).
Run: `cargo clippy -p lens-core --all-targets` and `cargo fmt`.

- [ ] **Step 5: Commit**

```bash
git add crates/lens-core/src/persist/control.rs crates/lens-core/src/persist/map.rs
git commit -m "feat(lens-core): P2 task 4 — sessions upsert (store-managed preserve) + load/list"
```

---

## Task 5: `SqliteControlStore` — cost_samples (`persist/control.rs`)

**Files:**
- Modify: `crates/lens-core/src/persist/control.rs` (replace the 2 cost stubs)

**Interfaces:**
- Produces: `impl ControlStore::{insert_cost_sample, cost_samples_in}` for `SqliteControlStore`

- [ ] **Step 1: Write the failing test (in `control.rs`)**

```rust
    #[test]
    fn cost_samples_insert_and_window_query() {
        use crate::domain::ids::{ConnectionId, SessionId};
        let (_d, s) = store();
        let conn = ConnectionId::new("conn_1");
        let sid = SessionId::new("conv_1");
        s.insert_cost_sample(&conn, &sid, 100, 1.0).unwrap();
        s.insert_cost_sample(&conn, &sid, 200, 2.5).unwrap();
        s.insert_cost_sample(&conn, &sid, 300, 4.0).unwrap();
        // Re-inserting the same sampled_at is idempotent (PK), value updated.
        s.insert_cost_sample(&conn, &sid, 300, 4.2).unwrap();
        let window = s.cost_samples_in(&conn, &sid, 150, 300).unwrap();
        assert_eq!(window, vec![(200, 2.5), (300, 4.2)]);
    }
```

Run: `cargo test -p lens-core persist::control::tests::cost_samples`
Expected: FAIL (`unimplemented!`).

- [ ] **Step 2: Implement (replace the stubs)**

```rust
    fn insert_cost_sample(
        &self,
        conn: &ConnectionId,
        id: &SessionId,
        sampled_at: i64,
        total_cost_usd: f64,
    ) -> Result<()> {
        self.guard_write()?;
        self.conn.execute(
            "INSERT INTO cost_samples (connection_id, session_id, sampled_at, total_cost_usd)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(connection_id, session_id, sampled_at)
               DO UPDATE SET total_cost_usd = excluded.total_cost_usd",
            rusqlite::params![conn.as_str(), id.as_str(), sampled_at, total_cost_usd],
        )?;
        Ok(())
    }

    fn cost_samples_in(
        &self,
        conn: &ConnectionId,
        id: &SessionId,
        since: i64,
        until: i64,
    ) -> Result<Vec<(i64, f64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT sampled_at, total_cost_usd FROM cost_samples
             WHERE connection_id = ?1 AND session_id = ?2 AND sampled_at BETWEEN ?3 AND ?4
             ORDER BY sampled_at",
        )?;
        let rows = stmt.query_map(
            rusqlite::params![conn.as_str(), id.as_str(), since, until],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
    }
```

- [ ] **Step 3: Run tests + gate**

Run: `cargo test -p lens-core persist::control`
Expected: PASS (4 tests). No `unimplemented!` remains in `control.rs`.
Run: `cargo clippy -p lens-core --all-targets` and `cargo fmt`.

- [ ] **Step 4: Commit**

```bash
git add crates/lens-core/src/persist/control.rs
git commit -m "feat(lens-core): P2 task 5 — cost_samples insert + window query"
```

---

## Task 6: `SqliteTranscriptStore` — items + reconcile (`persist/transcript.rs`, `persist/map.rs`)

**Files:**
- Create: `crates/lens-core/src/persist/transcript.rs`
- Modify: `crates/lens-core/src/persist/map.rs` (add `item_to_columns` + `row_to_item`)

**Interfaces:**
- Consumes: `db::open_db`, `schema::{TRANSCRIPT_DDL, SCHEMA_VERSION}`, `map::{item_kind_token, json_string, from_json}`, `TranscriptStore` trait.
- Produces:
  - `SqliteTranscriptStore::open(path, conn_id, session_id) -> Result<Self>` (stamps the self-describing meta on a fresh file; verifies it on an existing one)
  - `impl TranscriptStore for SqliteTranscriptStore`
  - `map::row_to_item(r: &rusqlite::Row) -> rusqlite::Result<Item>`

- [ ] **Step 1: Write the failing tests (in `transcript.rs`)**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ids::{ConnectionId, ItemId, SessionId};
    use crate::domain::item::{BlockContext, ContentBlock, Item, ItemKind};
    use crate::domain::scalars::Role;
    use crate::persist::TranscriptStore;
    use tempfile::tempdir;

    fn item(id: &str, turn: u32, text: &str) -> Item {
        Item {
            id: ItemId::new(id),
            seq: Some(1),
            ctx: BlockContext { agent: Some("coder".into()), depth: 0, turn },
            created_at: 1_700_000_000_000,
            kind: ItemKind::Message {
                role: Role::Assistant,
                content: vec![ContentBlock { kind: "text".into(), text: Some(text.into()), data: serde_json::Value::Null }],
            },
        }
    }

    fn store(dir: &std::path::Path) -> SqliteTranscriptStore {
        SqliteTranscriptStore::open(
            &dir.join("conv_1.db"),
            &ConnectionId::new("conn_1"),
            &SessionId::new("conv_1"),
        ).unwrap()
    }

    #[test]
    fn upsert_items_then_load_ordered_and_self_describing() {
        let d = tempdir().unwrap();
        let s = store(d.path());
        s.upsert_item(0, &item("item_a", 0, "a")).unwrap();
        s.upsert_item(1, &item("item_b", 0, "b")).unwrap();
        // Re-upsert item_a at ordinal 0 with edited text — no dup, payload updated.
        s.upsert_item(0, &item("item_a", 0, "a-edited")).unwrap();
        let items = s.load_items().unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].id.as_str(), "item_a");
        assert_eq!(items[1].id.as_str(), "item_b");
        match &items[0].kind {
            ItemKind::Message { content, .. } => assert_eq!(content[0].text.as_deref(), Some("a-edited")),
            _ => panic!("wrong kind"),
        }
        assert_eq!(items[0].ctx.agent.as_deref(), Some("coder"));
        assert_eq!(s.identity().unwrap(), (ConnectionId::new("conn_1"), SessionId::new("conv_1")));
    }

    #[test]
    fn reconcile_matches_server_truth_by_id() {
        let d = tempdir().unwrap();
        let s = store(d.path());
        // Disk has a, b, c.
        s.upsert_item(0, &item("item_a", 0, "a")).unwrap();
        s.upsert_item(1, &item("item_b", 0, "b")).unwrap();
        s.upsert_item(2, &item("item_c", 0, "c")).unwrap();
        // Server truth: b edited, c dropped (compaction), d appended, a kept.
        let truth = vec![
            item("item_a", 0, "a"),
            item("item_b", 1, "b-edited"),
            item("item_d", 1, "d"),
        ];
        s.reconcile(&truth).unwrap();
        let items = s.load_items().unwrap();
        let ids: Vec<_> = items.iter().map(|i| i.id.as_str().to_string()).collect();
        assert_eq!(ids, vec!["item_a", "item_b", "item_d"]); // c gone, order = truth
        match &items[1].kind {
            ItemKind::Message { content, .. } => assert_eq!(content[0].text.as_deref(), Some("b-edited")),
            _ => panic!(),
        }
        assert_eq!(items[1].ctx.turn, 1);
    }
}
```

Run: `cargo test -p lens-core persist::transcript`
Expected: FAIL (module not defined).

- [ ] **Step 2: Add `row_to_item` to `persist/map.rs`**

```rust
use crate::domain::ids::ItemId;
use crate::domain::item::{BlockContext, Item, ItemKind};

/// Reconstruct an `Item` from a transcript row. `payload` alone carries the full
/// tagged `ItemKind` (D-P2-9); `ordinal`/`kind` columns are read-contract only.
/// Total over decodable rows. Column order: item_id, live_seq, kind, payload,
/// agent, depth, turn, created_at.
pub fn row_to_item(r: &rusqlite::Row) -> rusqlite::Result<Item> {
    fn to_sql_err<E: std::error::Error + Send + Sync + 'static>(e: E) -> rusqlite::Error {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    }
    let payload: String = r.get(3)?;
    let kind: ItemKind = from_json(&payload).map_err(to_sql_err)?;
    Ok(Item {
        id: ItemId::new(r.get::<_, String>(0)?),
        seq: r.get::<_, Option<i64>>(1)?.map(|v| v as u64),
        ctx: BlockContext {
            agent: r.get(4)?,
            depth: r.get::<_, i64>(5)? as u32,
            turn: r.get::<_, i64>(6)? as u32,
        },
        created_at: r.get(7)?,
        kind,
    })
}
```

- [ ] **Step 3: Implement `persist/transcript.rs`**

```rust
//! `SqliteTranscriptStore` — the per-session role: one file per (connection,
//! session), holding only that session's `items` (§6.2). The actor owns this
//! file's WAL write connection (P3) — no cross-actor contention. The file is
//! self-describing: its `meta` carries schema_version + (connection_id, session_id).

use crate::domain::ids::{ConnectionId, SessionId};
use crate::domain::item::Item;
use crate::persist::db::open_db;
use crate::persist::map::{item_kind_token, json_string, row_to_item};
use crate::persist::schema::{SCHEMA_VERSION, TRANSCRIPT_DDL};
use crate::persist::{PersistError, Result, StoreMode, TranscriptStore};
use rusqlite::Connection;
use std::path::Path;

pub struct SqliteTranscriptStore {
    conn: Connection,
    mode: StoreMode,
}

impl SqliteTranscriptStore {
    /// Open (creating) the transcript file at `path`. On a fresh file, stamp
    /// `connection_id`/`session_id` into `meta` (self-describing, §6.2). On an
    /// existing file, this is idempotent — the ids are already recorded.
    pub fn open(path: &Path, conn_id: &ConnectionId, session_id: &SessionId) -> Result<Self> {
        let (conn, mode) = open_db(path, TRANSCRIPT_DDL, SCHEMA_VERSION)?;
        if mode == StoreMode::ReadWrite {
            conn.execute(
                "INSERT INTO meta (key, value) VALUES ('connection_id', ?1)
                 ON CONFLICT(key) DO NOTHING",
                [conn_id.as_str()],
            )?;
            conn.execute(
                "INSERT INTO meta (key, value) VALUES ('session_id', ?1)
                 ON CONFLICT(key) DO NOTHING",
                [session_id.as_str()],
            )?;
        }
        Ok(Self { conn, mode })
    }

    fn guard_write(&self) -> Result<()> {
        match self.mode {
            StoreMode::ReadWrite => Ok(()),
            StoreMode::ReadOnlyDegraded => Err(PersistError::ReadOnly),
        }
    }

    fn upsert_item_stmt(&self, ordinal: i64, item: &Item) -> Result<()> {
        let payload = json_string(&item.kind)?;
        let kind = item_kind_token(&item.kind);
        self.conn.execute(
            "INSERT INTO items (item_id, live_seq, ordinal, kind, payload, agent, depth, turn, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(item_id) DO UPDATE SET
               live_seq=excluded.live_seq, ordinal=excluded.ordinal, kind=excluded.kind,
               payload=excluded.payload, agent=excluded.agent, depth=excluded.depth,
               turn=excluded.turn, created_at=excluded.created_at",
            rusqlite::params![
                item.id.as_str(),
                item.seq.map(|v| v as i64),
                ordinal,
                kind,
                payload,
                item.ctx.agent,
                item.ctx.depth as i64,
                item.ctx.turn as i64,
                item.created_at,
            ],
        )?;
        Ok(())
    }
}

impl TranscriptStore for SqliteTranscriptStore {
    fn mode(&self) -> StoreMode {
        self.mode
    }

    fn identity(&self) -> Result<(ConnectionId, SessionId)> {
        let get = |key: &str| -> Result<String> {
            Ok(self.conn.query_row(
                "SELECT value FROM meta WHERE key = ?1",
                [key],
                |r| r.get(0),
            )?)
        };
        Ok((ConnectionId::new(get("connection_id")?), SessionId::new(get("session_id")?)))
    }

    /// PRECONDITION (D-P2-7): `ordinal` is a FRESH append position (the item's
    /// index in the actor's canonical `Vec<Item>`). Conflicts resolve on `item_id`
    /// only — reusing an ordinal for a *different* `item_id` raises `UNIQUE(ordinal)`
    /// (a non-panic `Err`). P3 routes any replace/reorder through `reconcile`, not here.
    fn upsert_item(&self, ordinal: i64, item: &Item) -> Result<()> {
        self.guard_write()?;
        self.upsert_item_stmt(ordinal, item)
    }

    fn load_items(&self) -> Result<Vec<Item>> {
        let mut stmt = self.conn.prepare(
            "SELECT item_id, live_seq, kind, payload, agent, depth, turn, created_at
             FROM items ORDER BY ordinal",
        )?;
        let rows = stmt.query_map([], |r| row_to_item(r))?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
    }

    fn reconcile(&self, items: &[Item]) -> Result<()> {
        self.guard_write()?;
        // Wrap in a single transaction: make the file match `items` exactly.
        // ordinal UNIQUE forbids two rows sharing an ordinal mid-update, so clear
        // ordinals to a disjoint negative range first, then re-stamp 0..n.
        self.conn.execute("BEGIN", [])?;
        let result = (|| -> Result<()> {
            // Park existing ordinals out of the way (negative = never a real ordinal).
            self.conn.execute("UPDATE items SET ordinal = -1 - ordinal", [])?;
            // Upsert every truth item at its canonical index.
            for (i, item) in items.iter().enumerate() {
                self.upsert_item_stmt(i as i64, item)?;
            }
            // Delete anything the upserts did not touch (ordinal still negative).
            self.conn.execute("DELETE FROM items WHERE ordinal < 0", [])?;
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.conn.execute("COMMIT", [])?;
                Ok(())
            }
            Err(e) => {
                let _ = self.conn.execute("ROLLBACK", []);
                Err(e)
            }
        }
    }
}

```

> Implementation note for the engineer: the `use` block imports only what
> `transcript.rs` itself references (`item_kind_token`/`json_string`/`row_to_item`);
> `from_json` lives in and is used by `map.rs`, so it is deliberately NOT imported
> here (REVIEW#4 — a dead import fails the zero-warning gate). If `cargo`/clippy
> flags any import as unused, delete it — zero warnings is the gate.

- [ ] **Step 4: Run tests + gate**

Run: `cargo test -p lens-core persist::transcript`
Expected: PASS (2 tests).
Run: `cargo clippy -p lens-core --all-targets` (zero warnings) and `cargo fmt`.

- [ ] **Step 5: Commit**

```bash
git add crates/lens-core/src/persist/transcript.rs crates/lens-core/src/persist/map.rs
git commit -m "feat(lens-core): P2 task 6 — SqliteTranscriptStore items upsert/load + reconcile-by-id"
```

---

## Task 7: Cross-store gate — degraded writes, lifecycle open/close, throughput bench

**Files:**
- Create: `crates/lens-core/benches/persist_throughput.rs`
- Modify: `crates/lens-core/Cargo.toml` (add `[[bench]]`)
- Test: a new integration test file `crates/lens-core/tests/persist_lifecycle.rs`

**Interfaces:**
- Consumes: the full `persist` public surface.

- [ ] **Step 1: Write the degraded-write + lifecycle integration tests**

Create `crates/lens-core/tests/persist_lifecycle.rs`:

```rust
//! P2 cross-store gate: schema-version degrade refuses writes on BOTH stores, and
//! a transcript file survives a full open→write→drop→reopen→load cycle (the Active
//! lifecycle open/close the P2 gate requires).

use lens_core::domain::ids::{ConnectionId, ItemId, SessionId};
use lens_core::domain::item::{BlockContext, ContentBlock, Item, ItemKind};
use lens_core::domain::scalars::Role;
use lens_core::persist::schema::SCHEMA_VERSION;
use lens_core::persist::{
    ControlStore, SqliteControlStore, SqliteTranscriptStore, StoreMode, TranscriptStore,
};
use rusqlite::Connection;
use tempfile::tempdir;

fn msg(id: &str) -> Item {
    Item {
        id: ItemId::new(id),
        seq: None,
        ctx: BlockContext { agent: None, depth: 0, turn: 0 },
        created_at: 1,
        kind: ItemKind::Message {
            role: Role::User,
            content: vec![ContentBlock { kind: "text".into(), text: Some("hi".into()), data: serde_json::Value::Null }],
        },
    }
}

#[test]
fn transcript_survives_close_and_reopen() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("conv_1.db");
    let conn = ConnectionId::new("conn_1");
    let sid = SessionId::new("conv_1");
    {
        let s = SqliteTranscriptStore::open(&path, &conn, &sid).unwrap();
        s.upsert_item(0, &msg("item_1")).unwrap();
        // store dropped here — file closed.
    }
    let s = SqliteTranscriptStore::open(&path, &conn, &sid).unwrap();
    let items = s.load_items().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].id.as_str(), "item_1");
    assert_eq!(s.identity().unwrap(), (conn, sid));
}

#[test]
fn future_schema_version_refuses_writes_on_both_stores() {
    let dir = tempdir().unwrap();

    // Control store.
    let cpath = dir.path().join("lens.db");
    drop(SqliteControlStore::open(&cpath).unwrap()); // create at current version
    bump_version(&cpath);
    let cs = SqliteControlStore::open(&cpath).unwrap();
    assert_eq!(cs.mode(), StoreMode::ReadOnlyDegraded);
    assert!(cs.load_connections().is_ok()); // reads still work
    // A write is refused, not a panic.
    let err = cs.insert_cost_sample(&ConnectionId::new("c"), &SessionId::new("s"), 1, 1.0);
    assert!(err.is_err());

    // Transcript store.
    let tpath = dir.path().join("t.db");
    drop(SqliteTranscriptStore::open(&tpath, &ConnectionId::new("c"), &SessionId::new("s")).unwrap());
    bump_version(&tpath);
    let ts = SqliteTranscriptStore::open(&tpath, &ConnectionId::new("c"), &SessionId::new("s")).unwrap();
    assert_eq!(ts.mode(), StoreMode::ReadOnlyDegraded);
    assert!(ts.load_items().is_ok());
    assert!(ts.upsert_item(0, &msg("x")).is_err());
}

fn bump_version(path: &std::path::Path) {
    let c = Connection::open(path).unwrap();
    c.execute(
        "UPDATE meta SET value = ?1 WHERE key = 'schema_version'",
        [(SCHEMA_VERSION + 1).to_string()],
    )
    .unwrap();
}
```

Run: `cargo test -p lens-core --test persist_lifecycle`
Expected: PASS (2 tests). (If `lens_core::domain`/`persist::schema` paths are not public, add the needed `pub` re-exports to `lib.rs` — `pub mod persist;` already exposes `persist::schema`; ensure `domain` is `pub`.)

- [ ] **Step 2: Add the throughput bench**

Create `crates/lens-core/benches/persist_throughput.rs`:

```rust
//! Persistence throughput (AGENTS.md benchmark-or-it's-not-done). Persistence is
//! I/O-bound; this measures the write-through + reload cost over a realistic item
//! count so regressions are visible, not to claim a CPU budget.

use criterion::{Criterion, criterion_group, criterion_main};
use lens_core::domain::ids::{ConnectionId, ItemId, SessionId};
use lens_core::domain::item::{BlockContext, ContentBlock, Item, ItemKind};
use lens_core::domain::scalars::Role;
use lens_core::persist::{SqliteTranscriptStore, TranscriptStore};
use std::hint::black_box;

fn item(i: usize) -> Item {
    Item {
        id: ItemId::new(format!("item_{i}")),
        seq: Some(i as u64),
        ctx: BlockContext { agent: None, depth: 0, turn: 0 },
        created_at: 1_700_000_000_000,
        kind: ItemKind::Message {
            role: Role::Assistant,
            content: vec![ContentBlock { kind: "text".into(), text: Some("lorem ipsum dolor".into()), data: serde_json::Value::Null }],
        },
    }
}

fn bench_transcript(c: &mut Criterion) {
    let items: Vec<Item> = (0..200).map(item).collect();
    c.bench_function("transcript_write_200_then_load", |b| {
        b.iter(|| {
            let dir = tempfile::tempdir().unwrap();
            let s = SqliteTranscriptStore::open(
                &dir.path().join("b.db"),
                &ConnectionId::new("c"),
                &SessionId::new("s"),
            )
            .unwrap();
            for (ord, it) in items.iter().enumerate() {
                s.upsert_item(ord as i64, it).unwrap();
            }
            black_box(s.load_items().unwrap());
        });
    });
}

criterion_group!(benches, bench_transcript);
criterion_main!(benches);
```

Add to `crates/lens-core/Cargo.toml` (`tempfile` must be a dev-dep — Task 1 added it):

```toml
[[bench]]
name = "persist_throughput"
harness = false
```

- [ ] **Step 3: Full gate**

Run:
```bash
cargo test -p lens-core
cargo clippy -p lens-core --all-targets
cargo fmt --check
grep -rn "unimplemented!\|todo!" crates/lens-core/src/persist/ && echo "STUBS REMAIN — fix" || echo "no stubs"
cargo bench -p lens-core --bench persist_throughput -- --warm-up-time 1 --measurement-time 2
```
Expected: all tests PASS; zero clippy warnings; fmt clean; "no stubs"; bench runs and prints a baseline (record it in the commit body / STATUS).

- [ ] **Step 4: Commit**

```bash
git add crates/lens-core/tests/persist_lifecycle.rs crates/lens-core/benches/persist_throughput.rs crates/lens-core/Cargo.toml
git commit -m "test(lens-core): P2 task 7 — degraded-write + lifecycle gate + throughput bench"
```

---

## Self-Review (completed against the spec)

- **§6.1 two-tier + trait abstraction** → Tasks 1/3/6 (`ControlStore`/`TranscriptStore` traits + SQLite impls). D-P2-1.
- **§6.1 portable, denormalized, Bridge-readable schema** → `schema.rs` DDL (Task 1); denormalized `agent`/`depth`/`turn` + stable `kind` vocabulary (Task 6, `item_kind_token`).
- **§6.1 WAL both tiers** → `db::open_db` pragma (Task 1); tested.
- **§6.2 sessions/connections/cost_samples/items/meta** → Tasks 1/3/4/5/6.
- **§6.2 self-describing transcript meta** → `SqliteTranscriptStore::open` + `identity()` (Task 6); tested.
- **§6.3 write-through upsert by item_id** → `upsert_item` (Task 6); by session fields → `upsert_session` (Task 4).
- **§6.3 reconcile by item id** → `reconcile(&[Item])` (Task 6); tested against edit/drop/append truth.
- **§6.3 schema_version gate, unknown → read-only-degraded** → `db::open_db` + `guard_write` (Tasks 1/3/6); tested on both stores (Task 7).
- **§2.5/§4.2 RAM-only exclusions** → no columns for `presence`/`StreamScratch`/`pending_user`; `load_session` defaults them (D-P2-5/6); tested.
- **AGENTS.md never-panic** → all loads total, `from_token`/`from_json` errors surface as `PersistError`, no `unwrap` on disk data; degrade tests (Task 7).
- **AGENTS.md benchmark** → `persist_throughput` (Task 7).
- **Drift decisions** (`cost_json`, `terminal_pending`, store-managed preserve, chrome re-derive) → D-P2-2..D-P2-6, all flagged for the Opus pre-build review.

**Gaps deferred (correctly, per spec):** retention/pruning/tombstone file ops (§15 open q, deferred); the wake wiring that *calls* these primitives (P3); `pinned`/`last_status` *writers* (§9 registry / §10 poll — P2 only reserves + preserves the columns).

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-07-08-state-model-p2-persistence.md`.

**Before build:** one Opus review of this plan (decisions block D-P2-1..9 primarily), per the branch review workflow. Then subagent-driven build (composer-2.5 per task) with an Opus review between tasks and a consolidated Opus end-of-branch review, then ff-merge to main + push (solo-project integration workflow).
