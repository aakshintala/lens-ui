//! P3 Task 0 — large-transcript latency spike (D12). THROWAWAY.
//!
//! Measures three latencies against a synthetic ~500 MiB / ~100k-item
//! `SqliteTranscriptStore` file with a BIMODAL item-size mix (D11):
//!   1. windowed page-load (scroll-back) — prototype primitive (does not exist yet)
//!   2. byte-budgeted cold-hydrate tail (Slept→focus) — prototype primitive
//!   3. reconcile-by-id scope: full-history (shipped impl) vs tail-bounded (prototype)
//!
//! Reuses the SHIPPED `Item`/`ItemKind` serde + `SqliteTranscriptStore` +
//! `row_to_item`, so numbers reflect real code paths. Generation uses a raw
//! batched transaction (the shipped `upsert_item` autocommits one txn/row — far
//! too slow for 100k rows; we note this in the doc).
//!
//! Usage:
//!   large-transcript            # generate if missing, then bench
//!   large-transcript gen        # (re)generate only
//!   large-transcript bench      # bench only (requires an existing db)

use lens_core::domain::ids::{ConnectionId, ItemId, SessionId};
use lens_core::domain::item::{BlockContext, ContentBlock, Item, ItemKind};
use lens_core::domain::scalars::Role;
use lens_core::persist::SqliteTranscriptStore;
use lens_core::persist::TranscriptStore;
use lens_core::persist::map::{item_kind_token, row_to_item};
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::time::Instant;

// ── synthesis parameters (D11 premise) ──────────────────────────────────────
const N_ITEMS: i64 = 100_000;
// Bimodal upper mode. D11 names ~200 KB dumps; we hold the DUMP SIZE fixed (it
// is the parameter the byte-window math cares about) and let the FRACTION fall
// out of the 500 MiB × 100k target. 5% × 200 KB alone would be ~1 GiB, so at
// 100k items / 500 MiB the large fraction lands at ~2.6%, not 5% — reported.
const LARGE_PAYLOAD_TEXT_BYTES: usize = 200_000;
const SMALL_PAYLOAD_TEXT_BYTES: usize = 60; // → ~130 B on-wire ItemKind json
// One large dump every LARGE_EVERY items → ~2.56% large, ~500 MiB total.
const LARGE_EVERY: i64 = 39;

const TARGET_TAIL_BYTES: usize = 8 * 1024 * 1024; // D11 ~8 MB resident tail
const ITERS: usize = 30; // ≥20 per the brief

fn db_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("data")
        .join("conv_spike.db")
}

fn conn_id() -> ConnectionId {
    ConnectionId::new("conn_spike")
}
fn sess_id() -> SessionId {
    SessionId::new("conv_spike")
}

// A large dump item (e.g. a 200 KB tool output / file paste).
fn large_item(i: i64) -> Item {
    let text = "x".repeat(LARGE_PAYLOAD_TEXT_BYTES);
    Item {
        id: ItemId::new(format!("item_{i:08}")),
        seq: Some(i as u64),
        ctx: BlockContext {
            agent: Some("coder".into()),
            depth: 0,
            turn: (i / 4) as u32,
        },
        created_at: 1_700_000_000_000 + i,
        kind: ItemKind::FunctionCallOutput {
            call_id: lens_core::domain::ids::CallId::new(format!("call_{i:08}")),
            output: text,
            arguments: serde_json::Value::Null,
        },
    }
}

// A small marker item (~100 B: a short assistant message / status marker).
fn small_item(i: i64) -> Item {
    let text = "m".repeat(SMALL_PAYLOAD_TEXT_BYTES);
    Item {
        id: ItemId::new(format!("item_{i:08}")),
        seq: Some(i as u64),
        ctx: BlockContext {
            agent: Some("coder".into()),
            depth: 0,
            turn: (i / 4) as u32,
        },
        created_at: 1_700_000_000_000 + i,
        kind: ItemKind::Message {
            role: Role::Assistant,
            content: vec![ContentBlock {
                kind: "text".into(),
                text: Some(text),
                data: serde_json::Value::Null,
            }],
        },
    }
}

fn make_item(i: i64) -> Item {
    if i % LARGE_EVERY == 0 {
        large_item(i)
    } else {
        small_item(i)
    }
}

/// The windowed page-load / hydrate SELECT column list. MUST match the order
/// `row_to_item` reads: item_id, live_seq, kind, payload, agent, depth, turn,
/// created_at.
const ITEM_COLS: &str = "item_id, live_seq, kind, payload, agent, depth, turn, created_at";

fn generate() {
    let path = db_path();
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p).unwrap();
    }
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(format!("{}-wal", path.display()));
    let _ = std::fs::remove_file(format!("{}-shm", path.display()));

    // Open via the shipped store so the file is a real, self-describing
    // TranscriptStore (correct meta + DDL + WAL).
    {
        let _ = SqliteTranscriptStore::open(&path, &conn_id(), &sess_id()).unwrap();
    }

    // Bulk-insert in ONE transaction via a raw connection — the shipped
    // `upsert_item` autocommits one txn/row (fsync storm on WAL) which is
    // pathological for 100k rows. Payload encoding is identical to the store's.
    let mut conn = Connection::open(&path).unwrap();
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")
        .unwrap();

    let t0 = Instant::now();
    let mut total_payload_bytes: u64 = 0;
    let mut n_large: i64 = 0;
    let tx = conn.transaction().unwrap();
    {
        let mut stmt = tx
            .prepare(
                "INSERT INTO items \
                 (item_id, live_seq, ordinal, kind, payload, agent, depth, turn, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )
            .unwrap();
        for i in 0..N_ITEMS {
            let item = make_item(i);
            if i % LARGE_EVERY == 0 {
                n_large += 1;
            }
            let payload = serde_json::to_string(&item.kind).unwrap();
            total_payload_bytes += payload.len() as u64;
            stmt.execute(rusqlite::params![
                item.id.as_str(),
                item.seq.map(|v| v as i64),
                i, // ordinal == append index
                item_kind_token(&item.kind),
                payload,
                item.ctx.agent,
                item.ctx.depth as i64,
                item.ctx.turn as i64,
                item.created_at,
            ])
            .unwrap();
        }
    }
    tx.commit().unwrap();
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .unwrap();
    let gen_dur = t0.elapsed();

    let file_bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    println!("── generation ──");
    println!("  rows:                {N_ITEMS}");
    println!(
        "  large dumps:         {n_large} ({:.2}%)",
        100.0 * n_large as f64 / N_ITEMS as f64
    );
    println!("  large payload text:  {LARGE_PAYLOAD_TEXT_BYTES} B");
    println!("  small payload text:  {SMALL_PAYLOAD_TEXT_BYTES} B");
    println!(
        "  total payload bytes: {} ({:.1} MiB)",
        total_payload_bytes,
        total_payload_bytes as f64 / 1_048_576.0
    );
    println!(
        "  on-disk file:        {} ({:.1} MiB)",
        file_bytes,
        file_bytes as f64 / 1_048_576.0
    );
    println!(
        "  generation time:     {:.2} s (single batched txn)",
        gen_dur.as_secs_f64()
    );
}

// ── measurement helpers ──────────────────────────────────────────────────────

/// Run `f` ITERS times (after `warm` warmups); return (p50, p90) micros.
fn bench<F: FnMut() -> usize>(warm: usize, mut f: F) -> (f64, f64, usize) {
    for _ in 0..warm {
        std::hint::black_box(f());
    }
    let mut samples = Vec::with_capacity(ITERS);
    let mut last_n = 0;
    for _ in 0..ITERS {
        let t = Instant::now();
        last_n = std::hint::black_box(f());
        samples.push(t.elapsed().as_secs_f64() * 1e6);
    }
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p = |q: f64| samples[((samples.len() as f64 * q) as usize).min(samples.len() - 1)];
    (p(0.50), p(0.90), last_n)
}

/// Prototype: windowed page-load by ordinal. Loads ONE page of the `limit`
/// items immediately BEFORE `before_ordinal`, decoded to `Item`s (ascending).
fn load_page(conn: &Connection, before_ordinal: i64, limit: i64) -> Vec<Item> {
    let sql =
        format!("SELECT {ITEM_COLS} FROM items WHERE ordinal < ?1 ORDER BY ordinal DESC LIMIT ?2");
    let mut stmt = conn.prepare_cached(&sql).unwrap();
    let mut rows = stmt
        .query(rusqlite::params![before_ordinal, limit])
        .unwrap();
    let mut out = Vec::with_capacity(limit as usize);
    while let Some(r) = rows.next().unwrap() {
        out.push(row_to_item(r).unwrap());
    }
    out.reverse(); // present ascending
    out
}

/// Prototype: byte-budgeted cold-hydrate tail. Walks newest→oldest, decoding
/// until cumulative payload bytes exceed `budget`. Only touches the tail rows
/// (early cursor break over the ordinal index) — NOT a full-table scan.
fn load_tail_by_bytes(conn: &Connection, budget: usize) -> (Vec<Item>, usize) {
    let sql = format!("SELECT {ITEM_COLS}, length(payload) FROM items ORDER BY ordinal DESC");
    let mut stmt = conn.prepare_cached(&sql).unwrap();
    let mut rows = stmt.query([]).unwrap();
    let mut out = Vec::new();
    let mut acc = 0usize;
    while let Some(r) = rows.next().unwrap() {
        let plen: i64 = r.get(8).unwrap();
        out.push(row_to_item(r).unwrap());
        acc += plen as usize;
        if acc >= budget {
            break;
        }
    }
    out.reverse();
    (out, acc)
}

/// Prototype: TAIL-BOUNDED reconcile-by-id. Reconciles only the ordinal range
/// `>= tail_start` against `tail_truth` (upsert by item_id at ordinal =
/// tail_start + i; delete tail rows absent from truth). Touches only the tail,
/// never the full history. Mirrors the shipped `reconcile`'s park-negative /
/// re-stamp / delete-untouched transaction, scoped to the tail.
fn reconcile_tail(conn: &Connection, tail_start: i64, tail_truth: &[Item]) {
    conn.execute("BEGIN", []).unwrap();
    // Park only the tail ordinals out of the way.
    conn.execute(
        "UPDATE items SET ordinal = -1 - ordinal WHERE ordinal >= ?1",
        rusqlite::params![tail_start],
    )
    .unwrap();
    let mut up = conn
        .prepare_cached(
            "INSERT INTO items \
             (item_id, live_seq, ordinal, kind, payload, agent, depth, turn, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) \
             ON CONFLICT(item_id) DO UPDATE SET \
               live_seq=excluded.live_seq, ordinal=excluded.ordinal, kind=excluded.kind, \
               payload=excluded.payload, agent=excluded.agent, depth=excluded.depth, \
               turn=excluded.turn, created_at=excluded.created_at",
        )
        .unwrap();
    for (i, item) in tail_truth.iter().enumerate() {
        let ordinal = tail_start + i as i64;
        let payload = serde_json::to_string(&item.kind).unwrap();
        up.execute(rusqlite::params![
            item.id.as_str(),
            item.seq.map(|v| v as i64),
            ordinal,
            item_kind_token(&item.kind),
            payload,
            item.ctx.agent,
            item.ctx.depth as i64,
            item.ctx.turn as i64,
            item.created_at,
        ])
        .unwrap();
    }
    drop(up);
    // Delete tail rows the truth did not touch (still parked negative). Non-tail
    // rows kept their positive ordinals and are untouched.
    conn.execute("DELETE FROM items WHERE ordinal < 0", [])
        .unwrap();
    conn.execute("COMMIT", []).unwrap();
}

fn bench_all() {
    let path = db_path();
    let file_bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    println!(
        "\n── measurement (db {:.1} MiB) ──",
        file_bytes as f64 / 1_048_576.0
    );

    let conn = Connection::open(&path).unwrap();
    let store = SqliteTranscriptStore::open(&path, &conn_id(), &sess_id()).unwrap();

    let max_ord: i64 = conn
        .query_row("SELECT MAX(ordinal) FROM items", [], |r| r.get(0))
        .unwrap();

    // Warm the OS page cache + sqlite so we measure warm-cache latency (noted in
    // doc). A full table count forces every page through cache once.
    let _cnt: i64 = conn
        .query_row("SELECT COUNT(*) FROM items", [], |r| r.get(0))
        .unwrap();

    println!(
        "\n[M1] windowed page-load (scroll-back), page from mid-history (ordinal ~{}):",
        max_ord / 2
    );
    for &sz in &[50i64, 200] {
        let mid = max_ord / 2;
        let (p50, p90, n) = bench(5, || load_page(&conn, mid, sz).len());
        println!("  page {sz:>4} items: p50 {p50:8.1} µs  p90 {p90:8.1} µs  ({n} items loaded)");
    }
    // Byte-budgeted page variant (~512 KiB) from mid-history.
    {
        let budget = 512 * 1024;
        let (p50, p90, n) = bench(5, || {
            // page backward from mid until ~512 KiB, capped at 500 rows
            let sql = format!(
                "SELECT {ITEM_COLS}, length(payload) FROM items WHERE ordinal < ?1 ORDER BY ordinal DESC LIMIT 500"
            );
            let mut stmt = conn.prepare_cached(&sql).unwrap();
            let mut rows = stmt.query(rusqlite::params![max_ord / 2]).unwrap();
            let mut acc = 0usize;
            let mut c = 0usize;
            while let Some(r) = rows.next().unwrap() {
                let plen: i64 = r.get(8).unwrap();
                std::hint::black_box(row_to_item(r).unwrap());
                acc += plen as usize;
                c += 1;
                if acc >= budget {
                    break;
                }
            }
            c
        });
        println!("  byte-budget ~512 KiB: p50 {p50:8.1} µs  p90 {p90:8.1} µs  ({n} items)");
    }

    println!("\n[M2] byte-budgeted cold-hydrate tail (~8 MB):");
    let (p50, p90, n) = bench(5, || load_tail_by_bytes(&conn, TARGET_TAIL_BYTES).0.len());
    let (tail, acc) = load_tail_by_bytes(&conn, TARGET_TAIL_BYTES);
    let n_large_tail = tail
        .iter()
        .filter(|it| matches!(it.kind, ItemKind::FunctionCallOutput { .. }))
        .count();
    println!("  p50 {p50:8.1} µs  p90 {p90:8.1} µs");
    println!(
        "  → 8 MB tail buys {n} items ({} large dumps + {} small), {:.1} MiB decoded",
        n_large_tail,
        n - n_large_tail,
        acc as f64 / 1_048_576.0
    );

    println!("\n[M3a] FULL-HISTORY reconcile (shipped impl, 100k rows):");
    // Load the full truth once (this is itself the O(transcript) load).
    let t = Instant::now();
    let all = store.load_items().unwrap().rows;
    let load_all_ms = t.elapsed().as_secs_f64() * 1e3;
    println!(
        "  (baseline: load_items() all {} rows = {:.1} ms)",
        all.len(),
        load_all_ms
    );
    // Fewer iters — each full reconcile rewrites 100k rows in a txn.
    {
        let iters = 5usize;
        let mut samples = Vec::new();
        // warm
        store.reconcile(&all).unwrap();
        for _ in 0..iters {
            let t = Instant::now();
            store.reconcile(&all).unwrap();
            samples.push(t.elapsed().as_secs_f64() * 1e3);
        }
        samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let p50 = samples[samples.len() / 2];
        let p90 = samples[(samples.len() as f64 * 0.9) as usize];
        println!(
            "  p50 {p50:8.1} ms  p90 {p90:8.1} ms  (n={iters}; identity reconcile of all rows)"
        );
    }

    println!("\n[M3b] TAIL-BOUNDED reconcile (prototype, last 50 / 200 / 500 items):");
    for &t_len in &[50i64, 200, 500] {
        let tail_start = max_ord + 1 - t_len;
        // Build the tail truth = the current tail items (identity reconcile).
        let tail_truth = load_page(&conn, max_ord + 1, t_len);
        assert_eq!(tail_truth.len() as i64, t_len);
        let (p50, p90, _n) = bench(3, || {
            reconcile_tail(&conn, tail_start, &tail_truth);
            tail_truth.len()
        });
        println!("  tail {t_len:>4} items: p50 {p50:8.1} µs  p90 {p90:8.1} µs");
    }
}

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "auto".into());
    let path = db_path();
    match mode.as_str() {
        "gen" => generate(),
        "bench" => bench_all(),
        _ => {
            if !path.exists() {
                generate();
            } else {
                let sz = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                println!(
                    "(reusing existing db, {:.1} MiB — `gen` to rebuild)",
                    sz as f64 / 1_048_576.0
                );
            }
            bench_all();
        }
    }
}
