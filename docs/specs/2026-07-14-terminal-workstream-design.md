# Shared terminal infrastructure & E2E GPUI terminal tab

> **⚠️ SUPERSEDED (2026-07-15) — VT-adoption mechanism only.**
> This doc's Ghostty story — a "narrow, attributed **port** of gpui-ghostty"
> gated by a provenance manifest that classifies every candidate file
> adopt/adapt/exclude — is **dead**. The terminal VT is now a **vendored
> `libghostty-rs` binding built from source** against a pinned Ghostty dev
> commit (patched `zig@0.15`). See memories `terminal-vt-adoption-model` +
> `zig-ghostty-macos26-scissor`, `docs/STATUS.md`, and
> `docs/handoffs/2026-07-15-terminal-vt-libghostty-rs.md`.
>
> The model-**independent** goals here still largely hold (omnigent WS
> transport, GPUI render/foreground-safety, typed boundaries owner-write /
> viewer-read-only). A new design pass will supersede this doc in full; until
> then, treat the VT-adoption sections as historical.

**Status:** SUPERSEDED (see banner) — was: user-approved design, reconciled after the 2026-07-14 terminal grill

## Goal

Build the shared terminal foundation as a standalone, renderable GPUI terminal
tab. It attaches to a real omnigent terminal, keeps terminal protocol knowledge
typed from transport through presentation, and is ready for `lens-ui` to host
without making `lens-ui` learn terminal REST, WebSocket, or Ghostty details.

Ghostty VT is the emulator. Lens takes a narrow, attributed port of the relevant
`gpui-ghostty` implementation, keeps its Ghostty VT and GPUI rendering work, and
replaces its local-PTY transport with omnigent's authenticated terminal
WebSocket.

## Scope

This slice delivers:

- Typed terminal list/get/create/delete and authenticated WS attach in
  `lens-client`; no generic WebSocket or JSON leaks to callers.
- A deep `lens-terminal` module with a small host interface and a standalone
  GPUI demo as its first consumer.
- Owner-write and viewer-read-only behavior; keyboard, IME, paste, resize,
  selection/copy, scrollback, mouse modes, titles, hyperlinks, and safe OSC
  handling.
- Brief reconnect with retained Ghostty emulator state and an explicit marker
  that output during the interruption may be missing.
- Reproducible Ghostty/GPUI/Zig inputs, deterministic tests, pinned-omnigent
  executable verification, and release-mode terminal benchmarks.

Out of scope:

- The native-harness rendered-stream/raw-TUI toggle; it has its own spec cycle.
- A generic incremental Bash-tool output surface. Omnigent 0.5.1 returns
  one-shot shell output and exposes no `call_id`-correlated stdout/stderr stream.
- Integrating the tab into the production `lens-ui` working area.
- A local PTY or `portable-pty`; omnigent owns the PTY.
- Inline graphics. Kitty graphics and Unicode image placement are deferred as
  one future parity workstream; Sixel and OSC 1337 are explicitly excluded.
- A client-callable terminal transfer operation. It is intentionally absent
  from the public omnigent 0.5.1 contract.

## Grounded adoption result

The adoption audit pins `gpui-ghostty` at
`e3025981c6211dd7db2a825dc364ffb5d342f45e` and its Ghostty submodule at
`6d2dd585a5d87fa745d48188dd096ca6e63014d0`. Before code enters Lens, the
provenance manifest must also pin the compatible GPUI and Zig inputs, retain
Apache-2.0/MIT notices, and classify every imported file as **adopt**,
**adapt**, or **exclude**.

The audit established that Ghostty implements Kitty graphics upstream, but the
candidate GPUI bridge exposes no APC/image state through its Zig/C/Rust seam and
paints only text and quads. Unsupported APC/DCS payloads must therefore be
consumed with strict bounds and without per-byte warnings. Unsupported Unicode
image-placeholder clusters render blank rather than as visible garbage. Lens
does not inherit Ghostty's large Kitty image allocation defaults.

## Pinned omnigent 0.5.1 facts

REST/resource assertions below are grounded in
`vendor/omnigent-0.5.1/openapi.json`: paths
`/v1/sessions/{session_id}/resources/terminals` and
`/v1/sessions/{session_id}/resources/terminals/{terminal_id}`, plus schemas
`SessionResourceObject`, `ResourceEventData`, `SessionResourceCreatedEvent`,
`SessionResourceDeletedEvent`, and `SessionSupersededEvent`. WS/internal facts
absent from OpenAPI were audited at omnigent source revision `08285468` in
`server/routes/terminal_attach.py`, `server/app.py`, `terminals/ws_bridge.py`,
`terminals/control_bridge.py`, and `server/routes/sessions.py`.

- Terminal resources expose `id`, `session_id`, `name`, `environment`, and
  metadata including `terminal_name`, `session_key`, `running`, and
  `terminal_transport`. `TerminalId` remains opaque to callers even though the
  current server derives it deterministically from the logical key.
- Public REST supports list/create, get, and delete. The server has an internal
  transfer route for native `/clear`, but it is hidden from OpenAPI and is not a
  Lens capability.
- Attach is
  `WS /v1/sessions/{session_id}/resources/terminals/{terminal_id}/attach`.
  Server output and client input are binary PTY bytes. Resize is a text frame:
  `{"type":"resize","cols":N,"rows":M}`.
- Lens explicitly requests `transport=pty`. Omnigent's default `control` mode
  captures tmux history on attach for xterm.js; feeding that capture into a
  retained Ghostty engine could duplicate history and cannot restore all VT
  state exactly.
- `read_only=true` requires read access, drops binary input, retains resize, and
  attaches tmux with `-r`. Interactive attach requires session owner level.
- Authorization rejection closes with `1008`; `4404` means the terminal is
  missing/dead; `4405` means the tmux client detached while the terminal remains
  alive; `4500` is an internal attach failure. Generic network closure has no
  replay or sequence proof.
- Agent switch resets terminals and emits `session.resource.deleted`; any
  successor arrives as a new `session.resource.created` resource.
- Native `/clear` moves the same running terminal internally and emits the
  public, transient `session.superseded { conversation_id,
  target_conversation_id, reason }` event. `lens-ui` already owns that session
  redirect and passes it into the terminal tab.
- Some server-owned REPL/Qwen terminals may be recreated on attach while
  reusing the same deterministic ID. A second `session.resource.created` is the
  only live generation signal. Resource lifecycle events are normally also
  persisted as `ResourceEventData` items for reconnect discovery, but 0.5.1
  exposes no immutable generation ID and direct publication can outlive a
  failed best-effort persistence attempt.

The 2026-07-14 audit exercised 32 focused omnigent tests covering resource
shapes, auth, proxying, transport selection, read-only input, resize, close
codes, reset deletion, exit classification, and `/clear` supersession. A full
external-server discover/type/resize/drop flow remains a shipping sentinel,
not a reason to replace the pinned contract with remembered behavior.

## Module ownership

### `lens-client`: terminal protocol module

`lens-client` owns all omnigent wire knowledge:

- Typed terminal resource/request/response values and WS frame/control values.
- URL scheme/path construction, authentication, `transport=pty`, access query,
  pre-attach GET, close-code classification, and bounded reader/writer queues.
- Off-foreground connection and reconnect work. Arbitrary PTY chunks are never
  dropped: sustained queue saturation deliberately disconnects into the visible
  reconnect flow.

The attachment does not own Ghostty, presentation, scrollback, app routing, or
policy prompts.

### `lens-terminal`: deep terminal module

The public identity values are:

```rust
pub enum TerminalTarget {
    Existing {
        session_id: SessionId,
        terminal_id: TerminalId,
    },
    OpenOrCreate {
        session_id: SessionId,
        key: TerminalKey,
    },
}

pub enum AccessIntent {
    Automatic,
    ReadOnly,
}
```

`TerminalKey` contains `terminal_name` and `session_key`. Access is separate
from identity: `Automatic` prefers write for the owner and read-only for other
viewers, while server authorization remains authoritative. A caller may force
read-only but may never assert authoritative write access.

The constructor is intentionally small:

```rust
pub fn open(
    target: TerminalTarget,
    client: Arc<Client>,
    options: TerminalOpenOptions,
    cx: &mut App,
) -> Entity<TerminalTab>;
```

It returns immediately in `Starting`; discovery, initial create, and attach run
off-thread. Failures become lifecycle values rather than constructor errors.
`TerminalOpenOptions` contains only access intent, a scrollback limit, and
initial user preferences.

The remaining interface is:

- `TerminalTab::focus_handle(cx)` for host-driven focus.
- `TerminalTab::presentation()` for the latest atomic title/lifecycle/access/
  progress value.
- One typed inbound `TerminalHostEvent` seam for session Sleep/wake/reset,
  `session.superseded`, normalized resource-generation signals, preference
  changes, memory pressure, and typed host-request responses.
- One typed outbound `TerminalEvent` stream for presentation changes and host
  requests. There is no arbitrary `RequestClose` and no client transfer request.

Host requests currently cover user-gesture URL opening, permissioned OSC 52
clipboard writes, and background notifications. Permission-requiring requests
carry a typed request ID and receive a typed response. Progress is terminal
presentation state, not an OS side effect.

Internally, a single engine worker owns non-`Send` Ghostty state; a transport
bridge binds it to `TerminalAttachment` through bounded queues; and the GPUI tab
renders only coalesced immutable frame/damage updates. Parsing, I/O, lock waits,
and unbounded allocation never run in `render` or on the foreground thread.

Each layer also implements the repository's stable typed, serializable
`Inspect` contract. On demand it exposes transport/queue/reconnect state,
engine dimensions/history/damage/lifecycle, and render visibility/cache/frame
statistics. When locally enabled, each layer records typed state transitions in
a fixed-capacity diagnostic ring; access is local/permission-gated. Inspection
is distinct from `TerminalEvent` and performs no snapshot construction, event
recording, allocation, or synchronization on hot paths while disabled.

### `lens-ui`: host adapter and policy

`lens-ui` chooses `Existing` versus `OpenOrCreate`, resolves
`ConnectionId -> Arc<Client>`, supplies access intent/preferences, wraps the
returned entity in its `ContentTab` adapter, and owns final chrome and OS policy.
It performs no terminal REST/WS calls and never handles Ghostty types.

## Identity and replacement semantics

- `Existing` attaches only to the named terminal resource. It never adopts a
  different resource or relaunches a process.
- `OpenOrCreate` discovers or creates the exact logical key only during initial
  opening. It is not a perpetual keep-alive promise.
- Manual deletion, unexplained disappearance, or a same-ID recreation outside a
  positively identified reset freezes the final frame and becomes `Detached`;
  recreation is always an explicit user action.
- During a positively identified agent reset, `OpenOrCreate` may wait for and
  adopt the new exact-key successor. It creates a fresh Ghostty engine and never
  mixes old and new history. `Existing` remains detached because the resource
  identity changed.
- During `session.superseded`, both targets may follow the same `TerminalId`
  into the target session because the server moved the same running PTY. The
  existing engine is retained; lens-ui owns the surrounding session redirect.
- Before every reconnect Lens GETs the exact terminal and consults persisted
  resource-event history. `404` stops reconnect. A duplicate `resource.created`
  for the attached ID is treated as a new generation, despite ID reuse. Because
  0.5.1 has no immutable generation token and resource-event persistence is
  best-effort, a narrow missed-event race remains an explicit upstream contract
  gap.

## Lifecycle

The tab renders modeled values and never panics the process:

`Starting`, `Live`, `Reconnecting`, `ReplacementWaiting`, `Sleeping`, `Ended`,
and `Detached`, with effective read-only/write access modeled separately.

- A generic transport/internal failure retries for 30 seconds with bounded
  exponential backoff. The retained frame is frozen read-only while retrying.
  A successful same-resource reconnect always adds a persistent marker that
  output during the interruption may be missing; omnigent supplies no replay or
  sequence proof.
- `4404`, terminal GET `404`, deletion, or exhausted retry becomes `Detached`.
  `4405` becomes `Detached` with the more precise meaning “terminal still
  running; client detached” and an explicit reattach action; Lens does not fight
  an intentional tmux detach loop.
- A `1008` write rejection disables input immediately. Lens refreshes access
  and may reattach read-only; loss of read access becomes an authorization
  `Detached` state.
- `Ended` is reserved for positively reported process termination and may show
  an exit code. Omnigent 0.5.1 exposes deletion and status effects but no public
  event that distinguishes normal exit from deletion/transfer, so ambiguous
  disappearance is `Detached`, never guessed as `Ended`.
- Normal exit never auto-closes a tab. `OpenOrCreate` may offer explicit
  relaunch only after positive `Ended`; otherwise the action is labeled
  “Create terminal again.”

### Deliberate Sleep/wake

Sleep is not reconnect. It closes the WS and releases the Ghostty engine and
full scrollback so resources are actually reclaimed. The open tab retains only
an immutable final viewport labeled `Session sleeping`. On wake it reattaches
only if the same observed resource generation survived; otherwise the viewport
becomes `Detached`. The missing immutable generation token leaves the narrow
same-ID race above. Sleep adds no reconnect-gap marker and never auto-creates a
terminal.

## Terminal behavior and policy

- A hidden/minimized but open terminal stays attached and keeps parsing output.
  It suppresses GPUI frame publication; becoming visible publishes one
  coalesced latest frame.
- Read-only viewers can scroll and select locally but send no keyboard, paste,
  or mouse input remotely.
- Mouse behavior follows Ghostty/XTSHIFTESCAPE. Shift normally enables local
  selection, a TUI may explicitly capture it, and a runtime toggle can force
  mouse interactions local.
- `Cmd+V` is a local user gesture. Bracketed paste is preserved. Multiline paste
  warns when bracketed mode is inactive, with a global disable/“Don't warn
  again” option. Read-only tabs expose no paste.
- OSC 52 writes require a strict decoded payload cap and host permission such
  as allow-once or allow-for-session, followed by a copy notice. OSC 52 reads
  are denied.
- Validated plain URLs and OSC 8 links are actionable only after a user gesture
  and become typed host requests; terminal output never opens a browser itself.
- OSC progress updates terminal-local presentation. Notification sequences
  become rate-limited host requests only while the tab is unfocused/backgrounded
  and are suppressed for read-only observers by default.
- Stable `identity_title` is `terminal_name:session_key`. Sanitized, bounded OSC
  0/2 text is optional `reported_title`; lens-ui composes/truncates the visible
  title. Reported text is never identity, routing, authorization, or the Lens OS
  window title. It survives same-resource reconnect and clears on replacement.

## Scrollback, memory, resize, and rendering

- Retain one bounded Ghostty emulator state, not a second raw-byte ring. The
  provisional per-terminal limit is Ghostty's app default of 10 MB (10,000,000
  bytes), allocated lazily, with oldest-first eviction and the visible grid
  always preserved. The setting applies to newly opened terminals.
- Track actual retained bytes fleet-wide. Under macOS memory warning, trim the
  least-recently-viewed hidden histories first and insert a visible truncation
  marker. Under critical pressure, deliberately disconnect least-recently-viewed
  hidden tabs, retain their final viewport, and expose explicit reattach. Never
  silently drop PTY bytes; keep the active tab connected and trim its old history
  only as a last resort.
- Live resize coalesces GPUI geometry, reflows the local engine off-thread, and
  sends only the newest `{cols, rows}`. During reconnect the retained engine
  tracks current geometry; the newest size is sent before input is re-enabled.
  Replacement engines start at current geometry. `Ended`, `Detached`, and
  `Sleeping` preserve the original final grid; container resize changes only
  clipping/padding.
- Scrollback is memory-only. It is released on tab close, deliberate Sleep, or
  Lens exit; terminal contents are never silently persisted to disk.

## Performance and verification

Release benchmarks run on the available Apple Silicon machine and record its
hardware, macOS, commit, and build metadata. Acceptance is:

- p95 frame time <= 8.3 ms;
- p99 frame time <= 11.1 ms;
- no more than 0.1% of frames above 11.1 ms;
- input-event-to-first-paint measured separately.

Required workloads are rapid typing with echo, sustained/bursty styled output,
scrolling a full 10 MB history, continuous resize/reflow, hidden-to-visible
catch-up, and one visible terminal with several hidden terminals streaming.
The 10 MB default and fleet soft budget are not final until these measurements
include real resident memory.

Benchmarks exist at every terminal stack level: Criterion coverage for
`lens-client` WS frame classification/control codec and bounded-queue
throughput; engine benchmarks for VT parsing, damage/frame construction,
scrolling, and reflow; and the GPUI frame-timing harness for the end-to-end
workloads above. Release results record throughput as well as latency and memory.

Completion also requires ported applicable upstream tests, deterministic typed
transport/lifecycle tests, GPUI focus/input/render tests, a real external
omnigent discover/create/type/resize/drop/reconnect flow, `rustfmt`, and
workspace-wide `cargo clippy --workspace --all-targets -- -D warnings`.
