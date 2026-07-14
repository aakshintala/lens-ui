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

5. **TUI-native harness handling** — ✅ **SPEC WRITTEN 2026-07-14**
   ([`docs/specs/2026-07-14-tui-native-toggle-design.md`](specs/2026-07-14-tui-native-toggle-design.md),
   commit `bf72ea3`; brainstormed → live spike → dual cross-family review →
   reworked). Blocked on build deps (Plan 7 terminal WS client + `lens-ui`
   viewport). Original gap framing below. — reading **(A)**: `claude-native` /
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
   threshold, poll cadence, ring-buffer size, transcript truncation tiers,
   `cost_samples` cadence) have no UI home. Where the user sees/changes them.
   - **Known requirement — TUI-by-default global** (from the TUI-native toggle
     spec, 2026-07-13): a global preference "prefer the raw TUI when a harness
     offers one." The TUI-native toggle spec deliberately keeps per-session
     current-view as *runtime-only* (always initializes to rendered); the durable
     "I live in the TUI" default lives here as a global, not per-session disk
     persistence. When honored, a fresh session materialization of a TUI-native
     harness initializes to TUI (riding the same `starting TUI…` pending state)
     instead of rendered.

8. **Data lifecycle / migration** — the two-tier SQLite store has a
   schema-version degrade path, but no app-level story for data location, backup,
   export, or "nuke and re-sync."

9. **Multi-machine identity** — two Lens instances (laptop + desktop) pointing at
   the same remote omnigent: independent replicas, or any Lens-side sync? Decide
   the posture even if the answer is "independent, no sync."

## Cross-spec risks discovered during design

- **Permissions spec — mode-change elicitations are TUI-only for native harnesses**
  (found 2026-07-14 spike, `docs/spikes/2026-07-14-tui-native-elicitation.md`).
  For `claude-native`, generic tool permissions round-trip fine from Lens's
  rendered `/resolve` path, but the **mode-change class** (e.g. `ExitPlanMode`
  "run in auto mode") **cannot be resolved from the API** — it structurally
  requires the harness TUI. The existing `permissions-and-elicitations.md` spec
  must (a) detect this class and route the user to the TUI toggle (or offer only
  round-trippable options) instead of a dead-end approve button, and (b) treat
  approval as pending until `elicitation_resolved`, never optimistic. Candidate
  omnigent bug report (like the client-message-id ask). The TUI-native toggle is
  the escape hatch this relies on.
