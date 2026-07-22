# Terminal Slice 1a — `lens-client` transport Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a typed omnigent terminal transport to `lens-client` — REST list/get/create/delete + an auth/access-modeled WebSocket attach — with **zero** `serde_json::Value` or generic-WebSocket leakage to callers.

**Architecture:** A new `lens_client::terminal` module. REST stays on the existing blocking-`reqwest` subservice pattern (`client.terminals(session_id)`). The WS attach runs on **one dedicated OS thread hosting a contained `tokio` current-thread runtime + `tokio-tungstenite`** (the Spike-B-proven shape), bridged to synchronous consumers by **bounded `crossbeam` channels**. tokio is fully encapsulated in `terminal/attach.rs` — it never appears in a public signature or another module. The layer **classifies** close causes and frames; it holds **no** Ghostty, presentation, scrollback, or lifecycle **policy** (that is Slice 1d).

**Tech Stack:** Rust 2024, `reqwest` (blocking, existing), `tokio` (current-thread, new — contained), `tokio-tungstenite` (rustls, new), `crossbeam-channel` (existing), `serde`/`serde_json`, `thiserror`.

## Global Constraints

- Rust edition 2024, `rust-version = 1.91`. Copy from `[workspace.package]`.
- **MANDATORY** Never block the gpui foreground thread; all connect/read/write off-thread. (No gpui here at all — `lens-client` has no gpui dep.)
- **MANDATORY** UI never panics: every failure is a typed `Result`/modeled value, never `unwrap`/`panic` on a transport path.
- **MANDATORY** Typed end-to-end: no `serde_json::Value` in any public type; no raw `tungstenite`/`tokio` type in any public signature.
- **MANDATORY** `cargo clippy --workspace --all-targets -- -D warnings` clean + `rustfmt`. `unsafe` forbidden (`unsafe_code = "deny"`).
- **MANDATORY** Benchmark-or-it's-not-done on perf paths: frame classification, control codec, bounded-queue throughput get Criterion benches (feature `bench`).
- **MANDATORY** Introspectable: the attachment exposes a gated, typed, serializable `Inspect`-style snapshot (transport/queue/reconnect state) with a fixed-capacity diagnostic ring; **zero cost when disabled**.
- Ground truth: `vendor/omnigent-0.5.1/openapi.json`; omnigent source rev `08285468`; the design `docs/specs/2026-07-16-terminal-workstream-design.md` ("Pinned omnigent 0.5.1 facts", "Module ownership → lens-client"); Spike B `docs/spikes/2026-07-15-pty-attach-contract.md` + captures under `docs/spikes/captures/2026-07-15-pty-attach/`.
- **The client does NOT send `transport=`.** Attach URL carries only `read_only`. (Corrects the dead 2026-07-14 spec.)
- **Close-code *classification* only** (typed causes). Stop/retry/downgrade/reattach **policy is Slice 1d** — do not implement it here.
- Reference reader for the existing subservice + error idiom: `crates/lens-client/src/sessions.rs`, `crates/lens-client/src/error.rs`, `crates/lens-client/src/http.rs` (`decode_json`), `crates/lens-client/src/reconnect.rs` (`Reopen`).

---

## File Structure

- `crates/lens-client/src/terminal/mod.rs` — module root; `pub use` the public surface; `Client::terminals(&self, SessionId) -> Terminals`.
- `crates/lens-client/src/terminal/rest.rs` — `Terminals` subservice: typed `list`/`get`/`create`/`delete`; typed `TerminalResource`, `TerminalCreate`, `TerminalMetadata`.
- `crates/lens-client/src/terminal/wire.rs` — typed WS frame values: `WsInbound` (Vt bytes / Text / Closed{cause}) and `WsOutbound` (Input bytes / Resize{cols,rows}); the Message↔typed **codec** (pure, tested, benched).
- `crates/lens-client/src/terminal/close.rs` — `CloseCause` classification enum + `classify_close(code) -> CloseCause`.
- `crates/lens-client/src/terminal/attach.rs` — `attach(...) -> AttachHandle`; the contained tokio runtime + `tokio-tungstenite` I/O thread; bounded in/out queues; saturation→disconnect; the `Backoff` re-attach helper; `AttachInspect` snapshot + diagnostic ring.
- `crates/lens-client/src/lib.rs` — add `pub mod terminal;` + top-level `pub use terminal::{...}`.
- `crates/lens-client/Cargo.toml` — add `tokio`, `tokio-tungstenite`, `futures-util`; a `[[bench]] name = "terminal_transport"`.
- `crates/lens-client/benches/terminal_transport.rs` — Criterion benches (feature `bench`).
- Tests: unit tests inline per module; the live end-to-end rider gated behind the existing `live-tests` feature in `crates/lens-client/tests/terminal_live.rs`.

---

### Task 1: Cargo deps + module skeleton

**Files:**
- Modify: `crates/lens-client/Cargo.toml`
- Create: `crates/lens-client/src/terminal/mod.rs`
- Modify: `crates/lens-client/src/lib.rs` (add `pub mod terminal;`)

**Interfaces:**
- Produces: `lens_client::terminal` module exists and compiles; `Client::terminals(&self, session: SessionId) -> terminal::Terminals<'_>` (stub returning a `Terminals` holding `&Client` + `SessionId`).

- [ ] **Step 1: Add deps.** In `Cargo.toml` `[dependencies]` add:
  ```toml
  tokio = { version = "1", default-features = false, features = ["rt", "net", "io-util", "sync", "time", "macros"] }
  tokio-tungstenite = { version = "0.26", features = ["rustls-tls-webpki-roots"] }
  futures-util = { version = "0.3", default-features = false, features = ["sink", "std"] }
  ```
  (Versions match the Spike-B `spikes/terminal-attach/Cargo.toml`. `rt` — not `rt-multi-thread`: this is a **current-thread** runtime.)

- [ ] **Step 2: Module skeleton.** `terminal/mod.rs`:
  ```rust
  //! Typed omnigent terminal transport: REST CRUD + WS attach. No serde_json::Value
  //! or raw WS types escape this module. Close causes are *classified*; lifecycle
  //! *policy* is lens-terminal's (Slice 1d).
  mod attach;
  mod close;
  mod rest;
  mod wire;

  pub use attach::{AttachHandle, AttachOptions, AttachInspect, Backoff};
  pub use close::CloseCause;
  pub use rest::{TerminalCreate, TerminalMetadata, TerminalResource, Terminals};
  pub use wire::{WsInbound, WsOutbound};
  ```
  Add `pub mod terminal;` to `lib.rs` and re-export the same names at crate root (mirror the existing `pub use sessions::{...}` style).

- [ ] **Step 3: Build.** Run: `cargo build -p lens-client`. Expected: compiles (empty modules with the stubs; add `#![allow(unused)]`-free stubs — declare the types as you create each file in later tasks; for this task, create `rest.rs`/`wire.rs`/`close.rs`/`attach.rs` with only the types named in the `pub use` as minimal placeholders so `mod.rs` resolves).

- [ ] **Step 4: Commit.**
  ```bash
  git add crates/lens-client/Cargo.toml crates/lens-client/src/lib.rs crates/lens-client/src/terminal/
  git commit -m "feat(lens-client): terminal transport module skeleton + deps (Slice 1a)"
  ```

---

### Task 2: REST CRUD (`Terminals` subservice)

**Files:**
- Modify: `crates/lens-client/src/terminal/rest.rs`
- Modify: `crates/lens-client/src/terminal/mod.rs` (`Client::terminals`)

**Interfaces:**
- Consumes: `Client` (`crates/lens-client/src/client.rs`), `SessionId`/`TerminalId` (`ids.rs`), `decode_json` (`http.rs`), `Result`/`ClientError` (`error.rs`).
- Produces:
  - `struct Terminals<'a> { client: &'a Client, session: SessionId }`
  - `fn list(&self) -> Result<Vec<TerminalResource>>`
  - `fn get(&self, tid: &TerminalId) -> Result<TerminalResource>`
  - `fn create(&self, req: &TerminalCreate) -> Result<TerminalResource>`
  - `fn delete(&self, tid: &TerminalId) -> Result<()>`
  - `struct TerminalCreate { pub terminal: String, pub session_key: String }` (Serialize)
  - `struct TerminalResource { pub id: TerminalId, pub session_id: SessionId, pub name: Option<String>, pub metadata: TerminalMetadata, .. }` (Deserialize)
  - `struct TerminalMetadata { pub terminal_name: Option<String>, pub session_key: Option<String>, pub running: Option<bool>, pub terminal_transport: Option<String> }`

**Grounding:** Routes `/v1/sessions/{sid}/resources/terminals[/{tid}]` (openapi.json). Create body shape `{"terminal": <name>, "session_key": <key>}` is the source-implied shape (Spike-B `ensure_terminal`, `spikes/terminal-attach/src/main.rs:229`); OpenAPI documents the route but omits the requestBody schema. Response is a `SessionResourceObject` — `id` is a string; metadata carries `terminal_name`/`session_key`/`running`/`terminal_transport` (design "Pinned omnigent facts"). Server emits explicit `null` for empty collections — reuse the `de_null_default` pattern from `sessions.rs:31` for any `Vec`/`Map` field.

**⚠ NOT greenfield — reconcile with existing dead wrappers.** The design says "no terminal.rs exists yet," but `sessions.rs` **already** has three provisional terminal wrappers with **no callers anywhere in the workspace** (verified): `create_terminal(id, opts: &serde_json::Value) -> ResourceObject` (`sessions.rs:1612` — leaks `Value`, and its `{"launch_args"}` body guess is **wrong**: Spike-B live-verified `{"terminal","session_key"}`), `delete_terminal` (`:1626` — fine but untyped-return), and `transfer_terminal` (`:1640`). This task **supersedes** `create_terminal`/`delete_terminal` with the typed `Terminals` subservice and **removes `transfer_terminal` entirely** — the design excludes any client-callable transfer ("A client-callable terminal transfer operation; intentionally absent"). Since nothing calls the three, deletion needs no caller migration. `ResourceObject` (`sessions.rs:1209`, id+object only) is too thin for terminals — the new `TerminalResource` carries the metadata; leave `ResourceObject` for other resource kinds.

- [ ] **Step 1: Failing test — create/get round-trip codec.** In `rest.rs` `#[cfg(test)]`:
  ```rust
  #[test]
  fn terminal_resource_decodes_metadata() {
      let body = r#"{"id":"term_abc","session_id":"sess_1","name":"shell",
        "metadata":{"terminal_name":"shell","session_key":"main","running":true,
        "terminal_transport":"control"}}"#;
      let r: TerminalResource = serde_json::from_str(body).unwrap();
      assert_eq!(r.id.as_str(), "term_abc");
      assert_eq!(r.metadata.terminal_name.as_deref(), Some("shell"));
      assert_eq!(r.metadata.running, Some(true));
  }
  #[test]
  fn terminal_create_serializes_wire_shape() {
      let c = TerminalCreate { terminal: "shell".into(), session_key: "main".into() };
      let v: serde_json::Value = serde_json::to_value(&c).unwrap();
      assert_eq!(v, serde_json::json!({"terminal":"shell","session_key":"main"}));
  }
  ```
- [ ] **Step 2: Run — expect fail** (types undefined). `cargo test -p lens-client terminal::rest`.
- [ ] **Step 3: Implement** the typed structs + the four methods. Methods build the URL via `self.client.conn.url(...)` (see how `sessions.rs` reaches the connection/http), send via the blocking client, and route status→body through `decode_json`. `delete` maps 2xx→`Ok(())`, `404`→`ClientError::NotFound`. Follow the exact error-mapping idiom in `sessions.rs`/`http.rs`; do not invent a new one.
- [ ] **Step 4: Run — expect pass.**
- [ ] **Step 5: Commit.** `feat(lens-client): typed terminal REST CRUD (Slice 1a)`

---

### Task 3: WS frame codec (`wire.rs`)

**Files:**
- Modify: `crates/lens-client/src/terminal/wire.rs`

**Interfaces:**
- Produces:
  - `enum WsInbound { Vt(Vec<u8>), Text(String), Closed(CloseCause) }`
  - `enum WsOutbound { Input(Vec<u8>), Resize { cols: u16, rows: u16 } }`
  - `fn encode_outbound(o: &WsOutbound) -> tokio_tungstenite::tungstenite::Message` (crate-internal — `pub(crate)`, NOT re-exported)
  - `fn classify_inbound(msg: tokio_tungstenite::tungstenite::Message) -> Option<WsInbound>` (crate-internal). `Ping`/`Pong`/`Frame` → `None`; `Binary` → `Vt`; `Text` → `Text`; `Close(frame)` → `Closed(classify_close(code))`.

**Grounding (design "Framing"):** server→client **binary** = raw VT (feed verbatim downstream); client→server **binary** = keystrokes/paste/mouse + DA/DSR back-channel; client→server **text** = `{"type":"resize","cols":N,"rows":M}`. Output is a **byte stream, not messages** — the consumer concatenates `Vt` payloads.

- [ ] **Step 1: Failing tests.**
  ```rust
  #[test]
  fn resize_encodes_exact_json_text_frame() {
      let m = encode_outbound(&WsOutbound::Resize { cols: 120, rows: 40 });
      match m { Message::Text(t) => assert_eq!(t.as_str(), r#"{"type":"resize","cols":120,"rows":40}"#), _ => panic!() }
  }
  #[test]
  fn input_encodes_binary_frame_verbatim() {
      let m = encode_outbound(&WsOutbound::Input(vec![0x1b, b'[', b'A']));
      match m { Message::Binary(b) => assert_eq!(&b[..], &[0x1b, b'[', b'A']), _ => panic!() }
  }
  #[test]
  fn binary_inbound_is_vt_bytes_verbatim() {
      let got = classify_inbound(Message::Binary(vec![0x1b, b'c'].into()));
      assert!(matches!(got, Some(WsInbound::Vt(b)) if b == vec![0x1b, b'c']));
  }
  #[test]
  fn ping_pong_are_ignored() {
      assert!(classify_inbound(Message::Ping(vec![].into())).is_none());
  }
  ```
  Note the exact resize JSON field order (`type`,`cols`,`rows`) — build it with a fixed `format!`, not a serde struct, so the byte order is guaranteed (matches Spike-B `run_resize`, `spikes/terminal-attach/src/main.rs:579`).
- [ ] **Step 2: Run — expect fail.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run — expect pass.**
- [ ] **Step 5: Commit.** `feat(lens-client): typed WS terminal frame codec (Slice 1a)`

---

### Task 4: Close-code classification (`close.rs`)

**Files:**
- Modify: `crates/lens-client/src/terminal/close.rs`

**Interfaces:**
- Produces:
  - ```rust
    #[non_exhaustive]
    pub enum CloseCause {
        TerminalNotFound,   // 4404 — live-confirmed
        TerminalDetached,   // 4405 — source-derived
        Internal,           // 4500 — source-derived
        Unauthorized,       // 1008
        Network,            // generic/no-code closure
    }
    ```
  - `pub(crate) fn classify_close(code: u16) -> CloseCause`

**Grounding (design "Close codes"):** `4404` missing/dead (**live-confirmed** via a bogus tid), `4405` detached-while-alive (source-derived), `4500` internal (source-derived), `1008` authorization, generic network otherwise. This task **classifies** only — the stop/retry/downgrade/reattach *policy* is Slice 1d.

- [ ] **Step 1: Failing test.**
  ```rust
  #[test]
  fn classifies_known_codes() {
      assert!(matches!(classify_close(4404), CloseCause::TerminalNotFound));
      assert!(matches!(classify_close(4405), CloseCause::TerminalDetached));
      assert!(matches!(classify_close(4500), CloseCause::Internal));
      assert!(matches!(classify_close(1008), CloseCause::Unauthorized));
      assert!(matches!(classify_close(1006), CloseCause::Network));
  }
  ```
- [ ] **Step 2: Run — expect fail.**
- [ ] **Step 3: Implement** the `match`.
- [ ] **Step 4: Run — expect pass.**
- [ ] **Step 5: Commit.** `feat(lens-client): typed terminal close-code classification (Slice 1a)`

---

### Task 5: WS attach — contained runtime, bounded queues, I/O thread

**Files:**
- Modify: `crates/lens-client/src/terminal/attach.rs`

**Interfaces:**
- Consumes: `wire::{WsInbound, WsOutbound, encode_outbound, classify_inbound}`, `close::CloseCause`, `Client` (for base URL + auth), `SessionId`/`TerminalId`.
- Produces:
  - `struct AttachOptions { pub read_only: bool }`
  - `struct AttachHandle { pub inbound: crossbeam_channel::Receiver<WsInbound>, pub outbound: crossbeam_channel::Sender<WsOutbound>, /* private: join handle, runtime shutdown, inspect */ }`
  - `fn attach(client: &Client, session: &SessionId, tid: &TerminalId, opts: AttachOptions) -> Result<AttachHandle>`
  - `impl AttachHandle { pub fn close(self); pub fn inspect(&self) -> AttachInspect; }`
  - `struct Backoff { /* 30s cap, exponential */ }` with `fn next_delay(&mut self) -> std::time::Duration` and `fn reset(&mut self)`.

**Design decisions (justify in the commit body; these are review focus points):**
- **Contained current-thread tokio runtime.** `attach` spawns ONE `std::thread`; inside it, `tokio::runtime::Builder::new_current_thread().enable_all().build()?.block_on(io_loop(...))`. tokio types never cross the thread boundary — only `WsInbound`/`WsOutbound` do, over crossbeam.
- **Full-duplex `io_loop`:** `connect_async(request)` → `.split()` into sink/stream (Spike-B `AttachConn::connect`, `spikes/terminal-attach/src/main.rs:409-418`). `tokio::select!` over: (a) `stream.next()` → `classify_inbound` → **try_send** to the bounded `inbound` channel; (b) an async view of the `outbound` crossbeam receiver → `encode_outbound` → `sink.send`. Use `tokio::task::spawn_blocking` or a small bridging task to await the crossbeam `outbound` receiver without blocking the runtime (or wrap outbound in a `tokio::sync::mpsc` fed by a tiny forwarder thread — pick one; document it).
- **Bounded queues + saturation → disconnect (design "Backpressure"):** both channels are `crossbeam_channel::bounded(CAP)`. If the **inbound** `try_send` fails (consumer not draining — sustained saturation), do **not** drop VT bytes silently: initiate a clean close and emit a final `WsInbound::Closed(CloseCause::Network)` so 1d enters visible reconnect. (Momentary fullness may block briefly; sustained fullness disconnects. Pick a policy — e.g. a bounded blocking send with a timeout, then disconnect — and test it.)
- **Auth:** apply the same auth the `Client`/`Connection` carries to the WS handshake request headers (Spike-B applied a Bearer header, `main.rs:400-407`). On a local dev server there is no auth (design); still route it through `Connection`'s auth so a token deployment works.
- **URL:** `ws(s)://<base>/v1/sessions/{sid}/resources/terminals/{tid}/attach?read_only=<bool>`. **No `transport=`.** http→ws scheme swap per Spike-B `http_to_ws_base` (`main.rs:168`).

- [ ] **Step 1: Failing test — bounded outbound + resize reaches the wire (deterministic, no server).** Test the codec-through-channel seam without a live socket: construct the outbound channel, push `WsOutbound::Resize{..}`, and assert `encode_outbound` of the drained value is the exact text frame. (The live socket path is covered by Task 8.) Also unit-test `Backoff`:
  ```rust
  #[test]
  fn backoff_grows_then_caps_at_30s() {
      let mut b = Backoff::default();
      let d0 = b.next_delay(); let d1 = b.next_delay();
      assert!(d1 >= d0);
      for _ in 0..20 { let d = b.next_delay(); assert!(d <= std::time::Duration::from_secs(30)); }
      b.reset(); assert!(b.next_delay() <= d1);
  }
  ```
- [ ] **Step 2: Run — expect fail.**
- [ ] **Step 3: Implement** `attach`, the I/O thread + contained runtime, `io_loop`, `Backoff`, and `close()` (signal shutdown → the runtime's `block_on` returns → `join`). No `unwrap` on any I/O path.
- [ ] **Step 4: Run — expect pass** (deterministic units). Real socket exercised in Task 8.
- [ ] **Step 5: Commit.** `feat(lens-client): WS terminal attach — contained runtime + bounded queues (Slice 1a)`

---

### Task 6: `Inspect` snapshot + diagnostic ring

**Files:**
- Modify: `crates/lens-client/src/terminal/attach.rs`

**Interfaces:**
- Produces:
  - `#[derive(Clone, Debug, Serialize)] struct AttachInspect { pub connected: bool, pub inbound_len: usize, pub inbound_cap: usize, pub outbound_len: usize, pub outbound_cap: usize, pub bytes_in: u64, pub bytes_out: u64, pub last_close: Option<CloseCause>, pub recent: Vec<InspectEvent> }`
  - A **fixed-capacity** ring (e.g. `[InspectEvent; N]` or a bounded `VecDeque`) recording connect/close/saturation transitions.

**Constraint (MANDATORY, design "Inspect"):** when introspection is **disabled** (a flag/`AtomicBool`), the hot path performs **zero** snapshot construction, event recording, allocation, or synchronization. Record events only behind the enable check; counters that are always maintained must be plain relaxed atomics (cheap), not locked.

- [ ] **Step 1: Failing test.** Enable inspect, drive a fake connect+close through the ring, assert `inspect().recent` holds those two events and `last_close` is set; then assert that with inspect disabled, the ring stays empty after the same drive.
- [ ] **Step 2: Run — expect fail.**
- [ ] **Step 3: Implement.** Gate all recording behind the enable `AtomicBool`. Ring is fixed-capacity (oldest-evicted).
- [ ] **Step 4: Run — expect pass.**
- [ ] **Step 5: Commit.** `feat(lens-client): terminal attach Inspect snapshot + diagnostic ring (Slice 1a)`

---

### Task 7: Criterion benches

**Files:**
- Create: `crates/lens-client/benches/terminal_transport.rs`
- Modify: `crates/lens-client/Cargo.toml` (add `[[bench]]`)

**Interfaces:**
- Consumes: `wire::{encode_outbound, classify_inbound}`, a `test-util`/`bench`-gated accessor if the codec fns are `pub(crate)` (mirror the existing `bench`/`test-util` gating in `lens-client` — see `stream::bench_api`).

- [ ] **Step 1:** Add `[[bench]] name = "terminal_transport"  harness = false  required-features = ["bench"]` (mirror the `sse_pipeline` bench entry in `Cargo.toml`).
- [ ] **Step 2:** Write benches: (a) `classify_inbound` on a representative binary VT frame; (b) `encode_outbound` for `Input` and `Resize`; (c) bounded-queue throughput — push/drain N `WsInbound::Vt` through a `bounded(CAP)` channel. Return owned outputs from the timed closure (avoid charging `Drop` to the body — memory `benchmark-validity-audit`).
- [ ] **Step 3:** Run: `cargo bench -p lens-client --features bench --bench terminal_transport -- --warm-up-time 1 --measurement-time 3`. Expected: completes, prints throughput/latency.
- [ ] **Step 4: Commit.** `bench(lens-client): terminal transport codec + queue benches (Slice 1a)`

---

### Task 8: Live end-to-end REST + attach rider (feature-gated)

**Files:**
- Create: `crates/lens-client/tests/terminal_live.rs`

**Interfaces:**
- Consumes: the full public `terminal` surface + `Client::new` handshake.

**Constraint:** gate the whole file behind `#![cfg(feature = "live-tests")]` so default `cargo test` stays offline/green. Read `OMNIGENT_BASE_URL` (default `http://127.0.0.1:8000`) and optional `OMNIGENT_SESSION_ID`; skip-with-log if the server is unreachable (do not fail the suite on a missing server). omnigent `0.5.1 (08285468)` is on PATH — the pinned rev — so this rider is runnable now.

- [ ] **Step 1:** Write the rider: build a `Client`, create a session if needed, `terminals(session).create(&TerminalCreate{terminal:"shell",session_key:"main"})`, `list`/`get` it, `attach(read_only:false)`, send `WsOutbound::Input(b"printf 'MARKER_A\\n'\n")`, assert an inbound `WsInbound::Vt` payload containing `MARKER_A` arrives within a timeout, send `WsOutbound::Resize{cols:120,rows:40}`, then `delete` and assert a subsequent `get` is `NotFound`. Also drive the **`4404` live-classify**: `attach` a bogus `TerminalId`, assert the inbound `Closed(TerminalNotFound)` (Spike-B live-confirmed this exact code).
- [ ] **Step 2:** Run: `cargo test -p lens-client --features live-tests --test terminal_live -- --nocapture`. Expected: PASS against the running omnigent.
- [ ] **Step 3: Commit.** `test(lens-client): live terminal REST + attach + 4404 rider (Slice 1a)`

---

### Task 9: Gate + slice review

- [ ] **Step 1:** `cargo fmt -p lens-client && cargo clippy --workspace --all-targets -- -D warnings` — must be clean.
- [ ] **Step 2:** `cargo test -p lens-client` (offline) — all green.
- [ ] **Step 3:** Cross-family review of the whole 1a diff (MANDATORY review diversity — author is composer, so review from a non-composer family: `codex` gpt-5.5 and/or `grok`). Fold findings; re-verify the gate.
- [ ] **Step 4: Commit** any review fixes; update `docs/STATUS.md` (memory `end-of-session-status-update`).

## Self-Review (author, before handoff)

- **Spec coverage:** REST CRUD ✓(T2); WS attach ✓(T5); close **classification** ✓(T4) — policy correctly deferred to 1d; reconnect *mechanics* (Backoff, re-attach primitive) ✓(T5); backpressure→visible-reconnect ✓(T5); no `serde_json::Value`/raw-WS leak ✓(typed `WsInbound`/`WsOutbound`, codec `pub(crate)`); Inspect ✓(T6); benches ✓(T7); live rider incl. 4404 ✓(T8). `transport=` omitted ✓(T5 URL).
- **Deferred (NOT this slice):** close-code policy, DA/DSR forwarding, Ghostty/Frame — all Slice 1b/1d.
- **Type consistency:** `WsInbound`/`WsOutbound`/`CloseCause`/`AttachHandle`/`TerminalResource` names identical across tasks. `attach()` signature is stable from T5.
