# Shared Terminal Infrastructure Implementation Roadmap

> **⚠️ SUPERSEDED (2026-07-15) — VT-adoption + provenance sequencing is dead.**
> The "provenance-controlled Ghostty **port**" / "narrow attributed gpui-ghostty
> port" / "adopt/adapt/exclude every candidate file" / "full provenance manifest
> before any upstream source enters Lens" packages no longer apply. VT is now a
> **vendored `libghostty-rs` binding built from source** (pinned Ghostty dev +
> patched `zig@0.15`); provenance collapsed to ordinary dependency vetting
> (`vendor/libghostty-rs/README.md`). The `--workspace` gate line is also stale —
> the real gate is scoped to production crates (vendored crates + spikes opt out).
> Current model: memories `terminal-vt-adoption-model` +
> `zig-ghostty-macos26-scissor`, `docs/STATUS.md`, and
> `docs/handoffs/2026-07-15-terminal-vt-libghostty-rs.md`. The omnigent-transport,
> GPUI-vertical-slice, and perf/lifecycle-hardening sequencing is model-agnostic
> and will be re-expressed by a new design pass.

> **For agentic workers:** This roadmap is sequencing authority, not an executable task plan. Before implementing a work package, create and approve its dedicated plan under `docs/plans/`; that plan must require `superpowers:subagent-driven-development` or `superpowers:executing-plans`, use checkbox steps, TDD, exact paths, exact verification, and frequent commits.

**Goal:** Deliver the standalone, host-ready Lens GPUI terminal surface defined by the approved terminal workstream spec, including real omnigent attach/input/output/resize/reconnect behavior.

**Architecture:** The work proceeds through dependency-gated packages: typed omnigent transport and a provenance-controlled Ghostty port converge in a foreground-safe engine/GPUI vertical slice, followed by behavior, lifecycle, observability, memory, and performance hardening. Each package lands only after independent review and verification, and later detailed plans are written against the interfaces actually delivered by earlier packages.

**Tech Stack:** Rust 2024, gpui 0.2.2, omnigent 0.5.1 HTTP/SSE/WebSocket contract, Ghostty VT through a narrow attributed gpui-ghostty port, Zig/C/Rust FFI, Criterion, GPUI frame probes, macOS Apple Silicon.

## Global Constraints

- Performance is the prime objective: p95 frame time <= 8.3 ms; p99 <= 11.1 ms; no more than 0.1% of frames above 11.1 ms; measure input-event-to-first-paint separately.
- Never block the GPUI foreground thread; parsing, I/O, waits, reflow, and unbounded allocation stay off-thread.
- The UI never panics the process; errors are modeled values.
- Typed end-to-end: no generic WebSocket, serde_json::Value, stringly event dispatch, or Ghostty types escape their owning boundary.
- Benchmark every performance layer; logic cores ship deterministic tests.
- Workspace-wide gate: cargo clippy --workspace --all-targets -- -D warnings, plus rustfmt.
- Unsafe is denied by default; every permitted unsafe block requires a // SAFETY: justification and narrow FFI ownership.
- Every layer implements gated, typed, serializable inspection plus a fixed-capacity event ring with zero hot-path work while disabled.
- Shared terminal infrastructure is separate from the native-harness toggle.
- lens-ui production integration is out of scope; the standalone demo is the first host.
- No upstream source enters Lens before the full provenance manifest pins gpui-ghostty, Ghostty, compatible GPUI and Zig inputs, retains licenses, and classifies every candidate file adopt/adapt/exclude.
- The first accepted vertical proof must use a real pinned omnigent server and include attach, input, output, resize, forced transient WS drop, same-resource reconnect with retained engine, and a persistent possible-output-gap marker.
- Never silently drop arbitrary PTY bytes; sustained queue saturation enters visible reconnect.
- Normal builds must be reproducible and offline with respect to adopted upstream sources.

---

## How to use this roadmap

This document sequences work packages and acceptance bars. It does not replace per-package executable plans.

1. Identify the next dependency-ready package (initially WP0 and WP1 in parallel).
2. Author and approve that package’s dedicated plan under `docs/plans/` against the interfaces and crates that already exist on the branch, not against hoped-for signatures from later packages.
3. Implement that plan with TDD, independent review, and frequent commits until its acceptance gate is green.
4. Only then write the next dependent package plan against the APIs that actually landed.
5. Do not open WP2 until WP0’s provenance gate is closed. Do not open WP3 until both WP1 and WP2 have landed. Do not open WP4 until WP3’s public host seam and vertical proof are accepted. Do not open WP5 until WP4 is accepted (serialized by default). Do not open WP6’s shared `TerminalTab`/state-machine edits until WP5 is accepted; WP6 scaffolding that avoids that surface may start after WP3. Do not open WP7 until WP0–WP6 are accepted.
6. Spec authority remains `docs/specs/2026-07-14-terminal-workstream-design.md` at commit `d4c7a66`. Adjacent design docs and `vendor/omnigent-0.5.1/openapi.json` resolve contract questions; inventing wire behavior is forbidden.

---

## Dependency DAG

```text
WP0 (provenance + GPUI 0.2.2 reconcile) ────► WP2 (sync engine core + Criterion) ──┐
                                                                                   ├──► WP3 (worker/bridge/demo + guarded reconnect + frame smoke)
WP1 (typed REST+WS + live REST rider) ─────────────────────────────────────────────┘
                                                                                    │         │
                                                                                    │         ▼
                                                                                    │       WP4 (interaction + OSC policy)
                                                                                    │         │
                                                                                    │         ▼
                                                                                    │       WP5 (reset/supersession/Sleep/fleet)
                                                                                    │         │
                                                                                    │         ▼
                                                                                    │       WP6 (extend Inspect/benches/harnesses)
                                                                                    │         │
                                                                                    └─────────┴──► WP7 (full E2E + performance acceptance)
```

The diagram shows acceptance order. WP6 may start layer-local scaffolding after
WP3 as described below, but it cannot pass its final gate until WP4 and WP5 are
accepted; WP1 transport evidence is also consumed directly by WP7.

Parallelism rules:

| Rule | Detail |
| --- | --- |
| WP0 ∥ WP1 | May proceed concurrently. Neither may import upstream Ghostty/gpui-ghostty source into the tree before WP0 closes. |
| WP2 after WP0 | Hard gate. Porting begins only after the provenance manifest pins gpui-ghostty, Ghostty, Zig, and reconciles GPUI with the workspace’s published `gpui 0.2.2` (no unexamined second GPUI dependency); retains licenses; and classifies every candidate file adopt/adapt/exclude. |
| WP3 after WP1+WP2 | Vertical slice consumes typed `TerminalAttachment` from WP1 and the synchronous engine core + immutable frame/damage types from WP2. WP3 owns the dedicated worker thread, bounded channels, transport bridge, and lifecycle coordination. |
| WP4 then WP5 (serialized) | Both start only after WP3 interfaces land. Detailed plans and implementation are serialized by default: WP4 completes before WP5 begins, so they do not contend on the shared `TerminalTab` / lifecycle state-machine surface. An alternative concurrent schedule is allowed only if the Codex orchestrator records an equally explicit non-overlap gate naming disjoint files and forbidding shared enum/state-machine edits in either package until the other’s API is landed and consumed. |
| WP6 after WP3; final after WP4+WP5 | Scaffolding that does not edit the shared `TerminalTab`/state-machine surface may begin after WP3 (for example extending already-landed Criterion benches or Inspect rings in layer-local modules). Final WP6 acceptance waits until WP4 and WP5 have landed, because interaction and lifecycle workloads, memory-pressure visibility, and integrated harness validity depend on both. |
| WP7 after all | Expanded deterministic + live E2E, ported upstream tests, full performance acceptance, rustfmt/clippy, and shipping evidence. |

---

## Work packages

### WP0 — Provenance and reproducible upstream pins

**Goal:** Close the hard pre-port gate so no Ghostty/gpui-ghostty source enters Lens without pinned inputs, license retention, an adopt/adapt/exclude inventory, and an explicit reconciliation of upstream GPUI expectations against the workspace’s published `gpui 0.2.2`.

**Dependencies:** None. May run in parallel with WP1.

**Expected produced interfaces / artifacts:**

- Provenance manifest (path chosen by the WP0 plan) pinning:
  - `gpui-ghostty` at `e3025981c6211dd7db2a825dc364ffb5d342f45e`
  - Ghostty submodule at `6d2dd585a5d87fa745d48188dd096ca6e63014d0`
  - Compatible Zig toolchain / input set (must be pinned by this package; not claimed known before WP0)
  - GPUI reconciliation record: compare `gpui-ghostty`’s expected GPUI revision against the workspace’s published `gpui 0.2.2`; choose a single GPUI dependency strategy that does not introduce an unexamined second GPUI crate; document API divergence and adaptation cost for WP2
- Per-file adopt / adapt / exclude inventory for every candidate upstream file
- Retained Apache-2.0 / MIT license notices for adopted and adapted material
- Documented reproducible, offline normal-build inputs for adopted upstream sources
- Explicit exclusions matching the spec: local PTY / `portable-pty`, Kitty graphics / APC image state, unsupported Unicode inline-image cells rendering as visible garbage, Sixel, OSC 1337, and Ghostty’s large Kitty image allocation defaults

**Acceptance gate:**

- Manifest complete; every candidate file classified; licenses retained
- Zig pin recorded with exact revision/version usable by offline builds
- GPUI reconciliation written: workspace `gpui 0.2.2` is the published baseline; any adaptation required to bind the port to that baseline is enumerated with cost notes; no second GPUI dependency is added without explicit examined justification in the manifest (default is one GPUI = workspace `0.2.2`)
- Independent high-risk review (Opus 4.8 via local Claude CLI) of provenance/licensing, exclude decisions, and GPUI reconciliation
- No upstream VT/render source merged into Lens crates until this gate is green

**Delegated author model:** grok-4.5-xhigh authors the executable WP0 plan; composer-2.5 performs inventory/mechanical manifest work; grok-4.5-xhigh owns pin/license/GPUI-reconciliation decisions.

**Independent reviewer:** Codex orchestrator reviews plan and change; Opus 4.8 via local Claude CLI reviews provenance/licensing and GPUI reconciliation as a high-risk gate. Producing model never reviews its own output.

**Consumed by:** WP2 (port may begin only after WP0); WP7 (shipping evidence includes provenance reproducibility and GPUI reconciliation).

---

### WP1 — Typed lens-client terminal REST + WebSocket

**Goal:** Give Lens a typed omnigent terminal protocol module: REST list/get/create/delete, authenticated WS attach with `transport=pty`, close-code classification, bounded queues/backpressure with deterministic saturation-to-disconnect, foundational typed inspection with a proven disabled path, and a live pinned-omnigent REST-shape rider — with the contradictory public transfer wrapper removed or made inaccessible.

**Dependencies:** None. May run in parallel with WP0. Must not pull Ghostty source.

**Expected produced interfaces / artifacts:**

- Typed terminal resource/request/response values grounded in `vendor/omnigent-0.5.1/openapi.json` (`SessionResourceObject` and related schemas) and WS facts audited at omnigent revision `08285468`
- Typed WS attach path: `/v1/sessions/{session_id}/resources/terminals/{terminal_id}/attach`
- Binary PTY input/output frames; text resize frame `{"type":"resize","cols":N,"rows":M}`
- Explicit `transport=pty` query (never silent default to `control`)
- Auth/access query modeling: interactive attach requires owner level; `read_only=true` requires read access, drops binary input, retains resize
- Close-code classification: `1008` authorization, `4404` missing/dead terminal, `4405` tmux client detached while terminal alive, `4500` internal attach failure, plus generic network closure without replay/sequence proof
- Pre-attach GET and bounded reader/writer queues; sustained saturation disconnects into visible reconnect rather than dropping arbitrary PTY bytes; saturation-to-disconnect proven with deterministic tests (controlled queue fill), not a flaky live race
- Off-foreground connection and reconnect work at the transport layer
- Foundational typed, serializable `Inspect` surface for transport/queue/reconnect state with fixed-capacity diagnostic ring; disabled-path proof that this layer performs no snapshot construction, event recording, allocation, or synchronization while Inspect is off
- Client deterministic tests and Criterion benches for WS frame classification/control codec and bounded-queue throughput
- Live pinned-omnigent contract rider exercising list/get/create/delete resource shapes against a real 0.5.1 server so REST-shape drift is caught before WP3/WP7
- Removal or inaccessibility of `Sessions::transfer_terminal` (and any public transfer wrapper) because transfer is absent from the public 0.5.1 contract

**Acceptance gate:**

- No generic WebSocket, `serde_json::Value`, or stringly event dispatch at the public client boundary
- Transfer capability not callable from public Lens API surface
- Deterministic tests cover list/get/create/delete shapes, auth/access queries, `transport=pty`, resize codec, close codes, and saturation-to-disconnect backpressure
- Live pinned-omnigent rider green for list/get/create/delete resource shapes
- Criterion benches exist for classification/codec and queue throughput
- Disabled Inspect path proven for this layer (no snapshot/event/alloc/sync while off)
- Opus 4.8 via local Claude CLI mandatory independent review of typed boundary and backpressure/disconnect semantics
- `cargo fmt` and workspace clippy gate clean for touched crates; package plan’s stated test commands green

**Delegated author model:** grok-4.5-xhigh authors the plan; composer-2.5 implements most typed wrappers/tests; grok-4.5-xhigh owns or pairs on close-code classification, backpressure, and typed-boundary design.

**Independent reviewer:** Codex orchestrator plus Opus 4.8 via local Claude CLI for typed boundary and backpressure/disconnect semantics. Producing model never reviews its own output.

**Consumed by:** WP3 (`TerminalAttachment` / typed client; consumes saturation-to-disconnect rule without re-proving it via live saturation induction), WP6 (extends client-layer benches/Inspect), WP7 (transport E2E).

---

### WP2 — Narrow Ghostty port and synchronous engine core

**Goal:** Land the attributed, narrow Ghostty VT engine core and immutable frame/damage types inside a future `lens-terminal` crate boundary, after WP0, excluding local PTY and inline graphics, under a strict FFI/unsafe gate, with Criterion benches shipping alongside the engine core.

**Dependencies:** WP0 complete.

**Expected produced interfaces / artifacts:**

- Narrow port of adopt/adapt files only, with attribution and license notices, bound to the WP0-reconciled single GPUI baseline (`gpui 0.2.2` unless WP0 recorded an examined exception)
- Synchronous engine core (parse, scrollback, damage, reflow) that can own non-`Send` Ghostty state on a single thread when later hosted by WP3; WP2 itself does not ship the dedicated worker thread, transport bridge, or lifecycle coordinator
- Immutable frame/damage update types that never leak Ghostty types across the owning boundary
- Strict-bounded consumption of unsupported APC/DCS payloads without per-byte warnings; unsupported Unicode inline-image cells render blank
- FFI surface limited to justified `unsafe` blocks with `// SAFETY:` notes and narrow ownership
- No local PTY / `portable-pty`; no Kitty graphics bridge; no Sixel / OSC 1337 support
- Foundation tests proving parse→damage→frame construction without omnigent (synthetic byte feed)
- Criterion benchmarks shipping with the engine core for VT parsing, damage/frame construction, scrolling, and reflow
- Engine-layer typed `Inspect` surface with fixed-capacity diagnostic ring; disabled-path proof that this layer performs no snapshot construction, event recording, allocation, or synchronization while Inspect is off

**Acceptance gate:**

- Opus 4.8 via local Claude CLI high-risk review of Ghostty/FFI ownership, unsafe inventory, and exclude compliance
- Ghostty types do not escape their owning boundary
- Criterion benches present and runnable for VT parsing, damage/frame construction, scrolling, and reflow
- Disabled Inspect path proven for this layer
- Offline reproducible build using WP0 pins and GPUI reconciliation
- Workspace clippy/rustfmt clean for the new crate surface; package tests green

**Delegated author model:** grok-4.5-xhigh authors the plan and owns or pairs on FFI and render-boundary work; composer-2.5 performs bounded/mechanical porting of adopt-classified files and Criterion scaffolding.

**Independent reviewer:** Codex orchestrator plus Opus 4.8 via local Claude CLI for Ghostty/FFI. Producing model never reviews its own output.

**Consumed by:** WP3 (hosts the sync engine on a dedicated worker; GPUI render of immutable frames), WP4 (selection/mouse/titles via engine), WP6 (extends engine benches/Inspect).

---

### WP3 — Engine worker, transport bridge, host seam, demo, first vertical proof

**Goal:** Converge WP1 transport and WP2 engine into a foreground-safe vertical slice: dedicated engine worker with create/teardown/recreate controls, transport bridge, public `TerminalTab` host seam, standalone GPUI demo, guarded same-generation reconnect, initial GPUI frame-timing smoke gate, and the first real pinned-omnigent proof including forced drop/reconnect/retained engine/gap marker.

**Dependencies:** WP1 and WP2.

**Expected produced interfaces / artifacts (names locked by the spec):**

- Public identity values:

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

- `TerminalKey` contains `terminal_name` and `session_key`
- Constructor:

```rust
pub fn open(
    target: TerminalTarget,
    client: Arc<Client>,
    options: TerminalOpenOptions,
    cx: &mut App,
) -> Entity<TerminalTab>;
```

- `TerminalOpenOptions` contains only access intent, a scrollback limit, and initial user preferences
- `TerminalTab::focus_handle(cx)` and `TerminalTab::presentation()` for latest atomic title/lifecycle/access/progress
- Typed inbound `TerminalHostEvent` seam and typed outbound `TerminalEvent` stream (presentation changes and host requests; no arbitrary `RequestClose`; no client transfer request)
- Lifecycle values present at least: `Starting`, `Live`, `Reconnecting`, plus real `Detached` paths required by the vertical proof and the persistent possible-output-gap marker after same-resource reconnect. Full heuristics for `Ended`, `Sleeping`, and `ReplacementWaiting` are owned by WP5; WP3 must still expose them as modeled lifecycle values on `presentation()` so later packages do not reshape the public enum surface
- Dedicated engine worker thread hosting WP2’s synchronous engine core; bounded channels; transport bridge to `TerminalAttachment`; lifecycle coordination on the worker side
- Forward-shaped engine-worker lifecycle controls for create, teardown, and recreate (WP3 exercises create/run/same-resource reconnect with retained engine; WP5 Sleep/replacement consumes teardown/recreate without redesigning the worker seam)
- Basic pre-reconnect guard in WP3: GET exact terminal resource before reconnect; consult generation signal (duplicate `resource.created` for the attached ID ⇒ new generation / do not treat as same-resource retain); `404` stops reconnect. This is the mechanism the vertical proof certifies. WP5 later adds reset/supersession/replacement and missed-event heuristics without reshaping this reconnect path
- GPUI tab renders coalesced immutable frame/damage updates only
- Standalone GPUI demo as first host (not production `lens-ui` integration)
- Provisional scrollback limit default: Ghostty app default 10 MB (10,000,000 bytes), allocated lazily, oldest-first eviction, visible grid preserved
- Tab/render-layer typed `Inspect` surface with fixed-capacity diagnostic ring; disabled-path proof that this layer performs no snapshot construction, event recording, allocation, or synchronization while Inspect is off
- Initial GPUI frame-timing smoke gate measuring against p95 <= 8.3 ms and p99 <= 11.1 ms on a minimal vertical-slice workload; input-event-to-first-paint and full multi-workload acceptance remain WP7 responsibilities

**Acceptance gate (non-weakened live bar):**

- Real pinned omnigent 0.5.1 server
- Attach, keyboard/input bytes, output render, resize (newest size sent before input re-enabled after reconnect)
- Forced transient WebSocket drop
- Same-resource reconnect with retained engine, using the WP3 pre-reconnect GET + generation-signal guard
- Persistent possible-output-gap marker after reconnect
- Consumes WP1’s saturation-to-disconnect rule at the bridge; live proof does not require flaky live saturation induction
- Constructor returns immediately in `Starting`; discovery/create/attach off-thread; failures are lifecycle values
- Engine-worker create/teardown/recreate controls exist and are covered by deterministic tests even where the live proof only drives create/run/reconnect
- Initial GPUI frame-timing smoke gate records p95/p99 against 8.3 ms / 11.1 ms budgets
- Disabled Inspect path proven for this layer
- Independent high-risk review (Opus 4.8 via local Claude CLI) of lifecycle/reconnect/guarded same-generation behavior
- Package tests + stated live proof commands green; clippy/rustfmt clean for touched crates

**Delegated author model:** grok-4.5-xhigh authors the plan and owns/pairs on state-machine, threading, worker lifecycle, and typed host seam; composer-2.5 implements demo wiring and bounded bridge glue under that design.

**Independent reviewer:** Codex orchestrator; Opus 4.8 via local Claude CLI on reconnect/retained-engine/gap-marker and the pre-reconnect GET/generation guard. Producing model never reviews its own output.

**Consumed by:** WP4, WP5 (Sleep/replacement consume create/teardown/recreate), WP6 (extends Inspect/benches/harnesses), WP7.

---

### WP4 — Read-only access, keyboard/IME/paste/selection/copy/mouse, titles/hyperlinks/OSC policy

**Goal:** Complete interactive and read-only terminal behavior and typed host-request policy on the Live surface delivered by WP3.

**Dependencies:** WP3 interfaces landed and accepted. Serialized ahead of WP5 by default (see Parallelism rules).

**Expected produced interfaces / artifacts:**

- Owner-write vs viewer-read-only: read-only can scroll/select locally; no remote keyboard, paste, or mouse input
- Keyboard, IME, paste (`Cmd+V` local gesture; bracketed paste preserved; multiline paste warning when bracketed mode inactive, with global disable / “Don’t warn again”); read-only tabs expose no paste
- Selection and copy
- Mouse modes following Ghostty/XTSHIFTESCAPE: Shift normally enables local selection; TUI may capture Shift; runtime toggle can force mouse local
- Live resize: coalesce GPUI geometry, reflow off-thread, send only newest `{cols, rows}`; during reconnect retained engine tracks geometry and newest size is sent before input re-enabled
- Stable `identity_title` = `terminal_name:session_key`; sanitized bounded OSC 0/2 → optional `reported_title`; reported text never identity/routing/authorization/OS window title; survives same-resource reconnect; clears on replacement
- OSC 52 writes: strict decoded payload cap + host permission (allow-once or allow-for-session) + copy notice; OSC 52 reads denied
- Validated plain URLs and OSC 8 links actionable only after user gesture → typed host requests; terminal output never opens a browser
- OSC progress updates terminal-local presentation; notification sequences become rate-limited host requests only while unfocused/backgrounded; suppressed for read-only observers by default
- Typed host-request ID/response round-trip for permissioned requests

**Acceptance gate:**

- Deterministic tests for access gating, paste policy, OSC 52 deny-read/permissioned-write, hyperlink gesture→host request, title sanitization bounds
- GPUI focus/input/render tests for keyboard/IME/selection/mouse paths
- No browser launch from terminal output; no Ghostty types at host boundary
- Opus 4.8 via local Claude CLI mandatory independent review of OSC 52/clipboard, URL gesture, and paste security policy
- Clippy/rustfmt + package verification green

**Delegated author model:** grok-4.5-xhigh authors the plan; composer-2.5 implements most input/selection UI wiring; grok-4.5-xhigh owns or pairs on OSC security policy and typed host-request protocol.

**Independent reviewer:** Codex orchestrator plus Opus 4.8 via local Claude CLI for OSC 52/clipboard, URL gesture, and paste security policy. Producing model never reviews its own output.

**Consumed by:** WP5 (must not reopen WP4’s interaction surface except via landed APIs), WP6 (interaction workloads), WP7 (behavior E2E).

---

### WP5 — Replacement, reset, supersession, Sleep/wake, detach/ended, hidden tabs, fleet pressure

**Goal:** Complete identity/replacement and extended lifecycle semantics on top of WP3’s guarded reconnect path and engine-worker create/teardown/recreate controls: reset/supersession/replacement, deliberate Sleep/wake resource reclaim, hidden-tab behavior, and fleet memory-pressure hooks — without reshaping the WP3 pre-reconnect GET/generation-signal reconnect mechanism.

**Dependencies:** WP4 accepted (serialized default). Consumes WP3’s reconnect guard and worker lifecycle controls as landed APIs.

**Expected produced interfaces / artifacts:**

- Full lifecycle set: `Starting`, `Live`, `Reconnecting`, `ReplacementWaiting`, `Sleeping`, `Ended`, `Detached`, with effective read-only/write modeled separately
- `Existing` never adopts a different resource or relaunches a process
- `OpenOrCreate` discovers/creates exact logical key only during initial opening; not a perpetual keep-alive
- Manual deletion / unexplained disappearance / same-ID recreation outside positively identified reset → freeze final frame → `Detached`; recreation is explicit user action
- Positively identified agent reset: `OpenOrCreate` may wait for and adopt exact-key successor using WP3’s recreate control for a fresh Ghostty engine (no mixed history); `Existing` stays `Detached`
- `session.superseded`: both targets may follow the same `TerminalId` into the target session with retained engine; surrounding session redirect remains host-owned (demo may inject the typed `TerminalHostEvent`)
- Missed-event / best-effort persistence heuristics layered on WP3’s reconnect guard without changing that guard’s GET + generation-signal contract; document the narrow missed-event race as the known 0.5.1 contract gap (no immutable generation token)
- Generic transport failure: retry 30s with bounded exponential backoff; frozen read-only retained frame; successful same-resource reconnect always adds persistent possible-output-gap marker (path already proven in WP3; WP5 extends surrounding lifecycle cases)
- `4404` / GET `404` / deletion / exhausted retry → `Detached`; `4405` → `Detached` with “terminal still running; client detached” + explicit reattach; do not fight intentional tmux detach loops
- `1008` write rejection disables input immediately; refresh access; may reattach read-only; loss of read access → authorization `Detached`
- `Ended` only for positively reported process termination (may show exit code); ambiguous disappearance is `Detached`, never guessed `Ended`
- Normal exit never auto-closes a tab; `OpenOrCreate` may offer relaunch only after positive `Ended`; otherwise “Create terminal again”
- Sleep ≠ reconnect: close WS, release Ghostty engine and full scrollback via WP3 teardown; retain immutable final viewport labeled `Session sleeping`; wake reattaches only if same observed resource generation survived (using WP3 guard + recreate as needed); else `Detached`; Sleep adds no reconnect-gap marker and never auto-creates
- Hidden/minimized open terminal stays attached and keeps parsing; suppresses GPUI frame publication; becoming visible publishes one coalesced latest frame
- Fleet memory-pressure hooks: track retained bytes fleet-wide; under macOS memory warning trim least-recently-viewed hidden histories first with visible truncation marker; under critical pressure deliberately disconnect least-recently-viewed hidden tabs, retain final viewport, expose explicit reattach; never silently drop PTY bytes; keep active tab connected; trim active old history only as last resort
- Scrollback memory-only; released on tab close, deliberate Sleep, or Lens exit; never silently persisted to disk
- `Ended`/`Detached`/`Sleeping` preserve original final grid; container resize changes only clipping/padding; replacement engines start at current geometry

**Acceptance gate:**

- Deterministic tests for reset/supersession/replacement, Sleep/wake reclaim via teardown/recreate, `4404`/`4405`/`1008` paths, hidden-tab frame suppression, memory-pressure trim/disconnect ordering, and proof that WP3’s pre-reconnect GET/generation-signal path is unchanged
- Opus 4.8 via local Claude CLI high-risk review of lifecycle/replacement/Sleep and missed-event heuristics
- No silent PTY byte drops; no auto-create on Sleep wake; no guessed `Ended`
- Clippy/rustfmt + package verification green

**Delegated author model:** grok-4.5-xhigh authors the plan and owns/pairs on state-machine and generation/missed-event heuristics; composer-2.5 implements bounded mechanical lifecycle wiring and tests.

**Independent reviewer:** Codex orchestrator plus Opus 4.8 via local Claude CLI for lifecycle/replacement/Sleep. Producing model never reviews its own output.

**Consumed by:** WP6 (lifecycle/memory workloads; final acceptance), WP7 (lifecycle live E2E).

---

### WP6 — Extend Inspect, diagnostic rings, memory accounting, and layer benchmarks

**Goal:** Integrate and extend the Inspect surfaces, Criterion benches, and GPUI frame harness already introduced by WP1/WP2/WP3: complete remaining ring/accounting coverage, validate harnesses end-to-end, and prepare release-metadata collection — without postponing disabled-path proofs or introducing late first-time layer benches.

**Dependencies:** WP3 landed for scaffolding that avoids shared `TerminalTab`/state-machine edits; final acceptance after WP4 and WP5.

**Expected produced interfaces / artifacts:**

- Integrated typed serializable `Inspect` across transport, engine, and render/tab layers: transport/queue/reconnect state; engine dimensions/history/damage/lifecycle; render visibility/cache/frame statistics — extending the per-layer surfaces and disabled-path proofs already required in WP1/WP2/WP3
- Fixed-capacity diagnostic rings recording typed state transitions when locally enabled; local/permission-gated access
- Inspection distinct from `TerminalEvent`; reaffirm zero snapshot construction, event recording, allocation, or synchronization on hot paths while disabled (integration proof, not the first introduction of the rule)
- Memory accounting APIs/hooks exposing per-terminal and fleet retained bytes; truncation markers observable via Inspect/presentation as delivered by WP5
- Criterion and harness extension (not late introduction): extend WP1 client classification/codec/queue benches; extend WP2 engine VT/damage/scroll/reflow benches; extend WP3 GPUI frame-timing smoke into the full workload harness shell WP7 will accept against
- Harness metadata recording hardware, macOS, commit, and build info for release runs

**Acceptance gate:**

- Every layer’s Inspect + ring is integrated; disabled-path proofs from WP1/WP2/WP3 remain green under the integrated suite
- Extended Criterion + GPUI harnesses cover all required layers and are ready for WP7 acceptance workloads
- Opus 4.8 via local Claude CLI mandatory independent review of benchmark/frame-harness validity
- Clippy/rustfmt + package verification green

**Delegated author model:** grok-4.5-xhigh authors the plan and owns/pairs on performance-critical harness validity; composer-2.5 implements mechanical Inspect/ring integration and bench extension.

**Independent reviewer:** Codex orchestrator plus Opus 4.8 via local Claude CLI for benchmark/frame-harness validity. Producing model never reviews its own output.

**Consumed by:** WP7 (performance acceptance and shipping evidence).

---

### WP7 — Expanded E2E, upstream tests, performance acceptance, shipping evidence

**Goal:** Prove the workstream complete against the approved spec: expanded deterministic and live E2E, ported applicable upstream tests, full performance acceptance on Apple Silicon, rustfmt/clippy, and shipping evidence.

**Dependencies:** WP0–WP6 accepted.

**Expected produced interfaces / artifacts:**

- Expanded deterministic typed transport/lifecycle tests and GPUI focus/input/render tests covering WP1–WP5 behaviors
- Ported applicable upstream Ghostty/gpui-ghostty tests consistent with adopt/adapt inventory
- Live external pinned-omnigent discover/create/type/resize/drop/reconnect flow (extends WP3 proof; remains a shipping sentinel)
- Release-mode performance acceptance on available Apple Silicon machine with recorded hardware, macOS, commit, and build metadata:
  - p95 frame time <= 8.3 ms
  - p99 frame time <= 11.1 ms
  - no more than 0.1% of frames above 11.1 ms
  - input-event-to-first-paint measured separately
- Required workloads: rapid typing with echo; sustained/bursty styled output; scrolling a full 10 MB history; continuous resize/reflow; hidden-to-visible catch-up; one visible terminal with several hidden terminals streaming
- Throughput, latency, and resident memory recorded; 10 MB default and fleet soft budget treated as provisional until these measurements include real RSS
- Workspace-wide `cargo clippy --workspace --all-targets -- -D warnings` and rustfmt clean
- Shipping evidence package: provenance reproducibility and GPUI `0.2.2` reconciliation confirmation, live E2E log/metadata, benchmark tables, and explicit statement that native-harness toggle and production `lens-ui` integration remain out of scope

**Acceptance gate:**

- Opus 4.8 via local Claude CLI high-risk review of final performance and live-E2E evidence
- All numeric frame budgets met or the package fails closed (no silent weakening)
- Provenance offline build still reproducible under the WP0 GPUI reconciliation
- Codex orchestrator integrates and may reject/revise delegate output until green

**Delegated author model:** grok-4.5-xhigh authors the plan and owns/pairs on performance-critical analysis; composer-2.5 runs mechanical test porting and evidence collection scripts defined by the plan.

**Independent reviewer:** Codex orchestrator plus Opus 4.8 via local Claude CLI for final performance/live-E2E. Producing model never reviews its own output.

**Consumed by:** Workstream completion; future hosts (including later `lens-ui` integration, which is a separate workstream).

---

## Planning ownership

| Role | Responsibility |
| --- | --- |
| grok-4.5-xhigh | Authors each executable work-package plan under `docs/plans/`. Owns or pairs on state-machine, FFI, threading, typed-boundary, and performance-critical implementation. |
| composer-2.5 | Repo mapping and most bounded/mechanical implementation under approved package plans. |
| Codex orchestrator | Reviews every plan and change, runs gates, integrates, and may reject or revise delegate output. |
| Opus 4.8 via local Claude CLI | Independent high-risk review. Mandatory callouts: this roadmap; provenance/GPUI reconciliation (WP0); Ghostty/FFI (WP2); typed boundary and backpressure/disconnect (WP1); lifecycle/reconnect/guarded same-generation (WP3); OSC 52/clipboard, URL gesture, and paste security (WP4); lifecycle/replacement/Sleep (WP5); benchmark/frame-harness validity (WP6); final performance/live-E2E (WP7). |
| Review diversity | A producing model never reviews its own output. Every non-trivial change receives review from a model family other than the author. |

---

## Detailed plan contract

Every executable package plan derived from this roadmap must:

1. Live under `docs/plans/` with a clear WP id in the title and a header requiring `superpowers:subagent-driven-development` or `superpowers:executing-plans`.
2. Restate the package goal, dependencies, and acceptance gate from this roadmap without weakening them.
3. Specify exact files to create/modify/test and the interfaces consumed/produced — using signatures locked by the spec or actually landed by prior packages, never invented speculative APIs.
4. Use checkbox steps (`- [ ]`) sized for red-green-refactor cycles.
5. For each logic change: write a failing test, run it and record the expected failure, implement the minimum code, run it and record the expected pass, then commit.
6. Give exact verification commands and expected failures/passes; include the workspace clippy and rustfmt gates before package completion.
7. Require frequent commits with focused messages.
8. Contain no unfinished markers, deferred notes, copy-from-elsewhere hedges, or vague “handle errors” language; model every error path as a concrete typed outcome or test.
9. Include a self-review checklist mapping the package’s spec obligations to steps.
10. Name the delegated author model and independent reviewer, matching Planning ownership above, including any Opus 4.8 via local Claude CLI mandatory callout for that package.
11. For WP3 and WP7 live proofs: name the pinned omnigent install/run method, the forced-drop technique, and the exact observations required (pre-reconnect GET + generation-signal guard, retained engine, gap marker, resize-before-input-reenable). WP3 must not require live queue-saturation induction; WP1 owns that deterministic proof.
12. For WP1: name the live pinned-omnigent REST rider commands for list/get/create/delete. For WP1/WP2/WP3: name the disabled-Inspect proof method for that layer. For WP2: name the Criterion bench targets. For WP3: name the frame-timing smoke gate commands and thresholds.

---

## Risk register

| Risk | Why it matters | Mitigation in this roadmap |
| --- | --- | --- |
| Provenance / licensing | Upstream source without pins or notices blocks legal and reproducible adoption. | WP0 hard gate; Opus review; offline build evidence in WP7. |
| Undocumented WS behavior | OpenAPI omits attach details; remembered behavior can drift. | Ground WS facts in audited revision `08285468` + pinned OpenAPI; WP1 live REST rider; live sentinel in WP3/WP7; no invented wire semantics. |
| No immutable generation token | Same-ID recreation can race best-effort resource-event persistence. | WP3 ships GET + generation-signal reconnect guard and certifies it in the vertical proof; WP5 adds missed-event heuristics without reshaping that path; never invents a client-side generation id pretending to close the gap. |
| Non-`Send` Ghostty ownership | Emulator state cannot cross threads freely; foreground misuse freezes or corrupts UI. | WP2 sync engine core keeps Ghostty types behind owning boundary; WP3 dedicated worker owns create/teardown/recreate; Opus FFI review. |
| GPUI revision coupling | gpui-ghostty may expect a GPUI revision other than workspace `0.2.2`. | WP0 reconciles against published `gpui 0.2.2`, records divergence/adaptation cost, and forbids an unexamined second GPUI dependency; WP2 builds against that reconciliation. |
| Backpressure | Dropping PTY bytes silently corrupts the session view. | WP1 deterministic saturation-to-disconnect; WP3 consumes the rule without flaky live saturation induction; WP7 re-verifies transport behavior. |
| Security-sensitive OSC / input | OSC 52 and hyperlinks can exfiltrate or open unexpected surfaces. | WP4: deny OSC 52 reads; permissioned writes with caps; gesture-only URL/OSC 8 host requests; Opus 4.8 mandatory review of OSC/clipboard/URL/paste policy. |
| Shared `TerminalTab` ownership | Concurrent WP4/WP5 edits can corrupt lifecycle/interaction seams. | WP4 then WP5 serialized by default; concurrent alternative only with an explicit non-overlap gate. |
| Memory / RSS | 10 MB default and fleet soft budget are provisional until measured. | WP5 pressure hooks; WP6 accounting; WP7 records real RSS before treating budgets as final. |
| Performance | Frame budget is the prime objective; regressions are merge-blocking. | WP1 client benches; WP2 engine Criterion; WP3 frame-timing smoke; WP6 extends/validates harnesses (Opus review); WP7 full numeric acceptance. |
| Late observability / benches | Deferring Inspect proofs or layer benches to WP6 hides regressions. | WP1/WP2/WP3 each introduce Inspect with disabled-path proof; WP2 ships engine Criterion; WP3 ships frame smoke; WP6 integrates/extends only. |

---

## Completion matrix

| Spec section | Work packages |
| --- | --- |
| Goal — standalone renderable GPUI terminal tab, typed through presentation, host-ready without `lens-ui` learning REST/WS/Ghostty | WP3, WP4, WP5, WP7 |
| Scope — typed client; deep `lens-terminal`; owner/viewer behavior; reconnect+gap marker; reproducible inputs; out-of-scope native-harness toggle, Bash-tool stream, `lens-ui` integration, local PTY, inline graphics, public transfer | WP0–WP7 (exclusions enforced in WP0/WP1/WP2; out-of-scope items never scheduled) |
| Grounded adoption result — gpui-ghostty/Ghostty pins, GPUI/Zig pins, licenses, adopt/adapt/exclude, graphics exclusions, GPUI `0.2.2` reconciliation | WP0, WP2 |
| Pinned omnigent 0.5.1 facts — REST shapes, WS attach, `transport=pty`, auth, close codes, reset/supersession, generation limits | WP1 (typed + live REST rider), WP3 (guarded reconnect proof), WP5 (reset/supersession/missed-event), WP7 |
| Module ownership — `lens-client` protocol; `lens-terminal` deep module + `Inspect`; `lens-ui` host adapter out of production scope | WP1, WP2 (sync engine), WP3 (worker/bridge/demo host), WP6 (Inspect/bench integration; production `lens-ui` not in this roadmap) |
| Identity and replacement semantics | WP3 (attach targets + basic generation-signal reconnect guard), WP5 (reset/supersession/replacement/missed-event) |
| Lifecycle — states, reconnect, close codes, `Ended` vs `Detached`, Sleep/wake | WP3 (reconnect+gap proof, worker create/teardown/recreate, guarded GET), WP5 (full set including Sleep and replacement) |
| Terminal behavior and policy — read-only, keyboard/IME/paste, mouse, OSC, titles, hyperlinks | WP4 |
| Scrollback, memory, resize, and rendering | WP2 (sync engine + frame/damage types + Criterion), WP3 (worker/bridge/resize in vertical slice + frame smoke), WP4 (live resize/input re-enable), WP5 (fleet pressure, Sleep reclaim, frozen grids), WP6 (accounting + harness extension) |
| Performance and verification | WP1 (client benches + disabled Inspect proof), WP2 (engine Criterion + disabled Inspect proof), WP3 (frame smoke + disabled Inspect proof), WP6 (extend/validate harnesses), WP7 (full acceptance + shipping evidence) |

---

## Roadmap self-review

- [x] Every major section of `docs/specs/2026-07-14-terminal-workstream-design.md` maps to one or more of WP0–WP7 in the completion matrix.
- [x] No spec requirement is silently deferred outside WP0–WP7; explicitly out-of-scope items (native-harness toggle, production `lens-ui` integration, local PTY, inline graphics, public transfer, Bash-tool incremental stream) are named as exclusions, not postponed packages.
- [x] Provenance gate is hard and precedes any upstream port (WP0 → WP2); WP0 reconciles GPUI against workspace `gpui 0.2.2` and forbids an unexamined second GPUI dependency.
- [x] Live interaction acceptance bar is not weakened: WP3 and WP7 require real pinned omnigent, attach/input/output/resize, forced drop, retained-engine reconnect via GET + generation-signal guard, and persistent possible-output-gap marker.
- [x] Backpressure rule preserved: WP1 proves saturation-to-disconnect deterministically; WP3 consumes it without flaky live saturation induction; never silently drop arbitrary PTY bytes.
- [x] WP2 is synchronous engine core + immutable frame/damage + Criterion; WP3 owns worker thread, bounded channels, transport bridge, lifecycle coordination, create/teardown/recreate, and frame-timing smoke.
- [x] WP1/WP2/WP3 each prove disabled Inspect paths at introduction; WP6 integrates/extends rather than postponing proofs or introducing layer benches late.
- [x] WP4 then WP5 are serialized by default (or an explicit non-overlap gate); WP6 scaffolding may overlap only off the shared `TerminalTab`/state-machine surface; WP6 final acceptance waits for WP4 and WP5.
- [x] Locked public interfaces (`TerminalTarget`, `AccessIntent`, `open`, `TerminalTab` focus/presentation, `TerminalHostEvent`, `TerminalEvent`) appear only as already specified; no invented signatures beyond the spec.
- [x] Zig pin is not claimed known before WP0; GPUI baseline is workspace `0.2.2` pending WP0 reconciliation record.
- [x] Parallelism rules state WP0∥WP1, WP2←WP0, WP3←WP1+WP2, WP4→WP5 serialized, WP6 scaffold after WP3 / final after WP4+WP5, WP7←all.
- [x] Planning ownership names Opus 4.8 via local Claude CLI mandatory callouts for roadmap, WP0, WP1, WP2, WP3, WP4, WP5, WP6, and WP7 high-risk seams (no vague “cross-family” wording at those seams).
- [x] Detailed plan contract forces TDD checkbox plans, exact verification, frequent commits, and no unfinished or deferred planning language.
- [x] Risk register covers provenance/licensing, undocumented WS behavior, missing generation token, non-`Send` Ghostty ownership, GPUI coupling, backpressure, OSC/input security, shared `TerminalTab` ownership, memory/RSS, performance, and late observability/benches.
- [x] This file remains sequencing authority, not an executable implementation plan or task-level code dump.
