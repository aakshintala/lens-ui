# Lens — capability map & design language

The keystone document of the Lens spec set. The full map of omnigent's
surface area (pinned at `0.3.0.dev0`), the UX vocabulary Lens commits to, the
cross-cutting decisions that hang over the rest of the specs, and the
verification posture.

**Status:** Draft, 2026-06-23.
**Audience:** Amogh + collaborators. Precise, technical, decision-relevant.
**Ground truth:** `omnigent-ai/omnigent` pinned at `0.3.0.dev0` (HEAD `36b2a11c`) —
`openapi.json` (the typed API surface, vendored at `vendor/omnigent-0.3.0.dev0/`; its `info.version` `"0.1.0"` is stale — trust package semver), `omnigent/{server,host,runner,stores,runtime,entities}/`
(the code), `README.md` (the user-facing contract), `designs/CLI_CONTRACT.md`
+ `docs/POLICIES.md` (the two surviving design docs). The internal
`designs/*.md` design-rationale docs (FRONTEND_SDK_V2,
COMPONENT_RESPONSIBILITIES, AGENTLOOP, etc.) **vanished in the open-sourcing**
and are NOT cited here.

---

## 0.1 How to read this set

The goal is **the complete design path to full omnigent (`0.3.0.dev0`) parity** — every
capability Lens needs to be a first-class native client, nothing on the floor.
This is deliberately **not an MVP plan**. Sequencing is a separate decision made
once the whole map is visible.

Three commitments:

1. **Spec everything; sequence later.** Each subsystem gets a full design spec
   now. Phasing is a separate decision made once the whole map is visible.
2. **Greenfield.** Lens is unconstrained by the existing omnigent UI invariants.
   §0.6 captures Lens's design language — the UX vocabulary the app will have.
3. **omnigent-native, one seam.** One engine, one source of truth (the omnigent
   SSE stream + REST). There is no `Backend` trait, no normalized cross-backend
   contract — the app's domain model *is* the omnigent model, lightly adapted
   for the UI. The seam lives in the typed client crate: it isolates contract
   churn; the rest of the app speaks a canonical Lens item model the
   application architecture & state model document pins.

### Spec set

The spec set uses descriptive filenames (no S-codes, no numbered IDs). The
documents, the capability they describe, and their dependencies:

| Document | Describes | Depends on |
|---|---|---|
| this document (capability map & design language) | omnigent surface → Lens capability map; Lens's UX vocabulary; cross-cutting decisions; verification posture | — |
| `typed-client.md` | the `lens-client` Rust crate — typed REST+SSE+WS client; full event taxonomy; no-replay reconnect; environment-scoped workspace endpoints; presence; terminal WS attach paths | — |
| `app-architecture-and-state-model.md` | how the typed client feeds the view-model, state store, command flow; the Bridge router; presence/co-viewers; switch-agent & fork flows | the typed client |
| `application-shell-and-layout.md` | board/home, focused-session window (chat + collapsible working area beside chat, ⌘D deep-focus), resource-rail navigator, global search/nav/palette, app-state chrome, Bridge (native, one rail destination), theme | the state model |
| `conversation-transcript.md` | the transcript surface, full fidelity; markdown security boundary | the state model |
| `workspace-and-terminals.md` | one-workspace-per-session ↔ task model; env-scoped fs/diff/search/shell; terminal WS attach + transfer | the state model |
| `agent-definition.md` | the net-new vocabulary; 19 harnesses; filesystem YAML; bundle upload clone; live switch-agent | the state model |
| `permissions-and-elicitations.md` | form/URL elicitations; `/resolve`; target_session_id; the four harness permission/elicitation hooks; per-session sharing (levels 1–3) + policy editor + identity | the state model |
| `sub-agent-topology.md` | child-session trees with richer summaries; pending-elicitation badges | the state model |
| `server-lifecycle.md` | Lens-supervised host daemon + server + embedded-runner; managed sandbox hosts; hosts/policies/permissions/fork topology | the typed client |
| `framework.md` | gpui (**resolved/locked**, decision D) vs React/TS (rejected); recon summary + residual spikes | this document |

Dependency shape: **the typed client → the state model → the application
shell**; the surface specs (transcript, workspace, agent definition,
permissions, sub-agent topology) depend on the state model and **dock their
content into the shell's slots**; **the typed client → the server lifecycle**;
**the framework** document is orthogonal.

**Shell vs content split:** layout cuts across capabilities — the
focused-session window governs where the transcript (conversation-transcript
doc) *and* the workspace surfaces (workspace-and-terminals doc) sit — so it
can't live in any one surface document. **The application shell document owns
the *containers and chrome*** (board/home, the focused-session window +
collapsible working area beside chat (⌘D deep-focus), the resource-rail navigator,
global search/nav/palette, app-state chrome, theme); surface documents own the *content that fills the
slots*, written container-agnostic. This split relocates three decisions that
were tentatively parked in the workspace doc into the application shell:
**task↔session** (§0.7-A — it's board grouping), the **focused-session
layout** (§0.7-F), and the **resource-rail navigator surface** (its data model
stays in the workspace doc, the navigator surface is the shell's). Bridge is a
native application-shell surface (§0.6).

---

## 0.2 The platform, and where Lens sits

omnigent separates three roles (verified in 0.2.0 code structure:
`omnigent/server/`, `omnigent/runner/`, `omnigent/host/`):

| Role | Owns | In the Lens world |
|---|---|---|
| **Server** | agents, conversations, files, REST/SSE API, task lifecycle. Zero execution state. | runs locally on the Mac (Lens spawns it); on a dedicated remote host (Lens supervises it); on a managed sandbox (Lens connects to it, the server provisions it); or on a **remote omnigent server Lens did not spawn and only connects to as a client** (e.g. an internal dev workspace) |
| **Runner** | harness subprocesses, OS env, filesystem, terminals (tmux PTYs), MCP, sub-agents, tool resolution. Outbound WS tunnel to the server. | runs wherever the server does (in local mode the server **embeds a runner** — verified `omnigent/cli.py:2382`); on remote hosts the runner is brought online via `omnigent host` |
| **Client** | REST + SSE + WS only — CLI, `ap-web`, Slack, Lens are all just clients | **Lens** — and Lens is a **multi-connection client**: one open Lens instance talks to N servers at once (a local one it spawned + one or more remote ones it only connects to). A session belongs to exactly one connection. |

Resolved lifecycle facts (grounded in 0.2.0):

- **`omnigent server` spawns an embedded runner in local mode** — verified. No
  separate `omnigent host` needed for local. Remote bring-your-own-runner hosts
  still go through `omnigent host` + `POST /v1/hosts/{host_id}/runners` (the
  `LaunchRunnerRequest` endpoint, verified openapi:4244).
- **Runners bind to sessions** via `PATCH /v1/sessions/{id} {runner_id}` —
  atomic bind, fails on offline runner with `INVALID_INPUT`. **Lens never
  spawns or scrapes an agent process.** Thin client: open a session, bind a
  runner, stream events, send messages, call workspace/terminal APIs. Lens's
  only "daemon-like" job is *bootstrapping and supervising* the server +
  runner (the server lifecycle document) — a launch/supervise concern, not an
  execution one.
- **Session provisioning, v0.2.0:** two modes via `host_type` on session create
  (`SessionCreateRequest`, `omnigent/server/schemas.py:1038`):
  - **`"external"` (default)** — bring-your-own-runner, as before. You provide
    `host_id` + `workspace`.
  - **`"managed"` (new in 0.2.0)** — the **server provisions a sandbox host**
    for the session (`host_id` + `workspace` must be null; server picks both;
    background flow). This is new in 0.2.0 — earlier omnigent had no
    server-side provisioning (bring your own runner). Sessions can now run in
    disposable Modal / Daytona / Islo sandboxes (README §"Run agents in cloud
    sandboxes"). The server lifecycle document covers the topology; the shell
    spec's new-session dialog (decision §0.7-A) surfaces the choice.
- **Localhost ⇒ implicit single-user, no auth for the *local* connection.**
  Remote servers use one of the real provider modes — `header`
  (`X-Forwarded-Email`-style, with `OMNIGENT_LOCAL_SINGLE_USER=1` as the
  local fallback) / `oidc` cookie / `accounts` username+password cookie
  (`server/auth.py:15-20`); the provider gates multi-user ownership checks.
  **Lens connects to N servers at once, and the remote ones may require auth
  from day one** — Lens is one user *of* each remote server, **but per decision
  §0.7-E it DOES implement the full multi-user/sharing UI** (grant levels 1–3,
  owner readout, public-read toggle, presence/co-viewers). So Lens's connection
  model has two shapes from v1: **spawn+supervise, no auth** (local Mac) and
  **connect-as-client, with auth** (remote, e.g. an internal dev workspace the
  user reaches from their work laptop). The auth UX is "Lens knows your
  token/cookie for this connection," **and** Lens surfaces the server's
  sharing/permissions/identity as first-class chrome on top of it.

---

## 0.3 Capability inventory (full 0.2.0 surface → Lens)

Legend — Lens build cost relative to "render what the API gives me":
▮sm small (direct render), ▮md medium (real UI work), ▮lg large (net-new
surface). Each row cites the owning document.

### Conversation & turns — owned by `conversation-transcript.md`

| omnigent v0.2.0 | Lens cost | Notes |
|---|---|---|
| Streaming text deltas (`response.output_text.delta`, with `message_id`/`index`/`final` for terminal-observed streaming) | ▮lg | the transcript core surface |
| Reasoning (`response.reasoning.started`, `reasoning_text.delta`, `reasoning_summary_text.delta`, `reasoning` items w/ `encrypted_content`) | ▮lg | collapsible thinking blocks; net-new |
| Tool calls/results (`function_call` / `function_call_output` items) | ▮lg | grouped tool spans with status |
| Native tools (`web_search_call`, `mcp_call`, `code_interpreter_call`, `image_generation_call`, `computer_call`, `file_search_call`) | ▮lg | per-provider opaque rendering; net-new |
| Response lifecycle (`created/queued/in_progress/completed/failed/incomplete/cancelled`) | ▮md | drives the card's status wave (shell §5) |
| **`session.status` is 3-state over REST, 5-state over SSE.** `SessionResponse.status`/`SessionListItem.status` = `idle\|running\|failed` (`schemas.py:1604,1869`); SSE `SessionStatusEvent.status` = `idle\|launching\|running\|waiting\|failed` (`schemas.py:2067`). The server cache collapses `waiting→running`. | ▮md | Active (SSE) cards fold the full 5-state; Slept/Archived (poll-fed) cards see only 3-state — Lens must **persist last fine-grained status** so slept cards don't regress to `idle`, and add `waiting`/`launching` to the card-wave vocabulary (shell §5.1) |
| Per-turn `usage` (input/output/**reasoning**/total/context tokens), `session.usage` event with `usage_by_model` (per-model cumulative cost) + a session `total_cost_usd` (**server-computed USD — Lens needs no price table**) | ▮md | cost surfacing; drives the card/project cumulative spend + the time-windowed global readout (decision §0.7-I) |
| `context_window` + `last_total_tokens` | ▮sm | context meter on the composer (shell §7.5) |
| `session.todos` (content/status/activeForm) | ▮sm | the **agent's own** per-session task list — rendered inline in the chat/transcript (conversation-transcript doc owns the placement). NOT routed through the Bridge; Bridge carries cross-agent communication and longer-term *planning* todos, which are a different concept from the agent's live session todos. |
| Compaction (`compaction.in_progress/completed/failed` + `compaction` item) | ▮md | "summarizing…" + summary marker; net-new |
| Retry (`response.retry`) + structured `response.error` (`source: llm\|execution\|tool`, code, message) | ▮md | typed error rendering at three altitudes (shell §14.3) |
| Slash-command / terminal-command items (`/skill`, `!cmd`) | ▮md | typed transcript items |
| Interrupt / fork / compact / switch-agent (POST events / PATCH) | ▮md | turn controls; **switch-agent is 0.2.0 net-new** (`POST /v1/sessions/{id}/switch-agent` fires `session.agent_changed`; `PUT /agent` is bundle storage) — decision §0.7-J |
| `session.heartbeat` / `response.heartbeat` (`last_event_seq` + `sequence_number`) | ▮sm | liveness + gap detection (the typed client); basis for no-replay reconnect |
| **8 new `session.*` chrome events (0.2.0):** `model_options`, `reasoning_effort`, `collaboration_mode`, `agent_changed`, `terminal_pending`, `sandbox_status`, `skills`, `presence` | ▮md | feed composer controls + card chrome + co-viewer indicators + sandbox badge + skills roster; additive |
| `OutputItemDoneEvent` (full per-item lifecycle) + `OutputFileDoneEvent` | ▮md | drives structured item rendering + file-attachment surface |

### Workspace, environments, resources, terminals — owned by `workspace-and-terminals.md`

**0.2.0 path shape:** all workspace endpoints are **environment-scoped** under
`/v1/sessions/{id}/resources/environments/{environment_id}/…`. omnigent models
**one primary environment per session** (`"default"`), with optional
terminal-scoped envs. This is why the workspace endpoints are environment-scoped.

| omnigent v0.2.0 | Lens cost | Notes |
|---|---|---|
| **One primary env per session (`"default"`) + optional terminal-scoped envs** | ▮lg | **decides Lens's task↔session model** (§0.7-A); multi-env is for terminals only, not multi-worktree |
| Git worktree created server-side (`git{branch_name (req), base_branch?}` + `host_id` on `SessionCreateRequest`; **transparent under `host_type:"managed"`**) | ▮md | session-creation flow; managed-sandbox option is part of the dialog (shell §7.6) |
| Filesystem read / write / edit (PATCH old/new or batch) / delete — env-scoped | ▮md | file tree + editor |
| `changes` (flat list: path/status created\|modified\|deleted) — env-scoped `GET …/environments/{env_id}/changes` | ▮sm | changed-files tray (shell §11) + the Review tab |
| `diff/{relative_path}` → `{before, after}` strings (**not** unified diff) — env-scoped | ▮md | Lens computes hunks client-side (proven by `imara-diff` impl in a reference GPUI app; framework recon) |
| `search` (substring + glob include/exclude, cap 500) — env-scoped `POST …/environments/{env_id}/search` | ▮md | fuzzy finder (no client index — server-side) |
| `shell` (POST command → stdout/stderr/exit/cwd, one-shot) — env-scoped | ▮sm | quick-command surface (not the terminal pane) |
| File resources (`POST /resources/files` upload, `GET …/files` list, `GET …/files/{id}`, `GET …/files/{id}/content`) | ▮md | attachments/uploads + multimodal input surface |
| **`SessionResourceObject`** (`env\|terminal\|file`) — the typed union over `GET /v1/sessions/{id}/resources` + `GET /v1/sessions/{id}/resources/{resource_id}` | ▮md | the **resource model** = workspace-and-terminals doc; the **resource-rail navigator surface** = the application shell doc (per the shell-vs-content split — the shell owns the navigator UI, the workspace doc owns the data model) |
| **Terminals:** `WS /v1/sessions/{id}/resources/terminals/{terminal_id}/attach` (**the `/v1` prefix IS required** — router mounted with `prefix="/v1"` at `app.py:1635-1642`; binary PTY frames + text `{"type":"resize"}` control + `read_only` query) — read-only by default (`tmux attach -r`), owner-level write attach. **NEW `POST …/terminals/{id}/transfer`** moves a terminal to another session without closing it (live `/clear` rotation). **NEW `DELETE …/terminals/{id}`**. No replay buffer — live attach only. `session.terminal.activity` + new `session.terminal_pending` events. | ▮lg | the terminal surface; reconnect loses scrollback (Lens-side ring buffer per decision §0.7-C); transfer is a new affordance |

### Agent definition — owned by `agent-definition.md`

| omnigent v0.2.0 | Lens cost | Notes |
|---|---|---|
| **Declarative agent spec** (`~/.omnigent/agents/*.yaml`): name, prompt, executor{harness, model, auth, context_window}, os_env{sandbox}, terminals, tools{mcp}, policies/guardrails, skills, sub_agents, compaction, params | ▮lg | biggest net-new vocabulary; **directory renamed `~/.omniagents` → `~/.omnigent`** (legacy fallback still reads the old dir) |
| **Agent registry — read-only over REST.** `GET /v1/agents` lists; no `POST/PUT/DELETE` on the HTTP surface. Spec authoring is **filesystem YAML + bundle upload** — bundled uploads create **session-scoped agent clones** via multipart `POST /v1/sessions` (`SessionCreateMetadata`) and `PUT /v1/sessions/{id}/agent` (`Body_update_session_agent_v1_sessions__session_id__agent_put`) | ▮lg | picker is table-stakes; authoring/editing is richer; bundle upload = "draft an agent, spin up a session to test it"; **live switch-agent** rebinds a running session (§0.7-J) |
| **Harnesses (19 canonical, `OMNIGENT_HARNESSES` `spec/_omnigent_compat.py:80-101`):** `antigravity`, `antigravity-native`, `claude-native`, `claude-sdk`, `codex`, `codex-native`, `copilot`, `cursor`, `cursor-native`, `goose`, `goose-native`, `hermes`, `openai-agents`, `open-responses`, `opencode-native`, `pi`, `pi-native`, `qwen`, `qwen-native`. `open-responses` IS canonical (adapter-routed); `hermes`/`goose` are canonical (the old "16" list wrongly dropped them). Aliases (`claude`, `opencode`, `github-copilot`, …) live in `OMNIGENT_HARNESS_ALIASES`. | ▮md | the picker enumerates the 19. `AgentObject.harness` is now a **free `string\|null`** (no openapi enum) — the typed client's hand-written list is a picker/validation aid verified against `OMNIGENT_HARNESSES`, not a wire enum |
| `AgentObject` schema (openapi:44): `{id, name, version, description, harness, mcp_servers, policies, skills, terminals}` — harness is now an explicit discriminant the UI reads without name-hardcoding | ▮md | drives the agent-picker badge (which icon, which "kind") |
| Model override (per-session) + reasoning effort (`none…max`) + skills — all per-session controls surfaced in the composer (shell §7.5) | ▮md | per-session controls |
| **Live switch-agent on a running session:** `POST /v1/sessions/{id}/switch-agent` (the trigger, `sessions.py:14214`; `PUT /agent` only stores the bundle, fires nothing); `session.agent_changed` (only `agent_id`+`agent_name`, no model/skills) follows. **API floor = `LEVEL_EDIT` (2), NOT owner.** Idle guard rejects cached `running`/`waiting` but **not `launching`**. 409 if a turn is running; rejected on sub-agent sessions and no-op swaps | ▮lg | the UI handoff flow (§0.7-J): transcript stays (no remount), card + composer re-render. Owner-only + `launching`-preflight are **Lens UI policy** layered over the edit-level API |

### Permissions, elicitations, sharing — owned by `permissions-and-elicitations.md`

| omnigent v0.2.0 | Lens cost | Notes |
|---|---|---|
| Elicitations: `response.elicitation_request` with `ElicitationRequestParams {mode: form\|url, message, requestedSchema, url, phase, policy_name, content_preview, **target_session_id**}` — `target_session_id` carries child→ancestor mirror routing (`None` = resolve against current session). **url-mode `url` is currently a RELATIVE `/approve/{session_id}/{elicitation_id}` page** (`approval.py:209`), not external OAuth — Lens validates scheme/origin and resolves against `base_url`. **`pending_elicitations` on the snapshot is a `list`** (plural — fan-out parents mirror multiple) | ▮lg | the permission path; form/url; the mirror case matters for the sub-agent topology |
| Reply — two paths: (a) **`POST /v1/sessions/{id}/events`** with `type=="approval"` and `ElicitationResult {action: accept\|decline\|cancel, content?}` (confirmed); (b) **NEW** `POST /v1/sessions/{id}/elicitations/{elicitation_id}/resolve` (RESTful, body `ElicitationResult`) — cleaner for url-mode OAuth | ▮sm | the Bridge action-queue primary surface |
| `response.elicitation_resolved` event — server-side timeout/cancel/turn-end clears the prompt without a verdict; the Bridge badge decrements in lockstep (idempotent) | ▮sm | the Bridge must subscribe; sticky badge bug otherwise |
| `GET /v1/sessions/{id}/elicitations/{elicitation_id}` — fetch pending state for a standalone approval page (deep-linkable) | ▮sm | deep-link from notification → specific approval |
| Native permission hooks — **four**, all server-initiated: `POST /v1/sessions/{id}/hooks/permission-request` (generic/claude-native), `/hooks/codex-elicitation-request` (codex), `/hooks/antigravity-elicitation-request` (`openapi.json:5739`), `/hooks/cursor-permission-request` (`openapi.json:5821`) | ▮md | fed into the same elicitation UI; the `external_elicitation_resolved` race handling must tolerate all four sources |
| Policies: server-wide (`GET/POST/PATCH/DELETE /v1/policies`, `GET /v1/policy-registry`), per-session (`GET/POST /v1/sessions/{id}/policies`, dry-run **`POST /v1/sessions/{id}/policies/evaluate`**) — stack server→agent→session, stricter-first | ▮md | the policy editor surface — browse the catalog, attach to a session/agent/server, evaluate, see results |
| Sharing: `PUT /v1/sessions/{id}/permissions {user_id, level}` — **grantable levels 1–3 only** (1=read, 2=edit, 3=manage; `Field(ge=1, le=3)`, `schemas.py:1905`); owner (4) is creation/admin-derived and **not grantable** (owner grants 403); `GET /v1/sessions/{id}/owner`; `__public__` capped at read | ▮lg | net-new; **full scope — omnigent supports sharing natively, Lens surfaces it** (sharing dialog with read/edit/manage, owner readout, public-read toggle); share-link requires ≥ manage(3) |
| Identity: `GET /v1/me` returns the current user; ownership drives "you don't own this session" affordances when connecting to an authed remote server | ▮sm | the multi-connection identity surface |
| Session labels (`/v1/sessions/{id}/labels`) — free-form tagging | ▮sm | board/card grouping; integrates with boards |

### Sub-agent topology — owned by `sub-agent-topology.md`

| omnigent v0.2.0 | Lens cost | Notes |
|---|---|---|
| **`ChildSessionSummary`** (`omnigent/server/schemas.py:558`) — parent_session_id, title, tool, session_name, kind="sub_agent", created_at/updated_at, agent_id, agent_name, current_task_id, current_task_status, busy, labels, last_task_error, last_message_preview, **pending_elicitations_count**. Powers `GET /v1/sessions/{id}/child_sessions` (openapi:5154). | ▮lg | the multi-agent model — greenfield in Lens. The richer summary (esp. `pending_elicitations_count`) feeds child-card badges. **Caveat:** `ChildSessionSummary` is **not exposed as a named schema in `openapi.json` components** — only the event `SessionChildSessionUpdatedEvent` is; codegen off openapi won't get a named type. Add to the typed client's contract-test list |
| Children are real sessions → have their own `/stream` (independent child SSE streams — confirmed by construction) | ▮md | deep-focus into a child opens its own focused-session window |
| `session.child_session.updated` carries the full summary; `session.created` (child variant) handles live incremental creation | ▮md | the parent's activity line (shell §5.2) surfaces short-lived children |

### Server/runner lifecycle — owned by `server-lifecycle.md`

| omnigent v0.2.0 | Lens cost | Notes |
|---|---|---|
| **Server + embedded runner (local mode):** Lens spawns/supervises `omnigent server` as a child process on the Mac. Embedded runner handles everything | ▮md | Lens's "daemon-like" job is launch/supervise, never execute |
| **Hosts registry:** `GET /v1/hosts`, `GET /v1/hosts/{id}` (**read-only — there is NO `POST`/`DELETE /v1/hosts`**; host registration is outbound-WS-tunnel/daemon based, `omnigent host`/`host_tunnel.py`), `POST /v1/hosts/{id}/directories` (create a folder), `GET /v1/hosts/{id}/filesystem[/{path}]`, `POST /v1/hosts/{id}/runners` (the launch primitive, `LaunchRunnerRequest{session_id, workspace, git?}`) | ▮md | topology editors in the server lifecycle spec; the per-host filesystem browse is new and useful for new-session repo-picking. Policy dry-run is `POST /v1/sessions/{id}/policies/evaluate`; batch host liveness via `GET /health` |
| **Managed sandbox hosts (0.2.0 net-new):** `host_type:"managed"` on session create triggers server-side sandbox provisioning (Modal/Daytona/Islo per README); `session.sandbox_status` event carries state | ▮md | the new-session dialog picks external-vs-managed; the server lifecycle spec supervises nothing for managed (the server does it) |
| WS tunnels + host tunnels for remote runner launch; stable, token-bound runner ids | ▮md | Lens-supervised remote-host case |
| `GET /api/version` (`{"version": "<semver>"}`, `app.py:1479`) — **the contract-version gate source**; `GET /v1/info` (unauthenticated capability/auth probe: `accounts_enabled, login_url, needs_setup, …` — **no version**); `GET /health` (liveness); `GET /v1/me` (auth identity) | ▮sm | the version check Lens uses to refuse-to-start on contract mismatch is `/api/version`, NOT `/v1/info`. Ready ladder: `/health` → `/api/version` → `/v1/info` |
| Auth: localhost = no auth; beyond — `X-Forwarded-Email` / OIDC; `OMNIGENT_AUTH_ENABLED` for multi-user ownership | ▮md | the connection-auth + identity surface (the server lifecycle spec owns the connect flow; the permissions spec surfaces identity/ownership when `OMNIGENT_AUTH_ENABLED` is on) |

### Cross-session orchestration primitives (boundaries depend on what's being orchestrated)

| omnigent v0.2.0 | Lens cost | Notes |
|---|---|---|
| `POST /v1/sessions/{source_id}/fork` (`SessionForkRequest`) — clone a conversation onto a new session, continue independently from the fork point | ▮md | the fork affordance on the card kebab (shell §5.3) |
| `POST /v1/sessions/{id}/comments` (line comments, with `anchor_content` for re-anchoring after edits) — `AddCommentRequest{path, start_index, end_index, anchor_content?, body}` | ▮md | the annotation engine (shell §13) |
| `POST /v1/sessions/{id}/comments/send` + `PATCH/DELETE /v1/sessions/{id}/comments/{comment_id}` — comment-send to agent + edit/withdraw | ▮md | routed comment feedback to the agent; supports the Review → send-to-agent flow |
| `session.presence` events — co-viewers of a shared session | ▮md | live co-viewer chrome in the focused-session header (shell §7.4) |

---

## 0.4 The streaming contract (forces the typed client's design)

omnigent SSE is **live-tail, no-replay**: `GET /v1/sessions/{id}/stream` does
not buffer past events. Correct reconnect = **snapshot (`GET /v1/sessions/{id}`)
+ history (`GET …/items`) + re-open stream + client-side dedup by
`sequence_number`**; the snapshot tells you what was already committed.
Heartbeats carry `last_event_seq` (response.heartbeat) and `sequence_number`
(both heartbeat kinds) so the client can detect a stalled producer / a gap —
on a gap, drop the in-progress accumulators and resync. **There is no global
event stream — only per-session SSE [verified 0.2.0].** Fleet status comes
from polling `GET /v1/sessions` (with cursor params `after/before`,
`kind=default|sub_agent|any`, and filters). This split is what lets the
dashboard show 50 cards while only the genuinely-active ones stream — the rest
are Slept or Archived (the stream count self-bounds via ~10-min auto-sleep; no
hard cap; see the state model doc's lifecycle model §3).

**`sequence_number`** (optional int) is on every event for dedup; pair it with
`last_event_seq` to detect gaps (events emitted while you were disconnected).

**Event families the typed client must parse** (full taxonomy lives there; here
is the inventory):

- `session.*` — status, input.consumed, interrupted, created, heartbeat,
  usage, model, todos, resource.created, resource.deleted, child_session.updated,
  changed_files.invalidated, terminal.activity. **8 new since v0.1.0:**
  model_options, reasoning_effort, collaboration_mode, agent_changed,
  terminal_pending, sandbox_status, skills, presence.
- `response.*` — created/queued/in_progress/completed/failed/incomplete/cancelled,
  output_text.delta, reasoning.started/reasoning_text.delta/reasoning_summary_text.delta,
  output_item.done, output_file.done, elicitation_request, elicitation_resolved,
  retry, error, compaction.in_progress/completed/failed, heartbeat,
  client_task.cancel.
- `turn.*` — started/completed/failed/cancelled (optional, not universally
  emitted).

**`POST /v1/sessions/{id}/events`** body is **generalized to a discriminated
`SessionEventInput`** in 0.2.0 — `type` is a string discriminator and `data`
is a free-form dict carrying the type-specific payload. So the typed client
cannot model each dispatch (`approval`, `interrupt`, `fork`, etc.) as a distinct
request schema; it must serialize a typed Rust enum into `data`. This is a
different shape than v0.1.0 and a load-bearing constraint for the client's
design.

**Persisted-vs-transient classification (which survive reconnect, which are
pure live observations) lives in the typed client document — this is a
load-bearing delegation.** The state model and every surface document that
reasons about reconnect (transcript, workspace, sub-agent topology) relies on
this classification being authoritative there. Adding a new event type
requires updating the classification in lockstep.

---

## 0.5 Lens's domain model (sketch — pinned by the state model)

With no seam, Lens's internal model is the omnigent model adapted for the UI —
not a normalization across backends. The state model document pins it; sketch:

- **`Session`** — id, agent_id, status, llm_model/model_override,
  context_window, last_total_tokens, cumulative cost (rolled up via
  `usage_by_model`), title, labels, runner_id, reasoning_effort,
  collaboration_mode, workspace, git_branch, host_type (external|managed),
  host_id, parent_session_id, external_session_id, pending-elicitation state,
  presence (co-viewers), skills.
- **`Item`** — typed union mirroring omnigent conversation items:
  `message{role, content blocks}`, `function_call`, `function_call_output`,
  `reasoning{summary, encrypted?}`, `native_tool{kind, data}`,
  `compaction{summary, token_count}`, `slash_command`, `terminal_command`,
  `resource_event`.
- **`StreamUpdate`** — the parsed-and-reduced form of the event taxonomy (§0.4)
  the UI subscribes to.
- **`Elicitation`** — pending request state (id, params, target_session_id,
  received_at), with `ElicitationResult` for the reply.
- **`ChildSession`** — a `ChildSessionSummary` mirror; the parent's view of a
  sub-agent.
- **`Workspace`/`Terminal`/`AgentSpec`/`Policy`** — thin Rust mirrors of the
  API models, owned by their surface documents (workspace & terminals, agent
  definition, permissions).

The reduction from raw SSE → `StreamUpdate` → app state lives in the state
model document; the per-surface rendering of that state lives in the surface
documents (transcript, workspace, agent definition, permissions, sub-agent
topology).

---

## 0.6 Lens's design language

The UX vocabulary Lens commits to — the surfaces, primitives, and interaction
patterns the app will have. These are design decisions, not inherited baggage.

### Core surfaces

> Ownership: surfaces tagged **[shell]** are spec'd by
  `application-shell-and-layout.md`; tagged **[state]** by
  `app-architecture-and-state-model.md`; tagged **[surface]** by the named
  surface document. The split follows the shell-vs-content rule in §0.1.

- **Board-of-cards** **[shell]** — agent sessions as cards on a board; the home
  is the active board. Groups (named projects) with **colored borders and a
  faint color-matched body tint** (the body shade lower-opacity than the border)
  contain nested cards. Lens generalizes to **recursive boards** (shell §4.2) —
  groups nest arbitrarily, not just one level.
- **Fleet** **[state]** — the abstract collection of agents/sessions you're supervising
  across all connections. Never a surface; "agents in the fleet", "fleet
  status", "across the fleet". The board is *how you look at* the fleet; the
  fleet is *what's there*.
- **Per-agent side pane** **[shell]** — a focused session gets a side pane (Files / Diff
  Review / Terminal / Canvas tabs). **Canvas** **[shell]** is the drawing surface an agent
  can present visual elements in (diagrams, recorded interactions, custom
  visualisations). Lens adapts to **a working area with tabs/splits + a chat
  column** (shell §7.2, §8).
- **Group as the organizing unit** **[shell]** — a grouping of related agent work. Resolved
  in §0.7-A: **one session = one unit of work** (= one worktree), and the board's
  recursive **Group** clusters related sessions. There is **no first-class "Task"
  entity**; the word "task" is retired in Lens (it collides with omnigent's
  turn-level "task" and the agent's `session.todos`) — use *turn* / *todos* / *Group*.
- **Bridge** **[state + shell]** — the collapsed single rail destination,
  with three sub-panes serving distinct modes:
  - **Inbox** (action-oriented) — the fleet-wide actionable queue: pending
    elicitations from any agent (Allow/Deny/Cancel verbs), agent-to-agent
    relays, routed comments, deferred notes, and *planning* todos (a distinct
    concept from an agent's live `session.todos`, which render inline in that
    session's chat). The Bridge router is Lens-side, built on omnigent's
    per-session comments + labels + elicitation aggregation; omnigent has no
    cross-session "messages" object, so agent-to-agent relay + planning-todos
    routing are Lens-side fabric.
  - **Log** (read-only) — a chronological session log with day/week/month
    rollup summaries. The human-readable "what happened" record.
  - **Knowledge** (read-oriented) — settled facts (Memories) + long-form
    pages (Wiki). Per-session/per-project knowledge capture; written by the
    Concierge when an Inbox item earns a memory, or authored directly.
  Placement decided in §0.7-H; ⌘I jumps to the next-needs-input agent
  (primary), ⌘⇧I opens Bridge Inbox (secondary); ⌘⇧N captures into Inbox.
- **Concierge** **[agent-definition]** — a long-standing agent that acts as the user's chief-of-staff:
  triages the Bridge Inbox, routes deferred items, files knowledge into
  Bridge Knowledge, and orchestrates cross-session follow-ups. The Concierge is a
  first-class agent (configured via `~/.omnigent/agents/`) with a stable
  session that persists across Lens restarts.
- **Canvas** **[shell]** — a drawing surface an agent can present visual elements in
  (diagrams, recorded interactions, custom visualisations). A side-pane tab
  per session. The word "canvas" is reserved for this surface; the board/fleet
  overview is the *board*, never the canvas.

### Core primitives

- **Typed identities over stringly-typed primitives** — branded ids,
  discriminated unions. A string-compare bug is a type-system fix, not a
  runtime check.
- **Design tokens for theming** — a gpui `Theme` struct (shell §15), semantic
  tokens not raw hex. Default theme is dark-first; light shipped via the same
  tokens. Compact density throughout. Sequencing (which themes ship first) is a
  shell-spec call, not a decision here.
- **Unified navigation primitive** — one `navigateToSession` funnel; no
  component mutates focus state directly.
- **Status lanes + cost surfacing** — fed by omnigent SSE + `usage_by_model` +
  the server's `total_cost_usd`. Cost is two-axis (decision §0.7-I):
  **cumulative** per-card/per-project (server-computed USD) and a **time-windowed**
  global readout (today / 7d / 30d, Lens-computed from a cost-sample series). The
  card wave encodes urgency on a corrected ladder (shell §5.1).
- **Derived, authoritative status** — status is folded from events into a
  per-session state machine; "needs attention" is sticky until a real user
  action clears it.
- **Auto-sleep quiet agents** — after a session has genuinely gone quiet
  (idle, no terminal activity) for a period (default ~10 min), Lens **sleeps**
  it: `stop_session` reclaims the server-side harness/PTY *and* the card dims
  (stays visible). Auto-sleep skips pinned sessions and sessions with a pending
  elicitation, and is terminal-aware (a live terminal counts as not-quiet).
  Wake = resume + re-bind a runner (the §0.7 lifecycle; state model §3).

### Power-user keyboard model

- **⌘I — Jump to next agent that needs input** — the control-room primary
  navigation primitive; routing through pending elicitations across the fleet.
  The keyboard entry into the Bridge (§0.7-H).
- **⌘D — Deep-focus** — hide the chat column AND maximise the side pane
  (board hidden); second press restores both. A real workflow mode for
  review-heavy supervision.
- **⌘\ — Toggle side pane**, **⌘[ / ⌘] — Previous / next side-pane tab**,
  **⌘s\ — Maximize / restore side pane**.
- **⌘1–⌘9 — Jump to agent/card N on the active board (positional). Frequent.** Board-switch is *not* on ⌘1-3 — it competes with card-jump and loses on frequency (you switch cards many times per session, boards a few times per day).
- **⌘⇧1–⌘⇧9 — Switch to board N** (resolved). Separate modifier from card-jump so the two never compete; board-switch is also reachable via ⌘K.
- **⌘L — Send selection to agent (@path:start-end)** — the editor→chat
  citation primitive.
- **⌘⇧C — Concierge panel** — pops the **floating Concierge chat panel**
  (transcript + mini-composer; transient by default — Enter posts, Esc returns
  focus to your work — or **📌-pinned** to stay floating alongside your focused
  session). The chief-of-staff surface you see *with* your other work (shell §13).
- **⌘K — Command palette** (agents + board-switch + actions).
- **⌘N — New main window** (multi-monitor), **⌘W — Close window** (Lens stays
  **resident** in the menu bar — shell §17.4), **⌘Q — Quit** (fully exits;
  stops background notifications).
- **⌘P — Fuzzy file open**, **⌘F — Project content search**, **⌘⇧P — Global
  palette**.
- **Editor/Review/Terminal tab shortcuts** — `` ^` `` toggle an interactive
  terminal in the focused session (scoped to its env/worktree), `j/k` prev/next
  changed file, `n/p` prev/next hunk, `^⇧[/^⇧]` prev/next shell.

### Net-new (no precedent informing the design)

Declarative agent specs + registry + picker (agent definition), the 16-harness
matrix (agent definition), sub-agent trees (sub-agent topology), form/URL
elicitations + the `/resolve` REST path (permissions), sharing/multi-user
(permissions), full-fidelity conversation rendering incl.
reasoning/compaction/native-tools (transcript), managed-sandbox provisioning +
the hosts registry (server lifecycle), fork (`POST /fork`), **live
switch-agent on a running session** (`POST /switch-agent` + `session.agent_changed`,
§0.7-J), policy editor, presence/co-viewer chrome, session labels.

---

## 0.7 Cross-cutting decisions

Each is a **decision**; its owning document carries the options + the
recommendation. **All are now resolved** (2026-06-24 grilling pass ratified the
leanings against the omnigent 0.2.0 source and reshaped the lifecycle model;
A–J below carry the resolutions). Three (H, I, J) were surfaced in the
brainstorm + reconciliation pass.

**A. Task ↔ session, given one-workspace-per-session. (owned by the
application shell)** omnigent has no multi-worktree session. The native option is
(ii) **task = session** (drop the multi-worktree concept; simplest, most native;
the board provides the "many related sessions" grouping). **0.2.0's
`host_type:"managed"` subtly expands the choice space:** a "task" might now span
a parent session + N managed-sandbox child sessions, each in its own worktree —
i.e. *orchestration-level* multi-worktree (Polly-style), just not
*session-level*. So options become: (i) a Lens "task" groups **N sessions** (one
per worktree/branch), restoring a multi-worktree feel at the grouping layer + a
cross-session fuzzy/search story; (ii) **task = session** (drop the
multi-worktree concept; simplest, most native; the board groups); (iii) hybrid.
**Resolved: (ii).** One session = one unit of work (= one worktree); the
board's recursive **Group** is the grouping layer — it already spans N sessions
across N worktrees, so **no first-class "Task" entity** is built. The multi-root
file tree shows the **focused session's worktree only by default**; sibling
worktrees in the Group are opt-in (auto-showing them invites "which branch am I
editing?" mistakes). **"Task" is retired as a Lens term** (it collides with
omnigent's turn-level task + the agent's `session.todos`). Genuine
multi-worktree-in-one-session is an omnigent change or a faked parent directory
of worktrees, not Lens's to simulate. Owned by the application shell.

**B. Sub-agent tree model. (owned by the sub-agent topology)** Greenfield.
0.2.0's rich `ChildSessionSummary` (with `pending_elicitations_count`,
`current_task_id`, `last_message_preview`, `agent_name`, `busy`) raises the
question of how much of the child is surfaced on the parent card before
drill-in. Options: render a sub-agent **rail/tree** under the parent card with
navigation into each child's stream; or surface children as an inspectable list
only (the activity-line line + tray); or hybrid (summary list on the card,
drill-in opens child's own focused-session view). **Resolved: hybrid,
tray-segment home.** When focused on a parent, the sub-agent tree lives in a
**"Sub-agents" segment in the volatile tray** above the composer (consistent
with Tasks/Changes/Terminals); the unfocused card shows a compact rollup on its
activity line; the in-transcript spawn span (transcript §8.6) is the
in-conversation record; drill-in opens the child's own window with a breadcrumb.
**Children never become top-level board cards.** Big/deep trees escalate to a
popover/side-pane. The tree/rail visual docks into the shell; the sub-agent
topology document owns the semantics.

**C. Terminal model. (owned by workspace & terminals)** tmux-PTY-over-WS attach
(path: `/v1/sessions/{id}/resources/terminals/{id}/attach` — the `/v1` prefix IS
required, from the router mount at `app.py:1635-1642`),
read-only default, transferable to a new session, no replay buffer. Reconnect
loses scrollback. Options: (i) Lens-side ring buffer for reconnect scrollback
(purely UI-state, no server-side buffer to lean on); (ii) accept
reconnect-from-blank + a visible "history unavailable" affordance; (iii)
hybrid (Lens-side buffer, best-effort beyond it). Also: do shells and
agent-terminals share one surface or render differently? **Resolved: (i)** — the
Lens-side ring buffer covers brief reconnects; a deliberate long **Sleep**
reclaims the PTY (§0.7 lifecycle), so its scrollback is gone by design (terminal-
aware auto-sleep avoids dropping a terminal you were watching). Shells and
agent-terminals share one surface, distinguished by the `kind` label.

**D. Framework. (owned by framework)** **Resolved: gpui.** Greenfield removes
all migration cost, the all-Rust win (the typed client's types flow straight
into the UI, no IPC) is unopposed, and the GPUI reconnaissance spike retired
most widget risk via Arbor / Paneflow / gpui-flow references. The
Bridge-webview risk is gone (rebuild native). The only remaining spike item is
markdown rendering (Paneflow forked GPUI for it). Decision locked; the framework
document owns the residual.

**E. Auth & multi-user posture. (owned by permissions + server lifecycle)**
**Resolved.** omnigent supports multi-user, sharing, and per-session
permissions natively — Lens surfaces them as first-class (not deferred).

- **Connecting to an authed remote server — in scope.** Lens is a
  multi-connection client; one of the connections may be a remote omnigent
  server (e.g. an internal dev workspace) that requires `X-Forwarded-Email` /
  an OIDC cookie / a bearer token. Lens stores the credential per-connection
  and presents it on requests.
- **Sharing/multi-user UI — in scope.** The `PUT /permissions`, owner
  readout, `__public__` toggle, and the per-session/per-server policy editor
  all surface in the permissions document. Identity (`GET /v1/me`) and
  ownership inform chrome (e.g. "you don't own this session" affordances).
  Don't bake "owner = me" into the domain model — a session may be owned by a
  teammate on a shared remote server.

The server lifecycle document owns the connection-auth model (local spawn = no
auth, remote connect = stored credential); the permissions document owns the
permissions/sharing/policy-editor UI; the application shell surfaces identity +
ownership affordances (the "shared" indicator on a card, owner in the session
header).

**F. Focused-session window layout. (owned by the application shell)** How the
workspace surfaces compose with the transcript. Options considered: tabs that
*replace* chat vs. a **collapsible working area that shrinks chat**
(terminal/editor/review dock beside the live transcript). *Rationale: the working
area beside chat — you watch the agent stream while reviewing its diff / driving a
terminal.* **⌘D deep-focus**
mode (hide chat, maximise side pane) is a third state this layout must support.
**Resolved: the collapsible working area + chat column** (shell §7); ⌘D
deep-focus hides chat + boards and maximizes the working area.

**G. Multi-window posture. (owned by the application shell)** One window
(board + focused side pane) vs. detachable per-session windows (⌘N for
multi-monitor + ⤢ detach to its own window). **Resolved: both** — gpui is
multi-window native; any destination or session can ⤢ detach to its own window
(shell §3, §7).

**H. Bridge — the collapsed surface. (owned by state model + application
shell)** **Resolved.** Bridge collapses the fleet-wide actionable queue and the
knowledge notebook into one **Bridge** rail destination with three sub-panes:
- **Inbox** — the fleet-wide actionable queue (pending elicitations with
  Allow/Deny/Cancel verbs, agent-to-agent relays, planning todos, deferred
  notes, ⌘⇧N quick-captures). Backs ⌘I (jump-to-next-agent, primary) and
  ⌘⇧I (open Inbox, secondary).
- **Log** — chronological session log with day/week/month rollup summaries
  (the human-readable "what happened" record).
- **Knowledge** — settled facts (Memories) + long-form pages (Wiki).
  Authored by the user or filed by the Concierge.

The routing fabric is a Lens-side service built on omnigent's per-session
comments + labels + elicitation aggregation, not something omnigent gives
directly. The Concierge triages Inbox, files to Knowledge, orchestrates
cross-session follow-ups. **Bridge Inbox UI — resolved** (2026-06-24 mockup
pass, `docs/design/renders/bridge-inbox.html`): a pinned **"Needs you"** band
(pending elicitations, matching ⌘I's act-now semantics) above a reverse-chron
stream (relays · planning todos · notes); item card = `kind · from→to ·
status · body · actions`; the filter chips slice both zones. The data + router
+ verbs are pinned here and in the state model.

**I. Spend readout. (owned by state model + application shell)** omnigent's
`session.usage` carries `usage_by_model` with a per-model `total_cost_usd` plus
a session `total_cost_usd`, summed over the session subtree (server-computed
USD — Lens needs no price table). **Resolved: two-axis.** **Cumulative** per-card and
per-project (Group) reads the server's `total_cost_usd` directly (exact,
available for Active *and* slept cards via the list poll). **Time-windowed**
global (today / 7d / 30d) is **Lens-computed** from a persisted cost-sample
series (`cost_samples` table — sample each session's cumulative `total_cost_usd`
on usage events / the list poll, difference per window), since the server's
per-owner daily rollup (`user_daily_cost`) is internal and exposed by no REST
endpoint. Caveat: Lens-computed windows count only spend Lens observed; a jump
that happened while Lens was closed lands on the next-observed day. "Today" =
local calendar day; 7d/30d rolling. Derivation is the state model's; the
surface is the shell's.

**J. Live switch-agent handoff. (owned by agent definition + application
shell)** `POST /v1/sessions/{id}/switch-agent` (`sessions.py:14214`) swaps the
agent spec on a running session (`PUT /agent` only stores the bundle, fires
nothing); `session.agent_changed` fires (carrying only `agent_id`+`agent_name`,
no model/skills — refetch the snapshot). The transcript continues across the swap
(it's the same session). **Resolved:** card + composer **re-render in place**;
transcript **stays (no remount)** with a `⇄ agent Y → Z` marker (the `from` is
synthesized from prior reducer state); the reducer keeps prior items' agent
attribution and tags post-swap items with the new agent. **Guards — corrected
grounding:** the **API floor is `LEVEL_EDIT` (2), NOT owner** (the earlier
"owner-only verified in source" was wrong). The server idle-guard rejects cached
`running`/`waiting` but **not `launching`**. So **owner-only + idle-only is a
Lens UI policy** stricter than the API: the card kebab disables "Switch agent ▸"
for non-owners and while busy (including a client-preflight of `launching`), and
hides it for sub-agents — 409 is the server's fallback. The switch resets runner
resources (open terminals re-attach). The agent definition document owns the
mechanism; the application shell owns the visual handoff.

---

## 0.8 Verification posture (whole set)

- **Mac-local is the primary development target.** Build + test Lens against a
  **local `omnigent server`** on the Mac (embedded runner). The 2026-06-22
  contract check verified the comments / elicitation / events surface against
  0.2.0 `openapi.json`. Re-verify on each lock-step omnigent release.
- **Remote-as-client is a first-class case.** Lens connects (with auth) to
  remote omnigent servers it didn't spawn — e.g. an internal dev workspace the
  user reaches from their work laptop. The connect-as-client path is validated
  by pointing Lens at a real remote server early; it doesn't wait for the
  server lifecycle doc's supervise-remote-host story.
- **Supervised remote hosts are a test step, not a phase gate.** The
  remote-supervise pieces — Lens supervising the server+runner on a dedicated
  host, SSE/WS longevity over the tunnel — are validated by flipping that host
  on.
- **Managed sandbox (Modal/Daytona/Islo)** — the new `host_type:"managed"`
  path. Specified fully in the server lifecycle document (not pre-sequenced
  here).
- **The omnigent server spike** (the 2026-06-23 brainstorm action item) is a
  one-to-two-day validation of server solidity before sinking weeks: drive
  `omnigent server start` + `omnigent run examples/polly/`, capture the SSE
  stream, diff observed events against `openapi.json`, measure cold-start +
  RSS, probe reconnect with `last_event_seq`. Its results gate the project — if
  the server is unstable, the Rust-sidecar contingency returns to the table
  with evidence.

---

## 0.9 What this spec is NOT

- Not an MVP plan. Sequencing is separate.
- Not a port of `ap-web`. The official web client is a reference/contrast; Lens
  is a deliberate native alternative.
- Not seam-driven. There is no `Backend` trait — omnigent is the backend; the
  typed client is the pin layer that could later become a trait if a Rust
  sidecar or another orchestrator emerges. The design keeps that door open
  *without building it*.
- Not omnigent. Lens never executes agents. The server lifecycle document owns
  the spawn/supervise of the server subprocess and the per-host RPC; everything
  else is pure REST + SSE + WS client.