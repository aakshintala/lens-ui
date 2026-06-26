# Spike findings — golden-SSE capture (Plan 3, capture-from-bytes)

**Date:** 2026-06-26 · **Pin:** omnigent `0.3.0.dev0` (`36b2a11c`, source build).
**Goal (Plan 3 step 1):** capture real SSE streams from the live pinned server and
model the typed `ServerStreamEvent` taxonomy from *bytes*, not the under-specified
openapi — the same capture-from-bytes discipline that would have caught the 4
guessed-envelope bugs in REST surface 2c–2e.

Raw captures preserved under [`captures/2026-06-26-sse/`](./captures/2026-06-26-sse/)
(happy-path stream/snapshot/items, interrupt stream, reasoning-effort-high stream,
a failed-session snapshot with an error item). Capture rig was a throwaway bash
harness (subscribe-first `curl -sN .../stream` → `POST /events` → drain → snapshot+items).

## Method
Drive a real claude-sdk turn, capture subscribe-first. The persistent daemon runner
**survives the `omnigent run` client exit**, so after warming once we attach our own
stream BEFORE posting the next message (no-replay → subscribe-first is mandatory) and
get a clean live capture. Snapshot (`?include_items&include_liveness`) + `GET /items`
captured after each turn for the bucket-A/bucket-B shapes.

## Captured stream event taxonomy (13 types, from bytes)

| `event:` / `type`                      | payload keys                                                        | bucket |
|----------------------------------------|---------------------------------------------------------------------|--------|
| `response.in_progress`                 | `response, sequence_number`                                         | C |
| `response.output_text.delta`           | `delta, message_id, index, final, sequence_number`                  | C |
| `response.reasoning.started`           | `sequence_number` (bare bracket — no payload)                       | C |
| `response.output_item.done`            | `item, sequence_number`                                             | A |
| `response.completed`                   | `response, sequence_number` (`response.output` is `[]` — already flushed) | A |
| `session.status`                       | `conversation_id, status, response_id, error, sequence_number`      | B |
| `session.usage`                        | `conversation_id, context_tokens, context_window, total_cost_usd, usage_by_model, sequence_number` | B |
| `session.presence`                     | `conversation_id, viewers, sequence_number`                         | C |
| `session.heartbeat`                    | `server_time, sequence_number`                                      | C |
| `session.resource.created`             | `resource, sequence_number`                                         | B |
| `session.input.consumed`  ⚠ undocumented | `data:{item_id, type, data}, sequence_number`                     | C (send-ack) |
| `session.changed_files.invalidated` ⚠ undocumented | `session_id, environment_id, sequence_number`           | C |
| `session.interrupted`     ⚠ undocumented | `data, sequence_number`                                           | C |

**`response.output_item.done` → `item.type` union (from bytes):**
- `function_call` — `{agent, arguments, call_id, id, name, status}`
- `message` — `{content, id, model, response_id, role, status}`
- `function_call_output` — `{arguments, call_id, id, output, response_id, status}`

## Load-bearing confirmations (vs typed-client.md §7)

1. **`sequence_number` null-vs-int split.** Only `response.*` stream events carry real
   ints (`reasoning.started`=3, `output_text.delta`=4…, `completed`=17). EVERY
   `session.*` chrome event has `sequence_number: null`. → sequence-dedup applies only
   to the `response.*` overlap window (bucket C). `Option<i64>` is correct.
2. **Persisted `GET /items` carry NO `sequence_number`** → merge-by-item-`id` (§7) is right.
3. **Snapshot `?include_items` exposes full chrome** for bucket-B reconstruction:
   `status, agent_id, agent_name, llm_model, model_override, model_options,
   reasoning_effort, todos, pending_elicitations, pending_inputs, sandbox_status,
   terminal_pending, usage_by_model, total_cost_usd, skills, archived, labels,
   runner_online, host_online, last_task_error, …`.

## Deltas vs the design (feed back into typed-client.md §7 + the enum)

- **Three undocumented stream events** must be added to the §7 reconnect classification
  in lockstep: `session.input.consumed` (C), `session.changed_files.invalidated` (C),
  `session.interrupted` (C).
- **Chrome naming:** snapshot uses `llm_model` + `model_override` (NOT `model`);
  `usage_by_model` + `total_cost_usd` are TOP-LEVEL on the snapshot (not under `usage`).
- The streamed `function_call` item lacks the `model` key the persisted one carries.

## Error / failed family (from failed sessions, bytes)

- **Persisted error item (bucket A):** `{type:"error", status:"completed", response_id,
  created_at, data:{source:"execution", code:<exception, e.g. ImportError/RuntimeError>,
  message}, created_by}`.
- **Chrome `last_task_error`:** `{code:"runner_error" (coarse category), message}` —
  note the item's `data.code` is the specific exception; `last_task_error.code` is the category.
- **`POST /events` HTTP validation error envelope (§11, distinct from the stream item):**
  `{"error":{"code":"invalid_input", "message":…}}`.

## Control-event acks (bytes)

- `interrupt` → ack `{queued:false}` + stream event `session.interrupted`.
- All control events report `queued:false` (vs `message` → `{queued:true, item_id}`).
- `compact` → blocked here: `{"error":{"code":"invalid_input","message":"Compaction
  requires a configured LLM model"}}` (claude-sdk session has `llm_model:null`).

## Not captured — environmentally blocked (model from schema / needs setup)

The only working harness on this box is **claude-sdk**, and it FOLDS reasoning into
`output_text` (emits a bare `response.reasoning.started`, no `reasoning_text.delta`).
Blockers found:
- **codex** binary quarantined as malware (deleted by the OS).
- **No `OPENAI_API_KEY`** set → `openai-agents` / `open-responses` / `codex` all fail
  `runner_error: ... no OPENAI_API_KEY or OPENAI_BASE_URL`.
- **claude-native** and **cursor** harnesses exit 1 locally / bind no persistent runner
  (harnesses outside the persistent set `[claude-native, claude-sdk, codex,
  openai-agents, open-responses, pi]` get an ephemeral runner that dies with the
  `omnigent run` process → no clean subscribe-first 2nd-turn capture).

Consequently model these from the openapi schema (flag as schema-derived, not
byte-verified), or capture later when the environment supports them:
- `response.reasoning_text.delta` / `response.reasoning_summary_text.delta`
  (trivial `{delta, sequence_number, type}`).
- Live `response.failed` / `session.status:"failed"` stream events (the `session.status`
  shape `{status, response_id, error}` IS captured; failure just populates `error`).
- `compact` ack (needs a configured `llm_model`).
- elicitation / approval flow (needs a gating policy).
- `child_session.*` / sub-agent events (needs a multi-agent agent config).
- terminal activity (`session.terminal.activity`, `session.terminal_pending`) — WS attach.
- `stop_session` ack — not driven (reclaims the runner; deferred to keep the warm runner).

## Disposition
- Capture rig discarded (bash, scratchpad). Raw captures kept under `captures/`.
- Next: write the Plan 3 implementation plan; model the `ServerStreamEvent` enum from
  the captured bytes above (schema-derived only where blocked, clearly flagged), and
  reconcile the three undocumented events into typed-client.md §7.

## Schema-derived variants pending byte-verification

Flagged `// SCHEMA-DERIVED (not byte-verified — re-capture at config-time)` in
`crates/lens-client/src/stream/event.rs` (Task 6, Plan 3a). Re-capture when the
harness environment supports each family.

**ResponseEvent (`response.*`):**
- `Failed` — `response.failed`
- `Incomplete` — `response.incomplete`
- `Cancelled` — `response.cancelled`
- `ReasoningTextDelta` — `response.reasoning_text.delta`
- `ReasoningSummaryTextDelta` — `response.reasoning_summary_text.delta`
- `CompactionInProgress` — `response.compaction.in_progress`
- `CompactionCompleted` — `response.compaction.completed`
- `CompactionFailed` — `response.compaction.failed`
- `Error` — `response.error`
- `ElicitationRequest` — `response.elicitation_request`
- `ElicitationResolved` — `response.elicitation_resolved`

**SessionEvent (`session.*`):**
- `ChildSessionUpdated` — `session.child_session.updated`
- `TerminalActivity` — `session.terminal.activity`
- `TerminalPending` — `session.terminal_pending`
- `Model` — `session.model`
- `Todos` — `session.todos`
- `ReasoningEffort` — `session.reasoning_effort`
- `ModelOptions` — `session.model_options`
- `SandboxStatus` — `session.sandbox_status`
- `Skills` — `session.skills`
