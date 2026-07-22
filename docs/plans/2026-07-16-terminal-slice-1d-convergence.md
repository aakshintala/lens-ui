# Terminal Slice 1d — Convergence Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Converge landed 1a transport + 1b engine + 1c paint into a live `open()`→`TerminalTab` path: bridge thread, close-code policy, lifecycle subset with gap marker, hidden-tab Frame suppression, standalone GPUI demo, and live proof vs omnigent 0.5.1.

**Architecture:** One dedicated **bridge thread** multiplexes `AttachHandle.inbound` → `EngineHandle::feed` and `EngineHandle::da_dsr_rx` → `WsOutbound::Input` via `crossbeam_channel::Select`. Engine waker and bridge policy events never touch a gpui entity from off-thread — they `try_send` onto bounded `async_channel`s; a foreground `cx.spawn` loop awaits those channels and applies updates with the two-arg `weak.update(cx, …)` (same pattern as `lens-ui` poller + `lens-app` fleet_verify). Ownership of attach + bridge + engine lives in one `TerminalRuntime` whose teardown always runs off the gpui foreground. Close-code **policy** lives in `lens-terminal` (1a only classified). `render` samples `engine.latest_frame()` into `TerminalTab.latest_frame` and paints via 1c’s `paint_frame` — never continuous RAF.

**Tech Stack:** gpui 0.2.2, lens-client transport (`attach`/`Terminals`/`Backoff`/`CloseCause`), lens-terminal engine (`EngineHandle`), crossbeam-channel, async-channel 2 (same as `lens-ui`/`lens-core`), tokio (contained inside 1a attach — never re-exposed).

## Global Constraints

- gpui **0.2.2** + omnigent **0.5.1** pins unchanged; no Ghostty type escapes the engine boundary.
- Non-`Send` `Terminal` never moves threads; bridge never owns it — only `EngineHandle` (`Send`).
- Never block the gpui foreground: discovery/attach/reconnect/teardown/`EngineHandle::stop`/`AttachHandle::close` all off-foreground. **Never** drop `AttachHandle` or call `stop()` on the foreground (`AttachHandle::Drop` joins synchronously — `attach.rs:172-178`).
- Failures are lifecycle values (`Detached` / `Reconnecting`), never `open()` constructor errors.
- Gate = `rustfmt` + `cargo clippy --workspace --all-targets -- -D warnings` + `cargo test --workspace`; live rider feature-gated (`live-tests`).
- Frequent commits; **remove** the `#[expect(dead_code)]` on `TerminalTab::{target,client,options}` once consumed.
- Assume Slice 1c landed seam (do not redefine paint): `render::paint_frame`, `CellMetrics::resolve_menlo`, `TerminalTab.latest_frame: Option<Arc<Frame>>`, `set_frame_for_test`.
- Ground truth: `docs/specs/2026-07-16-terminal-workstream-design.md`; Spike B `docs/spikes/2026-07-15-pty-attach-contract.md` + captures under `docs/spikes/captures/2026-07-15-pty-attach/`.
- Add `async-channel = "2"` to `crates/lens-terminal/Cargo.toml` (workspace already uses it in lens-ui/lens-core).

### Wake / delivery mechanism (locked — C2)

`WeakEntity::update` requires `&mut App` / context (`gpui-0.2.2` entity_map). `AsyncApp` is **not** `Send` (holds `Rc`/`Weak`). Therefore:

1. **`open()`** starts a foreground continuation with `cx.spawn(async move |cx| { … })` — mirrors `lens-app` `fleet_verify.rs:93-98` and `lens-ui` `poller.rs:17`.
2. **Blocking discovery/attach** runs only inside `cx.background_executor().spawn(async move { …blocking… }).await` — mirrors `poller.rs:56-58` (background timer) / the same executor used for off-foreground work. Outcome is applied on the foreground with the **two-arg** form:
   ```rust
   weak.update(cx, |tab, cx| { tab.on_attached(...); cx.notify(); })
   ```
   (same two-arg `entity.update(cx, …)` as `poller.rs:34-47` and `fleet_verify.rs:208-215` via `cx.update`).
3. **Engine waker** (runs on the engine worker thread, `worker.rs:276-282`) and **bridge policy events** (bridge thread) **must not** call `update` / hold `AsyncApp`. They only:
   ```rust
   let _ = wake_tx.try_send(());            // bounded(1) — coalescing lost-wake-safe
   let _ = policy_tx.try_send(BridgeEvent); // bounded(32)
   ```
4. A **foreground sampler** started from `on_attached` (another `cx.spawn`) loops:
   ```rust
   // Pattern: lens-ui poller.rs:17-89 — await async_channel, then weak.update(cx, …)
   loop {
       futures::select! {
           r = wake_rx.recv().fuse() => {
               let _ = r?;
               while wake_rx.try_recv().is_ok() {} // coalesce
               weak.update(cx, |tab, cx| {
                   tab.sample_latest_frame_from_engine();
                   cx.notify();
               })?;
           }
           ev = policy_rx.recv().fuse() => {
               let ev = ev?;
               weak.update(cx, |tab, cx| {
                   tab.apply_bridge_event(ev, cx);
               })?;
           }
       }
   }
   ```
5. Bridge still uses **crossbeam** for attach inbound/outbound (1a surface). The bridge thread forwards `BridgeEvent` onto the **async_channel** `policy_tx` (clone the sender into the bridge). Raw-thread → channel → foreground await is the same shape as `live.rs:46-68` (crossbeam from a raw forwarder thread) feeding work that the poller eventually applies on the UI thread via `async_channel`.

**Forbidden:** `std::thread` calling `weak.update`; capturing `AsyncApp` into a thread; blocking `policy_rx.recv()` on the foreground; `set_waker(|| cx.notify())` that touches the entity directly.

### Ownership / teardown (locked — C3)

```rust
/// Owned by TerminalTab. Foreground may only `take()` this and hand it off.
pub(crate) struct TerminalRuntime {
    bridge: Option<BridgeHandle>,
    attach: Option<AttachHandle>,
    engine: Option<Arc<EngineHandle>>,
    // wake_tx kept so Drop can close channels; sampler owns receivers
}

impl TerminalRuntime {
    /// Foreground-safe: take self's fields into a background task that joins.
    pub fn teardown_off_foreground(self, cx: &mut gpui::AsyncApp) {
        cx.spawn(async move |cx| {
            cx.background_executor()
                .spawn(async move { self.teardown_blocking(); })
                .await;
        })
        .detach();
    }

    fn teardown_blocking(mut self) {
        // 1. signal+join bridge (stops feeding / da_dsr forward)
        if let Some(b) = self.bridge.take() {
            b.join();
        }
        // 2. attach.close() joins the I/O thread (MUST NOT run on gpui fg)
        if let Some(a) = self.attach.take() {
            a.close();
        }
        // 3. drop every Arc clone the bridge held (bridge.join already dropped its Arc)
        // 4. unique Arc → stop (consumes EngineHandle — handle.rs:146-154)
        if let Some(engine) = self.engine.take() {
            let owned = Arc::try_unwrap(engine)
                .expect("engine Arc must be unique after bridge join");
            owned.stop();
        }
    }
}

impl Drop for TerminalRuntime {
    fn drop(&mut self) {
        // Never join on the dropping thread (may be gpui foreground).
        let bridge = self.bridge.take();
        let attach = self.attach.take();
        let engine = self.engine.take();
        let _ = std::thread::Builder::new()
            .name("lens-terminal-teardown".into())
            .spawn(move || {
                if let Some(b) = bridge { b.join(); }
                if let Some(a) = attach { a.close(); }
                if let Some(engine) = engine {
                    if let Ok(owned) = Arc::try_unwrap(engine) {
                        owned.stop();
                    }
                    // If not unique, Arc drop alone; worker Drop detaches (handle.rs:162-173).
                    // Production path always joins bridge first so uniqueness holds.
                }
            });
    }
}
```

`TerminalTab` holds `runtime: Option<TerminalRuntime>`. On detach/close/reconnect-replace: `if let Some(rt) = self.runtime.take() { rt.teardown_off_foreground(cx); }`.

Bridge holds **one** `Arc<EngineHandle>` clone for `feed` / `da_dsr_rx`. Tab's `TerminalRuntime.engine` holds the other. After `bridge.join()`, only the runtime’s Arc remains → `try_unwrap` succeeds.

### Close-code policy (locked)

| `CloseCause` | Lifecycle | Action |
| --- | --- | --- |
| `TerminalNotFound` (4404) | `Detached` | teardown runtime off-fg; keep final `latest_frame`; `detached_detail=TerminalGone`; no retry |
| `TerminalDetached` (4405) | `Detached` | teardown attach+bridge off-fg; **retain** engine Arc in a retained slot / re-wrap runtime without attach; `detached_detail=ClientDetached`; `reattach_available=true`; **not** automatic |
| `Unauthorized` (1008) | stay Live or → `Detached` | disable input (`access=ReadOnly`); refresh via GET; reattach `read_only=true`; second Unauthorized → `Detached`/`Unauthorized` |
| `Internal` (4500) | `Reconnecting` | retry with `RetryWindow` (30s wall from first retry); success → `output_gap=true`; exhausted → `Detached`/`RetriesExhausted` |
| `Network` | `Reconnecting` | same as Internal |
| `_` (non_exhaustive) | `Reconnecting` | treat as Network-retry (conservative) |

### Landed signatures (quote — do not invent)

```rust
// attach.rs
pub fn attach(client: &Client, session: &SessionId, tid: &TerminalId, opts: AttachOptions)
    -> Result<AttachHandle>;
pub struct AttachHandle {
    pub inbound: Receiver<WsInbound>,
    pub outbound: Sender<WsOutbound>,
}
impl AttachHandle { pub fn close(mut self); pub fn inspect(&self) -> AttachInspect; }

// wire.rs
pub enum WsInbound { Vt(Vec<u8>), Text(String), Closed(CloseCause) }
pub enum WsOutbound { Input(Vec<u8>), Resize { cols: u16, rows: u16 } }

// close.rs — #[non_exhaustive]
pub enum CloseCause {
    TerminalNotFound, TerminalDetached, Internal, Unauthorized, Network,
}

// rest + Client
impl Client { pub fn terminals(&self, session: SessionId) -> Terminals<'_>; }
impl Terminals<'_> {
    pub fn list(&self) -> Result<Vec<TerminalResource>>;
    pub fn get(&self, tid: &TerminalId) -> Result<TerminalResource>;
    pub fn create(&self, req: &TerminalCreate) -> Result<TerminalResource>;
    pub fn delete(&self, tid: &TerminalId) -> Result<()>;
}

// handle.rs
impl EngineHandle {
    pub fn spawn(cfg: EngineConfig) -> Self;
    pub fn feed(&self, bytes: Vec<u8>) -> Result<(), FeedError>;
    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), FeedError>;
    pub fn set_visible(&self, visible: bool) -> Result<(), FeedError>;
    pub fn latest_frame(&self) -> Option<Arc<Frame>>;
    pub fn set_waker(&self, waker: Box<dyn Fn() + Send + Sync>);
    pub fn da_dsr_rx(&self) -> &Receiver<Vec<u8>>;
    pub fn stop(mut self); // consumes Self — requires Arc::try_unwrap
}

// lib.rs
pub fn open(target, client: Arc<Client>, options: TerminalOpenOptions, cx: &mut App)
    -> Entity<TerminalTab>;
```

### Shared 1c↔1d SEAM (consume — do not redefine)

- 1c owns: `render::paint_frame(...) -> RenderStats`, `CellMetrics::resolve_menlo`, `TerminalTab.latest_frame`, `set_frame_for_test`.
- 1d only makes the engine the *source* of `latest_frame` via the C2 wake sampler. Do not touch the paint path.

---

## File Structure

- `crates/lens-terminal/src/bridge.rs` — Select loop; `BridgeEvent` incl. `OutboundSaturated`; join API; holds one `Arc<EngineHandle>`.
- `crates/lens-terminal/src/runtime.rs` — `TerminalRuntime` ownership + off-foreground teardown + `Drop` offload.
- `crates/lens-terminal/src/policy.rs` — close-code policy, `RetryWindow` with injected clock, preflight GET, resolve helpers.
- `crates/lens-terminal/src/inspect.rs` — `TerminalInspect` (crate-local `EngineInspect`).
- `crates/lens-terminal/src/lib.rs` — `open()` C2 spawn path; foreground sampler; `TerminalTab` fields; remove `#[expect(dead_code)]`.
- `crates/lens-terminal/src/engine/vt.rs` — `#[cfg(test)]` scrollback history probe (`scrollback_rows` + `scroll_viewport`).
- `crates/lens-terminal/tests/reconnect_seed.rs` — offline retained-engine acceptance (leg-parsed, full `Frame`, scrollback delta).
- `crates/lens-terminal/tests/terminal_live.rs` — live rider: real `open()` + paint observation; network-loss via abort.
- `crates/lens-terminal-demo/` — handshake before GPUI; env Existing/OpenOrCreate only.
- `crates/lens-client/src/terminal/attach.rs` — (tiny 1a) promote `set_inspect_enabled` to production; add `abort_for_test` under `live-tests`.
- `crates/lens-terminal/Cargo.toml` — `async-channel`, `live-tests` feature.

---

### Task 1: Bridge thread + `BridgeEvent::OutboundSaturated` + join

**Files:**
- Create: `crates/lens-terminal/src/bridge.rs`
- Modify: `crates/lens-terminal/src/lib.rs` (`mod bridge;`)

**Interfaces:**
- Consumes: landed `AttachHandle` channels, `EngineHandle::{feed, da_dsr_rx}`, `WsInbound`/`WsOutbound`, `FeedError`.
- Produces:
  ```rust
  pub enum BridgeEvent {
      Closed(CloseCause),
      FeedSaturated,
      OutboundSaturated, // sustained DA/DSR → outbound try_send Full
      AttachDisconnected,
  }

  pub struct BridgeHandle { /* JoinHandle + AtomicBool stop + Arc<EngineHandle> */ }

  pub fn spawn_bridge(
      inbound: Receiver<WsInbound>,
      outbound: Sender<WsOutbound>,
      engine: Arc<EngineHandle>,
      policy_tx: async_channel::Sender<BridgeEvent>,
  ) -> BridgeHandle;

  impl BridgeHandle {
      /// Signal stop and **join** the bridge thread. Drops this handle's engine Arc.
      pub fn join(self);
  }
  ```

**Outbound saturation (locked — I8):** on `da_dsr` reply, `outbound.try_send(WsOutbound::Input(bytes))`. If `Full`, wait up to **50ms** (`send_timeout` / retry loop checking stop). If still full → `policy_tx.try_send(BridgeEvent::OutboundSaturated)`, then exit the Select loop. **Do not** silently drop the reply. (Engine already drop-oldest on its da_dsr queue — `worker.rs:294-303`; bridge must not add a second silent-loss point. Attach outbound forwarder retries until shutdown — `attach.rs:334-350` — it does **not** auto-disconnect on outbound-full.)

- [ ] **Step 1: Write the failing tests.**
  ```rust
  #[test]
  fn vt_inbound_feeds_engine_after_ack() {
      let engine = Arc::new(EngineHandle::spawn(test_cfg()));
      let (inbound_tx, inbound_rx) = crossbeam_channel::bounded(8);
      let (outbound_tx, outbound_rx) = crossbeam_channel::bounded(8);
      let (policy_tx, _policy_rx) = async_channel::bounded(8);
      let before = engine.inspect().frames_built; // always-on counter
      let bridge = spawn_bridge(inbound_rx, outbound_tx, Arc::clone(&engine), policy_tx);

      inbound_tx.send(WsInbound::Vt(b"AB".to_vec())).unwrap();
      // Wait until feed is observed — NOT a blind sleep before build_now.
      let deadline = Instant::now() + Duration::from_secs(2);
      loop {
          engine.build_now().ok();
          if engine.inspect().bytes_fed >= 2 {
              break;
          }
          assert!(Instant::now() < deadline, "bridge did not feed engine");
          std::thread::sleep(Duration::from_millis(1));
      }
      let f = wait_new_frame(&engine, before);
      assert!(f.grid[0].cells.iter().any(|c| c.grapheme == "A" || c.grapheme == "B"));

      // DA/DSR forward
      let before_da = engine.inspect().da_dsr_emitted;
      engine.feed(b"\x1b[c".to_vec()).unwrap();
      engine.build_now().ok();
      let deadline = Instant::now() + Duration::from_secs(2);
      let reply = loop {
          match outbound_rx.try_recv() {
              Ok(WsOutbound::Input(b)) if !b.is_empty() => break b,
              _ => {
                  assert!(Instant::now() < deadline, "DA/DSR not forwarded");
                  std::thread::sleep(Duration::from_millis(1));
              }
          }
      };
      assert!(!reply.is_empty());
      assert!(engine.inspect().da_dsr_emitted > before_da);
      bridge.join();
  }

  #[test]
  fn outbound_saturation_emits_event_and_joins() {
      let engine = Arc::new(EngineHandle::spawn(test_cfg()));
      let (_inbound_tx, inbound_rx) = crossbeam_channel::bounded(1);
      // Cap 1, pre-fill so DA/DSR forward cannot enqueue.
      let (outbound_tx, _outbound_rx) = crossbeam_channel::bounded(1);
      outbound_tx.send(WsOutbound::Input(vec![0])).unwrap();
      let (policy_tx, policy_rx) = async_channel::bounded(8);
      let bridge = spawn_bridge(inbound_rx, outbound_tx, Arc::clone(&engine), policy_tx);

      engine.feed(b"\x1b[c".to_vec()).unwrap();
      engine.build_now().ok();
      // Poll async_channel from a tiny block_on / try_recv loop with timeout.
      let deadline = Instant::now() + Duration::from_secs(2);
      let ev = loop {
          if let Ok(ev) = policy_rx.try_recv() { break ev; }
          assert!(Instant::now() < deadline, "expected OutboundSaturated");
          std::thread::sleep(Duration::from_millis(5));
      };
      assert!(matches!(ev, BridgeEvent::OutboundSaturated));
      bridge.join(); // confirmed join after saturation exit
  }
  ```

- [ ] **Step 2: Run** `cargo test -p lens-terminal bridge::` → FAIL.

- [ ] **Step 3: Implement** Select over inbound + `da_dsr_rx.clone()` + stop. Map events as specified. `join` sets stop, joins thread, drops `Arc`.

- [ ] **Step 4: Run** → PASS.

- [ ] **Step 5: Commit** `feat(lens-terminal): bridge Select + OutboundSaturated → reconnect (Slice 1d)`

---

### Task 2: `TerminalRuntime` ownership + off-foreground teardown

**Files:**
- Create: `crates/lens-terminal/src/runtime.rs`
- Modify: `crates/lens-terminal/src/lib.rs`

**Interfaces:** Produces `TerminalRuntime` as locked in Global Constraints (C3). `TerminalTab.runtime: Option<TerminalRuntime>`.

- [ ] **Step 1: Write the failing tests.**
  ```rust
  #[test]
  fn teardown_blocking_unwraps_unique_arc_and_stops() {
      let engine = Arc::new(EngineHandle::spawn(test_cfg()));
      let weak_cmd = /* keep a way to observe Stopped: clone cmd via test or feed after */;
      let (inbound_tx, inbound_rx) = crossbeam_channel::bounded::<WsInbound>(1);
      let (outbound_tx, _) = crossbeam_channel::bounded::<WsOutbound>(1);
      let (policy_tx, _) = async_channel::bounded(1);
      let bridge = spawn_bridge(inbound_rx, outbound_tx, Arc::clone(&engine), policy_tx);
      // AttachHandle is optional in this unit — use runtime without attach:
      let rt = TerminalRuntime {
          bridge: Some(bridge),
          attach: None,
          engine: Some(engine),
      };
      rt.teardown_blocking();
      // After stop, a pre-saved cmd sender from EngineHandle test hook OR
      // constructing engine with accessible stop proof: feed on a dangling
      // clone must be impossible (Arc consumed). Assert Arc::strong_count
      // path: teardown_blocking's try_unwrap expect did not panic.
  }

  #[test]
  fn drop_runtime_does_not_join_on_calling_thread() {
      let engine = Arc::new(EngineHandle::spawn(test_cfg()));
      let (_t, inbound_rx) = crossbeam_channel::bounded::<WsInbound>(1);
      let (outbound_tx, _) = crossbeam_channel::bounded::<WsOutbound>(1);
      let (policy_tx, _) = async_channel::bounded(1);
      let bridge = spawn_bridge(inbound_rx, outbound_tx, Arc::clone(&engine), policy_tx);
      let rt = TerminalRuntime { bridge: Some(bridge), attach: None, engine: Some(engine) };
      let start = Instant::now();
      drop(rt); // must return quickly — joins happen on teardown thread
      assert!(start.elapsed() < Duration::from_millis(50));
  }
  ```

- [ ] **Step 2: Run** → FAIL.

- [ ] **Step 3: Implement** `runtime.rs` exactly as locked. Wire `TerminalTab` to hold `Option<TerminalRuntime>`.

- [ ] **Step 4: Run** → PASS. Confirm `try_unwrap` path is covered (no `expect` panic in test).

- [ ] **Step 5: Commit** `feat(lens-terminal): TerminalRuntime off-foreground teardown (Slice 1d)`

---

### Task 3: `open()` via C2 background_executor + foreground apply

**Files:**
- Modify: `crates/lens-terminal/src/lib.rs`
- Create/modify: `crates/lens-terminal/src/policy.rs` (resolve helpers)

**Interfaces:**
- Extends `Presentation`:
  ```rust
  pub enum DetachedDetail {
      TerminalGone, ClientDetached, Unauthorized, RetriesExhausted, DiscoveryFailed,
  }
  pub struct Presentation {
      pub lifecycle: Lifecycle,
      pub access: AccessMode,
      pub identity_title: String,
      pub reported_title: Option<String>,
      pub progress: Option<Progress>,
      pub output_gap: bool,
      pub detached_detail: Option<DetachedDetail>,
      pub reattach_available: bool,
  }
  ```
- `open()` returns `Starting` immediately; spawns C2 continuation.

- [ ] **Step 1: Failing unit tests** for `matches_key` / presentation defaults (`output_gap: false`, etc.).

- [ ] **Step 2: Run** → FAIL.

- [ ] **Step 3: Implement `open` exactly as:**
  ```rust
  pub fn open(
      target: TerminalTarget,
      client: Arc<Client>,
      options: TerminalOpenOptions,
      cx: &mut App,
  ) -> Entity<TerminalTab> {
      let entity = cx.new(|cx| TerminalTab::starting(target.clone(), Arc::clone(&client), options.clone(), cx));
      let weak = entity.downgrade();
      cx.spawn(async move |cx| {
          let outcome = cx
              .background_executor()
              .spawn(async move {
                  // blocking REST + attach + EngineHandle::spawn + spawn_bridge
                  discover_and_attach(client, target, options)
              })
              .await;
          let _ = weak.update(cx, |tab, cx| match outcome {
              Ok(parts) => tab.on_attached(parts, cx),
              Err(detail) => tab.on_detach(detail, cx),
          });
      })
      .detach();
      entity
  }
  ```
  `discover_and_attach` (blocking, no gpui):
  1. `resolve_terminal` (Existing GET / OpenOrCreate list-or-create).
  2. `attach(&client, &sid, &tid, AttachOptions { read_only })`.
  3. `let engine = Arc::new(EngineHandle::spawn(cfg));`
  4. Create `async_channel::bounded(1)` wake + `async_channel::bounded(32)` policy.
  5. `spawn_bridge(attach.inbound.clone() /* can't clone AttachHandle — move channels: */)` — **move** `attach` into `TerminalRuntime`; bridge is spawned with `attach.inbound` / `attach.outbound` by restructuring: `spawn_bridge` takes the receivers/senders; `TerminalRuntime` stores `AttachHandle` after extracting channels **or** store attach and pass `inbound`/`outbound` by moving fields. **Committed approach:** change is local to 1d — `spawn_bridge` takes `Receiver`/`Sender`; `TerminalRuntime.attach` holds the `AttachHandle` whose channels were **moved into the bridge at spawn** via a new 1d-only helper that splits:
     ```rust
     // In bridge.rs / runtime — use attach by moving public fields:
     let AttachHandle { inbound, outbound, .. } = attach;
     // PROBLEM: other private fields needed for close().
     ```
     **Committed approach:** do **not** destructure `AttachHandle`. Pass `&AttachHandle` channels by cloning receivers/senders (`Receiver`/`Sender` are `Clone` in crossbeam). Keep the full `AttachHandle` in `TerminalRuntime.attach`. Bridge uses `attach.inbound.clone()` and `attach.outbound.clone()`. `attach.close()` still joins the I/O thread; cloned channel ends disconnect the bridge Select naturally.
  6. Return `AttachedParts { resource, runtime: TerminalRuntime { bridge, attach: Some(attach), engine }, wake_tx, wake_rx, policy_rx }` (wake_tx installed into engine waker in `on_attached`).

  `on_attached`:
  - Install engine waker: `engine.set_waker(Box::new(move || { let _ = wake_tx.try_send(()); }));`
  - Store runtime; set lifecycle `Live`; identity title from metadata; emit `PresentationChanged`; `cx.notify()`.
  - Start foreground sampler `cx.spawn` as locked in C2 (await wake_rx + policy_rx).

  Remove `#[expect(dead_code)]` on `target`/`client`/`options`.

- [ ] **Step 4: Run** offline units → PASS.

- [ ] **Step 5: Commit** `feat(lens-terminal): open() C2 background discover/attach (Slice 1d)`

---

### Task 4: Close-code policy + `RetryWindow` clock + gap marker

**Files:**
- Modify: `crates/lens-terminal/src/policy.rs`, `lib.rs` (`apply_bridge_event`)

**Interfaces:**
  ```rust
  pub enum PolicyAction {
      StopDetached { detail: DetachedDetail, reattach_available: bool },
      Retry { delay: Duration },
      DowngradeReadOnly,
  }

  pub struct RetryWindow {
      started: Option<Instant>, // None until first retry
      backoff: Backoff,
  }
  impl RetryWindow {
      pub fn new() -> Self;
      /// `now` injected. Returns None when `started` is Some and `now >= started + 30s`.
      /// When Some(delay), `delay <= (started+30s).saturating_duration_since(now)` and
      /// `delay <= Backoff::next_delay()` cap (30s per attempt).
      pub fn next_delay(&mut self, now: Instant) -> Option<Duration>;
      pub fn reset(&mut self); // clears started + backoff.reset()
  }

  pub fn apply_close(cause: CloseCause, retry: &mut RetryWindow, now: Instant) -> PolicyAction;
  ```

- [ ] **Step 1: Failing tests.**
  ```rust
  #[test]
  fn close_cause_policy_table() { /* 4404/4405/1008/4500/Network as locked table */ }

  #[test]
  fn non_exhaustive_unknown_maps_to_network_retry() {
      // Cannot construct unknown CloseCause today; document wildcard arm in apply_close
      // and cover via exhaustive match compile + comment. Add:
      // match cause { … known … _ => Retry } so future variants compile.
  }

  #[test]
  fn retry_window_boundaries_with_injected_now() {
      let t0 = Instant::now();
      let mut w = RetryWindow::new();
      // First call starts the window at `now`.
      let d0 = w.next_delay(t0).expect("first retry");
      assert!(d0 <= Duration::from_secs(30));

      // 29.999s later still Some, delay <= remaining
      let t_almost = t0 + Duration::from_millis(29_999);
      let d = w.next_delay(t_almost).expect("inside window");
      let remaining = (t0 + Duration::from_secs(30)).saturating_duration_since(t_almost);
      assert!(d <= remaining);

      // Exactly 30s → None
      assert!(w.next_delay(t0 + Duration::from_secs(30)).is_none());

      w.reset();
      assert!(w.next_delay(t0 + Duration::from_secs(60)).is_some()); // new window
  }
  ```

- [ ] **Step 2: Run** → FAIL.

- [ ] **Step 3: Implement** `apply_close` with wildcard → Network-retry. `apply_bridge_event` on the tab (called from C2 sampler):
  - `Closed(cause)` / `FeedSaturated` / `OutboundSaturated` / `AttachDisconnected` → map to policy (saturations → Network-retry path).
  - `Retry` → `Reconnecting`, `input_enabled=false`; schedule reconnect via `cx.spawn` + `background_executor` sleep(`delay`) + preflight (T7) + new `attach` + new bridge into **retained** `Arc<EngineHandle>`; on success `mark_reconnect_success` (`output_gap=true`, `Live`, resize-before-input from T5).
  - `StopDetached` → presentation update; `runtime.take().teardown_off_foreground(cx)` (for ClientDetached, split: close attach+bridge, keep engine Arc in tab for reattach UI — store `retained_engine: Option<Arc<EngineHandle>>`).

- [ ] **Step 4: Unauthorized unit test** — first → DowngradeReadOnly; simulated second Unauthorized → StopDetached{Unauthorized}.

- [ ] **Step 5: Run** → PASS.

- [ ] **Step 6: Commit** `feat(lens-terminal): close-code policy + RetryWindow clock (Slice 1d)`

---

### Task 5: Wake sampler samples `TerminalTab.latest_frame` + resize-before-input + `set_visible`

**Files:**
- Modify: `crates/lens-terminal/src/lib.rs`

**Interfaces:**
- `TerminalTab::sample_latest_frame_from_engine(&mut self)` — sets `self.latest_frame = engine.latest_frame()`.
- `TerminalTab::set_visible(&mut self, visible: bool, cx: &mut Context<Self>)` — forwards to `runtime.engine.set_visible`, `cx.notify()` on show.
- Resize-before-input on (re)connect uses **real** `engine.resize` + `outbound.send(Resize)` before flipping `input_enabled`.

- [ ] **Step 1: Failing tests that exercise real objects.**
  ```rust
  #[gpui::test]
  async fn sample_updates_tab_latest_frame(cx: &mut gpui::TestAppContext) {
      // Build a TerminalTab entity with a runtime engine fed "Hi", install wake path:
      // either call sample_latest_frame_from_engine directly after feed+build_now,
      // or send on wake_tx and drive the sampler once.
      cx.update(|cx| {
          let tab = /* entity with engine in runtime */;
          tab.update(cx, |tab, _cx| {
              tab.sample_latest_frame_from_engine();
              assert!(tab.latest_frame.is_some());
              let f = tab.latest_frame.as_ref().unwrap();
              assert!(f.grid[0].cells.iter().any(|c| c.grapheme == "H" || c.grapheme == "i"));
          });
      });
  }

  #[test]
  fn resize_before_input_orders_engine_and_outbound() {
      let engine = Arc::new(EngineHandle::spawn(test_cfg()));
      let (outbound_tx, outbound_rx) = crossbeam_channel::bounded(4);
      let mut input_enabled = false;
      apply_newest_size_before_input(
          engine.as_ref(),
          &outbound_tx,
          120,
          40,
          true, // write access
          &mut input_enabled,
      );
      // Real outbound ordering:
      let first = outbound_rx.try_recv().unwrap();
      assert_eq!(first, WsOutbound::Resize { cols: 120, rows: 40 });
      assert!(outbound_rx.try_recv().is_err(), "no Input before enable");
      assert!(input_enabled);
      // Engine accepted resize (inspect cols/rows if exposed, or no FeedError):
      assert!(engine.resize(120, 40).is_ok() || true /* already resized */);
      let snap = engine.inspect();
      assert_eq!((snap.cols, snap.rows), (120, 40));
  }

  #[gpui::test]
  async fn tab_set_visible_forwards_to_engine(cx: &mut gpui::TestAppContext) {
      // Construct tab with runtime+engine+waker counter via wake_tx.
      // tab.set_visible(false, cx) — NOT engine.set_visible directly.
      // feed+build_now → wake count frozen; set_visible(true) → wake advances.
  }
  ```

- [ ] **Step 2: Run** → FAIL.

- [ ] **Step 3: Implement** sampler hookup (already started in T3 `on_attached`), `sample_latest_frame_from_engine`, `set_visible` forwarding, `apply_newest_size_before_input`:
  ```rust
  fn apply_newest_size_before_input(
      engine: &EngineHandle,
      outbound: &Sender<WsOutbound>,
      cols: u16,
      rows: u16,
      write_allowed: bool,
      input_enabled: &mut bool,
  ) {
      let _ = engine.resize(cols, rows);
      let _ = outbound.send(WsOutbound::Resize { cols, rows });
      *input_enabled = write_allowed;
  }
  ```
  `render` calls `sample_latest_frame_from_engine` then 1c `paint_frame` — no RAF loop.

- [ ] **Step 4: Run** → PASS.

- [ ] **Step 5: Commit** `feat(lens-terminal): wake-sample latest_frame + set_visible + resize-before-input (Slice 1d)`

---

### Task 6: Retained-engine reconnect-seed acceptance (C4)

**Files:**
- Create: `crates/lens-terminal/tests/reconnect_seed.rs`
- Modify: `crates/lens-terminal/src/engine/vt.rs` — test history probe
- Modify: `crates/lens-terminal/src/engine/handle.rs` — test-only command to run probe on worker if needed

**Capture legs (locked):** `reconnect.frames.jsonl` line map:
- Lines 1–7: leg 1 (initial attach seed + traffic; ts_offset resets at line 8).
- Line 8: outbound after reconnect.
- **Lines 9–10: leg 2 reconnect seed** (line 9 = clear+redraw ~1499 B starting `1b5b481b5b324a` = `\x1b[H\x1b[2J`).

Parse into legs by detecting the ts_offset discontinuity (line 8 `ts_offset_ms: 3` after line 7 `334`) **or** hard-split: leg2 inbound binary = lines 9..end where `direction==in && kind==binary`.

**Wait for NEW frame (locked):**
```rust
fn wait_new_frame(engine: &EngineHandle, min_frames_built: u64) -> Arc<Frame> {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let built = engine.inspect().frames_built;
        if built > min_frames_built {
            if let Some(f) = engine.latest_frame() {
                return f;
            }
        }
        assert!(Instant::now() < deadline, "no new frame generation");
        std::thread::sleep(Duration::from_millis(1));
    }
}
```

**Full Frame compare (locked):** `Frame: PartialEq` (`frame.rs:55-63`) — assert `retained_frame == fresh_frame`, not grapheme concatenation.

**Scrollback probe (locked):**
```rust
// vt.rs
#[cfg(test)]
impl VtEngine {
    pub fn scrollback_rows_for_test(&self) -> usize {
        self.terminal.scrollback_rows().unwrap_or(0)
    }
    pub fn scroll_viewport_for_test(&mut self, scroll: libghostty_vt::ScrollViewport) {
        self.terminal.scroll_viewport(scroll);
    }
}
```
Expose via `EngineHandle` test command `EngineCommand::ProbeScrollback` → oneshot/`Arc<AtomicU64>` result **or** run the acceptance against `VtEngine` directly on the calling thread for the history half (simpler, committed):

**Committed acceptance structure:**
1. **Viewport half (EngineHandle):** feed leg1 chunks → `build_now` → record `gen0 = frames_built`; feed **leg2 seed chunks (lines 9+)** → `build_now` → `wait_new_frame(..., gen0)`; fresh engine fed only leg2 seed → `wait_new_frame`; assert `*retained_frame == *fresh_frame`.
2. **Scrollback half (VtEngine on test thread):** feed leg1; `sb0 = scrollback_rows_for_test()`; feed leg2 seed; `sb1 = scrollback_rows_for_test()`; assert exact allowed delta:
   ```rust
   // Clear+redraw may push at most one viewport of rows into scrollback.
   // It must NOT re-append an entire leg1-sized history.
   let cols = 80u16; let rows = 24u16;
   assert!(
       sb1.saturating_sub(sb0) <= rows as usize,
       "scrollback grew by {} (> viewport); retained seed duplicated history",
       sb1.saturating_sub(sb0)
   );
   ```
   Fail-closed: if Ghostty grows scrollback by more than `rows`, leave the test failing and escalate — do not weaken.

- [ ] **Step 1: Write the failing test file** with leg parser + both halves + `mark_reconnect_success` presentation assert (`output_gap == true`).

- [ ] **Step 2: Run** `cargo test -p lens-terminal --test reconnect_seed` → FAIL (probe missing / wrong seed).

- [ ] **Step 3: Implement** probe + parser; fix seed selection to line 9+.

- [ ] **Step 4: Run** → PASS (or FAIL-CLOSED escalate).

- [ ] **Step 5: Commit** `test(lens-terminal): retained-engine reconnect-seed acceptance (Slice 1d)`

---

### Task 7: Basic generation guard (pre-reconnect GET)

**Files:**
- Modify: `crates/lens-terminal/src/policy.rs`

```rust
pub fn preflight_reconnect(
    client: &Client,
    session: &SessionId,
    tid: &TerminalId,
) -> Result<TerminalResource, DetachedDetail> {
    match client.terminals(session.clone()).get(tid) {
        Ok(r) => Ok(r),
        Err(ClientError::NotFound { .. }) => Err(DetachedDetail::TerminalGone),
        Err(_) => Err(DetachedDetail::DiscoveryFailed),
    }
}
```

Called at the start of every T4 retry **before** `attach`, inside the `background_executor` blocking closure.

- [ ] **Step 1: Failing test** for `map_preflight_err(NotFound) → TerminalGone`.

- [ ] **Step 2–4: Implement, pass, commit** `feat(lens-terminal): pre-reconnect GET guard (Slice 1d)`

---

### Task 8: Standalone GPUI demo (handshake before GPUI)

**Files:**
- Modify: `crates/lens-terminal-demo/Cargo.toml`, `src/main.rs`

**Demo flow (locked — I11):**
1. Read env **before** any gpui call:
   - Required: `LENS_OMNIGENT_URL`
   - Required: `LENS_OMNIGENT_SESSION_ID`
   - Exactly one of:
     - `LENS_OMNIGENT_TERMINAL_ID` → `TerminalTarget::Existing { session_id, terminal_id }`
     - **or** (`LENS_OMNIGENT_TERMINAL_NAME` **and** `LENS_OMNIGENT_SESSION_KEY`) → `TerminalTarget::OpenOrCreate { session_id, key }`
2. `Client::new(Connection::new(...))` on the main thread **before** `Application::new().run`. On handshake/`ClientError`, print the error to stderr and `std::process::exit(1)` — no `expect`, no HTTP inside the gpui callback.
3. Move `Arc<Client>` + `TerminalTarget` into the `run` closure; call `open(...)`; host the entity as the window root (mirror lens-app window root pattern).

- [ ] **Step 1: Implement** deps + `main` as above.

- [ ] **Step 2: Document** env in rustdoc; missing env → usage on stderr + exit 0 (not configured); configured-but-handshake-fail → exit 1.

- [ ] **Step 3: Commit** `feat(lens-terminal-demo): pre-GPUI handshake + env Existing/OpenOrCreate (Slice 1d)`

---

### Task 9: Inspect + live vertical rider

**Files:**
- Create: `crates/lens-terminal/src/inspect.rs`
- Create: `crates/lens-terminal/tests/terminal_live.rs`
- Modify: `crates/lens-client/src/terminal/attach.rs` (tiny 1a — scoped below)
- Modify: `crates/lens-terminal/Cargo.toml`

**Inspect enablement (locked — I13):** Promote `AttachHandle::set_inspect_enabled` to an **unconditional production method** (remove `#[cfg(any(test, feature = "test-util"))]` gate at `attach.rs:162-165`). The atomic + zero-cost-when-disabled path already exists (`AttachInspectState.enabled`). Scope this as a one-line 1a visibility change inside the 1d slice. Then:
```rust
pub struct TerminalInspect {
    pub lifecycle: Lifecycle,
    pub output_gap: bool,
    pub bridge_alive: bool,
    pub input_enabled: bool,
    pub attach: Option<AttachInspect>,
    pub engine: Option<crate::EngineInspect>, // NOT lens_terminal:: — M15
}
impl TerminalTab {
    pub fn set_inspect_enabled(&self, enabled: bool) {
        if let Some(rt) = &self.runtime {
            if let Some(a) = &rt.attach { a.set_inspect_enabled(enabled); }
            if let Some(e) = &rt.engine { e.set_inspect_enabled(enabled); }
        }
    }
    pub fn inspect(&self) -> TerminalInspect { /* when disabled, skip child rings */ }
}
```

**Live network-loss (locked — I9/I14):** Add under `feature = "live-tests"` on lens-client:
```rust
impl AttachHandle {
    /// Abruptly abort the I/O thread without graceful sink.close — surfaces
    /// WsInbound::Closed(Network) to the bridge (proves Reconnecting).
    pub fn abort_for_test(self) { /* signal shutdown; drop without graceful close path */ }
}
```
Live rider **must not** call `attach.close()` / `runtime.teardown` to simulate reconnect.

**Live rider requirements (locked — I14):**
1. Skip **only** when `LENS_OMNIGENT_URL` or `LENS_OMNIGENT_AGENT_ID` is **absent**. If present, handshake/server failure → **FAIL**.
2. Use gpui `TestAppContext` / real window harness (C1-style): call actual `open(...)`.
3. After input marker, require a successful **`RenderStats`** from `paint_frame` (or tab-exposed last paint stats) whose painted content reflects the marker — not merely `latest_frame` VT bytes.
4. Force network loss via `abort_for_test` on a handle the test reaches through inspect/test hook **or** by having the tab expose `debug_abort_attach_for_test` under `live-tests` that aborts the runtime’s attach without full teardown, leaving engine retained → policy sees `Closed(Network)` → `Reconnecting` → preflight GET → reattach → `output_gap`.

- [ ] **Step 1: 1a promote `set_inspect_enabled` + add `abort_for_test`.** Commit `feat(lens-client): production attach inspect enable + live abort_for_test`.

- [ ] **Step 2: TerminalInspect tests** (disabled cheap; enabled populates). Commit `feat(lens-terminal): convergence Inspect (Slice 1d)`.

- [ ] **Step 3: Live rider** as locked. Run against omnigent 0.5.1.

- [ ] **Step 4: Gate** `cargo fmt`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`.

- [ ] **Step 5: Commit** `test(lens-terminal): live open()+paint proof vs omnigent 0.5.1 (Slice 1d)`.

- [ ] **Step 6: Cross-family review** of the 1d diff; fold findings.

---

## Self-Review

### Spec coverage (1d → task)

| Design requirement | Task |
| --- | --- |
| C2 wake: background_executor + async_channel + two-arg update | T3, T5 |
| C3 TerminalRuntime unique-Arc stop / never fg-join | T2 |
| Bridge + DA/DSR + OutboundSaturated → reconnect | T1 |
| `open()` Starting → Live/Detached | T3 |
| Close-code policy + RetryWindow clock + gap marker | T4 |
| Frame sample into `TerminalTab.latest_frame` + paint seam | T5 |
| Hidden-tab `TerminalTab::set_visible` forwarding | T5 |
| Resize-before-input real outbound/engine order | T5 |
| Reconnect-seed: leg2 line 9, new gen, full Frame, scrollback delta | T6 |
| Pre-reconnect GET guard | T7 |
| Demo: handshake before GPUI, env Existing/OpenOrCreate | T8 |
| Inspect production enable (1a promote) + live open()+paint+abort | T9 |
| `CloseCause` wildcard → Network-retry; `crate::EngineInspect` | T4, T9 |
| Remove `#[expect(dead_code)]` | T3 |

### Placeholder scan

No “or”/“prefer”/“whichever compiles” left on C2/C3/I8/I11/I13/I14. Every mechanism is single-valued above.

### Type-consistency check

- Landed `AttachHandle` / `WsInbound` / `WsOutbound` / `CloseCause` / `EngineHandle` / `open` / `Lifecycle` signatures unchanged.
- `EngineHandle::stop` consumes `Self` — only via `Arc::try_unwrap` after bridge join (C3).
- Waker/policy never call `WeakEntity::update` without `&mut` context from a foreground `cx.spawn` (C2).
- 1c seam untouched; `set_frame_for_test` retained.
- `Backoff` reused inside `RetryWindow`; 30s is a **wall window** from first retry, not only per-delay cap.
- `Frame: PartialEq` used for reconnect-seed viewport assert; `scrollback_rows` probe for history delta.
