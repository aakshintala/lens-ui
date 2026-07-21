# Terminal Slice 2 — Interaction (design)

**Status:** ACTIVE — user-approved architecture (2026-07-17); **revised 2026-07-17 after
cross-family review** (grok-4.5 + gpt-5.6-sol, both source-verified against the pinned
Ghostty clone and the vendored binding). Subordinate to the workstream design
`docs/specs/2026-07-16-terminal-workstream-design.md` (the parent), which froze Slice 2's
*requirements* and the public surface. This doc adds Slice 2's **decomposition, input
architecture, and per-phase design** — it does not re-litigate the parent's requirements.

**Relationship to Slice 1:** Slice 2 builds **additively** on the shipped, reviewed,
gate-green engine (1b), render (1c), and convergence (1d) on `terminal-ws`. It does
**not** reopen the engine's threading contract.

Evidence base:
- **Ghostty precedent** (the reference emulator we vendor), read at the pinned commit
  `a887df42` — verbatim citations inline. Precedent, not reconstruction.
- **`libghostty-vt` capabilities** — the vendored binding, `on_*` effect surface and
  encoders audited (citations inline). **The binding does *not* expose everything the
  parent requires; the gaps are called out explicitly below, not papered over.**
- Parent design's **Threading & `Frame` seam** and **Render contract** sections.

---

## Scope

Slice 2 makes the terminal **interactive**. Delivered (across phases 2a–2d):

- Keyboard input (owner-write) with full VT key encoding; **IME** composition.
- Focus wiring; **read-only gating** (local scroll/select allowed; all encoded input —
  keys, paste, mouse, **and focus reports** — suppressed).
- Local text selection, copy, paste (bracketed + multiline warn), **OSC 52** write policy.
- Mouse-event reporting, shift/XTSHIFTESCAPE selection arbitration, mouse-local toggle.
- Output-side OSC presentation: titles, hyperlink gestures, and — **contingent on a
  binding extension (see 2d)** — progress and background notifications.

**In per the parent:** IME lands in 2a (the encoder handles it as a key field).

**Out of scope (unchanged):** production `lens-ui` hosting (Slice 2's consumer is the
standalone demo + a minimal in-demo host policy); full lifecycle/fleet (Slice 3); the
perf-acceptance harness (Slice 4); inline graphics.

---

## Architecture spine — Option A: single-owner engine thread + one ordered command stream

**Decision (user-approved): all input encoding and selection run on the engine thread
that owns the non-`Send` `Terminal`; the foreground lowers raw GPUI events into typed,
`Send`, Lens-owned commands.** No Ghostty type crosses the seam.

Option A was chosen over **Option B** (share the `Terminal` under a mutex + `unsafe impl
Send/Sync`, encode on the foreground under lock, literal Ghostty). B is rejected because
it needs owned `unsafe` on a non-`Send` FFI type **and** reintroduces the shared mutex
Slice 1 deliberately removed for the render path (GPUI's *single shared foreground render
thread* must never block on a per-terminal parse lock — see "Render-path constraint").
That deep reason stands; the latency argument is secondary.

### Ordering & the live-mode property (corrected after review)

⚠ **The earlier claim "encoding on the owner = the *same* live-mode guarantee as
Ghostty's live-under-lock" was wrong and is retracted.** Ghostty encodes a key *inside*
the mutex critical section it takes for that key (`Surface.zig:3255` `encodeKeyOpts` →
`key_encode.zig:136` `fromTerminal`), so the modes it reads are the terminal's state at
that serialization point. A naive Option A — foreground queues an *unencoded* key on a
**separate** channel from inbound VT — lets a large `Feed` batch (or a mode-changing VT
sequence) be processed **ahead of** an already-queued key, so the key encodes against
*later* modes than its stream position warrants (application-cursor/keypad, Kitty flags,
bracketed-paste, mouse protocol, focus mode can all differ). That is a real
**mode-timing/reordering bug**, not just latency.

**Fix — one ordered ingress+input command stream.** VT ingress (`Feed`) and user input
(`Key`/`Mouse`/`Paste`/`Selection`/`Copy`/`Focus`/`LocalScroll`) travel a **single
ordered channel** to the engine, processed **strictly in arrival order** with **`Feed`
chunked** (bounded max chunk) so the loop never drains a VT batch *ahead of* a queued
input command. A key then encodes against exactly the modes produced by the VT that
preceded it *in that stream* — which is Ghostty-equivalent up to the **inherent
concurrency of two event sources** (a server mode-change and a keypress that are
genuinely simultaneous have no total order in Ghostty either; whichever takes the mutex
first wins — here, whichever reaches the channel first wins). We restore Ghostty's
guarantee; we do not claim to beat physics. This is a **hard contract**, not a hint
(see Design Point 2). *(Alternative considered: a Lens-owned mode-epoch snapshot
published after each VT process and captured with each foreground event — more moving
parts, same result; the single ordered stream is simpler and chosen.)*

## Render-path constraint (recap — why single-owner)

Slice 1 diverged from Ghostty on the *threading of the repaint* (not the repaint
*contract*; both full-repaint every frame — Spike A confirmed dirty-tracking
non-load-bearing). Ghostty locks the shared `Terminal` on a **dedicated per-surface
renderer thread**; Lens builds an immutable `Frame` off-thread, foreground reads it
**lock-free** (`ArcSwap`). This is **forced by GPUI**: it renders on the *single shared
foreground thread that paints the whole app*, which must never block on a per-terminal
parse lock. The non-`Send` binding reinforces the same conclusion.

**Slice 2 render rule:**
- **Terminal-derived render state flows through the `Frame`** (engine-built): cells,
  styles, per-cell `selected` (the `FrameCell.selected: bool` 1c already paints), and —
  new in 2d — a per-cell **hyperlink URI** field. Selection is engine-side: gesture →
  command → engine mutates the `Terminal`'s selection (its **own gesture state machine**;
  Lens `SelectionGesture` values are transport DTOs onto it, not a second implementation)
  → `Frame` reflects it. Mirrors Ghostty (selection on the Screen, `Screen.zig:53`).
- **App-owned transient overlays draw foreground-side, over the `Frame`, no engine
  round-trip.** Ghostty stores preedit **renderer-side, not in the terminal**
  (`renderer/State.zig:26`, drawn via `addPreeditCell`). So **IME preedit is a foreground
  overlay at the cursor cell**; a future URL-hover highlight is the same shape.

## Ghostty semantics adopted (cited; scope-corrected)

1. **Live-mode encoding on the terminal-owning thread**, via the **single ordered stream**
   above so modes are read as-of the key's stream position (Ghostty `Surface.zig:3255`).
2. **Selection state on the `Terminal`**; gesture → command → `Frame` carries per-cell
   `selected` (Ghostty `Screen.zig:53`; drag under lock `Surface.zig:4667`).
3. **Mouse-motion cell-coalescing at the foreground** — **but only when the report format
   is *not* SGR-pixels** (Ghostty `mouse_encode.zig:104-117` skips coalescing for
   `sgr_pixels`). Coalescing needs a foreground-visible **mouse-mode/format snapshot**,
   and dedup **resets** when format, tracking mode, button state, or mouse-local policy
   changes (Ghostty keys dedup off `mouse.event_point`, `Surface.zig:3675`).
4. **Never silently drop keystrokes — but do *not* block the foreground.** Ghostty blocks
   its *surface-local* GUI producer on a full 64-slot mailbox (`mailbox.zig:70` instant,
   `:76-78` drop only if the wakeup notify fails, `:92` `.forever` block;
   `blocking_queue.zig:106`). **Lens's producer is the *shared* GPUI foreground**, which
   must never block (app-wide freeze) and must stay free to run the C3 teardown `take()`.
   So we adapt, not copy — see the command seam.
5. **IME commit unified through the key command** — Ghostty re-enters committed text via
   `keyCallback{ key: .unidentified, utf8: str }` (`apprt/gtk/class/surface.zig:3234-3240`).
   `KeyInput` carries a `utf8` field; no separate IME path.

---

## The command seam (illustrative — representations stay evolvable per the parent)

**Foreground → engine: one ordered command stream** (unifies VT ingress + input so
ordering is preserved; §"Ordering"):

```rust
pub(crate) enum EngineCommand {
    Feed(Bytes),                   // inbound VT (chunked; bounded max chunk)
    Key(KeyInput),                 // action, key, mods, utf8 (IME commit rides here)
    Mouse(MouseInput),             // button/motion/scroll, cell, mods, format snapshot
    Paste(PasteText),              // capped payload; bracketed-wrapped engine-side by live mode
    Selection(SelectionGesture),   // DTO onto the engine's gesture state machine
    Copy(CopyResponder),           // async, capacity-1, cancellation-safe (never fg-recv)
    Focus(bool),                   // focus in/out; encoded report gated by mode-1004 + access
    LocalScroll(ScrollDelta),      // viewport-only; ALLOWED in read-only (no PTY bytes)
    Resize { cols, rows },         // existing 1d path, folded into the ordered stream
    // control (Stop/SetVisible) stay on their existing priority path — see teardown
}
```

**Backpressure (adapted from Ghostty #4 — the highest-stakes correction):**
- The **foreground enqueues non-blockingly** (never blocks GPUI). It pushes into a
  foreground-local queue; an **off-foreground forwarder** performs any bounded blocking on
  the engine leg. Teardown severs the forwarder **without** requiring the foreground to
  exit a blocking send (preserves C3: `runtime.take()` on fg, joins off-fg; the 1d ledger
  explicitly rejected a foreground `send_blocking` for this reason).
- **The three guarantees — bounded memory, never-drop, never-block-fg — cannot all hold
  at once.** We choose: **never-block-fg + never-drop**, and bound memory *in practice*
  by coalescing mouse motion (#3) and capping paste payloads; keystrokes are low-rate so
  the foreground-local queue cannot realistically grow unbounded. If a hard memory ceiling
  is later required, the explicit fallback is **input rejection with a visible marker**,
  never a silent drop.
- **`Stop` must preempt `Feed` draining** so teardown never waits behind a flood or a
  multi-ms `Copy` extraction.

**Engine → foreground: two paths.**
- **`EngineEgress`** (bytes to WS): DA/DSR replies **and** encoded key/mouse/paste bytes
  ride one deliberate egress to `WsOutbound::Input`. *(Correction: encoded user input is
  **not** "the `on_pty_write` path" — that callback is for terminal-*initiated* replies.
  Both feed the same egress, but they are distinct producers.)*
- **`EnginePresentationEvent`** (typed presentation/host events for 2d) — emitted from
  `on_<effect>` callbacks (title, clipboard) **plus a side-channel OSC tap** for effects
  the binding has no callback for (progress/notifications — see 2d gap). Callbacks are
  non-blocking: **copy the payload into owned data inside the callback, enqueue, return**;
  never await the host from an effect (would stall `vt_write` / the engine thread).

**Read-only gate:** one gate keyed on **effective access ∧ `input_enabled`**. Read-only
**suppresses** `Key`/`Mouse`/`Paste`/`Focus`-report; **allows** `Selection`/`Copy`/
`LocalScroll` (local, no PTY bytes) and `Resize`. (1a already drops binary input
transport-side; this is the UI-level policy the parent assigns to `lens-terminal`.)

**Capability split (corrected):** the encoders are **pure** — `key::Encoder`/
`mouse::Encoder` take explicit `set_*` options and `paste::encode` takes a `bracketed:
bool`; **only the *authoritative mode source* (`set_options_from_terminal`) needs
`&Terminal`.** This is why the synchronous test seam (below) is real.

---

## Amendment 2026-07-20 — 2b/2c re-cut + `ClipboardPolicy` seam

Execution went **serial** (2a→2d→2b→2c on `terminal-ws`, no worktrees — see the execution
handoff). Two decisions taken when planning **2b** (user-approved 2026-07-20) supersede the
original 2b/2c phase split below:

1. **Selection + copy move from 2b into 2c.** Local selection and `Cmd+C` copy need GPUI
   mouse-drag capture + pixel→cell + motion-coalescing — the *exact* infrastructure 2c builds
   for mouse **reporting** — and 2c's headline **shift/XTSHIFTESCAPE arbitration** literally
   arbitrates selection-vs-report. Keeping them in 2b would build mouse capture twice (or make
   2c reach back into 2b). So the re-cut is:
   - **2b = OSC-52 output-clipboard write policy + `Cmd+V` paste** — both keyboard/output-only,
     **zero mouse dependency**. This is what 2d deferred, plus paste.
   - **2c = mouse capture → { local selection + `Cmd+C` copy, mouse reporting } + XTSHIFTESCAPE
     arbitration + pixel→cell/coalescing** — the capture stack is built **once**.
   The completion-matrix rows move accordingly (selection/copy → 2c; OSC-52 + paste stay 2b).
2. **Policy state is session-scoped behind an injectable `ClipboardPolicy` seam** — the paste
   multiline-warn "don't warn again" flag and the OSC-52 "allow for session" decision live in
   foreground per-tab state (reset per process), behind a small trait so `lens-ui` can inject a
   **persisted** impl later without touching 2b. The spec's earlier "**persisted**" wording for
   these two becomes a **documented deferral** — 2b ships the in-memory demo impl + the seam.

Binding facts pinned for 2b execution: `on_clipboard_write`'s result is **ignored by OSC-52**
(callback never backpressures; policy is applied on the **foreground**, async, never in the
callback); `ClipboardContent{ mime, data }` is already **base64-decoded** (the named byte cap is
on summed `data.len()`, applied **before** cloning owned copies); OSC-52 **reads** never reach the
callback (binding-level); `paste::encode(&mut, bracketed, &mut)` reads live mode 2004 engine-side.

## Per-phase design

Build order: **`2a ∥ 2d` (independent) → `2b` (needs 2a *and* 2d's presentation egress) →
`2c` (needs 2b + 2a)**. `2a`/`2d` run as parallel worktrees, each cross-family reviewed
by a *different* family (the 1a∥1b pattern). **2b must not begin against a 2d stub that
lacks the `EnginePresentationEvent` path** (OSC 52 arrives on it).
*(Superseded by the 2026-07-20 amendment above: execution was serial, not worktrees; and
selection/copy moved 2b→2c.)*

### 2a — Input path: keyboard + IME + focus + read-only gating

- **Ours:** the single ordered `EngineCommand` stream + engine-side `key::Encoder` driver
  (`set_options_from_terminal` per encode); GPUI `InputHandler` for keys + IME
  marked/commit; **IME preedit as a foreground overlay**; focus wiring; **focus-report
  suppression** (mode-1004 reports are PTY input) and the full read-only gate;
  **off-foreground, never-drop, never-block-fg** backpressure.
- **Delegated:** all key/IME byte encoding (`key::Encoder`, `key::Event`;
  `set_composing`/`set_utf8`, `key.rs:312+`).
- **Contract:** the ordered-stream + `Feed`-chunk fairness (Design Point 2).
- **Testing:** hermetic golden-bytes via the **pure encoder** (explicit `set_*` on the
  caller thread, no `Terminal`) for Lens→Ghostty event mapping; **engine-thread
  integration** (mode setup via `vt_write` DECSET → `Key` → bounded-ack outbound) for
  `set_options_from_terminal` + ordering; IME preedit/`InputHandler` in the real-window
  harness; IME composition also gets a **manual/live leg**.

### 2b — Clipboard: selection, copy, paste, OSC 52

- **Ours:** foreground cell hit-testing (pixel→cell from `Frame` geometry) →
  `Selection` gestures; `Cmd+C` → `Copy` (**async, capacity-1, cancellation-safe; the
  foreground never `recv`s**) → system clipboard; `Cmd+V` → `Paste` (capped, bracketed
  engine-side) with **multiline warn** (global-disable / **persisted** "don't warn
  again"); **OSC 52** write policy — **a named decoded-byte cap applied *before* cloning
  the callback payload**, defined aggregate/deny-vs-truncate behavior, and it must
  **preserve or explicitly reject `ClipboardWrite::location()` + MIME `contents()`**
  (`terminal.rs:1257-1267`), not flatten to one `Vec<u8>`. Reads are denied — the binding
  already never delivers read requests to the callback (`terminal.rs:1676-1677`); keep the
  UI policy as defense-in-depth. Read-only: select yes, paste no.
- **Delegated:** selection geometry/word/line/all (`selection::select_*`), scrollback text
  extraction (`format_selection_alloc`, `selection.rs:367`), bracketed-paste encoding
  (`paste::encode(&mut [u8], bracketed, &mut [u8])`, `paste.rs:69` — **in-place**).
- **Testing:** selection-op + copy-text golden (engine-thread); OSC 52 cap **cap−1/cap/
  cap+1** + `?`-read produces **no** host request; hit-testing in the real-window harness.

### 2c — Mouse

- **Ours:** GPUI mouse capture → **format-aware cell-coalescing** (skip for SGR-pixels;
  reset on format/mode/button/policy change) → `Mouse` commands; **shift ↔ XTSHIFTESCAPE
  arbitration** and runtime mouse-local toggle; read-only sends none.
- **⚠ Binding gap (gpt-C4):** the safe binding exposes mouse *tracking* mode but **no
  `mouse_shift_capture`/XTSHIFTESCAPE getter** (Ghostty reads a ternary
  `terminal.flags.mouse_shift_capture`, `Surface.zig:3712`). **2c opens with a small
  safe-FFI accessor** for that state (published in the foreground mouse-mode snapshot).
  *Fallback if the underlying C API doesn't expose it:* arbitrate on config
  (never/always) + readable DEC mouse modes only, and **defer the terminal-requested
  override** with a parent-matrix note — never infer it from ordinary DEC modes.
- **Delegated:** mouse-protocol encoding (`mouse::Encoder`, tracking/format/size).
- **Testing:** encode golden (engine-thread); capture/coalescing/arbitration in the
  real-window harness; live mouse-reporting vs a real mouse-mode TUI (`vim`) in the rider.

### 2d — Output-side OSC presentation: titles, hyperlinks, progress, notifications

- **Ours:** register effect callbacks on the engine thread → `EnginePresentationEvent` →
  `TerminalTab` → parent's `TerminalEvent`/`TerminalHostEvent`; a **minimal in-demo host
  policy** so all of it is verifiable without `lens-ui`.
  - **Titles — real effect.** `on_title_changed` + read `Terminal::title()`
    (`terminal.rs:612-623`, tested) inside the callback; sanitize + bound → `reported_title`
    (+ stable `identity_title`). *(The prior "title capture" open question is closed.)*
  - **OSC 52 — real effect.** `on_clipboard_write` (→ 2b's policy).
  - **Hyperlinks.** OSC 8 + validated **plain-URL** detection; per-cell URI added to
    `FrameCell` (`screen.rs:116` `hyperlink_uri`); hover/click gesture → typed host request.
  - **⚠ Progress + notifications — NO callback exists (C2 / gpt-C3).** The binding's `on_*`
    surface is `on_pty_write/bell/enquiry/xtversion/title_changed/pwd_changed/size/
    color_scheme/device_attributes/clipboard_write` — **no progress, no desktop-
    notification effect**; they exist only as parse-side `osc::CommandType::{ConemuProgressReport,
    ShowDesktopNotification}` (`osc.rs:130-136`) with **no payload accessor**. **2d opens
    with a spike + a small safe-FFI/binding extension** (a payload-bearing effect or
    getter) for these two. *Fallback if the C API doesn't expose the payloads:* **defer
    progress + notifications to a Slice-2 follow-up and amend the parent completion
    matrix** — do not claim a "parallel `osc::Parser` tap" covers them (it exposes payload
    only for titles).
- **Independent of the input path** → parallelizes with 2a; but **it owns the
  `EnginePresentationEvent` egress 2b depends on**, so its channel lands early.
- **Testing:** feed OSC bytes via `vt_write` → assert the emitted event; the demo host
  policy doubles as the request/response harness.

---

## Cross-cutting design points (baked in)

1. **Two-suite test seam.** (a) **Pure-encoder mapping** tests — construct
   `key::Encoder`/`mouse::Encoder` locally, set explicit modes via `set_*` (no `Terminal`,
   no thread hop), assert Lens-event→bytes. (b) **Engine-thread integration** — modes via
   `vt_write`, push a command with a **command-id/oneshot ack** defined as *"encoded bytes
   accepted by the test outbound receiver,"* synchronized with `recv_timeout` (deterministic).
   **No sleeps, no frame-polling** for synchronization (that reintroduces the flake class).
2. **Ordered-stream fairness (hard contract, not a hint).** Single ordered channel;
   pinned **max `Feed` chunk size**; defined ordering among `Feed`/input/`Stop`/publish;
   **`Stop` preempts continued `Feed` draining.** Test: 64 KiB `Feed` with an interleaved
   `Key`, asserting the key encodes against the pre-`Feed` modes if it arrived first.
3. **Format-aware foreground mouse-motion coalescing** (Ghostty #3): cell-coalesce except
   SGR-pixels; reset dedup on format/mode/button/policy change.
4. **`select-all` + copy off the publish hot path.** `format_selection_alloc` over a huge
   scrollback can be multi-ms; runs as an async `Copy`, may cost a one-frame hitch (rare,
   user-triggered) — never in the frame build/publish path; `Stop` preempts it.
5. **Off-foreground, never-drop, never-block-fg backpressure** — foreground enqueues
   non-blockingly; off-fg forwarder does bounded blocking; `Stop`-severable; explicit
   *reject-with-marker* fallback if a hard memory ceiling is ever imposed. **Corrects the
   record:** Slice 1 has **no** production keystroke path — `bridge.rs`'s `try_send` is
   DA/DSR outbound + visible-reconnect saturation, and `lib.rs:415-425` is a live-test
   hook; 2a introduces the *first* production input path and must adopt this policy from
   the start (it is not "fixing a 1d bug").
6. **IME manual acceptance leg** — compose→commit→cancel + OS candidate window aren't
   hermetically testable; add a manual/live leg like the 1d rider.

---

## Testing strategy (summary)

- **Hermetic golden** dominates 2a/2b/2c correctness: pure-encoder mapping (no thread) +
  engine-thread integration (bounded ack). We test *our mapping*, not xterm's spec.
- **Real-window GPUI harness** (`harness=false`, `test-util`-gated, xtask-executed on
  macOS) for real-font/event-loop paths: `InputHandler` (keys/IME preedit), mouse
  capture/hit-testing/coalescing/arbitration. `#[gpui::test]`/`NoopTextSystem`
  **false-greens** these (memory `gpui-test-noop-text-system`).
- **Live rider extension** (`tests/terminal_live.rs`) vs real omnigent 0.5.1: type→echo/
  edit, paste, selection→copy round-trip, mouse-reporting vs a mouse-mode TUI.
- **Inspect + benches per phase**: ordered-stream throughput/fairness; encode
  micro-benches; rings extended for input/selection/OSC.

## Performance implications

- **Input latency gains one hop + wakeup**; sub-µs encode, µs-scale hop, imperceptible vs
  ms paint. **But under output flood, latency is bounded by the ordered-stream fairness
  contract (Design Point 2), not left to chance** — measured against the parent's
  input-to-first-paint metric.
- **Render hot path unchanged** — encoded bytes go to `EngineEgress`; selection is already
  in the `Frame`. The 1c perf verdict holds.
- **Mouse-motion flooding** mitigated by format-aware coalescing (DP 3); **`select-all`
  copy** may cost a one-frame hitch (DP 4).

---

## Completion matrix (true anti-drop — each row separately testable, phase-owned)

| Requirement | Phase |
| --- | --- |
| Keyboard VT encoding (engine-side, live modes via ordered stream) | 2a |
| Ordered ingress+input stream + `Feed`-chunk fairness + `Stop` preempt | 2a |
| IME composition (commit via key path; preedit foreground overlay) | 2a |
| Focus wiring **+ focus-report (mode-1004) suppression in read-only** | 2a |
| Read-only gate (suppress key/paste/mouse/focus; allow scroll/select/copy) | 2a |
| **Local scroll in read-only** (`LocalScroll`, no PTY bytes) | 2a |
| Off-fg, never-drop, never-block-fg backpressure | 2a |
| Selection + copy (engine geometry/extract; fg hit-test; **async Copy**) | 2b |
| Paste (bracketed + multiline warn + **persisted "don't warn again"**) | 2b |
| OSC 52 **named byte cap** (cap−1/cap/cap+1) + MIME/location handling + **copy notice** | 2b |
| OSC 52 read-denial | 2b (binding + UI defense-in-depth) |
| Mouse reporting + **format-aware coalescing** + mouse-local toggle | 2c |
| **XTSHIFTESCAPE arbitration** (safe-FFI accessor **or** deferred override) | 2c |
| Titles: `on_title_changed`+`title()`, **sanitize/bound**, stable `identity_title` | 2d |
| Hyperlink: **`FrameCell` URI** + OSC 8 + **plain-URL validation** → host request | 2d |
| **Progress + notifications** (binding extension **or** deferred + matrix amendment) | 2d |
| Inspect + benches (ordered-stream/selection/OSC) | per-phase |
| Live proof vs real omnigent | 2a/2b/2c rider extension |

## Open questions / deferred

- **Progress + desktop-notification payload access** — no binding callback/getter; 2d
  opens with a spike + safe-FFI extension, else defer + amend the parent matrix. *(Blocks
  only those two 2d features, not titles/hyperlinks/OSC 52.)*
- **XTSHIFTESCAPE (`mouse_shift_capture`) accessor** — no safe getter; 2c opens with a
  safe-FFI accessor, else config-only arbitration + deferred override.
- **`Frame`-pool / reuse** to erase the per-display-sample `Frame` allocation — deferred
  optimization; measured-acceptable today, contract-preserving.
- **Option B (shared-mutex, literal Ghostty)** stays documented as the rejected
  alternative (owned `unsafe` + reopens the render threading contract).

---

## Review disposition (2026-07-17 cross-family pass)

grok-4.5 + gpt-5.6-sol, both source-verified; **verdict: revise before planning** — done
here. Folded: the live-mode-equivalence retraction + ordered-stream fix (both, Critical);
off-fg never-block-fg backpressure + the "1d correction" reframe (both, Critical); the 2d
progress/notification binding gap (both, Critical) + XTSHIFTESCAPE accessor gap (gpt,
Critical); read-only local-scroll + focus-report suppression, format-aware mouse
coalescing, OSC 52 full spec, capability-table pure-vs-mode split, two-suite test seam,
worker fairness, async `Copy`, `EngineEgress` naming, hyperlink `Frame` field, matrix
anti-drop rows (Important); and citation-drift + title-open-question-close + selection-
DTO clarifications (Minor). No finding was rejected; the two binding gaps are scoped as
FFI-or-defer dependencies rather than solved in-doc.
