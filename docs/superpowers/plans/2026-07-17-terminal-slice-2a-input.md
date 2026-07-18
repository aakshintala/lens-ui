# Terminal Slice 2a — Input Path Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the first production input path in `lens-terminal`: owner-side VT key encoding (live modes, full physical-key map), IME commit + foreground preedit overlay, focus wiring with mode-1004 report suppression, read-only gating with access-epoch revoke, `LocalScroll`, the ordered ingress/input command stream with worker-side Feed-chunk `Stop` preemption, and off-foreground never-drop / never-block-fg backpressure into `WsOutbound::Input`.

**Architecture:** Extend the existing single `EngineCommand` channel (`CMD_CHANNEL_CAP=256`) with `Key` / `Focus` / `LocalScroll` — **do not** add a second input channel. A `Feed` is an **atomic ordering unit**: the worker chunks it only for `Stop` preemption + a bounded work quantum — **never** to interleave later-arriving input mid-Feed (a Key that arrived after a Feed must see post-Feed modes). Encode keys on the engine thread via the `key_encoder`/`key_event` fields **this plan declares on `VtEngine`** with `set_options_from_terminal` per encode. User-input bytes and DA/DSR replies share the **`egress_*` channel** (this plan renames `da_dsr_*`→`egress_*`) as **distinct producers** with different full-channel policies (never-drop for keys; drop-oldest only for replies). Foreground `try_enqueue`s into a local queue; an off-fg forwarder does bounded blocking onto `cmd_tx` and is `Stop`-severable without blocking GPUI (preserves C3 `runtime.take()`).

**Tech Stack:** `crossbeam-channel`, `gpui` (`EntityInputHandler` / `ElementInputHandler` / `Window::handle_input`, `KeyDownEvent`/`KeyUpEvent`/`is_held`), `libghostty-vt` (`key::{Encoder,Event,Action,Key,Mods}`, `focus::Event`, `Terminal::scroll_viewport` / `Mode::FOCUS_EVENT` / cursor + `PointSpace`), `arc-swap`.

## Serial position (2a is FIRST — self-contained; no Task 0)

Slice 2 executes **serially on `terminal-ws`** (no worktrees, no merge): `2a → 2d → 2b → 2c`. **2a lands first**, so it **declares AND fills its own surface** — there is no shared foundation to build on. 2d lands *after* 2a and adds its own presentation surface against 2a's committed code; 2a therefore **never touches** presentation/hyperlink/mouse/host-request code.

**2a declares (new) and fills:**

| Surface 2a declares here | What 2a fills |
| --- | --- |
| Rename `da_dsr_*`→`egress_*` (channel, `emit_egress`, inspect `record_egress`) | Split **user-input** (never-drop) vs **reply** (drop-oldest) emit policy in worker |
| `VtEngine.key_encoder` / `key_event` fields (**`VtEngine::new` keeps its current arity** — 2d adds `presentation_tx` later) | `encode_key` / `encode_focus_report` / `local_scroll` using those fields |
| `Frame.cursor: Option<CursorPos>` + `CursorPos` in `frame.rs` | Viewport-safe cursor computation (I5) on the `cursor:` line of `build_frame` |
| `TerminalTab.ime_preedit` field (both literal sites) + `on_key_down`/`on_key_up` on the render div | `on_key_down` body + IME overlay + focus subs |
| `input_forwarder` on `EngineHandle`, `enqueue_input`, accessors | Forwarder wiring, non-blocking Drop, access-epoch |
| `EngineCommand::{Key,Focus,LocalScroll}` + arms | Full input command handling |

**2a does NOT touch** (2d owns, lands later): `presentation.rs`, the presentation channel, `VtEngine::new` arity, `on_title_changed`, `FrameCell.hyperlink_uri`, `next_host_request_id`, `on_mouse_down`, `drain_presentation_events`.

## Global Constraints

- **MAX_FEED_CHUNK = 4096** (4 KiB): pinned fairness quantum applied **only inside the worker** when draining one `EngineCommand::Feed`. A max inbound (≤64 KiB transport flood) enqueues as **one** `Feed`; the worker chunk-loop bounds VT work between `Stop` probes. **Do not** pre-split in `EngineHandle::feed`.
- **Feed = atomic ordering unit.** Chunking exists for **Stop-preemption + bounded work quantum ONLY**. Do **not** process mid-Feed `Key`/`Focus`/… before remaining Feed bytes (that would be sub-Feed input interleave and violate stream order). Mid-Feed arrivals go into `pending` and run **after** the Feed completes (so they correctly see post-Feed modes).
- **CMD_CHANNEL_CAP = 256** — do not change.
- **Per-task `lens-terminal` clippy MUST include `--features test-util` (and `live-tests` when touching live hooks).** Never pipe the gate through `tail`.
- **`Feed` stays `Vec<u8>`.**
- **Never block the GPUI foreground** on engine I/O; off-fg forwarder may block; `Stop`-severable; `EngineHandle::Drop` must use a **non-blocking** stop signal.
- **No sleeps / frame-polling for input ACK sync** — `recv_timeout` / barriers only. (`cargo test` takes **one** positional filter — split multi-name runs into separate invocations.)
- **`#[gpui::test]` / `NoopTextSystem` false-greens** — real IME/`InputHandler` claims go in `tests/input_realwindow.rs` (`harness = false`, `test-util`).
- Ground truth: Slice-2 interaction design (2a + DP1/2/5/6); real APIs in `worker.rs` / `handle.rs` / `vt.rs` / `bridge.rs` / `runtime.rs` / `key.rs` / `focus.rs` / gpui 0.2.2.

## Serial-seam note (2a → 2d)

2a and 2d edit **disjoint lines** of the shared functions, but serially, not in parallel. 2a owns: `EngineCommand` variants/arms, Feed chunk loop, `input_forwarder` on `EngineHandle`, `encode_key*`, the `egress` rename, `frame.rs` `cursor`/`CursorPos`, `build_frame` **`cursor:`** line, the `on_key_down` render hook + body, `ime_preedit`. Leave `build_frame` and `render` **clean and complete** for 2a's scope — 2d will add its `hyperlink_uri:` line, `on_mouse_down`, and `drain_presentation_events` call **on top of 2a's committed code**. Do **not** stub presentation hooks for 2d; 2d adds them itself.

## File Structure

| File | Responsibility |
| --- | --- |
| `crates/lens-terminal/src/engine/command.rs` | **Create.** `KeyInput` (incl. `access_epoch`), `KeyAction`, `LensKey` (**full** physical set), `KeyMods`, `ScrollDelta`, `InputAck`. |
| `crates/lens-terminal/src/engine/key_map.rs` | **Create.** GPUI/`LensKey` → Ghostty `Key` map + pure-encoder goldens. |
| `crates/lens-terminal/src/engine/worker.rs` | **Modify.** Rename `da_dsr_*`→`egress_*` (const/channels/`emit_egress`); `EngineCommand` variants; Feed chunking + bounded pending drain; Key/Focus/LocalScroll arms; split reply vs user-input egress emit. |
| `crates/lens-terminal/src/engine/handle.rs` | **Modify.** Rename `da_dsr_rx`→`egress_rx` (field + accessor); add `input_forwarder`; `enqueue_input`; access-epoch; non-blocking Drop; test barriers. |
| `crates/lens-terminal/src/engine/frame.rs` | **Modify.** Add `CursorPos` + `Frame.cursor: Option<CursorPos>` (fixtures/`Frame` literals get `cursor: None`). |
| `crates/lens-terminal/src/engine/vt.rs` | **Modify.** Add `key_encoder`/`key_event` fields on `VtEngine` (**keep `VtEngine::new` arity**); add `encode_key` / `encode_focus_report` / `local_scroll`; set + fill `cursor:` in `build_frame` (viewport-safe). |
| `crates/lens-terminal/src/engine/forwarder.rs` | **Create.** FG-local queue + off-fg forwarder; Stop-sever; purge-on-epoch; blocked-in-retry barrier for tests. |
| `crates/lens-terminal/src/engine/mod.rs` | **Modify.** `mod` + `pub use` for new modules. |
| `crates/lens-terminal/src/engine/inspect.rs` | **Modify.** Rename `record_da_dsr`→`record_egress`; input-path counters (keys encoded, user-egress accepts/rejects, feed chunks, stops preempted). |
| `crates/lens-terminal/src/input_gate.rs` | **Create.** `write_input_allowed` + unit tests. |
| `crates/lens-terminal/src/lib.rs` | **Modify.** Add `ime_preedit` field to `TerminalTab` (+ both literal sites); register `on_key_down`/`on_key_up` on the render div + fill; `EntityInputHandler`; focus subs; epoch bump on downgrade; gate; preedit overlay. |
| `crates/lens-terminal/src/render/state.rs` | **Modify.** `window.handle_input` during paint; preedit at `Frame.cursor` (hide if `None`). |
| `crates/lens-terminal/src/render/paint.rs` | **Modify.** Preedit overlay helper. |
| `crates/lens-terminal/benches/engine.rs` | **Modify.** Ordered-stream / key-encode micro-benches. |
| `crates/lens-terminal/Cargo.toml` | **Modify.** `[[test]] input_realwindow`. |
| `crates/lens-terminal/tests/input_realwindow.rs` | **Create.** Real keystroke through painted focused window + IME. |
| `crates/lens-terminal/tests/terminal_live.rs` | **Modify.** Manual IME checklist (DP6). |

---

### Task 1: `EngineCommand` + full physical `LensKey` map + pure-encoder goldens

**Files:**
- Create: `crates/lens-terminal/src/engine/command.rs`
- Create: `crates/lens-terminal/src/engine/key_map.rs`
- Modify: `crates/lens-terminal/src/engine/worker.rs` (`EngineCommand` enum + no-op arms)
- Modify: `crates/lens-terminal/src/engine/mod.rs`

**Interfaces:**
- Consumes: `libghostty_vt::key::{Encoder, Event, Action, Key, Mods}` — real signatures (verified; unchanged):
  - `Encoder::new() -> Result<Self>`
  - `Encoder::set_cursor_key_application` / `set_keypad_key_application` / `set_alt_esc_prefix` / `set_kitty_flags` / `set_options_from_terminal`
  - `Encoder::encode_to_vec(&mut self, event: &Event, vec: &mut Vec<u8>) -> Result<()>`
  - `Event::new` / `set_action` / `set_key` / `set_mods` / `set_composing` / `set_utf8`
- Produces:
  - `KeyAction { Press, Release, Repeat }`
  - `KeyMods { shift, alt, ctrl, super_key }`
  - `LensKey` — **full relevant Ghostty set**: `A`–`Z`, `Digit0`–`Digit9`, punctuation (`Minus`, `Equal`, `Bracket*`, `Semicolon`, `Quote`, `Backquote`, `Comma`, `Period`, `Slash`, `Backslash`, …), keypad (`Numpad0`–`9`, `NumpadEnter`, `NumpadAdd`, …), nav/editing/F-keys, `Unidentified`. **Do not** collapse letters/digits to `Unidentified` (encoder needs the physical key for Ctrl/Alt/Kitty — `key.rs:387-564`).
  - `KeyInput { action, key, mods, utf8: Option<String>, composing: bool, access_epoch: u64, ack: Option<Sender<InputAck>> }`
  - `InputAck { encoded: Vec<u8>, accepted: bool }` — `accepted=false` when user-input egress rejects (full); never claim success after a drop.
  - `ScrollDelta { Lines(i32), Top, Bottom }`
  - `EngineCommand::{ Feed(Vec<u8>), Key(KeyInput), Focus { focused: bool, report: bool, access_epoch: u64 }, LocalScroll(ScrollDelta), Resize, SetVisible, BuildNow, Stop }`
  - `keystroke_to_lens(key: &str) -> LensKey` mapping gpui physical `keystroke.key` strings
  - `encode_key_pure` / `apply_key_input_to_event`

- [ ] **Step 1: Write failing pure-encoder goldens** in `key_map.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use libghostty_vt::key::{Encoder, Event, KittyKeyFlags};

    fn base_arrow() -> KeyInput {
        KeyInput {
            action: KeyAction::Press,
            key: LensKey::ArrowUp,
            mods: KeyMods::default(),
            utf8: None,
            composing: false,
            access_epoch: 0,
            ack: None,
        }
    }

    #[test]
    fn arrow_up_normal_mode_encodes_csi_a() {
        let mut enc = Encoder::new().unwrap();
        enc.set_cursor_key_application(false);
        let mut ev = Event::new().unwrap();
        let input = base_arrow();
        let mut buf = Vec::new();
        encode_key_pure(&mut enc, &mut ev, &input, &mut buf).unwrap();
        assert_eq!(buf, b"\x1b[A");
    }

    #[test]
    fn ctrl_c_encodes_etx() {
        let mut enc = Encoder::new().unwrap();
        let mut ev = Event::new().unwrap();
        let input = KeyInput {
            action: KeyAction::Press,
            key: LensKey::C,
            mods: KeyMods { ctrl: true, ..KeyMods::default() },
            utf8: None, // physical key + mods — NOT utf8 "c"
            composing: false,
            access_epoch: 0,
            ack: None,
        };
        let mut buf = Vec::new();
        encode_key_pure(&mut enc, &mut ev, &input, &mut buf).unwrap();
        assert_eq!(buf, b"\x03");
    }

    #[test]
    fn alt_printable_a_encodes_esc_prefixed() {
        let mut enc = Encoder::new().unwrap();
        enc.set_alt_esc_prefix(true);
        let mut ev = Event::new().unwrap();
        let input = KeyInput {
            action: KeyAction::Press,
            key: LensKey::A,
            mods: KeyMods { alt: true, ..KeyMods::default() },
            utf8: Some("a".into()),
            composing: false,
            access_epoch: 0,
            ack: None,
        };
        let mut buf = Vec::new();
        encode_key_pure(&mut enc, &mut ev, &input, &mut buf).unwrap();
        assert_eq!(buf, b"\x1ba");
    }

    #[test]
    fn keypad_enter_encodes_under_application_keypad() {
        let mut enc = Encoder::new().unwrap();
        enc.set_keypad_key_application(true);
        let mut ev = Event::new().unwrap();
        let input = KeyInput {
            action: KeyAction::Press,
            key: LensKey::NumpadEnter,
            mods: KeyMods::default(),
            utf8: None,
            composing: false,
            access_epoch: 0,
            ack: None,
        };
        let mut buf = Vec::new();
        encode_key_pure(&mut enc, &mut ev, &input, &mut buf).unwrap();
        assert!(!buf.is_empty(), "keypad enter must produce bytes; got {buf:?}");
        // Pin exact golden from first green run.
    }

    #[test]
    fn kitty_flags_encode_release_and_repeat_distinctly() {
        let mut enc = Encoder::new().unwrap();
        enc.set_kitty_flags(KittyKeyFlags::DISAMBIGUATE | KittyKeyFlags::REPORT_EVENTS);
        let mut ev = Event::new().unwrap();
        let press_in = base_arrow();
        let mut press = Vec::new();
        encode_key_pure(&mut enc, &mut ev, &press_in, &mut press).unwrap();
        let mut release = Vec::new();
        let rel = KeyInput { action: KeyAction::Release, ..press_in.clone_without_ack() };
        encode_key_pure(&mut enc, &mut ev, &rel, &mut release).unwrap();
        let mut repeat = Vec::new();
        let rep = KeyInput { action: KeyAction::Repeat, ..press_in.clone_without_ack() };
        encode_key_pure(&mut enc, &mut ev, &rep, &mut repeat).unwrap();
        assert_ne!(press, release, "Kitty release must differ from press");
        assert_ne!(press, repeat, "Kitty repeat must differ from press");
    }

    #[test]
    fn ime_commit_utf8_field_encodes_text_bytes() {
        let mut enc = Encoder::new().unwrap();
        let mut ev = Event::new().unwrap();
        let input = KeyInput {
            action: KeyAction::Press,
            key: LensKey::Unidentified,
            mods: KeyMods::default(),
            utf8: Some("你好".into()),
            composing: false,
            access_epoch: 0,
            ack: None,
        };
        let mut buf = Vec::new();
        encode_key_pure(&mut enc, &mut ev, &input, &mut buf).unwrap();
        assert_eq!(buf, "你好".as_bytes());
    }

    #[test]
    fn composing_true_produces_no_pty_bytes() {
        let mut enc = Encoder::new().unwrap();
        let mut ev = Event::new().unwrap();
        let input = KeyInput {
            action: KeyAction::Press,
            key: LensKey::Unidentified,
            mods: KeyMods::default(),
            utf8: Some("n".into()),
            composing: true,
            access_epoch: 0,
            ack: None,
        };
        let mut buf = Vec::new();
        encode_key_pure(&mut enc, &mut ev, &input, &mut buf).unwrap();
        assert!(buf.is_empty(), "preedit must not emit PTY bytes; got {buf:?}");
    }
}
```

(`clone_without_ack` = small helper copying fields with `ack: None`.)

- [ ] **Step 2: Run — expect FAIL**

```bash
cargo test -p lens-terminal --lib engine::key_map -- --nocapture
```

Expected: compile fail (`key_map` / `KeyInput` missing).

- [ ] **Step 3: Implement** `command.rs` + `key_map.rs` + `EngineCommand` variants with no-op Key/Focus/LocalScroll arms. Map `"a"`…`"z"`, `"0"`…`"9"`, `"space"`, punctuation names, `"numpad0"`…, nav, F-keys per gpui `Keystroke.key` conventions (verify against gpui; pin strings that fail in Step 2).

```rust
pub(crate) enum EngineCommand {
    Feed(Vec<u8>),
    Key(KeyInput),
    Focus { focused: bool, report: bool, access_epoch: u64 },
    LocalScroll(ScrollDelta),
    Resize(u16, u16),
    SetVisible(bool),
    BuildNow,
    Stop,
}
```

```rust
EngineCommand::Key(_)
| EngineCommand::Focus { .. }
| EngineCommand::LocalScroll(_) => {}
```

- [ ] **Step 4: Run — expect PASS**

```bash
cargo test -p lens-terminal --lib engine::key_map -- --nocapture
```

- [ ] **Step 5: Commit**

```bash
git add crates/lens-terminal/src/engine/command.rs \
        crates/lens-terminal/src/engine/key_map.rs \
        crates/lens-terminal/src/engine/worker.rs \
        crates/lens-terminal/src/engine/mod.rs
git commit -m "$(cat <<'EOF'
feat(lens-terminal): EngineCommand Key/Focus/LocalScroll + full physical key map goldens (2a)

EOF
)"
```

---

### Task 2: Foundation decls (egress rename, key fields, `Frame.cursor`) + live-mode `encode_key` + never-drop user egress + viewport-safe cursor

**Step 0 (foundation declarations — do first, keep mechanical):**
- Rename `DA_DSR_CHANNEL_CAP`→`EGRESS_CHANNEL_CAP` (keep `64`); `WorkerChannels.da_dsr_tx/rx`→`egress_tx/rx`; `emit_da_dsr`→`emit_egress`; all `da_dsr` locals→`egress`; `handle.rs` field/accessor `da_dsr_rx`→`egress_rx`; `inspect.rs` `record_da_dsr`→`record_egress` + snapshot field `da_dsr_emitted`→`egress_emitted`. **The rename fans out to `bridge.rs`** (`engine.da_dsr_rx()`→`egress_rx()`, `da_dsr_rx` local/field, `forward_da_dsr`→`forward_egress`, `inspect().da_dsr_emitted`→`egress_emitted` **including the bridge tests at ~242/256**) and a stale comment in `runtime.rs` — **all in the same commit** or the crate won't compile. Purely mechanical; no counter-semantics change. **`VtEngine::new` keeps its current arity** (no `presentation_tx` — that's 2d).
- Add `key_encoder: libghostty_vt::key::Encoder<'static>` + `key_event: libghostty_vt::key::Event<'static>` to `VtEngine`, constructed in `VtEngine::new` (`Encoder::new()?` / `Event::new()?`). Task 2 uses them immediately, so no `#[expect(dead_code)]` needed.
- Add `CursorPos { col: u16, row: u16 }` + `Frame.cursor: Option<CursorPos>` to `frame.rs`; re-export `CursorPos` from `engine/mod.rs`. Every existing `Frame { … }` literal (fixtures, paint tests, reconnect_seed) gains `cursor: None` — the viewport fill below replaces the `build_frame` one.

**Files:**
- Modify: `crates/lens-terminal/src/engine/frame.rs` (add `CursorPos` + `Frame.cursor`)
- Modify: `crates/lens-terminal/src/engine/vt.rs` (add `key_encoder`/`key_event` fields; `encode_key`; set + fill `cursor:` in `build_frame`)
- Modify: `crates/lens-terminal/src/engine/worker.rs` (egress rename; Key arm; split egress emit)
- Modify: `crates/lens-terminal/src/engine/handle.rs` (`egress_rx` rename)
- Modify: `crates/lens-terminal/src/engine/inspect.rs` (egress rename; user-egress accept/reject counters)
- Modify: `crates/lens-terminal/src/engine/mod.rs` (re-export `CursorPos`)
- Modify: `crates/lens-terminal/src/bridge.rs` (**mandatory** egress rename: `egress_rx()` accessor call, `da_dsr_rx` local, `forward_da_dsr`→`forward_egress`, `da_dsr_emitted`→`egress_emitted` + bridge tests; **plus** any user-egress-full saturation event if surfaced)
- Modify: `crates/lens-terminal/src/runtime.rs` (stale `da_dsr` comment)
- Modify: fixtures/`Frame` literals gain `cursor: None` — only `render/fixtures.rs` has non-`build_frame` `Frame { … }` literals (grep-confirmed: no `Frame` literals in tests/benches/`reconnect_seed`)

**Interfaces:**
- Consumes: `self.key_encoder` / `self.key_event` (declared in Step 0) / `egress_tx/rx` / `emit_egress`.
- Produces:
  - `VtEngine::encode_key(&mut self, input: &KeyInput) -> Result<Vec<u8>, EngineError>` — `set_options_from_terminal(&self.terminal)` then `encode_key_pure`; composing → empty.
  - `emit_reply_egress(...)` — keep **drop-oldest** behavior; **only** for DA/DSR / `take_replies`.
  - `try_emit_user_input(egress_tx, bytes) -> Result<(), UserEgressFull>` — **`try_send` only**; on `Full` return `Err` (**never** drop-oldest). On `Err`: do **not** ACK as accepted; bump inspect `user_egress_rejected`; surface saturation toward reconnect policy (`UserEgressSaturated` **or** reuse `OutboundSaturated` with a clear comment).
  - Key arm: encode → `try_emit_user_input` → ACK `{ encoded, accepted }` reflecting **actual** acceptance (empty encode → `accepted: true`, no send).
  - `build_frame` **`cursor:`** line (I5) — the viewport fill (fixtures/other `Frame` literals keep `cursor: None`):

```rust
cursor: viewport_cursor_pos(&self.terminal, cols, rows),
```

```rust
fn viewport_cursor_pos(term: &Terminal<'_, '_>, cols: u16, rows: u16) -> Option<CursorPos> {
    // cursor_x/y are ACTIVE-AREA coords (terminal.rs:564-570) — NEVER unwrap_or(0).
    let ax = term.cursor_x().ok()?;
    let ay = term.cursor_y().ok()?;
    // Map active → viewport via grid_ref / point_from_grid_ref(PointSpace::Viewport).
    // If Ok(None) or out of [0,cols)×[0,rows) → None (hide preedit).
    ...
}
```

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn key_encodes_against_live_modes_via_ordered_feed_then_ack() {
    // DECSET ?1h then Key ArrowUp via cmd until T3; T4+ must use enqueue_input.
    // ack.encoded == b"\x1bOA" && ack.accepted
}

#[test]
fn user_input_egress_full_does_not_drop_or_false_ack() {
    // Fill egress to cap (64); enqueue Key with ack;
    // assert ack.accepted == false AND prior egress contents unchanged (no drop-oldest).
}

#[test]
fn build_frame_cursor_none_when_scrolled_out_of_viewport() {
    // Scroll viewport away from active cursor; build_frame → cursor.is_none()
    // (not Some(CursorPos{0,0})).
}
```

- [ ] **Step 2: Run — expect FAIL** (one filter each)

```bash
cargo test -p lens-terminal --lib key_encodes_against_live_modes -- --nocapture
cargo test -p lens-terminal --lib user_input_egress_full -- --nocapture
cargo test -p lens-terminal --lib build_frame_cursor_none -- --nocapture
```

- [ ] **Step 3: Implement** Step-0 decls (egress rename, `key_encoder`/`key_event` fields, `Frame.cursor`/`CursorPos`), then `encode_key`, split emit paths, viewport-safe cursor fill. **Do not** add `presentation_tx` to `VtEngine::new` (2d owns presentation).

- [ ] **Step 4: Run — expect PASS** (same three commands).

- [ ] **Step 5: Commit**

```bash
git add crates/lens-terminal/src/engine/frame.rs \
        crates/lens-terminal/src/engine/vt.rs \
        crates/lens-terminal/src/engine/worker.rs \
        crates/lens-terminal/src/engine/handle.rs \
        crates/lens-terminal/src/engine/inspect.rs \
        crates/lens-terminal/src/engine/mod.rs \
        crates/lens-terminal/src/render/fixtures.rs \
        crates/lens-terminal/src/bridge.rs \
        crates/lens-terminal/src/runtime.rs
git commit -m "$(cat <<'EOF'
feat(lens-terminal): egress rename, key-encoder fields, Frame.cursor, live encode_key, never-drop user egress (2a)

EOF
)"
```

---

### Task 3: Off-fg forwarder + non-blocking Drop + access-epoch tagging

**Files:**
- Create: `crates/lens-terminal/src/engine/forwarder.rs`
- Modify: `crates/lens-terminal/src/engine/handle.rs` (`input_forwarder: Option<InputForwarder>`, `enqueue_input`, `access_epoch: Arc<AtomicU64>`, Drop)
- Modify: `crates/lens-terminal/src/runtime.rs` (verify teardown still off-fg)

**Interfaces:**
- `InputForwarder`: unbounded local queue + `send_timeout` retry onto `cmd_tx` + `stop` flag.
- `try_enqueue` — never blocks fg.
- `sever_and_join` / `purge()` — drop pending local cmds (access downgrade).
- `EngineHandle::enqueue_input(cmd)` stamps current `access_epoch` onto `Key`/`Focus` when caller left 0 / always overwrite with current at enqueue time (pin: **stamp at enqueue**).
- `EngineHandle::bump_access_epoch(&self) -> u64` — fetch_add(1); `forwarder.purge()`.
- **Drop:** non-blocking stop (`try_send(Stop)` or dedicated wakeup/`AtomicBool`) — **never** blocking `cmd_tx.send(Stop)` on Drop. `stop()` (off-fg) still joins.
- Test barrier: forwarder sets `blocked_in_retry` when `send_timeout` returns Timeout once; sever test waits on it, then `sever_and_join`, completion via `recv_timeout` on a done channel.

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn try_enqueue_never_blocks_when_engine_channel_full() { /* 1000 enqueues < 50ms */ }

#[test]
fn sever_unblocks_forwarder_after_blocked_barrier() {
    // Fill cmd cap=1; enqueue Key; WAIT until forwarder signals blocked-in-retry;
    // sever_and_join; assert via done.recv_timeout(1s).
}

#[test]
fn drop_engine_handle_does_not_block_on_full_cmd_channel() {
    // Fill cmd channel; drop handle; elapsed < 50ms.
}

#[test]
fn forwarder_delivers_key_via_enqueue_input() {
    let h = EngineHandle::spawn(test_config());
    let (ack_tx, ack_rx) = crossbeam_channel::bounded(1);
    h.enqueue_input(EngineCommand::Key(KeyInput {
        action: KeyAction::Press,
        key: LensKey::ArrowUp,
        mods: KeyMods::default(),
        utf8: None,
        composing: false,
        access_epoch: 0,
        ack: Some(ack_tx),
    })).unwrap();
    let ack = ack_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(ack.accepted);
    h.stop();
}
```

- [ ] **Step 2: Run — expect FAIL** (separate invocations).

- [ ] **Step 3: Implement** forwarder + handle wiring + non-blocking Drop. Pin `send_timeout(50ms)` retry loop checking `stop`.

- [ ] **Step 4: Run — expect PASS.**

- [ ] **Step 5: Commit**

```bash
git add crates/lens-terminal/src/engine/forwarder.rs \
        crates/lens-terminal/src/engine/handle.rs \
        crates/lens-terminal/src/engine/mod.rs \
        crates/lens-terminal/src/runtime.rs
git commit -m "$(cat <<'EOF'
feat(lens-terminal): input forwarder with Stop-sever, epoch, non-blocking Drop (2a)

EOF
)"
```

---

### Task 4: Worker-side Feed chunking + deterministic Stop preempt + straddle

**Files:**
- Modify: `crates/lens-terminal/src/engine/worker.rs`
- Modify: `crates/lens-terminal/src/engine/handle.rs` (test barrier hook only)

**Interfaces:**
- `pub(crate) const MAX_FEED_CHUNK: usize = 4096;`
- **Invariant:** Chunking at arbitrary byte boundaries is safe because `vt_write` retains partial-sequence parser state across calls (streaming VT parser); a DECSET split across a chunk boundary still applies. Verified by the straddle test below.
- **Feed = atomic ordering unit** (restate): chunking is for Stop-preemption + bounded work quantum ONLY — **not** sub-Feed input interleave.
- `handle_feed_chunked(..., pending: &mut VecDeque<EngineCommand>, ...)`
  - Pass `pending` **explicitly**.
  - After each chunk: probe with **bounded** quantum `for _ in 0..PENDING_PROBE_CAP` (`const PENDING_PROBE_CAP: usize = 32`) — not unbounded `while let Ok` (prevents livelock under continuous producer).
  - `Stop` → `stopping=true`, return (leave remaining Feed unfed).
  - Non-Stop → `pending.push_back`; drain **after** Feed completes.
- **Deterministic Stop-preempt barrier (`cfg(test)`):**
  - `EngineHandle::test_arm_chunk_barrier(&self)` installs a barrier the worker waits on **after chunk 0** of a ≥2-chunk Feed.
  - Test: `feed(64KiB)` → wait for “after chunk0” → `send(Stop)` → release barrier → join; assert `MAX_FEED_CHUNK <= bytes_fed < 64KiB`.

- [ ] **Step 1: Write failing tests** (Keys via **`enqueue_input`**, not `cmd_sender`):

```rust
#[test]
fn feed_is_atomic_key_after_feed_sees_post_feed_modes() {
    let h = EngineHandle::spawn(test_config());
    h.feed(b"\x1b[?1h".to_vec()).unwrap();
    let (ack_tx, ack_rx) = crossbeam_channel::bounded(1);
    h.enqueue_input(EngineCommand::Key(KeyInput {
        action: KeyAction::Press,
        key: LensKey::ArrowUp,
        mods: KeyMods::default(),
        utf8: None,
        composing: false,
        access_epoch: 0,
        ack: Some(ack_tx),
    })).unwrap();
    let ack = ack_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(ack.encoded, b"\x1bOA");
    h.stop();
}

#[test]
fn key_before_feed_sees_pre_feed_modes() {
    let h = EngineHandle::spawn(test_config());
    let (ack_tx, ack_rx) = crossbeam_channel::bounded(1);
    h.enqueue_input(EngineCommand::Key(KeyInput {
        action: KeyAction::Press,
        key: LensKey::ArrowUp,
        mods: KeyMods::default(),
        utf8: None,
        composing: false,
        access_epoch: 0,
        ack: Some(ack_tx),
    })).unwrap();
    let mut mode_and_pad = Vec::with_capacity(64 * 1024);
    mode_and_pad.extend_from_slice(b"\x1b[?1h");
    mode_and_pad.resize(64 * 1024, b' ');
    h.feed(mode_and_pad).unwrap();
    let ack = ack_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(ack.encoded, b"\x1b[A");
    h.stop();
}

#[test]
fn stop_preempts_feed_between_chunks_deterministically() {
    let h = EngineHandle::spawn(test_config());
    h.set_inspect_enabled(true);
    h.test_arm_chunk_barrier();
    h.feed(vec![b'X'; 64 * 1024]).unwrap();
    h.test_wait_after_first_chunk(); // blocks until worker finished chunk0
    h.cmd_sender().send(EngineCommand::Stop).unwrap();
    h.test_release_chunk_barrier();
    // Snapshot before stop consumes the handle.
    let deadline = Instant::now() + Duration::from_secs(2);
    while h.cmd_sender().try_send(EngineCommand::BuildNow).is_ok() {
        assert!(Instant::now() < deadline, "worker did not exit after Stop");
    }
    let fed = h.inspect().bytes_fed;
    h.stop();
    assert!(fed >= MAX_FEED_CHUNK as u64);
    assert!(fed < 64 * 1024);
}

#[test]
fn decset_straddling_feed_chunk_boundary_still_applies() {
    assert_eq!(MAX_FEED_CHUNK, 4096);
    let h = EngineHandle::spawn(test_config());
    let mut buf = vec![b' '; 4094];
    buf.extend_from_slice(b"\x1b[?1h");
    h.feed(buf).unwrap();
    let (ack_tx, ack_rx) = crossbeam_channel::bounded(1);
    h.enqueue_input(EngineCommand::Key(KeyInput {
        action: KeyAction::Press,
        key: LensKey::ArrowUp,
        mods: KeyMods::default(),
        utf8: None,
        composing: false,
        access_epoch: 0,
        ack: Some(ack_tx),
    })).unwrap();
    let ack = ack_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(ack.encoded, b"\x1bOA");
    h.stop();
}
```

(Fix `stop_preempts` to snapshot `bytes_fed` before consuming `h` in `stop()`.)

- [ ] **Step 2: Run — expect FAIL** (four separate invocations).

- [ ] **Step 3: Implement** `handle_feed_chunked` + barrier + bounded pending drain. **Do not** change `EngineHandle::feed` (single whole-buffer `try_send`).

- [ ] **Step 4: Run — expect PASS.**

- [ ] **Step 5: Commit**

```bash
git add crates/lens-terminal/src/engine/worker.rs crates/lens-terminal/src/engine/handle.rs
git commit -m "$(cat <<'EOF'
fix(lens-terminal): worker Feed chunk Stop-preempt with deterministic barrier (2a)

EOF
)"
```

---

### Task 5: GPUI InputHandler + special-only keydown + real-window keystroke

**Files:**
- Modify: `crates/lens-terminal/src/lib.rs` (add `ime_preedit` field to `TerminalTab` + both literal sites `starting()`/`with_engine_for_test()`; register `on_key_down` + `on_key_up` on the render div + fill; `EntityInputHandler`; `cfg(any(test, feature = "test-util"))` hooks)
- Modify: `crates/lens-terminal/src/render/state.rs` (`handle_input` + preedit at `cursor`)
- Modify: `crates/lens-terminal/src/render/paint.rs`
- Modify: `crates/lens-terminal/Cargo.toml`
- Create: `crates/lens-terminal/tests/input_realwindow.rs`
- Modify: `crates/lens-terminal/tests/terminal_live.rs`

**Interfaces / critical split (no double-emit):**
- **`on_key_down` / `on_key_up`:** encode **ONLY** non-text / special / modified keys (nav, F-keys, Enter/Tab/Esc/Backspace/Delete, Ctrl/Alt/Super combos including Ctrl-C). Map `KeyDownEvent::is_held` → `Repeat`; `KeyUpEvent` → `Release`. **Do not** enqueue plain printable unmodified text from keydown — gpui auto-forwards `key_char` to `InputHandler::replace_text_in_range` (`window.rs:3553-3557`).
- **`replace_text_in_range`:** sole owner of committed text / IME commit → `KeyInput { key: Unidentified, utf8: Some(text), composing: false, access_epoch }`.
- **`replace_and_mark_text_in_range`:** set `ime_preedit` (this plan's `TerminalTab` field); no engine enqueue.
- Preedit overlay only when `frame.cursor.is_some()`; else hide.
- Hooks under `cfg(any(test, feature = "test-util"))`:
  - `debug_ime_commit_for_test(&mut self, text: &str) -> Option<Receiver<InputAck>>` — **enqueues and returns receiver**; **never** `recv_timeout` inside the entity method.
  - Writable fixture: `input_enabled = true`, `AccessMode::Write`, epoch synced (`with_engine_for_test` today starts `input_enabled=false` — fix for input tests).

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn printable_key_emits_exactly_once_via_input_handler_not_keydown() {
    // on_key_down unmodified "a" must NOT enqueue;
    // replace_text_in_range("a") enqueues once → exactly one accepted ACK / one egress.
}

#[test]
fn ime_commit_hook_returns_receiver_without_blocking() {
    let mut tab = /* writable with_engine_for_test */;
    let rx = tab.debug_ime_commit_for_test("你好").expect("rx");
    let ack = rx.recv_timeout(Duration::from_secs(2)).unwrap(); // OUTSIDE entity update
    assert_eq!(ack.encoded, "你好".as_bytes());
    assert!(ack.accepted);
}
```

- [ ] **Step 2: Run — expect FAIL.**

- [ ] **Step 3: Implement** `ime_preedit` field + both literal sites; register `on_key_down` + `on_key_up` on the render div; handler split + overlay + `window.handle_input` during paint.

- [ ] **Step 4: Real-window harness** — dispatch a **real** gpui keystroke into the painted focused window (not only debug hooks). Assert special-key path (ArrowUp) and printable-via-InputHandler. Manual IME checklist in `terminal_live.rs`.

```toml
[[test]]
name = "input_realwindow"
path = "tests/input_realwindow.rs"
harness = false
required-features = ["test-util"]
```

- [ ] **Step 5: Run**

```bash
cargo test -p lens-terminal --lib printable_key_emits_exactly_once -- --nocapture
cargo test -p lens-terminal --lib ime_commit_hook_returns_receiver -- --nocapture
cargo test -p lens-terminal --features test-util --test input_realwindow -- --nocapture
cargo clippy -p lens-terminal --all-targets --features test-util,live-tests -- -D warnings
```

- [ ] **Step 6: Commit**

```bash
git add crates/lens-terminal/src/lib.rs \
        crates/lens-terminal/src/render/state.rs \
        crates/lens-terminal/src/render/paint.rs \
        crates/lens-terminal/Cargo.toml \
        crates/lens-terminal/tests/input_realwindow.rs \
        crates/lens-terminal/tests/terminal_live.rs
git commit -m "$(cat <<'EOF'
feat(lens-terminal): InputHandler text path + special-only keydown + realwindow (2a)

EOF
)"
```

---

### Task 6: Focus + read-only gate + epoch revoke + LocalScroll + benches/inspect

**Files:**
- Create: `crates/lens-terminal/src/input_gate.rs`
- Modify: `crates/lens-terminal/src/engine/vt.rs` (`encode_focus_report`, `local_scroll`)
- Modify: `crates/lens-terminal/src/engine/worker.rs` (Focus/LocalScroll arms; epoch check at final egress)
- Modify: `crates/lens-terminal/src/lib.rs` (focus subs; downgrade → `bump_access_epoch` + purge)
- Modify: `crates/lens-terminal/benches/engine.rs` + `engine/inspect.rs`
- Scroll wheel → `LocalScroll` (allowed in read-only)

**Interfaces:**
- `write_input_allowed(access, input_enabled) -> bool`
- Worker final egress for `Key` / `Focus{report:true}`: if `cmd.access_epoch != current_epoch` → **suppress** (no emit; ACK `accepted: false` when present).
- On read-only downgrade (`lib.rs` ~648–651): `bump_access_epoch()` + `forwarder.purge()`.
- Focus via `Context::on_focus_in` / `on_blur` (gpui 0.2.2); `Focus { focused, report: write_input_allowed(), access_epoch }`.
- Negative focus test uses command barrier/ACK — **not** `build_now` + immediate `try_recv`.
- Benches (`feature = "bench"`): `key_encode_arrow_up`, `ordered_stream_feed_then_key_throughput`.
- Inspect: `keys_encoded`, `user_egress_accepted`, `user_egress_rejected`, `feed_chunks`, `stop_preempts`.

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn write_input_allowed_requires_write_access_and_input_enabled() {
    assert!(!write_input_allowed(AccessMode::ReadOnly, true));
    assert!(!write_input_allowed(AccessMode::Write, false));
    assert!(write_input_allowed(AccessMode::Write, true));
}

#[test]
fn focus_report_suppressed_when_report_false_with_ack_barrier() {
    // Focus{report:false} with handled-ack / barrier; then assert egress empty.
}

#[test]
fn focus_report_emits_csi_i_when_mode_on_and_report_true() {
    // feed ?1004h; Focus{report:true}; egress recv_timeout → b"\x1b[I"
}

#[test]
fn downgrade_revokes_queued_key_before_egress() {
    // enqueue_input Key while writable; bump_access_epoch + ReadOnly before process;
    // assert no user-input egress / ack.accepted == false.
}

#[test]
fn local_scroll_allowed_in_read_only_without_egress() { ... }
```

- [ ] **Step 2: Run — expect FAIL** (separate invocations).

- [ ] **Step 3: Implement** gate, focus, epoch recheck, LocalScroll, benches, inspect.

```rust
pub fn encode_focus_report(&mut self, focused: bool) -> Result<Option<Vec<u8>>, EngineError> {
    use libghostty_vt::focus::Event as FocusEv;
    use libghostty_vt::terminal::Mode;
    if !self.terminal.mode(Mode::FOCUS_EVENT)? {
        return Ok(None);
    }
    let ev = if focused { FocusEv::Gained } else { FocusEv::Lost };
    let mut buf = [0u8; 16];
    let n = ev.encode(&mut buf)?;
    Ok(Some(buf[..n].to_vec()))
}

pub fn local_scroll(&mut self, delta: ScrollDelta) {
    use libghostty_vt::terminal::ScrollViewport;
    let scroll = match delta {
        ScrollDelta::Lines(n) => ScrollViewport::Delta(n as isize),
        ScrollDelta::Top => ScrollViewport::Top,
        ScrollDelta::Bottom => ScrollViewport::Bottom,
    };
    self.terminal.scroll_viewport(scroll);
}
```

Focus subscriptions (real gpui 0.2.2 `Context` APIs):

```rust
self.focus_in_sub = Some(cx.on_focus_in(&self.focus_handle, window, |this, _w, cx| {
    this.on_focus_changed(true, cx);
}));
self.focus_out_sub = Some(cx.on_blur(&self.focus_handle, window, |this, _w, cx| {
    this.on_focus_changed(false, cx);
}));
```

Subscribe from first `render` via a guard if `on_attached` lacks `&mut Window`.

- [ ] **Step 4: Run — expect PASS** + final gate:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p lens-terminal --all-targets --features test-util,live-tests -- -D warnings
cargo test -p lens-terminal --lib
cargo test -p lens-terminal --features test-util --test input_realwindow
cargo bench -p lens-terminal --features bench --no-run
```

- [ ] **Step 5: Commit**

```bash
git add crates/lens-terminal/src/input_gate.rs \
        crates/lens-terminal/src/engine/ \
        crates/lens-terminal/src/lib.rs \
        crates/lens-terminal/benches/engine.rs
git commit -m "$(cat <<'EOF'
feat(lens-terminal): focus gate, access-epoch revoke, LocalScroll, input benches (2a)

EOF
)"
```

---

## Self-Review

### Serial structure (no Task 0)
- **2a is self-contained and lands first on `terminal-ws`.** It declares **and** fills its own surface: egress rename (T2 Step 0), `key_encoder`/`key_event` fields (T2), `Frame.cursor`/`CursorPos` (T2), `EngineCommand::{Key,Focus,LocalScroll}` (T1), `input_forwarder` (T3), `ime_preedit` + `on_key_down`/`on_key_up` render hooks (T5), `input_gate.rs` (T6).
- **`VtEngine::new` keeps its current arity** — 2a adds no `presentation_tx`. 2d (lands after 2a) adds the presentation channel, `on_title_changed`, `FrameCell.hyperlink_uri`, `on_mouse_down`, and `drain_presentation_events` against 2a's committed code. 2a must **not** stub any of those.

### Completion matrix (2a) + folded findings

| Matrix / finding | Where addressed |
| --- | --- |
| Keyboard VT encoding (full physical map) | T1 goldens Ctrl-C / Alt / keypad / Kitty; T2 live encode |
| Ordered stream + Feed-chunk Stop preempt | T4 — Feed atomic unit; worker-only chunk; deterministic barrier; `enqueue_input` tests |
| IME commit + FG preedit | T5 — InputHandler owns text; overlay at `cursor` |
| Focus + mode-1004 suppress | T6 |
| Read-only gate + LocalScroll | T6 + epoch revoke |
| Off-fg never-drop never-block | T3 forwarder; T2 never-drop **user** egress |
| **#1 double-emit printable** | T5 keydown special-only + exactly-once test |
| **#2 full VT key encoding** | T1 full `LensKey` + UP/Repeat |
| **#3 egress drop-oldest on keys** | T2 `try_emit_user_input` vs reply drop-oldest; ACK=`accepted` |
| **#4 vacuous/racy chunk tests** | T4 barrier + atomic-Feed note + forwarder routing |
| **#5 IME hook blocks fg** | T5 returns `Receiver`; writable fixture |
| **#6 pending livelock** | T4 explicit `pending` param + bounded probe |
| **#7 sever race + Drop blocks** | T3 blocked barrier; non-blocking Drop |
| **#8 downgrade queued input** | T6 epoch stamp + purge + egress recheck |
| **#9 realwindow bypass / focus race** | T5 real keystroke; T6 focus ACK barrier; `test-util` ctors |
| **#10 viewport-safe cursor (I5)** | T2 `Option<CursorPos>` fill — never `unwrap_or(0)` |
| **#11 benches/inspect** | T6 |
| **#12 cargo test filter** | All tasks use one filter per invocation |

**Placeholder scan:** no TBD; Focus/`access_epoch` shape stable from T1; no Task 0 — all declarations folded into T1/T2/T5; API signatures cited earlier remain unchanged.
