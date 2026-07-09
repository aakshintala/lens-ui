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
        VersionState::Known(v) if v < current_version => {
            (StoreMode::ReadWrite, Some(current_version))
        }
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
        assert_eq!(
            read_schema_version(&conn).unwrap(),
            VersionState::Known(SCHEMA_VERSION)
        );
        // WAL is on.
        let jm: String = conn
            .query_row("PRAGMA journal_mode;", [], |r| r.get(0))
            .unwrap();
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
            conn.execute(
                "UPDATE meta SET value = 'not-a-number' WHERE key = 'schema_version'",
                [],
            )
            .unwrap();
        }
        let (conn, mode) = open_db(&path, DDL, SCHEMA_VERSION).unwrap();
        assert_eq!(mode, StoreMode::ReadOnlyDegraded);
        // Reads still work in degraded mode.
        assert!(
            conn.query_row("SELECT COUNT(*) FROM meta", [], |r| r.get::<_, i64>(0))
                .is_ok()
        );
    }
}
