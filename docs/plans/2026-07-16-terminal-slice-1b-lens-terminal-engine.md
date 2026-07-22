# Terminal Slice 1b — `lens-terminal` engine core Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stand up the non-`Send` VT engine worker inside `lens-terminal`: a dedicated OS thread that owns the `libghostty-vt` `Terminal`, feeds it server VT bytes, builds an immutable Lens-owned `Frame` on that thread, and publishes it across the Send boundary under a throttled, lost-wake-safe publish protocol.

**Architecture:** A `libghostty-vt` `Terminal` is `!Send + !Sync`, so it lives for its whole life on **one pinned `std::thread`** (never gpui's migrating executor). The worker owns `Terminal` + `RenderState` + reusable row/cell iterators. Server VT bytes arrive over a **bounded `crossbeam` channel**; the worker `vt_write`s them, and — **throttled to at most one build per display sample** — snapshots the screen and copies every visible cell into an owned `Frame` (no Ghostty type escapes). The `Frame` publishes into an `arc_swap::ArcSwapOption<Frame>` slot and fires a **coalesced wake** under a dirty/ack `AtomicBool` (the foreground `cx.notify` binding is wired in 1d). VT replies (DA/DSR) from `on_pty_write` ride a separate **bounded engine→transport channel**. Teardown is a `Stop` command → drain → thread exit → off-foreground `join`.

**Tech Stack:** Rust 2024, `libghostty-vt` (vendored path dep — new), `arc-swap` (new), `crossbeam-channel` (new), `gpui` (already a dep; used only for the eventual `Frame`/entity glue, NOT on the engine thread). No `tokio`. `unsafe` forbidden.

## Global Constraints

- Rust edition 2024, `rust-version = 1.91`.
- **MANDATORY** The `libghostty-vt` `Terminal`/`RenderState`/iterators are **created, used, and dropped on the single engine thread only**. They never cross a thread boundary. The engine worker uses a raw `std::thread`, **not** gpui's background executor (which may migrate work).
- **MANDATORY** **No Ghostty type crosses the `Frame` seam.** `Frame` and its cells are Lens-owned plain data (`Rgb`, `CellStyle`, `String`/grapheme). The `libghostty_vt::style::RgbColor`/`Style`/`Snapshot` types stay inside `engine/vt.rs`.
- **MANDATORY** Never block the foreground: nothing here runs on the gpui foreground thread. The `on_pty_write` callback must **not** block on channel I/O — it appends to an in-thread buffer that the worker drains after `vt_write` returns (the callback fires synchronously inside `vt_write`; source `terminal.rs:77`).
- **MANDATORY** UI never panics: the worker converts every `libghostty-vt` `Result` error into a logged, modeled outcome; a build failure skips that frame, it never `unwrap`s on a hot path.
- **MANDATORY** `cargo clippy --workspace --all-targets -- -D warnings` clean + `rustfmt`. `unsafe` forbidden.
- **MANDATORY** Benchmark-or-it's-not-done: engine benches for VT parse, `Frame` construction, scroll, reflow (Criterion, feature `bench`).
- **MANDATORY** Introspectable: an engine `Inspect` snapshot (dimensions, scrollback/history extent, lifecycle, last-build stats) + fixed-capacity ring; **zero cost when disabled**.
- **Offline & deterministic:** every test in this slice is driven by **replayed Spike-B capture bytes** (`docs/spikes/captures/2026-07-15-pty-attach/*.frames.jsonl`) or hand-built VT byte sequences — **no live server, no gpui window**. (The live proof is Slice 1d.)
- Ground truth for the engine API: `vendor/libghostty-rs/libghostty-vt/src/{terminal.rs,style.rs,screen.rs,render.rs}`. Reference consumer: `spikes/terminal-render/src/{main.rs,paint.rs}` (`VtEngine`, `collect_rows`). The design `docs/specs/2026-07-16-terminal-workstream-design.md` ("Threading & the `Frame` seam", "Render contract").
- **Confirmed API facts** (do not re-derive): `Terminal::new(TerminalOptions { cols, rows, max_scrollback })? -> Terminal<'static,'static>`; builder `.on_pty_write(|term, data: &[u8]| {..})?`; `terminal.vt_write(&[u8])`; `terminal.resize(cols, rows, cell_w_px, cell_h_px)? `; `RenderState::new()?`; `render_state.update(&mut terminal)? -> Snapshot`; `RowIterator::new()?` / `CellIterator::new()?` (reusable, `.update(snapshot)` / `.update(&row)`); per-cell: `cell.raw_cell()?.wide()? -> screen::CellWide`, `cell.graphemes()? -> impl IntoIterator<char>`, `cell.fg_color()? -> Option<style::RgbColor>`, `cell.bg_color()? -> Option<..>`, `cell.style()? -> style::Style`, `cell.is_selected()? -> bool`; `snapshot.colors()? -> {foreground, background}`, `snapshot.cols()?`, `snapshot.rows()?`.

---

## File Structure

- `crates/lens-terminal/src/engine/mod.rs` — engine module root; `pub use` `EngineHandle`, `EngineConfig`, `Frame`, `FrameRow`, `FrameCell`, `CellStyle`, `Rgb`, `UnderlineStyle`, `EngineInspect`.
- `crates/lens-terminal/src/engine/frame.rs` — the Lens-owned `Frame` types (fills Slice 0's opaque `Frame`) + the builder that copies a `Snapshot` into an owned `Frame`.
- `crates/lens-terminal/src/engine/vt.rs` — `VtEngine`: owns `Terminal` + `RenderState` + iterators + the `on_pty_write` reply buffer; `feed`, `resize`, `build_frame`. **The only file that names a `libghostty_vt` type.**
- `crates/lens-terminal/src/engine/worker.rs` — the `std::thread` run-loop: command channel, throttle, publish-and-wake, DA/DSR drain, teardown.
- `crates/lens-terminal/src/engine/handle.rs` — `EngineHandle` (Send): spawn the worker; `feed`, `resize`, `set_visible`, `stop`, `latest_frame`, `set_waker`, `da_dsr_rx`, `inspect`.
- `crates/lens-terminal/src/lib.rs` — replace the opaque Slice-0 `Frame` with `pub use engine::Frame;`; add `mod engine; pub use engine::{...}`.
- `crates/lens-terminal/Cargo.toml` — add `libghostty-vt` (path), `arc-swap`, `crossbeam-channel`; `[[bench]] name = "engine"`.
- `crates/lens-terminal/benches/engine.rs` — Criterion benches.

---

### Task 1: Add the VT dep + relocate `Frame` behind an engine module

**Files:**
- Modify: `crates/lens-terminal/Cargo.toml`, `crates/lens-terminal/src/lib.rs`
- Create: `crates/lens-terminal/src/engine/mod.rs`, `crates/lens-terminal/src/engine/frame.rs`

**Interfaces:**
- Produces: `lens_terminal::Frame` still resolves (now `pub use engine::frame::Frame`), plus the new cell types. The Slice-0 opaque `Frame {}` is **replaced** by the concrete definition below.

**⚠ Build note:** adding `libghostty-vt` triggers the pinned-Ghostty `build.rs` **source fetch + Zig build** on first compile (network, then cached in `OUT_DIR`; the `ZIG` override + `build.rs` patch are already in place — the spikes compile against it). Expect a slow first build.

- [ ] **Step 1:** In `Cargo.toml` `[dependencies]` add:
  ```toml
  libghostty-vt = { path = "../../vendor/libghostty-rs/libghostty-vt" }
  arc-swap = "1"
  crossbeam-channel = "0.5"
  ```
  Remove the "NOT a dependency yet" note comment.
- [ ] **Step 2: Define the Lens-owned Frame types** in `engine/frame.rs` (no `libghostty_vt` import — pure Lens data):
  ```rust
  /// Resolved 24-bit color. No Ghostty/gpui type crosses the Frame seam.
  #[derive(Clone, Copy, Debug, PartialEq, Eq)]
  pub struct Rgb { pub r: u8, pub g: u8, pub b: u8 }

  /// Full SGR attribute set carried per cell (design: 1c renders the full set;
  /// paint.rs today does only bold+selection). Mirrors libghostty `style::Style`.
  #[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
  pub struct CellStyle {
      pub bold: bool, pub italic: bool, pub faint: bool, pub blink: bool,
      pub inverse: bool, pub invisible: bool, pub strikethrough: bool,
      pub overline: bool, pub underline: UnderlineStyle,
      pub underline_color: Option<Rgb>,
  }

  #[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
  pub enum UnderlineStyle { #[default] None, Single, Double, Curly, Dotted, Dashed }

  #[derive(Clone, Debug, PartialEq)]
  pub struct FrameCell {
      pub col: u16,          // grid column (wide spacer tails/heads are dropped)
      pub grapheme: String,  // one grapheme cluster; " " for blank
      pub fg: Rgb,
      pub bg: Option<Rgb>,   // None = default bg
      pub wide: bool,
      pub selected: bool,
      pub style: CellStyle,
  }

  #[derive(Clone, Debug, PartialEq)]
  pub struct FrameRow { pub cells: Vec<FrameCell> }

  /// Immutable owned snapshot of the visible grid — the Send boundary.
  #[derive(Clone, Debug, PartialEq)]
  pub struct Frame {
      pub cols: u16,
      pub rows: u16,
      pub default_fg: Rgb,
      pub default_bg: Rgb,
      pub grid: Vec<FrameRow>,
  }
  ```
  (This mirrors the spike's `RowPaint`/`CellPaint` exactly so 1c's lifted painter consumes it unchanged. `Frame` is `Clone` for tests/benches but is shared in production as `Arc<Frame>` — clone the `Arc`, not this.)
- [ ] **Step 3:** In `lib.rs`, add `mod engine;` and replace the opaque `pub struct Frame {}` with `pub use engine::frame::Frame;` (plus `pub use engine::frame::{FrameRow, FrameCell, CellStyle, Rgb, UnderlineStyle};`). Delete the Slice-0 placeholder struct.
- [ ] **Step 4:** `cargo build -p lens-terminal` (slow first build — fetches Ghostty). Expected: compiles.
- [ ] **Step 5: Commit.** `feat(lens-terminal): VT engine dep + Lens-owned Frame types (Slice 1b)`

---

### Task 2: `VtEngine` — own the Terminal, build an owned Frame from a snapshot

**Files:**
- Create: `crates/lens-terminal/src/engine/vt.rs`
- Modify: `crates/lens-terminal/src/engine/frame.rs` (add the `Snapshot → Frame` builder, or keep it in `vt.rs` — but the `libghostty_vt` types must stay in `vt.rs`; put the builder here)

**Interfaces:**
- Consumes: `frame::{Frame, FrameRow, FrameCell, CellStyle, Rgb, UnderlineStyle}`.
- Produces:
  - `struct EngineConfig { pub cols: u16, pub rows: u16, pub max_scrollback: usize, pub cell_w_px: u32, pub cell_h_px: u32 }`
  - `struct VtEngine { /* Terminal<'static,'static>, RenderState, RowIterator, CellIterator, reply buffer */ }`  — **`!Send`** (holds Ghostty types); never expose it across a thread.
  - `fn VtEngine::new(cfg: &EngineConfig, on_reply: impl FnMut(&[u8]) + 'static) -> Result<VtEngine, EngineError>` — installs `on_pty_write` to push into an in-thread `Rc<RefCell<Vec<u8>>>`; `on_reply` is called by the worker after draining (Task 4). Keep the closure capturing the buffer only.
  - `fn feed(&mut self, bytes: &[u8])` — `terminal.vt_write(bytes)`, then hand any buffered DA/DSR reply bytes to the caller (return them or drain via an accessor).
  - `fn take_replies(&mut self) -> Vec<u8>` — drains the `on_pty_write` buffer.
  - `fn resize(&mut self, cols: u16, rows: u16) -> Result<(), EngineError>` — `terminal.resize(cols, rows, cell_w_px, cell_h_px)`.
  - `fn build_frame(&mut self) -> Result<Frame, EngineError>` — the owned-copy builder.
  - `enum EngineError { … }` (`thiserror`) wrapping `libghostty_vt::error::Error`.

**The builder (grounded in `paint.rs::collect_rows`, `spikes/terminal-render/src/paint.rs:132`):**
- `let snap = self.render_state.update(&mut self.terminal)?;`
- `let colors = snap.colors()?; default_fg/bg = colors.foreground/background` → `Rgb`.
- `cols = snap.cols()?; rows = snap.rows()?`.
- For each row (via `RowIterator::update(&snap)`), each cell (via `CellIterator::update(&row)`): skip `CellWide::SpacerTail | SpacerHead`; read `graphemes` (join to `String`, `" "` if empty), `fg_color().unwrap_or(default_fg)`, `bg_color()`, `style()`, `is_selected()`, `wide == CellWide::Wide`; map `style::Style` → `CellStyle` (all flags + `UnderlineStyle` from `style.underline`; `underline_color` = `Some(Rgb)` only for the `StyleColor::Rgb` variant, else `None` — palette resolution deferred). Push a `FrameCell { col, .. }`.
- Convert every `libghostty_vt::style::RgbColor { r,g,b }` → `Rgb { r,g,b }` (add a `From`/helper in `vt.rs`).
- **Do NOT** compute a content hash here (that's a 1c render-cache concern) and do NOT call `set_dirty` (render-side bookkeeping; 1b owns the snapshot read only).

- [ ] **Step 1: Failing test — a known VT sequence yields the expected owned cells.** In `vt.rs` `#[cfg(test)]`:
  ```rust
  #[test]
  fn builds_frame_with_sgr_and_colors() {
      let mut e = VtEngine::new(&EngineConfig{cols:20,rows:3,max_scrollback:100,cell_w_px:8,cell_h_px:16}, |_|{}).unwrap();
      // bold + red fg "Hi", then reset newline
      e.feed(b"\x1b[1;31mHi\x1b[0m\r\n");
      let f = e.build_frame().unwrap();
      assert_eq!((f.cols, f.rows), (20, 3));
      let c0 = &f.grid[0].cells[0];
      assert_eq!(c0.grapheme, "H");
      assert!(c0.style.bold);
      assert_eq!(c0.fg, Rgb{r:.., g:.., b:..}); // the palette-red the engine resolves — assert exact from a first run
  }
  ```
  (Run once, read the resolved red, then pin it. Note memory `gpui-component-hex-roundtrip-lossy` is about the gpui bridge — irrelevant here; the engine resolves palette→RGB deterministically.)
- [ ] **Step 2: Run — expect fail.** `cargo test -p lens-terminal engine::vt`.
- [ ] **Step 3: Implement** `VtEngine`, the `on_pty_write` buffer, `feed`/`take_replies`/`resize`/`build_frame`, `EngineError`, and the `RgbColor→Rgb` / `Style→CellStyle` mappings.
- [ ] **Step 4: Run — expect pass.**
- [ ] **Step 5: Add a wide/emoji test.** Feed `"a日b😀c"` (CJK + emoji); assert the wide cell has `wide == true`, the spacer tail is dropped, and the column indices stay on the grid (`col` monotonic, emoji cell `col` accounts for the preceding wide cell). Run — pass.
- [ ] **Step 6: Commit.** `feat(lens-terminal): VtEngine owns Terminal + builds owned Frame (Slice 1b)`

---

### Task 3: Replay-driven Frame correctness (offline, deterministic)

**Files:**
- Create: `crates/lens-terminal/tests/replay_frame.rs`
- (Reuse the capture files under `docs/spikes/captures/2026-07-15-pty-attach/`.)

**Interfaces:**
- Consumes: `VtEngine`, `EngineConfig`, `Frame`.

- [ ] **Step 1:** Port the capture reader from `spikes/terminal-render/src/main.rs:469` (`load_replay_bytes`): concatenate the `hex` of every `{"direction":"in","kind":"binary"}` frame in a `.frames.jsonl`. Put it in the test file (hand-parse; no serde needed — the comment there explains why it is safe).
- [ ] **Step 2: Failing test:** feed the `attach.frames.jsonl` bytes into a `VtEngine` at the capture's grid (default `80×24` unless the capture header says otherwise), `build_frame()`, and assert invariants that hold for any real shell redraw: `f.grid.len() == f.rows as usize`, every row's cells have monotonically increasing `col < cols`, and the frame is not all-blank (some cell has a non-space grapheme). This proves the engine parses a **real** omnigent VT stream into a coherent owned frame.
- [ ] **Step 3: Run — expect fail then implement/adjust → pass.**
- [ ] **Step 4:** Add a **resize-reflow** replay test: feed the `resize.frames.jsonl` bytes, call `resize(120, 40)`, feed the post-resize bytes, `build_frame()`, assert `(cols,rows) == (120,40)` and no panic.
- [ ] **Step 5: Commit.** `test(lens-terminal): replay-driven Frame + resize correctness (Slice 1b)`

---

### Task 4: The engine worker thread — command loop, throttle, publish-and-wake, DA/DSR

**Files:**
- Create: `crates/lens-terminal/src/engine/worker.rs`, `crates/lens-terminal/src/engine/handle.rs`

**Interfaces:**
- Consumes: `VtEngine`, `EngineConfig`, `Frame`.
- Produces:
  - ```rust
    pub struct EngineHandle { /* Send: command Sender, ArcSwapOption<Frame>, wake AtomicBool, da_dsr Receiver, JoinHandle, EngineInspect handle */ }
    impl EngineHandle {
        pub fn spawn(cfg: EngineConfig) -> Self;                 // spawns the std::thread
        pub fn feed(&self, bytes: Vec<u8>);                      // transport→engine VT bytes (bounded; saturation → Err/backpressure signal)
        pub fn resize(&self, cols: u16, rows: u16);
        pub fn set_visible(&self, visible: bool);                // hidden → stop publishing frames
        pub fn latest_frame(&self) -> Option<Arc<Frame>>;        // UI samples this on wake
        pub fn set_waker(&self, waker: Box<dyn Fn() + Send + Sync>); // 1d sets cx.notify; tests set a counter
        pub fn da_dsr_rx(&self) -> crossbeam_channel::Receiver<Vec<u8>>; // reverse channel → 1d forwards to WS
        pub fn inspect(&self) -> EngineInspect;
        pub fn stop(self);                                        // Stop → drain → thread exit → join (off-foreground)
    }
    enum EngineCommand { Feed(Vec<u8>), Resize(u16,u16), SetVisible(bool), Stop }
    ```

**Worker design (design: "Threading & the Frame seam"):**
- `spawn` creates the bounded command channel + bounded `da_dsr` channel + `Arc<ArcSwapOption<Frame>>` + `Arc<AtomicBool>` dirty flag, then `std::thread::spawn` a closure that constructs `VtEngine` **inside the thread** (Ghostty types never cross) and runs the loop.
- Loop: `recv` a command (blocking). `Feed(b)` → `engine.feed(&b)`; drain `engine.take_replies()` → `try_send` to `da_dsr` (non-blocking; if full, drop-oldest or signal — pick and document; never block the loop); set the internal `dirty` flag. `Resize` → `engine.resize`. `SetVisible(false)` → suppress publishing. `Stop` → break.
- **Throttle + publish:** the worker does NOT build a Frame per `Feed`. After handling a command batch (drain the channel with `try_recv` until empty), if `dirty && visible && due` (a min-interval gate — inject a `Clock`/`Instant` source; for tests expose `build_now()` that ignores the gate), build a Frame, `slot.store(Some(Arc::new(frame)))`, set the wake ack, and call the `waker`. **Coalescing:** many `Feed`s between samples produce one Frame. A `SetVisible(true)` transition forces one publish.
- **Lost-wake safety:** publish sets `dirty=false` and `published=true` (a `store(Release)`); `latest_frame` loads `Acquire`. The waker is idempotent/coalesced — firing it twice before the UI samples is harmless (the UI reads the latest slot). Never miss the last Frame: always store the slot **before** firing the waker.
- **Teardown:** `stop` sends `Stop`, then `join`s the thread (the caller ensures this runs off the foreground — 1d calls it from an async task). Dropping the `JoinHandle` only detaches; `stop` must actually `join` so the `Terminal` + scrollback are reclaimed (design: "confirmed worker exit").

- [ ] **Step 1: Failing test — feed → coalesced frame appears.**
  ```rust
  #[test]
  fn feed_publishes_a_coalesced_frame_and_wakes() {
      let h = EngineHandle::spawn(EngineConfig{cols:20,rows:3,max_scrollback:100,cell_w_px:8,cell_h_px:16});
      let woke = Arc::new(AtomicUsize::new(0));
      { let w = woke.clone(); h.set_waker(Box::new(move || { w.fetch_add(1, Relaxed); })); }
      h.feed(b"AB".to_vec()); h.feed(b"CD".to_vec());
      h.build_now(); // deterministic: bypass the time gate
      let f = wait_for_frame(&h); // small poll loop with timeout
      assert!(f.grid[0].cells.iter().any(|c| c.grapheme == "A"));
      assert!(woke.load(Relaxed) >= 1);
      h.stop();
  }
  ```
  (`build_now` + `wait_for_frame` are test-support affordances on the handle/worker; gate the time-based throttle separately in Step 4.)
- [ ] **Step 2: Run — expect fail.**
- [ ] **Step 3: Implement** `worker.rs` + `handle.rs`.
- [ ] **Step 4: Run — expect pass.** Add: (a) a **DA/DSR** test — feed a Primary DA query `b"\x1b[c"`, assert `da_dsr_rx()` yields a non-empty reply (the engine's `on_pty_write` fires); (b) a **hidden-suppression** test — `set_visible(false)`, feed, `build_now`, assert no new frame published; `set_visible(true)` publishes one; (c) a **teardown** test — `stop()` returns and a subsequent `feed` is a no-op/Err (thread gone).
- [ ] **Step 5: Commit.** `feat(lens-terminal): engine worker — throttled publish-and-wake + DA/DSR + teardown (Slice 1b)`

---

### Task 5: Engine `Inspect` snapshot + diagnostic ring

**Files:**
- Modify: `crates/lens-terminal/src/engine/{worker.rs,handle.rs}`

**Interfaces:**
- Produces: `#[derive(Clone, Debug, Serialize)] pub struct EngineInspect { pub cols: u16, pub rows: u16, pub max_scrollback: usize, pub visible: bool, pub frames_built: u64, pub last_build_micros: u64, pub bytes_fed: u64, pub da_dsr_emitted: u64, pub recent: Vec<InspectEvent> }`.

**Constraint (MANDATORY):** when disabled, the worker performs **zero** extra snapshot construction, event recording, allocation, or synchronization on the feed/build hot path. Guard recording behind an enable `AtomicBool`; always-on counters are relaxed atomics.

- [ ] **Step 1: Failing test:** enable inspect, drive feed+build, assert `frames_built >= 1`, `bytes_fed > 0`, and the ring holds the build events; then disabled → ring empty after the same drive.
- [ ] **Step 2: Run → implement → pass.**
- [ ] **Step 3: Commit.** `feat(lens-terminal): engine Inspect snapshot + diagnostic ring (Slice 1b)`

---

### Task 6: Criterion engine benches

**Files:**
- Create: `crates/lens-terminal/benches/engine.rs`; modify `Cargo.toml` (`[[bench]] name = "engine"  harness = false  required-features = ["bench"]`).

- [ ] **Step 1:** Benches (all offline, feeding fixed byte fixtures or a replay capture into a `VtEngine` directly — not through the worker thread, to isolate CPU): (a) **VT parse** — `feed` a 200×50 full-redraw fixture; (b) **Frame construction** — `build_frame` on a seeded 200×50 grid (return the owned `Frame` from the timed closure — memory `benchmark-validity-audit`: don't charge `Drop` to the body); (c) **scroll** — feed N newlines past a full grid; (d) **reflow** — `resize(200,50)→(120,40)→(200,50)` on a seeded grid.
- [ ] **Step 2:** Run: `cargo bench -p lens-terminal --features bench --bench engine -- --warm-up-time 1 --measurement-time 3`. Expected: completes; record p50/p95. (Cross-check against Spike-A's snapshot ≪0.2 ms and the design's construction-dominated finding.)
- [ ] **Step 3: Commit.** `bench(lens-terminal): engine parse/frame/scroll/reflow benches (Slice 1b)`

---

### Task 7: Gate + slice review

- [ ] **Step 1:** `cargo fmt -p lens-terminal && cargo clippy --workspace --all-targets -- -D warnings` — clean.
- [ ] **Step 2:** `cargo test -p lens-terminal` — all green (offline).
- [ ] **Step 3:** Cross-family review of the whole 1b diff (author is composer → review from a non-composer family: `grok` and/or `codex`). Focus: the `!Send` discipline (no Ghostty type escaping `vt.rs`/the thread), lost-wake safety, teardown-actually-joins, `on_pty_write` non-blocking. Fold findings; re-verify the gate.
- [ ] **Step 4: Commit** review fixes; update `docs/STATUS.md`.

## Self-Review (author, before handoff)

- **Spec coverage:** engine worker owning non-`Send` `Terminal` ✓(T2,T4); `vt_write` ✓(T2); snapshot→`Frame` owned copy, no Ghostty type escaping ✓(T2, enforced by module boundary); resize reflow ✓(T2,T3); DA/DSR `on_pty_write` reverse channel ✓(T4); throttled publish-and-wake + lost-wake safety ✓(T4); hidden-tab Frame suppression **mechanism** ✓(T4) (the 1d Frame-seam wiring consumes it); teardown = confirmed worker exit/join ✓(T4); scrollback line-cap via `max_scrollback` ✓(T2 `EngineConfig`); full-SGR **carried in the Frame** for 1c ✓(T2 `CellStyle`); Inspect ✓(T5); benches ✓(T6); offline/deterministic via replay ✓(T3).
- **Deferred (NOT 1b):** the render/paint of the Frame (1c); the WS↔engine bridge + `cx.notify` waker binding + close-code policy (1d); byte-accurate scrollback accounting (Slice 3); palette→RGB underline-color resolution (1c refinement).
- **Type consistency:** `Frame`/`FrameRow`/`FrameCell`/`CellStyle`/`Rgb`/`EngineHandle`/`EngineConfig`/`EngineInspect` names identical across tasks. `build_frame`/`feed`/`resize`/`take_replies` signatures stable from T2. `EngineHandle` surface stable from T4.
- **Seam to 1c:** `Frame`'s `grid: Vec<FrameRow>` of `FrameCell{col,grapheme,fg,bg,wide,selected,style}` is byte-for-byte the shape `paint.rs::collect_rows` produces — 1c's lifted painter reads it directly instead of the live snapshot.
