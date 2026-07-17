# Terminal Slice 2 — Interaction (design)

**Status:** ACTIVE — user-approved architecture (2026-07-17). Subordinate to the
workstream design `docs/specs/2026-07-16-terminal-workstream-design.md` (the parent),
which froze Slice 2's *requirements* and the public surface. This doc adds Slice 2's
**decomposition, input architecture, and per-phase design** — it does not re-litigate
the parent's requirements.

**Relationship to Slice 1:** Slice 2 builds **additively** on the shipped, reviewed,
gate-green engine (1b), render (1c), and convergence (1d) on `terminal-ws`. It does
**not** reopen the engine's threading contract.

Evidence base for the architecture decisions below:
- **Ghostty precedent** (the reference emulator we vendor), read at the pinned commit
  `a887df42` — verbatim citations inline. This is *precedent*, not reconstruction.
- **`libghostty-vt` capabilities** — the vendored binding already ships the hard
  encoders/selection/OSC machinery (citations inline).
- Parent design's **Threading & `Frame` seam** and **Render contract** sections.

---

## Scope

Slice 2 makes the terminal **interactive**. Delivered (across phases 2a–2d):

- Keyboard input (owner-write) with full VT key encoding; **IME** composition.
- Focus wiring; **read-only gating** across all input paths.
- Local text selection, copy, paste (bracketed + multiline warn), **OSC 52** write policy.
- Mouse-event reporting, shift/XTSHIFTESCAPE selection arbitration, mouse-local toggle.
- Output-side OSC presentation: titles, hyperlink gestures, progress, background
  notifications.

**In per the parent, confirmed this cycle:** IME lands in 2a (not deferred) — the
encoder handles it as a field on the key event, so it is not a separable beast.

**Out of scope (unchanged from parent):** production `lens-ui` hosting (Slice 2's
consumer is the standalone demo + a minimal in-demo host policy); full lifecycle/fleet
(Slice 3); the perf-acceptance harness (Slice 4); inline graphics.

---

## Architecture spine — Option A: single-owner engine thread + typed command channel

**Decision (user-approved 2026-07-17): all input encoding and selection run on the
engine thread that owns the non-`Send` `Terminal`; the foreground lowers raw GPUI
events into a typed, `Send`, Lens-owned command channel.** No Ghostty type crosses the
seam; the invariant "no Ghostty type escapes the engine boundary" holds for free.

### Why this is the faithful adaptation of Ghostty, not an invention

`libghostty-vt` ships the hard parts, and **every one is gated behind the non-`Send`
`&Terminal`** — so they can only run on the owning thread:

| Capability | Binding gives us | Needs `&Terminal` |
| --- | --- | --- |
| Key encoding (+ IME via `is_composing`/`utf8`) | `key::Encoder` + `set_options_from_terminal` (`key.rs:101,136`) | yes |
| Mouse encoding (X10/normal/SGR) | `mouse::Encoder` + `set_options_from_terminal` (`mouse.rs:99,128`) | yes |
| Bracketed-paste wrapping | `paste::encode(data, bracketed, buf)` (`paste.rs:69`) | mode from Terminal |
| Selection geometry (word/line/all/output) | `selection::select_*`, `set_selection` (`selection.rs`) | yes |
| Copy text extraction **incl. scrollback** | `format_selection_alloc` (`selection.rs:367`) | yes |
| OSC/title/clipboard/progress effects | `on_<effect>` callbacks during `vt_write` (`terminal.rs:67`) | yes |

**Ghostty itself shares its `Terminal` across threads under one mutex** and encodes on
the input thread (verified at `a887df42`):

- The `Terminal` is a pointer aliased into `renderer_state` (`Surface.zig:599`),
  guarded by `renderer_state.mutex` (`renderer/State.zig:13`); GUI, renderer, and
  IO-parse threads touch it only under that lock.
- Key/mouse encoding runs **on the GUI thread, reading live modes under the lock**:
  `encodeKeyOpts` locks and builds options from `t.modes.get(.cursor_keys)`…
  (`Surface.zig:3255` → `key_encode.zig:59`), then queues *pre-encoded bytes* to the
  IO mailbox (`Termio.zig:392`).

We **cannot** replicate that literally in Rust without an `unsafe impl Send/Sync` on
the vendored `Terminal` **and** reintroducing the shared mutex we deliberately removed
for the render path (see next section). That "Option B" was weighed and **rejected**:
it reopens the shipped, reviewed engine threading contract and adds owned `unsafe`, to
buy only a µs-scale input-latency edge that is imperceptible against ms-scale paint —
and Ghostty's own input thread blocks on its lock during parse anyway.

**Option A delivers Ghostty's guarantee by a different mechanism:** encoding reads
*live* modes because it *runs on the owner*, exactly as Ghostty reads live modes
*under the lock*. Same "live modes at encode" property; no `unsafe`; additive to Slice 1.

## The render-path constraint that forces single-owner (recap)

Slice 1 diverged from Ghostty on the **threading of the repaint** (not the repaint
*contract* — both full-repaint every frame; Spike A confirmed dirty-tracking
non-load-bearing). Ghostty locks the shared `Terminal` on a **dedicated per-surface
renderer thread**; Lens builds an immutable `Frame` off-thread and the foreground reads
it **lock-free** (`ArcSwap`). This is **forced by GPUI**: GPUI renders on the *single
shared foreground thread that paints the entire app*, so it must never block on a
per-terminal parse lock (that would jank the whole UI). Ghostty's dedicated renderer
thread can safely block briefly (and built `lockDemand` to avoid starvation); ours
cannot. The non-`Send` binding reinforces the same conclusion.

**Consequence — the Slice 2 render rule:**

- **Terminal-derived render state flows through the `Frame`** (engine-built): cells,
  styles, and the **selection highlight** (the `Frame` already carries per-row
  selection; 1c's painter draws it). So selection is engine-side: gesture → command →
  engine mutates the `Terminal`'s selection → `Frame` reflects it. This mirrors
  Ghostty, which stores selection **on the Screen inside the Terminal**
  (`Screen.zig:53`).
- **App-owned transient overlays are drawn foreground-side, on top of the `Frame`, with
  no engine round-trip.** This matches Ghostty exactly: preedit is **renderer-side, not
  in the terminal** (`renderer/State.zig:26`, drawn via `addPreeditCell`). So **IME
  preedit is a foreground overlay at the cursor cell** (composition state is
  foreground-owned anyway). A future URL-hover highlight is the same shape.

## Ghostty semantics adopted wholesale (cited)

Option A takes the *mechanism* (command channel to the owner); these five *semantics*
come straight from Ghostty and are first-class Slice 2 decisions:

1. **Live-mode encoding on the terminal-owning thread** (= Ghostty's live-under-lock,
   `Surface.zig:3255`).
2. **Selection state on the `Terminal`**; gesture → command → `Frame` carries the
   `RowSelection` (`Screen.zig:53`, drag under lock `Surface.zig:4667`).
3. **Mouse-motion cell-coalescing at the foreground** before the command — report only
   when the viewport cell changes (Ghostty `mouse_encode.zig:106`, keyed off
   `mouse.event_point` `Surface.zig:3673`).
4. **Block-don't-drop keystroke backpressure on a bounded queue** — Ghostty's mailbox
   is fixed-64 and **blocks the producer `.forever`** on full, dropping only if the
   wakeup notify fails (`mailbox.zig:78`, `blocking_queue.zig:106`). This **corrects
   1d's `try_send`-drop for input**, which is wrong by this precedent.
5. **IME commit unified through the key command** — Ghostty re-enters committed text via
   `keyCallback{ key: .unidentified, utf8: str }` (`apprt/gtk/.../surface.zig:3232`),
   the same encode path as any key. So `KeyInput` carries a `utf8` field; no separate
   IME path.

---

## The command seam (illustrative — representations stay evolvable per the parent)

Two channels cross the foreground↔engine boundary in addition to the existing inbound-VT
and `Frame`-publish paths from Slice 1:

**Foreground → engine (typed input commands):**

```rust
// Send + Lens-owned; the engine thread translates each into a ghostty
// Event / selection op, encodes with live modes, and emits bytes on the
// existing outbound reverse channel (the on_pty_write path → bridge → WS).
pub(crate) enum EngineInput {
    Key(KeyInput),                 // action, key, mods, utf8 (IME commit rides here)
    Mouse(MouseInput),             // button/motion/scroll, cell coords, mods
    Paste(PasteText),              // bracketed-wrapped on the engine thread by live mode
    Selection(SelectionGesture),   // begin/extend/word/line/all/clear at a cell
    Copy(Responder<String>),       // extract current selection; text returned to fg
    FocusChanged(bool),            // focus in/out reporting (mode-gated)
}
```

- **Backpressure = bounded + block-the-producer** (Ghostty precedent #4), *not*
  `try_send`-drop. Keystrokes are never silently dropped.
- **`Copy` carries a response channel**: `format_selection_alloc` runs on the engine
  thread; the extracted text returns to the foreground, which owns the system clipboard.

**Engine → foreground (typed presentation/host events, for 2d):**

```rust
// Emitted from on_<effect> callbacks during vt_write (engine thread, non-blocking:
// enqueue and return). Bridge/runtime forwards to TerminalTab, which surfaces them as
// the parent's frozen TerminalEvent / TerminalHostEvent.
pub(crate) enum EnginePresentationEvent {
    TitleChanged(String),                       // OSC 0/2 → reported_title
    ClipboardWrite { data: Vec<u8> },           // OSC 52 → host permission + cap
    DesktopNotification { title, body },        // → rate-limited host request
    ProgressReport(Progress),                   // OSC 9;4 → presentation
    // hyperlink is per-cell Frame state + a hover→gesture host request, not a stream event
}
```

**Read-only gating** threads through the foreground: in read-only, the foreground
**suppresses `Key`/`Mouse`/`Paste`** commands but still issues `Selection`/`Copy`
(local) and still sends resize; the engine is unchanged. (1a already drops binary input
transport-side; this is the UI-level policy layer the parent assigns to `lens-terminal`.)

---

## Per-phase design

Build order: **`2a ∥ 2d` (independent) → `2b` (needs 2a) → `2c` (needs 2b+2a)**.
`2a`/`2d` run as parallel worktrees, each cross-family reviewed by a *different* family
(the 1a∥1b pattern).

### 2a — Input path: keyboard + IME + focus + read-only gating

- **Ours:** the `EngineInput` channel + engine-side `key::Encoder` driver
  (`set_options_from_terminal` each encode); GPUI `InputHandler` capturing keys and IME
  marked/commit text; **IME preedit as a foreground overlay** over the `Frame`; focus
  wiring via the existing `focus_handle`; read-only input suppression; block-don't-drop
  backpressure.
- **Delegated:** all key/IME byte encoding (`key::Encoder`, `key::Event`).
- **Seam note:** the engine worker's select loop gains an `EngineInput` arm; it must
  **not starve input under output flood** (chunk `vt_write`, service input between
  chunks) — Design Point (2) below.
- **Testing:** hermetic golden-bytes (set a mode via `vt_write` DECSET → push
  `Key` → assert outbound bytes) is the bulk of correctness. IME preedit + `InputHandler`
  need the real-window harness; IME composition also gets a **manual/live acceptance
  leg** (Design Point (6)).

### 2b — Clipboard: selection, copy, paste, OSC 52

- **Ours:** foreground cell hit-testing (pixel→cell from `Frame` geometry) → `Selection`
  gestures; `Cmd+C` copy → `Copy` command → system clipboard; `Cmd+V` paste → `Paste`
  command (bracketed-wrapped engine-side) with **multiline warn** (global-disable /
  "don't warn again"); **OSC 52** write policy (decoded-payload cap + host permission
  allow-once/session + copy notice; reads denied). Read-only: select yes, paste no.
- **Delegated:** selection geometry + word/line/all (`selection::select_*`), scrollback
  text extraction (`format_selection_alloc`), bracketed-paste encoding (`paste::encode`).
- **Seam note:** OSC 52 rides here (shares the clipboard-permission machinery with copy),
  even though it arrives as an `EnginePresentationEvent::ClipboardWrite` from the engine.
- **Testing:** selection-op and copy-text golden tests are hermetic (engine-side);
  hit-testing (pixel→cell) needs real font metrics → real-window harness.

### 2c — Mouse

- **Ours:** GPUI mouse capture (button/motion/scroll) → **cell-coalesced**
  (Ghostty #3) `Mouse` commands; **shift ↔ XTSHIFTESCAPE arbitration** (shift enables
  local selection unless the app captures it — Ghostty `Surface.zig:4613,3902,3712`);
  runtime mouse-local toggle; read-only sends none.
- **Delegated:** mouse-protocol encoding (`mouse::Encoder`, tracking mode/format).
- **Testing:** encode golden tests hermetic; capture + coalescing + arbitration in the
  real-window harness; live mouse-reporting proof needs a real mouse-mode TUI in the
  rider shell (e.g. `vim`).

### 2d — Output-side OSC presentation: titles, hyperlinks, progress, notifications

- **Ours:** register `on_<effect>` callbacks on the engine thread → `EnginePresentationEvent`
  → forward to `TerminalTab` → parent's `TerminalEvent`/`TerminalHostEvent`; OSC 0/2
  `reported_title` (sanitized/bounded) + stable `identity_title`; OSC 8 + validated
  plain-URL → user-gesture → typed host-request open; OSC progress → presentation;
  notifications → rate-limited host requests while unfocused (read-only suppressed);
  a **minimal in-demo host policy** so all of this is verifiable without `lens-ui`.
- **Delegated:** OSC parsing + effect dispatch (`terminal.rs` `on_<effect>`, `osc::CommandType`),
  hyperlink cell state (`screen.rs:hyperlink_uri`).
- **Independent of the input path** → parallelizes with 2a.
- **Testing:** feed OSC byte sequences via `vt_write` → assert the emitted event; the
  demo host policy doubles as the request/response test harness.

---

## Cross-cutting design points (baked in, not afterthoughts)

1. **Synchronous encode seam for tests.** Provide a test-only "encode on the caller
   thread with explicit modes" entry so *encoding* assertions stay off the thread
   boundary; reserve the full fg→engine→outbound round-trip for a few integration tests,
   gated on a **bounded ack** (never sleep-based). This is deliberate defense against the
   known cross-thread-timing flake class (cf. the `hidden_tab_suppresses_publish` flake).
2. **Worker input non-starvation under flood.** The engine select loop must service
   `EngineInput` promptly even during a 64 KiB output burst — chunk `vt_write`, check
   input between chunks. Input latency (fg→engine→WS) is measured, not assumed.
3. **Foreground mouse-motion cell-coalescing** (Ghostty #3) — before flooding the
   command channel/WS.
4. **`select-all` + copy off the publish hot path.** `format_selection_alloc` over a
   10M-byte-equivalent scrollback can be multi-ms; it runs as a `Copy` command, may cost
   a one-frame hitch (user-triggered, rare) — acceptable, but must not sit in the frame
   build/publish path.
5. **Block-don't-drop keystroke backpressure** (Ghostty #4) — bounded queue, block the
   producer; corrects 1d's input `try_send`.
6. **IME manual acceptance leg** — the compose→commit→cancel state machine and the
   OS-owned candidate window are not hermetically testable; add a manual/live leg like
   the 1d rider.

---

## Testing strategy (summary)

- **Hermetic golden-bytes on the engine thread** is the majority of 2a/2b/2c correctness:
  set modes via `vt_write`, push a command, assert outbound bytes / emitted event. Cheap,
  deterministic, no GPUI, no network — the pattern from Spike B and state-model
  golden-replay. We test *our mapping*, not xterm's spec (the encoders are Ghostty's).
- **Real-window GPUI harness** (`harness=false`, `test-util`-gated, xtask-executed on
  macOS) for everything that touches real font metrics or the GPUI event loop:
  `InputHandler` (keys/IME preedit), mouse capture/hit-testing/coalescing/arbitration.
  `#[gpui::test]`/`NoopTextSystem` **false-greens** these (memory
  `gpui-test-noop-text-system`) — do not use it for capture/hit-testing.
- **Live rider extension** (`tests/terminal_live.rs`) vs real omnigent 0.5.1, per 1d:
  type→echo/edit, paste, selection→copy round-trip, and mouse-reporting against a
  mouse-mode TUI in the rider shell.
- **Inspect + benches per phase** (parent discipline): input-command codec/throughput;
  encode micro-benches; the diagnostic rings extended for input/selection/OSC.

## Performance implications

- **Input latency gains one cross-thread hop + wakeup** (was fg→WS direct in 1d).
  Sub-µs encode + µs-scale hop, imperceptible vs ms-scale paint; the parent's
  input-event-to-first-paint metric becomes load-bearing and is measured, not assumed.
- **Render hot path is unchanged.** Encoding output goes to the outbound channel;
  selection is already in the `Frame`. The 1c perf verdict holds — new cost is all on the
  input (cold-relative-to-output) side.
- **Mouse-motion flooding** is the real risk, mitigated by cell-coalescing (Design
  Point 3). **`select-all`-copy** may cost a one-frame hitch (Design Point 4).

---

## Completion matrix (Slice 2 slice of the parent's anti-drop matrix)

| Requirement | Phase |
| --- | --- |
| Keyboard VT encoding (engine-side, live modes) | 2a |
| IME composition (commit via key path; preedit foreground overlay) | 2a |
| Focus wiring | 2a |
| Read-only input gating | 2a (policy) · threads 2b/2c |
| Block-don't-drop keystroke backpressure | 2a (corrects 1d) |
| Selection + copy (engine-side geometry/extraction; fg hit-testing) | 2b |
| Paste (bracketed + multiline warn) | 2b |
| OSC 52 write-cap + read-denial + permission | 2b |
| Mouse reporting + shift/XTSHIFTESCAPE + mouse-local toggle + coalescing | 2c |
| Titles (`reported_title` + stable `identity_title`) | 2d |
| Hyperlink gestures (OSC 8 + plain URL → host request) | 2d |
| OSC progress → presentation | 2d |
| Background notifications (rate-limited host request) | 2d |
| Inspect + benches (input/selection/OSC) | per-phase |
| Live proof vs real omnigent | 2a/2b/2c rider extension |

## Open questions / deferred

- **Title string capture:** `on_title_changed` fires during `vt_write`, but the binding
  notes the title itself is "tracked by the embedder via the OSC parser or its own state"
  — 2d confirms whether an accessor exists or we run `osc::Parser` on the title bytes.
  Implementation detail, not an architecture risk.
- **URL-hover highlight** (foreground overlay, like preedit) — only if 2d's hyperlink
  gesture wants hover affordance; otherwise click-through suffices.
- **`Frame`-pool / reuse** to erase the per-display-sample `Frame` allocation (the one
  place we do more work than Ghostty) — deferred optimization; measured-acceptable today,
  contract-preserving when needed.
- **Option B (shared-mutex, literal Ghostty)** stays documented as the considered
  alternative; revisit only if input-under-flood latency is ever measured to matter.
