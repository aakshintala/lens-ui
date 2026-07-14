# Shared terminal infrastructure & E2E GPUI terminal tab

**Status:** User-approved design, 2026-07-14

## Goal

Build the shared terminal foundation as a standalone, renderable GPUI terminal
tab. It must attach to a real omnigent terminal over its WebSocket interface,
handle live input, resize, output, short disconnects, and typed failure states.
`lens-ui` is deliberately not a dependency: it will host the completed tab as a
working-area surface later.

The terminal emulator is Ghostty VT. Lens will take a narrow, attributed port
of the relevant `gpui-ghostty` implementation rather than consume its
unpublished workspace or reimplement terminal rendering from scratch. The port
keeps Ghostty VT and GPUI rendering; Lens replaces its local-PTY transport with
the omnigent terminal WebSocket.

## Scope

The slice delivers:

- Typed terminal discovery, creation, deletion, transfer, and authenticated WS
  attach in `lens-client`; no generic WebSocket or JSON leaks to callers.
- A `lens-terminal` production crate with a public, host-friendly terminal tab
  interface and a standalone GPUI demo as its first consumer.
- Read/write attach for owners and read-only attach for other viewers; keyboard,
  paste, resize, selection/copy, scrollback, mouse/IME, terminal replies, and
  damage-aware rendering are in the adoption audit's behavior matrix.
- Brief reconnect with retained Lens-local terminal state and an explicit
  "output may have been missed" marker. The server has no terminal replay.
- A reproducible Ghostty/GPUI/Zig input chain, deterministic tests, live
  omnigent E2E verification, and release-mode terminal benchmarks.

Out of scope:

- The native-harness rendered-stream/raw-TUI toggle; it has its own spec cycle.
- Integrating the tab into `lens-ui` or designing its working-area container.
- A local PTY or `portable-pty`; omnigent owns the PTY and Lens attaches to it.
- Cross-session terminal-transfer tab routing; `lens-client` exposes the typed
  capability, while the future shell owns routing.

## Adoption gate: audit before port

No upstream source enters Lens before a dated adoption audit records the exact
`gpui-ghostty` commit, Ghostty revision, license/provenance of every candidate
file, and its disposition: **adopt**, **adapt**, or **exclude**. The result is
a deliberately small Lens-owned module, not a Git subtree or an ongoing fork.

The audit must cover:

1. Build provenance: exact GPUI, Ghostty, and Zig pins; static-link behavior;
   proof that a normal Cargo build never fetches source.
2. Rust/FFI safety: unsafe boundaries, ownership/lifetimes, error behavior, and
   Ghostty's single-thread ownership rule.
3. Lens compatibility: the upstream GPUI-version delta; no foreground I/O or
   waits; bounded queues; cache and damage behavior against the frame budget.
4. Terminal behavior: input, IME, bracketed paste, resize/reflow, scrollback,
   selection/copy, mouse modes, OSC/DSR handling, and title behavior.
5. Test and benchmark carry-over: every applicable upstream regression is
   ported or explicitly declined with a reason.

The audit also establishes the current WS behavior that is not represented in
the vendored OpenAPI document. Live verification, not a remembered route or
payload shape, is the contract sentinel for the attach path.

## Module design

### `lens-client`: terminal protocol module

`lens-client` owns all omnigent wire knowledge:

- Replace the existing untyped terminal lifecycle wrappers with typed resource
  models and request/response values as live bytes establish their shapes.
- Expose a managed `TerminalAttachment` for a `SessionId`, `TerminalId`, and
  attach access mode. It derives `ws`/`wss` from `Connection`, forwards the
  configured auth headers/cookie, sends binary input and typed resize controls,
  and receives binary PTY output.
- Keep background reader/writer/reconnect work off the GPUI foreground thread.
  Its public events are terminal-specific lifecycle values plus byte batches;
  bounded queues supply backpressure.

The attachment owns transport reconnect attempts. It does not own terminal
presentation, scrollback, or shell routing.

### `lens-terminal`: deep terminal module

`lens-terminal` is one production crate with internal modules, not several
shallow public crates. Its small external interface opens a GPUI terminal tab
from a typed target, `Arc<Client>`, and options. The future `lens-ui` host only
needs that entity and its typed host callbacks.

Internally:

- **Engine worker** — exclusively owns the `libghostty-vt` terminal state. It
  processes byte batches, encodes input, tracks scrollback/selection, and emits
  coalesced immutable frame/damage updates. Its non-Send Ghostty handles never
  cross threads.
- **Transport bridge** — binds one `TerminalAttachment` to that worker through
  bounded inbound and outbound channels. Tests use an internal scripted
  attachment adapter; consumers do not learn a transport trait.
- **GPUI tab** — owns focus, input/event routing, viewport metrics, clipboard
  actions, and cached Canvas render data. It reads the latest immutable update
  only; `render` performs no terminal parsing, I/O, lock wait, or unbounded
  allocation.
- **Demo binary** — opens the exact public terminal tab against a local or
  supplied omnigent connection. It is the E2E consumer and must not grow
  general `lens-ui` concerns.

## Lifecycle and errors

The tab renders values, never process-fatal errors:

`Starting` → `Live` → (`Reconnecting` → `Live` | `Detached`), with `ReadOnly`
as an access overlay. `session.terminal_pending` seeds `Starting`; an existing
resource is discovered or a terminal is created before attach.

During `Reconnecting`, the engine keeps its local screen and scrollback but
disables writes. A successful same-resource reconnect adds an honest gap marker
because the server supplies no replay. Terminal deletion, agent-switch resource
reset, server restart, or exhausted reconnect becomes `Detached`; Lens never
silently creates a replacement terminal.

Read-only mode renders and permits local selection/copy but rejects all
remote-writing input. Transfer remains a typed lifecycle capability; a future
shell closes or rebinds the source-session tab when it changes sessions.

## Build and verification

Lens pins the exact upstream source revision, Ghostty revision, and compatible
Zig toolchain in repository-controlled inputs. The normal build is offline with
respect to those sources; missing prerequisites fail fast with instructions.
All borrowed files retain their Apache-2.0/MIT notices and appear in a Lens
third-party provenance manifest.

Completion requires all of the following:

1. Applicable upstream VT/view regression tests are ported.
2. Deterministic Lens tests cover WS frames, auth forwarding, input/resize,
   read-only refusal, bounded backpressure, reconnect, and detached states.
3. GPUI tests cover focus, paste, selection/copy, resize coalescing, and view
   state transitions.
4. A live pinned-omnigent test or demo flow proves discover/create → attach →
   type → resize → receive output → forced transient WS drop → reattach with
   retained screen and gap marker.
5. Release-mode benchmarks cover VT byte throughput and GPUI frame timing for
   streamed output, scrolling, and resize. 120fps (8.3ms) is the target; 90fps
   (11.1ms) is a regression.

## Seams to pin and verify

- **Omnigent terminal WS:** REST is vendored in
  `vendor/omnigent-0.5.1/openapi.json`; attach framing and lifecycle must be
  validated live before and after a pin change.
- **Upstream port:** `gpui-ghostty`, Ghostty VT, GPUI, and Zig move together.
  The provenance manifest and audit identify the exact inputs and deliberately
  imported surface.
- **Terminal vs shell:** `lens-terminal` owns a tab for a target; `lens-ui`
  owns tab placement, multiple tabs, transfer routing, and the separate native
  harness toggle.
- **Reconnect:** retained local state prevents a blank surface, not missed
  server output. The marker must remain explicit until omnigent offers replay.
