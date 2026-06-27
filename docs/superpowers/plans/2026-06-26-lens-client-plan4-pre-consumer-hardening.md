# lens-client Plan 4 — Pre-Consumer Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the cheap-now, expensive-later gaps the consolidated lens-client review found, so the state-model can be built on a settled stream/error surface.

**Architecture:** Six surgical changes to the existing `lens-client` crate — one real correctness bug (phantom `ReasoningClosed` after reconnect), HTTP robustness (timeouts + an `unwrap_err` removal), backpressure (bounded channel), a stream shutdown handle, and bootstrap/reconnect symmetry so the future reducer is the single writer. No new endpoints. `generated.rs` is never touched.

**Tech Stack:** Rust 2024, `reqwest` blocking, `std::sync::mpsc`, `std::thread`, `serde`/`serde_json`, `thiserror`.

## Global Constraints

- **MANDATORY** The UI never panics the process — no `unwrap`/`expect`/`panic!`/`unreachable!` on any reachable path (incl. reader thread, reconnect, parse, bootstrap).
- **MANDATORY** Typed end-to-end — no stringly-typed dispatch; no new `serde_json::Value` reaching consumers.
- **MANDATORY** Never block the gpui foreground thread — all I/O stays on the reader thread; blocking-send backpressure is the intended mechanism.
- **MANDATORY** `clippy --all-targets -D warnings` clean + `rustfmt`; `generated.rs` byte-untouched.
- **MANDATORY** Benchmark-or-it's-not-done on perf paths; logic cores ship tests (every task here is TDD).
- **MANDATORY** Ground-truth discipline — protocol changes (Task 5) reconcile the contract docs (`docs/design/typed-client.md` §7, `app-architecture-and-state-model.md` §4.1) in the same task.
- Branch: `feat/lens-client-hardening` off `main` (`8a5a8b3`). Verify command for every task: `cargo test -p lens-client && cargo clippy -p lens-client --all-targets -- -D warnings && cargo fmt --check`.
- Out of scope (tracked elsewhere, do NOT touch here): event-surface recapture (#5 — separate capture spike); `ChildSessionUpdated`/poke-only chrome payloads (SCHEMA-DERIVED, re-capture at config-time); `info.databricks_features` Value; `ClientError::NotFound` rename + `Validation`/422 variant; WS terminal client (Plan 7).

---

### Task 1: Fix phantom `ReasoningClosed` after mid-reasoning reconnect

The reader resets only `seen_items` on reconnect (`reader.rs:205`) and deliberately does **not** flush on a transport `Err(_)` (`reader.rs:128`). So a drop *mid-reasoning* leaves `Normalizer::reasoning` populated; the first post-reconnect `OutputTextDelta`/`Completed` then emits a synthetic `ReasoningClosed` built from pre-drop deltas. Fix: reconnect must clear **all** transient normalizer state, not just the dedup set.

**Files:**
- Modify: `crates/lens-client/src/stream/normalize.rs:105-110` (rename + widen the reset method)
- Modify: `crates/lens-client/src/stream/reader.rs:205` (call site) and `crates/lens-client/src/stream/normalize.rs:182` (existing test name)
- Test: `crates/lens-client/src/stream/normalize.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Produces: `Normalizer::reset_transient(&mut self)` — clears `seen_items` **and** `reasoning`. Replaces `reset_seen_items`.

- [ ] **Step 1: Write the failing regression test** (add to `normalize.rs` tests)

```rust
#[test]
fn reset_transient_drops_open_reasoning_so_no_phantom_close_after_reconnect() {
    use super::super::event::{ResponseEvent, ServerStreamEvent};
    let mut n = Normalizer::default();
    // Open a reasoning bracket and accumulate a delta (mimics pre-drop state).
    let _ = n.push(ServerStreamEvent::Response(ResponseEvent::ReasoningStarted));
    let _ = n.push(ServerStreamEvent::Response(ResponseEvent::ReasoningTextDelta {
        delta: "pre-drop".into(),
    }));
    // Reconnect clears transient state.
    n.reset_transient();
    // First live event after reconnect must NOT carry a synthetic ReasoningClosed.
    let out = n.push(ServerStreamEvent::Response(ResponseEvent::OutputTextDelta {
        delta: "fresh".into(),
    }));
    assert_eq!(out.len(), 1, "expected only the delta, got a phantom close: {out:?}");
    assert!(matches!(
        out[0],
        ServerStreamEvent::Response(ResponseEvent::OutputTextDelta { .. })
    ));
}
```

- [ ] **Step 2: Run it — expect FAIL**

Run: `cargo test -p lens-client reset_transient_drops_open_reasoning -- --nocapture`
Expected: compile error (`reset_transient` not found) — then after Step 3, the assertion would have failed under the old `reset_seen_items` (it left `reasoning` set, emitting a 2-element vec).

- [ ] **Step 3: Rename + widen the method** (`normalize.rs:105-110`)

```rust
    /// Clear ALL transient mid-stream state — the `OutputItemDone` dedup set and
    /// any open reasoning bracket. Called by the reader on reconnect so neither
    /// `GET /items` replay is wrongly suppressed nor a stale synthetic
    /// `ReasoningClosed` leaks into the new live tail (typed-client §7 seam (a)).
    pub(crate) fn reset_transient(&mut self) {
        self.seen_items.clear();
        self.reasoning = None;
    }
```

- [ ] **Step 4: Update the call site** (`reader.rs:205`)

Replace `normalizer.reset_seen_items();` with `normalizer.reset_transient();`

- [ ] **Step 5: Update the existing test name** (`normalize.rs:182`)

Rename `reset_seen_items_allows_a_previously_seen_item_through` → `reset_transient_allows_a_previously_seen_item_through` and change its `n.reset_seen_items();` call to `n.reset_transient();` (the assertion is unchanged).

- [ ] **Step 6: Run the suite — expect PASS**

Run: `cargo test -p lens-client && cargo clippy -p lens-client --all-targets -- -D warnings && cargo fmt --check`
Expected: PASS, clippy/fmt clean.

- [ ] **Step 7: Commit**

```bash
git add crates/lens-client/src/stream/normalize.rs crates/lens-client/src/stream/reader.rs
git commit -m "fix(lens-client): reset open reasoning bracket on reconnect (no phantom ReasoningClosed)"
```

---

### Task 2: HTTP robustness — timeouts + remove `get_bytes` `unwrap_err`

Two issues in `client.rs`: (a) the blocking client has no `connect_timeout`, so a flaky remote hangs `Client::new` and the reader thread indefinitely; (b) `get_bytes` uses `check_status(...).unwrap_err()` (`client.rs:107`) — safe today but an `unwrap` on a reachable line.

**Critical SSE constraint:** do **not** set a total `.timeout()` on the shared client — it applies to body reads too and would kill a healthy but quiet SSE stream. Use `connect_timeout` (connect phase only, safe for SSE) on the client, plus a per-request total `.timeout()` on the short REST helpers **only** — never on the streaming GET.

**Files:**
- Modify: `crates/lens-client/src/client.rs` (builder + `get_json`/`send_json`/`get_bytes`/`send_multipart`)
- Modify: `crates/lens-client/src/reconnect.rs` (`HttpReopener::snapshot`/`items` get the REST timeout; `open_stream` does NOT)
- Test: `crates/lens-client/src/client.rs` (`#[cfg(test)] mod tests`) — assert constants only (timeout behavior needs a live/hung server, out of scope)

**Interfaces:**
- Produces: `pub(crate) const CONNECT_TIMEOUT: Duration` and `pub(crate) const REST_TIMEOUT: Duration` in `client.rs`.

- [ ] **Step 1: Write the failing test** (add to `client.rs` tests; create the `mod tests` block if absent)

```rust
#[cfg(test)]
mod tests {
    use super::{CONNECT_TIMEOUT, REST_TIMEOUT};
    use std::time::Duration;

    #[test]
    fn timeouts_are_bounded_and_connect_is_shorter() {
        assert_eq!(CONNECT_TIMEOUT, Duration::from_secs(10));
        assert_eq!(REST_TIMEOUT, Duration::from_secs(30));
        assert!(CONNECT_TIMEOUT < REST_TIMEOUT);
    }
}
```

- [ ] **Step 2: Run it — expect FAIL**

Run: `cargo test -p lens-client timeouts_are_bounded`
Expected: FAIL (constants not defined).

- [ ] **Step 3: Add constants + apply to the builder** (`client.rs`)

Add `use std::time::Duration;` near the top, then above `impl Client`:

```rust
/// Connect-phase timeout for ALL requests. Safe for SSE: it bounds only the
/// TCP/TLS handshake, never body reads — so a healthy quiet stream is untouched.
pub(crate) const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Total per-request timeout for SHORT REST calls only. Never applied to the
/// streaming GET (it would kill a healthy idle SSE body).
pub(crate) const REST_TIMEOUT: Duration = Duration::from_secs(30);
```

In `Client::new`, change the builder:

```rust
        let http = reqwest::blocking::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .map_err(ClientError::Network)?;
```

- [ ] **Step 4: Apply `REST_TIMEOUT` to the REST helpers** (`client.rs`)

In `get_json`, add `.timeout(REST_TIMEOUT)` to the request builder:

```rust
        let resp = self
            .conn()
            .auth
            .apply(self.http().get(url).query(query).timeout(REST_TIMEOUT))
            .send()?;
```

In `send_json`, after `let mut rb = self.http().request(method, url).query(query);`, add `rb = rb.timeout(REST_TIMEOUT);` before the optional `.json(b)`.

In `send_multipart`, add `.timeout(REST_TIMEOUT)` to its request builder (same pattern).

- [ ] **Step 5: Fix `get_bytes` — remove `unwrap_err`** (`client.rs:102-110`)

```rust
    pub(crate) fn get_bytes(&self, path: &str) -> crate::error::Result<Vec<u8>> {
        let url = self.conn().url(path)?;
        let resp = self
            .conn()
            .auth
            .apply(self.http().get(url).timeout(REST_TIMEOUT))
            .send()?;
        crate::http::check_status(path, resp.status().as_u16())?;
        Ok(resp.bytes()?.to_vec())
    }
```

- [ ] **Step 6: Apply `REST_TIMEOUT` to the reopener's REST calls — NOT `open_stream`** (`reconnect.rs`)

In `HttpReopener::snapshot` and `HttpReopener::items`, add `.timeout(crate::client::REST_TIMEOUT)` to each `.get(url)...` builder. **Leave `open_stream` with no `.timeout()`** (it's the SSE body). Add a one-line comment on `open_stream`:

```rust
    fn open_stream(&self) -> Result<Box<dyn Read + Send>> {
        // No total timeout: this is the long-lived SSE body. connect_timeout
        // (client-level) bounds the handshake; idle is handled by the stop flag.
        let url = self
```

- [ ] **Step 7: Run the suite — expect PASS**

Run: `cargo test -p lens-client && cargo clippy -p lens-client --all-targets -- -D warnings && cargo fmt --check`

- [ ] **Step 8: Commit**

```bash
git add crates/lens-client/src/client.rs crates/lens-client/src/reconnect.rs
git commit -m "fix(lens-client): connect+REST timeouts (not on SSE body) + drop get_bytes unwrap_err"
```

---

### Task 3: Bounded event channel (blocking-send backpressure)

`reader.rs:30` uses `mpsc::channel()` (unbounded), contradicting impl-spec §6's "blocking-send backpressure": a slow UI poller against a fast stream grows memory without bound. Switch to `sync_channel` so the reader blocks (off-foreground) when the consumer falls behind, which propagates TCP backpressure.

**Files:**
- Modify: `crates/lens-client/src/stream/reader.rs` (channel type + `run`/`reconnect` signatures + tests)

**Interfaces:**
- Produces: `pub(crate) const EVENT_CHANNEL_BOUND: usize` in `reader.rs`. The `EventStream::rx` type is unchanged (`mpsc::Receiver`); senders become `mpsc::SyncSender`.

- [ ] **Step 1: Write the failing test** (add to `reader.rs` tests)

```rust
#[test]
fn channel_is_bounded() {
    assert_eq!(EVENT_CHANNEL_BOUND, 1024);
}
```

- [ ] **Step 2: Run it — expect FAIL**

Run: `cargo test -p lens-client channel_is_bounded`
Expected: FAIL (const not defined).

- [ ] **Step 3: Add the const + switch the channel** (`reader.rs`)

Near the top of `reader.rs`:

```rust
/// Bound on the reader→poller channel. A full channel blocks the reader thread
/// (off the foreground), propagating backpressure to TCP (impl-spec §6). Sized
/// for a generous burst without unbounded growth under a stalled UI poller.
pub(crate) const EVENT_CHANNEL_BOUND: usize = 1024;
```

In `EventStream::spawn`, change `let (tx, rx) = mpsc::channel();` to:

```rust
        let (tx, rx) = mpsc::sync_channel(EVENT_CHANNEL_BOUND);
```

- [ ] **Step 4: Update sender types** (`reader.rs`)

Change `run`'s signature parameter `tx: mpsc::Sender<ServerStreamEvent>` → `tx: mpsc::SyncSender<ServerStreamEvent>`, and `reconnect`'s `tx: &mpsc::Sender<ServerStreamEvent>` → `tx: &mpsc::SyncSender<ServerStreamEvent>`. The `tx.send(ev).is_err()` call sites are unchanged (`SyncSender::send` blocks when full, returns `Err` when the receiver is dropped — exactly the desired semantics).

- [ ] **Step 5: Fix every test that constructs a channel** (`reader.rs` tests)

There are 7 `let (tx, rx) = mpsc::channel();` sites in the test module (≈ lines 304, 336, 521, 553, 591, 639, 681). Change **each** to `let (tx, rx) = mpsc::sync_channel(EVENT_CHANNEL_BOUND);`. Run `grep -n 'mpsc::channel' crates/lens-client/src/stream/reader.rs` and confirm zero remain.

- [ ] **Step 6: Run the suite — expect PASS**

Run: `cargo test -p lens-client && cargo clippy -p lens-client --all-targets -- -D warnings && cargo fmt --check`

- [ ] **Step 7: Commit**

```bash
git add crates/lens-client/src/stream/reader.rs
git commit -m "feat(lens-client): bounded sync_channel for reader→poller backpressure"
```

---

### Task 4: Stream shutdown handle (`EventStream::stop`)

`EventStream` has no cancellation: dropping it drops `rx`, but a reader parked in `body.read()` only notices when the next byte arrives. The 10-min auto-sleep teardown would otherwise leak a parked reader thread per session. Add a cooperative stop flag checked between reads and during reconnect backoff.

**Limitation (documented, deferred):** on a *completely silent* socket (no bytes, no heartbeats) the parked `body.read()` cannot be interrupted by a flag alone; the thread exits on the next read/heartbeat. omnigent emits `session.heartbeat`, so this is bounded in practice. A read-idle backstop is intentionally out of scope (it would conflict with the reconnect-on-`Err` path). Flag this in the doc comment.

**Files:**
- Modify: `crates/lens-client/src/stream/reader.rs` (`EventStream` struct, `spawn`, `run`, `reconnect` + a public `stop`)
- Test: `crates/lens-client/src/stream/reader.rs` tests

**Interfaces:**
- Consumes: `std::sync::Arc`, `std::sync::atomic::{AtomicBool, Ordering}`.
- Produces: `EventStream::stop(&self)`; `run`/`reconnect` gain a `stop: &Arc<AtomicBool>` parameter (checked before each read iteration and each backoff tick).

- [ ] **Step 1: Write the failing test** (add to `reader.rs` tests — drive `run` with a stop flag already set; assert it returns promptly without emitting). Reuse the existing `ExhaustReopener` mock (it needs a `snapshot` field — build it from the golden snapshot the way the existing reconnect tests do).

```rust
#[test]
fn run_returns_immediately_when_stop_is_set_at_entry() {
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    let (tx, rx) = mpsc::sync_channel(EVENT_CHANNEL_BOUND);
    // A body that would block/emit if read; the entry stop check must short-circuit.
    let body: Box<dyn Read + Send> = Box::new(StepRead {
        steps: vec![Ok(b"event: session.heartbeat\ndata: {}\n\n")],
        next: 0,
    });
    let stop = Arc::new(AtomicBool::new(true)); // already stopped
    let reopener = ExhaustReopener { snapshot: golden_snapshot() }; // reuse the file's golden-snapshot helper
    run(body, tx, reopener, |_d| {}, &stop);
    assert!(rx.try_recv().is_err(), "stopped run must emit nothing");
}
```

(Match the file's existing helpers: `StepRead` for the body, `ExhaustReopener`/`MockReopen` for `Reopen`, and whatever constructor the reconnect tests use to load the golden `SessionSnapshot`.)

- [ ] **Step 2: Run it — expect FAIL**

Run: `cargo test -p lens-client run_returns_immediately_when_stop`
Expected: FAIL (compile — `run` arity, `EventStream` has no `stop`).

- [ ] **Step 3: Add the flag to `EventStream` + `spawn`** (`reader.rs`)

```rust
pub struct EventStream {
    rx: mpsc::Receiver<ServerStreamEvent>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    _handle: JoinHandle<()>,
}
```

In `spawn`, before building the thread:

```rust
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let thread_stop = std::sync::Arc::clone(&stop);
```

Pass `&thread_stop` into `run` inside the closure, and store `stop` in the returned `EventStream { rx, stop, _handle: handle }`.

- [ ] **Step 4: Add `stop()` + the checks** (`reader.rs`)

```rust
    /// Cooperatively stop the reader. Takes effect on the next read/heartbeat or
    /// backoff tick (omnigent sends `session.heartbeat`, so this is bounded in
    /// practice). A fully silent socket is interrupted only when the next byte
    /// arrives — a read-idle backstop is deferred (it would race the reconnect path).
    pub fn stop(&self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
    }
```

In `run`, add `stop: &std::sync::Arc<std::sync::atomic::AtomicBool>` as the last parameter. Place this check **at `run` entry — before the parser/normalizer init** (so it precedes Task 5's bootstrap prelude), AND repeat it at the very top of the `loop {`:

```rust
        if stop.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
```

In `reconnect`, add the same `stop` parameter and check it at the top of the `for (i, &delay)` backoff loop (return `None` if set). Thread `stop` through the two `reconnect(&reopener, &sleep, &tx, &mut normalizer)` call sites in `run`.

- [ ] **Step 5: Update existing `run`/`reconnect` test call sites** to pass a fresh `&Arc::new(AtomicBool::new(false))`.

- [ ] **Step 6: Run the suite — expect PASS**

Run: `cargo test -p lens-client && cargo clippy -p lens-client --all-targets -- -D warnings && cargo fmt --check`

- [ ] **Step 7: Commit**

```bash
git add crates/lens-client/src/stream/reader.rs
git commit -m "feat(lens-client): cooperative EventStream::stop handle for reader teardown"
```

---

### Task 5: Bootstrap/reconnect symmetry — emit `SnapshotRestored` + items on first open

**Design decision (decide + record before coding — this touches the LOCKED state-model boundary).** Today `stream()` opens the body and tails; the consumer loads initial snapshot+items through a *second* path. That makes the future reducer NOT the single writer (app-arch §4.1) — bootstrap state arrives off-band and must stay byte-aligned with the reconnect fold forever. Fix: the reader emits the same post-open prelude on first connect as on reconnect — `SnapshotRestored(snapshot)` then replayed `OutputItemDone` items — **without** the `Reconnecting`/`Reconnected` markers (there is no gap on first connect).

**Failure policy (decided):** on first open, fetch snapshot then items; if either fails *retryably* (no `stop_reason`), skip the prelude and proceed to live-tail (no worse than today — the consumer simply gets no synthetic prelude); if either fails *fatally* (`stop_reason` matches), emit `Disconnected { reason }` and return. This keeps bootstrap panic-free and never blocks the foreground.

**Files:**
- Modify: `crates/lens-client/src/stream/reader.rs` (`run` — bootstrap prelude before the read loop)
- Modify: `docs/design/typed-client.md` §7 (bootstrap now mirrors reconnect's post-open sequence) and `docs/design/app-architecture-and-state-model.md` §4.1 (reducer is the single writer for bootstrap too)
- Test: `crates/lens-client/src/stream/reader.rs` tests

**Decomposition (the load-bearing refactor — why `run` must split):** The existing reconnect tests call `run` directly with `MockReopen`, whose `items` is a `take()`-once `Mutex<Option<ItemList>>`. Prepending a bootstrap prelude *inside* `run` would (a) reorder every reconnect test's asserted event sequence and (b) consume the single `items` payload the reconnect path needs, starving it. So split: extract the steady-state body into `read_loop(...)` and make `run = entry-stop-check + bootstrap(...) + read_loop(...)`. Reconnect tests retarget `read_loop` directly (they test the loop/reconnect, not bootstrap); bootstrap gets its own test. This is a cleaner seam anyway — the one-time prelude vs. the steady-state loop.

**Interfaces:**
- Consumes: `Reopen::snapshot`/`items` (already on the reader's `reopener`), `items_to_replay`, `ServerStreamEvent::SnapshotRestored`, `stop_reason`.
- Produces: `fn read_loop<Re: Reopen>(body, tx: SyncSender, reopener: Re, sleep, stop) ` (the former `run` body); `fn run<Re: Reopen>(body, tx, reopener, sleep, stop)` = entry stop-check → `bootstrap` → `read_loop`. `EventStream::spawn` keeps calling `run`. On first connect: event prefix `SnapshotRestored(snap)`, then N×`OutputItemDone`, then the live tail — no `Reconnecting`/`Reconnected`.

- [ ] **Step 1: Record the decision in the docs FIRST** (so ground-truth precedes code)

Add to `typed-client.md` §7 a short "Bootstrap" note: first open emits `SnapshotRestored` + replayed items (no `Reconnecting`/`Reconnected`), identical fold to reconnect; retryable prelude-fetch failure degrades to live-tail-only, fatal failure emits `Disconnected`. Add to `app-architecture-and-state-model.md` §4.1 that the reducer folds bootstrap `SnapshotRestored` exactly as reconnect (scalar restore, no transcript side-effects). Commit these doc edits as the first commit of the task.

- [ ] **Step 2: Extract `read_loop` (pure rename, no behavior change), retarget reconnect tests**

Rename the current `fn run` to `fn read_loop` (keep the exact body + the Task 4 loop-top stop check). Add a new thin `run` that just calls `read_loop` for now:

```rust
fn run<Re: Reopen>(
    body: Box<dyn Read + Send>,
    tx: mpsc::SyncSender<ServerStreamEvent>,
    reopener: Re,
    sleep: impl Fn(Duration),
    stop: &std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    if stop.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    read_loop(body, tx, reopener, sleep, stop);
}
```

Update the **reconnect-focused** tests (the `MockReopen`/`ExhaustReopener` ones at ≈ lines 521/553/591/639/681 that assert reconnect event order) to call `read_loop(...)` instead of `run(...)` — they test the loop, not bootstrap. Run `cargo test -p lens-client` — expect PASS (pure refactor).

- [ ] **Step 3: Write the failing bootstrap test** (assert the first two emitted events are `SnapshotRestored` then `OutputItemDone`, before any live event)

```rust
#[test]
fn first_open_emits_snapshot_then_items_before_live_tail() {
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    // MockReopen with snapshot=golden, items=Some(golden one-item list), no bodies.
    let reopener = MockReopen {
        snapshot: golden_snapshot(),
        snapshot_auth_401: false,
        items: std::sync::Mutex::new(Some(golden_item_list())),
        items_retry_503_first: false,
        items_call_count: std::sync::Mutex::new(0),
        bodies: std::sync::Mutex::new(vec![]),
        open_stream_always_503: true,
    };
    let (tx, rx) = mpsc::sync_channel(EVENT_CHANNEL_BOUND);
    let body: Box<dyn Read + Send> = Box::new(StepRead { steps: vec![Ok(b"")], next: 0 }); // immediate EOF
    let stop = Arc::new(AtomicBool::new(false));
    run(body, tx, reopener, |_d| {}, &stop);
    let evs: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
    assert!(matches!(evs[0], ServerStreamEvent::SnapshotRestored(_)), "first: {:?}", evs[0]);
    assert!(matches!(
        evs[1],
        ServerStreamEvent::Response(ResponseEvent::OutputItemDone { .. })
    ), "second: {:?}", evs[1]);
}
```

Use the file's existing golden-snapshot / golden-item-list helpers (the reconnect tests already load `happy_path.snapshot.json` / `happy_path.items.json`); match their constructor exactly. Run it — expect FAIL (no bootstrap yet, so `evs[0]` is `Disconnected`, not `SnapshotRestored`).

- [ ] **Step 4: Add `bootstrap` and wire it into `run`** (`reader.rs`)

```rust
/// First-open prelude: emit the same post-open sequence as reconnect, minus the
/// Reconnecting/Reconnected markers (no gap on first connect), so the consumer's
/// reducer is the single writer (app-arch §4.1). Returns `false` to abort `run`
/// (consumer gone, or a fatal fetch error for which Disconnected was sent).
/// Retryable fetch failure degrades to live-tail-only (no regression vs pre-Plan-4).
fn bootstrap<Re: Reopen>(
    reopener: &Re,
    tx: &mpsc::SyncSender<ServerStreamEvent>,
) -> bool {
    match reopener.snapshot().and_then(|snap| reopener.items().map(|items| (snap, items))) {
        Ok((snap, items)) => {
            if tx.send(ServerStreamEvent::SnapshotRestored(Box::new(snap))).is_err() {
                return false;
            }
            for ev in items_to_replay(items) {
                if tx.send(ev).is_err() {
                    return false;
                }
            }
            true
        }
        Err(e) => match stop_reason(&e) {
            Some(r) => {
                let _ = tx.send(ServerStreamEvent::Disconnected { reason: r });
                false
            }
            None => true, // retryable: skip prelude, proceed to live tail
        },
    }
}
```

Wire into `run` between the entry stop-check and `read_loop`:

```rust
    if !bootstrap(&reopener, &tx) {
        return;
    }
    read_loop(body, tx, reopener, sleep, stop);
```

(`SnapshotRestored(Box<SessionSnapshot>)` and `items_to_replay` match `reader.rs:207,212`.)

- [ ] **Step 5: Run the suite — expect PASS**

Run: `cargo test -p lens-client && cargo clippy -p lens-client --all-targets -- -D warnings && cargo fmt --check`
Expected: PASS. The reconnect tests are unaffected (they target `read_loop`); only `run`-level tests see the prelude.

- [ ] **Step 6: Commit**

```bash
git add crates/lens-client/src/stream/reader.rs docs/design/typed-client.md docs/design/app-architecture-and-state-model.md
git commit -m "feat(lens-client): emit SnapshotRestored+items on first open (reducer single-writer parity)"
```

---

### Task 6: Verify, review, document

**Files:**
- Modify: `docs/STATUS.md`, `docs/STATUS-ARCHIVE.md`, `.superpowers/sdd/progress.md`
- Modify: memory dir + `MEMORY.md` (a `plan4-hardening` learning if anything non-obvious surfaced)

- [ ] **Step 1: Full gate**

Run: `cargo test -p lens-client && cargo clippy -p lens-client --all-targets -- -D warnings && cargo fmt --check && cargo run -p xtask -- drift && git diff --stat -- crates/lens-client/src/generated.rs`
Expected: all green; `generated.rs` shows **no** diff.

- [ ] **Step 2: Consolidated cross-family review** — route the whole `feat/lens-client-hardening` diff through `cursor-delegate` `tier: diversity` (non-Claude, since composer/Claude authored it). Apply real findings; re-verify. Per `[[review-spend-policy]]`, one consolidated pass.

- [ ] **Step 3: Opus spot-check on Task 5** (the protocol/doc change) — confirm the bootstrap prelude + doc edits hold against the LOCKED §4.1/§7 boundary.

- [ ] **Step 4: Update STATUS + progress + memory**, then open the finish-branch step (`superpowers:finishing-a-development-branch`).

- [ ] **Step 5: Commit docs**

```bash
git add docs/STATUS.md docs/STATUS-ARCHIVE.md .superpowers/sdd/progress.md
git commit -m "docs(status): lens-client Plan 4 pre-consumer hardening complete"
```

---

## Deferred (NOT in this plan — tracked)

- **#5 event-surface recapture** — `session.agent_changed`, `response.created`/`queued`, `turn.*` are `DEFERRED→Unknown` and absent from the golden corpus; the only working harness (claude-sdk) emits none. Separate **capture spike** gated on a live server + a harness that drives them; model from real bytes (per decision), with a schema-model fallback if undrivable.
- `ChildSessionUpdated`/`Terminal*`/poke-only chrome payload loss — fold into #5's recapture (SCHEMA-DERIVED).
- `info.databricks_features: Value` (read-side leak) — type or make opaque.
- `ClientError::NotFound` false-friend rename + typed `Validation`/422 variant.
- Two status (`SessionStatusValue` vs `SessionStatus`) / two usage representations — document reducer normalization.
- WS terminal attach client (Plan 7).

## Self-Review

- **Spec coverage:** Tier 1 #1→Task 1, #2(timeouts)+#3(unwrap_err)→Task 2, #4(bounded channel)→Task 3; Tier 2 #7(stop)→Task 4, #6(bootstrap symmetry)→Task 5; #5 explicitly deferred to a spike. Covered.
- **Type consistency:** `reset_transient` (Task 1) used in reader Tasks; `EVENT_CHANNEL_BOUND` (Task 3) reused in Task 4/5 tests; `stop: &Arc<AtomicBool>` `run`/`reconnect` arity is introduced in Task 4 and the Task 5 test calls `run(.., &stop)` consistently; `SnapshotRestored(Box<SessionSnapshot>)` matches `reader.rs:207`.
- **Ordering hazard:** Tasks 3→4→5 all edit `reader.rs::run`/`reconnect` and MUST run sequentially (each rebases on the prior signature). Tasks 1 and 2 are independent and may precede them.
- **Placeholder scan:** every code step shows real code; the one mock-name caveat (Task 4/5) instructs reuse of the file's existing scripted `Reopen` double rather than inventing names.
