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
    pub(crate) conn: Connection,
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
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
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
        let c2 = ConnectionRecord {
            label: Some("Local dev".into()),
            ..c.clone()
        };
        s.upsert_connection(&c2).unwrap();
        let loaded = s.load_connections().unwrap();
        assert_eq!(loaded, vec![c2]);
    }
}
