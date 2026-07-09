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
        ctx: BlockContext {
            agent: None,
            depth: 0,
            turn: 0,
        },
        created_at: 1,
        kind: ItemKind::Message {
            role: Role::User,
            content: vec![ContentBlock {
                kind: "text".into(),
                text: Some("hi".into()),
                data: serde_json::Value::Null,
            }],
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
    let loaded = s.load_items().unwrap();
    assert!(loaded.is_clean());
    let items = loaded.rows;
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
    drop(
        SqliteTranscriptStore::open(&tpath, &ConnectionId::new("c"), &SessionId::new("s")).unwrap(),
    );
    bump_version(&tpath);
    let ts =
        SqliteTranscriptStore::open(&tpath, &ConnectionId::new("c"), &SessionId::new("s")).unwrap();
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
