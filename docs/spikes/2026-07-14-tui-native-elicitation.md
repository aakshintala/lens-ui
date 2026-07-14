# Spike — claude-native elicitation surfaces (TUI vs rendered)

**Date:** 2026-07-14 · **omnigent:** 0.5.1 (`08285468`, matches pin) ·
**Driver:** live claude-native session (`conv_5a6c7f4a…`), user drove the omnigent
web UI; Claude Max / Opus 4.8 real turns. **Purpose:** settle whether a
TUI-native harness's permission/elicitation prompt appears in the terminal, the
web/SSE surface, or both — the load-bearing input to the **TUI-native toggle**
spec's decision on whether Lens should render a competing elicitation overlay in
TUI mode.

## Method

Created a live `claude-native` session via `POST /v1/sessions`
(`agent_id = claude-native-ui`, external host, scratch git workspace). Runner
auto-created the `claude:main` agent-terminal (tmux). Instrumented two timestamped
recorders: (1) `GET /v1/sessions/{id}/stream` SSE tail; (2) `tmux capture-pane`
change-tracker on the pane socket. User triggered elicitations from the web UI and
resolved them in each surface while both logs recorded.

## Findings (evidence = SSE + pane logs, times are wall-clock)

### F1 — Prompts are genuinely dual-surface, in parallel (not fail-ask)
An `ExitPlanMode` request published `response.elicitation_request` (`01:51:55`)
**and** Claude's plan prompt was up in the pane at the same time. The source read
had left "does the in-pane prompt appear in parallel, or only on fail-ask?"
undetermined; **answer: parallel.**

### F2 — Web→terminal resolution is ASYMMETRIC and, for mode-changes, a dead end
- Approving `ExitPlanMode` **"run in auto mode"** in the **web UI** showed
  "Approved" locally but **emitted no `elicitation_resolved`**; the terminal stayed
  blocked ~10s. Resolution (`elicitation_resolved`, `01:52:31`, ~36s after request)
  only landed after the user **also approved in the TUI**. The web UI's "Approved"
  was a **premature client-optimistic render** — the server was still blocked.
- Reverse direction is clean: approving in the **TUI** resolved in ~13s
  (`01:55:18`→`01:55:31`) and surfaced to web as **"Approved elsewhere."**
- **Diagnosis:** `ExitPlanMode "run in auto mode"` bundles a **Claude-internal mode
  change**, which the `PermissionRequest` hook's JSON-reply channel cannot apply →
  this elicitation class is **structurally TUI-only**.

### F3 — Generic tool permissions DO round-trip from web
`Bash(touch …)` (`02:06:41`→resolved `02:06:50`→ran) and `Write` (`02:07:10`→
resolved→file created) both resolved correctly from the web UI. So the dead end is
**specific to the mode-change class**, not permissions in general. Rendered-mode
`/resolve` is fine for the common case.

### F4 — External approval can destabilize the harness's mode → transient loop
Approving from an external surface (web *or* TUI) left claude-native briefly
confused about its permission mode, producing repeated re-prompts before settling
back to `idle`. The two-surface model is brittle by construction.

### F5 — Mechanics
Elicitations carry `policy_name: "claude_native_permission"`, `phase:
"pre_tool_use"`, `permission_mode` (`plan`/`default`), and class-specific fields
(`exit_plan_mode`, `allow_all_edits`, `remember_scope`). `elicitation_resolved`
fires **twice** per resolution (once per channel) — dedupe by `elicitation_id`.

## Conclusions → design

1. **TUI-toggle (locked):** In TUI mode the **terminal owns harness-rendered
   prompts**; Lens renders **no competing interactive card** — only a passive
   "N pending — switch to rendered" badge as a safety net for elicitations with no
   terminal representation (server/MCP-originated). Rationale: F1 (both appear) +
   F4 (doubling is actively harmful).
2. **The toggle is load-bearing, not just fidelity:** by F2, flipping to the TUI is
   the **only** way to resolve the mode-change elicitation class on a native
   harness. "Flip to TUI to approve this" is the escape hatch for a rendered-mode
   dead end.
3. **Permissions-spec risk (separate doc):** rendered-mode elicitation is fine for
   common permissions (F3) but has (a) a **TUI-only class** (F2) and (b) an
   **external-approve-destabilizes-mode** fragility (F4). Candidate omnigent bug
   report; the permissions spec must detect the mode-change class and route the
   user to the TUI (or offer only round-trippable options) rather than presenting a
   dead-end approve button.

## Pin-and-verify / not covered
- Only `claude-native` tested. `cursor-native`/`codex-native`/`antigravity-native`
  use different hooks and (per prior source read) **do** inject tmux keystrokes on
  web-resolve — their asymmetry may differ. Re-verify per harness before relying on
  F2/F3 generalizing.
- Whether a **plain** plan-approval (no "run in auto mode" mode change) round-trips
  from web was not isolated.
