# Lens — spec gaps backlog

Deferred design areas surfaced 2026-07-13 that have **no (or partial) spec coverage**.
Each is an *independent* subsystem — it gets its own spec → plan → implementation
cycle, not one mega-spec. This file is the parking lot so we don't forget them;
pick one, brainstorm it to a design doc, then strike it here.

Ordering below is by "blocks shipping Lens to a second human" (roughly).

## Ranked backlog

1. **App release / signing / update** — *zero coverage; biggest true void.*
   Code-signing, notarization, DMG/pkg packaging, auto-update (Sparkle or
   equivalent), release channels, **Lens app versioning** (distinct from the
   omnigent contract pin), crash reporting. Blocks any distribution beyond
   `cargo run`.

2. **Bundle omnigent into the `.app`** — *partial; coupled to #1.*
   Supervision itself is designed (`server-lifecycle.md` §3: hermetic `uv` env,
   supervise, recovery, contract gate). The gap is **shipping omnigent inside the
   signed bundle** so first-run needs no network and no system Python — today it's
   `uv tool install omnigent==…`, which assumes network + working uv and conflicts
   with notarization/gatekeeper + offline first-run. Solve alongside #1.

3. **Observability of Lens itself** — logging, crash reporting, a
   user-exportable diagnostics bundle. Prereq for anyone but the author filing a
   useful bug.

4. **Secrets / credential lifecycle** — Keychain is used ad hoc today (cursor SDK
   key, remote-connection tokens, harness API keys). No spec owns *where* these
   live, how they're scoped per-server/per-harness, and rotation. `E. Auth &
   multi-user` (README) resolves the *posture*, not the credential storage
   lifecycle.

5. **TUI-native harness handling** — reading **(A)**: `claude-native` /
   `cursor-native` are PTY/TUI-only and **fold reasoning** into `output_text`
   (memory `live-event-recapture-findings`), so they can't produce a clean
   rendered reasoning stream. **Decision (2026-07-13):** the focused chat for a
   TUI-native harness offers a **per-session toggle** between the *rendered
   stream* and the *raw TUI*. Ties into the deferred **terminal WS attach
   (Plan 7)** in `workspace-and-terminals.md` and the two status/usage vocab
   normalizations. Needs its own spec: toggle placement, TUI surface (reuse the
   terminal WS attach surface?), how state/quiescence is tracked when the user is
   in raw-TUI mode.

6. **Onboarding / first-run product UX** — `server-lifecycle.md` §10 covers the
   first-run *backend*; the *product* empty-state ("no servers, no sessions —
   connect your first omnigent") is unspecified.

7. **Settings / preferences surface** — the STATUS "Tunables" (auto-sleep
   threshold, poll cadence, terminal scrollback/fleet-memory budgets, transcript truncation tiers,
   `cost_samples` cadence) have no UI home. Where the user sees/changes them.

8. **Data lifecycle / migration** — the two-tier SQLite store has a
   schema-version degrade path, but no app-level story for data location, backup,
   export, or "nuke and re-sync."

9. **Multi-machine identity** — two Lens instances (laptop + desktop) pointing at
   the same remote omnigent: independent replicas, or any Lens-side sync? Decide
   the posture even if the answer is "independent, no sync."

---

## Cross-repo seams (agreements to keep in sync, not backlog)

- **lens-ui ↔ lens-terminal integration seam** *(agreed 2026-07-14 during a grill
  of the lens-ui shell-skeleton design; recorded on both sides).* Direction is
  **lens-ui depends on lens-terminal** and *hosts* its tab — lens-ui is
  deliberately **not** a dependency of this workstream (§ this doc, "lens-ui is
  not a dependency"). Consequences both docs commit to:
  - **lens-terminal exports the constructor**
    `open(TerminalTarget, Arc<Client>, TerminalOpenOptions, cx) -> Entity<TerminalTab>`
    and public `TerminalTarget::{Existing { session_id, terminal_id },
    OpenOrCreate { session_id, key }}` plus
    `AccessIntent::{Automatic, ReadOnly}`. These values leak no Ghostty or
    transport types. `open` returns immediately in `Starting` and builds its
    own `TerminalAttachment` asynchronously.
  - **lens-ui owns routing and policy**: it chooses the logical target, resolves
    `ConnectionId → Arc<Client>`, supplies access intent/preferences, calls
    `open(...)`, and **wraps** the returned `Entity<TerminalTab>` in a lens-ui
    `ContentTab` adapter (lens-terminal cannot implement lens-ui's `ContentTab`
    because there is no dependency edge that way). It performs no terminal
    REST/WS work.
  - The host seam is one typed inbound `TerminalHostEvent` stream and one typed
    outbound `TerminalEvent` stream. Presentation updates atomically expose
    identity/reported title, lifecycle, effective access, and progress. Host
    requests cover user-gesture URL opens, permissioned OSC 52 clipboard writes,
    and background notifications. `TerminalTab::focus_handle(cx)` is direct,
    not a callback. There is no generic `RequestClose`.
  - Native `/clear` has no public terminal-transfer operation. `lens-ui` handles
    public `session.superseded`, then sends the typed supersession host event so
    the tab reattaches the same terminal under the target session. Lens never
    calls omnigent's schema-hidden internal transfer route.
  - lens-ui does **not** publish any attach type. An earlier lens-ui
    `SessionAttach { …, attach: TerminalAttachCapability }` sketch was **dropped**
    (wrong shape: no such capability exists, and it omitted `TerminalId`/access
    mode). If the `open(...)`/target shape changes here, update lens-ui §5.2.

---

## Upstream contract gaps

- **Immutable terminal generation identity** — omnigent 0.5.1 derives terminal
  IDs from `(terminal_name, session_key)` and may recreate a few server-owned
  terminal roles on attach while reusing that ID. It emits another live
  `session.resource.created` and normally persists a corresponding
  `ResourceEventData` item for reconnect discovery, but supplies no generation
  token and persistence is best-effort. Lens preflights GET, consults resource
  event history, and treats an observed duplicate creation as a replacement,
  but cannot prove the remaining race away. Omnigent should expose an immutable
  generation/resource ID (or an equivalent durable replacement discriminator).
