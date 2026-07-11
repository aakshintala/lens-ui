//! `SqliteTranscriptStore` — the per-session role: one file per (connection,
//! session), holding only that session's `items` (§6.2). The actor owns this
//! file's WAL write connection (P3) — no cross-actor contention. The file is
//! self-describing: its `meta` carries schema_version + (connection_id, session_id).

use crate::domain::ids::{ConnectionId, ItemId, SessionId};
use crate::domain::item::Item;
use crate::persist::db::open_db;
use crate::persist::map::{collect_skipping, item_kind_token, json_string, row_to_item};
use crate::persist::schema::{SCHEMA_VERSION, TRANSCRIPT_DDL};
use crate::persist::{Loaded, PersistError, Result, StoreMode, TranscriptStore};
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
        self.upsert_item_stmt_inner(ordinal, item, true)
    }

    /// `preserve_ordinal_on_conflict`: commit-path re-fires keep the stored row
    /// position (`ordinal=items.ordinal`); reconcile re-stamps (`ordinal=excluded.ordinal`).
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
            "INSERT INTO items (item_id, live_seq, ordinal, kind, payload, agent, depth, turn, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(item_id) DO UPDATE SET
               live_seq=excluded.live_seq, {ordinal_clause}, kind=excluded.kind,
               payload=excluded.payload, agent=excluded.agent, depth=excluded.depth,
               turn=excluded.turn, created_at=excluded.created_at"
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

    /// PRECONDITION (D-P2-7): `ordinal` is a FRESH append position (the item's
    /// index in the actor's canonical `Vec<Item>`). Conflicts resolve on `item_id`
    /// only — reusing an ordinal for a *different* `item_id` raises `UNIQUE(ordinal)`
    /// (a non-panic `Err`). P3 routes any replace/reorder through `reconcile`, not here.
    fn upsert_item(&self, ordinal: i64, item: &Item) -> Result<()> {
        self.guard_write()?;
        self.upsert_item_stmt(ordinal, item)
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

    fn frontier(&self) -> Result<Option<(i64, ItemId)>> {
        let row = self.conn.query_row(
            "SELECT ordinal, item_id FROM items ORDER BY ordinal DESC LIMIT 1",
            [],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
        );
        match row {
            Ok((ord, id)) => Ok(Some((ord, ItemId::new(id)))),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
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
    use crate::domain::ids::{ConnectionId, ItemId, SessionId};
    use crate::domain::item::{BlockContext, ContentBlock, Item, ItemKind};
    use crate::domain::scalars::Role;
    use crate::persist::TranscriptStore;
    use tempfile::tempdir;

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

    fn store(dir: &std::path::Path) -> SqliteTranscriptStore {
        SqliteTranscriptStore::open(
            &dir.join("conv_1.db"),
            &ConnectionId::new("conn_1"),
            &SessionId::new("conv_1"),
        )
        .unwrap()
    }

    #[test]
    fn frontier_returns_max_ordinal_and_its_id() {
        let d = tempdir().unwrap();
        let s = store(d.path());
        assert!(
            s.frontier().unwrap().is_none(),
            "empty transcript has no frontier"
        );
        s.upsert_item(0, &item("item_a", 0, "a")).unwrap();
        s.upsert_item(1, &item("item_b", 0, "b")).unwrap();
        assert_eq!(s.frontier().unwrap(), Some((1, ItemId::new("item_b"))));
    }

    #[test]
    fn refire_by_id_keeps_original_ordinal() {
        let d = tempdir().unwrap();
        let s = store(d.path());
        s.upsert_item(5, &item("item_a", 0, "a")).unwrap();
        // A far-back re-fire arrives with a different (blind) ordinal — position must not move.
        s.upsert_item(99, &item("item_a", 0, "a-refire")).unwrap();
        assert_eq!(s.frontier().unwrap(), Some((5, ItemId::new("item_a"))));
        let rows = s.load_items().unwrap().rows;
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn upsert_items_then_load_ordered_and_self_describing() {
        let d = tempdir().unwrap();
        let s = store(d.path());
        s.upsert_item(0, &item("item_a", 0, "a")).unwrap();
        s.upsert_item(1, &item("item_b", 0, "b")).unwrap();
        // Re-upsert item_a at ordinal 0 with edited text — no dup, payload updated.
        s.upsert_item(0, &item("item_a", 0, "a-edited")).unwrap();
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
        let items = s.load_items().unwrap().rows;
        let ids: Vec<_> = items.iter().map(|i| i.id.as_str().to_string()).collect();
        assert_eq!(ids, vec!["item_a", "item_b", "item_d"]); // c gone, order = truth
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
        // A row with an undecodable payload (or a future/unknown `kind`, which the
        // internally-tagged ItemKind can't degrade) is skipped + reported, never
        // aborting the whole transcript load (§6.3, deferred-#1 / covers deferred-#2).
        let d = tempdir().unwrap();
        let s = store(d.path());
        s.upsert_item(0, &item("item_a", 0, "a")).unwrap();
        s.upsert_item(1, &item("item_b", 0, "b")).unwrap();
        s.upsert_item(2, &item("item_c", 0, "c")).unwrap();
        // Simulate a future/foreign writer: an ItemKind tag this build doesn't know.
        s.conn
            .execute(
                "UPDATE items SET payload = '{\"kind\":\"quantum_tool\",\"x\":1}' WHERE item_id = 'item_b'",
                [],
            )
            .unwrap();
        let loaded = s.load_items().unwrap(); // MUST NOT Err
        let ids: Vec<_> = loaded
            .rows
            .iter()
            .map(|i| i.id.as_str().to_string())
            .collect();
        assert_eq!(ids, vec!["item_a", "item_c"]); // unknown-kind row skipped
        assert_eq!(loaded.skipped.len(), 1);
        assert_eq!(loaded.skipped[0].id, "item_b");
    }
}
