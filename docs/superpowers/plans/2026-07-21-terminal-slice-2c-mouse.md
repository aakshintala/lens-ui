# Terminal Slice 2c — Mouse (capture → selection + copy + reporting) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
>
> **Revision 3 (2026-07-21):** folds a second codex/gpt-5.6 delta re-review (Rev 2 → REVISE-BEFORE-EXECUTE; 3 Critical + 10 Important; **architecture accepted, binding audit confirmed correct against vendor source**). Rev 3 fixes the residual defects — see the **Revision history** appendix for the finding→fix map. Headline changes vs Rev 2: (C1) **the `Frame` mode hint is deleted** — the foreground forwards **every** mouse event with **no** mode decision, and a local click's hyperlink action is triggered by an engine-emitted `LocalClick` presentation event, not a stale snapshot; (C2) the gesture latch stores **its epoch + a suppression flag** and every Report event re-checks **current epoch AND `write_allowed`** (closes a read-only egress bypass); (C3) **wheel is a separate atomic command** that never touches the latch; plus Left-only selection, matching-button + all-button up-out, `Result`-returning selection mutations, `set_time` multi-click, a `MouseAck` on **every** command branch, self-contained task staging, and the real `VtEngine` shapes.

**Goal:** Make the terminal mouse-interactive: build the foreground mouse-capture stack **once** and fan it into (a) **local text selection + `Cmd+C` copy + `Cmd+A` select-all**, (b) **mouse-event reporting to the PTY** — with report-vs-select arbitration, gesture-owner latching, and format-aware motion coalescing all decided **engine-side at each command's ordered-stream position** against live terminal modes.

**Architecture (Option A spine).** The foreground is a **thin lowering layer with zero mode logic**: it captures raw GPUI mouse events (down/move/up for Left/Middle/Right, in and out of the hitbox, plus wheel), maps pixel→cell (`hit_test::pixel_to_cell`, `None` outside the grid), stamps a monotonic `time`, and **immediately** sends `EngineCommand::MouseGesture` / `EngineCommand::Wheel` / `SelectAll` / `Copy` down the **single ordered command stream** through the never-drop `InputForwarder`. It makes **no** report-vs-select decision and never inspects terminal modes. **The engine decides everything mode-dependent** on the thread that owns the non-`Send` `Terminal`: at a **Down** it reads **live** tracking/format, combines them with the **foreground-authoritative** flags carried on the command (shift, mouse-local toggle, report policy, `write_allowed`/access) and the **latched epoch**, decides + latches `{ owner, button, epoch, suppressed }`, and holds that owner through the **matching-button** release. Report → `mouse::Encoder` (epoch+`write_allowed` re-checked on every event, `try_emit_user_input(EgressKind::Input)`, `MouseAck`; format-aware coalescing via the encoder's own `set_track_last_cell`). Select (**Left only**) → libghostty's `selection::Gesture` machine → `set_selection` → the `Frame.selected` field 1c already paints; a **click with no drag** makes the engine emit `EnginePresentationEvent::LocalClick { col, row }`, which the foreground turns into its existing 2d `uri_for_gesture` → `OpenUrlRequest`. **Wheel** is atomic: the engine reports each notch as `Button::Four/Five` under live tracking + auth, else calls `local_scroll` — it never touches the gesture latch. **Copy** extracts `format_selection_alloc` off the publish hot path via an async capacity-1 responder the foreground never `recv`s (a background-executor task `recv`s, then a foreground `update` writes the clipboard).

**Why the split is principled:** terminal **modes** are engine-authoritative (read live at stream position — never a foreground snapshot); **access/shift/policy/toggle** are foreground-authoritative (the foreground owns `AccessMode` via lifecycle and the toggle/config) so they travel **on** the command and are read by the engine at the Down. This restores the design's ordered-stream live-mode guarantee (design lines 57–82).

**Tech Stack:** `libghostty-vt` (`mouse::{Encoder,Event,Action,Button,Format,EncoderSize,Position}`, `selection::{Selection,FormatOptions}` + `selection::gesture::{Gesture,PressEvent,DragEvent,ReleaseEvent,Behavior,Behaviors,Geometry}`, `terminal::{Point,PointCoordinate,Mode}`, `Terminal::{mode,grid_ref,set_selection,select_all,format_selection_alloc}`), gpui (`MouseButton`, `MouseDownEvent`/`MouseMoveEvent`/`MouseUpEvent`/`ScrollWheelEvent`, `on_mouse_down`/`on_mouse_move`/`on_mouse_up`/`on_mouse_up_out`/`on_scroll_wheel`, `cx.write_to_clipboard`, `background_executor`, `EventEmitter`), `crossbeam-channel`, `std::time::{Duration,Instant}`.

## Serial position (2c is FOURTH / LAST — lands after 2b)

Prerequisite: **2a + 2d + 2b committed on `terminal-ws`** (HEAD `e29a501`+). 2c edits their real code directly — no placeholders, no merge. Per the **2026-07-20 spec amendment** (`docs/specs/2026-07-17-terminal-slice-2-interaction-design.md`), local selection + `Cmd+C` copy **moved from 2b into 2c**.

### XTSHIFTESCAPE spike — RESOLVED (2026-07-21): no C-ABI accessor → config-only arbitration, program-override DEFERRED

Verified against the vendored binding **and** Ghostty upstream HEAD (2026-07-21): `mouse_shift_capture` (XTSHIFTESCAPE `CSI > Pp s`) is **not** in the C ABI (internal Zig ternary only; not a DEC mode; `MOUSE_TRACKING`(11) get-enum is the only mouse getter). 2c ships the fallback: arbitrate Shift-vs-report on config policy + readable DEC tracking modes; Shift+drag forces local selection; the runtime toggle forces mouse-local. **DEFER** honoring a program's `CSI > Pp s` override (parent-matrix note, Task 8). Re-vendor trigger: a `GHOSTTY_TERMINAL_DATA_MOUSE_SHIFT_CAPTURE` upstream. Memory: `terminal-2c-xtshiftescape-not-in-c-abi`.

### Surfaces 2a/2d/2b already built that 2c reuses (do NOT rebuild)

| Surface | Where | 2c use |
| --- | --- | --- |
| `hit_test::pixel_to_cell(origin, &CellMetrics, position, cols, rows) -> Option<(u16,u16)>` | `hit_test.rs:9` | cell mapping (as-is) |
| `hit_test::uri_for_gesture(&Frame, col, row) -> Option<String>` (2d) | `hit_test.rs:48` | `LocalClick` → URL |
| `FrameCell.selected: bool` + `SELECTION_BG` paint | `engine/frame.rs:45`, `render/paint.rs:26,95` | selection **render** done; 2c drives engine selection state only |
| `EngineCommand::Key` arm (epoch-gate → encode → `try_emit_user_input(EgressKind::Input)` → ack) | `engine/worker.rs:511-543` | the report branch mirrors it |
| `try_emit_user_input(egress, EgressKind::Input, &bytes)` (`try_send`; `false` on Full/Disconnected → inspect counter) | `engine/worker.rs:713-725` | report egress — see **egress-seam note** |
| `InputForwarder` (off-fg, never-drop retry, never-block-fg) + `enqueue_input` epoch stamping + `is_stale` | `engine/forwarder.rs`, `engine/handle.rs:140` | route mouse/wheel/select/copy off-foreground |
| `local_scroll(ScrollDelta)` (`LocalScroll` arm) | `engine/worker.rs:604`, `engine/vt.rs` | wheel local-scroll fallback |
| `write_input_allowed(access, input_enabled)` + `TerminalTab::write_input_allowed()` | `input_gate.rs`, `lib.rs:805` | source of the `write_allowed` flag carried on commands |
| Cmd+V intercept (`platform && key=="v"` top of `handle_key_down`, `cx.stop_propagation()`) | `lib.rs:873-910,1066` | Cmd+C / Cmd+A mirror it |
| OSC-52 drain → `cx.write_to_clipboard(ClipboardItem::new_string(...))` | `lib.rs:1354` | copy write |
| `on_mouse_down(Left)` hyperlink handler + `on_scroll_wheel` + `gpui_scroll_to_lens` line conversion | `render/state.rs:114-118`, `lib.rs:770,840` | extend; reuse wheel line conversion |
| `EnginePresentationEvent` channel + `drain_presentation_events` (title/hyperlink/OSC52) | `engine/presentation.rs`, `lib.rs:1270` | add `LocalClick` variant + drain arm |
| `presentation_realwindow.rs` / `input_realwindow.rs` harnesses (`last_paint_origin_for_test`, `cell_metrics_for_test`, `debug_mouse_down_for_test`, `latest_frame_for_test`, `attach_test_egress`, `await_single_egress`) | `tests/*` | template for `mouse_realwindow.rs` |
| `EngineInspect` counter + `enabled`-gated `record_*` pattern | `engine/inspect.rs` | mouse/copy counters |
| `engine_bench_api::encode_paste_bench` + `benches/engine.rs` | `lib.rs:74`, `benches/engine.rs` | `encode_mouse_bench` + benches |
| `VtEngine { cell_w_px:u32, cell_h_px:u32, key_encoder, .. }` (**no `cols()`/`rows()`; `build_frame(&mut) -> Result<Frame,EngineError>`; `new(&EngineConfig, on_reply, presentation_tx)`**), `EngineConfig { cols, rows, cell_w_px, cell_h_px }` | `engine/vt.rs:34-75,294,301` | Task 2 adds `cols`/`rows` fields |

**Egress-seam note (folds Rev-1 C1):** the committed report path (`try_emit_user_input`) uses `try_send` and only increments `user_egress_rejected` on a full/closed egress channel — a **shared, pre-existing property of the Key/Paste path**, NOT introduced by 2c. The *never-drop* guarantee 2c relies on is the **`InputForwarder` foreground→engine** hop. The **engine→egress-channel** hop behaves **identically** to Key/Paste; the bridge's own outbound-saturation policy (C2-era per-transport egress → visible reconnect) governs downstream. **2c does not re-architect this seam** — the report/wheel arms behave exactly like the committed Key arm, claim no stronger guarantee, and a Task-3 test asserts a report under a full egress channel yields the **same** outcome as a Key (rejected + inspect + `MouseAck` disposition `Reported`-but-not-delivered). A stronger never-drop-or-visible-reject is a separately-tracked cross-cutting egress change.

## Global Constraints

- **Per-task `lens-terminal` clippy MUST include `--features test-util,live-tests`:** `cargo clippy -p lens-terminal --all-targets --features test-util,live-tests -- -D warnings`. Workspace gate: `cargo clippy --workspace --all-targets -- -D warnings` + `cargo fmt --check`. **Never pipe gate/clippy/test output through `tail`.** **`cargo test` takes ONE positional filter** — use a module filter or separate commands; never `-- name1 name2`.
- **The foreground makes NO mode decision and forwards every mouse event.** No `Frame` mode hint exists. Terminal modes are read **live** on the engine thread inside the command handler. No Ghostty type crosses the seam.
- **Single ordered stream + never-block-fg + never-drop.** `MouseGesture`/`Wheel`/`SelectAll`/`Copy` ride the ordered `EngineCommand` channel **through the `InputForwarder`** (off-foreground, never-drop retry). The foreground **never** does a blocking `cmd_tx.send`. `MouseGesture`/`Wheel` are epoch-**stamped** but **NOT** forwarder-`is_stale`-revoked (a stale gesture may be a legitimate local selection; the *engine's report branch* enforces epoch). `SelectAll`/`Copy` carry no epoch.
- **Gesture-owner latching with epoch + suppression (folds Rev-2 C2, C3, I4).** The engine decides `Report{button}`|`Select` **at Down** and latches `{ owner, button, epoch, suppressed }`, holding it through the release of the **same button**. A non-matching Up **retains** the latch and acks a no-op. An Up with no latch acks a no-op. **Every Report event re-checks the *current* `access_epoch` AND the event's `write_allowed`;** if either fails the latch's `suppressed` flag is set and all remaining Report bytes through the matching Up are dropped (a teardown/reconnect that bumps the epoch therefore cannot leak reports from an in-flight latched gesture). Shift/policy/toggle/access changes affect only the **next** gesture — never split a press(report)→release(select). **Wheel never touches the latch.**
- **Only Left selects text (folds Rev-2 I5).** Under a Select decision, only `MouseButtonKind::Left` drives `apply_selection_*`. A non-tracked Right/Middle Down is an explicit no-op (acked `Ignored`).
- **Read-only suppresses reporting, allows select/copy. Access is ENGINE-authoritative via an ordered `SetAccess` command (folds Task-3 review Critical).** `write_allowed` is **NOT** carried on `MouseGesture`/`WheelInput` — a foreground-captured flag can be laundered into a post-downgrade epoch (stale `write_allowed=true` + freshly-stamped current epoch → reports in read-only). Instead the foreground sends `EngineCommand::SetAccess(bool)` through the **same ordered forwarder** on open and on every access change; the engine stores `write_allowed` in worker state and reads it **at the command's stream position**. The engine reports iff `engine.write_allowed && cmd_epoch == current_epoch && live_tracking != None && !shift && !mouse_local && policy == Auto`. The `access_epoch` check still revokes gestures queued **before** a teardown/downgrade bump (in-flight revocation); ordered `SetAccess` governs **steady-state** read-only. Under read-only a gesture over active tracking becomes **local selection** (Left) or no-op (Right/Middle). Copy/SelectAll never gated.
- **Copy off the publish hot path (Design Point 4).** `format_selection_alloc` runs inside the `Copy` handler; the foreground consumes via an async **capacity-1** responder it never `recv`s — a **background-executor** task does the blocking `recv`, then a **foreground `weak.update`/`cx.update`** writes the clipboard. Never block the GPUI foreground with a `recv`; never touch GPUI from the bg task. An in-flight `format_selection_alloc` FFI call is **non-cancellable** (bounded by scrollback; rare one-frame hitch accepted per DP4); `Stop` preempts only **between** commands (`worker.rs:349-407`).
- **Coalescing is engine-side with explicit reset state (folds Rev-2 I11, I15).** In the report branch: `mouse_encoder.set_track_last_cell(live_format != Format::SgrPixels)` + `set_any_button_pressed(<latched button live>)` before every encode. The worker stores `coalesce_state: { owner, tracking, format, mods }`; when **any** of those changes vs. the previous report event it calls `mouse_encoder.reset()` (clears the encoder's last-cell dedup) **before** encoding, so a SGR-pixels→cell transition or a modifier change cannot inherit stale dedup. Same-cell motion under a cell format then encodes empty → no egress (count coalesced). Press/Release never coalesce.
- **Per-tracking-mode motion semantics** (binding distinguishes them, `mouse.rs:291-305`): X10=press only; Normal(1000)=press+release; Button(1002)=motion only with a button latched; Any(1003)=all motion. The engine implements these explicitly via the latched button + `set_any_button_pressed`; it does not rely on the encoder to silently drop mis-routed events.
- **Wheel atomic (folds Rev-2 C3).** `EngineCommand::Wheel { lines: i32, col, row, px_x, px_y, mods, write_allowed, access_epoch, ack }`. Under `write_allowed && live_tracking != None`: report `|lines|` notches, each a `Button::Four` (lines<0 = down → Five, sign per `gpui_scroll_to_lens`) Press. Else: `engine.local_scroll(ScrollDelta::Lines(lines))`. Never modifies the gesture latch. Acks `Reported`/`ScrolledLocal`.
- **Multi-click via `set_time` (folds Rev-2 I8).** `MouseGesture` carries a monotonic `time: Duration` (foreground stamps from an `Instant` base; tests pass it explicitly). Every selection Press calls `press_event.set_time(g.time)` so the gesture machine derives single/double/triple-click Cell/Word/Line behavior itself. **No** foreground `click_count`.
- **`MouseAck` on every branch (folds Rev-2 I9).** `MouseGesture.ack`/`Wheel.ack` = `Option<Sender<MouseAck>>` where `MouseAck { encoded: Vec<u8>, disposition: GestureDisposition }` and `GestureDisposition { Reported, Selected, LocalClick, Suppressed, Ignored, Coalesced, ScrolledLocal }`. **Every** command branch sends exactly one `MouseAck` (report, select, coalesced-empty, suppressed, ignored, no-latch-up, wheel) so DP1 tests never hang. `InputAck` (Key/Paste) is untouched.
- **Bracketed-paste warn stays conservative (folds Rev-1 C17).** 2c does NOT touch the 2b multiline-paste warn (no stale-frame suppression). The 2b always-warn deferral **remains deferred**; note in Task 8 docs.
- **Do not break the 2d hyperlink click.** Hyperlink-open is foreground-owned but **triggered by the engine's `LocalClick`** classification (a Left click with no drag under no live tracking), never a foreground mode guess. The foreground drains `LocalClick { col, row }` → `uri_for_gesture(frame, col, row)` → `OpenUrlRequest`. Selection state is not mutated by a plain click (a Left Down installs at most a zero-width selection the click's engine handling clears on `LocalClick`).
- **No Ghostty type escapes the engine boundary.** Lens-owned `MouseGesture`/`MouseButtonKind`/`MouseEventKind`/`MouseReportPolicy`/`MouseReportEv`/`GestureOwner`/`MouseAck`/`GestureDisposition`/`CopyResponder`/`CopyResult` only.
- **`#[gpui::test]` / `NoopTextSystem` false-greens painted-window paths** (memory `gpui-test-noop-text-system`). 2c correctness is **hermetic**: engine-thread golden via `vt_write` DECSET + command push + **bounded-`MouseAck` `recv_timeout`** (NO sleeps, NO frame-polling for sync — Design Point 1). Real-window `mouse_realwindow.rs` is the end-of-slice proof. **Any real-window run: READ memory `terminal-realwindow-harness-pitfalls` FIRST**, user heads-up (opens macOS windows).
- **Every task's shown gate must compile and pass standalone** — no forward dependency on a type/method a later task creates (folds Rev-2 I10, I12). DTOs, counters, and the `mouse_local`/`report_policy` fields land in the first task that references them.
- Ground truth: `docs/specs/2026-07-17-terminal-slice-2-interaction-design.md`; **2a/2d/2b committed code**; APIs in `lib.rs`, `engine/{vt,worker,command,frame,forwarder,handle,inspect,presentation}.rs`, `hit_test.rs`, `input_gate.rs`, `render/{state,metrics,paint}.rs`, `vendor/libghostty-rs/libghostty-vt/src/{terminal,mouse,selection}.rs` + `selection/gesture.rs` + `fmt.rs`.

---

## Verified binding signatures (authoritative — vendor-sourced; code below compiles against these)

```rust
// terminal.rs
pub fn mode(&self, mode: Mode) -> Result<bool>;                       // 355-  (live mode read)
pub fn grid_ref(&self, point: Point) -> Result<GridRef<'_>>;          // 378-384  ONE Point, Result
pub fn set_selection(&self, selection: Option<&Selection<'_>>) -> Result<&Self>;  // 210-227
pub fn select_all(&self) -> Result<Option<Selection<'_>>>;
pub enum Point { Active(PointCoordinate), Viewport(PointCoordinate), Screen(_), History(_) }   // 752-
pub struct PointCoordinate { pub x: u16, pub y: u32 }                 // 820  0-indexed col/row
// selection.rs
pub fn format_selection_alloc<'a,'ctx:'a>(&self, alloc: Option<&'a Allocator<'ctx>>, options: FormatOptions) -> Result<Option<Bytes<'a>>>;  // 367-390, Ok(None) when no selection
pub struct FormatOptions<'t,'s>;  // ::default().with_unwrap(true).with_trim(true)   547-588 (Ghostty copy semantics)
// selection/gesture.rs
impl PressEvent  { fn new()->Result<Self>; fn set_position(&mut self,x:f64,y:f64)->Result<&mut Self>; fn set_time(&mut self,Duration)->Result<&mut Self>; fn set_behaviors(&mut self,&Behaviors)->Result<&mut Self>; fn apply<'t>(&mut self,&mut Gesture,&'t Terminal,GridRef<'t>)->Result<Option<Selection<'t>>>; }  // 326-334
impl DragEvent   { fn new()->Result<Self>; fn set_position(&mut self,x:f64,y:f64)->Result<&mut Self>; fn apply<'t>(&mut self,&mut Gesture,&'t Terminal,GridRef<'t>,geometry:Geometry)->Result<Option<Selection<'t>>>; }  // 451-462
impl ReleaseEvent{ fn new()->Result<Self>; fn apply<'t>(&mut self,&mut Gesture,&'t Terminal,Option<GridRef<'t>>)->Result<()>; }  // 367-382  returns ()
pub use ffi::SelectionGestureGeometry as Geometry;  // { columns:u32, cell_width:u32, padding_left:u32, screen_height:u32 } — columns/cell_width/screen_height must be non-zero
pub enum Behavior { Cell, Word, Line, Output }
impl Behaviors { fn new()->Self; fn with_single_click_behavior(self,Behavior)->Self; fn with_double_click_behavior(self,Behavior)->Self; fn with_triple_click_behavior(self,Behavior)->Self; }
impl Gesture { fn dragged(&self,&Terminal)->Result<bool>; }   // detect click-vs-drag for LocalClick
// The reusable PressEvent<'static>/DragEvent<'static>/ReleaseEvent<'static> fields are VALID: their
// lifetime is the ALLOCATOR lifetime, not a borrow of Terminal (gesture.rs:224-247). Their apply()
// borrows &terminal per call. But apply() takes &mut event AND &terminal AND &mut gesture at once —
// so store these as SEPARATE VtEngine fields and split-borrow via destructuring (see Task 2 note).
// mouse.rs
impl Encoder { fn new()->Result<Self>; fn set_options_from_terminal(&mut self,&Terminal)->&mut Self; fn set_size(&mut self,EncoderSize)->&mut Self; fn set_any_button_pressed(&mut self,bool)->&mut Self; fn set_track_last_cell(&mut self,bool)->&mut Self; fn reset(&mut self); fn encode_to_vec(&mut self,&Event,&mut Vec<u8>)->Result<()>; }  // 74-178
impl Event   { fn new()->Result<Self>; fn set_action(&mut self,Action)->&mut Self; fn set_button(&mut self,Option<Button>)->&mut Self; fn set_mods(&mut self,key::Mods)->&mut Self; fn set_position(&mut self,Position)->&mut Self; }
pub struct Position { pub x: f32, pub y: f32 }   // 27  surface pixels
pub enum Action { Press, Release, Motion }
pub enum Button { Unknown, Left, Right, Middle, Four, Five, .. }   // Four=wheel-up, Five=wheel-down
// Mode consts: X10_MOUSE(9) NORMAL_MOUSE(1000) BUTTON_MOUSE(1002) ANY_MOUSE(1003) SGR_MOUSE(1006) SGR_PIXELS_MOUSE(1016) UTF8_MOUSE(1005) URXVT_MOUSE(1015)
// VtEngine reality: NO cols()/rows() today; build_frame(&mut self)->Result<Frame,EngineError>; new(&EngineConfig, on_reply, presentation_tx)->Result<Self,_>; resize(&mut self, cols, rows).
```

---

## File Structure

- `crates/lens-terminal/src/engine/command.rs` — **modify**: `MouseGesture`, `MouseButtonKind`, `MouseEventKind`, `MouseReportPolicy`, `MouseReportEv` (engine adapter), `GestureOwner`, `MouseAck`, `GestureDisposition`, `CopyResponder`/`CopyResult`, `WheelInput`.
- `crates/lens-terminal/src/engine/presentation.rs` — **modify**: `EnginePresentationEvent::LocalClick { col, row }`.
- `crates/lens-terminal/src/engine/vt.rs` — **modify**: `cols`/`rows` fields (set in `new_shared`, updated in `resize`); `mouse_encoder`/`mouse_event`/`selection_gesture`/`press_event`/`drag_event`/`release_event` fields; `read_live_tracking`/`read_live_format`; `encode_mouse_report`; `apply_selection_press/drag/release` (→ `Result<bool>`), `select_all`, `clear_selection`, `extract_selection_text`, `gesture_dragged`.
- `crates/lens-terminal/src/engine/worker.rs` — **modify**: `EngineCommand::{MouseGesture,Wheel,SelectAll,Copy}` + `handle_command` arms; latch + coalesce state on the worker loop struct; emit `LocalClick`.
- `crates/lens-terminal/src/engine/forwarder.rs` — **modify**: `is_stale` false for the new variants.
- `crates/lens-terminal/src/engine/handle.rs` — **modify**: `enqueue_input` stamps `MouseGesture`/`Wheel` epoch; `enqueue_mouse_gesture`/`enqueue_wheel`/`select_all`/`request_copy` route through the forwarder.
- `crates/lens-terminal/src/engine/inspect.rs` — **modify**: mouse/copy counters.
- `crates/lens-terminal/src/lib.rs` — **modify**: element mouse handlers, `handle_mouse_*`, `handle_wheel`, `LocalClick` drain, Cmd+C/Cmd+A intercepts, async copy task, `mouse_local`/`report_policy` state + toggle, monotonic time base, test hooks.
- `crates/lens-terminal/src/render/state.rs` — **modify**: register `on_mouse_down/up(Left/Middle/Right)`, `on_mouse_move`, `on_mouse_up_out(Left/Middle/Right)`.
- `crates/lens-terminal/benches/engine.rs` + `lib.rs` (`engine_bench_api`) — **modify**: `encode_mouse_bench` + benches.
- `crates/lens-terminal/tests/mouse_realwindow.rs` — **create** (+ `Cargo.toml` `[[test]]`).
- `crates/lens-terminal/tests/terminal_live.rs` — **modify**: P6 live mouse-report rider.
- `docs/specs/2026-07-16-terminal-workstream-design.md` **and** `docs/specs/2026-07-17-terminal-slice-2-interaction-design.md` — **modify**.

> **Test-engine helper:** the `vt.rs` `#[cfg(test)]` module already builds engines (see `build_frame_cursor_none...` at `vt.rs:591`). **Reuse that existing constructor** (3-arg `VtEngine::new(&cfg, on_reply, presentation_tx)`); do not invent `test_config()`. `build_frame()` returns `Result` → `.expect("frame")`.

---

## Task 1: Shared DTOs + `LocalClick` presentation variant

**Files:** Modify `engine/command.rs`, `engine/presentation.rs`. Test: a trivial `#[test]` constructing each DTO (compile gate) or fold into Task 2 (no runtime behavior yet — this task is type scaffolding, so its "gate" is `cargo build -p lens-terminal --all-targets` + clippy).

**Interfaces (Produces — all `Send`, Lens-owned):**
```rust
// command.rs
#[derive(Clone, Copy, Debug, Eq, PartialEq)] pub(crate) enum MouseButtonKind { Left, Right, Middle }
#[derive(Clone, Copy, Debug, Eq, PartialEq)] pub(crate) enum MouseEventKind { Down, Move, Up }
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)] pub(crate) enum MouseReportPolicy { #[default] Auto, ForceLocal }
#[derive(Clone, Copy, Debug, Eq, PartialEq)] pub(crate) enum GestureOwner { Report, Select }
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GestureDisposition { Reported, Selected, LocalClick, Suppressed, Ignored, Coalesced, ScrolledLocal }
#[derive(Clone, Debug, Eq, PartialEq)] pub struct MouseAck { pub encoded: Vec<u8>, pub disposition: GestureDisposition }
#[derive(Clone, Debug)]
pub(crate) struct MouseGesture {
    pub kind: MouseEventKind,
    pub button: Option<MouseButtonKind>,   // None only for buttonless Move
    pub mods: KeyMods,                     // includes shift
    pub cell: Option<(u16, u16)>,          // None when outside the grid (up-out / move-out)
    pub px_x: f32, pub px_y: f32,          // surface-relative pixels
    pub time: Duration,                    // monotonic (for set_time multi-click)
    pub write_allowed: bool,
    pub mouse_local: bool,
    pub policy: MouseReportPolicy,
    pub access_epoch: u64,                 // stamped by enqueue_input
    pub ack: Option<Sender<MouseAck>>,     // tests; None in production
}
#[derive(Clone, Debug)]
pub(crate) struct WheelInput {
    pub lines: i32,                        // signed, from gpui_scroll_to_lens
    pub cell: Option<(u16, u16)>, pub px_x: f32, pub px_y: f32, pub mods: KeyMods,
    pub write_allowed: bool, pub access_epoch: u64, pub ack: Option<Sender<MouseAck>>,
}
// Engine-internal adapter used by encode_mouse_report (built inside the worker arm from a MouseGesture/WheelInput):
#[derive(Clone, Copy, Debug)]
pub(crate) struct MouseReportEv { pub action: MouseEventKind, pub button: Option<MouseButtonKind>, pub wheel: Option<bool /*up*/>, pub mods: KeyMods, pub px_x: f32, pub px_y: f32, pub any_button_pressed: bool }
#[derive(Clone, Debug)] pub struct CopyResult { pub text: Option<String> }
pub(crate) type CopyResponder = crossbeam_channel::Sender<CopyResult>;
// presentation.rs
pub(crate) enum EnginePresentationEvent { /* ...existing... */ LocalClick { col: u16, row: u16 } }
```

- [ ] **Step 1:** Add all DTOs to `command.rs` (import `KeyMods`, `Sender`, `std::time::Duration`). Add the `LocalClick` variant to `EnginePresentationEvent` in `presentation.rs` (and any exhaustive match on it — grep `EnginePresentationEvent::` and add arms; the `collect_presentation_drain` in `presentation.rs` should ignore `LocalClick` at the engine collector level since it is drained foreground-side — mirror how `ClipboardWrite` is handled).
- [ ] **Step 2: gate** — `cargo build -p lens-terminal --all-targets && cargo clippy -p lens-terminal --all-targets --features test-util,live-tests -- -D warnings && cargo fmt`. Expected: clean (unused DTOs may need `#[allow(dead_code, reason = "consumed in Task 2/3")]` — add narrowly).
- [ ] **Step 3: commit**
```bash
git add crates/lens-terminal/src/engine/{command,presentation}.rs
git commit -m "feat(terminal-2c): mouse/wheel/copy DTOs + LocalClick presentation variant"
```

---

## Task 2: Engine mouse/selection capability layer on `VtEngine`

**Files:** Modify `engine/vt.rs`. Test: `vt.rs` engine goldens (reuse the existing test constructor).

**Interfaces (Produces on `VtEngine`):**
```rust
pub(crate) fn read_live_tracking(&self) -> MouseTracking;   // Any>Button>Normal>X10>None
pub(crate) fn read_live_format(&self) -> MouseFormat;        // SgrPixels>Sgr>Urxvt>Utf8>X10
pub(crate) fn encode_mouse_report(&mut self, ev: &MouseReportEv) -> Result<Vec<u8>, EngineError>;
pub(crate) fn apply_selection_press(&mut self, col:u16, row:u16, px_x:f32, px_y:f32, time:Duration) -> Result<bool, EngineError>;
pub(crate) fn apply_selection_drag(&mut self, col:u16, row:u16, px_x:f32, px_y:f32) -> Result<bool, EngineError>;
pub(crate) fn apply_selection_release(&mut self, cell: Option<(u16,u16)>) -> Result<(), EngineError>;
pub(crate) fn gesture_dragged(&self) -> bool;               // for LocalClick classification
pub(crate) fn select_all(&mut self) -> Result<bool, EngineError>;
pub(crate) fn clear_selection(&mut self) -> Result<bool, EngineError>;
pub(crate) fn extract_selection_text(&self) -> Option<String>;
```
`MouseTracking { None,X10,Normal,Button,Any }` + `MouseFormat { X10,Utf8,Sgr,Urxvt,SgrPixels }` (add to `command.rs` or `vt.rs`).

- [ ] **Step 1: add `cols`/`rows` fields.** In `VtEngine`, add `cols: u16, rows: u16`; set from `cfg.cols`/`cfg.rows` in `new_shared`; update in `resize(&mut self, cols, rows)` (the existing method). These back `EncoderSize`/`Geometry` (no `self.cols()` method — use the fields directly).

- [ ] **Step 2: failing goldens** (three commands, ONE filter each):
```rust
#[test]
fn encode_mouse_report_sgr_press_left() {
    let mut e = /* existing test constructor */;
    e.feed(b"\x1b[?1000h\x1b[?1006h");
    let bytes = e.encode_mouse_report(&MouseReportEv { action: MouseEventKind::Down, button: Some(MouseButtonKind::Left), wheel: None, mods: KeyMods::default(), px_x: 0.0, px_y: 0.0, any_button_pressed: true }).expect("encode");
    assert_eq!(bytes, b"\x1b[<0;1;1M");   // adjust to the encoder's real 1-based output
}
#[test]
fn selection_press_drag_release_marks_and_extracts() {
    let mut e = /* ctor */; e.feed(b"copyme");
    assert!(e.apply_selection_press(0,0,0.0,0.0,Duration::ZERO).expect("press"));
    assert!(e.apply_selection_drag(3,0,24.0,0.0).expect("drag"));
    e.apply_selection_release(Some((3,0))).expect("release");
    assert_eq!(e.extract_selection_text().as_deref(), Some("copy"));
    let cols: Vec<u16> = e.build_frame().expect("f").grid[0].cells.iter().filter(|c| c.selected).map(|c| c.col).collect();
    assert!(cols.contains(&0) && cols.contains(&3), "got {cols:?}");
    assert!(e.clear_selection().expect("clear"));
    assert_eq!(e.extract_selection_text(), None);
    assert!(e.build_frame().expect("f").grid[0].cells.iter().all(|c| !c.selected));
}
#[test]
fn select_all_extract_and_double_click_word() {
    let mut e = /* ctor */; e.feed(b"foo bar");
    assert!(e.select_all().expect("all"));
    assert_eq!(e.extract_selection_text().as_deref().map(str::trim), Some("foo bar"));
    e.clear_selection().expect("clear");
    // double-click on "bar" (two timed presses) selects the word
    e.apply_selection_press(4,0,32.0,0.0,Duration::from_millis(0)).expect("p1");
    e.apply_selection_release(Some((4,0))).expect("r1");
    e.apply_selection_press(4,0,32.0,0.0,Duration::from_millis(120)).expect("p2");
    assert_eq!(e.extract_selection_text().as_deref(), Some("bar"));
}
```

- [ ] **Step 3: run → FAIL** — `cargo test -p lens-terminal --lib encode_mouse_report_sgr_press_left` (then the other two, one filter each).

- [ ] **Step 4: fields + reads.** Add `mouse_encoder: mouse::Encoder<'static>`, `mouse_event: mouse::Event<'static>`, `selection_gesture: gesture::Gesture<'static>`, `press_event: gesture::PressEvent<'static>`, `drag_event: gesture::DragEvent<'static>`, `release_event: gesture::ReleaseEvent<'static>` (all `::new()?` in `new_shared`). `read_live_tracking`/`read_live_format` via `self.terminal.mode(...)` (priority order per constraints).

- [ ] **Step 5: `encode_mouse_report`.**
```rust
pub(crate) fn encode_mouse_report(&mut self, ev: &MouseReportEv) -> Result<Vec<u8>, EngineError> {
    use libghostty_vt::mouse::{Action, Button, EncoderSize, Position};
    self.mouse_encoder.set_options_from_terminal(&self.terminal);
    self.mouse_encoder.set_size(EncoderSize {
        screen_width: self.cell_w_px.saturating_mul(u32::from(self.cols)),
        screen_height: self.cell_h_px.saturating_mul(u32::from(self.rows)),
        cell_width: self.cell_w_px, cell_height: self.cell_h_px,
        padding_top:0, padding_bottom:0, padding_right:0, padding_left:0,
    });
    self.mouse_encoder.set_any_button_pressed(ev.any_button_pressed);
    self.mouse_encoder.set_track_last_cell(self.read_live_format() != MouseFormat::SgrPixels);
    let action = match ev.action { MouseEventKind::Down => Action::Press, MouseEventKind::Up => Action::Release, MouseEventKind::Move => Action::Motion };
    let button = match ev.wheel { Some(true) => Some(Button::Four), Some(false) => Some(Button::Five), None => ev.button.map(|b| match b { MouseButtonKind::Left=>Button::Left, MouseButtonKind::Middle=>Button::Middle, MouseButtonKind::Right=>Button::Right }) };
    self.mouse_event.set_action(action);
    self.mouse_event.set_button(button);
    self.mouse_event.set_mods(mods_to_ghostty(ev.mods));  // reuse the key encoder's mods mapping (grep vt.rs)
    self.mouse_event.set_position(Position { x: ev.px_x, y: ev.px_y });
    let mut out = Vec::new();
    self.mouse_encoder.encode_to_vec(&self.mouse_event, &mut out)?;
    Ok(out)  // empty on no-tracking OR coalesced same-cell motion
}
```

- [ ] **Step 6: selection with split-borrow + `Result`.** `Geometry`/`grid_ref` need `&self.terminal` while `apply` needs `&mut self.{press,drag,release}_event` + `&mut self.selection_gesture` — **destructure the fields into locals** so the borrows don't overlap `&self`:
```rust
pub(crate) fn apply_selection_press(&mut self, col:u16, row:u16, px_x:f32, px_y:f32, time:Duration) -> Result<bool, EngineError> {
    use libghostty_vt::terminal::{Point, PointCoordinate};
    use libghostty_vt::selection::gesture::{Behavior, Behaviors};
    let Self { terminal, press_event, selection_gesture, .. } = self;   // split borrow
    let gref = terminal.grid_ref(Point::Viewport(PointCoordinate { x: col, y: u32::from(row) }))?;
    let behaviors = Behaviors::new().with_single_click_behavior(Behavior::Cell)
        .with_double_click_behavior(Behavior::Word).with_triple_click_behavior(Behavior::Line);
    press_event.set_behaviors(&behaviors)?;
    press_event.set_position(f64::from(px_x), f64::from(px_y))?;
    press_event.set_time(time)?;
    let sel = press_event.apply(selection_gesture, terminal, gref)?;
    terminal.set_selection(sel.as_ref())?;
    Ok(true)   // press handled; dirty (selection may be empty until drag / cleared by LocalClick)
}
pub(crate) fn apply_selection_drag(&mut self, col:u16, row:u16, px_x:f32, px_y:f32) -> Result<bool, EngineError> {
    use libghostty_vt::terminal::{Point, PointCoordinate};
    use libghostty_vt::selection::gesture::Geometry;
    let (cols, rows, cw, ch) = (self.cols, self.rows, self.cell_w_px, self.cell_h_px);
    let Self { terminal, drag_event, selection_gesture, .. } = self;
    let gref = terminal.grid_ref(Point::Viewport(PointCoordinate { x: col, y: u32::from(row) }))?;
    let geom = Geometry { columns: u32::from(cols).max(1), cell_width: cw.max(1), padding_left: 0, screen_height: ch.saturating_mul(u32::from(rows)).max(1) };
    drag_event.set_position(f64::from(px_x), f64::from(px_y))?;
    let sel = drag_event.apply(selection_gesture, terminal, gref, geom)?;
    terminal.set_selection(sel.as_ref())?;
    Ok(true)
}
pub(crate) fn apply_selection_release(&mut self, cell: Option<(u16,u16)>) -> Result<(), EngineError> {
    use libghostty_vt::terminal::{Point, PointCoordinate};
    let Self { terminal, release_event, selection_gesture, .. } = self;
    let gref = match cell { Some((c,r)) => Some(terminal.grid_ref(Point::Viewport(PointCoordinate { x:c, y:u32::from(r) }))?), None => None };
    release_event.apply(selection_gesture, terminal, gref)?;   // returns ()
    Ok(())
}
pub(crate) fn gesture_dragged(&self) -> bool { self.selection_gesture.dragged(&self.terminal).unwrap_or(false) }
pub(crate) fn select_all(&mut self) -> Result<bool, EngineError> {
    let sel = self.terminal.select_all()?;
    let installed = sel.is_some();
    self.terminal.set_selection(sel.as_ref())?;
    Ok(installed)
}
pub(crate) fn clear_selection(&mut self) -> Result<bool, EngineError> { self.terminal.set_selection(None)?; Ok(true) }
pub(crate) fn extract_selection_text(&self) -> Option<String> {
    use libghostty_vt::selection::FormatOptions;
    let opts = FormatOptions::default().with_unwrap(true).with_trim(true);
    match self.terminal.format_selection_alloc(None, opts) {
        Ok(Some(bytes)) => Some(String::from_utf8_lossy(bytes.as_ref()).into_owned()),
        _ => None,
    }
}
```
> **Executor:** confirm `Gesture::dragged` exists with this signature (recon §7 lists it); if the split-borrow still trips E0499 because `grid_ref` needs `&terminal` after `terminal` is moved into the destructure, take the `GridRef` **before** destructuring the event fields, or scope the `grid_ref` call in its own block returning an owned/tracked ref. Match the real borrow shape — the fields are independent so a field-level split borrow is always achievable.

- [ ] **Step 7: run → PASS** (all three, one filter each). 
- [ ] **Step 8: gate + commit**
```bash
git add crates/lens-terminal/src/engine/{vt,command}.rs
git commit -m "feat(terminal-2c): VtEngine mouse encoder + Result-returning selection gesture + copy extract"
```

---

## Task 3: `EngineCommand::MouseGesture` worker arm — arbitration, epoch+suppression latch, LocalClick, ack-every-branch

**Files:** Modify `engine/worker.rs` (variant + arm + latch/coalesce state), `engine/forwarder.rs` (`is_stale`→false), `engine/handle.rs` (`enqueue_input` stamp + `enqueue_mouse_gesture`), `engine/inspect.rs` (mouse counters). Test: engine-thread `MouseAck`-bounded goldens.

**Worker loop state (add to the struct that holds `access_epoch` etc.):**
```rust
struct MouseState {
    latch: Option<Latch>,                          // active gesture
    coalesce: Option<CoalesceKey>,                 // {owner,tracking,format,mods} of last report event
}
struct Latch { owner: GestureOwner, button: MouseButtonKind, epoch: u64, suppressed: bool, dragged: bool }
```

**Arm logic (at stream position; sends exactly one `MouseAck` on every path):**
```
Down (button b, cell, epoch e_cmd):
  let track = engine.read_live_tracking();
  let report = g.write_allowed && track != None && !g.mods.shift && !g.mouse_local && g.policy == Auto;
  if report {
     latch = Latch{Report,b,epoch:e_cmd,suppressed:false,dragged:false}; any_button=true;
     if e_cmd == current_epoch && g.write_allowed { emit_report(Down,b,..) -> ack Reported/Coalesced }
     else { latch.suppressed=true; ack Suppressed }
  } else if b == Left {
     latch = Latch{Select,Left,..}; engine.apply_selection_press(..)? ; dirty ; ack Selected
  } else { ack Ignored }   // Right/Middle, no tracking
Move (cell):
  match latch:
    Some(Report{..}) if button matches or motion:
        if suppressed { ack Suppressed }
        else if current_epoch != latch.epoch || !g.write_allowed { latch.suppressed=true; ack Suppressed }
        else { reset-coalesce-if-key-changed; emit_report(Move, any_button=true) -> empty? ack Coalesced : ack Reported }
        (respect per-mode: X10 no motion; Normal no motion; Button motion-with-button; Any all motion — if suppressed by mode, ack Ignored)
    Some(Select) => if cell.is_some() { engine.apply_selection_drag(..)?; latch.dragged=true; dirty } ; ack Selected
    None => if engine.read_live_tracking()==Any && g.write_allowed && current_epoch==e_cmd { emit buttonless Motion -> ack } else ack Ignored
Up (button b, cell):
  match latch where latch.button == b:
    Some(Report) => if !suppressed && ok { emit_report(Up) } ; latch=None; any_button=false; ack Reported/Suppressed
    Some(Select) => engine.apply_selection_release(cell)? ;
        if !latch.dragged { engine.clear_selection()?; emit LocalClick{col,row}; ack LocalClick } else { ack Selected }
        latch=None
    matching-none / non-matching button => keep latch; ack Ignored
```
The Report emit mirrors the committed `Key` arm (epoch pre-check inside the emit, `try_emit_user_input(EgressKind::Input)`, `inspect.record_mouse_encoded()` on non-empty / `record_mouse_report_coalesced()` on empty). `is_stale` false for `MouseGesture`. `enqueue_input` stamps `MouseGesture(g) => g.access_epoch = epoch`.

- [ ] **Step 1: failing `MouseAck`-bounded goldens** (DECSET feed, capacity-1 `MouseAck` ack, `recv_timeout` — DP1; NO sleeps): (a) tracking+`write_allowed` Down → ack `Reported` + SGR bytes; (b) no-tracking Left Down → ack `Selected`, frame cells selected, no egress; (c) `write_allowed=false` under tracking → ack `Selected` (report suppressed), no egress; (d) shift under tracking → `Selected`; (e) **latch epoch**: report Down then bump `access_epoch` (simulate downgrade) then Move → ack `Suppressed`, no egress (**read-only bypass closed**); (f) **matching button**: Left Down latched, Right Up → latch retained, ack `Ignored`; (g) **LocalClick**: no-tracking Left Down+Up with no Move → ack `LocalClick`; (h) no-tracking Right Down → ack `Ignored`, no selection; (i) **full-egress parity** (Rev-1 C1): fill egress, report Down → ack `Reported` but `user_egress_rejected` incremented (same as a sibling Key assertion). Use the existing ordered-ack test harness (2a/2b).

- [ ] **Step 2: run → FAIL**.
- [ ] **Step 3:** implement the variant, arm, `MouseState`, `is_stale`→false, `enqueue_input` stamp, `enqueue_mouse_gesture` (forwarder). Add `record_mouse_encoded`/`record_mouse_report_coalesced`/`record_mouse_suppressed` to `inspect.rs` **now** (land with first caller). Emit `LocalClick` via `presentation_tx`.
- [ ] **Step 4: run → PASS** (all cases).
- [ ] **Step 5: gate + commit**
```bash
git add crates/lens-terminal/src/engine/{worker,forwarder,handle,inspect,presentation}.rs
git commit -m "feat(terminal-2c): engine MouseGesture arbitration + epoch/suppression latch + LocalClick + ack-every-branch"
```

---

## Task 4: `EngineCommand::{Wheel,SelectAll,Copy}` + forwarder routing + async copy

**Files:** Modify `engine/worker.rs` (arms), `engine/handle.rs` (`enqueue_wheel`/`select_all`/`request_copy`), `engine/inspect.rs` (copy/wheel counters). Test: engine goldens.

**Arms:**
```rust
EngineCommand::Wheel(w) => {                     // atomic; never touches MouseState.latch
    let report = w.write_allowed && engine.read_live_tracking() != MouseTracking::None && w.access_epoch == current_epoch;
    if report {
        let up = w.lines > 0; let notches = w.lines.unsigned_abs();
        for _ in 0..notches { let bytes = engine.encode_mouse_report(&MouseReportEv{ action:MouseEventKind::Down, button:None, wheel:Some(up), mods:w.mods, px_x:w.px_x, px_y:w.py_y, any_button_pressed:false })?; try_emit_user_input(egress, Input, &bytes); }
        inspect.record_wheel_reported(); ack ScrolledLocal? -> ack Reported
    } else { engine.local_scroll(ScrollDelta::Lines(w.lines)); *dirty=true; *force_build=true; ack ScrolledLocal }
}
EngineCommand::SelectAll => { if engine.select_all()? { *dirty=true; *force_build=true; } }
EngineCommand::Copy(responder) => {
    inspect.record_copy_started();
    let text = engine.extract_selection_text();
    match &text { Some(_)=>inspect.record_copy_completed(), None=>inspect.record_copy_empty() }
    let _ = responder.try_send(CopyResult { text });   // capacity-1; discarded if fg dropped rx
}
```
`is_stale` false for all three. `Wheel` epoch-stamped (report gate); `SelectAll`/`Copy` no epoch. `EngineHandle::{enqueue_wheel, select_all, request_copy}` route through the forwarder; `request_copy` builds `bounded(1)` and returns `rx`.

- [ ] **Step 1: failing goldens:** wheel under tracking+auth → N SGR wheel reports egressed; wheel no-tracking → `local_scroll` invoked (assert viewport moved / inspect); wheel read-only under tracking → `local_scroll` (no egress); `request_copy` after a selection → `rx.recv_timeout` = `Some(text)`; empty selection → `Some(None)`; dropped rx before arm runs → no panic.
- [ ] **Step 2: run → FAIL**. **Step 3:** implement + handle methods + counters (`wheel_reported`, `copy_started/completed/empty`). **Step 4: run → PASS**.
- [ ] **Step 5: gate + commit**
```bash
git add crates/lens-terminal/src/engine/{worker,handle,inspect}.rs
git commit -m "feat(terminal-2c): atomic Wheel report/local-scroll + SelectAll + async capacity-1 Copy"
```

---

## Task 5: Foreground capture + lowering (all buttons/up-out, immediate send, LocalClick drain, Cmd+C/A copy)

**Files:** Modify `crates/lens-terminal/src/lib.rs` + `crates/lens-terminal/src/render/state.rs`. Test: `#[gpui::test]` where feasible; behavioral proof in Task 8.

- [ ] **Step 1: state + handlers.** Add to `TerminalTab`: `mouse_local: bool` (default false), `report_policy: MouseReportPolicy` (default `Auto`), `mouse_time_base: Instant` (set at open). *(Defining these here folds Rev-2 I10.)* In `render/state.rs` register (when `input: Some`): `on_mouse_down/up(MouseButton::{Left,Middle,Right})`, `on_mouse_move`, `on_mouse_up_out(MouseButton::{Left,Middle,Right})` → `tab.update(cx, |t,cx| t.handle_mouse_{down,move,up,up_out}(event, window, cx))`; keep `on_scroll_wheel` → `handle_wheel`.

- [ ] **Step 2: lowering (no deferral, no mode logic).** `handle_mouse_down/move/up/up_out`: compute `cell = pixel_to_cell(self.render.last_paint_origin(), &self.render.cell_metrics, event.position, frame.cols, frame.rows)` (`None` when outside), surface px = `event.position - last_paint_origin`, `time = self.mouse_time_base.elapsed()`, map GPUI `MouseButton`→`MouseButtonKind`, and build a `MouseGesture { write_allowed: self.write_input_allowed(), mouse_local: self.mouse_local, policy: self.report_policy, mods, .. }` sent **immediately** via `self.engine_handle().enqueue_mouse_gesture(g)`. `up_out` sends `kind: Up, cell: None`. No click-vs-drag or hyperlink decision here — the engine emits `LocalClick`.

- [ ] **Step 3: wheel + LocalClick drain.** `handle_wheel`: convert via the existing `gpui_scroll_to_lens` to signed lines → `self.engine_handle().enqueue_wheel(WheelInput { lines, cell, .., write_allowed })`. In `drain_presentation_events`, add a `LocalClick { col, row }` arm → `if let Some(url) = hit_test::uri_for_gesture(&frame, col, row) { cx.emit(TerminalEvent::OpenUrlRequest(url)) }` (reuse 2d validation).

- [ ] **Step 4: Cmd+C / Cmd+A + async copy.** At the top of `handle_key_down` (mirror Cmd+V): `platform && key=="c"` (no ctrl/alt/fn) → `self.handle_copy(cx)`, stop_propagation, return; `key=="a"` → `self.engine_handle().select_all()`, stop_propagation, return. `handle_copy`: `let rx = self.engine_handle().request_copy()?;` then a **two-stage** task — `cx.spawn(async move |this, cx| { let res = cx.background_executor().spawn(async move { rx.recv_timeout(COPY_TIMEOUT).ok() }).await; this.update(cx, |_t, cx| { if let Some(CopyResult{ text: Some(t) }) = res { if !t.is_empty() { cx.write_to_clipboard(ClipboardItem::new_string(t)); } } }).ok(); })`. Foreground never `recv`s inline; bg task never touches GPUI. Copy is allowed in read-only. Add `#[cfg(feature="test-util")]` `debug_mouse_{down,move,up,up_out}_for_test`, `debug_wheel_for_test`, `debug_handle_copy_for_test`, `debug_select_all_for_test`.

- [ ] **Step 5: gate + commit** (behavioral proof in Task 8; here assert compile + any `#[gpui::test]` emit checks).
```bash
git add crates/lens-terminal/src/lib.rs crates/lens-terminal/src/render/state.rs
git commit -m "feat(terminal-2c): foreground mouse lowering (all buttons/up-out, immediate send, LocalClick drain, Cmd+C/A)"
```

---

## Task 6: Mouse-local toggle + policy wiring + full arbitration coverage

**Files:** Modify `crates/lens-terminal/src/lib.rs`. Test: `#[gpui::test]` + engine goldens (reuse Task 3 harness).

- [ ] **Step 1:** `toggle_mouse_local(&mut self, cx)` flips `self.mouse_local` + `cx.notify()`; optional `set_report_policy`. (Fields already exist from Task 5.) Confirm every `MouseGesture`/`WheelInput` built in Task 5 reads `self.mouse_local`/`self.report_policy`/`self.write_input_allowed()`.
- [ ] **Step 2: failing tests** (engine goldens via Task 3 harness): `policy=ForceLocal` under tracking → `Selected`; `mouse_local=true` under tracking → `Selected`; `Auto`+no-shift+`write_allowed` under tracking → `Reported`; a `#[gpui::test]` asserting `toggle_mouse_local` flips the flag carried on the next gesture.
- [ ] **Step 3:** implement; ensure the flags are read by the **engine** arbiter (Task 3), not the foreground.
- [ ] **Step 4: run → PASS**. **Step 5: gate + commit**
```bash
git add crates/lens-terminal/src/lib.rs
git commit -m "feat(terminal-2c): mouse-local toggle + MouseReportPolicy carried to engine arbiter"
```

---

## Task 7: Per-mode motion semantics + coalescing reset + full Inspect + benches

**Files:** Modify `engine/worker.rs`/`vt.rs` (per-mode motion + coalesce reset), `engine/inspect.rs` (finalize counters), `benches/engine.rs` + `lib.rs`. Test: engine goldens.

- [ ] **Step 1: failing goldens:** (a) X10 → Move no report; Normal → Move no report, Up reports; Button → Move reports only with a button latched; Any → buttonless Move reports. (b) Sgr same-cell Move coalesced (empty → `Coalesced`); SgrPixels same-position Move always reports. (c) **coalesce reset**: a format transition Sgr→SgrPixels→Sgr does not inherit stale dedup (the second Sgr same-cell reports because `reset()` fired on the format change); a mods change (plain Move then Ctrl+Move same cell) reports (reset on mods change). (d) inspect counters increment.
- [ ] **Step 2: run → FAIL**. **Step 3:** implement per-mode gating (latched button + `read_live_tracking`) + the `coalesce: Option<CoalesceKey>` reset call (`mouse_encoder.reset()` when `{owner,tracking,format,mods}` changes). Finalize inspect counters (`mouse_encoded`, `mouse_reports_coalesced`, `mouse_suppressed`, `wheel_reported`, `copy_started/completed/empty`) with `enabled`-gating + add to any `EngineInspect` snapshot/serialization + default-asserts in existing inspect tests.
- [ ] **Step 4: run → PASS**.
- [ ] **Step 5: benches.** `engine_bench_api::encode_mouse_bench` (mirror `encode_paste_bench`) + `bench_mouse_encode_throughput` + `bench_mouse_motion_coalesced` in `benches/engine.rs`; record `build_frame` before/after (no new per-frame mode read now — the hint was dropped, so this should be a no-op confirm). Run `cargo bench -p lens-terminal --features bench mouse`.
- [ ] **Step 6: gate + commit**
```bash
git add crates/lens-terminal/src/engine/{worker,vt,inspect}.rs crates/lens-terminal/benches/engine.rs crates/lens-terminal/src/lib.rs
git commit -m "feat(terminal-2c): per-tracking-mode motion + coalesce-reset state + inspect + benches"
```

---

## Task 8: Real-window proof + live P6 + docs (both specs) + matrix

**Files:** Create `crates/lens-terminal/tests/mouse_realwindow.rs` (+ `Cargo.toml` `[[test]]`, `required-features=["test-util"]`); Modify `crates/lens-terminal/tests/terminal_live.rs`; Modify **both** `docs/specs/2026-07-16-...` and `docs/specs/2026-07-17-...`.

- [ ] **Step 1: real-window proof.** READ memory `terminal-realwindow-harness-pitfalls` FIRST. Mirror `presentation_realwindow.rs`. Phases: (P-report) `engine.feed(b"\x1b[?1000h\x1b[?1006h")` THROUGH the engine, poll `latest_frame_for_test()` until non-empty, `debug_mouse_down_for_test`+`debug_mouse_up_for_test` at a cell center, assert an SGR `EgressFrame` via `attach_test_egress` (mirror `await_single_egress`). (P-select) tracking OFF, drag across cells, assert `latest_frame_for_test()` shows `selected` cells + `debug_handle_copy_for_test` writes text (clipboard test hook). (P-localclick) tracking OFF, click (no drag) on a hyperlink cell → assert `OpenUrlRequest` emitted (poll across renders; HOLD the Subscription). (P-readonly) tracking ON, tab read-only → click → assert NO egress, selection still works. **User heads-up before running.**
- [ ] **Step 2: run** — `cargo test -p lens-terminal --features test-util --test mouse_realwindow`. Expected PASS. **The RUN is the only proof.**
- [ ] **Step 3: live P6** (gated `--features live-tests`, like P1–P5): against the omnigent 0.5.1 rider-shell (memory `omnigent-terminal-attach-live-run`), enable `?1000h?1006h` via a program, simulate a press, assert the SGR report round-trips (echo) or a mouse-mode TUI reacts. **Do not run automatically.** Record in the ledger whether run this session.
- [ ] **Step 4: docs — BOTH specs.** `2026-07-17`: §2c open question → **RESOLVED-DEFERRED** (XTSHIFTESCAPE); **DP3 amended** — arbitration + latching + coalescing engine-side at ordered-stream position; the `Frame` mode hint was considered and **rejected** (stale-authority); bracketed-paste-warn nuance **remains deferred**. `2026-07-16`: completion-matrix 2c rows done (reporting + coalescing + toggle; selection + copy + select-all); XTSHIFTESCAPE override **DEFERRED** + re-vendor trigger + memory ref.
- [ ] **Step 5: full gate + commit.** `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo clippy -p lens-terminal --all-targets --features test-util,live-tests -- -D warnings && cargo test -p lens-terminal --features test-util`
```bash
git add crates/lens-terminal/tests/{mouse_realwindow,terminal_live}.rs crates/lens-terminal/Cargo.toml docs/specs/2026-07-16-terminal-workstream-design.md docs/specs/2026-07-17-terminal-slice-2-interaction-design.md
git commit -m "test(terminal-2c): real-window mouse proof + live P6 + engine-side-arbitration/XTSHIFTESCAPE docs"
```

---

## Self-Review checklist (spec §2c rows + all review findings → tasks)

- [ ] Reporting + coalescing + toggle → T2/T3/T6/T7. Selection + copy + **select-all** → T2/T4/T5. XTSHIFTESCAPE config-fallback + defer → T3/T6/T8.
- [ ] Read-only: report suppressed, select/copy allowed → engine arbiter reads `write_allowed` (T3), copy ungated (T4/T5); tests T3c/T3e/T8 P-readonly.
- [ ] **Rev-1 C1 egress** — report/wheel arms mirror Key; full-egress parity test (T3i); no over-claim; shared-seam note.
- [ ] **Rev-1 C2 / Rev-2 accepted** — arbitration engine-side; **no** `Frame` mode hint (deleted).
- [ ] **Rev-2 C1** — hint deleted; hover motion always forwarded; hyperlink via engine `LocalClick` (T3/T5).
- [ ] **Rev-2 C2** — latch stores epoch+suppressed; every Report re-checks epoch+`write_allowed` (T3e read-only-bypass test).
- [ ] **Rev-2 C3** — Wheel atomic, never touches latch (T4).
- [ ] **Rev-2 I4** — matching-button release, all-button up-out, optional cell (T3f/T5).
- [ ] **Rev-2 I5** — Left-only selection; Right/Middle no-track = `Ignored` (T3h).
- [ ] **Rev-2 I6** — real `VtEngine` shapes: `cols`/`rows` fields, split-borrow, `build_frame()->Result`, 3-arg `new` (T2).
- [ ] **Rev-2 I7** — selection methods `Result<bool>`, propagate errors, dirty incl None-clear (T2).
- [ ] **Rev-2 I8** — `set_time` multi-click via `MouseGesture.time`; double-click-word golden (T2).
- [ ] **Rev-2 I9** — `MouseAck` on every branch (T3, all goldens `recv_timeout` a disposition).
- [ ] **Rev-2 I10** — `mouse_local`/`report_policy` fields defined in T5 (their first user).
- [ ] **Rev-2 I11/I15** — explicit `CoalesceKey` reset (owner/tracking/format/mods) + `reset()` (T7c).
- [ ] **Rev-2 I12** — immediate Down; local action deferred via `LocalClick`, not a deferred command (T3/T5).
- [ ] **Rev-2 I13** — one-filter test commands; `build_frame().expect` (T2).
- [ ] **Rev-1 C5/I6/I7 binding fidelity** — "Verified binding signatures" block + T2 code, vendor-sourced.
- [ ] **Rev-1 I11 copy fg/bg two-stage** (T5); **I17 bracketed-warn stays deferred** (Global + T8).
- [ ] **Type consistency** — all Lens DTO names identical across tasks.

## Reviewer + execution

- **Author:** Opus Rev 3 — folds Rev-1 (18) + Rev-2 (13) codex/gpt-5.6 findings; architecture validated twice; binding calls vendor-verified.
- **Execution (per user decision — no third plan review):** subagent-driven — composer-2.5 per task + **codex `gpt-5.6-sol` per-task cross-family review** + fix waves + gate after each (incl. `-p lens-terminal --features test-util,live-tests`) + **Opus whole-slice review** at the end (security callout: mouse-report-to-PTY + read-only latch-suppression + copy/clipboard). Real-window (Task 8) + live P6 with a user heads-up. The compiler + per-task reviews are the remaining feedback loop.

---

## Revision history (finding → fix)

- **Rev 1 → Rev 2 (18 findings):** foreground snapshot routing was the core error (C2) → arbitration moved engine-side; non-compiling binding calls (C5) → verified-signatures block; blocking/dropping local sends (C4) → forwarder-routed; copy fg/bg (I11) → two-stage; +docs/select-all/policy fixes.
- **Rev 2 → Rev 3 (3C+10I):** C1 stale hint (hover-drop + hyperlink) → **hint deleted**, hover always forwarded, hyperlink via engine `LocalClick`; C2 read-only egress bypass → **epoch+suppression latch**, per-event re-check; C3 wheel-in-latch → **atomic `Wheel`**; I4 release matching/out; I5 Left-only select; I6 real `VtEngine` shapes + split-borrow; I7 `Result` selection + dirty-on-None; I8 `set_time` multi-click; I9 `MouseAck` every branch; I10 fields in T5; I11/I15 explicit coalesce reset; I12 immediate Down; I13 one-filter tests.
