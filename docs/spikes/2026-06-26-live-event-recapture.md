# Spike findings — live event-surface recapture (Plan 4 #5)

**Date:** 2026-06-26 · **Pin:** omnigent `0.3.0.dev0` (`36b2a11c`, editable source at
`~/work/omnigent`). **Goal (Plan 4 deferral #5):** byte-verify the deferred /
`SCHEMA-DERIVED` event families the golden-SSE spike (2026-06-26) could not capture —
`reasoning_text.delta`, `session.agent_changed`, `child_session.*`, elicitation, the
chrome family — now that **native harnesses (`claude`/`cursor`) + a Cursor SDK API key**
are available on this box.

Raw corpus: [`captures/2026-06-26-live-recapture/`](./captures/2026-06-26-live-recapture/)
(15 `.sse` streams). Throwaway capture rig: `cap.sh` + ad-hoc `curl -sN` (subscribe-first),
discarded.

## What unblocked the spike (and what stayed blocked)

The blocker last time was "only claude-sdk works, and it folds reasoning." This box now has:

- **Native harnesses** launched via `omnigent claude` / `omnigent cursor` /
  `omnigent polly`. They run the vendor TUI in a tmux pane on a **persistent runner that
  survives the launcher** ("Detached. Agent still running") — so the headless launch +
  subscribe-first + REST `message` injection method works end-to-end. The foreground TTY
  attach fails harmlessly (`open terminal failed: not a terminal`); the session/runner are
  already bound by then.
- **Drive path:** `POST /v1/sessions/{id}/events` with
  `{"type":"message","data":{"role":"user","content":[{"type":"input_text","text":...}]}}`;
  the runner injects into the pane (native) or the executor (SDK).

**Key correction to the original plan:** the native TUI mirrors (claude-native,
cursor-native) **also fold reasoning into `output_text`** — they do *not* emit
`reasoning_text.delta`. Real reasoning deltas come only from the *reasoning-emitting inner
executors* (`codex`, `cursor` (SDK), `antigravity`, `pi`, `copilot`) via
`runtime/harnesses/_executor_adapter.py:848`. On this box:
- `codex` / `codex-native` → **no codex subscription** (240s idle-watchdog wedge).
- `pi` → standalone `pi` binary has no provider logged in (`No models available`); omnigent's
  ollama credential does not carry through.
- `cursor` (SDK) → required a separate, usage-billed **Cursor API key** (`crsr_…`, distinct
  from the `cursor-agent login` subscription) **plus** `pip install cursor-sdk`. Both were set
  up (key in keychain `keychain:cursor`); this is the harness that produced real reasoning deltas.

## Captured taxonomy — byte shapes + deltas vs the crate model

All `session.*` events carry `sequence_number: null`; `response.*` carry real ints (confirms
the golden-SSE seq-split, `Option<i64>` correct). Crate = `crates/lens-client/src/stream/event.rs`.

| Event (bytes) | Driver | Crate model delta |
|---|---|---|
| `response.reasoning_text.delta` `{delta, sequence_number}` | cursor SDK | ✅ matches `ReasoningTextDelta{delta}`. Was `SCHEMA-DERIVED` → now **byte-verified**. |
| `session.agent_changed` `{conversation_id, agent_id, agent_name}` | switch-agent route | ✅ matches. |
| `session.child_session.updated` `{conversation_id, child_session_id, child{id,title,tool,session_name,busy,current_task_status}}` | polly → claude_code | ⚠ **crate drops the whole `child{}` object** (`RawChildSessionUpdated._child` parsed then discarded). Consumer needs `tool`/`title`/`current_task_status`/`busy` to render sub-agents. |
| `session.created` (child spawn) `{conversation_id, child_session_id, agent_id, parent_session_id}` | polly | ⚠ **no typed match arm → degrades to `Unknown`.** Bytes now available to model. |
| `session.resource.deleted` (on agent switch) | switch-agent | ⚠ **no typed match arm → `Unknown`.** Pairs with `ResourceCreated`. |
| `session.model` `{conversation_id, model}` | `external_model_change` | ✅ matches `Model{model}`. |
| `session.reasoning_effort` `{conversation_id, reasoning_effort}` | `external_reasoning_effort_change` | ✅ matches. |
| `session.todos` `{conversation_id, todos:[{content, status, activeForm}]}` | claude-native TodoWrite | ✅ matches — `RawTodoItem` already `#[serde(rename="activeForm")]` (+ test), status enum matches. Was `SCHEMA-DERIVED` → now **byte-verified** (16 events, full lifecycle). |
| `response.compaction.in_progress` `{}` (bare) | `compact` | ✅ matches unit `CompactionInProgress`. |
| `response.cancelled` `{response{...full obj..., status:"cancelled"}}` | interrupt mid-turn | crate is unit `Cancelled` (response obj discarded — acceptable for a marker). |
| `session.interrupted` `{data:{requested_at, response_id}}` | interrupt | crate gets `requested_at`, **drops `response_id`**; note the `data` nesting. |
| `session.terminal.activity` `{session_id, terminal_id}` | any native turn | ✅ `terminal_id` (crate drops `session_id`). **Arrives via SSE — no WS attach needed**, contra the Plan-7 assumption. |
| `response.elicitation_request` `{elicitation_id, method:"elicitation/create", params:{mode, message, url, requestedSchema, phase, policy_name, content_preview, target_session_id}}` | policy agent (`ask_on_os_tools`) | ⚠ **crate keeps only `elicitation_id`** (`RawElicitationRequest._params` discarded). Consumer needs `message`/`content_preview`/`policy_name` to render the approval card. Same shape mirrored in snapshot `pending_elicitations`. |
| `response.elicitation_resolved` `{elicitation_id}` | `approval` accept | ✅ matches (no verdict echoed in the event). |
| `session.skills`, `response.heartbeat`, `response.failed` | incidental | grounded; shapes consistent with crate/openapi. |

### Driving notes (reusable)
- **agent switch:** `POST /v1/sessions/{id}/switch-agent {"agent_id":"ag_…"}` — fires
  `session.agent_changed` + `session.resource.deleted`; the response is a full snapshot.
- **child sessions:** `omnigent polly`, prompt it to delegate to **only** its `claude_code`
  sub-agent (its `codex`/`pi` sub-agents are broken here). `current_task_status` cycles
  `launching → in_progress → completed`.
- **model / effort / compaction:** `POST /events` `external_model_change`,
  `external_reasoning_effort_change`, `compact` — all return `{"queued":false}`.
- **todos:** must explicitly instruct the agent to *use TodoWrite*; claude-native forwards
  `PostToolUse`/`TodoWrite` hooks → `external_session_todos` → `session.todos`.
- **interrupt:** `POST /events {"type":"interrupt","data":{}}` mid-turn → `session.interrupted`
  + `response.cancelled`.
- **elicitation:** author a bundle dir with `config.yaml` declaring a guardrails policy
  (top-level `policies:` + `handler:` did **not** gate — must be
  `guardrails: policies: <name>: {type: function, on: [tool_call], function: {path: omnigent.policies.builtins.safety.ask_on_os_tools}}}`),
  run with `--tools coding`, ask it to write a file. Resolve with
  `POST /events {"type":"approval","data":{"elicitation_id":"…","action":"accept"}}`.

## Still blocked (environment, not modeling)

- `turn.*` (`turn.started/completed/failed/cancelled`) — emitted **only** by
  `codex_native_forwarder` (Codex app-server protocol). Needs a codex subscription. → stays
  `DEFERRED → Unknown`.
- `response.created` / `response.queued` — scaffold emits `response.created` but the runner
  defers it before the session stream (`runner/app.py:11912`); the surfacing path is the
  openai-agents / open-responses scaffold harnesses (need an OpenAI key). Never observed on the
  wire here.
- `response.reasoning_summary_text.delta` — Codex `summaryTextDelta` only; cursor SDK emits
  `reasoning_text.delta` but not the summary variant. → stays schema-derived.
- `response.compaction.completed` — requires a **configured `llm_model`**; every harness here
  uses subscription auth (`llm_model: null`), so only `in_progress` is reachable
  (`compact` errors `Compaction requires a configured LLM model`).
- `session.sandbox_status` — needs sandbox mode (`os_env.sandbox.type != none`); not driven.
- `session.terminal_pending`, `session.model_options` — situational; not surfaced by the turns driven.

## Disposition

- **Deliverable = capture only** (decided up-front). No crate changes this session; the deltas
  above feed a follow-on modeling plan that flips the byte-verified families
  `SCHEMA-DERIVED → MODELED` and grows the under-modeled payloads.
- **Highest-value modeling deltas** (consumer-facing, not just re-flagging):
  1. `child_session.updated` — expose the `child{}` object.
  2. `response.elicitation_request` — expose `params` (message/content_preview/policy_name) —
     the approval card can't render without it.
  3. `session.created` (child) + `session.resource.deleted` — promote from `DEFERRED_EVENT_TYPES`
     to typed arms (bytes now available).
  (`session.todos`, `session.model`, `session.reasoning_effort`, `reasoning_text.delta` already
  match the wire — just flip their `SCHEMA-DERIVED` flags to byte-verified.)
- Reconcile typed-client §7: terminal activity is an **SSE** event (drop the WS-only assumption
  for `session.terminal.activity`); fold the three still-blocked families' reasons.
