# Second-opinion verification - GPT-5.5

Ground truth checked directly against `/Users/aakshintala/work/omnigent` source and `openapi.json`. I did not modify omnigent.

## Verification Table

| Claim | Verdict | Ground-truth citation | Note |
|---|---|---|---|
| Harness count is disputed: spec 16 vs `_HARNESS_MODULES` 18/20 vs `OMNIGENT_HARNESSES` 19/20. | **NUANCED** | `omnigent/spec/_omnigent_compat.py:80-101`; `omnigent/runtime/harnesses/__init__.py:34-124`; `omnigent/runtime/workflow.py:143-145` | Definitive counts: `OMNIGENT_HARNESSES` has **19 canonical accepted spec harnesses**. `_HARNESS_MODULES` has **20 module keys**, but two are aliases (`claude`, `opencode`), so it has **18 canonical runtime module keys**. It omits `open-responses`, which remains accepted by the validator. `AgentHarnessType` has **7** legacy/ucode gateway entries and is not the picker list. Lens should use `OMNIGENT_HARNESSES` as the canonical user-facing accepted list, with aliases documented separately. |
| `GET /api/version` carries semver/package version; `GET /v1/info` is not the version gate. | **TRUE** | `omnigent/server/app.py:1479-1491`; `omnigent/server/app.py:1493-1564`; `openapi.json` path `/api/version`; `openapi.json` path `/v1/info` | `/api/version` returns `{"version": importlib.metadata.version("omnigent")}`. `/v1/info` returns runtime capability/auth fields: `accounts_enabled`, `login_url`, `needs_setup`, `databricks_features`, `managed_sandboxes_enabled`, `sandbox_provider`. The phrase "only capability flags" is slightly imprecise because it also returns strings/nulls such as `login_url` and `sandbox_provider`; it still has no version. |
| `POST /v1/sessions/{id}/switch-agent` requires `LEVEL_EDIT`, not owner-only. | **TRUE** | `omnigent/server/routes/sessions.py:14173-14186`; `omnigent/server/routes/sessions.py:14207-14216`; `omnigent/server/auth.py:76-79` | The route docstring says 403 when the caller lacks edit access, and the guard calls `_require_access_and_level(..., LEVEL_EDIT, ...)`. `LEVEL_EDIT = 2`, `LEVEL_OWNER = 4`. Owner-only applies to other lifecycle actions such as stop/delete/archive/runner attach, not this route. |
| REST session status is 3-state while SSE `session.status` is 5-state. | **TRUE** | `omnigent/server/schemas.py:1399-1402`; `omnigent/server/schemas.py:1601-1605`; `omnigent/server/schemas.py:1780-1869`; `omnigent/server/schemas.py:2015-2068`; `omnigent/server/routes/sessions.py:1792-1811` | `SessionResponse.status` and `SessionListItem.status` are `idle | running | failed`. SSE `SessionStatusEvent.status` is `idle | launching | running | waiting | failed`. The "card wave 5-state vs poll 3-state" claim is accurate for live-SSE cards vs REST/list snapshots. Also, the server helper collapses cached `waiting` to list `running`. |
| `PresenceViewer` fields are `user_id`, `joined_at`, `idle`, not `display_name`, `is_owner`, `last_seen_at`. | **TRUE** | `omnigent/server/schemas.py:2787-2804`; `omnigent/server/schemas.py:2807-2836` | Presence is a viewer list on `session.presence`; owner/display metadata must be derived elsewhere (`permission_level`, owner endpoint, `/v1/me`) if Lens wants enriched chrome. |
| `PUT /permissions` accepts levels 1-3; owner 4 is implicit/not grantable. | **TRUE** | `omnigent/server/schemas.py:1893-1905`; `omnigent/server/auth.py:76-79`; `omnigent/server/routes/sessions.py:18054-18113` | `GrantPermissionRequest.level = Field(ge=1, le=3)`. `LEVEL_OWNER = 4` exists internally, but grant route prevents modifying owner permissions and creator/admin ownership is handled separately. Public access is additionally capped at read level. |
| URL-mode elicitation `params.url` is a same-origin relative `/approve/...` path, not an arbitrary external OAuth URL. | **TRUE for current producer; NUANCED by schema docs** | `omnigent/runtime/policies/approval.py:157-172`; `omnigent/runtime/policies/approval.py:175-229`; `omnigent/server/schemas.py:2839-2884` | The current builder defaults to URL mode and sets `url = f"/approve/{session_id}/{elicitation_id}"`. The schema comments describe MCP url-mode generically as an external/OAuth URL, so clients should still validate schemes/origin. Current server-generated policy elicitations are relative same-origin approval pages. |
| Filesystem write is `PUT {content}` and search-replace edit is `PATCH {old_text,new_text}`. | **TRUE** | `openapi.json` path `/v1/sessions/{session_id}/resources/environments/{environment_id}/filesystem/{relative_path}`; `openapi.json:7100-7158`; `omnigent/server/routes/sessions.py:16473-16539` | OpenAPI describes `PATCH` as text replacement with `old_text`/`new_text`, and `PUT` as write/replace with `content`. No single "PATCH write/edit" contract should be used. |
| Terminal attach path is `/v1/sessions/{id}/resources/terminals/{tid}/attach`. | **TRUE** | `omnigent/server/routes/terminal_attach.py:103-130`; `omnigent/server/app.py:1635-1642` | The router-local path is `/sessions/.../attach`, but `create_app` mounts the router with `prefix="/v1"`. The external WebSocket URL must include `/v1`. |
| `response.elicitation_resolved` carries no verdict. | **TRUE** | `omnigent/server/schemas.py:2936-2962`; `omnigent/server/routes/sessions.py:3658-3664`; `omnigent/server/routes/sessions.py:3708-3813` | Important for transcript/Bridge behavior: resolved clears an outstanding prompt by `elicitation_id`; approve/decline/cancel verdict is only known locally or from the submitted `ElicitationResult`, not from the resolved event. |
| `switch-agent` is "idle-only." | **NUANCED** | `omnigent/server/routes/sessions.py:1788-1811`; `omnigent/server/routes/sessions.py:14237-14243` | The route rejects when `_session_status_from_cache(session_id) == "running"`. That helper maps cached `waiting` to `running`, so `waiting` is rejected too. `launching` is not in `_EXTERNAL_SESSION_STATUS_VALUES` and falls through to `idle` in this helper, so "idle-only" is product intent, not a complete server-side five-state guard. |

## Corrections to Prior Reviewers

1. **`OMNIGENT_HARNESSES` is 19, not 20.**  
   The `agent-definition-and-sub-agent-topology` review table says `OMNIGENT_HARNESSES` has 20 entries. Source shows 19 entries at `omnigent/spec/_omnigent_compat.py:80-101`. The 20 number belongs to `_HARNESS_MODULES` total keys, and that count includes alias keys.

2. **The best Lens picker baseline is 19 accepted canonical harnesses, not 18 runtime module keys.**  
   Some reviewers recommend `_HARNESS_MODULES` minus aliases as the user-facing list. That would drop `open-responses`, which is explicitly accepted by the spec validator (`omnigent/spec/_omnigent_compat.py:94-95`). For a pure client validating/choosing omnigent harness names, `OMNIGENT_HARNESSES` is the safer canonical list; `_HARNESS_MODULES` explains current subprocess module coverage.

3. **`switch-agent` does reject `waiting`, despite one prior claim.**  
   The app-architecture review says the route does not reject `waiting`; source contradicts that because `_session_status_from_cache` maps `waiting` to `running` (`omnigent/server/routes/sessions.py:1806-1808`), and the switch route rejects `running` (`omnigent/server/routes/sessions.py:14237-14243`). The real gap is `launching`, not `waiting`.

4. **"`/v1/info` only capability flags" is directionally right but overstated.**  
   It is not semver and should not be the contract gate. But it is not pure booleans: `login_url` and `sandbox_provider` are strings/nulls (`omnigent/server/app.py:1557-1564`). Specs should call it an unauthenticated runtime capability/auth probe.

5. **Terminal attach citations that stop at `terminal_attach.py:130` are incomplete.**  
   That line proves only the router-relative path. The external contract must cite the app mount at `omnigent/server/app.py:1635-1642`; otherwise reviewers can incorrectly preserve the `/v1`-less URL.

6. **One workspace review "positive grounding" line is wrong for hosts.**  
   It says `GET/POST/DELETE /v1/hosts` are present. OpenAPI has `GET /v1/hosts` and `GET /v1/hosts/{host_id}`, but host creation/deletion are not REST paths; host registration is tunnel/daemon based. The capability-map reviewer was correct to flag this.

7. **`ElicitationRequestParams.url` schema prose should not override the producer.**  
   Schema comments use MCP's generic "external URL" language (`omnigent/server/schemas.py:2854-2866`), but the current server producer sets a relative `/approve/{session_id}/{elicitation_id}` path (`omnigent/runtime/policies/approval.py:209-229`). Prior findings that say "not external OAuth" are correct for current omnigent-generated approval cards, with URL validation still required for future/generic MCP passthrough.

## Missed Items

1. **`GET /v1/sessions/{id}/labels` deserves explicit Lens treatment.**  
   It is present in the baseline and OpenAPI (`openapi.json` path `/v1/sessions/{session_id}/labels`) but most reviews treat labels only as embedded session fields. Source describes this endpoint as a lightweight spawn-time labels read (`openapi.json:5999-6001`). Lens board grouping/guardrail labels should decide whether to use snapshot labels only or this cheap refresh endpoint.

2. **`GET /v1/sessions/{id}/owner` is separate from presence and permission level.**  
   Reviewers correctly flagged presence shape, but the owner endpoint should be part of the replacement plan for the invented `PresenceViewer.is_owner` field. OpenAPI path `/v1/sessions/{session_id}/owner` returns `{"owner": ...}` (`openapi.json:6083-6085`).

3. **`GET /v1/runners/{runner_id}/status` matters for wake/rebind UX.**  
   Capability-map caught it as absent, but downstream state/wake specs should use it or consciously skip it. OpenAPI path `/v1/runners/{runner_id}/status` returns `runner_id` and `online` (`openapi.json:4650-4653`), which is useful before attempting relaunch/rebind flows.

4. **`POST /v1/hosts/{host_id}/directories` is not just host browse.**  
   Several specs talk about host filesystem pickers but do not pin the "new folder" action. OpenAPI path `/v1/hosts/{host_id}/directories` creates a host directory and is owner-scoped (`openapi.json:4042-4044`).

5. **`POST /v1/sessions/{id}/mcp` is a real client-visible JSON-RPC proxy, not obviously runner-only.**  
   Capability-map flags it as unmapped, but the miss is consequential if Lens wants MCP tool browsing/calls or policy previews. OpenAPI path `/v1/sessions/{session_id}/mcp` supports `initialize`, `tools/list`, and `tools/call` (`openapi.json:6042-6045`).

6. **`GET /v1/sessions/{id}/agent/contents` is probably not a Lens UX endpoint, but it should be explicitly excluded.**  
   It downloads the raw bound agent bundle (`openapi.json:5152-5154`). A pure client may not need it, yet a spec claiming full API parity should mark it "runner/debug only" rather than omit it silently.

7. **`WS /v1/sessions/updates` is source-grounded but absent from the 59 REST-path baseline.**  
   It is not in OpenAPI, so the baseline rightly excludes it, but source/test references mean Lens should choose poll-only vs push updates deliberately. The application-shell review caught this; it should be promoted into typed-client/server-lifecycle drift notes.
