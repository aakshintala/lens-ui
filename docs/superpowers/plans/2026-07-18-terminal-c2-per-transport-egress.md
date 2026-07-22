# Terminal C2 — Per-Transport Egress Channel Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close deferred Critical **C2** — user input (and query replies) queued in the engine's egress channel must never reach a *different* connection after a reconnect or read-only downgrade (a read-only-enforcement bypass).

**Architecture (pivoted after the NO-GO review — see "Why per-transport" below):** Give **each bridge its own egress channel** instead of one engine-retained channel that every bridge clones. The single-threaded worker holds a *swappable* egress sender, changed only via an **in-order command** (`EngineCommand::SetEgress`). On reconnect the caller creates a fresh channel, hands the receiver to the new bridge and the sender to the worker. Because a feed and its reply-emit are one uninterrupted worker dispatch, a query's reply always follows its source connection. When a bridge stops, it drains-and-drops its own residue and surfaces a single "input discarded" notice. There is **no shared MPMC receiver and no generation stamp/filter** — connection identity *is* the channel.

**Tech Stack:** Rust, `crossbeam_channel` (bounded SPSC now — one producer swap, one consumer bridge), gpui 0.2.2, `async_channel` (policy events).

## Global Constraints

- `xtask gate` = `cargo fmt --check`, `clippy --workspace --all-targets -D warnings`, workspace tests. **For `lens-terminal`, per-task clippy MUST include `--features test-util` and `--features live-tests`.** Never pipe the gate through `tail`.
- Commit verified work per task; the whole slice lands as commits on `terminal-ws` (no PR, no merge to `main`).
- `EGRESS_CHANNEL_CAP` is `64` (`worker.rs:20`, used at `worker.rs:125`; `handle.rs:72` uses the literal `64`). Keep the cap.
- **`access_epoch` is NOT renamed, but it IS now bumped on *every* teardown** (was downgrade-only in 2a). This is load-bearing: per-transport channels isolate already-*emitted* residue, but pre-disconnect input also sits *upstream* in the `InputForwarder` queue / `cmd_tx` (un-encoded). Bumping `access_epoch` makes the forwarder's `is_stale` check (`forwarder.rs:141`) and the worker's final recheck (`worker.rs:502`) drop that upstream residue so it can never be encoded onto the new connection. The two layers are complementary: **bump** = un-encoded upstream residue; **per-transport** = already-encoded residue. Neither alone is sufficient.

---

## Why per-transport (what the review changed)

The prior plan (typed egress + monotonic `egress_generation` stamp + bridge-side receive filter on a **shared retained channel**) was reviewed NO-GO (gpt-5.6, with race-path + security subagents). Two findings were structural, both rooted in the *shared* channel:

- **Old/new bridge MPMC race:** `teardown_transport_off_foreground` (`lib.rs:851-867`) joins the old bridge on a *detached* task while `schedule_reconnect` proceeds independently (`DowngradeReadOnly` even uses `Duration::ZERO`, `lib.rs:974`). The new bridge can spawn (`lib.rs:1107`) before the old one exits, and both hold clones of the same `egress_rx` (`bridge.rs:39`) — competing MPMC consumers. A stop-flagged old bridge can still win a `select` and consume/drop a *fresh* frame → newly-accepted input lost, breaking the 2a never-drop invariant. A shared channel + receive filter is intrinsically racy.
- **Reply leak:** the engine buffers replies to *every* query (DA/DSR/DECRQM/DECRPM — verified in vendored `terminal.rs`), not just DA. On a shared channel a reply to the *old* connection's query can be emitted after the boundary and reach the fresh (possibly read-only) connection.

Per-transport channels remove the shared channel, so both disappear structurally: no competition (each bridge owns its receiver), and replies follow their source connection (feed→reply-emit is one dispatch; the sender swap only lands *between* commands). The generation stamp/filter and the 73-site rename become unnecessary.

## Policy (settled with the user)

Drop pre-disconnect residue (at-most-once / clean slate). `input_enabled` is `false` on every teardown path (`lib.rs:967,980`; detach via `set_detached_presentation` `lib.rs:888`), so no new input is enqueued during the reconnect window — only pre-disconnect residue is at issue, and for a shell replaying partial commands is dangerous. Surface the drop minimally (one-shot notice). The worker-side `access_epoch` revocation (2a, unchanged) remains the second layer that also stops write bytes reaching the *old* connection after a downgrade.

---

## Current shape (ground truth, verified)

- Egress channel: `crossbeam_channel::bounded(64)` of raw `Vec<u8>` (`worker.rs:119-125`). `EngineHandle` **retains** `egress_rx` (`handle.rs:41`) and exposes it via `pub fn egress_rx(&self) -> &Receiver<Vec<u8>>` (`handle.rs:203`). Each bridge **clones** it (`bridge.rs:39`).
- Worker is passed `egress_rx` but never uses it — it is threaded through to `handle_command` as `_egress_rx` (`worker.rs:480`, unused). **Vestigial — remove it.**
- Two emit sites in `worker.rs`: `try_emit_user_input` (encoded `Key` at `worker.rs:506`, focus report at `worker.rs:543`; never-drop) and `emit_reply_egress` (DA/DSR at `worker.rs:406`; best-effort drop-on-full). The `Key` and `Focus` worker arms are distinct (`worker.rs:493,529`) so Input-vs-Report classification is implementable.
- `TerminalRuntime` (`runtime.rs:11-15`) holds `bridge/attach/engine`; `take_transport` (`runtime.rs:19`) removes bridge+attach and **retains the engine**; `install_transport` (`runtime.rs:23`) sets them.
- Production `spawn_bridge` call sites: initial attach `policy.rs:152` (`discover_and_attach`), reconnect `lib.rs:1107` (`on_reconnect_success`). Test call sites: `bridge.rs:225,273,297`, `runtime.rs:137,157`.
- `EngineHandle::egress_rx()` is **public** and `EngineHandle` is re-exported (`lib.rs:303`); the integration test `tests/input_realwindow.rs` reads it directly (`:128,:212,:227`) with no bridge. That test is built with `--features test-util,live-tests` under the gate.
- Foreground input choke point: `try_enqueue_key` (`lib.rs:692`), used by `handle_key_down` (`lib.rs:646`), `handle_key_up` (`lib.rs:677`), `enqueue_committed_text` (`lib.rs:709`). It takes `&mut self` **without** `Context`. Focus reports use a separate path (not `try_enqueue_key`).
- `apply_bridge_event` (`lib.rs:925`) maps each `BridgeEvent` to a `PolicyAction` via a match (`lib.rs:928-939`).
- Inspect snapshot is `TerminalInspect` (constructed `lib.rs:556`, defined `inspect.rs:10`) — there is **no** `PresentationSnapshot`. It mirrors `output_gap` (`lib.rs:558`).

---

## File Structure

- `engine/worker.rs` — **owns** `EgressFrame`/`EgressKind`; `EngineCommand::SetEgress(Option<Sender<EgressFrame>>)`; swappable local egress sender in the loop; kind-tagged emits; drop vestigial `egress_rx`.
- `engine/handle.rs` — drop retained `egress_rx` field + `egress_rx()`; add `attach_egress(tx)` (pub(crate)) + `attach_test_egress() -> Receiver<EgressFrame>` (test-util); re-export `EgressFrame`/`EgressKind`.
- `bridge.rs` — `spawn_bridge` takes an **owned** `Receiver<EgressFrame>`; forwards `frame.bytes`; on stop, drain-drop residue + emit one `BridgeEvent::StaleInputDiscarded` if any `Input` frame was dropped.
- `policy.rs`, `lib.rs`, `runtime.rs` — create a fresh channel at each attach; `engine.attach_egress(tx)` + `spawn_bridge(rx)`; presentation surfacing.
- `engine/mod.rs`, `lib.rs` — export the new types.

---

## Task 1: Per-transport egress channel (typed + swappable), no residue policy yet

Replace the retained-shared channel with a per-bridge channel and a worker sender swapped via an in-order command. After this task a single connection and a reconnect both work exactly as before, but each connection has its **own** channel (residue is naturally isolated; the drop-on-stop + surfacing lands in Task 2).

**Files:**
- Modify: `engine/worker.rs`, `engine/handle.rs`, `engine/mod.rs`, `bridge.rs`, `policy.rs`, `lib.rs` (`on_reconnect_success`), `runtime.rs` (test sites), `tests/input_realwindow.rs`
- Test: `engine/handle.rs` (`#[cfg(test)]`)

**Interfaces:**
- Produces:
  - `pub struct EgressFrame { pub kind: EgressKind, pub bytes: Vec<u8> }`
  - `pub enum EgressKind { Input, Other }` (Input = encoded keystrokes/committed text; Other = focus reports + DA/DSR replies — never surfaced)
  - `EngineCommand::SetEgress(Option<crossbeam_channel::Sender<EgressFrame>>)`
  - `EngineHandle::attach_egress(&self, tx: Sender<EgressFrame>)` — pub(crate); enqueues `SetEgress(Some(tx))` via `cmd_tx`
  - `#[cfg(any(test, feature = "test-util"))] EngineHandle::attach_test_egress(&self) -> Receiver<EgressFrame>` — creates a `bounded(64)` channel, attaches the sender, returns the receiver
  - `spawn_bridge(inbound, outbound, engine, policy_tx, egress_rx: Receiver<EgressFrame>) -> BridgeHandle`
- Removed: `EngineHandle::egress_rx()`, the retained `egress_rx` field, the vestigial worker `egress_rx` param.

- [ ] **Step 1: Write the failing test**

In `handle.rs` tests — attaching a channel and feeding a DA query yields a kind-tagged frame; a second `attach_test_egress` swaps the worker to a new channel:

```rust
#[test]
fn egress_goes_to_the_currently_attached_channel() {
    let h = EngineHandle::spawn(test_config());
    let rx1 = h.attach_test_egress();
    h.feed(b"\x1b[c".to_vec()).unwrap(); // Primary DA → reply (kind Other)
    h.build_now().ok();
    let f = recv_frame(&rx1);
    assert_eq!(f.kind, EgressKind::Other);
    assert!(!f.bytes.is_empty());

    // Swap to a fresh channel; the old one receives nothing further.
    let rx2 = h.attach_test_egress();
    h.feed(b"\x1b[c".to_vec()).unwrap();
    h.build_now().ok();
    let f2 = recv_frame(&rx2);
    assert_eq!(f2.kind, EgressKind::Other);
    assert!(rx1.try_recv().is_err(), "old channel must not receive after swap");
    h.stop_and_join();
}
```

Add a small `recv_frame(rx: &Receiver<EgressFrame>) -> EgressFrame` poll helper in the test module (2s deadline, `build_now` loop) mirroring the existing egress-poll patterns. `test_config` is the existing helper (`handle.rs:342`). `stop_and_join`/equivalent teardown per the existing tests.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p lens-terminal --features test-util egress_goes_to_the_currently_attached_channel`
Expected: FAIL to compile (`EgressFrame`, `attach_test_egress` undefined).

- [ ] **Step 3: Define the types + `SetEgress`**

`worker.rs`, near `UserEgressFull` (`worker.rs:96`):

```rust
/// One unit of engine→transport egress, routed to the bridge for the connection that
/// is currently attached. Each connection owns its own channel (C2): residue from a
/// prior connection lives in that connection's channel and is never delivered to a
/// different one.
pub struct EgressFrame {
    pub kind: EgressKind,
    pub bytes: Vec<u8>,
}

/// `Input` = encoded user keystrokes / committed text — a stale drop is user-visible
/// data loss (surfaced). `Other` = focus reports and DA/DSR replies — protocol
/// housekeeping, dropped silently.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EgressKind {
    Input,
    Other,
}
```

Add `SetEgress(Option<Sender<EgressFrame>>)` to `EngineCommand` (`worker.rs:100`).

- [ ] **Step 4: Make the worker's egress sender swappable + kind-tag emits + drop vestigial rx**

`worker.rs` (resolve the wiring cleanly — no egress channel is created at worker spawn anymore):
- `WorkerChannels`: **remove** `egress_tx` and `egress_rx` entirely (the egress channel is created per-attach by the caller, not bundled with the worker). `worker_channels()` returns only `cmd_tx`/`cmd_rx`.
- `spawn_worker`: drop both egress params. The worker's egress sender is a **loop-local** `let mut egress: Option<Sender<EgressFrame>> = None;`, mutated only by `EngineCommand::SetEgress(tx) => egress = tx;`.
- Remove the vestigial `egress_rx` threading entirely from `dispatch_command`, `handle_command` (the `_egress_rx`), `handle_feed_chunked`, `drain_pending_after_feed`, `feed_chunk`. Thread `egress: Option<&Sender<EgressFrame>>` (and `&mut` where `SetEgress` swaps it) where `egress_tx` was threaded.
- `EGRESS_CHANNEL_CAP` (`worker.rs:20`) is now unused at spawn — make it `pub(crate)` and reuse it at the per-attach channel creation sites (`attach_test_egress`, `discover_and_attach`, `on_reconnect_success`) as `bounded(EGRESS_CHANNEL_CAP)` so it stays referenced and the cap stays single-sourced (avoids a dead-const `-D warnings` failure).
- Pass `&mut Option<Sender<EgressFrame>>` (or `&mut` to the loop-local) into `dispatch_command`/`handle_command` so `SetEgress` can swap it: `EngineCommand::SetEgress(tx) => { *egress = tx; }`.
- Emit helpers take `Option<&Sender<EgressFrame>>`. **Honest ack (closes review finding: None/Disconnected must NOT report success):** user-input emit returns *whether the bytes were actually delivered to a live channel*; `None` (no bridge attached) and `Disconnected` (bridge gone) both count as **not delivered** → `accepted: false`, exactly like `Full`. This corrects the 2a code that returned `Ok(())` on `Disconnected` (`worker.rs:675`), which would false-ack un-sent input.

```rust
/// Returns true iff `bytes` were handed to a live egress channel.
fn try_emit_user_input(
    tx: Option<&Sender<EgressFrame>>,
    kind: EgressKind,
    bytes: &[u8],
) -> bool {
    let Some(tx) = tx else { return false }; // no bridge attached → not delivered
    match tx.try_send(EgressFrame { kind, bytes: bytes.to_vec() }) {
        Ok(()) => true,
        Err(TrySendError::Full(_)) => false,         // never-drop reject → reconnect via bridge
        Err(TrySendError::Disconnected(_)) => false, // bridge gone → not delivered
    }
}

fn emit_reply_egress(tx: Option<&Sender<EgressFrame>>, replies: Vec<u8>) {
    let Some(tx) = tx else { return }; // no reply ack; drop silently
    let _ = tx.try_send(EgressFrame { kind: EgressKind::Other, bytes: replies });
    // Full/Disconnected → best-effort drop; never evict never-drop input.
}
```

- `Key` arm (`worker.rs:493-527`): call `try_emit_user_input(egress, EgressKind::Input, &bytes)`; set `accepted` from its bool, and `record_user_egress_accepted()` / `record_user_egress_rejected()` accordingly (unchanged counter semantics — `Full`, `None`, and `Disconnected` all record *rejected*). `Focus` arm (`worker.rs:543-549`): `try_emit_user_input(egress, EgressKind::Other, &bytes)` — focus has no `InputAck`, but **keep the counter recording** (2a records accepted/rejected for focus): `if delivered { record_user_egress_accepted() } else { record_user_egress_rejected() }`. Do not drop this branch. `feed_chunk` reply (`worker.rs:406`): `emit_reply_egress(egress, replies)`. Thread the `Option<&Sender<EgressFrame>>` where `egress_tx` was threaded. Update the 2a test `user_input_egress_full_does_not_drop_or_false_ack` and any test asserting Ok-on-Disconnected to expect `accepted:false` on a disconnected/None channel.
- Keep the `access_epoch` recheck logic (`worker.rs:487,502,539`) exactly as-is.

`handle.rs`:
- Drop the `egress_rx` field (`handle.rs:41`), the `egress_rx()` accessor (`handle.rs:203`), and stop passing `egress_rx` into `spawn_worker` / `spawn_from_parts` / `spawn_with_cmd_cap`. The worker starts with `egress: None`.
- Add:

```rust
/// Point the worker's egress at `tx` via an in-order `SetEgress` command, so a query
/// fed on the prior connection has already emitted its reply to the prior channel
/// before this takes effect. Returns `Err` if the command could not be enqueued —
/// the caller MUST NOT then spawn a bridge on the paired receiver (see below).
pub(crate) fn attach_egress(
    &self,
    tx: crossbeam_channel::Sender<super::worker::EgressFrame>,
) -> Result<(), FeedError> {
    self.cmd_tx
        .try_send(EngineCommand::SetEgress(Some(tx)))
        .map_err(|e| match e {
            TrySendError::Full(_) => FeedError::Full,
            TrySendError::Disconnected(_) => FeedError::Stopped,
        })
}

#[cfg(any(test, feature = "test-util"))]
pub fn attach_test_egress(&self) -> crossbeam_channel::Receiver<super::worker::EgressFrame> {
    let (tx, rx) = crossbeam_channel::bounded(64);
    self.attach_egress(tx).expect("attach_test_egress: cmd_tx full");
    rx
}
```

**Why `Result` matters (closes review High finding):** if `attach_egress` fails and the caller *still* spawns the bridge, the `tx` is dropped (returned in the `try_send` Err and discarded), so the bridge's `egress_rx` sees all senders gone → `Disconnected` → the bridge falsely emits `EngineStopped` (`bridge.rs:115`) and the tab tears down as if the engine died. The caller therefore treats an `attach_egress` error as a (retryable) attach failure and does **not** spawn the bridge (Step 6). At initial attach `cmd_tx` is empty and at reconnect the engine has been idle through the reconnect window, so this path is near-unreachable — but it must fail safe, not false-stop. `attach_test_egress` is `pub` (gated) because the external `tests/input_realwindow.rs` (a separate crate) consumes it; it is only compiled under `test`/`test-util`.

`engine/mod.rs` + `lib.rs`: export `EgressFrame`, `EgressKind` (the integration test consumes them via the public re-export, mirroring how `EngineHandle` is re-exported at `lib.rs:303`).

- [ ] **Step 5: Bridge owns its receiver**

`bridge.rs`:
- `spawn_bridge` signature gains `egress_rx: Receiver<EgressFrame>` (last param) and **stops cloning** `engine.egress_rx()` (delete `bridge.rs:39`). `bridge_loop`'s `egress_rx: Receiver<EgressFrame>`.
- `forward_egress` takes `bytes: Vec<u8>`; the egress arm builds `WsOutbound::Input(frame.bytes)` — forward every frame this task (drop-on-stop lands in Task 2).
- The 3 bridge test call sites (`225,273,297`) create a channel and attach it: `let rx = engine.attach_test_egress(); let bridge = spawn_bridge(.., rx);` (their existing `outbound_rx` assertions are unchanged).

- [ ] **Step 6: Bump `access_epoch` on every teardown (Critical — revokes upstream residue)**

`lib.rs` `teardown_transport_off_foreground` (`lib.rs:851`) — bump synchronously at the top, before `take_transport`, so the `InputForwarder`'s `is_stale` check and the worker's recheck drop any pre-disconnect input still queued upstream (un-encoded). This is what stops a plain `Retry` from flushing pre-disconnect keystrokes onto the reconnected channel after the `SetEgress` swap:

```rust
fn teardown_transport_off_foreground(&mut self, cx: &mut Context<Self>) {
    if let Some(rt) = &mut self.runtime {
        // Revoke any input still queued upstream (forwarder/cmd_tx): it belongs to the
        // connection being torn down and must never be encoded onto the next one.
        // Per-transport channels isolate already-emitted residue; this closes the
        // un-encoded-residue path (C2). Covers Retry AND downgrade.
        if let Some(engine) = rt.engine_ref() {
            engine.bump_access_epoch();
        }
        let (bridge, attach) = rt.take_transport();
        // ... unchanged ...
    }
}
```

Keep the existing early `bump_access_epoch()` in `DowngradeReadOnly` (`lib.rs:962`) — it revokes ASAP, before teardown; a double bump is harmless (monotonic). `on_reconnect_success` stamps new input against the post-bump epoch, so post-reconnect keystrokes are valid. (`rt.engine_ref()` → `Option<&Arc<EngineHandle>>`, `runtime.rs:41`.)

- [ ] **Step 7: Wire the production attach sites (with fail-safe on `attach_egress`)**

`policy.rs` `discover_and_attach` (`policy.rs:148-157`): after `let engine = Arc::new(EngineHandle::spawn(cfg));`, create the channel, attach (mapping failure to the attach-failed error), then spawn the bridge only on success:

```rust
let engine = Arc::new(EngineHandle::spawn(cfg));
let (egress_tx, egress_rx) = crossbeam_channel::bounded(64);
engine
    .attach_egress(egress_tx)
    .map_err(|_| DetachedDetail::DiscoveryFailed)?; // do NOT spawn a bridge on an orphan channel
// ...
let bridge = bridge::spawn_bridge(
    attach_handle.inbound.clone(),
    attach_handle.outbound.clone(),
    Arc::clone(&engine),
    policy_tx.clone(),
    egress_rx,
);
```

`lib.rs` `on_reconnect_success` (`lib.rs:1104-1112`): fresh channel `bounded(EGRESS_CHANNEL_CAP)`, `engine.attach_egress(egress_tx)` **before** spawning. On error, this defensive path (near-unreachable — `CMD_CHANNEL_CAP=256` and the engine is idle through the reconnect window) must fail safe: (a) **do not** spawn the bridge (a dropped `tx` → false `EngineStopped`, Step 4); (b) close the just-obtained `attach` **off-foreground**, exactly mirroring the existing `else`-branch at `lib.rs:1089-1101` (`cx.spawn(async move |_w, cx| cx.background_executor().spawn(async move { attach.close() }).await).detach()`) — never drop an `AttachHandle` on the gpui foreground; (c) re-arm reconnection via `self.schedule_reconnect(<retry delay>, cx)` **without** `policy.retry.reset()`, so the retry budget/window is not bypassed; then `return`. On success, `spawn_bridge(.., egress_rx)`.

`runtime.rs` test sites (`137,157`): `let rx = engine.attach_test_egress();` then pass `rx`.

- [ ] **Step 8: Migrate EVERY `egress_rx()` user (unit + integration)**

Removing `egress_rx()` breaks all current consumers — enumerate and migrate each (the reviewer flagged unaccounted users). `grep -rn "egress_rx()" crates/lens-terminal` and convert each:

- **`handle.rs` `#[cfg(test)]`** (`:522,:850,:905,:1020,:1076` and the `.len()` sites `:1057,:1063,:1069`): replace the retained accessor with a per-test attached channel — `let rx = h.attach_test_egress();` once after spawn, then `rx.try_recv()` / `rx.len()`, reading `.bytes`. The two fullness tests (`user_input_egress_full_*`, `reply_egress_full_*`) that fill to 64 now fill the *attached* channel; keep the cap assertions on `rx`.
- **`bridge.rs` `#[cfg(test)]`** (`:225,:273,:297`): `let rx = engine.attach_test_egress();` then `spawn_bridge(.., rx)` (Step 5). The `:247` `outbound_rx` DA-forward assert reads `WsOutbound::Input` (unchanged).
- **`runtime.rs` `#[cfg(test)]`** (`:137,:157`): as Step 7.
- **`lib.rs` `#[cfg(test)]`** (`:1673,:1687,:1693,:1698` — the `TerminalTab::with_engine_for_test` keydown/committed-text test): after `EngineHandle::spawn`, `let rx = engine.attach_test_egress();` and read `rx.try_recv()` / `rx.recv_timeout(..)` using `.bytes` (`assert_eq!(frame.bytes, b"a")`).
- **`tests/input_realwindow.rs`** (external crate): in the fixture ctor (~`:112`) `let egress = engine.attach_test_egress();`, store `egress: Receiver<EgressFrame>`; `drain_egress` (`:127`) and `await_single_egress` (`:199,:212,:227`) read `self.egress` and use `frame.bytes`. `use lens_terminal::EgressFrame;`.

After migration, `grep -rn "egress_rx()" crates/lens-terminal` returns nothing.

- [ ] **Step 9: Run the new test + gate**

Run: `cargo test -p lens-terminal --features test-util egress_goes_to_the_currently_attached_channel` → PASS
Run: `cargo xtask gate` **and** `cargo clippy -p lens-terminal --all-targets --features test-util,live-tests -- -D warnings` → PASS (watch for newly-unused imports after dropping `egress_rx`)
Run: `cargo test -p lens-terminal --features test-util,live-tests --test input_realwindow` → PASS (encoding path unchanged).

- [ ] **Step 10: Commit**

```bash
git add -A crates/lens-terminal
git commit -m "refactor(lens-terminal): per-transport egress channel (swappable worker sender) — C2 T1"
```

---

## Task 2: Drop residue on bridge stop + surface it (`StaleInputDiscarded`)

When a bridge stops (reconnect/downgrade/detach), it must **drop** its remaining egress (drop policy) rather than let it linger, and surface a single notice if any of it was user `Input`. Because the channel is per-bridge (owned, not shared), draining on stop is race-free.

**Files:**
- Modify: `bridge.rs` (`BridgeEvent::StaleInputDiscarded`; drain-drop-count on loop exit)
- Test: `bridge.rs` (`#[cfg(test)]`), `lib.rs` (`#[cfg(test)]` lifecycle-level)

**Interfaces:**
- Consumes: `EgressFrame`, `EgressKind`, `spawn_bridge(.., egress_rx)` (T1).
- Produces: `BridgeEvent::StaleInputDiscarded`.

- [ ] **Step 1: Write the failing tests**

Test A (bridge-level) — a stopped bridge drops queued Input and emits exactly one notice:

```rust
#[test]
fn stopped_bridge_drops_residue_and_surfaces_once() {
    let engine = Arc::new(EngineHandle::spawn(test_cfg()));
    let (egress_tx, egress_rx) = crossbeam_channel::bounded(64);
    // Two queued Input frames + one Other, none yet forwarded.
    egress_tx.send(EgressFrame { kind: EgressKind::Input, bytes: b"ab".to_vec() }).unwrap();
    egress_tx.send(EgressFrame { kind: EgressKind::Input, bytes: b"cd".to_vec() }).unwrap();
    egress_tx.send(EgressFrame { kind: EgressKind::Other, bytes: b"\x1b[0n".to_vec() }).unwrap();

    let (_inbound_tx, inbound_rx) = crossbeam_channel::bounded(8);
    // Cap-1 outbound, pre-filled, so the bridge cannot forward before we stop it.
    let (outbound_tx, outbound_rx) = crossbeam_channel::bounded(1);
    outbound_tx.send(WsOutbound::Input(vec![9])).unwrap();
    let (policy_tx, policy_rx) = async_channel::bounded(8);
    let bridge = spawn_bridge(inbound_rx, outbound_tx, Arc::clone(&engine), policy_tx, egress_rx);

    bridge.join(); // stop → drain-drop residue

    // Exactly one StaleInputDiscarded (coalesced), and no residue on outbound beyond the pre-fill.
    let mut notices = 0;
    while let Ok(ev) = policy_rx.try_recv() {
        if matches!(ev, BridgeEvent::StaleInputDiscarded) { notices += 1; }
    }
    assert_eq!(notices, 1, "coalesced to one notice");
    // Only the pre-filled sentinel could be on outbound; the two Input frames must NOT appear.
    let forwarded: Vec<_> = std::iter::from_fn(|| outbound_rx.try_recv().ok()).collect();
    assert!(forwarded.iter().all(|m| !matches!(m, WsOutbound::Input(b) if b == b"ab" || b == b"cd")));
}
```

Test B (bridge-level) — a stopped bridge with only `Other` residue emits **no** notice (assert `policy_rx` yields no `StaleInputDiscarded`).

Test C (lifecycle-level, in `lib.rs`) — reconnect does not replay pre-disconnect Input to the new outbound, covering **both** residue paths:
  - **C1 (already-emitted residue):** queue Input into the engine's current egress channel, trigger `Retry`, assert the new attach's outbound never receives it.
  - **C2 (un-encoded upstream residue — the Critical):** must be **deterministic**, not timing-dependent — `enqueue_input` alone only admits the `Key` to the forwarder (`handle.rs:133-145`); the worker could encode it onto the old channel before `Retry`, letting the test pass even without the Step 6 bump (false green). Instead, use the 2a worker-stall barrier: `engine.test_stall_worker()` (`handle.rs:229`) so the worker cannot drain `cmd_tx`; `enqueue_input` the `Key` and confirm it is provably held upstream (forwarder/`cmd_tx`, not emitted); trigger `Retry` (teardown bumps `access_epoch`); `engine.test_release_worker()`; then assert the key is **never** encoded onto the new attach's outbound. The test MUST be shown to FAIL with Step 6 removed and PASS with it (state this explicitly in the test comment; verify by temporarily deleting the bump during development).
  - Also add a `DowngradeReadOnly` variant asserting write bytes queued pre-downgrade never reach the read-only reconnection (same stall-barrier technique).

Follow the harness the neighboring reconnect/`output_gap` tests use — do not invent one. If those tests cannot observe the new attach's outbound, add a `#[cfg(test)]` seam on `TerminalRuntime`/attach to inspect it. (`test_stall_worker`/`test_release_worker` are `#[cfg(any(test, feature = "test-util"))]`, `handle.rs:229-239`.)

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p lens-terminal --features test-util stopped_bridge_drops_residue_and_surfaces_once` (+ B, C individually — one filter per invocation)
Expected: FAIL (`StaleInputDiscarded` undefined; residue currently forwarded, not dropped).

- [ ] **Step 3: Add the variant + drain-drop-count on stop**

`bridge.rs` — add to `BridgeEvent` (`bridge.rs:15`):

```rust
/// The bridge stopped with un-forwarded user `Input` still queued in its egress
/// channel; that residue was dropped (not replayed to any connection). Coalesced to
/// one per bridge stop. Reply/focus (`Other`) residue is dropped silently.
StaleInputDiscarded,
```

At every `break` out of `bridge_loop` (the loop currently breaks on stop, `AttachDisconnected`, `EngineStopped`, etc.), route through a single cleanup that drains the owned `egress_rx`:

```rust
// After the select loop exits, before returning:
let mut dropped_input = false;
while let Ok(frame) = egress_rx.try_recv() {
    if frame.kind == EgressKind::Input && !frame.bytes.is_empty() {
        dropped_input = true;
    }
    // all residue is dropped (drop policy); Other is silent
}
if dropped_input {
    let _ = policy_tx.try_send(BridgeEvent::StaleInputDiscarded);
}
```

Structure the loop so all exits fall through to this drain (e.g., `break` to after the loop, then drain, then return) — one `try_send` max, so it cannot flood the 32-cap policy channel (closes review finding 3). **Best-effort by design:** the worker's sender isn't swapped until `on_reconnect_success` (after this bridge is joined), so a frame the worker pushes *after* the drain sees `Empty` lingers in this channel until the bridge's owned receiver is dropped on thread return (then GC'd — never delivered elsewhere). The notice may therefore *undercount* dropped input; that is acceptable for a cosmetic one-shot notice and does not affect the security property (those frames are still never delivered to another connection). Document this in a code comment.

- [ ] **Step 4: Run tests + gate**

Run: `cargo test -p lens-terminal --features test-util` (A, B, C + all existing) → PASS
Run: `cargo xtask gate` + `cargo clippy -p lens-terminal --all-targets --features test-util,live-tests -- -D warnings` → PASS

- [ ] **Step 5: Commit**

```bash
git add -A crates/lens-terminal
git commit -m "fix(lens-terminal): drop egress residue on bridge stop + coalesced stale-input notice — closes C2 (C2 T2)"
```

---

## Task 3: Surface the discard in the presentation (minimal)

Capture the signal (only available at stop time); render minimally. Rich treatment is deferred to the presentation slice (2d+).

**Files:**
- Modify: `lib.rs` (`Presentation.input_discarded`; `apply_bridge_event` early arm; clear on next accepted Key; `TerminalInspect` mirror), `inspect.rs` (mirror field)
- Test: `lib.rs` (`#[cfg(test)]`)

**Interfaces:**
- Consumes: `BridgeEvent::StaleInputDiscarded` (T2).
- Produces: `Presentation.input_discarded: bool`, `TerminalInspect.input_discarded: bool`.

- [ ] **Step 1: Write the failing test**

`lib.rs` — using the neighboring presentation-test harness: deliver `BridgeEvent::StaleInputDiscarded`, assert `input_discarded` becomes true; then a successful `handle_key_down` (write-enabled) clears it; a focus report does **not** clear it. Match the construction the adjacent `output_gap` tests use (`lib.rs` reconnect tests). A `#[gpui::test]` alone will not validate real text dispatch (NoopTextSystem — see `[[gpui-test-noop-text-system]]`); for the *clear* assertion drive the real committed-text/keydown path the way the 2a real-window test does, or assert at the `try_enqueue_key`-returns-true boundary rather than through faked text shaping.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p lens-terminal --features test-util <test_name>`
Expected: FAIL (`input_discarded` missing).

- [ ] **Step 3: Add the field + mirror**

`Presentation` (`lib.rs:272`), after `output_gap`:

```rust
/// Set when a reconnect/downgrade dropped user input that had not been sent (C2).
/// One-shot notice — cleared on the next accepted user keystroke. Cosmetic; never
/// identity/authorization.
pub input_discarded: bool,
```

Initialize `false` at every `Presentation { .. }` literal (`lib.rs:430` region, `lib.rs:1359` region). Add `input_discarded: bool` to `TerminalInspect` (`inspect.rs:10`) and set it from `self.presentation.input_discarded` at `lib.rs:558`.

- [ ] **Step 4: Handle the event as a match arm (not an early return)**

Adding a `BridgeEvent` variant makes the `apply_bridge_event` match (`lib.rs:928-939`) non-exhaustive; a pre-match `if` does **not** satisfy exhaustiveness (closes review finding 4). Add it as an arm whose block ends in `return` (which coerces to the match's `PolicyAction` result type):

```rust
let action = match ev {
    // ... existing arms ...
    BridgeEvent::StaleInputDiscarded => {
        self.presentation.input_discarded = true;
        cx.emit(TerminalEvent::PresentationChanged);
        cx.notify();
        return; // not a policy transition
    }
};
```

- [ ] **Step 5: Clear on the next accepted keystroke (defined semantics)**

"Accepted" = admitted to the forwarder (`try_enqueue_key` returned `true`) — a local, synchronous signal the foreground already has. Clear **after** success, in the callers that hold `cx` (closes review finding 9; `try_enqueue_key` itself has no `Context`). Add a tiny helper and call it on the `enqueued`/`true` branches only:

```rust
fn clear_input_discarded(&mut self, cx: &mut Context<Self>) {
    if self.presentation.input_discarded {
        self.presentation.input_discarded = false;
        cx.emit(TerminalEvent::PresentationChanged);
        cx.notify();
    }
}
```

Call it in `handle_key_down` inside `if enqueued { ... }` (`lib.rs:655`) and in the committed-text path after `enqueue_committed_text` returns true (`EntityInputHandler::replace_text_in_range`). Do **not** clear in `handle_key_up` (a release is not new input) and **not** on focus reports. Do not clear on a failed enqueue.

- [ ] **Step 6: Minimal render**

Where the render consumes `output_gap` (the reconnect discontinuity indicator), add one adjacent subtle indicator when `input_discarded`, reusing existing `output_gap` styling — a single element, no new visual language. If `output_gap` is only state (not yet rendered text), likewise wire `input_discarded` as state + the `TerminalInspect` mirror only, with a `// rendered by presentation slice (2d+)` note; do not introduce a new widget in C2.

- [ ] **Step 7: Run test + gate**

Run: `cargo test -p lens-terminal --features test-util <test_name>` → PASS
Run: `cargo xtask gate` + `cargo clippy -p lens-terminal --all-targets --features test-util,live-tests -- -D warnings` → PASS

- [ ] **Step 8: Commit**

```bash
git add -A crates/lens-terminal
git commit -m "feat(lens-terminal): surface stale-input-discarded as one-shot presentation notice (C2 T3)"
```

---

## Post-execution

- [ ] Cross-family whole-slice review (a family **other** than gpt-5.6 for diversity — grok-4.5 / gemini-3.5 via cursor-delegate), plus an Opus synthesis pass given the security surface. Focus: no residue path to a *different* connection (input or reply), never-drop preserved on the live connection, the `SetEgress` swap ordering vs feed/reply dispatch, notice coalescing, clear-on-input semantics.
- [ ] A live rider if feasible (drive a real reconnect/downgrade against omnigent 0.5.1 and confirm no replayed keystrokes) — else document why the deterministic lifecycle tests (T2 Test C) are the authoritative proof.
- [ ] Update memory `terminal-slice-2a-executed` (C2 CLOSED + the pivot), refresh the handoff and `docs/STATUS.md`.
- [ ] Then execute 2d (presentation).

---

## Review findings folded (two NO-GO rounds)

**Round 1 (shared-channel design):** 1 MPMC race + 2 reply leak → removed at the root by the per-transport pivot; 3, 4, 8, 9 → closed as below.

**Round 2 (per-transport design):** confirmed 1, 2, 3, 4, 8, 9 closed; reopened the following, now fixed:

- **★ Critical — upstream (un-encoded) residue.** Per-transport isolates *emitted* frames, but pre-disconnect input in the `InputForwarder`/`cmd_tx` would be encoded onto the new channel after `SetEgress` because `Retry` didn't bump `access_epoch`. **Fix:** bump `access_epoch` on every teardown (Task 1 Step 6) + lifecycle test C2 (Task 2 Step 1). This is the layer the pivot wrongly dropped.
- **High — `attach_egress` failure → false `EngineStopped`.** Now returns `Result`; caller does not spawn a bridge on an orphan channel (Task 1 Steps 4, 7).
- **High — None/Disconnected egress false-ack.** Emit now returns delivered-or-not; None/Disconnected → `accepted:false` (Task 1 Step 4), correcting 2a `worker.rs:675`.
- **Build blocker — `attach_test_egress` visibility + unaccounted `egress_rx()` users.** `attach_test_egress` is gated-`pub`; every `egress_rx()` user (handle/bridge/runtime tests + integration test) migrated (Task 1 Step 8).
- **Finding 6 (security proof)** → now rests on channel isolation **plus** upstream epoch revocation — both stated in Self-Review.
- **Finding 7 (tests)** → C split into C1 (emitted) + C2 (upstream) + downgrade (Task 2 Step 1).
- **Finding 10 (factual)** → `TerminalInspect` not `PresentationSnapshot`, `test_config`, one filter per `cargo test`, no rename, line refs refreshed.
- **Confirmed safe by review:** reply routing within a chunked `Feed` (pending commands execute only after the full feed, `worker.rs:369-389`); reconnect-triggering bridge events terminate the old bridge loop.

## Self-Review

- **Coverage:** per-transport channel (T1), residue drop + surface signal (T2), presentation notice (T3). Security property (no cross-connection residue), reply-source, never-drop, coalescing, exhaustiveness, API/tests all mapped to tasks + the folded-findings table. ✅
- **Type consistency:** `EgressFrame { kind, bytes }`, `EgressKind { Input, Other }`, `EngineCommand::SetEgress(Option<Sender<EgressFrame>>)`, `attach_egress`/`attach_test_egress`, `spawn_bridge(.., egress_rx: Receiver<EgressFrame>)`, `BridgeEvent::StaleInputDiscarded`, `Presentation.input_discarded`/`TerminalInspect.input_discarded` — consistent across tasks. ✅
- **Security argument (two layers):** (1) **Already-emitted residue** — after any teardown the worker's sender still points at the *old* channel until `on_reconnect_success` swaps it; the new bridge reads a *fresh, empty* channel, so no old-connection frame (input or reply) can reach it. Old residue is dropped when the old bridge stops (drain-drop) or when its receiver is dropped. (2) **Un-encoded upstream residue** — `bump_access_epoch()` on every teardown (Task 1 Step 6) makes the `InputForwarder` `is_stale` (`forwarder.rs:141`) and the worker recheck (`worker.rs:502`) drop pre-disconnect input still queued in the forwarder/`cmd_tx`, so it is never encoded onto the new channel. Both layers are required; neither alone suffices. `access_epoch` additionally stops write bytes reaching the *old* connection after a downgrade (2a). ✅
- **Placeholder scan:** Task 2 Step 1 (Test C) and Task 3 Step 1/6 defer to "the existing harness/render pattern" — fidelity instructions to match neighbors, not unspecified logic; the fields, events, and clear logic are fully given. ✅
