# Grounding baseline for the Lens spec review

Independently verified by the lead reviewer against the sibling omnigent repo
(`/Users/aakshintala/work/omnigent`) at HEAD (`36b2a11c`, committed 2026-06-24).
Hand this to the deep-pass (synthesis / second-opinion) agents as authoritative.

## Version posture (DRIFT)
- Specs are written against omnigent **v0.2.0** (README says "0.2.0-alpha").
- Checked-out omnigent source is **0.3.0.dev0** (pyproject), `openapi.json` `info.version` = **"0.1.0"**, OpenAPI **3.2.0**.
- `openapi.json` is **NOT present in the lens repo** despite the README calling it "checked-in". It exists only in the omnigent repo. Every spec claim "verifiable against the checked-in openapi.json" is, in this repo, unverifiable without the sibling checkout.

## API surface: 59 REST paths (authoritative)
NOTE: this list is REST only. WebSocket terminal-attach paths are NOT in openapi.json
(they live in `omnigent/server/routes/terminal_attach.py`). WS grounding MUST cite source.

```
GET    /api/version
GET    /health
GET    /v1/agents
GET    /v1/hosts
GET    /v1/hosts/{host_id}
POST   /v1/hosts/{host_id}/directories
GET    /v1/hosts/{host_id}/filesystem
GET    /v1/hosts/{host_id}/filesystem/{path}
POST   /v1/hosts/{host_id}/runners
GET    /v1/info
GET    /v1/me
GET,POST           /v1/policies
DELETE,GET,PATCH   /v1/policies/{policy_id}
GET    /v1/policy-registry
GET    /v1/runners
GET    /v1/runners/{runner_id}/status
GET,POST           /v1/sessions
DELETE,GET,PATCH   /v1/sessions/{session_id}
GET,PUT            /v1/sessions/{session_id}/agent
GET    /v1/sessions/{session_id}/agent/contents
GET    /v1/sessions/{session_id}/child_sessions
GET,POST           /v1/sessions/{session_id}/comments
POST   /v1/sessions/{session_id}/comments/send
DELETE,PATCH       /v1/sessions/{session_id}/comments/{comment_id}
GET    /v1/sessions/{session_id}/elicitations/{elicitation_id}
POST   /v1/sessions/{session_id}/elicitations/{elicitation_id}/resolve
POST   /v1/sessions/{session_id}/events
POST   /v1/sessions/{session_id}/hooks/antigravity-elicitation-request
POST   /v1/sessions/{session_id}/hooks/codex-elicitation-request
POST   /v1/sessions/{session_id}/hooks/cursor-permission-request
POST   /v1/sessions/{session_id}/hooks/permission-request
GET    /v1/sessions/{session_id}/items
GET    /v1/sessions/{session_id}/labels
POST   /v1/sessions/{session_id}/mcp
GET    /v1/sessions/{session_id}/owner
GET,PUT            /v1/sessions/{session_id}/permissions
DELETE             /v1/sessions/{session_id}/permissions/{target_user_id}
GET,POST           /v1/sessions/{session_id}/policies
POST   /v1/sessions/{session_id}/policies/evaluate
DELETE,GET,PATCH   /v1/sessions/{session_id}/policies/{policy_id}
GET    /v1/sessions/{session_id}/resources
GET    /v1/sessions/{session_id}/resources/environments
GET    /v1/sessions/{session_id}/resources/environments/{environment_id}
GET    /v1/sessions/{session_id}/resources/environments/{environment_id}/changes
GET    /v1/sessions/{session_id}/resources/environments/{environment_id}/diff/{relative_path}
GET    /v1/sessions/{session_id}/resources/environments/{environment_id}/filesystem
DELETE,GET,PATCH,PUT /v1/sessions/{session_id}/resources/environments/{environment_id}/filesystem/{relative_path}
GET    /v1/sessions/{session_id}/resources/environments/{environment_id}/search
POST   /v1/sessions/{session_id}/resources/environments/{environment_id}/shell
GET,POST           /v1/sessions/{session_id}/resources/files
DELETE,GET         /v1/sessions/{session_id}/resources/files/{file_id}
GET    /v1/sessions/{session_id}/resources/files/{file_id}/content
GET,POST           /v1/sessions/{session_id}/resources/terminals
DELETE,GET         /v1/sessions/{session_id}/resources/terminals/{terminal_id}
POST   /v1/sessions/{session_id}/resources/terminals/{terminal_id}/transfer
GET    /v1/sessions/{session_id}/resources/{resource_id}
GET    /v1/sessions/{session_id}/stream
POST   /v1/sessions/{session_id}/switch-agent
POST   /v1/sessions/{source_id}/fork
```

## Harness count: 19, not 16 (DRIFT)
`OMNIGENT_HARNESSES` (`omnigent/spec/_omnigent_compat.py:80`) lists **19** canonical harnesses:
antigravity, antigravity-native, claude-native, claude-sdk, codex, codex-native,
copilot, cursor, cursor-native, goose, goose-native, hermes, openai-agents,
open-responses, opencode-native, pi, pi-native, qwen, qwen-native.
Plus aliases in `OMNIGENT_HARNESS_ALIASES` (claude, native-pi, openai-agents-sdk, opencode, github-copilot, ...).
(Separately, `runtime/workflow.py:143` `AgentHarnessType` Literal lists only 7 ucode-gateway harness types — a different, narrower set. The spec's "16 harnesses" matches neither.)

## Notable grounding nuances for synthesis
- Elicitation/permission hooks are harness-specific: `codex-elicitation-request`, `cursor-permission-request`, `antigravity-elicitation-request`, plus generic `permission-request`. A spec that says only "codex permission hook" understates the surface.
- `/v1/sessions/{session_id}/elicitations/{elicitation_id}/resolve` exists (matches the permissions doc `/resolve`); verify the request body / `target_session_id` claim against `schemas.py`.
- `switch-agent` and `fork` are real POST endpoints (decisions J and the fork topology are grounded).
- `child_sessions`, `comments` (+ `comments/send`), `owner`, `permissions`, `labels`, `mcp` all exist — relevant to sub-agent topology, Bridge relay, sharing, presence.
- Spend: there is no obvious cost endpoint in the 59 paths; decision I's `total_cost_usd` likely rides on session objects/stream events — verify where `total_cost_usd` actually comes from.
