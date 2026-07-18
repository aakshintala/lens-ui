# Terminal Slice 2d — Output-Side OSC Presentation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship terminal→host presentation effects for titles (full) and hyperlinks (click → typed host request), building the presentation skeleton (types, channel, `on_title_changed`, `FrameCell.hyperlink_uri`, render hooks) so Slice 2b can later register OSC 52 without redesigning the channel.

**Architecture:** 2d lands **serially, after 2a**, on `terminal-ws` — it edits 2a's committed code (no worktree, no merge). This plan **declares and wires** the entire presentation surface AND fills its bodies: create `engine/presentation.rs` (types + constants), add the presentation channel to `WorkerChannels`/`worker_channels()`/`spawn_worker`/`EngineHandle`, add the `presentation_tx` param to `VtEngine::new` (+ call-site fan-out), register `on_title_changed` (sanitize/bound inside, latest-title semantics), add `FrameCell.hyperlink_uri` + real OSC 8 extraction (interned URIs), add `TerminalTab.next_host_request_id` + the `on_mouse_down`/`drain_presentation_events` render hooks, click-only URL validation + gesture → `OpenUrlRequest`, and presentation-path inspect/benches. **Do not** register `on_clipboard_write` here — 2b owns that (pre-clone cap + policy). Demo host DENYs/no-ops any clipboard request; never auto-allows.

**Tech Stack:** gpui `EventEmitter`, `libghostty-vt` (`Terminal::title`, `on_title_changed`, `GridRef::hyperlink_uri`), `crossbeam-channel`, `url` (`url::Url` — already in `crates/lens-terminal/Cargo.toml` as `url = "2"`; reuse it).

## Serial position (2d is SECOND — lands after 2a; declares + fills its own surface)

Prerequisite: **2a is committed on `terminal-ws`.** 2d edits 2a's real code directly (no worktree, no merge, no Task 0). 2d **declares AND fills** the entire presentation surface — nothing is pre-declared for it. `VtEngine::new` currently has 2a's arity `(cfg, on_reply)`; **2d adds the `presentation_tx` param** and updates every call site (production + fixtures 2a left).

| Surface 2d declares here | What 2d fills |
| --- | --- |
| `engine/presentation.rs` (**Create**): `EnginePresentationEvent` / `ClipboardLocation` / `ClipboardMimePart` + `PRESENTATION_CHANNEL_CAP=64` / `MAX_REPORTED_TITLE_CHARS=512` / `MAX_HYPERLINK_URI_BYTES=8192` | `sanitize_reported_title`, `validate_open_url`, plain-URL scan helpers |
| Presentation channel in `WorkerChannels` / `worker_channels()` / `spawn_worker` / `EngineHandle` + `presentation_tx` param on `VtEngine::new` | `enqueue_presentation()` / `presentation_rx()` methods; latest-title slot |
| `on_title_changed` registration in `VtEngine::new` | Sanitize/bound **inside** the closure; latest-title slot |
| `FrameCell.hyperlink_uri: Option<Arc<str>>` (declare directly as `Arc<str>`; fixtures get `None`) | Real extraction on the `hyperlink_uri:` line via `GridRef::hyperlink_uri` |
| `TerminalTab.next_host_request_id: u64` + both literal sites; `on_mouse_down` + `drain_presentation_events` on the render div | `on_mouse_down` body + drain arms; mint request ids |

**2d must not touch** 2a-owned surface: `EngineCommand` input variants, the egress rename, `Frame.cursor`, `key_encoder`/`key_event`, `ime_preedit`, `on_key_down`. Edit `build_frame` / `render` only to **add** 2d's lines next to 2a's.

## Global Constraints

- **Per-task `lens-terminal` clippy MUST include `--features test-util` (and `live-tests` when touching live hooks):** e.g. `cargo clippy -p lens-terminal --all-targets --features test-util,live-tests -- -D warnings`. Workspace gate remains `cargo clippy --workspace --all-targets -- -D warnings`.
- **Never pipe the gate / clippy / test output through `tail`.**
- **`cargo test` takes ONE positional filter** — never pass multiple test-name args; run separate commands or one shared prefix.
- **Effect callbacks never block:** inside `on_*`, copy payload → owned data → non-blocking enqueue/slot update → return; never `recv`, never await host policy, never take a GPUI lock.
- **`MAX_REPORTED_TITLE_CHARS = 512`:** sanitize then bound. Order is **TRIM (ASCII whitespace) THEN truncate** to 512 Unicode scalars. Strip C0/DEL/C1 before trim. Justification: well above typical shell/OSC titles; the binding does not cap (`Terminal::title()`); Lens policy owns the bound so `reported_title` cannot grow without limit. (Distinct from stable `identity_title`.)
- **Latest-title semantics (not drop-newest):** a dedicated latest-title slot (overwrite) + non-blocking wake so the freshest title always wins — including while the tab is hidden / the presentation channel is full. Do **not** rely on drop-newest-on-full for titles.
- **`PRESENTATION_CHANNEL_CAP = 64`:** match egress. Hyperlink (and later clipboard) floods must not stall `vt_write`. Titles prefer the latest-title slot; channel `Full` must not leave a stale title permanently.
- **`MAX_HYPERLINK_URI_BYTES = 8192`:** URI extraction buffer / reject larger URIs. On `Error::OutOfSpace { required }`, grow **once** only if `required` is **strictly greater** than current `buf.len()` and `<= MAX_HYPERLINK_URI_BYTES`; reject non-growing / zero `required` (prevents infinite loops).
- **Hyperlink storage:** intern URIs (`Arc<str>` or id table) — not a fresh `String` per cell. Minimize `Terminal::grid_ref` lookups (`terminal.rs:362-377` warns against render-loop use). Prefer one lookup per contiguous OSC-8 run / unique URI, not per cell when the URI is unchanged.
- **URL validation:** use `url::Url`. Reject raw surrounding whitespace, **all** controls/whitespace, and backslashes **before** parse. Require exact `http` / `https` scheme + non-empty `host_str()`. Adversarial tests must cover `data:`, `file:`, spaces, backslash, `https://#frag`, `https://?x`, `http://`, malformed host.
- **Click-only in 2d:** implement and claim **primary-button click** (`on_mouse_down`) → `OpenUrlRequest`. **Defer hover** (cursor/highlight) to a Slice-2 follow-up — do not claim hover.
- **OSC 52 registration is 2b's job** — 2d delivers only the already-declared `EnginePresentationEvent::ClipboardWrite` variant. Demo must **Deny / no-op** clipboard requests, never auto-allow. Do not claim `Busy` backpressure for OSC 52 — the binding documents that OSC 52 ignores callback results (`terminal.rs:1345-1348`).
- **`TerminalTab::presentation()` returns an owned `Presentation`** (`lib.rs:394-397`), not a reference — clone/own wording accordingly.
- **No Ghostty type escapes the engine boundary** — Lens-owned presentation types / `FrameCell` fields only.
- **`#[gpui::test]` / `NoopTextSystem` false-greens hit-testing** — click claims go in `tests/presentation_realwindow.rs` (`harness = false`, `test-util`), not `#[gpui::test]`.
- Ground truth: `docs/specs/2026-07-17-terminal-slice-2-interaction-design.md` (2d + command seam + render-path constraint); **2a's committed code** on `terminal-ws`; real APIs in `lib.rs`, `policy.rs`, `frame.rs`, `vt.rs`, `worker.rs`, `vendor/.../terminal.rs`, `vendor/.../screen.rs`.

## Deferred (out of this plan)

OSC progress reports (`CONEMU_PROGRESS_REPORT`, OSC 9;4) and desktop notifications (`SHOW_DESKTOP_NOTIFICATION`, OSC 9 / OSC 777) are **out of scope**. A 2026-07-17 read-only FFI spike verified against the vendored `bindings.rs` that `ghostty_osc_command_data`'s `OscCommandData` selector enum exposes exactly one real kind (`CHANGE_WINDOW_TITLE_STR`) and there is no matching `on_*` terminal callback — the parser recognizes the command *type* but yields no payload. Do **not** plan progress/notification features, FFI accessors, or a raw-byte OSC tap. The parent completion matrix and Open-contract-gaps already defer them. `TerminalTab::presentation()` keeps its `progress: Option<Progress>` field but leaves it **unpopulated**. Nothing else about progress or notifications in this plan.

**Also deferred from 2d (accepted review / boundary):**

- **Hover** hyperlink affordance (cursor / highlight) — Slice-2 follow-up; 2d is click-only.
- **`on_clipboard_write` registration**, pre-clone decoded-byte cap, allow/deny policy, copy notice, cap±1 tests — **Slice 2b** (see Task 5 note).

## Serial-seam note (2a → 2d)

2d lands on 2a's committed code and edits **disjoint lines** of the shared functions:

| Region | Owner |
| --- | --- |
| `EngineCommand` variants + `handle_command` arms; Feed fairness; `Stop`; egress input forwarder; egress rename; `Frame.cursor`; `build_frame` `cursor:` line; `on_key_down`; `ime_preedit` | **2a (already landed)** |
| `presentation.rs`; presentation channel wiring; `VtEngine::new` `presentation_tx` param; title-closure sanitize/bound + latest-title; `FrameCell.hyperlink_uri` + `build_frame` `hyperlink_uri:` line; `on_mouse_down` body; `drain_presentation_events`; `enqueue_presentation()`; `next_host_request_id` | **2d (this plan)** |

Do **not** touch 2a-owned definitions. Do **not** plan input/keys/mouse-reporting/selection/paste/copy (2a/2b/2c).

## File Structure

| File | Responsibility |
| --- | --- |
| `crates/lens-terminal/src/engine/presentation.rs` | **Create.** Types (`EnginePresentationEvent`, `ClipboardLocation`, `ClipboardMimePart`) + constants (`PRESENTATION_CHANNEL_CAP`, `MAX_REPORTED_TITLE_CHARS`, `MAX_HYPERLINK_URI_BYTES`) + `sanitize_reported_title`, `validate_open_url`, plain-URL cell-index scan helpers + unit tests. |
| `crates/lens-terminal/src/engine/mod.rs` | **Modify.** `mod presentation;` + `pub use` presentation types. |
| `crates/lens-terminal/src/engine/worker.rs` | **Modify.** Add presentation channel to `WorkerChannels`/`worker_channels()`; thread `presentation_tx` into `spawn_worker` → `VtEngine::new`. |
| `crates/lens-terminal/src/engine/vt.rs` | **Modify.** Add `presentation_tx` param to `VtEngine::new`; register + sanitize `on_title_changed`; add + fill `hyperlink_uri:` extraction (intern + safe grow); latest-title slot wiring. |
| `crates/lens-terminal/src/engine/frame.rs` | **Modify.** Add `FrameCell.hyperlink_uri: Option<Arc<str>>` (fixtures get `None`). |
| `crates/lens-terminal/src/engine/handle.rs` | **Modify.** Add presentation `_rx`/`_tx` fields + `spawn()` wiring; `enqueue_presentation()`; `presentation_rx()`; latest-title slot accessors if held on the handle. |
| `VtEngine::new` call-site fan-out | **Modify.** Every `VtEngine::new(&cfg, on_reply)` (vt.rs tests, `reconnect_seed.rs`, benches) gains a throwaway `presentation_tx`: `let (tx,_rx)=crossbeam_channel::bounded(1); VtEngine::new(&cfg, on_reply, tx)`. |
| `crates/lens-terminal/src/engine/inspect.rs` | **Modify.** Presentation-path counters (titles applied, hyperlink opens, drops / slot overwrites). |
| `crates/lens-terminal/src/lib.rs` | **Modify.** Fill `drain_presentation_events` arms; fill `on_mouse_down`; `TerminalEvent` / `TerminalHostEvent` for OpenUrl; mint `next_host_request_id`. |
| `crates/lens-terminal/src/hit_test.rs` | **Create.** Pure `pixel_to_cell` + `uri_for_gesture` (OSC 8 field or plain-URL scan). |
| `crates/lens-terminal/benches/engine.rs` | **Modify.** Callback-throughput + dense-hyperlink-frame benches. |
| `crates/lens-terminal/Cargo.toml` | **Modify.** Add `[[test]] name = "presentation_realwindow"` (`harness = false`, `required-features = ["test-util"]`). Confirm `url = "2"` present. |
| `crates/lens-terminal/tests/presentation_realwindow.rs` | **Create.** Real-window harness for **click** → `OpenUrlRequest` (not hover; not `#[gpui::test]`). |
| `crates/lens-terminal-demo/src/main.rs` | **Modify.** Subscribe to `TerminalEvent`; allow/log `OpenUrlRequest`; **Deny/no-op** any clipboard request. |
| `docs/superpowers/plans/2026-07-17-terminal-slice-2d-presentation.md` | This plan. |

---

### Task 1: presentation channel foundation + `EngineHandle` methods + drain arms + latest-title slot

**Step 0 (foundation — declare the presentation surface; do first):**
- **Create `engine/presentation.rs`** with the types + constants (from the "Serial position" table): `EnginePresentationEvent { TitleChanged(String), HyperlinkOpen { url: String }, ClipboardWrite { location: ClipboardLocation, contents: Vec<ClipboardMimePart> } }`, `ClipboardLocation { Standard, Selection, Primary }`, `ClipboardMimePart { mime: String, data: String }`, `PRESENTATION_CHANNEL_CAP: usize = 64`, `MAX_REPORTED_TITLE_CHARS: usize = 512`, `MAX_HYPERLINK_URI_BYTES: usize = 8192`. Add `mod presentation;` + `pub use` to `engine/mod.rs`.
- **Wire the channel:** `WorkerChannels` gains `presentation_tx/rx`; `worker_channels()` constructs it at `PRESENTATION_CHANNEL_CAP`; `spawn_worker` gains a `presentation_tx: Sender<EnginePresentationEvent>` param and threads it into `VtEngine::new`; `EngineHandle` gains `presentation_rx` + a `presentation_tx` clone, wired in `spawn()`.
- **`VtEngine::new`:** add the `presentation_tx` param (2a left arity `(cfg, on_reply)`; 2d makes it `(cfg, on_reply, presentation_tx)`). Register a **bare** `on_title_changed` that `try_send`s `TitleChanged(raw)` (Task 2 wraps it with sanitize + latest-title slot). Update **every** `VtEngine::new` call site (production `spawn_worker`; test/fixture/bench sites 2a left) with a throwaway `presentation_tx`.

**Files:**
- Create: `crates/lens-terminal/src/engine/presentation.rs`
- Modify: `crates/lens-terminal/src/engine/mod.rs` (`mod` + `pub use`)
- Modify: `crates/lens-terminal/src/engine/worker.rs` (channel + `spawn_worker` param)
- Modify: `crates/lens-terminal/src/engine/vt.rs` (`VtEngine::new` param + bare `on_title_changed`; latest-title slot Task 2 fills sanitize)
- Modify: `crates/lens-terminal/src/engine/handle.rs` (presentation fields + `spawn()` wiring; `presentation_rx()`/`enqueue_presentation()` methods)
- Modify: `crates/lens-terminal/src/lib.rs` (`drain_presentation_events` method + call it from `render`)
- Modify: `VtEngine::new` call sites (fixtures/benches/reconnect_seed)
- Test: `handle.rs` / `presentation.rs` / `lib.rs`

**Interfaces:**
- Consumes: the presentation `_rx`/`_tx` fields declared in Step 0; bare title events.
- Produces:
  - `EngineHandle::presentation_rx(&self) -> &Receiver<EnginePresentationEvent>` (add it; Step 0 declared the field)
  - `EngineHandle::enqueue_presentation(&self, ev: EnginePresentationEvent) -> Result<(), FeedError>` (`try_send`)
  - Latest-title slot: always overwrite with the newest title string; non-blocking wake so FG drain sees it even when the channel is full / tab was hidden
  - Drain maps `TitleChanged` / slot → `presentation.reported_title` + `cx.emit(TerminalEvent::PresentationChanged)`; stubs for `HyperlinkOpen` (Task 4) and `ClipboardWrite` (no-op / Task 5 note)

- [ ] **Step 1: Write the failing handle + latest-title tests**

```rust
#[test]
fn engine_handle_exposes_presentation_rx_after_title_feed() {
    use crate::engine::presentation::EnginePresentationEvent;
    use std::time::Duration;
    let h = EngineHandle::spawn(EngineConfig {
        cols: 40,
        rows: 8,
        max_scrollback: 32,
        cell_w_px: 8,
        cell_h_px: 16,
    });
    h.feed(b"\x1b]2;ViaHandle\x1b\\".to_vec()).unwrap();
    // Prefer sampling the latest-title slot if exposed; else recv on channel.
    let title = h
        .take_latest_title()
        .or_else(|| {
            h.presentation_rx()
                .recv_timeout(Duration::from_secs(2))
                .ok()
                .and_then(|ev| match ev {
                    EnginePresentationEvent::TitleChanged(t) => Some(t),
                    _ => None,
                })
        })
        .expect("presentation title");
    assert_eq!(title, "ViaHandle");
    h.stop();
}

#[test]
fn latest_title_wins_when_channel_full() {
    // Fill the presentation channel with non-title events (or TitleChanged),
    // then feed a final OSC title. After wake/drain, reported/latest must be
    // the FINAL title — not a stale earlier one.
    let (tx, rx) = crossbeam_channel::bounded(1);
    // Construct VtEngine with presentation_tx + assert final title via slot.
    // Exact setup mirrors the new VtEngine::new arity (cfg, on_reply, presentation_tx).
    let _ = (tx, rx);
    // Implement against the real latest-title API chosen in Step 3.
}
```

- [ ] **Step 2: Run tests — expect FAIL**

```bash
cargo test -p lens-terminal --features test-util engine_handle_exposes_presentation_rx_after_title_feed -- --nocapture
```
```bash
cargo test -p lens-terminal --features test-util latest_title_wins_when_channel_full -- --nocapture
```
Expected: FAIL — missing `enqueue_presentation` / latest-title slot / drain apply.

- [ ] **Step 3: Minimal implementation**

```rust
pub fn presentation_rx(&self) -> &Receiver<EnginePresentationEvent> {
    &self.presentation_rx
}

pub fn enqueue_presentation(
    &self,
    ev: EnginePresentationEvent,
) -> Result<(), FeedError> {
    self.presentation_tx.try_send(ev).map_err(|e| match e {
        TrySendError::Full(_) => FeedError::Full,
        TrySendError::Disconnected(_) => FeedError::Stopped,
    })
}
```

Latest-title design (pin one; do not invent a second):

```rust
// Shared with the title callback (Arc<ArcSwapOption<String>> or Mutex<Option<String>>).
// In on_title_changed (Task 2 wraps with sanitize): always store the newest title
// into the slot (overwrite), then try_send(TitleChanged) as a wake hint if desired,
// and wake the FG sampler. If try_send is Full, the slot still holds the truth.
```

Add the `drain_presentation_events` method + arms (and call it from `render`):

```rust
fn drain_presentation_events(&mut self, cx: &mut Context<Self>) {
    let Some(engine) = self.runtime.as_ref().and_then(|r| r.engine.as_ref()) else {
        return;
    };
    // 1) Apply latest-title slot first (freshest wins).
    if let Some(title) = engine.take_latest_title() {
        apply_title_to_presentation(&mut self.presentation, title);
        cx.emit(TerminalEvent::PresentationChanged);
    }
    // 2) Drain channel events.
    while let Ok(ev) = engine.presentation_rx().try_recv() {
        match ev {
            EnginePresentationEvent::TitleChanged(title) => {
                // Slot is authoritative; still apply if slot empty / as wake coalescing.
                apply_title_to_presentation(&mut self.presentation, title);
                cx.emit(TerminalEvent::PresentationChanged);
            }
            EnginePresentationEvent::HyperlinkOpen { .. } => {
                // Task 4 fills.
            }
            EnginePresentationEvent::ClipboardWrite { .. } => {
                // 2b owns policy/registration. 2d: no-op (do not emit Allow).
            }
        }
    }
}
```

Add the drain call to `render` (alongside 2a's `sample_latest_frame_from_engine()`) and to the `wake_rx` / sample path as needed so hidden→visible still applies the freshest title.

- [ ] **Step 4: Run tests — expect PASS**

```bash
cargo test -p lens-terminal --features test-util engine_handle_exposes_presentation_rx_after_title_feed -- --nocapture
```
```bash
cargo test -p lens-terminal --features test-util latest_title_wins_when_channel_full -- --nocapture
```
```bash
cargo test -p lens-terminal --features test-util --lib -- --nocapture
```
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/lens-terminal/src/engine/handle.rs \
  crates/lens-terminal/src/engine/vt.rs \
  crates/lens-terminal/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(terminal-2d): presentation handle methods, drain arms, latest-title slot

EOF
)"
```

---

### Task 2: Titles — sanitize/bound inside existing closure → `reported_title`

**Files:**
- Modify: `crates/lens-terminal/src/engine/presentation.rs` (add `sanitize_reported_title`)
- Modify: `crates/lens-terminal/src/engine/vt.rs` (**inside** the `on_title_changed` body registered in Task 1 — do not re-register)
- Modify: `crates/lens-terminal/src/lib.rs` (`apply_title_to_presentation`; never writes `identity_title`)
- Test: `presentation.rs` unit tests + OSC integration

**Interfaces:**
- Consumes: existing bare callback; `Presentation::{identity_title, reported_title}`.
- Produces:
  - `pub fn sanitize_reported_title(raw: &str) -> Option<String>`
    1. Strip C0 (`0x00..=0x1F`), DEL (`0x7F`), C1 (`0x80..=0x9F`)
    2. **Trim leading/trailing ASCII whitespace** (`' ' | '\t' | '\n' | '\r' | '\x0C'`) — not Unicode `trim()`
    3. **Then** truncate to `MAX_REPORTED_TITLE_CHARS` Unicode scalars
    4. Return `None` if empty after sanitize (clears `reported_title`)
  - Invariant: OSC title path **never** assigns `presentation.identity_title`

- [ ] **Step 1: Write failing sanitize + identity-stability tests**

```rust
#[test]
fn sanitize_strips_controls_and_bounds_length() {
    let dirty = format!("ab\u{0007}cd{}", "X".repeat(600));
    let clean = sanitize_reported_title(&dirty).expect("Some");
    assert!(!clean.contains('\u{0007}'));
    assert_eq!(clean.chars().count(), MAX_REPORTED_TITLE_CHARS);
    assert!(clean.starts_with("abcd"));
}

#[test]
fn sanitize_trims_ascii_whitespace_before_truncate() {
    // Leading/trailing ASCII spaces must be removed BEFORE the 512-scalar bound.
    let mut s = String::from("   ");
    s.push_str(&"Y".repeat(510));
    s.push_str("   ");
    let clean = sanitize_reported_title(&s).expect("Some");
    assert_eq!(clean.chars().count(), 510);
    assert!(!clean.starts_with(' '));
    assert!(!clean.ends_with(' '));
}

#[test]
fn sanitize_empty_after_controls_returns_none() {
    assert_eq!(sanitize_reported_title("\u{0007}\u{001b}"), None);
    assert_eq!(sanitize_reported_title("   "), None);
}

#[test]
fn apply_title_event_updates_reported_only() {
    let mut presentation = Presentation {
        lifecycle: Lifecycle::Live,
        access: AccessMode::Write,
        identity_title: "main:workspace".into(),
        reported_title: None,
        progress: None,
        output_gap: false,
        detached_detail: None,
        reattach_available: false,
    };
    apply_title_to_presentation(&mut presentation, "Shell Title".into());
    assert_eq!(presentation.identity_title, "main:workspace");
    assert_eq!(presentation.reported_title.as_deref(), Some("Shell Title"));
}
```

- [ ] **Step 2: Run — expect FAIL**

```bash
cargo test -p lens-terminal --features test-util sanitize_strips_controls_and_bounds_length -- --nocapture
```
```bash
cargo test -p lens-terminal --features test-util sanitize_trims_ascii_whitespace_before_truncate -- --nocapture
```
```bash
cargo test -p lens-terminal --features test-util apply_title_event_updates_reported_only -- --nocapture
```
Expected: FAIL — helpers missing / wrong trim-then-truncate order.

- [ ] **Step 3: Implement sanitize + edit existing closure body**

```rust
pub fn sanitize_reported_title(raw: &str) -> Option<String> {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        let cu = ch as u32;
        if cu <= 0x1F || cu == 0x7F || (0x80..=0x9F).contains(&cu) {
            continue;
        }
        out.push(ch);
    }
    let trimmed = out.trim_matches(|c: char| matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0C'));
    if trimmed.is_empty() {
        return None;
    }
    let bounded: String = trimmed.chars().take(MAX_REPORTED_TITLE_CHARS).collect();
    Some(bounded)
}
```

Inside the **existing** Task-1 closure (do not call `on_title_changed` again):

```rust
// was: try_send(TitleChanged(title.to_owned()))
let Ok(title) = term.title() else { return };
match sanitize_reported_title(title) {
    Some(clean) => {
        latest_title_slot.store(Some(Arc::new(clean.clone()))); // overwrite
        let _ = title_tx.try_send(EnginePresentationEvent::TitleChanged(clean));
        wake(); // non-blocking
    }
    None => {
        latest_title_slot.store(None);
        let _ = title_tx.try_send(EnginePresentationEvent::TitleChanged(String::new()));
        wake();
    }
}
```

**Pin empty-title signaling:** `TitleChanged("")` / empty slot means clear `reported_title` (`None`):

```rust
fn apply_title_to_presentation(presentation: &mut Presentation, title: String) {
    if title.is_empty() {
        presentation.reported_title = None;
    } else {
        presentation.reported_title = Some(title);
    }
    // NEVER touch presentation.identity_title here.
}
```

```rust
#[test]
fn osc2_title_is_sanitized_before_enqueue() {
    let (tx, rx) = crossbeam_channel::bounded(PRESENTATION_CHANNEL_CAP);
    let mut engine = VtEngine::new(&cfg(), |_| {}, tx).unwrap();
    engine.feed(b"\x1b]2;Hi\x07There\x1b\\");
    let ev = rx.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(
        ev,
        EnginePresentationEvent::TitleChanged("HiThere".into())
    );
}
```

- [ ] **Step 4: Run — expect PASS**

```bash
cargo test -p lens-terminal --features test-util sanitize_ -- --nocapture
```
```bash
cargo test -p lens-terminal --features test-util osc2_title -- --nocapture
```
```bash
cargo clippy -p lens-terminal --all-targets --features test-util,live-tests -- -D warnings
```
Expected: PASS / clippy clean.

- [ ] **Step 5: Commit**

```bash
git add crates/lens-terminal/src/engine/presentation.rs \
  crates/lens-terminal/src/engine/vt.rs \
  crates/lens-terminal/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(terminal-2d): sanitize and bound OSC titles into reported_title

EOF
)"
```

---

### Task 3: Fill `hyperlink_uri` extraction (intern + safe grow) + OSC 8

**Files:**
- Modify: `crates/lens-terminal/src/engine/frame.rs` (**add** `FrameCell.hyperlink_uri: Option<Arc<str>>`)
- Modify: `crates/lens-terminal/src/engine/vt.rs` (add the `hyperlink_uri:` line to every `FrameCell` literal in `build_frame` + fill extraction)
- Test: `vt.rs` OSC 8 integration tests
- Note: every existing `FrameCell { … }` literal (fixtures, paint tests) gains `hyperlink_uri: None` (`None` fits `Option<Arc<str>>`)

**Interfaces:**
- Consumes: `Cell::has_hyperlink`, `Terminal::grid_ref`, `GridRef::hyperlink_uri`.
- Produces:
  - Real `FrameCell.hyperlink_uri: Option<Arc<str>>`
  - Frame-local URI intern (reuse `Arc<str>` for identical URIs in one `build_frame`)
  - Reject non-growing `OutOfSpace { required }`

- [ ] **Step 1: Write failing OSC 8 frame tests**

```rust
#[test]
fn osc8_hyperlink_populates_frame_cell_uri() {
    let (tx, _rx) = crossbeam_channel::bounded(1);
    let mut e = VtEngine::new(&test_config(), |_| {}, tx).unwrap();
    e.feed(b"\x1b]8;;https://example.com/x\x1b\\link\x1b]8;;\x1b\\");
    let f = e.build_frame().unwrap();
    let cell = f.grid[0]
        .cells
        .iter()
        .find(|c| c.grapheme == "l")
        .expect("linked cell");
    assert_eq!(
        cell.hyperlink_uri.as_deref(),
        Some("https://example.com/x")
    );
}

#[test]
fn osc8_closer_clears_subsequent_cells() {
    let (tx, _rx) = crossbeam_channel::bounded(1);
    let mut e = VtEngine::new(&test_config(), |_| {}, tx).unwrap();
    e.feed(b"\x1b]8;;https://example.com\x1b\\L\x1b]8;;\x1b\\X");
    let f = e.build_frame().unwrap();
    let l = f.grid[0].cells.iter().find(|c| c.grapheme == "L").unwrap();
    let x = f.grid[0].cells.iter().find(|c| c.grapheme == "X").unwrap();
    assert_eq!(l.hyperlink_uri.as_deref(), Some("https://example.com"));
    assert_eq!(x.hyperlink_uri, None);
}
```

- [ ] **Step 2: Run — expect FAIL**

```bash
cargo test -p lens-terminal --features test-util osc8_hyperlink_populates_frame_cell_uri -- --nocapture
```
```bash
cargo test -p lens-terminal --features test-util osc8_closer_clears_subsequent_cells -- --nocapture
```
Expected: FAIL — field/extraction not present yet.

- [ ] **Step 3: Fill extraction on the `hyperlink_uri:` line**

```rust
use std::collections::HashMap;
use std::sync::Arc;
use super::presentation::MAX_HYPERLINK_URI_BYTES;

fn read_hyperlink_uri(
    terminal: &Terminal<'_, '_>,
    col: u16,
    row: u32,
    intern: &mut HashMap<Vec<u8>, Arc<str>>,
) -> Option<Arc<str>> {
    let grid_ref = terminal
        .grid_ref(Point::Viewport(PointCoordinate { x: col, y: row }))
        .ok()?;
    let mut buf = vec![0u8; 512];
    loop {
        match grid_ref.hyperlink_uri(&mut buf) {
            Ok(0) => return None,
            Ok(n) => {
                if n > MAX_HYPERLINK_URI_BYTES {
                    return None;
                }
                let bytes = &buf[..n];
                if let Some(existing) = intern.get(bytes) {
                    return Some(Arc::clone(existing));
                }
                let s = std::str::from_utf8(bytes).ok()?.to_owned();
                let arc: Arc<str> = Arc::from(s);
                intern.insert(bytes.to_vec(), Arc::clone(&arc));
                return Some(arc);
            }
            Err(libghostty_vt::error::Error::OutOfSpace { required }) => {
                // Reject non-growing / zero / over-cap — prevents infinite loops.
                if required <= buf.len() || required > MAX_HYPERLINK_URI_BYTES {
                    return None;
                }
                buf.resize(required, 0);
            }
            Err(_) => return None,
        }
    }
}
```

In `build_frame`, replace `hyperlink_uri: None` with extraction; **minimize** `grid_ref` calls — e.g. only when `raw.has_hyperlink()`, and reuse the previous cell's `Arc` when still in the same OSC-8 run if the binding allows detecting continuity; at minimum intern identical byte payloads via the `HashMap` above.

```rust
let mut uri_intern: HashMap<Vec<u8>, Arc<str>> = HashMap::new();
// ...
let hyperlink_uri = if raw.has_hyperlink().unwrap_or(false) {
    read_hyperlink_uri(&self.terminal, this_col, row_y, &mut uri_intern)
} else {
    None
};
```

Add `FrameCell.hyperlink_uri: Option<Arc<str>>` to `frame.rs`.

- [ ] **Step 4: Run — expect PASS**

```bash
cargo test -p lens-terminal --features test-util osc8_ -- --nocapture
```
```bash
cargo test -p lens-terminal --features test-util --lib -- --nocapture
```
Expected: PASS. If OSC 8 needs BEL instead of ST on this pin, try `\x07` and record the working sequence in the test comment.

- [ ] **Step 5: Commit**

```bash
git add crates/lens-terminal/src/engine/frame.rs \
  crates/lens-terminal/src/engine/vt.rs
git commit -m "$(cat <<'EOF'
feat(terminal-2d): extract interned OSC 8 hyperlink URIs into Frame cells

EOF
)"
```

---

### Task 4: URL validation (`url` crate) + plain-URL cell scan + click → host request + demo

**Files:**
- Modify: `crates/lens-terminal/src/engine/presentation.rs` (`validate_open_url`, `plain_url_covering_cell`)
- Create: `crates/lens-terminal/src/hit_test.rs`
- Modify: `crates/lens-terminal/src/lib.rs` (add `next_host_request_id: u64` field to `TerminalTab` + both literal sites; register + fill `on_mouse_down` on the render div; events; drain `HyperlinkOpen`; mint ids)
- Modify: `crates/lens-terminal/Cargo.toml` (`presentation_realwindow` test; confirm `url = "2"`)
- Create: `crates/lens-terminal/tests/presentation_realwindow.rs`
- Modify: `crates/lens-terminal-demo/src/main.rs`

**Interfaces:**
- Consumes: `FrameCell.hyperlink_uri`, `CellMetrics`, `EngineHandle::enqueue_presentation`.
- Produces:
  - ```rust
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub struct HostRequestId(pub u64);

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum HostRequestDecision {
        Allow,
        Deny,
    }

    // TerminalEvent:
    OpenUrlRequest { id: HostRequestId, url: String },

    // TerminalHostEvent:
    HostRequestResponse {
        id: HostRequestId,
        decision: HostRequestDecision,
    },
    ```
  - `pub fn validate_open_url(raw: &str) -> Option<String>` — see Step 3 contract
  - `pub fn plain_url_covering_cell(...)` — **cell/char index**, never byte offsets from `str::find` alone
  - `pixel_to_cell` / `uri_for_gesture`
  - **Click-only:** primary `on_mouse_down` (this plan's render hook) → validate → enqueue `HyperlinkOpen` → drain → `OpenUrlRequest`. Hover deferred.
  - Read-only tabs may still open URLs (local gesture, no PTY bytes). Do not auto-fire on OSC 8 alone.

- [ ] **Step 1: Write failing validation + hit-test + adversarial URL tests**

```rust
#[test]
fn validate_open_url_accepts_https_rejects_dangerous() {
    assert_eq!(
        validate_open_url("https://example.com/a"),
        Some("https://example.com/a".into())
    );
    assert_eq!(validate_open_url("javascript:alert(1)"), None);
    assert_eq!(validate_open_url("data:text/html,hi"), None);
    assert_eq!(validate_open_url("file:///etc/passwd"), None);
    assert_eq!(validate_open_url(" https://example.com"), None); // surrounding space
    assert_eq!(validate_open_url("https://example.com "), None);
    assert_eq!(validate_open_url("https://example.com/\r\nINJECT"), None);
    assert_eq!(validate_open_url(r"https://example.com\path"), None); // backslash
    assert_eq!(validate_open_url("https://#frag"), None); // no host
    assert_eq!(validate_open_url("https://?x"), None);
    assert_eq!(validate_open_url("http://"), None);
    assert_eq!(validate_open_url("ftp://example.com"), None);
}

#[test]
fn plain_url_covering_cell_uses_cell_index_not_bytes() {
    // Multibyte prefix: char/cell index ≠ byte offset.
    let row = "見 https://example.com/x";
    // '見' is one cell; space; then URL. Cell index of 'h' is 2.
    assert_eq!(
        plain_url_covering_cell(row, 2).as_deref(),
        Some("https://example.com/x")
    );
    assert_eq!(plain_url_covering_cell(row, 0), None);
}

#[test]
fn uri_for_gesture_prefers_osc8_field() {
    // Synthetic Frame with Arc<str> hyperlink_uri on cells — assert preference
    // over plain-URL scan.
}
```

- [ ] **Step 2: Run — expect FAIL**

```bash
cargo test -p lens-terminal --features test-util validate_open_url_accepts_https_rejects_dangerous -- --nocapture
```
```bash
cargo test -p lens-terminal --features test-util plain_url_covering_cell_uses_cell_index_not_bytes -- --nocapture
```
Expected: FAIL — functions missing / byte-index bug.

- [ ] **Step 3: Implement validation, cell-index scan, click gesture, demo**

```rust
use url::Url;

pub fn validate_open_url(raw: &str) -> Option<String> {
    // Reject BEFORE parse: surrounding whitespace, any control/whitespace, backslash.
    if raw.is_empty() || raw.len() > MAX_HYPERLINK_URI_BYTES {
        return None;
    }
    if raw.as_bytes().first().is_some_and(u8::is_ascii_whitespace)
        || raw.as_bytes().last().is_some_and(u8::is_ascii_whitespace)
    {
        return None;
    }
    if raw.chars().any(|c| {
        let u = c as u32;
        u <= 0x1F || u == 0x7F || c.is_whitespace() || c == '\\'
    }) {
        return None;
    }
    let parsed = Url::parse(raw).ok()?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return None;
    }
    let host = parsed.host_str()?;
    if host.is_empty() {
        return None;
    }
    Some(raw.to_owned())
}

/// Scan by Unicode scalar / cell index. Build a dense cell vector (one char per
/// cell) and search for `http://` / `https://` spans in cell space — never use
/// raw `str::find` byte offsets as `col`.
pub fn plain_url_covering_cell(row_text: &str, col: usize) -> Option<String> {
    let cells: Vec<char> = row_text.chars().collect();
    if col >= cells.len() {
        return None;
    }
    // Find candidate starts at cell indices where the next chars spell http(s)://
    let mut i = 0;
    while i < cells.len() {
        if starts_url_scheme_at(&cells, i) {
            let end = cells[i..]
                .iter()
                .position(|c| c.is_whitespace() || matches!(c, '"' | '\'' | ')' | '(' | '<' | '>'))
                .map(|rel| i + rel)
                .unwrap_or(cells.len());
            if col >= i && col < end {
                let url: String = cells[i..end].iter().collect();
                return validate_open_url(&url);
            }
            i = end.max(i + 1);
        } else {
            i += 1;
        }
    }
    None
}
```

`hit_test.rs`: `pixel_to_cell` + `uri_for_gesture` (prefer OSC 8 `Arc<str>`, else plain-URL on a dense cell row built from `FrameCell.col` → grapheme).

Register `.on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))` on the render div (next to 2a's `on_key_down`), and fill the **`on_mouse_down`** body (not mouse-up; not hover):

```rust
fn on_mouse_down(
    &mut self,
    event: &gpui::MouseDownEvent,
    _window: &mut Window,
    cx: &mut Context<Self>,
) {
    if event.button != gpui::MouseButton::Left {
        return;
    }
    let Some(frame) = self.render.latest_frame() else { return };
    let Some(metrics) = self.render.cell_metrics.clone() else { return };
    let Some(origin) = self.render.last_paint_origin else { return };
    let Some((col, row)) =
        crate::hit_test::pixel_to_cell(origin, &metrics, event.position, frame.cols, frame.rows)
    else {
        return;
    };
    let Some(url) = crate::hit_test::uri_for_gesture(frame.as_ref(), col, row) else {
        return;
    };
    let Some(engine) = self.runtime.as_ref().and_then(|r| r.engine.as_ref()) else {
        return;
    };
    let _ = engine.enqueue_presentation(EnginePresentationEvent::HyperlinkOpen { url });
    self.drain_presentation_events(cx);
}
```

If `TabRenderState` lacks `latest_frame()` / `last_paint_origin`, add thin accessors (set `last_paint_origin` in the canvas paint closure).

Drain `HyperlinkOpen`:

```rust
EnginePresentationEvent::HyperlinkOpen { url } => {
    if let Some(url) = crate::engine::presentation::validate_open_url(&url) {
        let id = HostRequestId(self.next_host_request_id);
        self.next_host_request_id = self.next_host_request_id.wrapping_add(1);
        cx.emit(TerminalEvent::OpenUrlRequest { id, url });
    }
}
```

**Realwindow test (click only):** mount tab / set frame with OSC-8 cells → dispatch left mouse-down at cell center → assert `OpenUrlRequest`. Do **not** assert hover.

Demo (`lens-terminal-demo`):

```rust
TerminalEvent::OpenUrlRequest { id, url } => {
    eprintln!("demo: OpenUrlRequest id={id:?} url={url}");
    this.update(cx, |tab, cx| {
        tab.on_host_event(
            TerminalHostEvent::HostRequestResponse {
                id: *id,
                decision: HostRequestDecision::Allow,
            },
            cx,
        );
    });
}
TerminalEvent::PresentationChanged => {
    // presentation() returns owned Presentation — clone is already done by the getter.
    let p = this.read(cx).presentation();
    eprintln!(
        "demo: presentation identity={} reported={:?}",
        p.identity_title, p.reported_title
    );
}
TerminalEvent::ClipboardWriteRequest { id, .. } => {
    // Never auto-allow. 2d demo: Deny / no-op. 2b owns real policy.
    eprintln!("demo: ClipboardWriteRequest id={id:?} → Deny (2d no-op)");
    this.update(cx, |tab, cx| {
        tab.on_host_event(
            TerminalHostEvent::HostRequestResponse {
                id: *id,
                decision: HostRequestDecision::Deny,
            },
            cx,
        );
    });
}
```

Note: `ClipboardWriteRequest` need not be emitted by 2d (registration deferred); if the variant exists for 2b forward-compat, the demo Deny arm documents the policy. Do not invent OSC-52 emission here.

- [ ] **Step 4: Run — expect PASS**

```bash
cargo test -p lens-terminal --features test-util validate_open_url -- --nocapture
```
```bash
cargo test -p lens-terminal --features test-util plain_url_covering -- --nocapture
```
```bash
cargo test -p lens-terminal --features test-util uri_for_gesture -- --nocapture
```
```bash
cargo test -p lens-terminal --features test-util --test presentation_realwindow -- --nocapture
```
```bash
cargo clippy -p lens-terminal --all-targets --features test-util,live-tests -- -D warnings
```
```bash
cargo check -p lens-terminal-demo
```
Expected: unit tests PASS; realwindow PASS on macOS (document skip-on-non-mac if xtask gates like `render_realwindow`).

- [ ] **Step 5: Commit**

```bash
git add crates/lens-terminal/src/engine/presentation.rs \
  crates/lens-terminal/src/hit_test.rs \
  crates/lens-terminal/src/lib.rs \
  crates/lens-terminal/Cargo.toml \
  crates/lens-terminal/tests/presentation_realwindow.rs \
  crates/lens-terminal-demo/src/main.rs
git commit -m "$(cat <<'EOF'
feat(terminal-2d): click-validated hyperlink gestures into OpenUrlRequest

EOF
)"
```

---

### Task 5: OSC 52 / clipboard — deferred to 2b (documentation only)

**Do not implement `on_clipboard_write` registration or payload cloning in 2d.**

2d's Task 1 declares `EnginePresentationEvent::ClipboardWrite { location, contents }`. Emitting it by registering the Ghostty callback and cloning every MIME part here would defeat Slice 2b's required **cap BEFORE cloning** (`interaction-design.md` OSC-52 policy) and would tempt demo auto-Allow.

**2b owns:**

1. Re-thread `presentation_tx` into `on_clipboard_write` registration at `VtEngine` construction (2d's Task 1 left a note that 2b re-threads; the title clone may already consume one clone).
2. Pre-clone decoded-byte cap; drop/deny over-cap **before** allocating owned MIME copies.
3. Allow/deny / allow-once/session policy + copy notice + cap−1/cap/cap+1 tests.
4. Mapping drain → `TerminalEvent::ClipboardWriteRequest` (if not already a no-op stub).

**2d delivers:** the typed variant on the shared channel enum only. Drain arm stays no-op (or Deny-only if a request somehow appears). Demo Deny/no-op (Task 4). No OSC-52 feed tests in this plan. No claim that `ClipboardWriteError::Busy` is observed by OSC 52 (binding ignores callback results).

- [ ] **Step 1: Confirm no clipboard registration in 2d tree**

```bash
rg -n "on_clipboard_write" crates/lens-terminal/src/
```
Expected: no matches in 2d-owned code (or only comments pointing to 2b).

- [ ] **Step 2: No commit required** unless a comment cross-link is added in `presentation.rs` / `vt.rs`. If adding a comment only:

```bash
git add crates/lens-terminal/src/engine/presentation.rs
git commit -m "$(cat <<'EOF'
docs(terminal-2d): note OSC 52 registration and policy owned by Slice 2b

EOF
)"
```

---

### Task 6: Inspect counters + callback-throughput + dense-hyperlink-frame benches

**Files:**
- Modify: `crates/lens-terminal/src/engine/inspect.rs` (presentation counters + ring events)
- Modify: `crates/lens-terminal/src/engine/vt.rs` / `handle.rs` / `lib.rs` (record on title apply / hyperlink open / slot overwrite)
- Modify: `crates/lens-terminal/benches/engine.rs` (new benches; `VtEngine::new` call sites use a throwaway `presentation_tx` — Task 1 added the arity)

**Interfaces:**
- Produces (names illustrative — match existing inspect style):
  - Counters: e.g. `titles_applied`, `title_slot_overwrites`, `hyperlink_opens`, `presentation_channel_full_drops`
  - Ring kinds when inspect enabled (zero-cost when off)
  - Bench: `presentation_title_callback_throughput` — feed many OSC 2 titles; measure callback→slot path
  - Bench: `engine_frame_build_dense_hyperlink_200x50` — full grid under one OSC 8 URI; guard 1c frame-build verdict

- [ ] **Step 1: Write failing inspect snapshot expectations + bench stubs**

Add a unit test that enables inspect, feeds an OSC title, and asserts the new counter increments. Add criterion functions that compile (may land red if helpers missing).

- [ ] **Step 2: Run — expect FAIL / missing fields**

```bash
cargo test -p lens-terminal --features test-util presentation_inspect -- --nocapture
```

- [ ] **Step 3: Implement counters + benches**

Wire `record_*` behind the existing `enabled` gate. Dense-hyperlink bench: seed with OSC 8 covering the viewport, then `build_frame` in the hot loop — confirms intern + minimized `grid_ref` stay within the 1c budget class (document the observed ms; do not silently regress).

- [ ] **Step 4: Run — expect PASS**

```bash
cargo test -p lens-terminal --features test-util presentation_inspect -- --nocapture
```
```bash
cargo bench -p lens-terminal --bench engine -- --quick
```
```bash
cargo clippy -p lens-terminal --all-targets --features test-util,live-tests -- -D warnings
```

- [ ] **Step 5: Commit**

```bash
git add crates/lens-terminal/src/engine/inspect.rs \
  crates/lens-terminal/src/engine/vt.rs \
  crates/lens-terminal/src/engine/handle.rs \
  crates/lens-terminal/src/lib.rs \
  crates/lens-terminal/benches/engine.rs
git commit -m "$(cat <<'EOF'
perf(terminal-2d): presentation inspect counters and hyperlink/title benches

EOF
)"
```

---

## Testing strategy (how tasks target each seam)

| Seam | How |
| --- | --- |
| Title OSC → latest-title / `reported_title` | Feed via `VtEngine::feed` / `EngineHandle::feed`; assert slot + drain; **no sleeps**. Channel-full test proves freshest title wins. |
| Hyperlink URI on Frame | OSC 8 feed → `build_frame` → assert interned `FrameCell.hyperlink_uri`. |
| URL validation / plain-URL | Pure unit tests — adversarial schemes + multibyte cell-index scan. |
| Click → host request | `tests/presentation_realwindow.rs` (`harness = false`, `test-util`). **Click only** — no hover claim. Not `#[gpui::test]`. |
| OSC 52 | **Not in 2d** — 2b. Demo Deny/no-op only. |
| Inspect + benches | Counter tests + criterion title-callback + dense-hyperlink frame benches. |
| Demo request/response | `lens-terminal-demo` subscribes; Allow OpenUrl; Deny clipboard. `presentation()` is owned. |

---

## Self-Review

**Spec coverage (2d completion-matrix rows):**

| Requirement | Task |
| --- | --- |
| Titles: sanitize/bound inside existing `on_title_changed`, stable `identity_title`, latest-title semantics | T1 (slot + drain) + T2 (sanitize/bound + reported-only) |
| Hyperlink: fill `FrameCell` URI + OSC 8 + plain-URL validation → **click** host request | T3 (extraction + intern) + T4 (validate + click gesture + `OpenUrlRequest` + demo) |
| `EnginePresentationEvent` backbone for 2b | T1 declares + methods/drain; **T5 defers clipboard registration to 2b** |
| Inspect + benches (matrix-mandated) | T6 |
| Progress + notifications | Deferred paragraph only — no tasks |
| Hover affordance | Explicitly deferred (T4 click-only) |

**Serial structure (no Task 0):**

- 2d lands **after 2a** and is self-contained: T1 **declares** the presentation surface (types, channel wiring, `VtEngine::new` `presentation_tx` param, bare `on_title_changed`, presentation methods, `drain_presentation_events` + render call) and fills the methods/drain; T2 sanitize; T3 adds `FrameCell.hyperlink_uri` + extraction; T4 adds `next_host_request_id` + `on_mouse_down` register/body + validation/gesture.
- **`VtEngine::new` arity change is 2d's** (2a kept `(cfg, on_reply)`); T1 updates every call site.
- Serial-seam note frames 2d as adding disjoint lines on top of 2a's committed `build_frame`/`render`.

**gpt-5.6 findings folded:**

1. **Critical — URL validation:** `url::Url` + exact `http`/`https` + non-empty host; reject whitespace/controls/backslash before parse; adversarial tests (`data:`, `file:`, spaces, backslash, empty host shapes).
2. **Critical — OSC 52 seam:** registration deferred to 2b entirely; T5 is a boundary note; demo Deny/no-op; no Busy-backpressure claim (OSC 52 ignores callback results).
3. **Important — title overflow:** latest-title slot + non-blocking wake (T1).
4. **Important — hyperlink termination + perf:** reject non-growing `required`; `Arc<str>` intern; minimize `grid_ref`; dense-hyperlink bench (T3 + T6).
5. **Important — plain-URL byte/cell mix:** cell/char-index scan + multibyte test (T4).
6. **Important — hover:** narrowed to click-only; hover deferred.
7. **Important — benches/inspect:** T6.
8. **Minor:** `presentation()` owned wording; remove Busy/OSC-52 claim; sanitize trim-then-truncate (ASCII); split multi-name `cargo test` commands.

**Gaps closed in this plan:**

- Binding gap for progress/notifications documented as out-of-scope; `progress` field stays unpopulated.
- Effect-callback non-blocking contract pinned; titles use latest-title semantics rather than drop-newest.
- `identity_title` vs `reported_title` separation with an apply helper that never writes identity from OSC.
- Hyperlink URI on `Frame` (render-path constraint), interned, not a side channel.
- OSC 52 **not registered** in 2d — 2b owns pre-clone cap + policy.
- Real-window vs `#[gpui::test]` false-green called out for click hit-testing.
- Concrete constants declared in T1: `MAX_REPORTED_TITLE_CHARS = 512`, `PRESENTATION_CHANNEL_CAP = 64`, `MAX_HYPERLINK_URI_BYTES = 8192`.

**Placeholder scan:** no TBD / "add error handling" / "similar to Task N" steps — each code step shows concrete Rust (or an explicit 2b boundary note for T5).

**Type consistency:** T1 `EnginePresentationEvent::{TitleChanged, HyperlinkOpen, ClipboardWrite}` → drain → `TerminalEvent::{PresentationChanged, OpenUrlRequest}` (clipboard emit deferred); `FrameCell.hyperlink_uri: Option<Arc<str>>`; `HostRequestId` for open-URL (and later clipboard in 2b); demo Deny for clipboard.
