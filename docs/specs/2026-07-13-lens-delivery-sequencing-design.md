# Lens delivery sequencing design

**Status:** Approved for planning on 2026-07-13.

## Purpose

Order the remaining Lens work as a ship-quality native macOS client, not an MVP.
This is a sequencing and ownership design. It does not authorize a single
monolithic implementation plan: every numbered package below needs its own
specification and plan before code changes.

Lens remains a typed, multi-connection Rust client of omnigent. The existing
`lens-client` → `lens-core` → `lens-store` path remains the only transport,
canonical-state, persistence, and foreground-replica path. No new UI surface
may bypass that path or add a second reducer/persistence mirror.

## Decisions

### Delivery posture

- Ship direct to **Apple Silicon** Macs using Developer ID signing and
  notarization. The initial release has one stable update channel.
- Lens updates its supervised omnigent runtime as an **atomic coupled update**:
  provision the release-pinned runtime in a versioned slot, health-check it,
  then activate it atomically. Retain the prior slot for rollback.
- Before a destructive local-data migration, create and verify a backup. Keep
  one generation of application/runtime compatibility so the preceding Lens
  release can open the prior schema after rollback.
- Diagnostics are local-only by default. Lens has no automatic telemetry or
  crash upload. It supports a user-initiated, redacted diagnostics export.
- Remote non-loopback connections require HTTPS and system trust. Lens does
  not persist certificate exceptions. Loopback HTTP remains allowed.
- Lens detects third-party harness prerequisites and explains/remediates their
  setup, but never installs vendor CLIs or manages vendor credentials.

### Native-TUI interaction surface

Native-harness TUI support is a permanent product capability, not a temporary
workaround. For an eligible session (native harness, live terminal resource,
and permitted write attach), Lens stores a per-session presentation choice:

```rust
enum InteractionSurface {
    Rendered,
    NativeTui { terminal_id: TerminalId },
}
```

The choice is local presentation state. Switching it neither restarts the
harness nor reopens/replays the stream nor mutates the server session.

- **Rendered** exposes Lens's transcript and composer. A submit takes the
  established typed command path. Omnigent forwards that input to the active
  native harness where applicable.
- **Native TUI** exposes the attached vendor PTY as the full focused-chat
  column. The actor remains subscribed, reduces, and persists the same server
  stream in the background.
- Either surface may submit successive turns. The selected surface owns
  foreground keyboard focus; Lens does not expose two enabled composers in one
  focused view.
- Unsubmitted input is deliberately surface-local. Lens preserves its own
  composer draft in the Lens draft store; the native TUI preserves its
  unsubmitted terminal buffer in its still-live PTY/tmux session. Lens never
  tries to parse or migrate arbitrary terminal bytes into the composer.
- SDK-only harnesses are rendered-only. Losing the terminal resource falls
  back visibly to Rendered; it does not invent or replay terminal input.

Developer-tools builds add a read-only rendered transcript shadow while Native
TUI is selected. It is compiled behind a developer-tools feature and absent
from signed production builds. Its purpose is continuous dogfood of row
identity, projection, persistence, reconnect, and live-tail behavior against
real TUI-driven sessions. It has no composer, controls, or authority.

### Remote and managed hosts

The first release connects to already-running local and remote omnigent
servers, and supports server-selected managed sandboxes. Remote host SSH
bootstrap/installation and user-visible managed-sandbox provider selection are
intentional post-launch deferrals. The omnigent 0.5.1 Electron client follows
the same split: it connects to a supplied server URL, never SSH-bootstraps a
remote host, and offers one server-advertised managed-sandbox option rather
than a provider picker.

## Rolling contract baseline

Omnigent releases regularly. Lens must advance deliberately without copying a
version string into every design document.

1. Maintain one canonical **contract baseline** with exact omnigent release,
   source revision, vendored OpenAPI checksum, capture date, and compatibility
   status.
2. Maintain a **consumer-coverage ledger** mapping each Lens module/surface to
   its consumed REST paths, schemas, SSE discriminators, WebSocket protocol,
   live/golden captures, and status (`modeled`, `deferred`, or
   `schema-derived`).
3. Design documents cite stable path/schema/event anchors and the current
   baseline/ledger; they do not repeat a literal pin in every header.
4. Each upstream release runs `xtask drift` and produces a focused impact
   packet. An unconsumed change only updates the baseline. A compatible
   consumed addition updates its ledger and targeted evidence. A semantic or
   breaking consumed change updates the owning specification, implementation,
   and evidence before Lens adopts the new pin. Wire behavior outside OpenAPI
   requires a golden or live capture.

Lens does not track upstream HEAD silently. It evaluates each release promptly
and advances only through this focused compatibility gate.

## Ordered work packages

### 0. Decision and contract baseline

1. Reconcile stale 0.2/0.3 design references with the current baseline;
   correct historical reconnect ownership, harness-count, and elicitation
   placement contradictions.
2. Create the contract baseline and consumer-coverage ledger.
3. Specify: multi-connection app registry and paginated discovery poll;
   composer draft-store durability/reconciliation; retention and first-open
   huge-history behavior; Bridge data boundaries; global cost policy; and the
   mandatory cross-layer Inspect contract.
4. Start the upstream client-message-id request. Preserve its adoption seam;
   do not block the first UI build. It remains the robust solution for the
   accepted D28/D30 ambiguity residuals.
5. Specify release/update/data-recovery and local diagnostics contracts.

**Gate:** current code, capture corpus, specs, and contract claims have a
single current source of truth. No package below relies on stale historical
semantics.

### 1. App control plane

Create `lens-app` as a host over existing modules, not a second engine:

- multi-connection registry; focus-driven summary/detail promotion; session
  lifecycle and cross-session poll;
- local managed-runtime provision/supervision, health, restart, logs, and
  first-run remediation;
- remote onboarding, Keychain credential storage, HTTPS/system-trust policy,
  and harness prerequisite diagnosis;
- typed, gated Inspect snapshots and transition rings.

**Gate:** local and already-running remote connections recover and surface
health without foreground-thread I/O; pagination and active-stream-over-poll
precedence are proven.

### 2. Native-TUI vertical dogfood loop

Pull forward the terminal path:

- add typed terminal WebSocket attach (binary PTY frames, JSON resize/control,
  ownership/read-only behavior);
- build a GPUI terminal widget with focus, paste, resize, reconnect-gap/ring,
  lifecycle handling, and input/paint benchmarks;
- add a minimal focused Lens window with terminal-resource discovery and the
  per-session interaction-surface switcher.

**Gate:** a real native-harness session is controllable inside Lens, including
permission/read-only behavior and server/terminal loss recovery.

### 3. Rendered transcript cutover

Build the first full rendered consumer:

- D23 disk `RowSource`, off-thread paging, retained id-keyed entities, live
  scratch tail, and no clear/recreate/remount behavior;
- pure `Item`/scratch projection covering item variants, tool pairing,
  errors, compaction, reconnect, agent/resource/todo markers, and send fates;
- native GPUI list virtualization; vendor-and-patch markdown with progressive
  streaming, selection preservation, scroll anchoring, and Lens-owned
  link/image/path sanitization;
- composer-owned durable drafts and recovery UI.

**Gate:** native TUI and rendered composer can submit consecutive turns in one
session; switching is local/no-restart; transcript correctness and frame-time
contracts hold under real captured streams.

### 4. Shell and shared primitives

Build board/groups/cards/waves/archive; rail, navigator, working-area
splits/tabs, tray, deep focus, multiwindow, launchers, search/palette/keymap,
theme/tokens/icons/scrollbars, and the Bridge container. Board/card views use
coarse summaries only, never transcript detail. Build reusable annotation
machinery only after a concrete anchor contract exists.

**Gate:** board/tab/group state survives relaunch; keyboard and multiwindow
routing are tested; fleet views remain within their performance budgets.

### 5. Hosted product surfaces

Implement, in dependency order:

1. approvals/elicitations, sharing, policy, identity, and presence;
2. agent registry, harness presentation, bundle/YAML flow, controls, and live
   switch behavior;
3. sub-agent topology;
4. environment files, search, attachments, Review/diffs/comments, worktrees;
5. Bridge Inbox/Log/Knowledge once its local data contract is specified;
6. terminal-transfer UX.

Each surface obtains current-pin captures before relying on a weakly modeled
field or route. Canvas/Concierge and universal annotation wait for their
payload, security, persistence, and anchor specifications.

### 6. Release hardening

Implement and verify the direct-distribution system: signing, entitlements,
notarization, stable signed feed, update verification, coupled runtime slots,
health-gated activation, backup/compatibility rollback, and local diagnostics
export. Add CI and run release-profile behavioral and performance gates.

**Gate:** a clean Apple-Silicon Mac can install, update, restart the runtime,
rollback one release after a data migration, and recover from a failed update
without losing user state.

## Required evidence

Every relevant package supplies focused, deterministic proof:

- logic contracts for reducer/command/persistence behavior;
- current-pin golden/live captures for consumed omnigent behavior;
- forced server-drop/reconnect and runner-bound terminal/resource scenarios;
- render paint, input-to-first-paint, streaming, and scroll performance against
  the 120 fps target and 90 fps regression line;
- developer-tools Inspect snapshots with zero-cost-off behavior;
- Apple-Silicon installation, update, rollback, backup-restore, signing, and
  privacy-redaction drills.

Existing client/core/store benchmarks remain necessary but do not substitute
for end-to-end render or release evidence.

## Post-launch deferred work

- remote SSH host installation/management;
- managed-sandbox provider selection and provider-specific capability/cost UX;
- notification v2 for a fully quit Lens, if a suitable server push contract
  exists;
- VS Code/terminal theme importers;
- deeper drift checks and `gap == Some(0)` proof when a scheduled consumer
  relies on them.

## Seams and change triggers

This sequence depends on the omnigent contract remaining compatible at each
advance. Any release that changes a consumed endpoint, event, item shape,
terminal protocol, or native-harness input behavior stops the affected package
at the rolling-baseline gate until its targeted evidence and owner spec update.
The client-message-id contract changes send/reconcile and composer recovery;
when available it requires a focused client/core/UI migration rather than an
additional compatibility shim.
