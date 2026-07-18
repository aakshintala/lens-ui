# Terminal Slice 2 — Task 0: Shared Foundation Skeleton (merge-seam contract)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. This is a **single mechanical foundation commit** that lands on `terminal-ws` **before** the 2a and 2d worktrees branch. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Land one combined-skeleton commit that pre-declares *every* field, parameter, struct-literal, and render/build hook-point that **both** Slice-2a (input) and Slice-2d (presentation) will touch on the shared engine constructors — as compiling **no-op placeholders** — so that afterwards 2a and 2d each become a **single writer** to every shared definition and only fill **disjoint bodies/lines**. This dissolves the merge-seam collisions the 2026-07-17 gpt-5.6 review flagged (Critical #6).

**Architecture:** Additive-only. Rename the DA/DSR reverse channel to the generalized **egress** channel (2a routes encoded input bytes onto it); add the **presentation** channel + `EnginePresentationEvent` types (2d); add `Frame.cursor` + `FrameCell.hyperlink_uri`; add `VtEngine` key-encoder fields + a `presentation_tx` constructor param with a bare `on_title_changed`; add the `TerminalTab` interaction fields + render hook-points. **No new behavior** — every placeholder is inert (cursor always `None`, hyperlink always `None`, key encoder constructed-but-unused, only the raw title flows through presentation). The existing test suite must stay green.

**Tech Stack:** `crossbeam-channel`, `libghostty-vt` (`key::{Encoder,Event}`, `Terminal::on_title_changed`), `gpui`.

## Global Constraints

- **Additive & inert only.** No encoding, no gesture handling, no sanitize, no extraction in this task — those are 2a/2d bodies. If a placeholder would change observable behavior, it is wrong.
- **The single-writer rule (the whole point).** After this commit: 2a is the *only* plan that edits the `EngineHandle` struct definition (adds the forwarder), the `EngineCommand` enum, and the `cursor:` line in `build_frame` + the `on_key_down` body. 2d is the *only* plan that edits the `on_title_changed` closure body, the `hyperlink_uri:` line in `build_frame`, and the `on_mouse_down` body. Neither edits a definition the other also edits. Preserve this — if a later revision needs a shared definition changed, change it **here**, not in 2a/2d.
- **`cargo clippy -p lens-terminal --all-targets --features test-util,live-tests -- -D warnings`** must pass. Placeholder-unused fields get `#[expect(dead_code, reason = "…2a/2d fills…")]`, never `#[allow]` that rots. Never pipe the gate through `tail`.
- **Existing suite green:** every current `lens-terminal` test passes unchanged except for mechanical literal updates (new struct fields).
- Ground truth for shapes: real code at `engine/{frame.rs,worker.rs,handle.rs,vt.rs,mod.rs}`, `lib.rs`, `render/state.rs`. Review context: the gpt-5.6 findings folded here are #6 (merge seam) and, opportunistically, the *shape* of I5 (`Frame.cursor` is `Option`, viewport-safe — 2a fills the real computation).

---

## Combined target shapes (authoritative)

### `engine/presentation.rs` — **Create** (types only, no logic)

```rust
//! Engine → foreground presentation egress (Slice 2d) — TYPES ONLY here.
//! Title sanitize, hyperlink extraction, URL validation, OSC-52 policy are
//! filled by 2d / 2b. Task 0 only declares the channel payload + constants.

use crossbeam_channel::Sender;

/// Channel capacity for engine→foreground presentation events (matches egress).
pub const PRESENTATION_CHANNEL_CAP: usize = 64;
/// Reported-title bound (2d truncates to this many Unicode scalars).
pub const MAX_REPORTED_TITLE_CHARS: usize = 512;
/// Hyperlink URI extraction ceiling (2d).
pub const MAX_HYPERLINK_URI_BYTES: usize = 8192;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClipboardLocation {
    Standard,
    Selection,
    Primary,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipboardMimePart {
    pub mime: String,
    /// Decoded per-MIME representation. Preserve parts; never flatten.
    pub data: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EnginePresentationEvent {
    TitleChanged(String),
    /// Enqueued foreground-side by 2d's gesture path onto the same channel.
    HyperlinkOpen { url: String },
    /// 2d/2b fill the emission; Task 0 only declares the variant.
    ClipboardWrite {
        location: ClipboardLocation,
        contents: Vec<ClipboardMimePart>,
    },
}

/// Convenience alias used across worker/handle wiring.
pub(crate) type PresentationSender = Sender<EnginePresentationEvent>;
```

### `engine/frame.rs` — **Modify** (`frame.rs:36-63`)

Add `CursorPos`; add `cursor` to `Frame`; add `hyperlink_uri` to `FrameCell`:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CursorPos {
    pub col: u16,
    pub row: u16,
}

pub struct FrameCell {
    pub col: u16,
    pub grapheme: String,
    pub fg: Rgb,
    pub bg: Option<Rgb>,
    pub wide: bool,
    pub selected: bool,
    pub style: CellStyle,
    /// OSC 8 hyperlink URI (2d fills extraction). Task 0: always `None`.
    pub hyperlink_uri: Option<String>,
}

pub struct Frame {
    pub cols: u16,
    pub rows: u16,
    pub default_fg: Rgb,
    pub default_bg: Rgb,
    pub grid: Vec<FrameRow>,
    /// Cursor cell in viewport coords; `None` when hidden or scrolled out of
    /// the visible viewport. Task 0: always `None`; **2a** fills the real
    /// viewport-safe computation (folds review finding I5 — do NOT fabricate a
    /// top-left cursor via `unwrap_or(0)`).
    pub cursor: Option<CursorPos>,
}
```

### `engine/worker.rs` — **Modify**

- Rename `DA_DSR_CHANNEL_CAP` → `EGRESS_CHANNEL_CAP` (keep value `64`).
- `WorkerChannels`: rename `da_dsr_tx/rx` → `egress_tx/rx`; add `presentation_tx/rx`.
- `worker_channels()`: construct the presentation channel at `PRESENTATION_CHANNEL_CAP`.
- `spawn_worker`: add `egress_*` (renamed) + `presentation_tx: PresentationSender` params; pass `presentation_tx` into `VtEngine::new`.
- `handle_command` / `emit_da_dsr` → `emit_egress`; all `da_dsr` locals → `egress`.
- **`EngineCommand` enum is untouched here** (2a is its sole writer).

```rust
pub(crate) struct WorkerChannels {
    pub cmd_tx: Sender<EngineCommand>,
    pub cmd_rx: Receiver<EngineCommand>,
    pub egress_tx: Sender<Vec<u8>>,
    pub egress_rx: Receiver<Vec<u8>>,
    pub presentation_tx: Sender<EnginePresentationEvent>,
    pub presentation_rx: Receiver<EnginePresentationEvent>,
}
```

### `engine/inspect.rs` — **Modify**

Rename the `da_dsr`-named counter/method (`record_da_dsr` → `record_egress`, field likewise) for consistency with the channel rename. Update any inspect snapshot field name + its tests in the **same** commit. (Purely mechanical rename; no counter-semantics change.)

### `engine/vt.rs` — **Modify** (`vt.rs:34-73`)

Add key-encoder fields + `presentation_tx` param + bare `on_title_changed`:

```rust
pub struct VtEngine {
    terminal: Terminal<'static, 'static>,
    render_state: RenderState<'static>,
    rows: RowIterator<'static>,
    cells: CellIterator<'static>,
    cell_w_px: u32,
    cell_h_px: u32,
    reply_buffer: Rc<RefCell<Vec<u8>>>,
    #[expect(dead_code, reason = "worker invokes after take_replies in Task 4")]
    on_reply: OnReplyFn,
    #[expect(dead_code, reason = "2a fills key encoding")]
    key_encoder: libghostty_vt::key::Encoder<'static>,
    #[expect(dead_code, reason = "2a fills key encoding")]
    key_event: libghostty_vt::key::Event<'static>,
}

pub fn new(
    cfg: &EngineConfig,
    on_reply: impl FnMut(&[u8]) + 'static,
    presentation_tx: crossbeam_channel::Sender<super::presentation::EnginePresentationEvent>,
) -> Result<Self, EngineError> {
    use super::presentation::EnginePresentationEvent;
    let reply_buffer = Rc::new(RefCell::new(Vec::new()));
    let buf = Rc::clone(&reply_buffer);
    let mut terminal = Terminal::new(TerminalOptions {
        cols: cfg.cols,
        rows: cfg.rows,
        max_scrollback: cfg.max_scrollback,
    })?;
    terminal.on_pty_write(move |_term, data| {
        buf.borrow_mut().extend_from_slice(data);
    })?;
    // Bare title effect: enqueue the raw title. 2d adds sanitize + bound.
    let title_tx = presentation_tx.clone();
    terminal.on_title_changed(move |term| {
        let Ok(title) = term.title() else { return };
        let _ = title_tx.try_send(EnginePresentationEvent::TitleChanged(title.to_owned()));
    })?;
    // Note: `presentation_tx` is consumed by the title closure clone above.
    // 2b re-threads it for clipboard registration; do NOT store it yet.
    let _ = &presentation_tx;

    let key_encoder = libghostty_vt::key::Encoder::new()?;
    let key_event = libghostty_vt::key::Event::new()?;

    Ok(Self {
        terminal,
        render_state: RenderState::new()?,
        rows: RowIterator::new()?,
        cells: CellIterator::new()?,
        cell_w_px: cfg.cell_w_px,
        cell_h_px: cfg.cell_h_px,
        reply_buffer,
        on_reply: Box::new(on_reply) as OnReplyFn,
        key_encoder,
        key_event,
    })
}
```

`build_frame`: set `cursor: None` on the `Frame` literal, and `hyperlink_uri: None` on every `FrameCell` literal. **These two placeholder lines are the disjoint seams** — 2a replaces the `cursor:` line, 2d replaces each `hyperlink_uri:` line.

### `engine/handle.rs` — **Modify** (`handle.rs:26-83`)

`EngineHandle` struct: rename `da_dsr_rx` → `egress_rx`; add `presentation_rx` + `presentation_tx`. `spawn()`: destructure the new `WorkerChannels`, thread `presentation_tx` into `spawn_worker`, and store `presentation_rx` + a `presentation_tx` clone.

```rust
pub struct EngineHandle {
    cmd_tx: Sender<EngineCommand>,
    frame_slot: Arc<ArcSwapOption<Frame>>,
    frame_ready: Arc<AtomicBool>,
    waker: WakerSlot,
    egress_rx: Receiver<Vec<u8>>,
    presentation_rx: Receiver<EnginePresentationEvent>,
    #[expect(dead_code, reason = "2d/2b enqueue foreground-side presentation events")]
    presentation_tx: Sender<EnginePresentationEvent>,
    inspect: Arc<InspectShared>,
    join: Option<JoinHandle<()>>,
    #[cfg(test)]
    test_build_failures: Arc<AtomicUsize>,
    // NOTE: 2a adds `input_forwarder: Option<InputForwarder>` here — it is the
    // SOLE writer of this struct after Task 0, so no merge conflict with 2d.
}
```

Accessors `egress_rx()` (renamed from `da_dsr_rx()`), `presentation_rx()`, `enqueue_presentation()` are added by their owning plan (2a/2d) as **methods** (single-writer per method; method additions don't collide with 2a's struct-field addition).

### `lib.rs` — **Modify**

`TerminalTab` struct (`lib.rs:293-319`): add both interaction fields so each plan is a single writer to its own body, not the definition:

```rust
    /// IME preedit overlay text (2a fills). Foreground overlay, not engine state.
    ime_preedit: Option<String>,
    /// Monotonic id for typed host requests (2d fills). Starts at 0.
    next_host_request_id: u64,
```

Update **both** literal sites — `starting()` (`lib.rs:330-345`) and `with_engine_for_test()` (`lib.rs:358-386`) — with `ime_preedit: None, next_host_request_id: 0`.

`TerminalTab::render` (`lib.rs:887-897`): establish the combined hook-points as inert no-ops:

```rust
fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    self.sample_latest_frame_from_engine();
    self.drain_presentation_events(cx); // 2d fills the arms; Task 0: inert drain
    let title = self.presentation.identity_title.clone();
    let life = format!("{:?}", self.lifecycle);
    self.render
        .render_element(&self.focus_handle, &title, &life, window, cx)
        .on_key_down(cx.listener(Self::on_key_down))   // 2a fills body
        .on_mouse_down(gpui::MouseButton::Left, cx.listener(Self::on_mouse_down)) // 2d fills body
}
```

Stub methods (Task 0 bodies are empty; owning plan fills):

```rust
impl TerminalTab {
    // 2a fills: lower keystroke → gated KeyInput → enqueue_input.
    fn on_key_down(&mut self, _ev: &gpui::KeyDownEvent, _window: &mut Window, _cx: &mut Context<Self>) {}
    // 2d fills: hit-test → hyperlink gesture → OpenUrlRequest.
    fn on_mouse_down(&mut self, _ev: &gpui::MouseDownEvent, _window: &mut Window, _cx: &mut Context<Self>) {}
    // 2d fills the match arms; Task 0 drains-and-discards to keep the channel from backing up.
    fn drain_presentation_events(&mut self, _cx: &mut Context<Self>) {
        let Some(engine) = self.runtime.as_ref().and_then(|r| r.engine.as_ref()) else { return };
        while engine.presentation_rx().try_recv().is_ok() {}
    }
}
```

*(If `render_element`'s `Div` return type or `on_mouse_down`/`KeyDownEvent`/`MouseDownEvent` paths need a different exact gpui 0.2.2 signature, use the real one — verify against `gpui-0.2.2/src/{elements/div.rs,interactive.rs}`. The point is the hook-points exist and are inert.)*

### Call-site + fixture fan-out — **Modify** (mechanical)

Every one of these must be updated so the crate compiles and existing tests stay green:

- **`VtEngine::new` call sites** gain the `presentation_tx` arg. Production: `spawn_worker`. Tests/helpers: any `VtEngine::new(&cfg, |_| {})` in `vt.rs`, `reconnect_seed.rs`, and any new `presentation.rs` test → `let (tx, _rx) = crossbeam_channel::bounded(1); VtEngine::new(&cfg, |_| {}, tx)`.
- **`Frame { … }` literals** gain `cursor: None`: `render/fixtures.rs`, `render/paint.rs` tests (`paint.rs:571` area), any in `lib.rs`/`reconnect_seed.rs` tests.
- **`FrameCell { … }` literals** gain `hyperlink_uri: None`: `render/fixtures.rs` (all builders, incl. the closure at `fixtures.rs:60`), `render/paint.rs` (`paint.rs:571`), any others.
- **`WorkerChannels` destructures** (`handle.rs:48-53`) use the new field names + bind the presentation pair.
- **`engine/mod.rs`**: `mod presentation;` + `pub use presentation::{ClipboardLocation, ClipboardMimePart, EnginePresentationEvent, MAX_REPORTED_TITLE_CHARS};` and re-export `CursorPos` from `frame`.

---

## Steps

- [ ] **Step 1: Create `engine/presentation.rs`** with the types + constants above; add `mod presentation;` + `pub use` to `engine/mod.rs`; re-export `CursorPos`.

- [ ] **Step 2: `frame.rs`** — add `CursorPos`, `Frame.cursor`, `FrameCell.hyperlink_uri`.

- [ ] **Step 3: `worker.rs` + `inspect.rs`** — egress rename (const/channels/`emit_egress`/inspect counter) + add presentation channel to `WorkerChannels`/`worker_channels()`/`spawn_worker` + thread `presentation_tx` into `VtEngine::new`.

- [ ] **Step 4: `vt.rs`** — add `key_encoder`/`key_event` fields + `presentation_tx` param + bare `on_title_changed`; set `cursor: None` + `hyperlink_uri: None` placeholders in `build_frame`.

- [ ] **Step 5: `handle.rs`** — rename `egress_rx`, add `presentation_rx`/`presentation_tx` fields + `spawn()` wiring; rename the `da_dsr_rx()` accessor to `egress_rx()`; add a minimal `presentation_rx()` accessor (needed by the Task-0 inert drain).

- [ ] **Step 6: `lib.rs`** — add `ime_preedit`/`next_host_request_id` fields + update both literals; add the render hook-points + the three inert stub methods.

- [ ] **Step 7: Fixture/call-site fan-out** — update every `VtEngine::new` call, `Frame`/`FrameCell` literal, and `WorkerChannels` destructure per the fan-out list.

- [ ] **Step 8: Compile + gate (no `tail`)**

```bash
cargo build -p lens-terminal --all-targets --features test-util,live-tests
cargo test -p lens-terminal --features test-util --lib
cargo clippy -p lens-terminal --all-targets --features test-util,live-tests -- -D warnings
cargo fmt --all -- --check
```
Expected: compiles; **all existing tests pass unchanged**; clippy clean. If any existing test now fails, the change was not inert — fix the placeholder, don't edit the test's intent.

- [ ] **Step 9: Commit**

```bash
git add crates/lens-terminal/src/engine/ crates/lens-terminal/src/lib.rs crates/lens-terminal/src/render/
git commit -m "$(cat <<'EOF'
feat(lens-terminal): Slice-2 shared foundation skeleton (egress rename, presentation channel, cursor/hyperlink frame fields, render hook-points)

Merge-seam contract for 2a∥2d: pre-declares every shared field/param/literal so
each parallel plan is a single writer to disjoint bodies. All placeholders inert.
EOF
)"
```

---

## Handoff to 2a / 2d (what each fills — DO here nothing they own)

| Shared definition (declared in Task 0) | 2a fills | 2d fills |
| --- | --- | --- |
| `EngineHandle` struct | `input_forwarder` field + `enqueue_input`/`egress_rx()` bodies | `presentation_rx()`/`enqueue_presentation()` methods |
| `EngineCommand` enum | Key/Focus/LocalScroll variants + arms | — |
| `VtEngine` key fields | `encode_key` / `set_options_from_terminal` usage | — |
| `on_title_changed` closure body | — | sanitize + bound |
| `build_frame` `cursor:` line | real viewport-safe cursor (I5) | — |
| `build_frame` `hyperlink_uri:` line | — | real `GridRef::hyperlink_uri` extraction |
| `render` `on_key_down` body | keystroke → gated `KeyInput` | — |
| `render` `on_mouse_down` body | — | hit-test → `OpenUrlRequest` |
| `drain_presentation_events` arms | — | title→`reported_title`, hyperlink, clipboard |
| `TerminalTab.ime_preedit` | preedit overlay | — |
| `TerminalTab.next_host_request_id` | — | host-request id minting |

**Merge expectation after Task 0:** 2a and 2d touch **disjoint** definitions and **disjoint lines** of the two shared functions (`build_frame`, `render`). The final merge is additive; run the full gate on the merged tree regardless.
