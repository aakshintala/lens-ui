# Review: `typed-client.md`

**Reviewed against:** omnigent @ `0.3.0.dev0` (`openapi.json` info.version `"0.1.0"`, 59 paths, 8100 lines)  
**Spec baseline:** v0.2.0 (2026-06-23)

## TL;DR

- **Blocker:** §8 pins the contract gate on `GET /v1/info` “build/version”, but current `GET /v1/info` returns auth/capability flags only — no semver. Version lives at `GET /api/version` (`app.py:1479-1491`, `openapi.json` `/api/version`).
- **Blocker:** §7 reconnect step 5 filters `GET /items` rows by `sequence_number`, but persisted conversation items have no `sequence_number` field (`ConversationItem` / `to_api_dict()` in `omnigent/entities/conversation.py:628-675`). The dedup/replay protocol as written cannot work.
- **Major:** §6 `SessionEventInput` lists ~5 discriminators; the route allows ~20+ (`sessions.py:771-792`, `_ALLOWED_EVENT_TYPES`), including `compact`, `slash_command`, `mcp_elicitation`, and many `external_*` types documented in `openapi.json` `POST …/events` (line 5690). Contract tests would pass a false-negative surface.
- **Major:** §3 endpoint inventory omits current openapi paths (`GET /v1/runners`, `GET /v1/sessions/{id}/agent`, `POST …/hooks/antigravity-elicitation-request`, `POST …/hooks/cursor-permission-request`) and non-`/v1` paths the gate needs (`/api/version`, `/health`).
- **Major:** §7 persisted/transient table treats most `session.*` chrome events as “persisted / re-emitted from history”, but openapi marks them **transient (SSE-only)** with state restored from the **session snapshot** on reconnect — not from `GET /items` replay (`session.status`, `session.created`, `session.usage`, etc.; see openapi Category tags).

---

## Findings

### Blockers

- **[SEVERITY: blocker] [openapi grounding + technical feasibility]** Contract gate probes wrong endpoint for version
  - Location: §8 (lines 386–402), also implied §1 scope (line 24)
  - Evidence: Spec says `GET /v1/info` returns “server build/version” and compares to `PINNED_OMNIGENT_VERSION`. Current `GET /v1/info` returns capability booleans/strings only (`accounts_enabled`, `login_url`, `needs_setup`, `databricks_features`, `managed_sandboxes_enabled`, `sandbox_provider`) — **no version field** (`omnigent/server/app.py:1493-1564`, `openapi.json` `/v1/info` ~4339). Package version is `GET /api/version` → `{"version": "<semver>"}` (`app.py:1479-1491`, `openapi.json` `/api/version` ~3790).
  - Recommendation: Gate on `GET /api/version` (primary) and optionally probe `GET /v1/info` for capability flags. Update server-lifecycle cross-ref. Keep feature-detection fallback but include `/api/version`.

- **[SEVERITY: blocker] [technical feasibility]** Reconnect history dedup uses nonexistent item field
  - Location: §7 steps 5–7 (lines 295–306)
  - Evidence: Spec: “Items with `sequence_number ≤ last_seen_seq` are discarded” after `GET /v1/sessions/{id}/items`. Conversation items persisted and returned by that route are `ConversationItem` objects with `id`, `type`, `status`, `response_id`, `created_at`, `data`, `created_by` — **no `sequence_number`** (`omnigent/entities/conversation.py:628-675`; route at `sessions.py:15209-15261`).
  - Recommendation: Reconnect transcript fill should merge snapshot/`GET /items` by **item id** (or `created_at` + id cursor), not SSE seq. Reserve `sequence_number` dedup for the live SSE overlap window (step 7). Document synthesis of `OutputItemDone` from items if the state model expects stream-shaped events.

---

### Major — openapi / version drift

- **[SEVERITY: major] [openapi grounding]** Missing endpoints in §3 inventory
  - Location: §3 (lines 77–177)
  - Evidence: Present in current `openapi.json` but absent from spec table:
    - `GET /v1/runners` (~4620) — list online runners for requesting user
    - `GET /v1/sessions/{session_id}/agent` (~5058) — bound agent metadata (PUT is listed; GET is not)
    - `POST /v1/sessions/{session_id}/hooks/antigravity-elicitation-request` (~5739)
    - `POST /v1/sessions/{session_id}/hooks/cursor-permission-request` (~5821)
    - Non-`/v1` but load-bearing: `GET /api/version` (~3790), `GET /health` (~3813, batch session liveness)
  - Recommendation: Add to §3 (or a “non-v1 / probe endpoints” subsection). Include in §9 contract-test reachability. Lens is client-only for hooks (server-initiated long-poll), but typed client should know they exist for completeness.

- **[SEVERITY: major] [openapi grounding]** `SessionEventInput` discriminator set severely incomplete
  - Location: §6 (lines 230–267), Rust sketch (lines 246–257)
  - Evidence: Spec lists `Message`, `FunctionCallOutput`, `Approval`, `Interrupt`, `StopSession`. Route `_ALLOWED_EVENT_TYPES` unions item types + control types (`sessions.py:771-792`): adds `compact`, `slash_command`, `mcp_elicitation`, and `external_assistant_message`, `external_conversation_item`, `external_output_text_delta`, `external_output_reasoning_delta`, `external_session_interrupted`, `external_elicitation_resolved`, `external_session_status`, `external_session_usage`, `external_compaction_status`, `external_model_change`, `external_reasoning_effort_change`, `external_session_todos`, `external_subagent_start`, `external_codex_subagent_start`, `external_codex_collaboration_mode_change`. Openapi `POST …/events` description (~5690) documents the `external_*` set explicitly.
  - Recommendation: Expand §6 enum + contract-test harness to full `_ALLOWED_EVENT_TYPES`. Lens may only *send* a subset (`message`, `function_call_output`, `approval`, `interrupt`, `stop_session`, `compact`), but the parser/validator must accept the server's full dispatch table. Cross-ref capability-map “compact via POST /events”.

- **[SEVERITY: major] [openapi grounding + clarity]** Persisted/transient classification contradicts openapi
  - Location: §7 “Persisted-vs-transient classification” (lines 337–354)
  - Evidence: Spec lists as **Persisted**: `session.status`, `session.created`, `session.usage`, `session.model`, `session.todos`, `session.model_options`, `session.reasoning_effort`, `session.collaboration_mode`, `session.skills`, `session.resource.*`, `session.child_session.updated`. Openapi marks these as **Category: transient (SSE-only)** with reconnect state from snapshot/REST, e.g. `SessionStatusEvent` (~3000), `SessionCreatedEvent` (~2210), `SessionUsageEvent` (~3197), `SessionAgentChangedEvent` (~2051). They are not re-emitted from `GET /items`.
  - Recommendation: Split classification into three buckets: **(A) item-backed / replayable from `GET /items`** (`response.output_item.done`, lifecycle terminals), **(B) snapshot-restored** (most `session.*` chrome — read snapshot fields on reconnect), **(C) truly transient** (deltas, heartbeats, presence). Align §7 step 4/5 with this — snapshot carries chrome; items carry transcript.

- **[SEVERITY: major] [openapi grounding]** `GET /v1/sessions/{id}` query params incomplete
  - Location: §3 sessions table (line 83)
  - Evidence: Spec lists `include_items?`, `include_liveness?`. Openapi also documents `refresh_state` (~4974) to refresh runner-owned snapshot fields.
  - Recommendation: Add `refresh_state` to the GET snapshot row and `GetOpts` sketch.

- **[SEVERITY: major] [openapi grounding]** Permission level semantics wrong for grants API
  - Location: §3 permissions row (line 159)
  - Evidence: Spec: “level 1=read..4=owner”. Openapi `PermissionGrantRequest` / `PermissionObject`: levels **1=read, 2=edit, 3=manage** only (`schemas.py:1900-1916`, `openapi.json` ~867). `LEVEL_OWNER = 4` exists in auth (`auth.py:76-79`) but is derived (creator/admin), not grantable via `PUT /permissions`.
  - Recommendation: Document grantable 1–3; treat owner (4) as implicit for session creator. Note `__public__` read grant unchanged.

- **[SEVERITY: major] [openapi grounding]** Harness registry drift — `hermes` not in canonical 16
  - Location: §9 item 3 (lines 421–426), capability-map cross-ref
  - Evidence: Spec claims “16 at 0.2.0 HEAD” canonical harnesses. Current `_HARNESS_MODULES` has **20 keys**, including net-new **`hermes`** (`omnigent/runtime/harnesses/__init__.py:118-123`) plus alias keys `claude`, `opencode`, `goose`. Spec path cites `harness_aliases.py` under `runtime/harnesses/`; file is `omnigent/harness_aliases.py`.
  - Recommendation: Re-verify canonical picker list against `_HARNESS_MODULES` minus aliases; add `hermes` (and decide on headless `goose` vs `goose-native`). Fix harness_aliases path.

- **[SEVERITY: major] [openapi grounding]** `ServerStreamEvent` / session event sketches omit wire fields
  - Location: §10 `SessionEvent` enum (lines 493–556)
  - Evidence:
    - `SessionStatus`: openapi adds `conversation_id`, `response_id`, `error` (`SessionStatusEvent` ~3000) — spec only has `status`.
    - `Created`: openapi `session.created` is **child sub-agent spawn** on parent stream (`child_session_id`, `agent_id`, `parent_session_id`; ~2209) — not generic “session was just created”.
    - `Interrupted`: wire uses nested envelope `{type, data: {requested_at, response_id?}}` (`SessionInterruptedEvent` ~2518) — spec has unit variant.
    - `InputConsumed`: wire uses nested `{type, data: SessionInputConsumedPayload}` (~2420) — spec flattens to `item_id`/`item_type`.
    - `ChildSessionUpdated`: requires top-level `child_session_id` + partial `child` dict (~2129) — spec only shows `child: ChildSessionSummary`.
    - `Heartbeat` (session): openapi includes optional `server_time` (~2396) — spec says “sequence_number only” (line 193).
    - `TurnFailed`: `error` is object (`additionalProperties`, ~3365) — spec `error: String` (line 650).
    - `TerminalActivity`: wire includes `session_id` (~3081) — spec only `terminal_id`.
  - Recommendation: Align enum variants with openapi schemas (or document normalization layer that strips/adds fields). Rename `Created` → `ChildSessionSpawned`.

- **[SEVERITY: major] [clarity]** Contradictory PUT `/agent` row in sessions table
  - Location: §3 sessions table (line 90) vs note (lines 94–95)
  - Evidence: Line 90 still says PUT `/agent` “switch-agent … fires `session.agent_changed`”. Lines 94–95 correctly state PUT is bundle storage; switch is `POST /switch-agent` (`openapi.json` ~7997, emits `session.agent_changed` per ~2051).
  - Recommendation: Fix line 90 to match line 94 (bundle upload only; does not fire agent_changed).

---

### Major — cross-document / completeness

- **[SEVERITY: major] [cross-document consistency]** Contract gate shared with server-lifecycle is built on wrong endpoint
  - Location: §8; `server-lifecycle.md` §8 (lines 262–275)
  - Evidence: Both docs assume `GET /v1/info` yields semver for `PINNED_OMNIGENT_VERSION`. Source shows otherwise (see blocker above).
  - Recommendation: Fix both docs together; lifecycle “ready detection” can keep pinging `/v1/info` for liveness, but version pin must use `/api/version`.

- **[SEVERITY: major] [completeness]** Reconnect protocol does not cover snapshot-restored chrome
  - Location: §7 steps 4–5 (lines 292–298)
  - Evidence: After reconnect, transient `session.*` chrome (todos, skills, model_options, sandbox_status, agent binding) must come from `GET /v1/sessions/{id}` snapshot fields, not from item replay. Spec step 5 only mentions items + seq filter.
  - Recommendation: Step 4: apply full snapshot to session scalars/collections. Step 5: items for transcript only. Step 6+: optional `GET child_sessions`, `GET elicitations/{id}` for mixed cases in §7 mixed bucket.

- **[SEVERITY: major] [completeness]** `session.agent_changed` reconnect handling underspecified
  - Location: §7 persisted list (implicit), §10 `AgentChanged` (line 543)
  - Evidence: Openapi: **transient** — “on reconnect clients read the new binding from the session snapshot” (`SessionAgentChangedEvent` ~2051). State model inserts transcript `AgentChanged` item on live event (`app-architecture-and-state-model.md` §12.2). After reconnect, snapshot has new agent but no marker item.
  - Recommendation: §7: classify `session.agent_changed` as transient/snapshot-restored; note state model may need snapshot-diff to avoid duplicate markers.

- **[SEVERITY: major] [cross-document consistency]** Native permission hooks incomplete vs openapi
  - Location: §3 hooks table (lines 154–155); `permissions-and-elicitations.md` §6
  - Evidence: Spec lists claude + codex hooks only. Openapi adds antigravity + cursor-native hooks (~5739, ~5821). permissions doc §6 same gap.
  - Recommendation: Add both hooks to §3; permissions doc should reference all four (Lens won't POST them, but elicitation UX must tolerate all harness sources).

- **[SEVERITY: major] [completeness]** `compact` dispatch missing from command-flow contract
  - Location: §6; state model §7 (`app-architecture-and-state-model.md` ~614-641)
  - Evidence: Capability-map lists “compact” via `POST /events` (`capability-map-and-design-language.md` line 149). Route supports `_COMPACT_TYPE = "compact"` (`sessions.py:294`). Neither typed-client §6 nor state-model §7 command list includes `SessionEventInput::Compact`.
  - Recommendation: Add `Compact` variant to §6 and state-model §7.

---

### Minor

- **[SEVERITY: minor] [openapi grounding]** Openapi metadata drift from spec header
  - Location: Header (lines 12, 75, 411)
  - Evidence: Spec: “7978 lines”, v0.2.0. Current file: **8100 lines**, info.version **"0.1.0"** (`openapi.json:3786`), package 0.3.0.dev0.
  - Recommendation: Note intentional pin to vendored 0.2.0 snapshot; refresh vendor blob + line count on bump.

- **[SEVERITY: minor] [openapi grounding]** `session.input.consumed` marked provisional upstream
  - Location: §10 `InputConsumed` (lines 500–503)
  - Evidence: Openapi description: “event name is **provisional** — may be renamed” (`SessionInputConsumedEvent` ~2420).
  - Recommendation: Parser should key off typed struct / openapi discriminator mapping, not hardcoded string; flag in contract tests.

- **[SEVERITY: minor] [openapi grounding]** `session.changed_files.invalidated` classification
  - Location: §7 transient list (missing); §10 `ChangedFilesInvalidated` (lines 527–529)
  - Evidence: Openapi: “transient (not persisted — the REST list is source of truth)” (`SessionChangedFilesInvalidatedEvent` ~2093).
  - Recommendation: Add to transient bucket; reconnect should refetch `GET …/environments/{env_id}/changes`, not replay event.

- **[SEVERITY: minor] [openapi grounding]** Terminal WS attach `read_only` query param
  - Location: §5 (lines 208–226)
  - Evidence: Route accepts `read_only: bool = Query(default=False)` (`terminal_attach.py:130-148`). Spec describes read-only default via tmux `-r` but not the query param contract.
  - Recommendation: Document `?read_only=true|false`; owner may write-attach with `read_only=false`.

- **[SEVERITY: minor] [technical feasibility]** §7 lists `session.heartbeat` in both transient and mixed
  - Location: §7 (lines 347–354)
  - Evidence: Same event type appears under transient and mixed with conflicting guidance.
  - Recommendation: Keep only in transient; gap detection uses `response.heartbeat.last_event_seq`.

- **[SEVERITY: minor] [technical feasibility]** SSE thread model vs framework doc
  - Location: §4 (lines 200–204)
  - Evidence: Spec uses dedicated OS thread + `Mpsc` + UI poller. `framework.md` §2.1 says “All I/O is in `cx.background_spawn`, never on the UI thread.”
  - Recommendation: Clarify: blocking `reqwest` body read runs off UI thread (thread or `background_spawn`), events arrive via channel — align wording with framework doc.

- **[SEVERITY: minor] [completeness]** `/health` batch liveness not surfaced
  - Location: §3 (absent)
  - Evidence: `GET /health?session_ids=…` returns per-session `runner_online` / `host_online` (`openapi.json` ~3813). Useful for fleet/board polling alongside `GET /v1/sessions`.
  - Recommendation: Optional `Client::health()` for sleeping-session liveness without opening SSE.

- **[SEVERITY: minor] [completeness]** `ResponseEvent::Retry` sketch under-specified
  - Location: §10 (line 632)
  - Evidence: Openapi `RetryEvent` requires `source`, `attempt`, `max_attempts`, `delay_seconds`, `error`, optional `tool_name` (~1814-1820).
  - Recommendation: Expand Retry variant fields in sketch.

---

### Nits

- **[SEVERITY: nit] [openapi grounding]** Verified correct (no change needed)
  - Location: §3 switch-agent, §5 terminal WS
  - Evidence: `POST /v1/sessions/{session_id}/switch-agent` exists (~7997); WS path `/sessions/{session_id}/resources/terminals/{terminal_id}/attach` without `/v1` (`terminal_attach.py:130`). `ChildSessionSummary` still absent from `components/schemas` (only referenced in descriptions ~2130, ~5196) — hand-written mirror remains required.

- **[SEVERITY: nit] [clarity]** §7 retry timing label
  - Location: §7 step 2 (lines 284–288)
  - Evidence: Backoff sequence sums to ~6.1s before first 3000ms repeat; “~7s” is approximate.
  - Recommendation: Either document “first pass ~6s, then 3s indefinitely until user retry” or cap total automated retries explicitly.

- **[SEVERITY: nit] [cross-document consistency]** Decision J (live switch-agent) aligned
  - Location: §3 POST `/switch-agent` (lines 92, 543)
  - Evidence: `SessionSwitchAgentRequest{agent_id}` only (~3066); owner-only idle-only 409 documented in tests/routes. Matches capability-map §0.7-J.

- **[SEVERITY: nit] [cross-document consistency]** Decision C (terminal ring buffer) aligned
  - Location: §5 (lines 218–220)
  - Evidence: Matches `workspace-and-terminals.md` §0.7-C; server has no replay buffer on attach.

---

## Verified OK (spot-checks, not exhaustive)

- Environment-scoped workspace paths under `/v1/sessions/{id}/resources/environments/{env_id}/…` match openapi (~6625–7370).
- Full SSE discriminator set in openapi (48 event type strings) matches §10 coverage except gaps noted above; turn.* events correctly marked supplementary (~644-646).
- `ElicitationResult` actions `accept|decline|cancel` match openapi (~708-718).
- `sequence_number` on SSE events + `response.heartbeat.last_event_seq` / `server_time` match §4 (~900-943).
- Auth model `Bearer` / cookie / `X-Forwarded-Email` matches `auth.py` header modes.
- `PUT /v1/sessions/{id}/agent` = bundle upload; `POST /switch-agent` = in-place rebind — correctly explained in §3 note (lines 94–95) aside from line 90 table typo.
