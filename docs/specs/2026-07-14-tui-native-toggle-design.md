# TUI-native harness toggle — design

**Date:** 2026-07-14 · **Status:** Draft, reworked after cross-family review
(grok-4.5 + gpt-5.6/codex; verdicts ship-with-fixes / needs-rework — all
findings folded in) · **omnigent pin:** 0.5.1 (`08285468`)

A focused-session feature: for a session whose harness is **TUI-native** (the
`-native` variants — `claude-native`, `cursor-native`, …), the chat column offers
a **per-session toggle** between Lens's **rendered stream** and the harness's
**raw TUI**. Grounded in a live claude-native spike
([`docs/spikes/2026-07-14-tui-native-elicitation.md`](../spikes/2026-07-14-tui-native-elicitation.md))
and two omnigent 0.5.1 source investigations (cited inline).

---

## 0. Scope & boundaries

**This document owns:** the toggle's mode model, elicitation *routing* (which
surface owns a given prompt), detection, switch-agent behavior, view persistence,
attach lifecycle, and access rules — i.e. wiring the *existing* agent-terminal
(workspace §9) into the focused chat column as an alternate view of the same
conversation.

**It does NOT own** (defers, with cross-refs):
- Terminal WS client / ring buffer / attach mechanics — **workspace §9** (LOCKED
  decision C covers the §9.2 ring buffer only; §9.1/§9.4 are the attach + agent-
  terminal definitions this consumes).
- Native elicitation card rendering + `/resolve`, and **cross-surface arbitration**
  (an elicitation appearing in a conversation window *and* the Bridge Inbox) —
  **permissions & elicitations** + **Bridge**. This doc only decides *which
  surface owns* a prompt (§3.1), not how cards render or how Bridge dedups.
- The **TUI-by-default** global preference — **Settings** (`SPEC-GAPS.md` #7).
- The **mode-change elicitation dead-end** fix — **permissions & elicitations**
  (`SPEC-GAPS.md` cross-spec risks).

**Required cross-doc edit (follow-up, not this spec):** README decision G
("any session can ⤢ detach to its own window") must be tightened to **detach =
move; a conversation is shown in ≤1 window** (see §6).

## 1. Purpose & motivation

Priority order:

1. **Correctness.** For **claude-native**, some elicitations are **structurally
   TUI-only** — unresolvable through Lens's rendered `/resolve` path. The spike
   proved this for `ExitPlanMode "run in auto mode"` (a Claude-internal mode
   change the permission-hook reply channel can't apply; spike F2). For that
   class the TUI is the *only* resolution surface → "flip to TUI to approve" is a
   required escape hatch. **This claim is claude-native-measured**; other harnesses
   are pin-and-verify (§9a).
2. **Fidelity.** The rendered stream is a *lossy mirror*: the transcript forwarder
   folds reasoning into `output_text`, and live deltas are best-effort (dropped
   chunk-POSTs are not retried; only the final committed item is durable). The TUI
   is authoritative in-flight.
3. **Authenticity.** Native slash-commands, the harness's own input history, and
   its native prompts exist only in the real TUI.

### 1.1 Harness tiers — graceful degradation

Native-harness fidelity is **bounded by omnigent's transcript-forwarder maturity**
(the bridge that mirrors a TUI harness's activity into SSE — the source of the
folded reasoning, best-effort deltas, and dual-surface elicitation mess). Lens
therefore treats harnesses in two tiers, and this must be explicit so the toggle
is not an implicit bet on that bridge:

- **First-class tier — SDK-driven harnesses** (`claude-sdk`, `openai-agents`, …):
  clean SSE, no forwarder in the path. Lens's core experience targets these.
- **Best-effort tier — `-native` harnesses:** rendered *and* TUI fidelity ride the
  forwarder; both improve as omnigent's bridge does. The toggle is upside that
  scales with that maturity, not a dependency the core rests on.

Lens's core value (multi-server, N-warm-streams, board, Bridge) routes through
SSE + `/items`, **not** the agent-terminal — so a weak forwarder degrades the
native tier, it does not compromise the product.

## 2. Substrate — reuse, not net-new

The "raw TUI" is the **agent-terminal** already defined in workspace §9.4: a tmux
PTY running the harness's own TUI (e.g. `claude --resume`), attached via
`WS /v1/sessions/{id}/resources/terminals/{tid}/attach` (§9.1), drawn in the
existing terminal widget with the §9.2 ring-buffer reconnect. It may also appear
as a working-area **Terminal tab** (§9.4) — the toggle surfaces the *same*
resource in the chat column; one WS attachment + ring buffer is shared and
rendered in either place (§7).

**Coherence guarantee (committed turns + input).** A server-side **transcript
forwarder** runs continuously alongside the agent-terminal, mirroring every
*committed* turn to `external_conversation_item` / `external_output_text_delta` →
persisted to `/items`, published on `/stream` (omnigent `claude_native_forwarder.py`,
`sessions.py`). Consequences:

- **Input source is irrelevant to transcript state.** Lens's composer
  (`POST /events` → `tmux send-keys` into the pane for native sessions) and direct
  keystrokes into the PTY are indistinguishable to the harness; the forwarder
  mirrors both. Lens's state model stays coherent regardless of which surface
  drives a *turn*.
- **This coherence covers turns/input, NOT elicitation resolution.** Elicitation
  resolution is **asymmetric** (spike F2) — see §3.1. Do not read "same input
  path" as "either surface resolves anything."
- **Capture is transcript-based (committed turns).** Uncommitted draft text in the
  TUI box is not mirrored until submit — a real in-flight difference (§9c), and the
  reason a switch must warn about draft loss (§5).

## 3. Mode model — a full interaction swap

The toggle swaps the **entire interaction model** of the chat column:

| | **Rendered mode** | **TUI mode** |
|---|---|---|
| Output | Lens native bubbles (SSE-driven) | the harness's real terminal |
| Input | Lens composer | the terminal's own prompt line |
| Affordances | Lens input-history, send-recovery, optimistic bubble | native slash-commands, native input-history |
| Scrollback | rendered transcript (disk-backed, full history) | terminal ring buffer (ephemeral tail) |
| Elicitations | Lens cards inline | **routed per §3.1** — not simply "terminal-owned" |

A **swap, not a downgrade**: native affordances replace Lens's.

### 3.1 Elicitation routing (the load-bearing rule)

Every pending elicitation is assigned exactly one **route**, computed from
`(harness, elicitation class, viewer's access level)`. The route decides which
surface owns it; the passive badge counts only what Lens must present. `N` and the
route are derived from the `pending_elicitations` map (source of truth), which is
the only durable enumerator (polling is reconciliation, not the trigger).

| Route | When | Surface in TUI mode |
|---|---|---|
| **`Terminal`** | harness-rendered prompt on a **verified** harness, viewer **can write-attach** (owner) | answered in the pane; **no Lens card** (avoids the double-surface UX of spike F1/F4) |
| **`LensCard`** | server/MCP-originated (URL/OAuth, policy, sharing), or a harness-rendered prompt on an **unverified** harness (safe default, §9a) | interactive Lens card stays shown |
| **`OwnerRequired`** | harness-rendered prompt the **viewer cannot resolve** — non-owner (read-only attach, can't type in the pane) **or** the claude-native mode-change class (F2) that the rendered path can't resolve | a non-interactive notice + owner notification; **never suppressed into nothing**, and **never advertised as "flip to TUI"** to a viewer who can't write-attach |

Rules:
- **The badge = count of non-`Terminal` routes** ("N pending — act here / switch to
  rendered"). `Terminal`-routed prompts are answered in the pane and not counted.
- **Suppression is per-harness and opt-in.** `Terminal` routing (suppress the Lens
  card) is enabled only for **verified** harnesses — **claude-native today**. For
  unverified harnesses the default is `LensCard` (keep the card): worst case a
  double-surface annoyance, **never a hidden/unresolvable prompt**.
- **Never optimistic.** A prompt is pending until `response.elicitation_resolved`
  (dedupe by `elicitation_id` — it fires twice, once per channel, spike F5). Lens
  must not render "approved" on click (the web UI's premature "Approved" was the
  spike's worst UX).
- **Cross-surface (Bridge) arbitration** — whether a `LensCard`/`OwnerRequired`
  prompt *also* shows in the Bridge Inbox — is a permissions/Bridge concern (§0).

### 3.2 Placement + deep-focus

A segmented control in the focused-session **chat-column header**. ⌘D deep-focus
(shell §7.1) **hides the chat column entirely** (maximizing the working area), so
the toggle and the chat-resident TUI view are hidden in deep-focus — symmetric
with the rendered chat, and acceptable for review-heavy supervision **because
neither prompt class is stranded:** `Terminal` prompts remain answerable in the
working-area **Terminal tab** (§2/§9.4), and `LensCard`/`OwnerRequired` prompts
surface in the **Bridge Inbox** (a rail destination that survives deep-focus).
Hidden when capability is false (§4); shown-but-pending when capability holds but
readiness is `Starting` (§4).

## 4. Detection — capability + readiness, kept separate

- **Capability** (*does the toggle exist?*) = the session's **current harness** is
  in Lens's TUI-native set (agent-definition §4 registry / `AgentObject.harness`).
  A pure function of the harness, recomputed on switch-agent (§5). **Not** inferred
  from terminal presence.
- **Readiness** (*is the TUI usable now?*) = a tri-state from snapshot + resource
  state + WS outcome, **never conflated with forwarder/SSE health**:
  - `Starting` — `terminal_pending == true` (snapshot field) or attach in flight.
  - `Ready(terminal_id)` — an agent-terminal resource exists and the WS attaches.
  - `Unavailable(reason)` — auto-create failed / no terminal / attach error; a
    *missing* terminal is **not** always "starting" (`terminal_pending`
    distinguishes spin-up from failure).

The toggle is **shown-but-pending** while capability holds and readiness is
`Starting`; it surfaces `Unavailable` explicitly (retry affordance) rather than a
dead control. *Pin-and-verify:* confirm each TUI-native harness auto-creates an
agent-terminal (spike confirmed claude-native).

## 5. Switch-agent behavior

A live agent-switch fires `_reset_runner_resources_after_switch` (workspace §9.3):
the old agent-terminal drops, the new harness's comes up. Lens recomputes
**capability + readiness** as part of the in-place card+composer re-render it
already does (README decision J).

**View rule:** *keep the current view if the new harness supports it; else fall
back to rendered, which becomes the current view.*

| Transition (current view) | Behavior |
|---|---|
| TUI-native → TUI-native (in TUI) | **Stay TUI.** Old attach cancelled; ride `Starting`; attach the new harness's terminal. **Segment the ring buffer** with a `↻ switched to <harness>` break (an *extension* of §9.2's `↻ reconnected`, not the same event). |
| TUI-native → non-TUI (in TUI) | **Force-fallback to rendered, signposted** (`<harness> has no TUI — showing rendered stream`); toggle hides. Owner-initiated + idle-gated → never mid-turn. |
| non-TUI → TUI-native (in rendered) | Toggle **appears**; stay rendered (opt-in). |
| any → any (in rendered) | Stay rendered. |

**Epoch fence (required).** Bind each terminal attach to a monotonically-increasing
switch epoch; a later switch **cancels stale attaches** and ignores late
resource-created events for a superseded epoch (repeated switches otherwise leave
old WS attaches rebinding after a newer switch). An explicit user toggle always
wins over an in-flight fallback.

**Native `/clear` supersession.** A native slash-command the toggle *enables* —
Claude `/clear` — emits **`session.superseded`** and moves the live terminal to a
*new* conversation (omnigent `SessionSupersededEvent` / `_post_clear_supersession`).
The binding must **atomically rebind/redirect** to `target_conversation_id`, not
strand the old attach.

**Draft-text warning.** Idle-gating does not protect **unsubmitted TUI draft text**
(not mirrored, §2). Warn before a switch/fallback that would discard it.

During any `Starting` window show the terminal's re-attaching state, **not** a
bounce to rendered (flipping rendered↔TUI↔rendered is more jarring than a stable
pane). There is **no long-lived "TUI preference"** across harness changes — only
*current view*, changed solely by explicit toggle or forced fallback.

## 6. View persistence — runtime-only, window-local

- **Current-view lives in `WindowState`** (per-window) and is **never persisted.**
  Every fresh materialization — app restart, or an actual Lens Sleep→wake — starts
  **rendered**. Rationale: simplest invariant (a fresh load has no prior view →
  the cheap, instantly-coherent default); cold start never eagerly attaches PTYs;
  rendered is warm-SSE-instant, TUI always has a `Starting` delay.
- **One conversation ⇒ ≤1 window.** Pop-out **moves** a conversation to its own
  window; navigating to an already-open conversation **raises the existing window**
  (single-instance-per-document), so a conversation is never shown twice. This
  dissolves the multi-window dual-surface problem: there is never a second window
  driving the same pane, so no cross-window write-lock is needed. (Requires the
  README decision-G edit, §0.)
- On **wake**, reconcile **pending elicitations first** (route per §3.1) and show
  the readiness tri-state — do not assume a torn-down terminal (§7).
- The durable "I live in the TUI" default is a **global** in the Settings spec
  (`SPEC-GAPS.md` #7) — not per-session disk persistence. The split-rule middle
  ground (persist across wake but not restart) is rejected as "sometimes sticky."

## 7. Attach lifecycle

- **Lazy-attach on first flip.** For a running native session the agent-terminal
  already exists (runner auto-creates at boot), so first flip is a WS attach +
  redraw, not a spawn.
- **Warm while the window is open.** Keep the WS + ring buffer alive for the
  window's lifetime so flip-back → re-flip is instant and scrollback survives.
- **Detach on window-close or an actual Lens Sleep.** Sleep sends *best-effort*
  `stop_session`; the server **may** terminate the PTY (workspace §9.2) — do **not**
  assume teardown. Auto-sleep is terminal-aware: it must **not** fire while the TUI
  is attached or an elicitation is pending (§9.2 / state model).
- Cost is one live attach per *open TUI-flipped window* — it does not touch the
  N-warm-SSE-streams budget.
- **Acceptance criteria** (this is a foreground path): p50/p95 **first-PTY-byte on
  flip** and **hot-reflip** budgets; attach/redraw must **not block the foreground**;
  `Unavailable` → retry is a tested path. (Targets set at plan time; "sub-second" is
  a hypothesis, not a spec.)
- **Deferred tuning knob:** "keep the last K attachments warm (LRU)" — add only if
  re-flip on recently-closed windows proves slow with real usage. Not v1.

## 8. Access & inheritance from workspace §9

- **Read-only for non-owners.** Agent-terminals are read-only by default; only
  `LEVEL_OWNER` write-attaches (§9.1). A non-owner in TUI mode *watches*; they
  cannot type — hence the `OwnerRequired` route (§3.1) for any harness prompt they
  can't resolve. Terminal **write-ownership is session-global**, but with one
  conversation ⇒ ≤1 window (§6) there is never in-app contention for it.
- **Board card unaffected.** N-warm SSE keeps a card's rendered live state current
  even while its window is in TUI mode.
- **Interrupt / steering** in TUI mode uses the terminal's native controls; Lens's
  rendered-mode interrupt is a rendered-mode affordance.

## 9. Seams / pin-and-verify

- **(a) Per-harness verification matrix.** F1–F4 are **claude-native-measured**.
  `cursor-native` / `codex-native` / `antigravity-native` / `pi-native` /
  `qwen-native` (and the rest of agent-definition §4's `*-native`) use different
  permission hooks and — per the earlier source read — **do inject tmux keystrokes
  on web-resolve**, so their elicitation asymmetry may differ (possibly *better*).
  Each harness needs a verified entry before its harness-prompts get `Terminal`
  routing; until then they default to `LensCard` (§3.1). The **toggle mechanism
  itself is generic** and ships for all TUI-native harnesses in v1 — only the
  suppression *optimization* is gated.
- **(b) Mode-change dead-end** is a permissions-spec concern (cross-ref spike +
  `SPEC-GAPS.md`); the `OwnerRequired` route depends on that spec defining the
  owner-resolution path. F4 (external approval destabilizes the harness's mode) is
  a **mitigation rationale for suppression, not proof of safety** — the permissions
  spec must define loop/re-prompt + cross-surface arbitration.
- **(c) Draft-text fidelity** — transcript-based capture shows keystrokes live in
  the TUI but the rendered stream moves only on submit; reflect in UX copy + the §5
  switch warning so it isn't read as a bug.
- **(d) Agent-terminal ⟺ every TUI-native harness** — confirmed for claude-native;
  verify others auto-create one (else capability must attach on demand, §4).
- **(e) Bridge cross-surface arbitration** (§3.1) — owned by permissions/Bridge.

## 10. Out of scope / deferred

- TUI-by-default **global** → Settings (`SPEC-GAPS.md` #7).
- Permissions-spec **mode-change / OwnerRequired resolution path** + Bridge
  arbitration → permissions & elicitations.
- **LRU warm-attachment pool** tuning knob (§7).
- **Per-harness elicitation verification** for non-claude natives (§9a).
- README **decision-G tightening** (§0) — a separate cross-doc edit.

## Cross-references

- `workspace-and-terminals.md` §9 — agent-terminal / WS attach / ring buffer
  (decision C = §9.2 ring buffer).
- `permissions-and-elicitations.md` — native cards, `/resolve`, the mode-change
  dead-end + `OwnerRequired` resolution this hands off.
- `application-shell-and-layout.md` §7.1 — focused-session layout + ⌘D deep-focus
  (hides chat); README decisions F, G, J.
- `docs/spikes/2026-07-14-tui-native-elicitation.md` — empirical grounding.
- `SPEC-GAPS.md` — Settings (#7) + cross-spec permissions risk.
