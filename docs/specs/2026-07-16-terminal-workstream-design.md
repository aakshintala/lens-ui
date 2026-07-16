# Shared terminal infrastructure & E2E GPUI terminal tab (refreshed)

**Status:** ACTIVE — user-approved design (2026-07-16), refreshed after the
design-pass spikes.

**Supersedes in full:**
- `docs/specs/2026-07-14-terminal-workstream-design.md` — the "narrow attributed
  **port** of gpui-ghostty" + provenance-manifest model is dead.
- `docs/plans/2026-07-15-terminal-workstream-roadmap.md` — WP0–WP7 sequencing was
  authored against that dead port model. Its *method* (full-scope spec → sliced
  plans against landed APIs → cross-family review at seams) is kept; its *content*
  is re-expressed here as Slices 0–4.

This doc is self-contained; read it, not the superseded pair. The
model-independent requirements from the 2026-07-14 spec (public API, lifecycle,
identity/replacement, OSC/paste/mouse policy, memory/resize, perf acceptance)
are carried forward verbatim in intent; the VT-adoption, transport-selection, and
render sections are corrected by the two spikes.

Evidence base:
- **Spike A** (render viability): `docs/spikes/2026-07-15-terminal-render-viability.md`.
- **Spike B** (PTY-attach contract): `docs/spikes/2026-07-15-pty-attach-contract.md`.
- VT foundation: memories `terminal-vt-adoption-model`, `terminal-vt-vendored-executed`,
  `zig-ghostty-macos26-scissor`; `vendor/libghostty-rs/README.md`.
- Contract ground truth: `vendor/omnigent-0.5.1/openapi.json` + omnigent source
  revision `08285468` (`server/routes/terminal_attach.py`, `server/app.py`,
  `terminals/ws_bridge.py`, `terminals/control_bridge.py`, `server/routes/sessions.py`).

---

## Goal

Build the shared terminal foundation as a standalone, renderable GPUI terminal
tab. It attaches to a real omnigent terminal, keeps terminal-protocol knowledge
typed from transport through presentation, and is ready for `lens-ui` to host
without making `lens-ui` learn terminal REST, WebSocket, or Ghostty details.

Ghostty VT is the emulator, consumed as a **vendored `libghostty-rs` binding
built from source** (see "VT foundation"). The client owns a GPUI render layer
and replaces gpui-ghostty's local-PTY transport with omnigent's terminal
WebSocket.

## Scope

Delivered by this workstream (across the slices in "Build sequence"):

- Typed terminal list/get/create/delete and **auth/access-modeled** WS attach in
  `lens-client`; no generic WebSocket or `serde_json::Value` leaks to callers.
- A deep `lens-terminal` module wrapping `libghostty-vt`, with a small host
  interface and a standalone GPUI demo as its first consumer.
- Owner-write and viewer-read-only behavior; keyboard, IME, paste, resize,
  selection/copy, scrollback, mouse modes, titles, hyperlinks, and safe OSC
  handling.
- Brief reconnect with a retained emulator and an explicit marker that output
  during the interruption may be missing.
- Reproducible, pinned builds — **offline *after* the first cached build**, not on
  a clean build: `build.rs` fetches the pinned Ghostty source and Zig fetches its
  system deps over the network (both pinned/reproducible), then cache in `OUT_DIR`.
  Full clean-build offline (a vendored Ghostty tree) is deferred to the CI trigger.
  Deterministic tests, pinned-omnigent executable verification, and release-mode
  terminal benchmarks.

Out of scope:

- The native-harness rendered-stream/raw-TUI toggle; separate spec cycle.
- A generic incremental Bash-tool output surface. Omnigent 0.5.1 returns one-shot
  shell output with no `call_id`-correlated stdout/stderr stream.
- Integrating the tab into the production `lens-ui` working area.
- A local PTY / `portable-pty`; omnigent owns the PTY.
- Inline graphics. Kitty graphics and Unicode image placement are a deferred
  future parity workstream; Sixel and OSC 1337 are excluded.
- A client-callable terminal transfer operation; intentionally absent from the
  public omnigent 0.5.1 contract.
- A bundled-font / font-registry story. The terminal renders with the **system
  monospace `Menlo`** referenced by name (macOS-only posture; guaranteed present).
  ⚠ Menlo's grid-alignment is an **unproven hypothesis** — Spike A only proved a
  *missing* font mis-aligns and never tested Menlo — so Slice 1c **gates** it (see
  Render contract); **fallback = bundle a font** (reopening `lens-fonts`) if the
  gate fails. Font *selection* (bundled defaults, user-supplied files, Nerd-Font
  symbol fallback) is a runtime **font registry** deferred to the settings
  workstream and owned by `lens-app` (`docs/SPEC-GAPS.md` §7).

## VT foundation (replaces the dead "grounded adoption result")

The VT engine is **vendored `libghostty-rs`** (Uzaaft, MIT/Apache), built from a
pinned Ghostty **dev** commit with the patched `zig@0.15` toolchain. Two crates
are vendored under `vendor/libghostty-rs/`: `libghostty-vt-sys` (checked-in
bindings) and `libghostty-vt` (the safe `Terminal`/`vt_write`/`RenderState`/`Cell`
API). Ghostty *source* is fetched by `build.rs` at a pinned commit (crates-only
vendoring; a `GHOSTTY_SOURCE_DIR` tree and a prebuilt `.a` are both deferred to
the same trigger — when CI lands). Provenance is ordinary dependency vetting, not
a per-file adopt/adapt/exclude manifest. gpui-ghostty is reference-only.

- **No Ghostty type ever escapes `lens-terminal`'s engine boundary.** The public
  surface exposes only Lens-owned values (`Frame`, presentation, lifecycle).
- **Graphics exclusion by construction.** `lens-terminal`'s render layer paints
  only text + background quads (`paint.rs`), so image/graphics cells render blank.
  `libghostty-vt` parses APC/DCS; unsupported image payloads are consumed with
  strict bounds and no per-byte warnings. Lens does not inherit Ghostty's large
  Kitty image-allocation defaults.
- Build reproducibility: **a clean build fetches** the pinned Ghostty source
  (blobless, cached in `OUT_DIR`) and Zig's system deps — pinned/reproducible but
  **not offline**; only cached rebuilds are offline (see Scope). The `ZIG`
  override + one-line `build.rs` patch are documented in the vendor README and
  re-applied on each pin bump.

## Pinned omnigent 0.5.1 facts (corrected by Spike B)

REST assertions are grounded in `openapi.json` (paths
`/v1/sessions/{sid}/resources/terminals[/{tid}]`; schemas `SessionResourceObject`,
`ResourceEventData`, `SessionResourceCreatedEvent`, `SessionResourceDeletedEvent`,
`SessionSupersededEvent`). WS facts absent from OpenAPI were source-audited at
`08285468`; the wire contract + `4404` were **live-verified** (Spike B, B2);
`4405`/`4500` remain **source-derived** (not live-triggered).

- Terminal resources expose `id`, `session_id`, `name`, `environment`, and
  metadata incl. `terminal_name`, `session_key`, `running`, `terminal_transport`.
  `TerminalId` is opaque to callers even though the server derives it
  deterministically from `(terminal_name, session_key)`.
- Public REST supports list/create/get/delete. The internal transfer route for
  native `/clear` is hidden from OpenAPI and is **not** a Lens capability.
- Attach is `WS /v1/sessions/{sid}/resources/terminals/{tid}/attach`. The `101`
  upgrade happens **before** the terminal lookup (a bad `tid` gets `101` then an
  app-level close). No auth on a local dev server (`permission_store is None`).
- **The wire is transport-independent raw VT** (the load-bearing Spike-B finding).
  omnigent has two server-side bridges — `control` (default) and `pty` — but both
  deliver the **same raw-VT wire contract** (binary frames of raw VT); the tmux
  control-mode `%output` protocol is decoded **server-side**. **The client cannot
  tell which served it, needs no tmux-control parser, and does not request a
  transport** (correcting the 2026-07-14 spec's `transport=pty`). The reconnect
  **seed bytes/sizes differ** (control `capture-pane` ~1.4 KB, pty `tmux attach`
  redraw ~3.1 KB), both raw VT, both prefixed with a clear. ⚠ Whether feeding a
  reconnect seed into a **retained** engine (which already holds scrollback)
  duplicates history is **NOT proven by Spike B** — it is a **Slice 1d acceptance
  test** (see Threading). `pty` remains a **documented** escape hatch (a future
  change, never a silent per-attach toggle).
- Framing: server→client **binary** = raw VT → feed verbatim to `vt_write`;
  client→server **binary** = keystrokes/paste/mouse + the `on_pty_write` DA/DSR
  back-channel; client→server **text** = `{"type":"resize","cols":N,"rows":M}`.
  Output is a byte stream, not messages (server coalesces reads into bounded
  frames, flood cap 64 KiB / interactive cap 2 KiB; feed the concatenation).
- `read_only=true` requires read access, drops binary input, retains resize,
  attaches tmux `-r`. Interactive attach requires session-owner level.
- **Reconnect = current-screen redraw, no byte-replay.** Re-attaching the same
  `tid` yields a fresh snapshot of the current screen; output emitted while
  detached is applied to the tmux pane but never replayed — a **transient gap**,
  not state loss. tmux panes outlive detach; the native-pane reaper reclaims an
  idle pane after ~30 min (`OMNIGENT_NATIVE_PANE_IDLE_TIMEOUT_S`).
- **Close codes** — `lens-client` **classifies** these typed causes; `lens-terminal`
  owns the *policy* (see Lifecycle), because stop/retry/downgrade/reattach needs
  terminal identity + lifecycle state the transport layer must not hold. `4404`
  terminal missing/dead (**live-confirmed** via a bogus `tid`) → policy: stop.
  `1008` authorization → policy: disable input, refresh access, may downgrade to
  read-only. `4405` tmux client detached while terminal alive (source-derived) →
  policy: `Detached` with reattach **available, not automatic** (must NOT read as
  gone). `4500` internal (source-derived) → policy: retry with backoff. Generic
  network closure has no replay/sequence proof.
- Agent switch resets terminals: `session.resource.deleted`, then any successor
  as a new `session.resource.created`.
- Native `/clear` moves the same running terminal internally and emits the
  public, transient `session.superseded { conversation_id, target_conversation_id,
  reason }`. `lens-ui` owns that session redirect and passes it into the tab.
- Some server-owned terminals may be recreated on attach reusing the same
  deterministic ID; a second `session.resource.created` is the only live
  generation signal. Resource events are normally also persisted as
  `ResourceEventData` items, but 0.5.1 exposes **no immutable generation token**
  and persistence is best-effort (see "Open contract gaps").

## Render contract (from Spike A)

**Full-snapshot repaint.** Each frame, re-read every visible cell from the engine
snapshot, rebuild an immutable `Frame`, and emit all quads + glyphs. Measured
**ASCII PerRow** full-redraw p95 = **2.77 ms @ 200×50** (budget 8.3 ms); the
snapshot read is ≪ 0.2 ms; **shaping + primitive emission dominate** (snapshot is
negligible). **⚠ The PerCell wide/emoji path is heavier and can miss budget:
~6.3 ms @ 200×50, ~16.5 ms @ 400×100 (over 8.3 ms).** Therefore:

- **Dirty/damage tracking is NOT part of the contract** — an optional later
  optimization only. No per-frame diffing machinery.
- **Wide/emoji cells need per-cell glyph placement.** Per-row `shape_line` drifts
  CJK/emoji off the monospace grid; ASCII rows shape per-row (fast). The lifted
  painter uses PerRow for ASCII rows and PerCell for any row containing a wide
  cell.
- **System monospace `Menlo` — unproven hypothesis, gated.** References `Menlo`
  by name (macOS-guaranteed). Spike A **never tested Menlo** — it only proved a
  *missing* font falls back to proportional. Slice 1c **gates** Menlo with a
  **live-GPUI** test (the probe needs a real text system — *not* a static
  build-time check): exact non-fallback resolution + post-emoji advance +
  box-drawing alignment. **If the gate fails, bundle a font** (reopen
  `lens-fonts`). No bundled font *unless the gate forces it*.
- **Full SGR is new work, not spike-proven.** `paint.rs` today emits only
  fg/bg/bold/selection (underline/strikethrough always `None`; no italic/dim/
  reverse). Slice 1c **extends** it to the full attribute set the wire carries;
  the spike proved layout viability, not SGR coverage.
- **Liftable artifact:** `spikes/terminal-render/src/paint.rs` (cell→quad+glyph
  mapping). Three codex-found fixes to apply on lift: (1) key any shape cache on
  `(font_size, font, content)` + retain the full key — or drop the cache (it
  barely helped); (2) the alignment probe must check the cell *after* an emoji,
  not just its start; (3) clear dirty only after confirmed-successful paint and
  surface paint errors.
- **⚠ Perf caveat:** 2.77 ms is paint-closure CPU only (no vsync/present).
  Slice 1c **re-measures end-to-end** in the real GPUI demo before the perf
  verdict is treated as final.

## Threading & the `Frame` seam

The hard constraint: the `libghostty-vt` `Terminal` is **non-`Send`** — it never
moves between threads or is touched from two. Everything follows:

- **Engine worker = a dedicated, pinned `std::thread`** that owns the `Terminal`
  for its lifetime and is the single writer of terminal state. It **cannot** use
  gpui's background executor (which may migrate work across threads). Spawn once,
  own forever. **Teardown = signal stop → the worker drains + exits → an
  off-foreground `join`.** Dropping a `JoinHandle` only *detaches*; it does NOT
  stop the worker or free the `Terminal`/scrollback — so Sleep/replacement
  (which must actually reclaim the engine) completes only after a **confirmed
  worker exit**. This is the create/teardown/recreate worker-lifecycle control.
- **`Frame` is the Send boundary.** The engine snapshots and builds the immutable
  `Frame` (plain owned cell data — fg/bg/style/width/grapheme) **on the engine
  thread**, then publishes it as an `Arc<Frame>`. No Ghostty type crosses.
- **Engine→transport reverse channel.** The engine also *emits* bytes — VT replies
  (DA/DSR) via `libghostty`'s `on_pty_write` callback. These ride a **bounded
  engine→transport channel** to the WS binary-input path; the callback must **not**
  block the engine loop on WS I/O. This is distinct from UI→engine keystroke input.
- **Publish-and-wake, throttled-to-coalesce.** The engine **throttles** snapshot→
  `Frame` construction to at most once per display sample (mark dirty on new bytes;
  build a `Frame` only when due — **not** per byte/chunk), publishes it into a
  shared slot (`ArcSwap`/mutex-`Arc`), and sends a **coalesced foreground wake**
  (`cx.notify()` via an async handle) under a **lost-wake-safe dirty/ack**
  protocol. The UI samples the slot on wake / its `request_animation_frame`.
  `ArcSwap` coalesces *paints* but neither schedules RAF nor throttles construction
  — hence the explicit wake **and** worker-side throttle (continuous RAF would burn
  foreground work while idle). A visibility change (hidden→shown) forces one
  publication.
- **Backpressure** rides the transport→engine bounded channel: sustained
  saturation enters **visible reconnect** rather than dropping PTY bytes. The
  render side never backs up (it samples, not queues).
- **Hidden tabs** keep parsing (engine thread runs, scrollback stays live) but
  **stop publishing Frames** → zero GPUI cost while hidden; on show, publish one
  coalesced latest Frame.

This is `lens-core`'s single-writer + replica pattern, with the one twist that the
writer owns a non-`Send` FFI object and therefore runs on an OS thread, not the
gpui executor.

## Module ownership

### `lens-client` — terminal protocol

Owns all omnigent wire knowledge: typed terminal resource/request/response and WS
frame/control values; URL/path construction; auth; access query; pre-attach GET;
close-code classification; bounded reader/writer queues. Off-foreground connect +
reconnect. Arbitrary PTY chunks are never dropped — sustained queue saturation
deliberately disconnects into the visible reconnect flow. The attachment owns no
Ghostty, presentation, scrollback, routing, or policy. *(This is the deferred
lens-client "Plan 7" terminal surface; no `tungstenite`/`terminal.rs` exists yet
— built in Slice 1a.)*

### `lens-terminal` — deep terminal module

Public identity values:

```rust
pub enum TerminalTarget {
    Existing { session_id: SessionId, terminal_id: TerminalId },
    OpenOrCreate { session_id: SessionId, key: TerminalKey },
}

pub enum AccessIntent { Automatic, ReadOnly }
```

`TerminalKey` holds `terminal_name` + `session_key`. Access is separate from
identity: `Automatic` prefers write for the owner, read-only for other viewers;
server authorization remains authoritative. A caller may force read-only but never
assert authoritative write.

```rust
pub fn open(
    target: TerminalTarget,
    client: Arc<Client>,
    options: TerminalOpenOptions,
    cx: &mut App,
) -> Entity<TerminalTab>;
```

Returns immediately in `Starting`; discovery/create/attach run off-thread;
failures become lifecycle values, not constructor errors. `TerminalOpenOptions`
holds only access intent, a scrollback limit, and initial user preferences.

Remaining interface:

- `TerminalTab::focus_handle(cx)` — host-driven focus (direct, not a callback).
- `TerminalTab::presentation()` — latest atomic title/lifecycle/access/progress.
- One typed inbound `TerminalHostEvent` seam (session Sleep/wake/reset,
  `session.superseded`, normalized resource-generation signals, preference
  changes, memory pressure, typed host-request responses).
- One typed outbound `TerminalEvent` stream (presentation changes + host
  requests). No arbitrary `RequestClose`; no client transfer request.

Host requests cover user-gesture URL opening, permissioned OSC 52 clipboard
writes, and background notifications; permission-requiring requests carry a typed
request ID and a typed response. Progress is presentation state, not an OS side
effect.

Internally: the engine worker + `Frame` seam above; a transport bridge to
`lens-client`'s attachment through bounded queues; the full-snapshot GPUI render.
Parsing, I/O, lock waits, and unbounded allocation never run in `render` or on the
foreground thread.

Every layer implements the repo's typed, serializable `Inspect` contract
(transport/queue/reconnect; engine dimensions/history/lifecycle; render
visibility/frame stats) with a fixed-capacity diagnostic ring, local/permission-
gated, distinct from `TerminalEvent`, performing **zero** snapshot construction,
event recording, allocation, or synchronization on hot paths while disabled.

### `lens-ui` — host adapter and policy

Chooses `Existing` vs `OpenOrCreate`, resolves `ConnectionId → Arc<Client>`,
supplies access intent/preferences, wraps the returned entity in its `ContentTab`
adapter, and owns final chrome + OS policy. It makes no terminal REST/WS calls and
never handles Ghostty types. *(Now present on-branch after the main merge.)*

## Identity and replacement semantics

- `Existing` attaches only to the named resource; never adopts a different
  resource or relaunches a process.
- `OpenOrCreate` discovers/creates the exact logical key **only during initial
  opening**; it is not a perpetual keep-alive.
- Manual deletion, unexplained disappearance, or same-ID recreation outside a
  positively identified reset → freeze the final frame → `Detached`; recreation is
  always an explicit user action.
- During a positively identified agent reset, `OpenOrCreate` may wait for and
  adopt the new exact-key successor, creating a **fresh** engine (never mixing old
  and new history). `Existing` stays `Detached` (identity changed).
- During `session.superseded`, both targets may follow the same `TerminalId` into
  the target session (server moved the same PTY); the existing engine is retained;
  lens-ui owns the surrounding redirect.
- Before every reconnect, GET the exact terminal and consult persisted
  resource-event history. `404` stops reconnect. A duplicate `resource.created`
  for the attached ID is a new generation despite ID reuse. With no immutable
  generation token and best-effort event persistence, a narrow missed-event race
  remains an explicit upstream contract gap.

## Lifecycle

The tab renders modeled values and never panics. States: `Starting`, `Live`,
`Reconnecting`, `ReplacementWaiting`, `Sleeping`, `Ended`, `Detached`, with
effective read-only/write modeled separately.

- Generic transport/internal failure retries **30 s** with bounded exponential
  backoff; the retained frame is frozen read-only while retrying. A successful
  same-resource reconnect **always** adds a persistent "output during the
  interruption may be missing" marker (no replay/sequence proof exists).
- `4404`, terminal GET `404`, deletion, or exhausted retry → `Detached`. `4405` →
  `Detached` meaning "terminal still running; client detached" + an explicit
  reattach action; Lens does not fight an intentional tmux detach loop.
- `1008` write rejection disables input immediately; refresh access, may reattach
  read-only; loss of read access → authorization `Detached`.
- `Ended` only for positively reported process termination (may show exit code).
  0.5.1 exposes no event distinguishing normal exit from deletion/transfer, so
  ambiguous disappearance is `Detached`, never guessed `Ended`.
- Normal exit never auto-closes a tab. `OpenOrCreate` offers explicit relaunch
  only after positive `Ended`; otherwise the action reads "Create terminal again."

### Deliberate Sleep/wake

Sleep ≠ reconnect. It closes the WS and **releases the engine + full scrollback**
so resources are actually reclaimed; the tab retains only an immutable final
viewport labeled `Session sleeping`. Wake reattaches only if the same observed
resource generation survived (pre-reconnect GET + generation guard, recreate as
needed); else `Detached`. Sleep adds no reconnect-gap marker and never
auto-creates a terminal.

## Terminal behavior and policy

- Hidden/minimized open terminal stays attached and keeps parsing; suppresses GPUI
  frame publication; becoming visible publishes one coalesced latest frame.
- Read-only viewers scroll/select locally but send no keyboard/paste/mouse input.
- Mouse follows Ghostty/XTSHIFTESCAPE: Shift normally enables local selection, a
  TUI may capture it, a runtime toggle can force mouse-local.
- `Cmd+V` is a local gesture; bracketed paste preserved; multiline paste warns
  when bracketed mode is inactive (global disable / "Don't warn again"); read-only
  tabs expose no paste.
- OSC 52 writes: strict decoded payload cap + host permission (allow-once /
  allow-for-session) + copy notice. OSC 52 reads denied.
- Validated plain URLs and OSC 8 links are actionable only after a user gesture →
  typed host requests; terminal output never opens a browser.
- OSC progress updates terminal-local presentation; notification sequences become
  rate-limited host requests only while unfocused/backgrounded, suppressed for
  read-only observers by default.
- Stable `identity_title` = `terminal_name:session_key`. Sanitized, bounded OSC
  0/2 text is optional `reported_title` (lens-ui composes/truncates the visible
  title); reported text is never identity/routing/authorization/OS-window title;
  survives same-resource reconnect; clears on replacement.

## Scrollback, memory, resize

- Retain one bounded emulator state (no second raw-byte ring). **Near-term the cap
  is a LINE count** — `libghostty-vt::Options::max_scrollback` is *lines*, not
  bytes, and the binding exposes no byte-level trim (verified in the vendored
  source). Provisional default ≈ the line-equivalent of Ghostty's **10,000,000-byte
  (decimal, not MiB)** app default; lazily allocated, oldest-first eviction, visible
  grid always preserved; applies to newly opened terminals. **Byte-accurate
  retained-byte accounting + selective byte-trim require a safe-FFI extension —
  deferred to Slice 3/4** (fleet memory pressure). Until then fleet accounting
  *estimates* bytes (≈ lines × cols × per-cell size).
- Track retained bytes fleet-wide (estimated until the FFI extension lands). Under
  macOS memory warning, trim
  least-recently-viewed hidden histories first + insert a visible truncation
  marker. Under critical pressure, disconnect least-recently-viewed hidden tabs
  (retain final viewport, expose explicit reattach). Never silently drop PTY
  bytes; keep the active tab connected; trim its old history only as a last resort.
- Live resize coalesces GPUI geometry, reflows off-thread, sends only the newest
  `{cols, rows}`. During reconnect the retained engine tracks geometry; the newest
  size is sent **before** input is re-enabled. Replacement engines start at current
  geometry. `Ended`/`Detached`/`Sleeping` preserve the final grid; container
  resize changes only clipping/padding.
- Scrollback is memory-only; released on tab close, deliberate Sleep, or Lens
  exit; never silently persisted to disk.

## Performance and verification

Release benchmarks run on the available Apple Silicon machine, recording hardware,
macOS, commit, build metadata. Acceptance:

- p95 frame time ≤ 8.3 ms; p99 ≤ 11.1 ms; ≤ 0.1% of frames > 11.1 ms;
  input-event-to-first-paint measured separately.

Required workloads: rapid typing with echo; sustained/bursty styled output;
**dense wide/emoji @ 200×50 and 400×100** (the PerCell budget risk); scrolling a
full history (line-equivalent of 10,000,000 bytes); continuous resize/reflow;
hidden-to-visible catch-up; one visible terminal with several hidden terminals
streaming. The scrollback default and fleet soft budget are provisional until
measurements include real RSS. The PerCell wide/emoji path is a **fail-closed
gate**: if it misses budget at a required grid, Slice 1c/4 must land an
optimization (or the budget explicitly re-scoped), not silently pass.

Benchmarks at every level: Criterion for `lens-client` WS frame
classification/control codec and bounded-queue throughput; engine benches for VT
parse, `Frame` construction, scroll, reflow; the GPUI frame-timing harness for the
end-to-end workloads. Release results record throughput, latency, and memory.

Completion also requires deterministic typed transport/lifecycle tests, GPUI
focus/input/render tests, a real external omnigent discover/create/type/resize/
drop/reconnect flow, `rustfmt`, and workspace-wide
`cargo clippy --workspace --all-targets -- -D warnings`. *(There are no upstream
Ghostty tests to "port": `libghostty-rs` ships its own suite as the upstream
coverage; our tests cover our mapping.)*

## Build sequence (slices)

The design freezes the **full** public surface above; slices build behind it, each
planned against APIs that actually landed. **This cycle builds through Slice 1,
then reassesses with real end-to-end perf.** Inspect + benchmarks are threaded
per-slice, never deferred.

- **Slice 0 — Surface freeze (names + invariants, not representations).** Freeze
  the **opaque public type *names*** lens-ui binds to — `open`/`TerminalTarget`/
  `AccessIntent`/`TerminalOpenOptions`, the 7 lifecycle **variant names**,
  `TerminalHostEvent`/`TerminalEvent` (opaque seams), `Frame` (opaque immutable
  snapshot) — plus the **seam invariants** (`open` returns in `Starting`; failures
  are lifecycle values; no Ghostty type escapes; exactly one inbound + one outbound
  event stream; `focus_handle`/`presentation` accessors). **Internal
  representations stay evolvable** — `Frame` fields, event payloads, options fields
  fill in as their producing+consuming slices converge (avoids premature
  layer-boundary binding). Crate skeletons (`lens-terminal`, demo).
- **Slice 1 — Live vertical slice to first pixels** (four plans):
  - **1a — `lens-client` transport.** Typed REST list/get/create/delete +
    auth/access-modeled WS attach + **classify** close causes
    (`4404`/`4405`/`4500`/`1008`) + reconnect *mechanics* (backoff, re-attach) +
    bounded queues with saturation→visible-reconnect. **Close-code *policy*
    (stop/retry/downgrade/reattach) lives in `lens-terminal` (1d), not here.**
    Omits `transport=`. Deterministic tests + live REST rider.
  - **1b — `lens-terminal` engine core.** Engine worker thread owning the
    non-`Send` `Terminal`; `vt_write`; snapshot→`Frame`; resize. Driven by
    replayed Spike-B captures — offline/deterministic.
  - **1c — `lens-terminal` render.** Lift `paint.rs` + the 3 fixes + per-cell
    wide/emoji + **extend to full SGR** (paint.rs does fg/bg/bold/selection only —
    italic/underline/strikethrough/reverse/dim are new work) + **gate system
    `Menlo`** with the live-GPUI resolution/alignment test (bundle-fallback if it
    fails). Paints a `Frame`. GPUI frame-timing smoke gate incl. **dense
    wide/emoji @ 200×50 and 400×100 (fail-closed)** → re-measure end-to-end.
  - **1d — Convergence + demo + live proof.** Wire `open()`/`TerminalTab`/
    `presentation()`; transport↔engine bridge; lifecycle subset
    `Starting`/`Live`/`Reconnecting`/`Detached` + reconnect gap marker; standalone
    GPUI demo; live vertical proof vs real omnigent 0.5.1 (attach/input/output/
    resize/forced-drop/same-resource-reconnect/retained-engine/gap-marker).
    **Owns close-code policy** (consuming 1a's classification) + the DA/DSR
    forwarding + **hidden-tab Frame suppression** (it's part of the Frame seam) +
    the **retained-engine-seed acceptance test** (does re-seeding a retained engine
    duplicate scrollback? define the expected clear/redraw + gap-marker semantics).
    1a∥1b are independent; 1c needs 1b; 1d needs 1a+1b+1c.
- **Slice 2 — Interaction** (next cycle): keyboard/IME/paste/selection/copy/mouse
  modes, OSC 52 policy, titles, hyperlink gestures, read-only gating.
- **Slice 3 — Lifecycle & fleet:** reset/supersession/generation guard, Sleep/wake
  reclaim (confirmed-worker-exit teardown), `ReplacementWaiting`/`Sleeping`/`Ended`,
  memory-pressure trim/disconnect + the byte-accounting FFI extension. (Hidden-tab
  Frame suppression lands in 1d; fleet *trim/disconnect* is here.)
- **Slice 4 — Perf acceptance:** full workload harness, real RSS, the numeric
  frame-budget gate, shipping evidence.

Build discipline (house style + `CLAUDE.md`): subagent-driven, composer-2.5
authors, ≥1 cross-family review at each seam, TDD, frequent commits.

## Completion matrix (anti-drop)

Every requirement maps to a slice; deferral is explicit, never forgotten.

| Requirement | Slice(s) |
| --- | --- |
| Public API surface — opaque names + seam invariants (`open`/targets/access/events/`Frame`/7-variant lifecycle) | 0 |
| Transport: REST CRUD, WS attach, close-cause **classification**, reconnect mechanics, backpressure→reconnect | 1a |
| Auth / read-only access modeling (attach-level) | 1a (transport), 2 (read-only UI gating) |
| Close-code **policy** (stop/retry/downgrade/reattach) | 1d |
| Engine: VT parse, scrollback (line-cap), `Frame`, resize reflow (non-`Send` worker) | 1b |
| DA/DSR (`on_pty_write`) reverse channel | 1b (engine callback), 1d (forward to WS) |
| Resize end-to-end: codec / engine reflow / newest-size-before-input ordering | 1a + 1b + 1d; during-reconnect 3 |
| Render: full-snapshot, per-cell wide/emoji, **full-SGR (extend `paint.rs`)**, **Menlo gate** (bundle-fallback) | 1c |
| Frame publish/wake protocol + hidden-tab Frame suppression | 1d |
| Lifecycle basic (`Starting`/`Live`/`Reconnecting`/`Detached`) + gap marker | 1d |
| Retained-engine reconnect-seed semantics (scrollback dup / gap-marker) | 1d (acceptance test) |
| Live vertical proof vs real omnigent | 1d |
| Identity/replacement: `Existing`/`OpenOrCreate`, generation guard | 1d (basic guard), 3 (full) |
| Interaction: keyboard/IME/paste/selection/copy/mouse; OSC 52 write-cap + **read-denial**; OSC progress + background notifications; titles; hyperlink gestures | 2 |
| Lifecycle full: `ReplacementWaiting`/`Sleeping`/`Ended`, Sleep/wake (confirmed-exit teardown), supersession | 3 |
| Fleet memory-pressure trim/disconnect + **byte-accounting FFI extension** | 3 |
| Inspect + diagnostic rings (disabled-path proof) | per-slice (1a/1b/1c/1d **+ 2/3 extensions**), integrated 4 |
| Benchmarks (client codec/queue, engine parse/frame/scroll/reflow, GPUI frame incl. dense wide/emoji) | per-slice, full harness 4 |
| Verification gates: deterministic + GPUI + live tests, `rustfmt`, workspace clippy | per-slice, full 4 |
| Perf acceptance (numeric budgets, real RSS, PerCell fail-closed gate, workloads) | 4 |
| Build acceptance: offline-after-cache; full clean-build offline (vendored Ghostty tree) | 4 / CI trigger |
| Font registry / bundled defaults / Nerd-Font symbols | deferred → settings workstream (SPEC-GAPS §7) |

## Open contract gaps

- **No immutable terminal generation token.** Same-`(terminal_name, session_key)`
  ID reuse means a same-ID reconnect cannot prove same-instance vs. fresh. The
  pre-reconnect GET + duplicate-`resource.created` signal is best-effort; the
  residual race is a documented 0.5.1 gap (parked omnigent feature-request; see
  `docs/SPEC-GAPS.md`). Never invent a client-side generation ID pretending to
  close it.
- **`session.superseded` redirect target is dropped by lens-core today**
  (`reduce/folds.rs` folds it to nothing) — surfacing it (e.g.
  `StreamUpdate::Superseded { target_conversation_id, reason }`) is Slice 3 /
  terminal-integration work, not the lens-ui skeleton (cross-spec risk recorded in
  SPEC-GAPS).
