# Agent definition

The net-new vocabulary: declarative agent specs, the registry picker, the
19-harness matrix, model & reasoning-effort controls, the bundle-upload clone
flow, and the live switch-agent handoff.

**Status:** Draft, 2026-06-23. Written fresh against omnigent `0.3.0.dev0`.
**Depends on:** the state model (reads `SessionState.agent_id`, `agent_name`,
`llm_model`, `model_options`, `reasoning_effort`, `collaboration_mode`,
`skills`; issues switch-agent via the command flow).
**Seams to:** the application shell (the new-session dialog shell + the
composer's model/effort/collaboration-mode controls' placement + the card
kebab's switch-agent picker), the typed client (the agents subservice + the
bundle upload endpoint).

---

## 1. Scope & boundaries

**This document owns:**

- **The agent spec model** — `~/.omnigent/agents/*.yaml` parsing + the typed
  mirror (§2).
- **The registry picker** — `GET /v1/agents` surfaces the catalog; how the
  picker UI renders (§3).
- **The harness matrix** — the 19 canonical harnesses (`OMNIGENT_HARNESSES`);
  the picker badge per harness; native vs. SDK classification (§4).
- **Model controls** — per-session model override, reasoning effort, skills,
  collaboration_mode (codex-native Plan); these live on the composer (§5).
- **The bundle-upload clone flow** — multipart `POST /v1/sessions` with
  `SessionCreateMetadata`, `PUT /v1/sessions/{id}/agent`, `GET
  /v1/sessions/{id}/agent/contents` (§6).
- **Live switch-agent** — `POST /v1/sessions/{id}/switch-agent` triggers the
  handoff; this document owns the mechanism, the application shell owns the
  visual (§7).
- **The Concierge spec** — Lens's long-standing chief-of-staff agent; its
  spec YAML is a first-class agent (§8).

**This document does NOT own:**

- Session creation lifecycle (the state model + shell new-session flow).
- The composer UI (the application shell — this document owns the *controls*
  and their data; the shell owns *placement*).
- Harness execution (server/runner — the server-lifecycle document).
- MCP server configuration inside an agent spec (the agent spec carries it;
  the runtime owns the execution).

---

## 2. The agent spec model

An agent is a **directory** (`config.yaml` + optional `skills/`, sub-agent
subdirs). The local registry scans `~/.omnigent/agents/<name>/config.yaml`. The
real spec shape (verified against `omnigent/examples/*/agents/*/config.yaml` and
`omnigent/spec/_omnigent_compat.py`):

```yaml
spec_version: 1
name: my_agent
description: Optional free-text description.
prompt: |
  You are a helpful data analyst.
executor:
  type: omnigent              # executor.type — "omnigent" wraps an omnigent harness
  config:
    harness: claude-sdk        # one of the 19 harnesses (§4); required when type==omnigent
    profile: ...               # optional executor profile
os_env:
  type: caller_process         # or a sandboxed env type
  cwd: .
  sandbox:
    type: none                 # none | seatbelt | bwrap (nested under sandbox.type)
guardrails:
  policies:                    # a MAP, not a list
    blast_radius:
      type: function
      on: [tool_call]
      function:
        path: mypackage.mymodule.policy
skills: [planning, review]      # skills bundled in skills/<name>/SKILL.md
# Sub-agents are DIRECTORY-BASED: each lives in its own subdir with its own
# config.yaml, not inlined under a `tools:`/`sub_agents:` map here.
```

**Lens reads this YAML** via the typed client's `Agents::list()` (which reads
the server's scan of `~/.omnigent/agents/`). The picker surfaces name +
description + harness + the tools/policies/skills/terminals summaries
(`AgentObject` schema in `openapi.json:44`).

```rust
pub struct AgentSpec {
    pub id: AgentId,
    pub name: String,
    pub version: u32,                      // monotonic, server-side
    pub description: Option<String>,
    pub harness: Harness,                   // §4
    pub mcp_servers: Vec<MCPServerSummary>,
    pub policies: Vec<PolicySummary>,
    pub skills: Vec<SkillSummary>,
    pub terminals: Vec<String>,             // declared terminal names
}
```

---

## 3. The registry picker

`GET /v1/agents` returns the catalog. **No REST CRUD** — authoring is
filesystem YAML + bundle upload (§6). The picker:

- **Lists agents** with the harness badge (§4), the description, the skills
  roster, and a "has sub-agents" affordance.
- **Filters by harness** (e.g. only show claude-native agents if the user
  prefers).
- **Quick-add** from a group's ＋ (shell §7.6 quick-add flow) — inherits the
  group's default agent.
- **Author/edit** — opens the YAML in the editor tab; Lens watches
  `~/.omnigent/agents/` for changes and refreshes the picker. Authoring is a
  filesystem pass, not a REST mutation.
- **Draft + test** — the bundle-upload flow (§6) lets you spin up a session
  with a draft agent CLONE — useful for iterating on a spec without
  polluting the registry.

---

## 4. The harness matrix

**19 canonical harnesses** — the validator-accepted set `OMNIGENT_HARNESSES`
(`omnigent/spec/_omnigent_compat.py:80-101`), the authoritative source for the
picker:

| Harness | Kind | Provider |
|---|---|---|
| `claude-sdk` | SDK | Anthropic (Claude Code SDK) |
| `claude-native` | Native CLI | Anthropic (Claude Code TUI in tmux) |
| `codex` | SDK | OpenAI (Codex SDK) |
| `codex-native` | Native CLI | OpenAI (Codex TUI) |
| `cursor` | SDK | Cursor (Cursor CLI) |
| `cursor-native` | Native CLI | Cursor |
| `openai-agents` | SDK | OpenAI Agents SDK |
| `open-responses` | SDK | OpenAI Responses (adapter-routed; no dedicated runtime module wrap) |
| `pi` | SDK | Coder Pi |
| `pi-native` | Native CLI | Coder Pi TUI |
| `antigravity` | SDK | Google Antigravity SDK |
| `antigravity-native` | Native CLI | Google Antigravity (agy) TUI |
| `qwen` | SDK | Qwen Code |
| `qwen-native` | Native CLI | Qwen Code (ACP-piped) TUI |
| `goose` | SDK | Block's Goose |
| `goose-native` | Native CLI | Block's Goose TUI |
| `hermes` | SDK | Hermes |
| `opencode-native` | Native CLI | OpenCode native-server harness |
| `copilot` | SDK | GitHub Copilot SDK |

`open-responses` is canonical-but-adapter-routed (accepted by the validator, no
dedicated subprocess module). **Aliases** (`OMNIGENT_HARNESS_ALIASES`,
`_omnigent_compat.py:104-118`: `claude`, `opencode`, `github-copilot`, …) are
normalized before dispatch and documented separately, not shown as picker
entries. Stale figures to ignore: "16" (old spec list, dropped `goose`/`hermes`),
"11" (an old intro typo), "18" (`_HARNESS_MODULES` minus aliases — undercounts
because it drops `open-responses`), "20" (raw `_HARNESS_MODULES` keys including
two alias keys), "7" (`AgentHarnessType` — a narrow ucode-gateway set, not the
picker list).

**Native vs. SDK classification:** native harnesses boot a vendor TUI in a
terminal and route user messages into that running process; the runner must not
replay history. SDK harnesses run in-process model turns.

**`harness` is a free `string | null`** in the schema (`AgentObject`,
`openapi.json` ~62-71) — no wire enum. The typed client's hand-written list of 19
is a Lens-side picker/validation aid, verified against `OMNIGENT_HARNESSES`, not
an openapi enum.

**Picker badge per harness** — each harness has an icon (Claude / OpenAI /
Cursor / Pi / Google / Qwen / Goose); native variants get a small "TUI"
pill. The badge drives the card's `<harness> · <model>` line (shell §5.1).

---

## 5. Model & per-session controls

Per-session, via the composer (shell §7.5):

| Control | Source | Effect |
|---|---|---|
| **Model override** | `PATCH /v1/sessions/{id}` `model_override`; `session.model` event | Surfaces in the work-section chip (§4 transcript). The picker lists models from `session.model_options` (0.2.0 chrome event). |
| **Reasoning effort** | `PATCH /v1/sessions/{id}` `reasoning_effort`; `session.reasoning_effort` event | `none\|minimal\|low\|medium\|high\|xhigh\|max` — a slider/dropdown on the composer. |
| **Skills** | `session.skills` event; `PATCH` labels | Toggle skills per session; the skills list comes from the agent spec. |
| **Collaboration mode** | `PATCH /v1/sessions/{id}` `collaboration_mode`; `session.collaboration_mode` event | Codex-native Plan mode toggle — appears only when the harness is `codex-native`. |

The composer renders these; this document owns the data + the typed client
calls. The shell owns the placement.

---

## 6. Bundle upload + clone

A draft agent can be uploaded as a bundle when creating a session —
`POST /v1/sessions` with multipart `SessionCreateMetadata` (the bundle) —
creating a **session-scoped agent clone**. The clone is *not* in the
registry; it lives for the session's lifetime.

- `PUT /v1/sessions/{id}/agent` — store the bundle for an existing session
  (used by switch-agent, §7).
- `GET /v1/sessions/{id}/agent/contents` — fetch the bundle's contents (for
  editing the draft in-flight).

**Use case:** "draft an agent, spin up a session to test it, iterate, then
file the final YAML in `~/.omnigent/agents/` when ready." The draft-test
loop never touches the global registry.

---

## 7. Live switch-agent (decision J — capability map §0.7-J)

`POST /v1/sessions/{id}/switch-agent` (body `SessionSwitchAgentRequest`, route at
`omnigent/server/routes/sessions.py:14214`) swaps the agent spec on a running
session. `session.agent_changed` fires (`sessions.py:14353`).

**The flow this document owns:**

1. **The card kebab's "Switch agent ▸" picker** (shell §5.3) opens the agent
   picker.
2. If the selected agent has a **different bundle** (e.g. an in-flight edit),
   upload it first via `PUT /v1/sessions/{id}/agent`.
3. `POST /v1/sessions/{id}/switch-agent` with the bundle reference.
4. `session.agent_changed` arrives via the SSE stream — but it carries **only
   `agent_id` + `agent_name`** (`schemas.py:2218-2221`), **no model/skills
   payload**. The state model (§12.2) handles the handoff: update
   `SessionState.agent_id` + `agent_name` from the event, **refetch the session
   snapshot** (`GET /v1/sessions/{id}`) for the new `llm_model` + `model_options`
   + `reasoning_effort` + `skills`, synthesize the `from`-agent from prior reducer
   state, and push a synthetic `AgentChanged` item with a locally-allocated id (it
   does not arrive from `GET /items` on a later reconnect).
5. The transcript keeps its history with an inline `⇄ agent Y → Z` marker
   (transcript doc §13); the card + composer re-render in place; **the
   transcript does not remount.**

**Constraints:**

- **API floor is `LEVEL_EDIT` (2), NOT owner.** The route calls
  `_require_access_and_level(..., LEVEL_EDIT, ...)` (`sessions.py:14214`);
  docstring: "403 if the caller lacks edit access". Owner-only is a **Lens UI
  policy** (decision J) layered on top — *not* an API constraint. The earlier
  "owner-only verified in source" claim was wrong. Lens disables the menu item
  for `permission_level < 4` as a product choice; the server would accept an
  editor.
- **Idle-only (mostly server-enforced).** **409 if a turn is running** —
  "switching mid-turn would tear the running harness subprocess out from under an
  active stream." The server rejects when `_session_status_from_cache == "running"`,
  and the cache collapses `waiting → running`, so `waiting` is also rejected. But
  `launching` is **not** in the cache value set and falls through to `idle` — the
  server does **not** reject it. So Lens must **client-preflight `launching`**
  (disable set = `{running, waiting, launching}`) before POSTing; the picker
  surfaces a server 409 as "wait for the current turn to finish."
- **Not a sub-agent** (400) and **not a no-op** swap to the agent already bound
  (400). The kebab item is hidden for sub-agent sessions.
- **Runner resources reset.** The switch fires the server's
  `_reset_runner_resources_after_switch` — **open terminals on the session drop
  and must re-attach** (the transcript itself is untouched).
- **Same session, same conversation.** The conversation continues across the
  swap; the new agent inherits the prior turns. This is a feature, not a bug —
  it's how you'd hand a half-finished investigation from a coder agent to a
  reviewer agent.
- **Model continuity.** If the new agent has a different default model, the
  `llm_model` field updates; the user can still override per-session.

---

## 8. The Concierge

A **long-standing chief-of-staff agent** (capability map §0.6). Lives at
`~/.omnigent/agents/concierge.yaml`. Spec'd here; its lifecycle + chrome are
the state model's (§12.3) + the shell's (§13).

- **Spec fields:** defaults to `claude-sdk` harness; carries a long system
  prompt describing the chief-of-staff role; declares tools that are an MCP
  server Lens exposes locally (Bridge Inbox + Bridge Knowledge access).
- **Local-only, on the always-on local server.** The Concierge must run where
  its runner can reach Lens's local Bridge MCP and where Lens controls
  `~/.omnigent/agents/`; that is the **local server, which Lens runs as
  always-on baseline infrastructure** regardless of work-connections (state
  model §12.3; server lifecycle §3, §10).
- **Onboarding:** Lens creates the Concierge's session on first launch if none
  exists; stores its `SessionId` in `meta` and re-attaches on subsequent
  launches (--resume semantics).
- **The MCP tools the Concierge reads** — Lens exposes a local MCP server with
  tools over the two Bridge modes (capability map §0.6): `bridge.inbox.list` /
  `bridge.inbox.act` (read the actionable queue, resolve/defer/reply items) and
  `bridge.knowledge.read` / `bridge.knowledge.write` (Memories + Wiki pages).
  The exact tool schema is a forward spec; the boundary is sketched here, pinned
  when the first build implements it.

---

## 9. Open questions

- **Picker authoring UX** — the spec YAML opens in the editor; but a form-based
  authoring UI (structured fields) is a future call. v1 is "open the YAML,
  edit, save, the picker refreshes."
- **Skill authoring** — `skills/<name>/SKILL.md` bundled in the spec; Lens
  surfaces the list but doesn't author skills in-app. Defer to a future spec.
- **Sub-agent declaration** — `sub_agents` in the spec is a forward field; the
  sub-agent topology document owns how a parent agent's sub-agent declarations
  map to child sessions + the topology UI.
- **Concierge MCP tool schema** — pinned when the first build implements it;
  this document owns the boundary, the build pass pins the JSON.