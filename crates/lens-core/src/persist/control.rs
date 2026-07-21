//! `SqliteControlStore` — the control-plane role (`lens.db`): connections,
//! sessions, cost_samples (§6.2). One blocking `rusqlite::Connection`; P3 wraps
//! it in the serialized control-plane writer.

use crate::domain::ids::{ConnectionId, SessionId};
use crate::domain::session::SessionState;
use crate::persist::db::open_db;
use crate::persist::schema::{CONTROL_DDL, SCHEMA_VERSION};
use crate::persist::{ConnectionRecord, ControlStore, Loaded, PersistError, Result, StoreMode};
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

    // sessions + cost_samples: Task 5 for cost.
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
               created_at, archived, lifecycle, last_focused_at, updated_at
             ) VALUES (
               ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17,
               ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32, ?33
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
               created_at=CASE WHEN sessions.created_at != 0 THEN sessions.created_at ELSE excluded.created_at END, archived=excluded.archived, lifecycle=excluded.lifecycle,
               last_focused_at=excluded.last_focused_at,
               updated_at=excluded.updated_at",
            rusqlite::params![
                s.connection_id.as_str(),
                s.id.as_str(),
                s.agent_id.as_str(),
                s.agent_name,
                s.runner_id.as_ref().map(|v| v.as_str()),
                s.parent_session_id.as_ref().map(|v| v.as_str()),
                status,
                last_task_error,
                s.llm_model,
                s.model_override,
                s.reasoning_effort,
                s.collaboration_mode,
                s.context_window.map(|v| v as i64),
                s.last_total_tokens.map(|v| v as i64),
                s.cumulative_cost.total_cost_usd,
                usage_by_model,
                cost_json,
                s.workspace,
                s.git_branch,
                host_type,
                s.host_id.as_ref().map(|v| v.as_str()),
                s.title,
                labels,
                s.permission_level,
                s.owner,
                todos,
                skills,
                s.terminal_pending as i64,
                s.created_at,
                s.archived as i64,
                lifecycle,
                s.last_focused_at,
                now_ms,
            ],
        )?;
        Ok(())
    }

    fn load_session(&self, conn: &ConnectionId, id: &SessionId) -> Result<Option<SessionState>> {
        use crate::persist::map::{SESSION_COLUMNS, row_to_session};
        let sql =
            format!("SELECT {SESSION_COLUMNS} FROM sessions WHERE connection_id = ?1 AND id = ?2");
        let mut stmt = self.conn.prepare(&sql)?;
        let mut rows = stmt.query(rusqlite::params![conn.as_str(), id.as_str()])?;
        match rows.next()? {
            Some(r) => Ok(Some(row_to_session(r)?)),
            None => Ok(None),
        }
    }

    fn list_sessions(&self, conn: &ConnectionId) -> Result<Loaded<SessionState>> {
        use crate::persist::map::{SESSION_COLUMNS, collect_skipping, row_to_session};
        let sql = format!(
            "SELECT {SESSION_COLUMNS} FROM sessions WHERE connection_id = ?1 \
             ORDER BY last_focused_at DESC, id"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let mut rows = stmt.query(rusqlite::params![conn.as_str()])?;
        // id_col = 1 (`id` is the 2nd column in SESSION_COLUMNS).
        collect_skipping(&mut rows, 1, row_to_session)
    }

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
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
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

    fn conn_record() -> ConnectionRecord {
        ConnectionRecord {
            id: ConnectionId::new("conn_1"),
            base_url: "u".into(),
            auth_kind: "none".into(),
            label: None,
            server_info: None,
            created_at: 1,
        }
    }

    fn session_fixture() -> SessionState {
        use crate::domain::ids::{AgentId, SessionId};
        SessionState::new(
            ConnectionId::new("conn_1"),
            SessionId::new("conv_1"),
            AgentId::new("agent_1"),
        )
    }

    #[test]
    fn upsert_session_keeps_existing_nonzero_created_at() {
        let d = tempdir().unwrap();
        let store = SqliteControlStore::open(&d.path().join("lens.db")).unwrap();
        store.upsert_connection(&conn_record()).unwrap();

        let mut s = session_fixture();
        s.created_at = 1_700_000_000;
        store.upsert_session(&s, 1).unwrap();

        // A later actor upsert with a not-yet-bootstrapped created_at=0 must NOT clobber.
        s.created_at = 0;
        store.upsert_session(&s, 2).unwrap();

        let loaded = store
            .load_session(&s.connection_id, &s.id)
            .unwrap()
            .unwrap();
        assert_eq!(
            loaded.created_at, 1_700_000_000,
            "non-zero created_at preserved"
        );
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

    #[test]
    fn session_upsert_then_load_roundtrips_persisted_fields() {
        use crate::domain::ids::{AgentId, ConnectionId, SessionId};
        use crate::domain::item::{BlockContext, ContentBlock, Item, ItemKind};
        use crate::domain::scalars::{Role, SessionStatusValue};
        use crate::domain::usage::{Cost, ModelUsage, Usage};
        use std::collections::BTreeMap;
        use std::sync::Arc;

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
        by_model.insert(
            "opus".to_string(),
            ModelUsage {
                input_tokens: Some(3),
                ..Default::default()
            },
        );
        st.cumulative_cost = Cost {
            cumulative_usage: Usage {
                input_tokens: 3,
                output_tokens: 4,
                total_tokens: 7,
                usage_by_model: by_model,
                ..Default::default()
            },
            total_cost_usd: Some(0.5),
        };
        // items are NOT persisted here (they live in the transcript file, D-P2-6).
        st.items.push(Arc::new(Item {
            id: crate::domain::ids::ItemId::new("item_1"),
            seq: None,
            ctx: BlockContext {
                agent: None,
                depth: 0,
                response_id: None,
            },
            created_at: 1,
            kind: ItemKind::Message {
                role: Role::User,
                content: vec![ContentBlock {
                    kind: "text".into(),
                    text: Some("x".into()),
                    data: serde_json::Value::Null,
                }],
            },
        }));

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
            id: ConnectionId::new("conn_1"),
            base_url: "u".into(),
            auth_kind: "none".into(),
            label: None,
            server_info: None,
            created_at: 1,
        })
        .unwrap();
        let st = SessionState::new(
            ConnectionId::new("conn_1"),
            SessionId::new("conv_1"),
            AgentId::new("a"),
        );
        s.upsert_session(&st, 10).unwrap();
        // Simulate P3/§9 writes to ALL THREE store-managed columns (D-P2-4).
        s.conn
            .execute(
                "UPDATE sessions SET pinned = 1, last_status = 'waiting', tombstoned_at = 999 WHERE id = 'conv_1'",
                [],
            )
            .unwrap();
        // A later reducer fold re-upserts the session — must NOT clobber them.
        s.upsert_session(&st, 20).unwrap();
        let pinned: i64 = s
            .conn
            .query_row("SELECT pinned FROM sessions WHERE id='conv_1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        let last_status: Option<String> = s
            .conn
            .query_row(
                "SELECT last_status FROM sessions WHERE id='conv_1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let tombstoned_at: Option<i64> = s
            .conn
            .query_row(
                "SELECT tombstoned_at FROM sessions WHERE id='conv_1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let updated_at: i64 = s
            .conn
            .query_row(
                "SELECT updated_at FROM sessions WHERE id='conv_1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
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
            AgentId::new("a"),
        );
        st.status = SessionStatusValue::Waiting;
        s.upsert_session(&st, 1).unwrap();
        let status: String = s
            .conn
            .query_row("SELECT status FROM sessions WHERE id='conv_1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        let host_type: String = s
            .conn
            .query_row(
                "SELECT host_type FROM sessions WHERE id='conv_1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let lifecycle: String = s
            .conn
            .query_row(
                "SELECT lifecycle FROM sessions WHERE id='conv_1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, "waiting"); // not "\"waiting\""
        assert_eq!(host_type, "external");
        assert_eq!(lifecycle, "active");
    }

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

    #[test]
    fn unknown_host_type_and_lifecycle_tokens_degrade_not_fail_the_list() {
        // Review#end-of-branch: a future/Bridge-written enum token must degrade to
        // `Unknown`, never abort the whole `list_sessions` load (§6.3 / D-P2-8).
        use crate::domain::ids::{AgentId, ConnectionId, SessionId};
        use crate::domain::scalars::{HostType, SessionLifecycle};
        let (_d, s) = store();
        let conn = ConnectionId::new("conn_1");
        s.upsert_connection(&ConnectionRecord {
            id: conn.clone(),
            base_url: "u".into(),
            auth_kind: "none".into(),
            label: None,
            server_info: None,
            created_at: 1,
        })
        .unwrap();
        let st = SessionState::new(conn.clone(), SessionId::new("conv_1"), AgentId::new("a"));
        s.upsert_session(&st, 1).unwrap();
        // Simulate a newer writer / Bridge storing tokens this build doesn't know.
        s.conn
            .execute(
                "UPDATE sessions SET host_type = 'sandboxed', lifecycle = 'snoozed' WHERE id = 'conv_1'",
                [],
            )
            .unwrap();
        let loaded = s.list_sessions(&conn).unwrap(); // MUST NOT Err
        // An unknown ENUM TOKEN decodes to `Unknown` — the row is clean, not skipped.
        assert!(loaded.is_clean());
        assert_eq!(loaded.rows.len(), 1);
        assert_eq!(loaded.rows[0].host_type, HostType::Unknown);
        assert_eq!(loaded.rows[0].lifecycle, SessionLifecycle::Unknown);
    }

    #[test]
    fn corrupt_row_is_skipped_not_fatal_and_good_rows_survive() {
        // A genuinely-undecodable row (malformed json in a NON-degrading column)
        // is skipped + reported, never aborting the whole list (§6.3, deferred-#1).
        use crate::domain::ids::{AgentId, ConnectionId, SessionId};
        let (_d, s) = store();
        let conn = ConnectionId::new("conn_1");
        s.upsert_connection(&ConnectionRecord {
            id: conn.clone(),
            base_url: "u".into(),
            auth_kind: "none".into(),
            label: None,
            server_info: None,
            created_at: 1,
        })
        .unwrap();
        for sid in ["conv_good_a", "conv_bad", "conv_good_b"] {
            let st = SessionState::new(conn.clone(), SessionId::new(sid), AgentId::new("a"));
            s.upsert_session(&st, 1).unwrap();
        }
        // Corrupt one row's `cost_json` (a required-to-parse json column) into garbage.
        s.conn
            .execute(
                "UPDATE sessions SET cost_json = '{not valid json' WHERE id = 'conv_bad'",
                [],
            )
            .unwrap();
        let loaded = s.list_sessions(&conn).unwrap(); // MUST NOT Err
        let ids: Vec<_> = loaded
            .rows
            .iter()
            .map(|r| r.id.as_str().to_string())
            .collect();
        assert_eq!(ids, vec!["conv_good_a", "conv_good_b"]); // bad one skipped, good ones survive
        assert_eq!(loaded.skipped.len(), 1);
        assert_eq!(loaded.skipped[0].id, "conv_bad"); // reported BY ID (observable)
    }
}
