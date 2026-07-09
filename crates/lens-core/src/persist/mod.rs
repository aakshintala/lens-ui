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

/// A row that failed to decode during a multi-row load. Recorded (not swallowed,
/// not propagated) so one corrupt row degrades to a skip instead of failing the
/// whole load — and stays OBSERVABLE for the caller to surface / re-fetch (§6.3;
/// AGENTS.md "no silent caps"). lens-core has no logger by design; the app layer
/// decides what to do with these.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkippedRow {
    /// The row's identity (`item_id` / session `id`), or `"<unreadable-id>"` if
    /// even the id column could not be read.
    pub id: String,
    /// The decode failure, stringified.
    pub reason: String,
}

/// The outcome of a multi-row load: the decodable rows plus any that were skipped.
/// A genuinely-corrupt row (truncated json, unknown internally-tagged `kind`, …)
/// lands in `skipped`; a real cursor/IO error still fails the whole load (outer
/// `Err`). `skipped` empty ⇒ a fully clean load.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Loaded<T> {
    pub rows: Vec<T>,
    pub skipped: Vec<SkippedRow>,
}

impl<T> Loaded<T> {
    /// True when every row decoded (no corruption).
    pub fn is_clean(&self) -> bool {
        self.skipped.is_empty()
    }
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
    /// All sessions for `conn`, newest-focused first. A row that fails to decode is
    /// skipped (recorded in `Loaded::skipped`), never aborting the whole list.
    fn list_sessions(&self, conn: &ConnectionId) -> Result<Loaded<SessionState>>;
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
    /// All items ordered by `ordinal`. A row that fails to decode is skipped
    /// (recorded in `Loaded::skipped`), never aborting the whole transcript load.
    fn load_items(&self) -> Result<Loaded<Item>>;
    /// Make the file match server truth by `item_id`: upsert each at `ordinal =
    /// index`, delete rows whose id is absent (§6.3 reconcile-by-id).
    fn reconcile(&self, items: &[Item]) -> Result<()>;
}
