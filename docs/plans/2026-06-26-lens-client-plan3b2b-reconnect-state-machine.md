# lens-client Plan 3b-2b — §7 reconnect state machine

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the SSE reader thread reconnect-safe end-to-end inside the crate: on a transport drop it backs off, re-reads the session snapshot + items, and resumes the live stream — emitting the synthetic lifecycle markers (`Reconnecting`/`Reconnected`/`SnapshotRestored`/`Disconnected`) on the existing mpsc channel so the consumer stays purely event-driven and never sees raw reconnect mechanics.

**Architecture:** The reader gains a `Reopen` capability (a `Send` trait: open a fresh stream body, fetch the snapshot, fetch items) injected at `Sessions::stream`. The existing `Err(_) => return` seam becomes a reconnect loop. Bucket-B chrome restore is emitted as **one** synthetic `ServerStreamEvent::SnapshotRestored(SessionSnapshot)` (decision A2, typed-client §7), bucket-A history is replayed as `OutputItemDone`, and the live overlap is de-duplicated by `sequence_number`. The `Reopen` trait makes the whole state machine unit-testable with a scripted mock — no server.

**Tech Stack:** Rust 2024, `reqwest::blocking`, `std::sync::mpsc`, `std::thread`. No async runtime (D2). `serde`/`serde_json`.

## Global Constraints

- **Pin:** omnigent `0.3.0.dev0` (`36b2a11c`), frozen per `docs/adr/0001-omnigent-contract-pinning.md`. Ground truth: `docs/spikes/captures/2026-06-26-sse/`.
- **No `Value` to consumers.** Public surfaces expose typed fields/getters only (the snapshot rides inside `SnapshotRestored` as the already-typed `SessionSnapshot`).
- **`generated.rs` is never hand-edited.** Codegen output only.
- **The UI never panics** (AGENTS.md). `parse_event` is total; the reconnect loop maps every error to a stream value, never an `unwrap`/`panic`.
- **No async runtime, no tokio, no flume** (D2). One blocking OS thread per stream; `std::sync::mpsc` to the UI poller.
- **Sync/blocking public API.** `Sessions::stream` stays blocking and returns an `EventStream` whose reader thread is already running.
- **Verification per task:** `cargo test -p lens-client` (serverless, always green) + `cargo clippy -p lens-client --all-targets -- -D warnings` + `cargo fmt --check`. Live (`--features live-tests`) is opt-in and NOT required to land a task.
- **Cross-family review:** this is temporal/stateful code — route the consolidated end-of-plan review through a non-author family (`gpt-5.5`/`gemini-3.5`), per `[[composer-delegation-profile]]` and `[[review-spend-policy]]`.

---

## Design decisions this plan pins (surface at review; reconcile into §7)

These are implementation-level calls the design left open. They are deliberate and flagged so review can reject any one independently:

1. **`Disconnected { reason }` carries a typed reason.** §7 pinned the *name* `Disconnected` but not its payload. The §7 stop-immediately table needs four distinct app actions (re-auth / access-denied / remove / surface-failure) plus retries-exhausted, so an opaque marker is insufficient. We add `enum DisconnectReason { Unauthorized, Forbidden, NotFound, SessionFailed, RetriesExhausted }`. It stays a *stream value*, not a `ClientError` (honoring "give-up cannot be a synchronous `Result`").
2. **`gap` is `None` unless the live overlap proves contiguity.** §7 defines `gap = Some(0)` (clean) vs `Some(N>0)`/`None` (missed). We cannot prove zero-loss from the snapshot alone (it carries no seq). So this plan emits `Reconnected { gap: None }` on every reconnect **unless** the first post-reopen live frame's `sequence_number == last_seen_seq + 1`, in which case `gap = Some(0)`. `None` is always safe (it only clears transient accumulators the drop likely lost anyway). This realizes §7's semantics without a fragile heuristic; `Some(N>0)` is never synthesized (folded into `None`).
3. **Seq-dedup reads `sequence_number` off the raw frame, not the typed event.** Typed events strip `sequence_number` (only `Heartbeat` exposes it). Rather than thread seq through every variant, the reader peeks `sequence_number` off each `SseFrame`'s JSON (cheap) to maintain `last_seen_seq` and to drop the post-reopen overlap. The typed event surface stays clean.
4. **Items replay is single-page, newest-after for v1.** `Reopener::items` fetches one page (server default order). If `has_more` is true the plan replays what it got and logs nothing further; deep backfill/pagination is deferred (the reducer merges by `id`, so a later live event or a subsequent reconnect fills gaps). Flagged for a follow-up if captures show truncation in practice.

---

## File structure

- **Create** `crates/lens-client/src/reconnect.rs` — the `Reopen` trait, the real `HttpReopener` (clones `conn` + `http` + `SessionId`; `Send + 'static`), the backoff schedule, and `items_to_replay`.
- **Modify** `crates/lens-client/src/stream/event.rs` — add the four synthetic `ServerStreamEvent` variants + `DisconnectReason`; add `SessionSnapshot`/nested `PartialEq`.
- **Modify** `crates/lens-client/src/stream/sse.rs` — add `SseFrame::sequence_number()` peek helper (or a free fn in `reconnect.rs` if it keeps `sse` dependency-free).
- **Modify** `crates/lens-client/src/stream/normalize.rs` — add `Normalizer::reset_seen_items()`.
- **Modify** `crates/lens-client/src/stream/reader.rs` — `run` becomes generic over `Reopen` + an injected `sleep`; `Err(_)` seam drives the reconnect state machine; seq tracking + overlap dedup.
- **Modify** `crates/lens-client/src/sessions.rs` — `SessionSnapshot`/`ModelUsage`/`SkillRef` derive `PartialEq`; `Sessions::stream` builds an `HttpReopener` and passes it to `EventStream::spawn`.
- **Modify** `crates/lens-client/src/lib.rs` — `mod reconnect;` (and any re-exports).

---

### Task 1: Synthetic lifecycle variants on `ServerStreamEvent`

**Files:**
- Modify: `crates/lens-client/src/stream/event.rs`
- Modify: `crates/lens-client/src/sessions.rs` (derive `PartialEq` on `SessionSnapshot` + nested)
- Test: inline `#[cfg(test)]` in `event.rs`

**Interfaces:**
- Produces: `ServerStreamEvent::{Reconnecting { attempt: u32 }, Reconnected { gap: Option<u64> }, SnapshotRestored(Box<SessionSnapshot>), Disconnected { reason: DisconnectReason }}`; `pub enum DisconnectReason { Unauthorized, Forbidden, NotFound, SessionFailed, RetriesExhausted }`. (`Box` keeps the enum small — `SessionSnapshot` is large.)

- [ ] **Step 1: Make `SessionSnapshot` and its nested types `PartialEq`.** In `sessions.rs`, add `PartialEq` to the derives on `SessionSnapshot`, `ModelUsage`, and `SkillRef` (and any other struct `SessionSnapshot` owns by value that is not already `PartialEq`). Example:

```rust
#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
pub struct SessionSnapshot {
    // … unchanged fields …
}
```

- [ ] **Step 2: Write the failing test** (in `event.rs` tests):

```rust
#[test]
fn synthetic_lifecycle_variants_exist_and_compare() {
    let a = ServerStreamEvent::Reconnecting { attempt: 2 };
    let b = ServerStreamEvent::Reconnected { gap: None };
    let c = ServerStreamEvent::Disconnected { reason: DisconnectReason::NotFound };
    assert_eq!(a, ServerStreamEvent::Reconnecting { attempt: 2 });
    assert_ne!(b, ServerStreamEvent::Reconnected { gap: Some(0) });
    assert_ne!(c, ServerStreamEvent::Disconnected { reason: DisconnectReason::Unauthorized });
}
```

- [ ] **Step 3: Run it, verify it fails to compile** (`Reconnecting` unknown).

Run: `cargo test -p lens-client synthetic_lifecycle_variants_exist_and_compare`
Expected: compile error — no variant `Reconnecting`.

- [ ] **Step 4: Add the variants + `DisconnectReason`.** In `event.rs`, extend the enum (keep existing variants):

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum ServerStreamEvent {
    Session(SessionEvent),
    Response(ResponseEvent),
    /// Crate-synthetic: a reconnect attempt is in flight (typed-client §7 step 2).
    Reconnecting { attempt: u32 },
    /// Crate-synthetic: stream re-opened. `gap` per §7 / plan decision 2:
    /// `Some(0)` = provably contiguous overlap; `None` = clear transient state.
    Reconnected { gap: Option<u64> },
    /// Crate-synthetic: bucket-B chrome restore (decision A2, typed-client §7).
    /// Emitted after `Reconnected`, before replayed history. Boxed (large payload).
    SnapshotRestored(Box<crate::sessions::SessionSnapshot>),
    /// Crate-synthetic: terminal. Last event before the channel closes (§7 step 3).
    Disconnected { reason: DisconnectReason },
    Unknown { event_type: String },
}

/// Why the stream gave up (typed-client §7 stop-immediately table + retries-exhausted).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisconnectReason {
    Unauthorized,    // 401 — re-auth
    Forbidden,       // 403 — access denied, remove session
    NotFound,        // 404 — session deleted, remove
    SessionFailed,   // snapshot status == failed — surface, no retry
    RetriesExhausted // backoff window elapsed
}
```

Ensure `DisconnectReason` is re-exported wherever `ServerStreamEvent` is (mirror the existing `pub use` in `stream/mod.rs`).

- [ ] **Step 5: Run the test, verify it passes.**

Run: `cargo test -p lens-client synthetic_lifecycle_variants_exist_and_compare`
Expected: PASS.

- [ ] **Step 6: Commit.**

```bash
git add crates/lens-client/src/stream/event.rs crates/lens-client/src/stream/mod.rs crates/lens-client/src/sessions.rs
git commit -m "feat(lens-client): synthetic reconnect-lifecycle ServerStreamEvent variants"
```

---

### Task 2: `Normalizer::reset_seen_items` (gap reset seam)

**Files:**
- Modify: `crates/lens-client/src/stream/normalize.rs`
- Test: inline tests in `normalize.rs`

**Interfaces:**
- Produces: `pub(crate) fn Normalizer::reset_seen_items(&mut self)` — clears `seen_items` so replayed `GET /items` history is not suppressed as an already-seen re-fire (typed-client §7 seam (a)). Reasoning accumulator is NOT touched (a reconnect mid-reasoning is closed by the reader's drop handling, not here).

- [ ] **Step 1: Write the failing test:**

```rust
#[test]
fn reset_seen_items_allows_a_previously_seen_item_through() {
    let mut n = Normalizer::default();
    let first = fn_call("toolu_1", "completed", "fc_a");
    assert_eq!(n.push(first.clone()), vec![first.clone()]);
    // Without reset, an identical re-fire is suppressed:
    assert!(n.push(fn_call("toolu_1", "completed", "fc_b")).is_empty());
    // After reset (reconnect with gap != Some(0)), the same item replays:
    n.reset_seen_items();
    let replay = fn_call("toolu_1", "completed", "fc_c");
    assert_eq!(n.push(replay.clone()), vec![replay]);
}
```

- [ ] **Step 2: Run it, verify it fails** (`reset_seen_items` undefined).

Run: `cargo test -p lens-client reset_seen_items_allows`
Expected: FAIL — no method `reset_seen_items`.

- [ ] **Step 3: Implement** in `impl Normalizer`:

```rust
/// Clear the `OutputItemDone` dedup set. Called by the reader on
/// `Reconnected { gap }` when `gap != Some(0)`, so `GET /items` history
/// replay is not wrongly suppressed (typed-client §7 seam (a)).
pub(crate) fn reset_seen_items(&mut self) {
    self.seen_items.clear();
}
```

- [ ] **Step 4: Run the test, verify it passes.**

Run: `cargo test -p lens-client reset_seen_items_allows`
Expected: PASS.

- [ ] **Step 5: Commit.**

```bash
git add crates/lens-client/src/stream/normalize.rs
git commit -m "feat(lens-client): Normalizer::reset_seen_items for reconnect history replay"
```

---

### Task 3: Frame `sequence_number` peek

**Files:**
- Modify: `crates/lens-client/src/stream/sse.rs`
- Test: inline tests in `sse.rs`

**Interfaces:**
- Produces: `pub(crate) fn SseFrame::sequence_number(&self) -> Option<u64>` — a cheap serde peek of the `sequence_number` field off the frame's `data` JSON; `None` when absent/unparseable (persisted items, lifecycle frames, malformed data). Used by the reader for `last_seen_seq` tracking and overlap dedup (plan decision 3).

- [ ] **Step 1: Write the failing test** (use the existing `SseFrame` constructor/shape in `sse.rs` — match how `parse_event` reads `frame.data`):

```rust
#[test]
fn frame_sequence_number_peeks_data_json() {
    let f = SseFrame {
        event: "response.output_text.delta".into(),
        data: r#"{"sequence_number":7,"delta":"hi"}"#.into(),
    };
    assert_eq!(f.sequence_number(), Some(7));

    let no_seq = SseFrame { event: "x".into(), data: r#"{"id":"item_1"}"#.into() };
    assert_eq!(no_seq.sequence_number(), None);

    let null_seq = SseFrame { event: "x".into(), data: r#"{"sequence_number":null}"#.into() };
    assert_eq!(null_seq.sequence_number(), None);

    let junk = SseFrame { event: "x".into(), data: "not json".into() };
    assert_eq!(junk.sequence_number(), None);
}
```

> NOTE: match the real `SseFrame` field names/visibility. If fields are private, construct via the parser (`SseParser::push`) instead and adapt the test.

- [ ] **Step 2: Run it, verify it fails** (`sequence_number` undefined).

Run: `cargo test -p lens-client frame_sequence_number_peeks_data_json`
Expected: FAIL.

- [ ] **Step 3: Implement** on `impl SseFrame`:

```rust
impl SseFrame {
    /// Peek `sequence_number` off the frame's data JSON without full typing.
    /// `None` when absent, null, or unparseable — only seq-bearing live frames
    /// (heartbeats, response deltas) carry it (typed-client §7 / plan decision 3).
    pub(crate) fn sequence_number(&self) -> Option<u64> {
        #[derive(serde::Deserialize)]
        struct SeqPeek { sequence_number: Option<u64> }
        serde_json::from_str::<SeqPeek>(&self.data).ok()?.sequence_number
    }
}
```

- [ ] **Step 4: Run the test, verify it passes.**

Run: `cargo test -p lens-client frame_sequence_number_peeks_data_json`
Expected: PASS.

- [ ] **Step 5: Commit.**

```bash
git add crates/lens-client/src/stream/sse.rs
git commit -m "feat(lens-client): SseFrame::sequence_number peek for overlap dedup"
```

---

### Task 4: `reconnect` module — `Reopen` trait, `HttpReopener`, backoff, items replay

**Files:**
- Create: `crates/lens-client/src/reconnect.rs`
- Modify: `crates/lens-client/src/lib.rs` (add `mod reconnect;`)
- Test: inline tests in `reconnect.rs`

**Interfaces:**
- Produces:
  - `pub(crate) trait Reopen: Send { fn open_stream(&self) -> Result<Box<dyn std::io::Read + Send>>; fn snapshot(&self) -> Result<SessionSnapshot>; fn items(&self) -> Result<ItemList>; }`
  - `pub(crate) struct HttpReopener { http: reqwest::blocking::Client, conn: Connection, session_id: SessionId }` implementing `Reopen` (all fields `Clone + Send + 'static`; no `info` needed).
  - `pub(crate) const BACKOFF_MS: &[u64] = &[100, 200, 400, 800, 1600, 3000, 3000];` (≈9s worst case; ~7s through the first six per §7).
  - `pub(crate) fn items_to_replay(list: ItemList) -> Vec<ServerStreamEvent>` — each `Item` → `ServerStreamEvent::Response(ResponseEvent::OutputItemDone { item })` (bucket A; consumer merges by `id`).
- Consumes: `Item::id()` (Plan 3b-2a), `SessionSnapshot`/`ItemList` (Plan 3b-2a), `check_status` (http.rs).

- [ ] **Step 1: Write the failing test for `items_to_replay`:**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::{Item, ServerStreamEvent};
    use crate::stream::event::ResponseEvent;

    #[test]
    fn items_to_replay_maps_each_item_to_output_item_done() {
        // Build an ItemList from the golden /items capture so payloads are real.
        let raw = include_str!(
            "../../../docs/spikes/captures/2026-06-26-sse/happy_path.items.json"
        );
        let list: ItemList = serde_json::from_str(raw).expect("parse items capture");
        let n = list.items().len();
        assert!(n > 0, "fixture must have items");
        let out = items_to_replay(list);
        assert_eq!(out.len(), n);
        assert!(out.iter().all(|e| matches!(
            e,
            ServerStreamEvent::Response(ResponseEvent::OutputItemDone { .. })
        )));
    }
}
```

> NOTE: if `ItemList`'s `Deserialize` is not directly wired to the capture's envelope, deserialize via the existing `Sessions::items` decode path/helper instead; reuse whatever `de_items` uses. The point is a real, typed `ItemList`, not a hand-built stub.

- [ ] **Step 2: Run it, verify it fails** (module/function missing).

Run: `cargo test -p lens-client items_to_replay_maps`
Expected: FAIL — unresolved `reconnect` / `items_to_replay`.

- [ ] **Step 3: Create `reconnect.rs` with the trait, `HttpReopener`, backoff, and `items_to_replay`:**

```rust
//! No-replay reconnect (typed-client.md §7). The reader thread owns the protocol
//! end-to-end; the consumer only sees synthetic lifecycle ServerStreamEvents.
//! This module supplies the re-issue capability (`Reopen`) + helpers.

use std::io::Read;

use crate::client::Client;
use crate::connection::Connection;
use crate::error::Result;
use crate::ids::SessionId;
use crate::sessions::{GetOpts, ItemList, ItemsPage, SessionSnapshot};
use crate::stream::ServerStreamEvent;
use crate::stream::event::ResponseEvent;

/// §7 backoff schedule (ms): 100→200→400→800→1600→3000→3000. ~7s through six.
pub(crate) const BACKOFF_MS: &[u64] = &[100, 200, 400, 800, 1600, 3000, 3000];

/// The reader's re-issue capability. `Send` so it can live on the reader thread;
/// a trait so the reconnect state machine is unit-testable with a scripted mock.
pub(crate) trait Reopen: Send {
    /// Open a fresh `GET /stream` body.
    fn open_stream(&self) -> Result<Box<dyn Read + Send>>;
    /// `GET /v1/sessions/{id}` with items+liveness (bucket B chrome).
    fn snapshot(&self) -> Result<SessionSnapshot>;
    /// `GET /v1/sessions/{id}/items` (bucket A history).
    fn items(&self) -> Result<ItemList>;
}

/// Real impl: clones the cheap, `Send + 'static` request machinery. No `info`.
pub(crate) struct HttpReopener {
    http: reqwest::blocking::Client,
    conn: Connection,
    session_id: SessionId,
}

impl HttpReopener {
    pub(crate) fn new(client: &Client, session_id: SessionId) -> Self {
        Self {
            http: client.http().clone(),
            conn: client.conn().clone(),
            session_id,
        }
    }
}

impl Reopen for HttpReopener {
    fn open_stream(&self) -> Result<Box<dyn Read + Send>> {
        let url = self
            .conn
            .url(&format!("/v1/sessions/{}/stream", self.session_id))?;
        let resp = self.conn.auth.apply(self.http.get(url)).send()?;
        crate::http::check_status("v1/sessions/stream", resp.status().as_u16())?;
        Ok(Box::new(resp))
    }

    fn snapshot(&self) -> Result<SessionSnapshot> {
        // GetOpts has PUBLIC fields (struct literal, not builders) and a PRIVATE
        // `to_query`. Bump `GetOpts::to_query` to `pub(crate)` (one-word change in
        // sessions.rs) so this cross-module call compiles, OR build the query inline.
        let opts = GetOpts { include_items: true, include_liveness: true };
        let url = self
            .conn
            .url(&format!("/v1/sessions/{}", self.session_id))?;
        let resp = self
            .conn
            .auth
            .apply(self.http.get(url).query(&opts.to_query()))
            .send()?;
        let status = resp.status().as_u16();
        let body = resp.text()?;
        crate::http::decode_json("v1/sessions", status, &body)
    }

    fn items(&self) -> Result<ItemList> {
        let page = ItemsPage::default();
        let url = self
            .conn
            .url(&format!("/v1/sessions/{}/items", self.session_id))?;
        let resp = self
            .conn
            .auth
            .apply(self.http.get(url).query(&page.to_query()))
            .send()?;
        let status = resp.status().as_u16();
        let body = resp.text()?;
        crate::http::decode_json("v1/sessions/items", status, &body)
    }
}

/// Bucket A: replay the durable transcript as `OutputItemDone` events. The
/// consumer merges by `Item::id()` (idempotent upsert), so duplicates are safe.
pub(crate) fn items_to_replay(list: ItemList) -> Vec<ServerStreamEvent> {
    list.into_items()
        .into_iter()
        .map(|item| ServerStreamEvent::Response(ResponseEvent::OutputItemDone { item }))
        .collect()
}
```

> NOTE: this assumes `GetOpts::include_items/include_liveness/to_query`, `ItemsPage::default/to_query`, and `ItemList::into_items()` exist. If `GetOpts`/`ItemsPage` builders differ, adapt to the actual API from Plan 3b-2a (read `sessions.rs`). Add a `pub(crate) fn ItemList::into_items(self) -> Vec<Item>` if absent (mirrors the existing `items(&self)` getter) — small, in this same task.

- [ ] **Step 4: Add `mod reconnect;` to `lib.rs`** (next to `mod stream;`), and add any missing small helpers the NOTE calls out (`ItemList::into_items`, `GetOpts` builders) in their owning files.

- [ ] **Step 5: Run the test, verify it passes.**

Run: `cargo test -p lens-client items_to_replay_maps`
Expected: PASS.

- [ ] **Step 6: Commit.**

```bash
git add crates/lens-client/src/reconnect.rs crates/lens-client/src/lib.rs crates/lens-client/src/sessions.rs
git commit -m "feat(lens-client): reconnect Reopen trait + HttpReopener + items replay"
```

---

### Task 5: Reconnect state machine in the reader thread

This is the load-bearing task. The reader's `Err(_) => return` seam becomes the §7 protocol. Keep `run` generic over `Reopen` + an injected `sleep` so it is fully unit-testable.

**Files:**
- Modify: `crates/lens-client/src/stream/reader.rs`
- Test: inline tests in `reader.rs` (scripted `Reopen` mock)

**Interfaces:**
- Consumes: `Reopen`, `BACKOFF_MS`, `items_to_replay` (Task 4); `Normalizer::reset_seen_items` (Task 2); `SseFrame::sequence_number` (Task 3); the synthetic variants (Task 1).
- Produces: `EventStream::spawn(resp, reopener)` signature; reader emits, in order on reconnect: `Reconnecting{attempt}`(×N) → on success `Reconnected{gap}` → (reset normalizer if `gap != Some(0)`) → `SnapshotRestored(snapshot)` → replayed `OutputItemDone`(bucket A) → live tail (seq-deduped); on give-up/stop `Disconnected{reason}` then channel closes.

**State machine (reference — implement against this):**

```
read loop over current body:
  Ok(0)  -> clean EOF: parser.finish + normalizer.flush, then RECONNECT (server closed; try to resume)
  Ok(n)  -> for each frame: update last_seen_seq = max(last_seen_seq, frame.sequence_number());
            (if in overlap-dedup window: drop frames with seq <= resume_floor until first seq > floor, then exit window)
            feed normalizer -> send; on send Err -> return (consumer dropped)
  Err(_) -> RECONNECT (transport drop; do NOT flush synthetic ReasoningClosed)

RECONNECT(reopener, last_seen_seq):
  for (attempt, delay) in BACKOFF_MS.enumerate():
      send Reconnecting { attempt: attempt as u32 + 1 }   // (send Err -> return)
      sleep(Duration::from_millis(delay))
      match reopener.snapshot():
          Err(e) if stop_reason(&e).is_some() -> send Disconnected{reason}; return
          Err(_) -> continue            // transient; keep backing off
          Ok(snap) if snap.status() == failed ->
              send Reconnected{gap:None}; normalizer.reset_seen_items();
              send SnapshotRestored(snap); send Disconnected{SessionFailed}; return
          Ok(snap) ->
              match reopener.open_stream():
                  Err(e) if stop_reason -> send Disconnected{reason}; return
                  Err(_) -> continue
                  Ok(new_body) ->
                      // success path
                      send Reconnected { gap: None }       // decision 2 (None unless proven)
                      normalizer.reset_seen_items()         // gap != Some(0)
                      send SnapshotRestored(Box::new(snap))
                      for ev in items_to_replay(reopener.items()?) { send ev }  // items() Err -> continue backoff
                      set overlap-dedup window: resume_floor = last_seen_seq
                      return CONTINUE(new_body)             // resume outer read loop on new_body
  // backoff exhausted:
  send Disconnected { reason: RetriesExhausted }; return
```

> The `gap == Some(0)` refinement (decision 2): when the FIRST post-reopen frame with a `sequence_number` equals `resume_floor + 1`, the prior `Reconnected { gap: None }` was conservative but harmless — v1 does NOT retroactively upgrade it. Emitting `gap: None` always for v1 is acceptable; the `Some(0)` path is left as a flagged TODO in code (decision 2) and is NOT required to pass tests. Keep it simple: **emit `gap: None` unconditionally in v1.** (This removes the overlap-contiguity bookkeeping; `resume_floor` is still used to drop duplicate overlap frames.)

`stop_reason(&ClientError) -> Option<DisconnectReason>`: `check_status` (http.rs) encodes **401|403 → `ClientError::Auth { status }`** and **everything else (incl. 404) → `ClientError::Server { status, body }`**. So map:

```rust
pub(crate) fn stop_reason(e: &ClientError) -> Option<DisconnectReason> {
    match e {
        ClientError::Auth { status: 401 } => Some(DisconnectReason::Unauthorized),
        ClientError::Auth { status: 403 } => Some(DisconnectReason::Forbidden),
        ClientError::Server { status: 404, .. } => Some(DisconnectReason::NotFound),
        _ => None, // network/5xx/parse — retryable, keep backing off
    }
}
```

- [ ] **Step 1: Write the failing tests** with a scripted mock `Reopen`:

```rust
#[cfg(test)]
mod reconnect_tests {
    use super::*;
    use crate::reconnect::Reopen;
    use crate::sessions::{ItemList, SessionSnapshot};
    use crate::stream::{ServerStreamEvent, DisconnectReason};
    use std::io::{self, Cursor, Read};
    use std::sync::Mutex;

    // A mock that scripts: snapshot result, items result, and a queue of stream
    // bodies handed out on each open_stream() call.
    struct MockReopen {
        snapshot: SessionSnapshot,
        items: Mutex<Option<ItemList>>,
        bodies: Mutex<Vec<Vec<u8>>>, // popped front-first
    }
    impl Reopen for MockReopen {
        fn open_stream(&self) -> crate::error::Result<Box<dyn Read + Send>> {
            let mut b = self.bodies.lock().unwrap();
            if b.is_empty() {
                // simulate a transient open failure -> keep backing off / exhaust
                return Err(crate::error::ClientError::Network(
                    /* build a network error or a sentinel retryable error */
                    unimplemented!("use a retryable ClientError per error.rs")
                ));
            }
            Ok(Box::new(Cursor::new(b.remove(0))))
        }
        fn snapshot(&self) -> crate::error::Result<SessionSnapshot> { Ok(self.snapshot.clone()) }
        fn items(&self) -> crate::error::Result<ItemList> {
            Ok(self.items.lock().unwrap().take().expect("items once"))
        }
    }

    #[test]
    fn drop_then_reconnect_emits_lifecycle_in_order() {
        // First body: one frame then EOF (Cursor EOF == server close -> reconnect).
        // Second body (after reopen): a small live frame then EOF.
        // Assert sequence: ... Reconnecting+ , Reconnected{None}, SnapshotRestored, replayed items, live frame.
        // Use a no-op sleep so the test is instant.
        // (Construct SessionSnapshot + ItemList from the golden captures.)
    }

    #[test]
    fn unauthorized_snapshot_emits_disconnected_unauthorized_and_stops() {
        // snapshot() returns a 401 ClientError -> Disconnected{Unauthorized}, channel closes, no retry.
    }

    #[test]
    fn failed_status_snapshot_emits_snapshot_then_disconnected() {
        // snapshot.status == failed -> Reconnected, SnapshotRestored, Disconnected{SessionFailed}.
    }

    #[test]
    fn exhausted_backoff_emits_retries_exhausted() {
        // open_stream always retryable-errors -> N Reconnecting then Disconnected{RetriesExhausted}.
    }
}
```

> Fill these bodies in concretely during implementation — build `SessionSnapshot`/`ItemList` from `docs/spikes/captures/2026-06-26-sse/happy_path.{snapshot,items}.json` (deserialize, same as Task 4). Inject `sleep = |_| {}`. Each test drives `run(first_body, tx, reopener, no_op_sleep)` and collects `rx.iter()`.

- [ ] **Step 2: Run them, verify they fail** (signature/behavior not present).

Run: `cargo test -p lens-client reconnect_tests`
Expected: FAIL (compile: `run` arity; or behavioral).

- [ ] **Step 3: Rewrite `run` + `EventStream::spawn`.** Make `run` generic and drive the state machine:

```rust
pub(crate) fn spawn<Re: Reopen + 'static>(
    resp: reqwest::blocking::Response,
    reopener: Re,
) -> Self {
    let (tx, rx) = mpsc::channel();
    let handle = std::thread::Builder::new()
        .name("lens-sse-reader".into())
        .spawn(move || run(Box::new(resp) as Box<dyn Read + Send>, tx, reopener, |d| std::thread::sleep(d)))
        .expect("spawn SSE reader thread");
    EventStream { rx, _handle: handle }
}
```

`run` signature:

```rust
fn run<Re: Reopen>(
    mut body: Box<dyn std::io::Read + Send>,
    tx: mpsc::Sender<ServerStreamEvent>,
    reopener: Re,
    sleep: impl Fn(std::time::Duration),
) { /* read loop + reconnect() per the state-machine reference above */ }
```

Implement the read loop, `last_seen_seq` tracking via `frame.sequence_number()`, the overlap-dedup window (drop frames with `seq <= resume_floor` until the first `seq > resume_floor`, then disable the window), and the `reconnect()` inner routine. On `Ok(0)` (clean EOF) flush the normalizer's reasoning bracket BEFORE reconnecting; on `Err(_)` do NOT flush (the existing §7a invariant). On any `tx.send(..).is_err()` return immediately (consumer dropped).

- [ ] **Step 4: Update the two pre-existing reader tests** (`transport_error_does_not_synthesize_reasoning_closed`, `clean_eof_flushes_dangling_reasoning_closed`). They call `run(reader, tx)` — now 4-arg. Pass a mock `Reopen` whose `open_stream` immediately returns a retryable error so the machine exhausts `BACKOFF_MS` and emits `Disconnected{RetriesExhausted}`, plus a no-op sleep. Adjust assertions:
  - `transport_error_…`: still assert NO `ReasoningClosed` is emitted (drop must not flush), AND now assert a terminal `Disconnected` appears.
  - `clean_eof_…`: clean EOF now flushes `ReasoningClosed` (assert exactly one) *then* attempts reconnect (exhausts → `Disconnected`). Assert the `ReasoningClosed` precedes `Disconnected`.

- [ ] **Step 5: Run the full reader test module, verify it passes.**

Run: `cargo test -p lens-client --lib stream::reader`
Expected: PASS (new reconnect tests + the two updated ones).

- [ ] **Step 6: Commit.**

```bash
git add crates/lens-client/src/stream/reader.rs
git commit -m "feat(lens-client): §7 reconnect state machine in the SSE reader thread"
```

---

### Task 6: Wire `Sessions::stream` to the reopener

**Files:**
- Modify: `crates/lens-client/src/sessions.rs`
- Test: a `--features live-tests` test (gated; not required to land) + a serverless compile/shape test.

**Interfaces:**
- Consumes: `HttpReopener::new(client, session_id)` (Task 4); `EventStream::spawn(resp, reopener)` (Task 5).

- [ ] **Step 1: Update `Sessions::stream`** to build the reopener from the live `Client` + the session id, and pass it to `spawn`:

```rust
pub fn stream(
    &self,
    id: &crate::ids::SessionId,
) -> crate::error::Result<crate::stream::EventStream> {
    let url = self.client.conn().url(&format!("/v1/sessions/{id}/stream"))?;
    let resp = self
        .client
        .conn()
        .auth
        .apply(self.client.http().get(url))
        .send()?;
    let status = resp.status().as_u16();
    crate::http::check_status("v1/sessions/stream", status)?;
    let reopener = crate::reconnect::HttpReopener::new(self.client, id.clone());
    Ok(crate::stream::EventStream::spawn(resp, reopener))
}
```

> NOTE: `SessionId` is already `Clone` (branded-id macro derives `Clone`), so `id.clone()` is fine and no `ids.rs` change is needed. `Client::http()`/`conn()` are already `pub(crate)`.

- [ ] **Step 2: Run the full suite + clippy + fmt.**

Run:
```bash
cargo test -p lens-client
cargo clippy -p lens-client --all-targets -- -D warnings
cargo fmt --check
```
Expected: all green; no `generated.rs` change in `git diff`.

- [ ] **Step 3: (Optional, live) add a gated reconnect smoke test** under `#[cfg(feature = "live-tests")]`: open a stream against a warm session, kill the body mid-stream (drop the underlying connection or stop/start the daemon per the transport-stability spike harness), and assert the channel yields `Reconnecting`→`Reconnected`→`SnapshotRestored` then resumes. Mark `#[ignore]` if no scripted server-kill is available this session; capture as a follow-up. **Not required to land.**

- [ ] **Step 4: Commit.**

```bash
git add crates/lens-client/src/sessions.rs
git commit -m "feat(lens-client): Sessions::stream builds HttpReopener for reconnect"
```

---

### Task 7: Docs + status + handoff

**Files:**
- Modify: `docs/design/typed-client.md` §7 (reconcile decisions 1–4 from this plan into the protocol prose; the A2 chrome decision is already recorded).
- Modify: `docs/STATUS.md` + `docs/STATUS-ARCHIVE.md` (per `[[end-of-session-status-update]]`).
- Create: `docs/handoffs/2026-06-26-lens-client-plan3b2b-execution.md` (if multi-session).

- [ ] **Step 1: Fold the four plan decisions into typed-client §7** — `Disconnected { reason }` payload + the `DisconnectReason` mapping table; `gap: None` v1 semantics (note the `Some(0)` deferral); frame-level seq peek; single-page items replay. Keep §7 authoritative.
- [ ] **Step 2: Update STATUS** — Plan 3b-2b executed; list commits; note live-test status (gated/deferred if not run); next = Plan 3c contract-drift CI.
- [ ] **Step 3: Write the handoff** if work spans sessions — what shipped, what's deferred (live reconnect smoke, `gap==Some(0)`, items pagination), where 3c picks up.
- [ ] **Step 4: Commit.**

```bash
git add docs/
git commit -m "docs(status): Plan 3b-2b reconnect state machine executed; §7 reconciled"
```

---

## Self-review

**Spec coverage (vs typed-client §7 steps 1–7 + three-bucket):**
- Step 1 detect disconnect → Task 5 `Err(_)`/`Ok(0)` seams. ✅
- Step 2 backoff + `Reconnecting{attempt}` → Task 4 `BACKOFF_MS` + Task 5 emit. ✅
- Step 3 give-up + terminal `Disconnected` → Task 5 `RetriesExhausted` + decision 1. ✅
- Step 4 snapshot/bucket-B chrome → Task 4 `Reopener::snapshot` + Task 1 `SnapshotRestored` (A2). ✅
- Step 5 bucket-A history via `/items`, merge by id → Task 4 `items_to_replay`. ✅
- Step 6 `Reconnected` precedes history (now `Reconnected`→`SnapshotRestored`→history) → Task 5 ordering. ✅
- Step 7 re-open + seq-dedup overlap → Task 3 peek + Task 5 dedup window. ✅
- Seam (a) `seen_items` reset on `gap != Some(0)` → Task 2 + Task 5 (v1 always resets, since gap is always `None`). ✅
- Seam (b) synthetic markers bypass normalization → Task 5 sends them directly, never through `normalizer.push`. ✅
- Stop-immediately 401/403/404/failed → Task 5 `stop_reason` + `SessionFailed`. ✅

**Placeholder scan:** the `MockReopen` error construction and the four reconnect test bodies are marked to fill concretely against `error.rs` + the golden captures during implementation (they need the real `ClientError` retryable/stop encoding, which the implementer reads in-task). The state-machine reference is pseudocode by design (Task 5 prose), with the exact emit order and signatures pinned. No "add error handling"/"TBD" left in shipped code paths.

**Type consistency:** `SnapshotRestored(Box<SessionSnapshot>)`, `Reconnected { gap: Option<u64> }`, `DisconnectReason` variants, `Reopen::{open_stream→Box<dyn Read+Send>, snapshot→SessionSnapshot, items→ItemList}`, `items_to_replay(ItemList)->Vec<ServerStreamEvent>`, `EventStream::spawn(resp, reopener)`, `Normalizer::reset_seen_items`, `SseFrame::sequence_number()->Option<u64>` — used consistently across Tasks 1–6.

**Known v1 deferrals (flagged, not gaps):** `gap == Some(0)` proof (decision 2 — always `None` in v1); items pagination/backfill (decision 4); live reconnect smoke test (Task 6 step 3, gated). Each is a conscious scope cut with a safe fallback, recorded for §7 + the handoff.
