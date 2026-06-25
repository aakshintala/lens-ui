# Lens — STATUS archive

Full dated session entries, append-only (newest at the bottom). The live
forward-looking state is in `STATUS.md`; detailed logs roll here as they age.

---

### 2026-06-24 — grilling pass, doc walkthrough, first renders

**Grounding.** Pulled omnigent 0.2.0 source into `../omnigent/`; ground-truthed
every contract claim. Render workflow set to **local-only** (`docs/design/renders/`).

**Factual corrections (verified against source).**
- Harnesses **16** (added `copilot`, `opencode-native`, `antigravity-native`, `qwen-native`).
- Cost is **server-computed** (`total_cost_usd`) — no Lens price table.
- Session status has **5** values — added `Launching`.
- **Fork** is `POST /fork`, not a `SessionEventInput` arm.
- **switch-agent** = `POST /switch-agent` (not `PUT /agent`); guards: owner-only +
  idle-only (409) + no sub-agents + no no-op; runner resources reset.
- `child_session.updated` carries **partial deltas** (merge, not replace).
- `X-Forwarded-Email` is **trusted-proxy** auth (OIDC cookie is the real remote credential).

**Cross-cutting decisions — all resolved (ledger was drifting).**
- **A** task=session; **Group** is the grouping (no `Task` entity); single-root
  file tree default; "task" retired as a term.
- **B** sub-agent home = tray "Sub-agents" segment; children never board cards.
- **C/F/G** ratified (ring buffer; collapsible working area + ⌘D; multi-window).
- **I** two-axis cost: cumulative per-card/project (server USD) + time-windowed
  global (today/7d/30d) via new `cost_samples` table.
- **J** switch-agent in-place handoff + verified guards.

**Session lifecycle — reshaped (was "Sleeping = client detach").**
- **Sleep** = `stop_session` (reclaim harness/PTY) + dim, stays visible;
  auto after ~10-min quiet (terminal-aware, skips pinned/needs-input).
- **Archive** = `stop_session` + hide (no longer UI-only).
- **Delete** = server delete. **No stream cap** (self-bounds via auto-sleep).
- Wake = resume + re-bind runner.

**New decisions (walkthrough).**
- **Status→wave** mapping pinned; **Ready** = idle + unviewed completion;
  **Scheduled** reserved but cut from v1 (it's the `/loop` state).
- **Concierge** is local-server-only → **local server is always-on baseline
  infra**; renders as a **floating pinnable chat panel** (⌘⇧C).
- **Keybindings:** `^\`` toggle terminal, **⌘⇧C** Concierge, **⌘⇧1-9** board-switch.
- **Bridge Inbox UI** (closes decision H): **pinned "Needs you" band + reverse-chron
  stream**; card = `kind · from→to · status · body · actions`.
- **Card design:** icon tile · status · title · `harness·model` · activity line ·
  `📁 repo ⑂ branch` rows · host+cost+ctx bar; **tinted group bodies**.
- **Residency + notifications:** resident menu-bar app; native needs-input
  notifications + `lens://` deep-links; ⌘W hides, ⌘Q quits; background poll
  throttles (not pauses).

**Renders (local):** `board-home.html`, `focused-session.html`, `bridge-inbox.html`.

**Cleanup:** scrubbed internal lineage (Cairn/MessageCenter/"older spec"/predecessor/
infinite canvas), kept the real reference apps (Arbor/Paneflow/gpui-flow) + Polly;
fixed editing-artifact typos.

**Docs touched:** all 11 in `docs/design/` + README.
