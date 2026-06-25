# Review: `agent-definition.md` + `sub-agent-topology.md`

**Reviewed against:** omnigent @ `0.3.0.dev0` (`pyproject.toml`, `openapi.json` OpenAPI 3.2.0 / 59 paths)  
**Spec baseline:** v0.2.0 (2026-06-23)

## TL;DR

- **Harness count drift:** Spec claims **16** harnesses; current runtime registry has **18** canonical harness modules (`goose`, `hermes` missing from spec). Validator set `OMNIGENT_HARNESSES` has **20** names (still includes `open-responses`, which has no `_HARNESS_MODULES` entry). Intro line still says “11-harness matrix.”
- **Blocker — switch-agent guard wrong:** Spec + capability map say **owner-only** (`LEVEL_OWNER` / `permission_level >= 4`); source requires **`LEVEL_EDIT` (2)** only (`sessions.py:14214-14216`, `auth.py:76-79`). Permissions doc UI rule (< 4 disables) is stricter than the API, but agent-definition claims “verified in source” for owner-only — false.
- **Major — YAML schema oversimplified:** Example uses flat `executor.harness`, inline `sub_agents:`, list-form `terminals: [shell]`, and omits `executor.type: omnigent` / `executor.config.harness`; real parsing lives in `omnigent/spec/types.py`, `parser.py`, `omnigent.py`.
- **Major — switch-agent flow error:** `SessionSwitchAgentRequest` is **`agent_id` only** (built-in agents); no bundle reference on switch. PUT `/agent` is a separate, same-name constraint — steps 2–3 in §7 conflate draft-bundle iteration with switch-agent.
- **Major — child summary field drift:** `ChildSessionSummary.agent_name` and `current_task_id` are **`None` in 0.3.0** (`sessions.py:11811-11815`, tasks table removed); sub-agent topology tray pills relying on `agent_name` need fallback to `tool`/`title`.

---

## agent-definition.md

### Blockers

- **[SEVERITY: blocker] [grounding + drift]** Switch-agent permission guard misstated as owner-only
  - Location: §7 “Constraints” (lines 225–228); echoed capability-map §0.7-J and typed-client §3
  - Evidence: Spec: “Requires `LEVEL_OWNER` (`permission_level >= 4`).” Route calls `_require_access_and_level(..., LEVEL_EDIT, ...)` (`omnigent/server/routes/sessions.py:14214-14216`). `LEVEL_EDIT = 2`, `LEVEL_OWNER = 4` (`omnigent/server/auth.py:76-79`). Docstring lists 403 for “lacks edit access,” not owner-only.
  - Recommendation: Either (a) align Lens UI with API — enable switch for edit-level (≥2) shared sessions, or (b) if product intent is owner-only, document that as a **Lens policy** atop the API and drop “verified in source.” Reconcile permissions-and-elicitations §7 (`permission_level < 4` disables) with actual server gate.

- **[SEVERITY: blocker] [grounding + drift]** Harness count is 18 (registry), not 16; two harnesses omitted
  - Location: §4 title + table (lines 125–146); intro (line 4 “11-harness matrix”); §2 example comment (line 59)
  - Evidence: `_HARNESS_MODULES` registers **18** canonical keys (excluding aliases `claude` → `claude-sdk`, `opencode` → `opencode-native`): `omnigent/runtime/harnesses/__init__.py:34-124`. Spec table lists 16 — **missing `goose`** (headless ACP, line 97-101) and **`hermes`** (line 118-123). `OMNIGENT_HARNESSES` validator frozenset has **20** entries including **`open-responses`** (`omnigent/spec/_omnigent_compat.py:80-101`) despite spec §4 claiming it was “dropped upstream.”
  - Recommendation: Re-baseline §4 to **18 runtime harnesses** (+ aliases doc). Add `goose` / `hermes` rows with Kind/Provider. Note `open-responses` is validator-accepted but has no dedicated `_HARNESS_MODULES` wrap (adapter-routed). Fix intro “11-harness” → current count. Fix cited path: `harness_aliases.py` is `omnigent/harness_aliases.py`, not under `runtime/harnesses/`.

### Major

- **[SEVERITY: major] [grounding + drift]** Agent YAML example does not match omnigent spec shape
  - Location: §2 (lines 54–80), `AgentSpec` Rust mirror (lines 87–98)
  - Evidence: Example shows `executor.harness`, `executor.model`, top-level `policies:`, inline `sub_agents:`, `terminals: [shell]`. Actual model: `executor.type` discriminator (`omnigent` | `claude_sdk` | `agents_sdk`), harness under `executor.config.harness` for omnigent type (`omnigent/spec/types.py:487-544`, `_omnigent_compat.py:138-175`); `policies:` translated to `guardrails` (`omnigent/spec/omnigent.py:574+`); sub-agents discovered from `agents/<name>/` dirs (`parser.py:2475+`) plus inline `tools` agents; `terminals:` is a **map** `name → TerminalEnvSpec`, not a string list (`types.py:1437-1443`). `AgentObject` from API exposes `harness`, `mcp_servers`, `policies`, `skills`, `terminals` — not the full YAML surface (`openapi.json` `AgentObject` ~line 44).
  - Recommendation: Split §2 into (1) author-facing omnigent YAML (link to upstream spec / example bundles) and (2) Lens `AgentSpec` mirror of **`AgentObject` + registry metadata**, not raw YAML fields. Mark `sub_agents:` inline as directory-based unless omnigent single-file format is pinned.

- **[SEVERITY: major] [completeness]** Switch-agent flow incorrectly pairs PUT bundle with switch body
  - Location: §7 steps 2–3 (lines 214–216); §6 cross-ref (line 194)
  - Evidence: `SessionSwitchAgentRequest` has single field `agent_id` — must be a **built-in** (`session_id IS NULL`) agent (`schemas.py:1761-1777`, `sessions.py:14252-14260`). Switch clones `target_agent.bundle_location`, not a client-uploaded bundle (`sessions.py:14325-14333`). PUT `/v1/sessions/{id}/agent` requires uploaded bundle **spec name matches existing** session agent (`sessions.py:18470-18481`, 18525). No bundle field on switch endpoint (`openapi.json` `/switch-agent` ~7997).
  - Recommendation: Decouple §6 draft-test loop (multipart create + PUT on session-scoped clone) from §7 switch (built-in `agent_id` only). Remove “POST switch-agent with the bundle reference.” Clarify PUT is for iterating the **current** session-scoped agent, not for selecting switch target.

- **[SEVERITY: major] [grounding + completeness]** `session.agent_changed` carries only agent id/name — not model/skills chrome
  - Location: §7 step 4 (lines 217–220); state-model cross-ref
  - Evidence: `SessionAgentChangedEvent` fields: `conversation_id`, `agent_id`, `agent_name` only (`schemas.py:2192-2221`). Openapi Category: **transient (SSE-only)**; reconnect reads binding from snapshot. Spec lists updates to `llm_model`, `model_options`, `reasoning_effort`, `skills` driven by this event — those require **`GET /v1/sessions/{id}` snapshot refresh** (or subsequent chrome events), not the event payload alone.
  - Recommendation: State model step: on `session.agent_changed`, **refetch snapshot** (or await `session.model_options` / `session.skills` deltas). Document same-provider model carry-over and native history rebuild rules from switch route docstring (`sessions.py:14288-14300`).

- **[SEVERITY: major] [grounding]** Idle-only guard blocks `waiting` as well as `running`
  - Location: §7 “Idle-only” (lines 229–232)
  - Evidence: Switch rejects when `_session_status_from_cache(session_id) == "running"` (`sessions.py:14239-14243`). Cache maps both `"running"` and `"waiting"` → `"running"` (`sessions.py:1806-1808`). Parent **waiting on sub-agent** cannot switch even though no turn is active on parent harness.
  - Recommendation: Spec UI “busy” disable should include **`waiting`** status, not only in-flight turns. Surface distinct copy: “session waiting — resolve or interrupt first.”

- **[SEVERITY: major] [grounding]** OpenAPI harness field is now free string, not enum — spec’s enum-lag note is stale
  - Location: §4 “Openapi vs. code lag” (lines 157–162)
  - Evidence: Current `AgentObject.harness` is `string | null`, not a closed enum (`openapi.json:62-71`). Spec still describes missing enum members (`copilot`, etc.).
  - Recommendation: Update to: openapi uses open string; canonical list is `_HARNESS_MODULES` / `OMNIGENT_HARNESSES`. Typed client should mirror registry, not enum.

- **[SEVERITY: major] [cross-doc consistency]** Owner-only switch-agent contradicts permissions doc and server
  - Location: §7 vs permissions-and-elicitations §7 (line 184)
  - Evidence: Permissions doc: `permission_level < 4` → switch disabled. Server: `LEVEL_EDIT` (2) sufficient. Capability-map §0.7-J: “owner-only **and** idle-only (verified in source).”
  - Recommendation: Single decision across capability-map, permissions, agent-definition, typed-client: API floor vs Lens UI floor.

### Minor / clarity

- **[SEVERITY: minor] [clarity]** Internal harness-count contradiction in same doc
  - Location: Line 4 (“11-harness matrix”) vs §4 (“16 harnesses”)
  - Evidence: Same file, two counts.
  - Recommendation: One authoritative count + “as of omnigent X.Y.”

- **[SEVERITY: minor] [grounding]** Citation path drift for switch-agent and harness aliases
  - Location: §7 (line 207), §4 (lines 126, 153)
  - Evidence: Switch-agent at `sessions.py:14176` — correct module, line shifted. `harness_aliases.py` cited under `runtime/harnesses/` — file is `omnigent/harness_aliases.py`. No `routes/bundles.py`; bundle routes live in `sessions.py` (`PUT/GET …/agent`, multipart `POST /v1/sessions`).
  - Recommendation: Fix paths; drop bundles.py reference if present elsewhere.

- **[SEVERITY: minor] [completeness]** Bundle PUT same-name constraint not documented
  - Location: §6 (lines 193–196)
  - Evidence: PUT rejects when spec name ≠ bound agent name (`sessions.py:18520-18526`). Idempotent on unchanged content (line 18472).
  - Recommendation: Add to §6: draft iteration must keep agent name stable; switching to a differently named spec requires new session (multipart create), not PUT.

- **[SEVERITY: minor] [feasibility]** `Agents::list()` reads server scan — no REST agent CRUD for Lens picker
  - Location: §3 (lines 105–106)
  - Evidence: `GET /v1/agents` lists registry; `POST /api/agents` also exists for bundle upload to global registry (`API.md:20-47`) — Lens spec intentionally filesystem-only for authoring.
  - Recommendation: Explicitly note Lens ignores `POST /api/agents`; global registry mutations are out of scope.

- **[SEVERITY: minor] [vocabulary / clarity]** Net-new vocabulary mixes YAML author model with API catalog model
  - Location: §1–§2
  - Evidence: §2 YAML block is author-centric; §3–§4 pivot to `AgentObject` / harness badges without a glossary bridging `executor.config.harness` → `AgentObject.harness`.
  - Recommendation: Add a short mapping table (YAML field → API field → Lens `AgentSpec` field).

---

## sub-agent-topology.md

### Blockers

- **[SEVERITY: blocker] [grounding + drift]** `ChildSessionSummary.agent_name` and `current_task_id` are unset in 0.3.0
  - Location: §2 `ChildSession` mirror (lines 54–73); §3 tray pills (lines 107–108, 125–126)
  - Evidence: `_child_session_summary_from_conversation` sets `agent_name=None`, `current_task_id=None` with comment “tasks table is gone” (`sessions.py:11811-11816`). `tool` / `title` parsing still populated (lines 11758-11781). Openapi/schemas docstrings still describe task-derived `agent_name` (`schemas.py:603-607`) — doc/code mismatch upstream.
  - Recommendation: Lens `ChildSession` should treat `agent_name` as optional; display **`tool`** (spawn prefix / Codex nickname) as primary label. Re-verify after omnigent lock-step. Contract-test against live `GET …/child_sessions`, not schemas.py comments alone.

### Major

- **[SEVERITY: major] [completeness]** Recursive pending-elicitation rollup is not server-provided on parent summary
  - Location: §6 “Recursive rollup” (lines 183–185)
  - Evidence: `pending_elicitations_count` on each child is `pending_elicitations.count_for(conv.id)` — **direct child only** (`sessions.py:11824`). `GET …/child_sessions` returns **direct children only** (`test_sessions_subagent_context.py` “lists only direct children”). No field aggregates grandchildren onto parent’s `ChildSessionSummary`.
  - Recommendation: State model must **walk the tree**: subscribe to each focused ancestor’s stream, maintain per-parent child maps, and sum descendant counts client-side (or fetch nested `child_sessions` per child). Document that mirror-routed elicitations (`target_session_id`, permissions §5) appear on **ancestor stream**, not necessarily in child row counts.

- **[SEVERITY: major] [completeness]** `session.created` is not a full `ChildSessionSummary`
  - Location: §2 (lines 87–88)
  - Evidence: `SessionCreatedEvent` wire shape: `child_session_id`, `agent_id`, `parent_session_id` only (`schemas.py:2492-2547`). No `busy`, `pending_elicitations_count`, or preview fields.
  - Recommendation: On `session.created`, seed minimal row then **merge** first `session.child_session.updated` or lazy `GET …/child_sessions`. Do not expect full summary from create event.

- **[SEVERITY: major] [grounding]** `ChildSessionSummary` still absent as named OpenAPI component
  - Location: §2 (lines 76–80)
  - Evidence: Grep `openapi.json` for `"ChildSessionSummary"` — **no component schema**; event description references class by name (`SessionChildSessionUpdatedEvent` ~2129). Partial `child` dict on wire.
  - Recommendation: Confirmed — hand-written mirror remains required. Pin contract test to `schemas.py:558-664` + sample `GET …/child_sessions` response, not openapi components.

- **[SEVERITY: major] [completeness]** Deep sub-agent trees require multi-hop client fetching
  - Location: §5 “Sub-agent depth” (lines 168–171)
  - Evidence: `child_sessions` endpoint scopes to **direct** children (`parent_conversation_id=session_id`, `openapi.json` ~5194). Recursive breadcrumb “‹ root › / mid › leaf” implies Lens holds parent chain in session registry — OK — but tray tree for root parent **does not include grandchildren** without opening mid-level child and fetching its children or subscribing to its stream.
  - Recommendation: Spec §5: when child is focused, tray shows **that session’s** children; root tray lists direct children only. For rollup badges at root, state model must aggregate across levels (see §6 fix).

- **[SEVERITY: major] [cross-doc consistency]** Pending-elicitation badges align with permissions doc — with caveats
  - Location: §6 (lines 176–192)
  - Evidence: Permissions §5: resolve via `POST /v1/sessions/{target_session_id}/elicitations/{id}/resolve`. Child row badge uses `pending_elicitations_count` on summary (`schemas.py:637-644`). Activity-line promotion (“needs you”) matches permissions Bridge badge story.
  - Recommendation: Add explicit rule: when elicitation is **mirrored to parent stream**, parent’s **`SessionState.pending_elicitation`** drives composer widget; child row badge may still show count — dedupe UI so user doesn’t see two widgets for one elicitation. Cross-ref permissions §2 lifecycle.

### Minor

- **[SEVERITY: minor] [grounding]** `connection_id` on `ChildSession` is Lens-local — correct but unstated in boundaries
  - Location: §2 (line 56)
  - Evidence: Not in omnigent schema; composite key from state model.
  - Recommendation: One sentence in §1: `connection_id` is Lens-only; server identifies children by `id` + `parent_session_id`.

- **[SEVERITY: minor] [completeness]** Child tombstones — no server event for cleanup
  - Location: §7 open questions (lines 203–206)
  - Evidence: Child rows persist in store; no documented `session.child_session.removed` event in openapi grep. Completion inferred from `busy=false`, `current_task_status`, labels.
  - Recommendation: Tombstone policy should key off `session.child_session.updated` status deltas + optional label closed-marker parsing (`sessions.py:11758-11759` `title_without_closed_marker`).

- **[SEVERITY: minor] [feasibility]** Tray tree scalability vs recursive spec
  - Location: §3 zones 2–3 (lines 122–130), §5 depth (lines 168–171)
  - Evidence: Hybrid B decision (tray home, no board cards) matches application-shell §14. Recursive visual + bounded chip is feasible but state sync cost grows with depth × fan-out.
  - Recommendation: Cap initial tree depth in tray (direct children); defer nested tree to child focused window or popover (already hinted §7).

- **[SEVERITY: minor] [clarity]** `current_task_status` semantics narrowed in 0.3.0
  - Location: §2 (line 65), §3 (line 107)
  - Evidence: Now derived from cache/labels: often `None` or `"failed"` only (`sessions.py:11790-11792`), not full task lifecycle enum from tasks table.
  - Recommendation: Tray pills should prefer `busy` + `last_message_preview` + `pending_elicitations_count` over `current_task_status` unless value is present.

---

## Verified harness inventory (0.3.0.dev0)

| Source | Count | Notes |
|--------|-------|-------|
| Spec §4 claim | **16** | Missing `goose`, `hermes` |
| `_HARNESS_MODULES` (canonical keys, excl. `claude`, `opencode` aliases) | **18** | Runtime wraps |
| `OMNIGENT_HARNESSES` (validator) | **20** | Includes `open-responses` (no dedicated module) |
| Spec intro | **11** | Stale typo |

**18 canonical runtime harnesses:** `claude-sdk`, `claude-native`, `codex`, `codex-native`, `cursor`, `cursor-native`, `openai-agents`, `pi`, `pi-native`, `antigravity`, `antigravity-native`, `qwen`, `qwen-native`, `goose`, `goose-native`, `opencode-native`, `copilot`, `hermes`.
