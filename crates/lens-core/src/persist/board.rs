//! `SqliteBoardStore` — board layout in the control-plane db (`lens.db`).

use crate::domain::board::{
    Board, BoardError, BoardItem, BoardItemKind, BoardLayout, DEFAULT_BOARD_ID, DEFAULT_BOARD_NAME,
    PlacementTarget,
};
use crate::domain::ids::{BoardId, BoardItemId, ConnectionId, SessionId};
use crate::persist::db::open_db;
use crate::persist::map::collect_skipping;
use crate::persist::schema::{CONTROL_DDL, SCHEMA_VERSION};
use crate::persist::{Loaded, PersistError, Result, StoreMode};
use rusqlite::{Connection, OptionalExtension};
use std::collections::HashSet;
use std::path::Path;

pub trait BoardStore {
    fn mode(&self) -> StoreMode;

    /// Full layout load. In `ReadWrite` mode, sessions without a card row are
    /// lazily placed on the default board root (§4 startup reconcile).
    fn load_layout(&self) -> Result<Loaded<BoardLayout>>;

    fn place_session(
        &self,
        conn: &ConnectionId,
        session: &SessionId,
        target: &PlacementTarget,
    ) -> Result<()>;

    fn remove_session(&self, conn: &ConnectionId, session: &SessionId) -> Result<()>;

    /// Batch placement (§3.3): place each non-tombstoned, not-already-present session,
    /// persisting each touched board ONCE inside ONE transaction (one persist vs k).
    /// Tombstoned/duplicate entries are skipped. Callers re-read via `load_layout` for
    /// the reconciled view (read-time lazy-place + tombstone-prune).
    fn place_sessions(
        &self,
        placements: &[(ConnectionId, SessionId)],
        target: &PlacementTarget,
    ) -> Result<()>;

    fn create_group(
        &self,
        board_id: &BoardId,
        parent_item_id: Option<BoardItemId>,
        ordinal: i32,
        name: &str,
    ) -> Result<BoardItemId>;

    fn move_item(
        &self,
        item_id: &BoardItemId,
        new_board_id: &BoardId,
        new_parent: Option<BoardItemId>,
        new_ordinal: i32,
    ) -> Result<()>;

    fn ungroup(&self, group_id: &BoardItemId) -> Result<()>;

    fn rename(&self, item_id: &BoardItemId, name: &str) -> Result<()>;

    fn archive(&self, item_id: &BoardItemId) -> Result<()>;

    fn set_collapsed(&self, group_id: &BoardItemId, collapsed: bool) -> Result<()>;

    fn set_color(&self, group_id: &BoardItemId, token: &str) -> Result<()>;
}

pub struct SqliteBoardStore {
    pub(crate) conn: Connection,
    mode: StoreMode,
    /// Monotonic minting counter for the `seq` field of `{prefix}_{seq}_{ms}` ids.
    /// Seeded past the highest `seq` *ever embedded in an existing id* (NOT the row
    /// count) so `seq` is never reused across reopens — even after deletes drop the
    /// count below the high-water mark. Combined with the `ms` suffix this makes
    /// `item_id`s globally unique without depending on a monotonic wall clock.
    next_seq: std::sync::atomic::AtomicU64,
}

impl SqliteBoardStore {
    pub fn open(path: &Path) -> Result<Self> {
        let (conn, mode) = open_db(path, CONTROL_DDL, SCHEMA_VERSION)?;
        let seed = Self::max_embedded_seq(&conn)?.map_or(0, |m| m + 1);
        let store = Self {
            conn,
            mode,
            next_seq: std::sync::atomic::AtomicU64::new(seed),
        };
        if store.mode == StoreMode::ReadWrite {
            store.ensure_default_board()?;
        }
        Ok(store)
    }

    /// Highest `seq` embedded in any existing `board_items.item_id` (`{prefix}_{seq}_{ms}`),
    /// or `None` if there are no parseable rows. Ids with an unexpected shape are ignored.
    fn max_embedded_seq(conn: &Connection) -> Result<Option<u64>> {
        let mut stmt = conn.prepare("SELECT item_id FROM board_items")?;
        let mut rows = stmt.query([])?;
        let mut max: Option<u64> = None;
        while let Some(row) = rows.next()? {
            let id: String = row.get(0)?;
            // `{prefix}_{seq}_{ms}` — seq is the second-to-last `_`-separated field.
            if let Some(seq) = id.rsplit('_').nth(1).and_then(|s| s.parse::<u64>().ok()) {
                max = Some(max.map_or(seq, |m| m.max(seq)));
            }
        }
        Ok(max)
    }

    fn guard_write(&self) -> Result<()> {
        match self.mode {
            StoreMode::ReadWrite => Ok(()),
            StoreMode::ReadOnlyDegraded => Err(PersistError::ReadOnly),
        }
    }

    fn now_ms(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }

    fn new_item_id(&self, prefix: &str) -> BoardItemId {
        let seq = self
            .next_seq
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        BoardItemId::new(format!("{prefix}_{}_{}", seq, self.now_ms()))
    }

    /// Is `(conn, session)` present in the `sessions` table with `tombstoned_at` set?
    /// A session absent from the table is NOT tombstoned (absence ≠ delete, §4).
    fn is_tombstoned(&self, conn: &ConnectionId, session: &SessionId) -> Result<bool> {
        let found: Option<i64> = self
            .conn
            .query_row(
                "SELECT 1 FROM sessions
                 WHERE connection_id = ?1 AND id = ?2 AND tombstoned_at IS NOT NULL",
                rusqlite::params![conn.as_str(), session.as_str()],
                |r| r.get(0),
            )
            .optional()?;
        Ok(found.is_some())
    }

    fn ensure_default_board(&self) -> Result<()> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM boards", [], |r| r.get(0))?;
        if count == 0 {
            let now = self.now_ms();
            self.conn.execute(
                "INSERT INTO boards (id, name, ordinal, created_at, updated_at)
                 VALUES (?1, ?2, 0, ?3, ?3)",
                rusqlite::params![DEFAULT_BOARD_ID, DEFAULT_BOARD_NAME, now],
            )?;
        }
        Ok(())
    }

    fn load_boards(&self) -> Result<Loaded<Board>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, ordinal, created_at, updated_at FROM boards ORDER BY ordinal, id",
        )?;
        let mut rows = stmt.query([])?;
        collect_skipping(&mut rows, 0, |r| {
            Ok(Board {
                id: BoardId::new(r.get::<_, String>(0)?),
                name: r.get(1)?,
                ordinal: r.get(2)?,
                created_at: r.get(3)?,
                updated_at: r.get(4)?,
            })
        })
    }

    fn load_items(&self) -> Result<Loaded<BoardItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT item_id, board_id, parent_item_id, ordinal, kind,
                    session_conn_id, session_id, group_name, color_token,
                    collapsed, archived, created_at
             FROM board_items ORDER BY board_id, parent_item_id, ordinal, item_id",
        )?;
        let mut rows = stmt.query([])?;
        collect_skipping(&mut rows, 0, row_to_board_item)
    }

    fn load_layout_inner(&self) -> Result<Loaded<BoardLayout>> {
        let boards = self.load_boards()?;
        let items = self.load_items()?;
        let mut skipped = boards.skipped;
        skipped.extend(items.skipped);
        Ok(Loaded {
            rows: vec![BoardLayout {
                boards: boards.rows,
                items: items.rows,
            }],
            skipped,
        })
    }

    /// Bidirectional startup reconcile (§4): place a loose card for every live
    /// session that lacks one, AND drop cards whose session is **tombstoned**
    /// (tombstone ≡ delete for placement). A session merely *absent* from the
    /// `sessions` table is left alone — a card may legitimately exist ahead of its
    /// control row (optimistic placement), so absence must not nuke a placement;
    /// only an explicit `tombstoned_at` prunes. Idempotent.
    fn reconcile_sessions(&self, layout: &mut BoardLayout) -> Result<()> {
        let live: Vec<(ConnectionId, SessionId)> = {
            // ORDER BY created_at, id: deterministic loose-append order when a batch of
            // pre-existing sessions is lazily placed on the same load (e.g. v2→v3 upgrade).
            let mut stmt = self.conn.prepare(
                "SELECT connection_id, id FROM sessions
                 WHERE tombstoned_at IS NULL ORDER BY created_at, id",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    ConnectionId::new(r.get::<_, String>(0)?),
                    SessionId::new(r.get::<_, String>(1)?),
                ))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        let tombstoned: HashSet<(String, String)> = {
            let mut stmt = self.conn.prepare(
                "SELECT connection_id, id FROM sessions WHERE tombstoned_at IS NOT NULL",
            )?;
            let rows =
                stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            rows.collect::<rusqlite::Result<HashSet<_>>>()?
        };

        // Cards whose session is tombstoned → prune (capture ids before removal).
        let stale: Vec<(BoardItemId, ConnectionId, SessionId)> = layout
            .items
            .iter()
            .filter_map(|i| match &i.kind {
                BoardItemKind::Card { conn, session }
                    if tombstoned
                        .contains(&(conn.as_str().to_string(), session.as_str().to_string())) =>
                {
                    Some((i.id.clone(), conn.clone(), session.clone()))
                }
                _ => None,
            })
            .collect();

        let mut dirty = !stale.is_empty();
        for (_, conn, session) in &stale {
            layout.remove_session(conn, session)?;
        }
        for (conn, session) in &live {
            if layout.find_card(conn, session).is_none() {
                let item_id = self.new_item_id("card");
                layout.place_session(
                    conn.clone(),
                    session.clone(),
                    &PlacementTarget::default(),
                    item_id,
                    self.now_ms(),
                )?;
                dirty = true;
            }
        }
        if !dirty {
            return Ok(());
        }
        // Persist atomically: drop stale rows, then re-upsert every surviving item
        // (covers added cards + any sibling renumber from the prunes).
        let tx = self.conn.unchecked_transaction()?;
        for (item_id, _, _) in &stale {
            self.delete_item(item_id)?;
        }
        let board_ids: Vec<BoardId> = layout.boards.iter().map(|b| b.id.clone()).collect();
        for board_id in &board_ids {
            self.persist_board_items(layout, board_id)?;
        }
        tx.commit()?;
        Ok(())
    }

    fn upsert_item(&self, item: &BoardItem) -> Result<()> {
        let (kind, session_conn, session_id, group_name, color_token, collapsed, archived) =
            match &item.kind {
                BoardItemKind::Card { conn, session } => (
                    "card",
                    Some(conn.as_str()),
                    Some(session.as_str()),
                    None,
                    None,
                    0_i64,
                    0_i64,
                ),
                BoardItemKind::Group {
                    name,
                    color_token,
                    collapsed,
                    archived,
                } => (
                    "group",
                    None,
                    None,
                    Some(name.as_str()),
                    color_token.as_deref(),
                    i64::from(*collapsed),
                    i64::from(*archived),
                ),
            };
        self.conn.execute(
            "INSERT INTO board_items (
               item_id, board_id, parent_item_id, ordinal, kind,
               session_conn_id, session_id, group_name, color_token,
               collapsed, archived, group_config, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, NULL, ?12)
             ON CONFLICT(item_id) DO UPDATE SET
               board_id = excluded.board_id,
               parent_item_id = excluded.parent_item_id,
               ordinal = excluded.ordinal,
               kind = excluded.kind,
               session_conn_id = excluded.session_conn_id,
               session_id = excluded.session_id,
               group_name = excluded.group_name,
               color_token = excluded.color_token,
               collapsed = excluded.collapsed,
               archived = excluded.archived",
            rusqlite::params![
                item.id.as_str(),
                item.board_id.as_str(),
                item.parent_item_id.as_ref().map(|p| p.as_str()),
                item.ordinal,
                kind,
                session_conn,
                session_id,
                group_name,
                color_token,
                collapsed,
                archived,
                item.created_at,
            ],
        )?;
        Ok(())
    }

    fn persist_board_items(&self, layout: &BoardLayout, board_id: &BoardId) -> Result<()> {
        for item in layout.items.iter().filter(|i| &i.board_id == board_id) {
            self.upsert_item(item)?;
        }
        Ok(())
    }

    fn delete_item(&self, item_id: &BoardItemId) -> Result<()> {
        self.conn.execute(
            "DELETE FROM board_items WHERE item_id = ?1",
            [item_id.as_str()],
        )?;
        Ok(())
    }

    fn touch_board(&self, board_id: &BoardId) -> Result<()> {
        self.conn.execute(
            "UPDATE boards SET updated_at = ?1 WHERE id = ?2",
            rusqlite::params![self.now_ms(), board_id.as_str()],
        )?;
        Ok(())
    }
}

fn row_to_board_item(r: &rusqlite::Row<'_>) -> rusqlite::Result<BoardItem> {
    let kind: String = r.get(4)?;
    let item = BoardItem {
        id: BoardItemId::new(r.get::<_, String>(0)?),
        board_id: BoardId::new(r.get::<_, String>(1)?),
        parent_item_id: r.get::<_, Option<String>>(2)?.map(BoardItemId::new),
        ordinal: r.get(3)?,
        kind: match kind.as_str() {
            "card" => {
                let conn: Option<String> = r.get(5)?;
                let session: Option<String> = r.get(6)?;
                let (Some(conn), Some(session)) = (conn, session) else {
                    return Err(rusqlite::Error::InvalidColumnType(
                        5,
                        "session_conn_id/session_id".into(),
                        rusqlite::types::Type::Text,
                    ));
                };
                BoardItemKind::Card {
                    conn: ConnectionId::new(conn),
                    session: SessionId::new(session),
                }
            }
            "group" => {
                let name: Option<String> = r.get(7)?;
                let Some(name) = name else {
                    return Err(rusqlite::Error::InvalidColumnType(
                        7,
                        "group_name".into(),
                        rusqlite::types::Type::Text,
                    ));
                };
                BoardItemKind::Group {
                    name,
                    color_token: r.get(8)?,
                    collapsed: r.get::<_, i64>(9)? != 0,
                    archived: r.get::<_, i64>(10)? != 0,
                }
            }
            other => {
                return Err(rusqlite::Error::InvalidColumnType(
                    4,
                    format!("kind={other}"),
                    rusqlite::types::Type::Text,
                ));
            }
        },
        created_at: r.get(11)?,
    };
    Ok(item)
}

impl BoardStore for SqliteBoardStore {
    fn mode(&self) -> StoreMode {
        self.mode
    }

    fn load_layout(&self) -> Result<Loaded<BoardLayout>> {
        let mut loaded = self.load_layout_inner()?;
        let mut layout = loaded.rows.pop().unwrap_or_default();
        if self.mode == StoreMode::ReadWrite {
            if layout.boards.is_empty() {
                self.ensure_default_board()?;
                layout.boards = self.load_boards()?.rows;
            }
            self.reconcile_sessions(&mut layout)?;
        }
        loaded.rows = vec![layout];
        Ok(loaded)
    }

    fn place_session(
        &self,
        conn: &ConnectionId,
        session: &SessionId,
        target: &PlacementTarget,
    ) -> Result<()> {
        self.guard_write()?;
        // Tombstone ≡ delete for placement (§4): refuse to (re)place a tombstoned
        // session, matching `reconcile_sessions`. Otherwise the card would linger
        // until the next full load pruned it. No-op (not an error) — the caller's
        // intent (a placement) is simply already void.
        if self.is_tombstoned(conn, session)? {
            return Ok(());
        }
        let mut loaded = self.load_layout_inner()?;
        let mut layout = loaded.rows.pop().unwrap_or_default();
        if layout.boards.is_empty() {
            self.ensure_default_board()?;
            layout.boards = self.load_boards()?.rows;
        }
        if layout.find_card(conn, session).is_some() {
            return Ok(());
        }
        let item_id = self.new_item_id("card");
        let created_at = self.now_ms();
        layout.place_session(
            conn.clone(),
            session.clone(),
            target,
            item_id.clone(),
            created_at,
        )?;
        let item = layout.item(&item_id).expect("just inserted");
        let board_id = item.board_id.clone();
        let tx = self.conn.unchecked_transaction()?;
        self.persist_board_items(&layout, &board_id)?;
        self.touch_board(&board_id)?;
        tx.commit()?;
        Ok(())
    }

    fn place_sessions(
        &self,
        placements: &[(ConnectionId, SessionId)],
        target: &PlacementTarget,
    ) -> Result<()> {
        self.guard_write()?;
        let mut loaded = self.load_layout_inner()?;
        let mut layout = loaded.rows.pop().unwrap_or_default();
        if layout.boards.is_empty() {
            self.ensure_default_board()?;
            layout.boards = self.load_boards()?.rows;
        }
        let mut touched: std::collections::HashSet<BoardId> = std::collections::HashSet::new();
        for (conn, session) in placements {
            if self.is_tombstoned(conn, session)? {
                continue;
            }
            if layout.find_card(conn, session).is_some() {
                continue;
            }
            let item_id = self.new_item_id("card");
            let created_at = self.now_ms();
            layout.place_session(
                conn.clone(),
                session.clone(),
                target,
                item_id.clone(),
                created_at,
            )?;
            let board_id = layout
                .item(&item_id)
                .expect("just inserted")
                .board_id
                .clone();
            touched.insert(board_id);
        }
        if touched.is_empty() {
            return Ok(());
        }
        let tx = self.conn.unchecked_transaction()?;
        for board_id in &touched {
            self.persist_board_items(&layout, board_id)?;
            self.touch_board(board_id)?;
        }
        tx.commit()?;
        Ok(())
    }

    fn remove_session(&self, conn: &ConnectionId, session: &SessionId) -> Result<()> {
        self.guard_write()?;
        let mut loaded = self.load_layout_inner()?;
        let mut layout = loaded.rows.pop().unwrap_or_default();
        let Some(existing) = layout.find_card(conn, session) else {
            return Ok(());
        };
        let item_id = existing.id.clone();
        let board_id = existing.board_id.clone();
        layout.remove_session(conn, session)?;
        let tx = self.conn.unchecked_transaction()?;
        self.delete_item(&item_id)?;
        self.persist_board_items(&layout, &board_id)?;
        self.touch_board(&board_id)?;
        tx.commit()?;
        Ok(())
    }

    fn create_group(
        &self,
        board_id: &BoardId,
        parent_item_id: Option<BoardItemId>,
        ordinal: i32,
        name: &str,
    ) -> Result<BoardItemId> {
        self.guard_write()?;
        let mut loaded = self.load_layout_inner()?;
        let mut layout = loaded.rows.pop().unwrap_or_default();
        let item_id = self.new_item_id("group");
        let created_at = self.now_ms();
        layout.create_group(
            board_id,
            parent_item_id,
            ordinal,
            name,
            item_id.clone(),
            created_at,
        )?;
        let tx = self.conn.unchecked_transaction()?;
        self.persist_board_items(&layout, board_id)?;
        self.touch_board(board_id)?;
        tx.commit()?;
        Ok(item_id)
    }

    fn move_item(
        &self,
        item_id: &BoardItemId,
        new_board_id: &BoardId,
        new_parent: Option<BoardItemId>,
        new_ordinal: i32,
    ) -> Result<()> {
        self.guard_write()?;
        let mut loaded = self.load_layout_inner()?;
        let mut layout = loaded.rows.pop().unwrap_or_default();
        let old_board = layout
            .item(item_id)
            .ok_or(BoardError::ItemNotFound)?
            .board_id
            .clone();
        layout.move_item(item_id, new_board_id, new_parent, new_ordinal)?;
        let tx = self.conn.unchecked_transaction()?;
        // Persist the destination board FIRST so the moved subtree's rows get their
        // new `board_id` before the source board is re-persisted (order is harmless
        // within the transaction, but keeps the moved rows unambiguously owned).
        self.persist_board_items(&layout, new_board_id)?;
        if old_board != *new_board_id {
            self.persist_board_items(&layout, &old_board)?;
        }
        self.touch_board(new_board_id)?;
        if old_board != *new_board_id {
            self.touch_board(&old_board)?;
        }
        tx.commit()?;
        Ok(())
    }

    fn ungroup(&self, group_id: &BoardItemId) -> Result<()> {
        self.guard_write()?;
        let mut loaded = self.load_layout_inner()?;
        let mut layout = loaded.rows.pop().unwrap_or_default();
        let board_id = layout
            .item(group_id)
            .ok_or(BoardError::ItemNotFound)?
            .board_id
            .clone();
        layout.ungroup(group_id)?;
        let tx = self.conn.unchecked_transaction()?;
        // Persist the reparented children FIRST — this rewrites their `parent_item_id`
        // off the group in the DB, so deleting the group row triggers no
        // `ON DELETE CASCADE` against them (which would otherwise wipe the subtree).
        self.persist_board_items(&layout, &board_id)?;
        self.delete_item(group_id)?;
        self.touch_board(&board_id)?;
        tx.commit()?;
        Ok(())
    }

    fn rename(&self, item_id: &BoardItemId, name: &str) -> Result<()> {
        self.guard_write()?;
        let mut loaded = self.load_layout_inner()?;
        let mut layout = loaded.rows.pop().unwrap_or_default();
        let board_id = layout
            .item(item_id)
            .ok_or(BoardError::ItemNotFound)?
            .board_id
            .clone();
        layout.rename(item_id, name)?;
        let tx = self.conn.unchecked_transaction()?;
        self.upsert_item(layout.item(item_id).unwrap())?;
        self.touch_board(&board_id)?;
        tx.commit()?;
        Ok(())
    }

    fn archive(&self, item_id: &BoardItemId) -> Result<()> {
        self.guard_write()?;
        let mut loaded = self.load_layout_inner()?;
        let mut layout = loaded.rows.pop().unwrap_or_default();
        let board_id = layout
            .item(item_id)
            .ok_or(BoardError::ItemNotFound)?
            .board_id
            .clone();
        layout.archive(item_id)?;
        let tx = self.conn.unchecked_transaction()?;
        self.upsert_item(layout.item(item_id).unwrap())?;
        self.touch_board(&board_id)?;
        tx.commit()?;
        Ok(())
    }

    fn set_collapsed(&self, group_id: &BoardItemId, collapsed: bool) -> Result<()> {
        self.guard_write()?;
        let mut loaded = self.load_layout_inner()?;
        let mut layout = loaded.rows.pop().unwrap_or_default();
        let board_id = layout
            .item(group_id)
            .ok_or(BoardError::ItemNotFound)?
            .board_id
            .clone();
        layout.set_collapsed(group_id, collapsed)?;
        let tx = self.conn.unchecked_transaction()?;
        self.upsert_item(layout.item(group_id).unwrap())?;
        self.touch_board(&board_id)?;
        tx.commit()?;
        Ok(())
    }

    fn set_color(&self, group_id: &BoardItemId, token: &str) -> Result<()> {
        self.guard_write()?;
        let mut loaded = self.load_layout_inner()?;
        let mut layout = loaded.rows.pop().unwrap_or_default();
        let board_id = layout
            .item(group_id)
            .ok_or(BoardError::ItemNotFound)?
            .board_id
            .clone();
        layout.set_color(group_id, token)?;
        let tx = self.conn.unchecked_transaction()?;
        self.upsert_item(layout.item(group_id).unwrap())?;
        self.touch_board(&board_id)?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ids::{AgentId, ConnectionId, SessionId};
    use crate::domain::session::SessionState;
    use crate::persist::control::SqliteControlStore;
    use crate::persist::db::open_db;
    use crate::persist::schema::SCHEMA_VERSION;
    use crate::persist::{ConnectionRecord, ControlStore, StoreMode};
    use rusqlite::Connection;
    use tempfile::tempdir;

    const V2_CONTROL_DDL: &str = r#"
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

    fn board_store() -> (tempfile::TempDir, SqliteBoardStore) {
        let dir = tempdir().unwrap();
        let s = SqliteBoardStore::open(&dir.path().join("lens.db")).unwrap();
        (dir, s)
    }

    fn seed_session(control: &SqliteControlStore, conn: &str, session: &str) {
        control
            .upsert_connection(&ConnectionRecord {
                id: ConnectionId::new(conn),
                base_url: "u".into(),
                auth_kind: "none".into(),
                label: None,
                server_info: None,
                created_at: 1,
            })
            .unwrap();
        let st = SessionState::new(
            ConnectionId::new(conn),
            SessionId::new(session),
            AgentId::new("agent"),
        );
        control.upsert_session(&st, 1).unwrap();
    }

    #[test]
    fn fresh_open_seeds_default_board() {
        let (_d, store) = board_store();
        let layout = store.load_layout().unwrap();
        assert!(layout.is_clean());
        let layout = &layout.rows[0];
        assert_eq!(layout.boards.len(), 1);
        assert_eq!(layout.boards[0].id.as_str(), DEFAULT_BOARD_ID);
        assert_eq!(layout.boards[0].name, DEFAULT_BOARD_NAME);
    }

    #[test]
    fn place_session_round_trips_after_reopen() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("lens.db");
        let conn = ConnectionId::new("conn_1");
        let sid = SessionId::new("conv_1");
        {
            let store = SqliteBoardStore::open(&path).unwrap();
            store
                .place_session(&conn, &sid, &PlacementTarget::default())
                .unwrap();
        }
        let store = SqliteBoardStore::open(&path).unwrap();
        let layout = store.load_layout().unwrap().rows[0].clone();
        let card = layout.find_card(&conn, &sid).expect("card present");
        assert_eq!(card.ordinal, 0);
        assert!(card.parent_item_id.is_none());
    }

    #[test]
    fn place_session_is_idempotent_in_store() {
        let (_d, store) = board_store();
        let conn = ConnectionId::new("conn_1");
        let sid = SessionId::new("conv_1");
        store
            .place_session(&conn, &sid, &PlacementTarget::default())
            .unwrap();
        store
            .place_session(&conn, &sid, &PlacementTarget::default())
            .unwrap();
        let layout = store.load_layout().unwrap().rows[0].clone();
        assert_eq!(layout.items.len(), 1);
    }

    #[test]
    fn mutations_round_trip_tree_shape_and_flags() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("lens.db");
        let conn = ConnectionId::new("conn_1");
        let board_id = BoardId::new(DEFAULT_BOARD_ID);
        {
            let store = SqliteBoardStore::open(&path).unwrap();
            store
                .place_session(&conn, &SessionId::new("s1"), &PlacementTarget::default())
                .unwrap();
            store
                .place_session(&conn, &SessionId::new("s2"), &PlacementTarget::default())
                .unwrap();
            let g = store.create_group(&board_id, None, 1, "grp").unwrap();
            let layout = store.load_layout().unwrap().rows[0].clone();
            let card = layout
                .find_card(&conn, &SessionId::new("s2"))
                .unwrap()
                .id
                .clone();
            store
                .move_item(&card, &board_id, Some(g.clone()), 0)
                .unwrap();
            store.rename(&g, "renamed").unwrap();
            store.set_collapsed(&g, true).unwrap();
            store.set_color(&g, "teal").unwrap();
            store.archive(&g).unwrap();
        }
        let store = SqliteBoardStore::open(&path).unwrap();
        let layout = store.load_layout().unwrap().rows[0].clone();
        let root = layout.children(&board_id, None);
        assert_eq!(root.len(), 2);
        assert!(matches!(
            &root[0].kind,
            BoardItemKind::Card { session, .. } if session.as_str() == "s1"
        ));
        assert!(matches!(&root[1].kind, BoardItemKind::Group { .. }));
        let groups: Vec<_> = layout
            .items
            .iter()
            .filter(|i| matches!(i.kind, BoardItemKind::Group { .. }))
            .collect();
        assert_eq!(groups.len(), 1);
        assert!(matches!(
            &groups[0].kind,
            BoardItemKind::Group {
                name,
                color_token: Some(token),
                collapsed: true,
                archived: true,
            } if name == "renamed" && token == "teal"
        ));
        let in_group = layout.children(&board_id, Some(&groups[0].id));
        assert_eq!(in_group.len(), 1);
    }

    #[test]
    fn ungroup_persists_reparented_children() {
        let (_d, store) = board_store();
        let conn = ConnectionId::new("c");
        let board_id = BoardId::new(DEFAULT_BOARD_ID);
        store
            .place_session(&conn, &SessionId::new("s1"), &PlacementTarget::default())
            .unwrap();
        let g = store.create_group(&board_id, None, 0, "g").unwrap();
        store
            .place_session(
                &conn,
                &SessionId::new("s2"),
                &PlacementTarget {
                    board_id: Some(board_id.clone()),
                    parent_item_id: Some(g.clone()),
                    ordinal: None,
                },
            )
            .unwrap();
        store.ungroup(&g).unwrap();
        let layout = store.load_layout().unwrap().rows[0].clone();
        let root = layout.children(&board_id, None);
        assert_eq!(root.len(), 2);
        assert!(!layout.items.iter().any(|i| i.id == g));
    }

    #[test]
    fn remove_session_deletes_card_row() {
        let (_d, store) = board_store();
        let conn = ConnectionId::new("c");
        let sid = SessionId::new("s1");
        store
            .place_session(&conn, &sid, &PlacementTarget::default())
            .unwrap();
        store.remove_session(&conn, &sid).unwrap();
        let layout = store.load_layout().unwrap().rows[0].clone();
        assert!(layout.find_card(&conn, &sid).is_none());
        let count: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM board_items", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn archive_keeps_group_row_walkable() {
        let (_d, store) = board_store();
        let board_id = BoardId::new(DEFAULT_BOARD_ID);
        let g = store.create_group(&board_id, None, 0, "g").unwrap();
        store.archive(&g).unwrap();
        let count: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM board_items WHERE item_id = ?1",
                [g.as_str()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
        let layout = store.load_layout().unwrap().rows[0].clone();
        assert!(layout.item(&g).is_some());
    }

    #[test]
    fn v2_db_migrates_and_lazy_places_existing_session() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("lens.db");
        {
            let (conn, _) = open_db(&path, V2_CONTROL_DDL, 2).unwrap();
            conn.execute(
                "INSERT INTO connections (id, base_url, auth_kind, created_at) VALUES ('conn_1', 'u', 'none', 1)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO sessions (
                   connection_id, id, agent_id, status, host_type, lifecycle,
                   terminal_pending, created_at, archived, updated_at
                 ) VALUES ('conn_1', 'conv_existing', 'agent', 'idle', 'external', 'active', 0, 1, 0, 1)",
                [],
            )
            .unwrap();
        }
        let control = SqliteControlStore::open(&path).unwrap();
        assert_eq!(control.mode(), StoreMode::ReadWrite);
        let board = SqliteBoardStore::open(&path).unwrap();
        let tables: Vec<String> = board
            .conn
            .prepare(
                "SELECT name FROM sqlite_master WHERE type='table' AND name IN ('boards', 'board_items') ORDER BY name",
            )
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert_eq!(tables, vec!["board_items", "boards"]);
        let layout = board.load_layout().unwrap().rows[0].clone();
        let card = layout
            .find_card(
                &ConnectionId::new("conn_1"),
                &SessionId::new("conv_existing"),
            )
            .expect("lazy placement on first load");
        assert_eq!(card.ordinal, 0);
        assert!(card.parent_item_id.is_none());
        let version: u32 = board
            .conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'schema_version'",
                [],
                |r| r.get::<_, String>(0),
            )
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
    }

    #[test]
    fn corrupt_board_item_row_is_skipped_not_fatal() {
        let (_d, store) = board_store();
        let conn = ConnectionId::new("c");
        store
            .place_session(&conn, &SessionId::new("good"), &PlacementTarget::default())
            .unwrap();
        store
            .conn
            .execute(
                "INSERT INTO board_items (item_id, board_id, ordinal, kind, collapsed, archived, created_at)
                 VALUES ('bad', ?, 9, 'card', 0, 0, 1)",
                [DEFAULT_BOARD_ID],
            )
            .unwrap();
        let loaded = store.load_layout().unwrap();
        assert_eq!(loaded.rows[0].items.len(), 1);
        assert_eq!(loaded.skipped.len(), 1);
        assert_eq!(loaded.skipped[0].id, "bad");
    }

    #[test]
    fn read_only_degraded_refuses_writes() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("lens.db");
        drop(SqliteBoardStore::open(&path).unwrap());
        let c = Connection::open(&path).unwrap();
        c.execute(
            "UPDATE meta SET value = ?1 WHERE key = 'schema_version'",
            [(SCHEMA_VERSION + 1).to_string()],
        )
        .unwrap();
        let store = SqliteBoardStore::open(&path).unwrap();
        assert_eq!(store.mode(), StoreMode::ReadOnlyDegraded);
        assert!(
            store
                .place_session(
                    &ConnectionId::new("c"),
                    &SessionId::new("s"),
                    &PlacementTarget::default()
                )
                .is_err()
        );
    }

    #[test]
    fn tombstoned_session_removal_via_remove_session() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("lens.db");
        let control = SqliteControlStore::open(&path).unwrap();
        seed_session(&control, "conn_1", "conv_1");
        let board = SqliteBoardStore::open(&path).unwrap();
        board
            .place_session(
                &ConnectionId::new("conn_1"),
                &SessionId::new("conv_1"),
                &PlacementTarget::default(),
            )
            .unwrap();
        board
            .remove_session(&ConnectionId::new("conn_1"), &SessionId::new("conv_1"))
            .unwrap();
        control
            .conn
            .execute(
                "UPDATE sessions SET tombstoned_at = 1 WHERE connection_id = 'conn_1' AND id = 'conv_1'",
                [],
            )
            .unwrap();
        let layout = board.load_layout().unwrap().rows[0].clone();
        assert!(
            layout
                .find_card(&ConnectionId::new("conn_1"), &SessionId::new("conv_1"))
                .is_none()
        );
    }

    // Reconcile must prune a card whose session becomes tombstoned AFTER placement
    // (the §4 tombstone-prune branch — distinct from remove_session).
    #[test]
    fn reconcile_prunes_card_when_session_tombstoned() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("lens.db");
        let control = SqliteControlStore::open(&path).unwrap();
        seed_session(&control, "conn_1", "conv_1");
        let board = SqliteBoardStore::open(&path).unwrap();
        board
            .place_session(
                &ConnectionId::new("conn_1"),
                &SessionId::new("conv_1"),
                &PlacementTarget::default(),
            )
            .unwrap();
        assert!(
            board.load_layout().unwrap().rows[0]
                .find_card(&ConnectionId::new("conn_1"), &SessionId::new("conv_1"))
                .is_some(),
            "card present while session is live"
        );
        control
            .conn
            .execute(
                "UPDATE sessions SET tombstoned_at = 1 WHERE connection_id = 'conn_1' AND id = 'conv_1'",
                [],
            )
            .unwrap();
        let layout = board.load_layout().unwrap().rows[0].clone();
        assert!(
            layout
                .find_card(&ConnectionId::new("conn_1"), &SessionId::new("conv_1"))
                .is_none(),
            "reconcile pruned the tombstoned session's card"
        );
        let count: i64 = board
            .conn
            .query_row("SELECT COUNT(*) FROM board_items", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0, "row deleted, not just hidden");
    }

    // Placing a tombstoned session is a no-op — no card row is created (§4).
    #[test]
    fn place_tombstoned_session_is_noop() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("lens.db");
        let control = SqliteControlStore::open(&path).unwrap();
        seed_session(&control, "conn_1", "conv_1");
        control
            .conn
            .execute(
                "UPDATE sessions SET tombstoned_at = 1 WHERE connection_id = 'conn_1' AND id = 'conv_1'",
                [],
            )
            .unwrap();
        let board = SqliteBoardStore::open(&path).unwrap();
        board
            .place_session(
                &ConnectionId::new("conn_1"),
                &SessionId::new("conv_1"),
                &PlacementTarget::default(),
            )
            .unwrap();
        let count: i64 = board
            .conn
            .query_row("SELECT COUNT(*) FROM board_items", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0, "no card created for a tombstoned session");
    }

    fn tombstone_session(store: &SqliteBoardStore, conn: &ConnectionId, session: &SessionId) {
        let path = store.conn.path().expect("db path");
        let control = SqliteControlStore::open(Path::new(path)).unwrap();
        seed_session(&control, conn.as_str(), session.as_str());
        control
            .conn
            .execute(
                &format!(
                    "UPDATE sessions SET tombstoned_at = 1 WHERE connection_id = '{}' AND id = '{}'",
                    conn.as_str(),
                    session.as_str()
                ),
                [],
            )
            .unwrap();
    }

    #[test]
    fn place_sessions_batch_places_all_in_one_pass() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteBoardStore::open(&dir.path().join("lens.db")).unwrap();
        let conn = ConnectionId::new("c1");
        let target = PlacementTarget {
            board_id: None,
            parent_item_id: None,
            ordinal: None,
        };
        store
            .place_sessions(
                &[
                    (conn.clone(), SessionId::new("s1")),
                    (conn.clone(), SessionId::new("s2")),
                    (conn.clone(), SessionId::new("s3")),
                ],
                &target,
            )
            .unwrap();

        let layout = store
            .load_layout()
            .unwrap()
            .rows
            .into_iter()
            .next()
            .unwrap();
        let cards: Vec<_> = layout
            .items
            .iter()
            .filter_map(|i| match &i.kind {
                BoardItemKind::Card { session, .. } => Some(session.as_str().to_string()),
                _ => None,
            })
            .collect();
        assert_eq!(cards.len(), 3);
        assert!(
            ["s1", "s2", "s3"]
                .iter()
                .all(|s| cards.iter().any(|c| c == s))
        );
    }

    // Load-bearing for the batch's CENTRAL purpose (one persist per touched board vs k).
    // `persist_board_items` upserts every item on the board, so ONE batched persist of k
    // new cards writes k times; a naive per-session loop (k separate persists) re-writes the
    // accumulating set 1+2+…+k times. A temp trigger counting board_items writes distinguishes
    // them: this test FAILS if place_sessions regresses to a per-session persist loop.
    #[test]
    fn place_sessions_batch_persists_each_board_once() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteBoardStore::open(&dir.path().join("lens.db")).unwrap();
        store
            .conn
            .execute_batch(
                "CREATE TEMP TABLE _wc (n INTEGER NOT NULL);
                 INSERT INTO _wc VALUES (0);
                 CREATE TEMP TRIGGER _wc_ins AFTER INSERT ON board_items BEGIN UPDATE _wc SET n = n + 1; END;
                 CREATE TEMP TRIGGER _wc_upd AFTER UPDATE ON board_items BEGIN UPDATE _wc SET n = n + 1; END;",
            )
            .unwrap();
        let conn = ConnectionId::new("c1");
        let target = PlacementTarget {
            board_id: None,
            parent_item_id: None,
            ordinal: None,
        };
        store
            .place_sessions(
                &[
                    (conn.clone(), SessionId::new("s1")),
                    (conn.clone(), SessionId::new("s2")),
                    (conn.clone(), SessionId::new("s3")),
                ],
                &target,
            )
            .unwrap();
        let writes: i64 = store
            .conn
            .query_row("SELECT n FROM _wc", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            writes, 3,
            "one persist of 3 new cards = 3 board_items writes; a per-session loop would be 1+2+3=6"
        );
    }

    #[test]
    fn place_sessions_skips_tombstoned_and_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteBoardStore::open(&dir.path().join("lens.db")).unwrap();
        let conn = ConnectionId::new("c1");
        let target = PlacementTarget {
            board_id: None,
            parent_item_id: None,
            ordinal: None,
        };
        store
            .place_session(&conn, &SessionId::new("s1"), &target)
            .unwrap();
        tombstone_session(&store, &conn, &SessionId::new("s2"));

        store
            .place_sessions(
                &[
                    (conn.clone(), SessionId::new("s1")),
                    (conn.clone(), SessionId::new("s2")),
                    (conn.clone(), SessionId::new("s3")),
                ],
                &target,
            )
            .unwrap();

        // Read the RAW persisted rows (load_layout_inner does NOT reconcile). Reading via
        // load_layout() would prune tombstoned cards itself, masking whether the BATCH
        // skipped s2 — this asserts the batch's own tombstone check (board.rs place_sessions).
        let layout = store
            .load_layout_inner()
            .unwrap()
            .rows
            .into_iter()
            .next()
            .unwrap();
        let mut cards: Vec<String> = layout
            .items
            .iter()
            .filter_map(|i| match &i.kind {
                BoardItemKind::Card { session, .. } => Some(session.as_str().to_string()),
                _ => None,
            })
            .collect();
        cards.sort();
        assert_eq!(
            cards,
            vec!["s1".to_string(), "s3".to_string()],
            "batch places s1 (dup no-op) + s3; tombstoned s2 skipped by the batch, not by load reconcile"
        );
    }

    // Regression for the high-water-mark seed: after delete+reopen, a freshly minted
    // item_id must not reuse a surviving row's seq (the old COUNT(*) seed could).
    #[test]
    fn item_id_seq_not_reused_after_delete_reopen() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("lens.db");
        let c = ConnectionId::new("c");
        {
            let store = SqliteBoardStore::open(&path).unwrap();
            for sid in ["s0", "s1", "s2"] {
                store
                    .place_session(&c, &SessionId::new(sid), &PlacementTarget::default())
                    .unwrap();
            }
            store.remove_session(&c, &SessionId::new("s0")).unwrap();
        }
        let survivors: Vec<String> = {
            let store = SqliteBoardStore::open(&path).unwrap();
            store.load_layout().unwrap().rows[0]
                .items
                .iter()
                .map(|i| i.id.as_str().to_string())
                .collect()
        };
        let seq_of = |id: &str| -> u64 { id.rsplit('_').nth(1).unwrap().parse().unwrap() };
        let max_survivor_seq = survivors.iter().map(|s| seq_of(s)).max().unwrap();

        let store = SqliteBoardStore::open(&path).unwrap();
        store
            .place_session(&c, &SessionId::new("s_new"), &PlacementTarget::default())
            .unwrap();
        let layout = store.load_layout().unwrap().rows[0].clone();
        for sid in ["s1", "s2", "s_new"] {
            assert!(
                layout.find_card(&c, &SessionId::new(sid)).is_some(),
                "session {sid} kept its card"
            );
        }
        let ids: Vec<&str> = layout.items.iter().map(|i| i.id.as_str()).collect();
        let unique: HashSet<_> = ids.iter().collect();
        assert_eq!(ids.len(), unique.len(), "no duplicate item_ids: {ids:?}");
        let new_seq = seq_of(
            layout
                .find_card(&c, &SessionId::new("s_new"))
                .unwrap()
                .id
                .as_str(),
        );
        assert!(
            new_seq > max_survivor_seq,
            "new seq {new_seq} must exceed max survivor seq {max_survivor_seq}"
        );
    }

    #[test]
    fn nested_ungroup_persists() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("lens.db");
        let board = BoardId::new(DEFAULT_BOARD_ID);
        let c = ConnectionId::new("c");
        let g2;
        {
            let s = SqliteBoardStore::open(&path).unwrap();
            let g1 = s.create_group(&board, None, 0, "g1").unwrap();
            g2 = s.create_group(&board, Some(g1.clone()), 0, "g2").unwrap();
            s.place_session(
                &c,
                &SessionId::new("s1"),
                &PlacementTarget {
                    board_id: Some(board.clone()),
                    parent_item_id: Some(g2.clone()),
                    ordinal: None,
                },
            )
            .unwrap();
            s.ungroup(&g1).unwrap();
        }
        let s = SqliteBoardStore::open(&path).unwrap();
        let layout = s.load_layout().unwrap().rows[0].clone();
        let root = layout.children(&board, None);
        assert_eq!(root.len(), 1, "g2 reparented to root");
        assert_eq!(root[0].id, g2);
        let in_g2 = layout.children(&board, Some(&g2));
        assert_eq!(in_g2.len(), 1, "card still under g2");
        assert!(matches!(
            &in_g2[0].kind,
            BoardItemKind::Card { session, .. } if session.as_str() == "s1"
        ));
    }

    #[test]
    fn remove_middle_card_in_group_renumbers() {
        let (_d, s) = board_store();
        let board = BoardId::new(DEFAULT_BOARD_ID);
        let c = ConnectionId::new("c");
        let g = s.create_group(&board, None, 0, "g").unwrap();
        for sid in ["a", "b", "c"] {
            s.place_session(
                &c,
                &SessionId::new(sid),
                &PlacementTarget {
                    board_id: Some(board.clone()),
                    parent_item_id: Some(g.clone()),
                    ordinal: None,
                },
            )
            .unwrap();
        }
        s.remove_session(&c, &SessionId::new("b")).unwrap();
        let layout = s.load_layout().unwrap().rows[0].clone();
        let kids = layout.children(&board, Some(&g));
        let sessions: Vec<&str> = kids
            .iter()
            .map(|i| match &i.kind {
                BoardItemKind::Card { session, .. } => session.as_str(),
                _ => "?",
            })
            .collect();
        assert_eq!(sessions, vec!["a", "c"]);
        assert_eq!(
            kids.iter().map(|i| i.ordinal).collect::<Vec<_>>(),
            vec![0, 1]
        );
    }

    #[test]
    fn move_card_out_of_group_renumbers_both_sibling_sets() {
        let (_d, s) = board_store();
        let board = BoardId::new(DEFAULT_BOARD_ID);
        let c = ConnectionId::new("c");
        for sid in ["L0", "L1"] {
            s.place_session(&c, &SessionId::new(sid), &PlacementTarget::default())
                .unwrap();
        }
        let g = s.create_group(&board, None, 2, "g").unwrap();
        for sid in ["x", "y"] {
            s.place_session(
                &c,
                &SessionId::new(sid),
                &PlacementTarget {
                    board_id: Some(board.clone()),
                    parent_item_id: Some(g.clone()),
                    ordinal: None,
                },
            )
            .unwrap();
        }
        let x = s.load_layout().unwrap().rows[0]
            .find_card(&c, &SessionId::new("x"))
            .unwrap()
            .id
            .clone();
        s.move_item(&x, &board, None, 0).unwrap();
        let layout = s.load_layout().unwrap().rows[0].clone();
        let root_ids: Vec<&str> = layout
            .children(&board, None)
            .iter()
            .map(|i| match &i.kind {
                BoardItemKind::Card { session, .. } => session.as_str(),
                BoardItemKind::Group { name, .. } => name.as_str(),
            })
            .collect();
        assert_eq!(root_ids, vec!["x", "L0", "L1", "g"]);
        assert_eq!(
            layout
                .children(&board, None)
                .iter()
                .map(|i| i.ordinal)
                .collect::<Vec<_>>(),
            vec![0, 1, 2, 3]
        );
        let in_g = layout.children(&board, Some(&g));
        assert_eq!(in_g.len(), 1, "only y remains in group");
        assert_eq!(in_g[0].ordinal, 0, "y renumbered to 0");
    }

    #[test]
    fn place_explicit_ordinal_shifts_siblings() {
        let (_d, s) = board_store();
        let board = BoardId::new(DEFAULT_BOARD_ID);
        let c = ConnectionId::new("c");
        for sid in ["a", "b"] {
            s.place_session(&c, &SessionId::new(sid), &PlacementTarget::default())
                .unwrap();
        }
        s.place_session(
            &c,
            &SessionId::new("mid"),
            &PlacementTarget {
                board_id: Some(board.clone()),
                parent_item_id: None,
                ordinal: Some(1),
            },
        )
        .unwrap();
        let layout = s.load_layout().unwrap().rows[0].clone();
        let ids: Vec<&str> = layout
            .children(&board, None)
            .iter()
            .map(|i| match &i.kind {
                BoardItemKind::Card { session, .. } => session.as_str(),
                _ => "?",
            })
            .collect();
        assert_eq!(ids, vec!["a", "mid", "b"]);
    }
}
