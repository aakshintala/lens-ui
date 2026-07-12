//! `SqliteTranscriptStore` — the per-session role: one file per (connection,
//! session), holding only that session's `items` (§6.2). The actor owns this
//! file's WAL write connection (P3) — no cross-actor contention. The file is
//! self-describing: its `meta` carries schema_version + (connection_id, session_id).

use crate::domain::ids::{ConnectionId, ItemId, SessionId};
use crate::domain::item::{Item, ItemKind};
use crate::persist::db::open_db;
use crate::persist::map::{collect_skipping, item_kind_token, json_string, row_to_item};
use crate::persist::schema::{SCHEMA_VERSION, TRANSCRIPT_DDL};
use crate::persist::{
    LiveKey, Loaded, PersistError, ReconcileOutcome, Result, StoreMode, TranscriptStore,
};
use rusqlite::{Connection, OptionalExtension};
use std::path::Path;

pub struct SqliteTranscriptStore {
    conn: Connection,
    mode: StoreMode,
}

fn item_call_id(item: &Item) -> Option<&str> {
    match &item.kind {
        ItemKind::FunctionCall { call_id, .. } | ItemKind::FunctionCallOutput { call_id, .. } => {
            Some(call_id.as_str())
        }
        _ => None,
    }
}

fn is_duplicate_column_err(e: &rusqlite::Error) -> bool {
    matches!(
        e,
        rusqlite::Error::SqliteFailure(err, Some(msg))
            if err.code == rusqlite::ErrorCode::Unknown
                && msg.contains("duplicate column")
    )
}

/// Idempotent column migration for v1 transcript files (F5/R2-7).
fn migrate_transcript_columns(conn: &Connection) -> Result<()> {
    let columns: Vec<String> = conn
        .prepare("PRAGMA table_info(items)")?
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<std::result::Result<_, _>>()?;

    if !columns.iter().any(|c| c == "provisional")
        && let Err(e) = conn.execute(
            "ALTER TABLE items ADD COLUMN provisional INTEGER NOT NULL DEFAULT 0",
            [],
        )
        && !is_duplicate_column_err(&e)
    {
        return Err(e.into());
    }
    if !columns.iter().any(|c| c == "call_id")
        && let Err(e) = conn.execute("ALTER TABLE items ADD COLUMN call_id TEXT", [])
        && !is_duplicate_column_err(&e)
    {
        return Err(e.into());
    }
    Ok(())
}

impl SqliteTranscriptStore {
    /// Open (creating) the transcript file at `path`. On a fresh file, stamp
    /// `connection_id`/`session_id` into `meta` (self-describing, §6.2). On an
    /// existing file, this is idempotent — the ids are already recorded.
    pub fn open(path: &Path, conn_id: &ConnectionId, session_id: &SessionId) -> Result<Self> {
        let (conn, mode) = open_db(path, TRANSCRIPT_DDL, SCHEMA_VERSION)?;
        if mode == StoreMode::ReadWrite {
            migrate_transcript_columns(&conn)?;
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

    fn upsert_item_stmt(
        &self,
        ordinal: i64,
        item: &Item,
        provisional: bool,
        preserve_ordinal_on_conflict: bool,
    ) -> Result<i64> {
        self.upsert_item_stmt_returning(ordinal, item, provisional, preserve_ordinal_on_conflict)
    }

    /// `preserve_ordinal_on_conflict`: commit-path re-fires keep the stored row
    /// position (`ordinal=items.ordinal`); reconcile re-stamps (`ordinal=excluded.ordinal`).
    fn upsert_item_stmt_returning(
        &self,
        ordinal: i64,
        item: &Item,
        provisional: bool,
        preserve_ordinal_on_conflict: bool,
    ) -> Result<i64> {
        let ordinal_clause = if preserve_ordinal_on_conflict {
            "ordinal=items.ordinal"
        } else {
            "ordinal=excluded.ordinal"
        };
        let payload = json_string(&item.kind)?;
        let kind = item_kind_token(&item.kind);
        let call_id = if provisional {
            item_call_id(item)
        } else {
            None
        };
        let sql = format!(
            "INSERT INTO items (item_id, live_seq, ordinal, kind, payload, agent, depth, turn, created_at, provisional, call_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(item_id) DO UPDATE SET
               live_seq=excluded.live_seq, {ordinal_clause}, kind=excluded.kind,
               payload=excluded.payload, agent=excluded.agent, depth=excluded.depth,
               turn=excluded.turn, created_at=excluded.created_at,
               provisional=excluded.provisional, call_id=excluded.call_id
             RETURNING ordinal"
        );
        self.conn
            .query_row(
                &sql,
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
                    provisional as i64,
                    call_id,
                ],
                |r| r.get(0),
            )
            .map_err(Into::into)
    }

    /// Reconcile re-stamp path — no `RETURNING`; ordinal is always the passed index.
    fn upsert_item_stmt_inner(
        &self,
        ordinal: i64,
        item: &Item,
        preserve_ordinal_on_conflict: bool,
    ) -> Result<()> {
        let ordinal_clause = if preserve_ordinal_on_conflict {
            "ordinal=items.ordinal"
        } else {
            "ordinal=excluded.ordinal"
        };
        let payload = json_string(&item.kind)?;
        let kind = item_kind_token(&item.kind);
        let sql = format!(
            "INSERT INTO items (item_id, live_seq, ordinal, kind, payload, agent, depth, turn, created_at, provisional, call_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, NULL)
             ON CONFLICT(item_id) DO UPDATE SET
               live_seq=excluded.live_seq, {ordinal_clause}, kind=excluded.kind,
               payload=excluded.payload, agent=excluded.agent, depth=excluded.depth,
               turn=excluded.turn, created_at=excluded.created_at,
               provisional=0, call_id=NULL"
        );
        self.conn.execute(
            &sql,
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

    fn find_provisional_match(
        &self,
        tx: &rusqlite::Transaction<'_>,
        live_key: &LiveKey,
    ) -> Result<Option<i64>> {
        tx.query_row(
            "SELECT ordinal FROM items WHERE provisional = 1 AND (
               item_id = ?1 OR (
                 ?2 IS NOT NULL AND call_id = ?2 AND kind = ?3
               )
             ) ORDER BY ordinal LIMIT 1",
            rusqlite::params![
                live_key.id.as_str(),
                live_key.call_id.as_ref().map(|c| c.as_str()),
                live_key.scaffold_kind,
            ],
            |r| r.get(0),
        )
        .optional()
        .map_err(Into::into)
    }
}

impl TranscriptStore for SqliteTranscriptStore {
    fn mode(&self) -> StoreMode {
        self.mode
    }

    fn identity(&self) -> Result<(ConnectionId, SessionId)> {
        let get = |key: &str| -> Result<String> {
            Ok(self
                .conn
                .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))?)
        };
        Ok((
            ConnectionId::new(get("connection_id")?),
            SessionId::new(get("session_id")?),
        ))
    }

    /// D20 commit path: `ordinal` is the actor's `next_ordinal` cursor. Conflicts
    /// resolve on `item_id` only — re-fire preserves the stored ordinal (`RETURNING`
    /// may differ from the requested value). Reconcile re-stamps via its own txn.
    fn upsert_item(&self, ordinal: i64, item: &Item, provisional: bool) -> Result<i64> {
        self.guard_write()?;
        self.upsert_item_stmt(ordinal, item, provisional, true)
    }

    fn load_items(&self) -> Result<Loaded<Item>> {
        let mut stmt = self.conn.prepare(
            "SELECT item_id, live_seq, kind, payload, agent, depth, turn, created_at
             FROM items ORDER BY ordinal",
        )?;
        let mut rows = stmt.query([])?;
        // id_col = 0 (`item_id` is the 1st selected column).
        collect_skipping(&mut rows, 0, row_to_item)
    }

    fn store_frontier(&self) -> Result<Option<(i64, ItemId)>> {
        let row = self.conn.query_row(
            "SELECT ordinal, item_id FROM items WHERE provisional = 0
             ORDER BY ordinal DESC LIMIT 1",
            [],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
        );
        match row {
            Ok((ord, id)) => Ok(Some((ord, ItemId::new(id)))),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn next_ordinal_seed(&self) -> Result<i64> {
        let max: Option<i64> = self
            .conn
            .query_row("SELECT MAX(ordinal) FROM items", [], |r| r.get(0))
            .optional()?
            .flatten();
        Ok(max.map(|o| o + 1).unwrap_or(0))
    }

    fn reconcile_store_item(
        &self,
        store_item: &Item,
        live_key: &LiveKey,
    ) -> Result<ReconcileOutcome> {
        self.guard_write()?;
        let tx = self.conn.unchecked_transaction()?;
        let matched_ordinal = self.find_provisional_match(&tx, live_key)?;
        let outcome = match matched_ordinal {
            Some(ord) => {
                let existing_store_ord: Option<i64> = tx
                    .query_row(
                        "SELECT ordinal FROM items WHERE item_id = ?1 AND ordinal != ?2",
                        rusqlite::params![store_item.id.as_str(), ord],
                        |r| r.get(0),
                    )
                    .optional()?;
                match existing_store_ord {
                    Some(store_ord) => {
                        tx.execute(
                            "DELETE FROM items WHERE ordinal = ?1 AND provisional = 1",
                            [ord],
                        )?;
                        tx.execute(
                            "UPDATE items SET kind = ?1, payload = ?2, provisional = 0
                               WHERE item_id = ?3",
                            rusqlite::params![
                                item_kind_token(&store_item.kind),
                                json_string(&store_item.kind)?,
                                store_item.id.as_str(),
                            ],
                        )?;
                        ReconcileOutcome::Folded { ordinal: store_ord }
                    }
                    None => {
                        tx.execute(
                            "UPDATE items SET item_id = ?1, live_seq = NULL, provisional = 0,
                               call_id = NULL, kind = ?2, payload = ?3 WHERE ordinal = ?4",
                            rusqlite::params![
                                store_item.id.as_str(),
                                item_kind_token(&store_item.kind),
                                json_string(&store_item.kind)?,
                                ord,
                            ],
                        )?;
                        ReconcileOutcome::Folded { ordinal: ord }
                    }
                }
            }
            None => ReconcileOutcome::NoMatch,
        };
        tx.commit()?;
        Ok(outcome)
    }

    fn reconcile(&self, items: &[Item]) -> Result<()> {
        self.guard_write()?;
        // Wrap in a single transaction: make the file match `items` exactly.
        // ordinal UNIQUE forbids two rows sharing an ordinal mid-update, so clear
        // ordinals to a disjoint negative range first, then re-stamp 0..n.
        self.conn.execute("BEGIN", [])?;
        let result = (|| -> Result<()> {
            // Park existing ordinals out of the way (negative = never a real ordinal).
            self.conn
                .execute("UPDATE items SET ordinal = -1 - ordinal", [])?;
            // Upsert every truth item at its canonical index.
            for (i, item) in items.iter().enumerate() {
                self.upsert_item_stmt_inner(i as i64, item, false)?;
            }
            // Delete anything the upserts did not touch (ordinal still negative).
            self.conn
                .execute("DELETE FROM items WHERE ordinal < 0", [])?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ids::{CallId, ConnectionId, ItemId, SessionId};
    use crate::domain::item::{BlockContext, ContentBlock, Item, ItemKind};
    use crate::domain::scalars::Role;
    use crate::persist::TranscriptStore;
    use crate::persist::db::{VersionState, open_db, read_schema_version};
    use serde_json::json;
    use tempfile::tempdir;

    const V1_TRANSCRIPT_DDL: &str = r#"
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

    fn item(id: &str, turn: u32, text: &str) -> Item {
        Item {
            id: ItemId::new(id),
            seq: Some(1),
            ctx: BlockContext {
                agent: Some("coder".into()),
                depth: 0,
                turn,
            },
            created_at: 1_700_000_000_000,
            kind: ItemKind::Message {
                role: Role::Assistant,
                content: vec![ContentBlock {
                    kind: "text".into(),
                    text: Some(text.into()),
                    data: serde_json::Value::Null,
                }],
            },
        }
    }

    fn user_message(id: &str, text: &str) -> Item {
        Item {
            id: ItemId::new(id),
            seq: None,
            ctx: BlockContext {
                agent: None,
                depth: 0,
                turn: 0,
            },
            created_at: 1_700_000_000_100,
            kind: ItemKind::Message {
                role: Role::User,
                content: vec![ContentBlock {
                    kind: "input_text".into(),
                    text: Some(text.into()),
                    data: serde_json::Value::Null,
                }],
            },
        }
    }

    fn function_call(id: &str, call_id: &str) -> Item {
        Item {
            id: ItemId::new(id),
            seq: Some(1),
            ctx: BlockContext {
                agent: None,
                depth: 0,
                turn: 0,
            },
            created_at: 1_700_000_000_000,
            kind: ItemKind::FunctionCall {
                call_id: CallId::new(call_id),
                name: "TaskUpdate".into(),
                arguments: json!({}),
                status: "completed".into(),
                agent_name: None,
            },
        }
    }

    fn function_call_output(id: &str, call_id: &str, output: &str) -> Item {
        Item {
            id: ItemId::new(id),
            seq: Some(1),
            ctx: BlockContext {
                agent: None,
                depth: 0,
                turn: 0,
            },
            created_at: 1_700_000_000_000,
            kind: ItemKind::FunctionCallOutput {
                call_id: CallId::new(call_id),
                output: output.into(),
                arguments: json!({}),
            },
        }
    }

    fn store(dir: &std::path::Path) -> SqliteTranscriptStore {
        SqliteTranscriptStore::open(
            &dir.join("conv_1.db"),
            &ConnectionId::new("conn_1"),
            &SessionId::new("conv_1"),
        )
        .unwrap()
    }

    fn provisional_flag(s: &SqliteTranscriptStore, id: &str) -> i64 {
        s.conn
            .query_row(
                "SELECT provisional FROM items WHERE item_id = ?1",
                [id],
                |r| r.get(0),
            )
            .unwrap()
    }

    #[test]
    fn migrate_v1_file_adds_provisional_and_call_id_columns() {
        let d = tempdir().unwrap();
        let path = d.path().join("conv_1.db");
        {
            let (conn, _) = open_db(&path, V1_TRANSCRIPT_DDL, 1).unwrap();
            conn.execute(
                "INSERT INTO meta (key, value) VALUES ('connection_id', 'conn_1')
                 ON CONFLICT(key) DO NOTHING",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO meta (key, value) VALUES ('session_id', 'conv_1')
                 ON CONFLICT(key) DO NOTHING",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO items (item_id, ordinal, kind, payload, created_at)
                 VALUES ('item_a', 0, 'message', '{}', 1)",
                [],
            )
            .unwrap();
            assert_eq!(read_schema_version(&conn).unwrap(), VersionState::Known(1));
        }

        let s = SqliteTranscriptStore::open(
            &path,
            &ConnectionId::new("conn_1"),
            &SessionId::new("conv_1"),
        )
        .unwrap();
        assert_eq!(
            read_schema_version(&s.conn).unwrap(),
            VersionState::Known(2)
        );

        let columns: Vec<String> = s
            .conn
            .prepare("PRAGMA table_info(items)")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        assert!(columns.contains(&"provisional".to_string()));
        assert!(columns.contains(&"call_id".to_string()));

        let prov: i64 = s
            .conn
            .query_row(
                "SELECT provisional FROM items WHERE item_id = 'item_a'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(prov, 0);
    }

    #[test]
    fn store_frontier_ignores_provisional() {
        let d = tempdir().unwrap();
        let s = store(d.path());
        s.upsert_item(0, &item("item_a", 0, "a"), false).unwrap();
        s.upsert_item(1, &item("item_b", 0, "b"), false).unwrap();
        s.upsert_item(2, &function_call("fc_live", "call_1"), true)
            .unwrap();
        assert_eq!(
            s.store_frontier().unwrap(),
            Some((1, ItemId::new("item_b")))
        );
    }

    #[test]
    fn next_ordinal_seed_counts_provisional() {
        let d = tempdir().unwrap();
        let s = store(d.path());
        s.upsert_item(0, &item("item_a", 0, "a"), false).unwrap();
        s.upsert_item(1, &item("item_b", 0, "b"), false).unwrap();
        s.upsert_item(2, &function_call("fc_live", "call_1"), true)
            .unwrap();
        assert_eq!(s.next_ordinal_seed().unwrap(), 3);
    }

    #[test]
    fn reconcile_scaffold_tool_folds_by_call_id() {
        let d = tempdir().unwrap();
        let s = store(d.path());
        s.upsert_item(5, &function_call("fc_live", "call_1"), true)
            .unwrap();

        let store_row = function_call("msg_store", "call_1");
        let live_key = LiveKey {
            id: store_row.id.clone(),
            call_id: Some(CallId::new("call_1")),
            scaffold_kind: Some("function_call"),
        };
        let outcome = s.reconcile_store_item(&store_row, &live_key).unwrap();
        assert_eq!(outcome, ReconcileOutcome::Folded { ordinal: 5 });

        let rows = s.load_items().unwrap().rows;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id.as_str(), "msg_store");
        assert_eq!(provisional_flag(&s, "msg_store"), 0);
    }

    #[test]
    fn reconcile_message_folds_by_id() {
        let d = tempdir().unwrap();
        let s = store(d.path());
        s.upsert_item(0, &user_message("msg_1", "hello"), true)
            .unwrap();

        let store_row = user_message("msg_1", "hello from store");
        let live_key = LiveKey {
            id: store_row.id.clone(),
            call_id: None,
            scaffold_kind: None,
        };
        let outcome = s.reconcile_store_item(&store_row, &live_key).unwrap();
        assert_eq!(outcome, ReconcileOutcome::Folded { ordinal: 0 });
        assert_eq!(provisional_flag(&s, "msg_1"), 0);
    }

    #[test]
    fn reconcile_fco_folds_by_call_id_into_provisional_fco() {
        let d = tempdir().unwrap();
        let s = store(d.path());
        s.upsert_item(
            5,
            &function_call_output("fco_live", "call_1", "live out"),
            true,
        )
        .unwrap();

        let store_row = function_call_output("fco_store", "call_1", "store out");
        let live_key = LiveKey {
            id: store_row.id.clone(),
            call_id: Some(CallId::new("call_1")),
            scaffold_kind: Some("function_call_output"),
        };
        let outcome = s.reconcile_store_item(&store_row, &live_key).unwrap();
        assert_eq!(outcome, ReconcileOutcome::Folded { ordinal: 5 });

        let rows = s.load_items().unwrap().rows;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id.as_str(), "fco_store");
        assert_eq!(provisional_flag(&s, "fco_store"), 0);
    }

    #[test]
    fn reconcile_fco_does_not_fold_into_provisional_function_call() {
        let d = tempdir().unwrap();
        let s = store(d.path());
        s.upsert_item(5, &function_call("fc_live", "call_1"), true)
            .unwrap();

        let store_row = function_call_output("fco_store", "call_1", "tool output");
        let live_key = LiveKey {
            id: store_row.id.clone(),
            call_id: Some(CallId::new("call_1")),
            scaffold_kind: Some("function_call_output"),
        };
        let outcome = s.reconcile_store_item(&store_row, &live_key).unwrap();
        assert_eq!(outcome, ReconcileOutcome::NoMatch);

        let rows = s.load_items().unwrap().rows;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id.as_str(), "fc_live");
        match &rows[0].kind {
            ItemKind::FunctionCall { .. } => {}
            other => panic!("expected FunctionCall, got {other:?}"),
        }
    }

    #[test]
    fn reconcile_when_store_id_already_present_deletes_provisional() {
        let d = tempdir().unwrap();
        let s = store(d.path());
        let store_row = function_call("msg_store", "call_1");
        s.upsert_item(0, &store_row, false).unwrap();
        s.upsert_item(5, &function_call("fc_live", "call_1"), true)
            .unwrap();

        let live_key = LiveKey {
            id: store_row.id.clone(),
            call_id: Some(CallId::new("call_1")),
            scaffold_kind: Some("function_call"),
        };
        let outcome = s.reconcile_store_item(&store_row, &live_key).unwrap();
        assert_eq!(outcome, ReconcileOutcome::Folded { ordinal: 0 });

        let rows = s.load_items().unwrap().rows;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id.as_str(), "msg_store");
        assert_eq!(provisional_flag(&s, "msg_store"), 0);
    }

    #[test]
    fn refire_by_id_keeps_original_ordinal() {
        let d = tempdir().unwrap();
        let s = store(d.path());
        s.upsert_item(5, &item("item_a", 0, "a"), false).unwrap();
        s.upsert_item(99, &item("item_a", 0, "a-refire"), false)
            .unwrap();
        assert_eq!(
            s.store_frontier().unwrap(),
            Some((5, ItemId::new("item_a")))
        );
        let rows = s.load_items().unwrap().rows;
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn refire_returns_stored_ordinal_without_bumping_cursor() {
        let d = tempdir().unwrap();
        let s = store(d.path());
        s.upsert_item(0, &item("item_a", 0, "a"), false).unwrap();
        s.upsert_item(1, &item("item_b", 0, "b"), false).unwrap();
        let stored = s
            .upsert_item(99, &item("item_a", 0, "a-refire"), false)
            .unwrap();
        assert_eq!(stored, 0);
        let stored = s.upsert_item(2, &item("item_c", 0, "c"), false).unwrap();
        assert_eq!(stored, 2);
        let rows = s.load_items().unwrap().rows;
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[2].id.as_str(), "item_c");
    }

    #[test]
    fn upsert_items_then_load_ordered_and_self_describing() {
        let d = tempdir().unwrap();
        let s = store(d.path());
        s.upsert_item(0, &item("item_a", 0, "a"), false).unwrap();
        s.upsert_item(1, &item("item_b", 0, "b"), false).unwrap();
        s.upsert_item(0, &item("item_a", 0, "a-edited"), false)
            .unwrap();
        let items = s.load_items().unwrap().rows;
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].id.as_str(), "item_a");
        assert_eq!(items[1].id.as_str(), "item_b");
        match &items[0].kind {
            ItemKind::Message { content, .. } => {
                assert_eq!(content[0].text.as_deref(), Some("a-edited"));
            }
            _ => panic!("wrong kind"),
        }
        assert_eq!(items[0].ctx.agent.as_deref(), Some("coder"));
        assert_eq!(
            s.identity().unwrap(),
            (ConnectionId::new("conn_1"), SessionId::new("conv_1"))
        );
    }

    #[test]
    fn reconcile_matches_server_truth_by_id() {
        let d = tempdir().unwrap();
        let s = store(d.path());
        s.upsert_item(0, &item("item_a", 0, "a"), false).unwrap();
        s.upsert_item(1, &item("item_b", 0, "b"), false).unwrap();
        s.upsert_item(2, &item("item_c", 0, "c"), false).unwrap();
        let truth = vec![
            item("item_a", 0, "a"),
            item("item_b", 1, "b-edited"),
            item("item_d", 1, "d"),
        ];
        s.reconcile(&truth).unwrap();
        let items = s.load_items().unwrap().rows;
        let ids: Vec<_> = items.iter().map(|i| i.id.as_str().to_string()).collect();
        assert_eq!(ids, vec!["item_a", "item_b", "item_d"]);
        match &items[1].kind {
            ItemKind::Message { content, .. } => {
                assert_eq!(content[0].text.as_deref(), Some("b-edited"));
            }
            _ => panic!(),
        }
        assert_eq!(items[1].ctx.turn, 1);
    }

    #[test]
    fn corrupt_item_is_skipped_not_fatal_and_good_items_survive() {
        let d = tempdir().unwrap();
        let s = store(d.path());
        s.upsert_item(0, &item("item_a", 0, "a"), false).unwrap();
        s.upsert_item(1, &item("item_b", 0, "b"), false).unwrap();
        s.upsert_item(2, &item("item_c", 0, "c"), false).unwrap();
        s.conn
            .execute(
                "UPDATE items SET payload = '{\"kind\":\"quantum_tool\",\"x\":1}' WHERE item_id = 'item_b'",
                [],
            )
            .unwrap();
        let loaded = s.load_items().unwrap();
        let ids: Vec<_> = loaded
            .rows
            .iter()
            .map(|i| i.id.as_str().to_string())
            .collect();
        assert_eq!(ids, vec!["item_a", "item_c"]);
        assert_eq!(loaded.skipped.len(), 1);
        assert_eq!(loaded.skipped[0].id, "item_b");
    }
}
