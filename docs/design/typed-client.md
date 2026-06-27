# The typed client (`lens-client`)

The Rust crate that owns all knowledge of omnigent's HTTP + SSE + WS contract.
It is the single seam between the app and the server: the rest of Lens speaks
Lens's canonical item model (pinned by the application architecture & state
model document), and this crate translates between that and omnigent's wire
shapes. A contract change is a localized edit here + a regenerated type, not an
app-wide ripple.

**Status:** Draft, 2026-06-23.
**Depends on:** nothing (this is the foundation).
**Ground truth:** `omnigent-ai/omnigent` pinned at `0.3.0.dev0` (package semver; HEAD `36b2a11c`) — `openapi.json` (the typed API surface; note its `info.version` is a stale `"0.1.0"` — trust the package semver and the route source, not `info.version`), `omnigent/server/schemas.py` (Pydantic models behind the openapi), `omnigent/server/routes/` (the route handlers), `omnigent/server/routes/terminal_attach.py` (the WS terminal path, not in openapi).

---

## 1. Scope & boundaries

**This crate owns:**

- The HTTP client — typed requests + responses for every endpoint in `openapi.json`.
- The SSE stream parser — `GET /v1/sessions/{id}/stream`, the full event taxonomy, `sequence_number` dedup.
- The WS terminal attach client — `WS /v1/sessions/{id}/resources/terminals/{id}/attach` (the `/v1` prefix IS required — the router is mounted with `prefix="/v1"` at `app.py:1635-1642`; the bare `terminal_attach.py:130` path is router-relative. Not in openapi, read from source).
- The no-replay reconnect protocol — snapshot + history + reopen + dedup.
- The contract-version gate — `GET /api/version` (the semver source), refuse-to-start on mismatch. (`GET /v1/info` is the unauthenticated capability/auth probe; `GET /health` is liveness — neither carries a version.)
- Per-connection auth — the credential the HTTP/WS clients present for this connection.
- Codegen scaffolding off `openapi.json` + the hand-written typed enum layer on top.

**This crate does NOT own:**

- The view-model / state store / command flow (the application architecture & state model document).
- *How* events are rendered (the conversation transcript, workspace, permissions, sub-agent topology documents).
- Server spawn/supervise (the server lifecycle document owns the process lifecycle; this crate is given a base URL + credential and told to talk to it).
- What the connection *means* to the user (the application shell owns the connections surface and health rollout).

The crate exposes a `Client` per connection. The state model creates one
`Client` per omnigent server Lens is attached to, and the app holds N at once.

---

## 2. The connection model

Lens is a **multi-connection client** — it talks to N omnigent servers at once
(local-spawned + one or more remote-only). Each connection is an instance of:

```rust
pub struct Connection {
    pub id: ConnectionId,        // branded, Lens-local
    pub base_url: Url,           // e.g. http://localhost:8000, https://omnigent.internal.dev
    pub auth: Auth,              // None | Bearer(String) | Cookie(String) | ForwardedEmail(String)
    pub info: ServerInfo,        // from GET /v1/info; pinned at handshake
}

pub enum Auth {
    None,                        // localhost — no auth
    Bearer { token: String },    // bearer header
    Cookie { value: String },    // cookie header
    ForwardedEmail { email: String }, // X-Forwarded-Email header
}
```

The crate **never** assumes auth is `None`. The server lifecycle document spawns
the local connection with `Auth::None`; a remote connection (e.g. an internal
dev workspace) is constructed with whatever credential the user supplied in the
"add connection" flow. Every HTTP request and WS upgrade from this connection
inserts the auth.

The contract gate runs once per connection, at handshake (§8).

---

## 3. The HTTP surface

All paths under `/v1/...` (plus the unversioned `/api/version` + `/health`). The
crate generates Rust types from `openapi.json` and the hand-written `Client`
method layer wraps them. Verbatim paths from `openapi.json` @ `0.3.0.dev0`:

### Sessions — the primary object

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/v1/sessions` | fleet poll — cursor (`after`/`before`), `kind=default\|sub_agent\|any`, `search_query`, `include_archived`, `agent_id`/`agent_name` filters |
| `POST` | `/v1/sessions` | create a session — multipart: `SessionCreateMetadata` (with optional bundled agent spec) or simple JSON; `host_type: external\|managed`; `git{branch_name, base_branch?}` + `host_id` |
| `GET` | `/v1/sessions/{id}` | snapshot — `include_items?`, `include_liveness?` params |
| `PATCH` | `/v1/sessions/{id}` | update — `runner_id` (bind), `archived`, `silent`, `labels`, `model_override`, `reasoning_effort`, `cost_control_mode_override`, `collaboration_mode`, `terminal_launch_args`, `external_session_id` |
| `DELETE` | `/v1/sessions/{id}` | delete — `?delete_branch=true` cleans the worktree |
| `GET` | `/v1/sessions/{id}/stream` | the SSE stream (§4) |
| `POST` | `/v1/sessions/{id}/events` | send an event into the session (§6 — generalized `SessionEventInput` body) |
| `GET` | `/v1/sessions/{id}/items` | history — paginated conversation items |
| `POST` | `/v1/sessions/{source_id}/fork` | fork — `SessionForkRequest` (clone conversation onto a new session) |
| `PUT` | `/v1/sessions/{id}/agent` | bundle upload/storage only — same-name, idempotent on unchanged content; **does NOT fire `session.agent_changed`** |
| `GET` | `/v1/sessions/{id}/agent/contents` | fetch the agent bundle's contents (runner/debug only — not a Lens UX endpoint) |
| `POST` | `/v1/sessions/{id}/switch-agent` | **the actual switch-agent path** (verified `omnigent/server/routes/sessions.py:14214`, body `SessionSwitchAgentRequest`) — emits `session.agent_changed` (`sessions.py:14353`); rejects sub-agents + no-op swaps. **API floor = `LEVEL_EDIT` (2), not owner** (`_require_access_and_level(..., LEVEL_EDIT, ...)`, docstring "403 if the caller lacks edit access"). Idle guard rejects cached `running` (and `waiting`, which the cache collapses to `running`) but **not `launching`** (falls through to `idle`). Owner-only + idle-only is a Lens UI policy (decision J) layered on top — **not** the API contract |

**The `PUT /v1/sessions/{id}/agent` endpoint above is bundle storage, not the switch trigger.** `POST /switch-agent` is the path that actually fires the agent swap (and the `session.agent_changed` event). The crate models both: `PUT /agent` for bundle storage, `POST /switch-agent` for the swap. The agent definition document owns which UI action triggers which. Note `session.agent_changed` carries only `agent_id` + `agent_name` (`schemas.py:2218-2221`) — **no model/skills payload**; consumers must refetch the session snapshot for the new agent's model/skills, and synthesize the `from`-agent from prior reducer state.

### Session resources — environments, filesystem, terminals, files

**Environment-scoped workspace endpoints** (the 0.2.0 path change — these
endpoints are now environment-scoped):

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/v1/sessions/{id}/resources` | list resources |
| `GET` | `/v1/sessions/{id}/resources/{resource_id}` | fetch one resource |
| `GET` | `/v1/sessions/{id}/resources/environments` | list environments |
| `GET` | `/v1/sessions/{id}/resources/environments/{env_id}` | fetch one env |
| `GET` | `/v1/sessions/{id}/resources/environments/{env_id}/filesystem` | top-level fs listing |
| `GET` | `/v1/sessions/{id}/resources/environments/{env_id}/filesystem/{relative_path}` | read a file |
| `GET` | `/v1/sessions/{id}/resources/environments/{env_id}/changes` | changed-files list |
| `GET` | `/v1/sessions/{id}/resources/environments/{env_id}/diff/{relative_path}` | diff — returns `{before, after}` strings, NOT unified diff |
| `POST` | `/v1/sessions/{id}/resources/environments/{env_id}/search` | server-side search — substring + glob include/exclude, cap 500 |
| `POST` | `/v1/sessions/{id}/resources/environments/{env_id}/shell` | one-shot shell command |
| `GET/POST` | `/v1/sessions/{id}/resources/files` | list / upload file resources |
| `GET` | `/v1/sessions/{id}/resources/files/{file_id}` | file metadata |
| `GET` | `/v1/sessions/{id}/resources/files/{file_id}/content` | file content |
| `GET/POST` | `/v1/sessions/{id}/resources/terminals` | list / create terminals |
| `DELETE` | `/v1/sessions/{id}/resources/terminals/{terminal_id}` | destroy a terminal |
| `POST` | `/v1/sessions/{id}/resources/terminals/{terminal_id}/transfer` | transfer a terminal to a different session (live `/clear` rotation) |

**Terminal WS attach IS under `/v1/`** — it lives at
`ws://{host}/v1/sessions/{session_id}/resources/terminals/{terminal_id}/attach`.
The router defines the bare `/sessions/.../attach` path (`terminal_attach.py:103-130`)
but `create_app` mounts that router with `prefix="/v1"` (`app.py:1635-1642`), so the
external URL carries `/v1`. Runner proxy + ap-web both use the `/v1`-prefixed URL.
Section 5 covers it.

### Sub-agents & comments

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/v1/sessions/{id}/child_sessions` | list child sessions (returns `ChildSessionSummary`; **caveat below**) |
| `POST` | `/v1/sessions/{id}/comments` | add a line comment — `AddCommentRequest{path, start_index, end_index, anchor_content?, body}` |
| `PATCH` | `/v1/sessions/{id}/comments/{comment_id}` | edit a comment |
| `DELETE` | `/v1/sessions/{id}/comments/{comment_id}` | delete a comment |
| `POST` | `/v1/sessions/{id}/comments/send` | send comments to the agent as feedback (Review → send-to-agent) |

**Caveat on `ChildSessionSummary`:** the schema is **not exposed in
`openapi.json` `components/schemas`** — only the event
`SessionChildSessionUpdatedEvent` is. Codegen off `openapi.json` won't produce a
named type for the `GET child_sessions` response. The crate hand-writes a
`ChildSessionSummary` mirror struct from `omnigent/server/schemas.py:558` and
adds it to the contract-test list (§9). **The live event carries a PARTIAL
summary** (per the openapi description: "a status delta omits
`last_message_preview`; a preview delta carries only it" — an open dict, not the
strict model); the **full** summary arrives only on the snapshot/`GET
child_sessions`. So the mirror's event-carried fields are `Option`, and the
state model **merges present fields over the cached child row** rather than
replacing it.

### Elicitations, hooks, labels, permissions, owner, policies

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/v1/sessions/{id}/elicitations/{elicitation_id}` | fetch pending state (deep-linkable) |
| `POST` | `/v1/sessions/{id}/elicitations/{elicitation_id}/resolve` | RESTful resolve — body `ElicitationResult{action: accept\|decline\|cancel, content?}` |
| `POST` | `/v1/sessions/{id}/hooks/permission-request` | generic/claude-native permission hook |
| `POST` | `/v1/sessions/{id}/hooks/codex-elicitation-request` | codex elicitation hook |
| `POST` | `/v1/sessions/{id}/hooks/antigravity-elicitation-request` | antigravity elicitation hook (`openapi.json:5739`) |
| `POST` | `/v1/sessions/{id}/hooks/cursor-permission-request` | cursor permission hook (`openapi.json:5821`) |
| `GET/PUT/DELETE` | `/v1/sessions/{id}/labels` | free-form labels |
| `GET` | `/v1/sessions/{id}/mcp` | session MCP state |
| `GET` | `/v1/sessions/{id}/owner` | owner |
| `GET/PUT/DELETE` | `/v1/sessions/{id}/permissions[/{target_user_id}]` | per-session permission grants — **grantable levels 1–3 only** (`GrantPermissionRequest.level = Field(ge=1, le=3)`, `schemas.py:1905`): 1=read, 2=edit, 3=manage. Owner (4) is creator/admin-derived and **not grantable** (owner grants 403). `__public__` is capped at read |
| `GET/POST` | `/v1/sessions/{id}/policies` | session-scoped policies |
| `GET/DELETE` | `/v1/sessions/{id}/policies/{policy_id}` | one policy |
| `POST` | `/v1/sessions/{id}/policies/evaluate` | evaluate a policy against hypothetical input |

**The four `…/hooks/*` paths are server-initiated** (harness-side adapters POST them inbound; Lens never calls them). Lens does not own these as client requests — but the elicitation UI and the `external_elicitation_resolved` race handling must tolerate all four harness sources (generic `permission-request`, `codex-elicitation-request`, `antigravity-elicitation-request`, `cursor-permission-request`).

### Agents, hosts, runners, server info

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/v1/agents` | list agents (read-only; **no REST CRUD** — authoring is filesystem YAML + bundle upload) |
| `GET` | `/v1/hosts[/{host_id}]` | hosts registry — **read-only; there is NO `POST`/`DELETE /v1/hosts`.** Host registration is outbound-WS-tunnel/daemon based (`omnigent host` / `host_tunnel.py`) + managed provisioning, not REST CRUD |
| `POST` | `/v1/hosts/{id}/directories` | create a directory on the host (owner-scoped) |
| `GET` | `/v1/hosts/{id}/filesystem[/{path}]` | browse host filesystem (useful for new-session repo-picking) |
| `POST` | `/v1/hosts/{id}/runners` | launch a runner on a host — `LaunchRunnerRequest{session_id, workspace, git?}` |
| `GET/POST/PATCH/DELETE` | `/v1/policies[/{policy_id}]` | server-wide policies |
| `GET` | `/v1/policy-registry` | policy catalog (what can be attached) |
| `GET` | `/v1/runners` | list runners |
| `GET` | `/v1/runners/{runner_id}/status` | runner status — `{runner_id, online}`; check before a wake/rebind relaunch |
| `GET` | `/api/version` | **the contract-gate source** — `{"version": "<semver>"}` (`app.py:1479`) |
| `GET` | `/v1/info` | unauthenticated runtime capability/auth probe — `accounts_enabled, login_url, needs_setup, databricks_features, managed_sandboxes_enabled, sandbox_provider`. **No version field** (`app.py:1493`) |
| `GET` | `/health` | liveness only |
| `GET` | `/v1/me` | auth identity — used by ownership chrome |

---

## 4. The SSE stream

`GET /v1/sessions/{id}/stream` (openapi:7819). **Live-tail, no replay** — the
server does not buffer past events. The crate opens one stream per *active*
session; the state model's liveness layer decides which sessions are active —
**no hard stream cap**: the active set self-bounds via 10-min terminal-aware
auto-sleep (state-model §3.3). (An earlier draft cited a "~8 concurrent streams"
cap; that cap was removed — do not reintroduce it.) Sleeping sessions are
repoll-ed for status via `GET /v1/sessions`.

Every event carries `sequence_number: Option<i64>` for dedup. Heartbeats carry
additional gap-detection fields:

- `response.heartbeat`: `last_event_seq: Option<i64>` + `sequence_number: Option<i64>` + `server_time: Option<String>` (ISO 8601 UTC).
- `session.heartbeat`: `sequence_number: Option<i64>` only.

The crate uses these as: on a gap (last seen `sequence_number` jumps), **drop
in-progress accumulators and resync** from snapshot (§7). Heartbeats also drive
stall-detection — a missed cadence (default 15s per `omnigent/runtime/workflow.py`)
without any event signals a stalled producer.

The SSE parser uses a dedicated OS thread that holds the HTTP body stream
(`reqwest` blocking) and writes parsed events into an `Arc<Mpsc>` of typed
`ServerStreamEvent`s; a UI-thread poller drains it via `cx.background_spawn` +
`cx.notify()`. This is the Arbor-pattern bridge proven in the framework
reconnaissance (framework §2.1).

---

## 5. The terminal WS attach

Live terminal I/O is **not** over HTTP — it's a WebSocket at
`ws://{host}/v1/sessions/{id}/resources/terminals/{terminal_id}/attach` (the
`/v1` prefix comes from the router mount at `app.py:1635-1642`; the bare path in
`terminal_attach.py:103-130` is router-relative). Frames are binary PTY bytes
inbound/outbound; control is text JSON (`{"type":"resize", ...}`). A `read_only`
query param controls write access.

- **Read-only by default** — the server attaches via `tmux attach -r` (read-only
  tmux mode). Write attach requires `LEVEL_OWNER` (the session owner).
- **Transferable** — `POST …/terminals/{id}/transfer` moves a terminal to a new
  session without closing it.
- **No replay buffer** — live attach only. Reconnect loses scrollback;
  `workspace-and-terminals.md` decision §0.7-C pins the Lens-side ring buffer
  for reconnect scrollback.
- `session.terminal.activity` is delivered on the **SSE stream** (byte-verified
  2026-06-26 — `docs/spikes/2026-06-26-live-event-recapture.md`), not via the WS
  terminal attach; it signals which terminal is active so the workspace drawer can
  highlight/focus without opening an attach. Live terminal **content** remains
  WS-only (above). `session.terminal_pending` (0.2.0 net-new) also drives the
  workspace drawer.

The crate's WS client uses `tungstenite` (the same crate the recon's Arbor
reference uses) wrapped in the same thread → channel → UI-poller bridge as the
SSE parser.

---

## 6. The generalized `/events` body (load-bearing)

In 0.2.0, `POST /v1/sessions/{id}/events` takes a discriminated
`SessionEventInput`:

```
{ "type": "<discriminator>", "data": { <type-specific payload> } }
```

`type` is a string; `data` is a free-form JSON object. So the crate
**cannot** model each dispatch (`approval`, `interrupt`, `fork`, etc.) as a
distinct request schema — it must serialize a typed Rust enum into `data`.

The typed Rust surface:

```rust
pub enum SessionEventInput {
    Message { /* text content blocks */ },
    FunctionCallOutput { /* client tool result */ },
    Approval { elicitation_id: ElicitationId, result: ElicitationResult },
    Interrupt { /* optional target tool call id */ },
    Compact,                     // request context compaction (_COMPACT_TYPE)
    StopSession,                 // terminate the live session (reclaim runner)
    // Lens SENDS only the subset above. But the route's _ALLOWED_EVENT_TYPES
    // (sessions.py:771) is much larger — it unions ITEM_TYPE_TO_DATA_CLS keys
    // (message, function_call_output, slash_command, …) with: interrupt,
    // approval, mcp_elicitation, compact, stop_session, AND the full
    // external_* forwarding family (external_assistant_message,
    // external_conversation_item, external_output_text_delta,
    // external_output_reasoning_delta, external_session_interrupted,
    // external_elicitation_resolved, external_session_status,
    // external_session_usage, external_compaction_status, external_model_change,
    // external_reasoning_effort_change, external_session_todos,
    // external_subagent_start, external_codex_subagent_start,
    // external_codex_collaboration_mode_change). The contract-test parser/
    // validator must ACCEPT the full dispatch table even though Lens only sends
    // the subset, so an unrecognized type from a forwarder doesn't crash the
    // round-trip. NOTE: **fork is NOT here** — it is a dedicated endpoint,
    // POST /v1/sessions/{source_id}/fork, not an /events dispatch.
}

impl SessionEventInput {
    /// Serialize into the wire shape: { "type": "<discrim>", "data": <payload> }
    pub fn to_json(&self) -> serde_json::Value { /* ... */ }
}
```

The discriminators are read from `openapi.json`'s `SessionEventInput` schema +
the route handler's dispatch table (`omnigent/server/routes/sessions.py`). The
contract-test suite (§9) pins them — adding a new `type` requires a crate bump.

**Approval reply** — previously an open question (the discriminator wasn't
enumerable). Verified against the source: `type == "approval"`,
with `ElicitationResult { action: accept|decline|cancel, content? }` in `data`.
The dedicated `POST /v1/sessions/{id}/elicitations/{elicitation_id}/resolve`
endpoint is the RESTful counterpart (cleaner for url-mode OAuth) and is the
preferred path when an `elicitation_id` is on hand.

---

## 7. The no-replay reconnect protocol

SSE is no-replay. Correct reconnect:

1. **Detect disconnect** — stream closes or errors. Record `last_seen_seq`
   from the most recent heartbeat.
2. **Short retry phase** — exponential backoff, 3s cap:
   `100ms → 200ms → 400ms → 800ms → 1600ms → 3000ms → 3000ms → …`. The crate
   emits `ServerStreamEvent::Reconnecting { attempt }` on each retry, so the
   application shell can raise its "↻ Reconnecting" indicator immediately on
   disconnect — not gated on this phase completing. Total wall-clock before
   giving up: ~7s.
3. **Give up** — if the retry phase expires, the crate emits a terminal
   `ServerStreamEvent::Disconnected { reason: DisconnectReason }` (the last event
   before the channel closes; `recv()` returns `None` after) and stops. Because
   reconnect runs *after* the stream is open, give-up cannot be a synchronous
   `Result` — it is a stream value, **not** a `ClientError` variant. The typed
   `reason` (see the stop-immediately table) tells the app whether to re-auth,
   remove the session, surface a failure, or offer a manual retry. A
   user-initiated retry reopens via `Sessions::stream()`, restarting from step 2.
4. **On success — snapshot (bucket B: chrome)** — `GET /v1/sessions/{id}` (with
   `include_items=true, include_liveness=true`) confirms the session exists and
   **restores all session chrome** (status, usage, model, todos, model_options,
   reasoning_effort, collaboration_mode, skills, archived, presence-count, …).
   This chrome is **SSE-only/transient on the wire** — it is NOT replayed from
   `GET /items`; it is reconstructed from the snapshot. The crate emits the
   restore as a **single synthetic `ServerStreamEvent::SnapshotRestored(SessionSnapshot)`**
   carrying the typed snapshot payload (the consumer never applies the snapshot
   itself — see the ownership decision below). Do not expect a
   `session.agent_changed` marker on wake — the snapshot's current
   `agent_id`/`agent_name` ride inside `SnapshotRestored`, and the reducer's
   arm for it folds the scalar **without** the live-transition transcript
   side-effect (no spurious `AgentChanged` item).
5. **History (bucket A: transcript)** — `GET /v1/sessions/{id}/items` fills the
   durable conversation items. **Persisted `ConversationItem` has no
   `sequence_number`** (`entities/conversation.py:644` — only `id`), so items are
   merged into the transcript by **item `id`** (idempotent upsert), NOT by
   sequence. The rest are replayed into the stream as `ServerStreamEvent` values.
6. **Emit `Reconnected { gap }` → `SnapshotRestored`** — before any replayed
   history items, in this order: `Reconnected { gap }` first (so the state model
   clears transient accumulators when `gap != Some(0)`), then
   `SnapshotRestored(SessionSnapshot)` (bucket-B chrome), then the replayed
   `GET /items` history (bucket A). The ordering is load-bearing: transient state
   is cleared, chrome is restored, *then* history lands.
7. **Re-open stream + dedup (bucket C: live overlap)** — `GET /v1/sessions/{id}/stream`.
   `sequence_number` dedup applies **only to the live SSE overlap window**: events
   that fire between the snapshot/history read and the stream re-opening may arrive
   twice. Discard *stream* events whose `sequence_number` is ≤ `last_seen_seq`.
   (This dedup never touches `GET /items` rows — they carry no `sequence_number`.)
   A gap (sequence jumps) triggers another snapshot+history resync.

**Three-bucket model (load-bearing):** (A) **item-backed** transcript from
`GET /items`, merged by item `id`; (B) **snapshot-restored** chrome, applied from
`GET /v1/sessions/{id}` scalars/collections; (C) **truly transient** stream
deltas/heartbeats/presence that are never persisted and only re-derive on the next
live event. `sequence_number` is a live-stream dedup key only — never an item key.

### Stop-immediately conditions (no retry)

Each maps to a `DisconnectReason` carried by the terminal `Disconnected { reason }`:

| Error | `DisconnectReason` | Meaning | App action |
|---|---|---|---|
| 401 | `Unauthorized` | Auth invalid or expired | The connection-auth model prompts re-auth |
| 403 | `Forbidden` | Lost permission to session | Show access-denied, remove session |
| 404 | `NotFound` | Session deleted | Remove session from UI |
| `session.status = "failed"` in snapshot | `SessionFailed` | Server-side terminal failure | Surface error, no retry |
| backoff window elapsed | `RetriesExhausted` | Transient drop never recovered | Offer a manual retry (re-`stream()`) |

`DisconnectReason` is a stream value, not a `ClientError`. The crate's
`stop_reason` maps HTTP status off the snapshot/open-stream call: **401/403 →
`Unauthorized`/`Forbidden`** (encoded as `ClientError::Auth`), **404 →
`NotFound`** (encoded as `ClientError::Server { status: 404 }`); every other
error (network/5xx/parse) is **retryable** and keeps backing off.

**`SessionFailed` is terminal, not a resume.** When the reconnect snapshot
reports `status == failed`, the crate emits `SnapshotRestored(snapshot)` (so the
UI gets the terminal failure state) immediately followed by
`Disconnected { reason: SessionFailed }` — it does **not** emit `Reconnected`
(no live tail resumes), and it does not reset the normalizer.

### `Reconnected { gap: Option<u64> }`

- `gap = Some(0)` — clean reconnect, nothing missed. Keep in-memory rendered
  state and continue.
- `gap = Some(N > 0)` or `gap = None` — clear all transient accumulators
  (in-progress `OutputTextDelta`, open reasoning section, partial tool state)
  before processing replayed history. Mid-stream text that was never persisted
  is gone; no recovery is possible. The transcript surface shows a visual break
  (`↻ reconnected`) so the jump is legible.

**v1 (Plan 3b-2b) emits `gap: None` unconditionally.** The snapshot carries no
`sequence_number`, so contiguity cannot be proven from it; rather than synthesize
a fragile heuristic, v1 always emits `None` (the always-safe path — it only clears
transient accumulators the drop likely lost anyway). The `Some(0)` clean-overlap
proof is deliberately deferred: `resume_floor = last_seen_seq` is still tracked and
used to **drop the duplicate live overlap** (frames whose `sequence_number ≤ floor`
after re-open), but it is never promoted to `gap = Some(0)`. `Some(N > 0)` is never
synthesized (folded into `None`).

### The SSE stream vs. the items DB

The SSE stream (`GET /stream`) is live-tail, no-replay. The server never seeks
back in the stream. What `GET /items` returns is a separate thing: the server's
durable DB of persisted conversation items. The state model does not need to
maintain a full history cache — it fetches from `GET /items` on reconnect.

The crate owns the protocol; the state model just keeps draining the
`ServerStreamEvent` stream (app-arch state-model §8: "the pump just keeps
reading") and never sees raw reconnect mechanics. The crate emits
`ServerStreamEvent` — `StreamUpdate` is the state model's *reduced* output
(§13), not the crate's emission. The reconnect-lifecycle markers ride the
same stream as synthetic values: `Reconnecting { attempt }` → `Reconnected { gap }`
→ `SnapshotRestored(SessionSnapshot)` (on success) … → terminal `Disconnected`
(on give-up).

**Decision — bucket-B chrome restore ownership (2026-06-26, Opus design; A2).**
On reconnect the crate emits chrome restore as **one synthetic
`ServerStreamEvent::SnapshotRestored(SessionSnapshot)`**, NOT as direct
consumer apply and NOT as per-field synthetic `session.*` events.
- **Why not consumer-applies-snapshot (Option B):** state-model §1 is LOCKED —
  "this layer does NOT own reconnect" and "the boundary upstream is the
  `ServerStreamEvent` stream." A direct apply would make the consumer aware of
  reconnect and ingest a non-`ServerStreamEvent` payload, breaking that boundary
  and the §4.1 single-writer/replayable invariant (the reducer is the only
  writer of `SessionState`).
- **Why not per-field synthetic `session.*` events (Option A1):** replaying the
  snapshot through the existing per-event folds re-runs their side-effects — a
  `session.agent_changed` fold pushes an `AgentChanged` *item* into the
  transcript (state-model §4.1), so every wake would inject a spurious
  agent-change marker. It also needs a snapshot-field→event map maintained in
  lockstep with the wire forever.
- **Why A2 (chosen):** one event carrying the typed `SessionSnapshot` keeps the
  reducer the single writer and the consumer purely event-driven; the reducer
  gets one dedicated arm that bulk-folds chrome scalars/collections with **no
  transcript side-effects**; it reuses the typed snapshot reads (Plan 3b-2a)
  wholesale — no per-field mapping. The state-model reducer (§4.1) must add a
  `SnapshotRestored` arm: chrome-scalar/collection fold only, no `AgentChanged`
  item insertion, no presence-marker emission.

**Two seams this opens for the reconnect implementation (Plan 3b-2):** (a) the
crate's §7a normalizer holds per-stream dedup state (`seen_items`) — it MUST be
reset on `Reconnected { gap != Some(0) }`, or the `GET /items` history replay is
wrongly suppressed as an already-seen re-fire; (b) the synthetic markers
(`Reconnecting`, `Reconnected`, `SnapshotRestored`, `Disconnected`) are not wire
events — they bypass dedup/reasoning normalization (pass through untouched).

**Three-bucket reconnect classification** (which events survive reconnect and
how) — **load-bearing delegation, authoritative here**. Note: ALL `session.*`
chrome events are transient/SSE-only *on the wire*; they survive reconnect only
because their **state** is restored from the session snapshot, NOT because the
events themselves replay from `GET /items`.

- **Bucket A — item-backed** (restored from `GET /items`, merged by item `id`):
  all `response.output_item.done` items (messages, function_call,
  function_call_output, reasoning, native_tool, compaction, slash_command,
  terminal_command, error), all `response.completed/failed/incomplete/cancelled`
  insofar as they finalize persisted items.
- **Bucket B — snapshot-restored chrome** (NOT item-replayable; reconstructed
  from `GET /v1/sessions/{id}` scalars/collections): `session.status`,
  `session.created`, `session.usage`, `session.model`, `session.todos`,
  `session.model_options`, `session.reasoning_effort`,
  `session.collaboration_mode`, `session.skills`, `session.agent_changed`
  (apply current `agent_id`/`agent_name` from snapshot — no marker on wake),
  `session.resource.*`, `session.child_session.updated`,
  `pending_elicitations` (re-fetchable via `GET elicitation_id`).
- **Bucket C — truly transient** (gone on reconnect; safe to drop, re-derive on
  next live event): `response.output_text.delta`, `response.reasoning.*.delta`,
  `response.in_progress`, `response.heartbeat`, `session.heartbeat`,
  `session.presence`, `session.terminal.activity`, `session.terminal_pending`,
  `session.sandbox_status`, `response.elicitation_resolved` (re-derived from the
  absence of pending state — the event itself carries no verdict).

**Adding a new event type requires updating this classification in lockstep.**
The state model and every surface document that reasons about reconnect relies
on it being authoritative here.

### Event taxonomy — byte-verification & blocked families

The crate partitions wire discriminators into `MODELED_EVENT_TYPES` and
`DEFERRED_EVENT_TYPES` (`stream/event.rs`); the offline `taxonomy_drift` test
asserts their disjoint union equals the pinned openapi mapping. Within modeled
types, families are either **byte-verified** (captured on the wire) or
**schema-derived** (openapi-shaped, not yet captured). Ground-truth corpus:
`docs/spikes/2026-06-26-live-event-recapture.md` (15 `.sse` streams, 2026-06-26).

**Byte-verified** (promoted or de-flagged per the live recapture spike):

- `session.agent_changed`
- `session.created` (child spawn)
- `session.resource.deleted`
- `session.child_session.updated` (now carries typed `child{}`)
- `response.elicitation_request` (now carries typed `params`)
- `response.reasoning_text.delta`
- `session.model`, `session.reasoning_effort`, `session.todos`
- `response.cancelled`, `session.interrupted`
- `response.compaction.in_progress`
- `session.terminal.activity` — **SSE**, not WS attach (terminal *content* remains
  WS, §5)
- `session.skills`
- `response.failed`
- `response.elicitation_resolved`

**Still blocked** (environment / harness — remain `DEFERRED` or schema-derived; a
live `Unknown` or uncaptured shape is expected):

| Family | Reason |
|---|---|
| `turn.*` | Codex-native / Codex app-server protocol only — needs a codex subscription |
| `response.created` / `response.queued` | openai-agents / open-responses scaffold; runner defers `response.created` before the session stream |
| `response.reasoning_summary_text.delta` | Codex `summaryTextDelta` only |
| `response.compaction.completed` | Requires a configured `llm_model` (subscription auth here uses `llm_model: null`) |
| `response.error` | Schema-derived — not captured on the wire in the recapture corpus |

### v1 as-built (Plan 3b-2b)

The reconnect state machine lives in the SSE reader thread (`stream::reader`),
generic over a `Reopen` capability (`snapshot` / `items` / `open_stream`) so it
is unit-testable with a scripted mock — no server. As-built specifics:

- **Backoff schedule:** `[100, 200, 400, 800, 1600, 3000, 3000]` ms (7 attempts,
  ~9s worst case). Each attempt emits `Reconnecting { attempt }` (1-based).
- **`sequence_number` is peeked off the raw `SseFrame` JSON**, not the typed
  event (typed events strip it; only `Heartbeat` exposes it). The peek is `None`
  for persisted items, lifecycle frames, and malformed data — those are never
  dropped by the overlap filter.
- **Fetch order on a successful attempt is `snapshot → items → open_stream`** —
  `open_stream` is the *last* fallible call, so a retryable `/items` (or snapshot)
  failure re-loops the backoff **without** discarding an already-opened no-replay
  body. Markers are emitted only after all three succeed, in the load-bearing
  order: `Reconnected { gap: None }` → `reset_transient` → `SnapshotRestored` →
  replayed `/items` history (each item as `OutputItemDone`, sent directly,
  bypassing the normalizer).
- **Items replay is single-page, best-effort** (server default order). `has_more`
  truncation is not paginated in v1; the reducer merges by `Item::id()`, so a
  later live event or subsequent reconnect fills any gap. Flagged for follow-up if
  captures show truncation in practice.
- **Clean EOF (`Ok(0)`) flushes the synthetic `ReasoningClosed` bracket before
  reconnecting; a transport error (`Err`) does not** (the bracket did not end, the
  connection did — §7a invariant).
- **No panic on the data path.** `Sessions::stream` / `EventStream::spawn` return
  `Result`; a thread-spawn failure surfaces as `ClientError::ThreadSpawn` rather
  than an `expect`.
- **Deferred:** `gap == Some(0)` contiguity proof; `/items` pagination/backfill;
  a gated live reconnect smoke test (no scripted server-kill harness this session).

### Bootstrap (first open) — Plan 4

First open of `Sessions::stream()` emits the **same post-open prelude as
reconnect, minus the `Reconnecting`/`Reconnected` markers** (there is no gap on
first connect): `SnapshotRestored(SessionSnapshot)` → replayed `GET /items`
history (each as `OutputItemDone`), then the live tail. This makes the consumer's
reducer the **single writer** for initial state too (app-arch §4.1) — the
consumer no longer loads the opening snapshot/items through a second path that
must stay byte-aligned with the reconnect fold.

- The live body is already open before the prelude fetch (subscribe-first), so
  events buffered between open and the snapshot/items read are processed after the
  prelude; bucket-A items dedupe by `Item::id()`.
- **Failure policy:** a *retryable* prelude-fetch failure (no `stop_reason`)
  degrades to live-tail-only — no prelude emitted, no regression versus
  pre-Plan-4 behavior. A *fatal* failure (`stop_reason`: 401/403/404) emits the
  terminal `Disconnected { reason }` and stops. Bootstrap never emits
  `Reconnecting`/`Reconnected`.
- The synthetic `SnapshotRestored` and replayed items bypass normalization,
  exactly as on reconnect (the markers-bypass seam, above). The reducer folds a
  bootstrap `SnapshotRestored` with the *same* arm as reconnect — chrome
  scalar/collection restore only, no transcript side-effects.

---

## 7a. Normalization guarantees

The crate normalizes the raw SSE stream before handing events to the state
model. The following guarantees hold; nothing beyond them:

- **`OutputItemDone` re-fire suppression** — a second `output_item.done` whose
  `(kind, call_id, status)` was already emitted is dropped (claude-sdk's MCP path
  double-fires identical items). This is **literal-duplicate suppression, not a
  collapse to one event per `call_id`**: the captured `function_call`
  `in_progress`→`completed` progression (same `call_id`, differing `status`) is
  preserved as two events so the state model keeps the "tool starting" signal.
  (Earlier drafts said "each `call_id` appears exactly once"; relaxed 2026-06-26
  per the golden-SSE bytes — see `docs/spikes/2026-06-26-golden-sse-capture.md`.)
- **`ReasoningClosed` synthesis (synthetic event)** — the SSE stream has no
  explicit reasoning-end event. The crate emits `ReasoningClosed` when the first
  `OutputTextDelta` or `Completed` arrives after a `ReasoningStarted`, carrying
  the accumulated `full_text` + `summary_text` so the renderer need not
  re-accumulate. The state model treats reasoning as a proper open/close bracket
  without tracking implicit state. `response.reasoning_text.delta` is
  **byte-verified** (cursor SDK — `docs/spikes/2026-06-26-live-event-recapture.md`);
  the `ReasoningClosed` event itself remains **synthetic** (no wire counterpart).
  claude-sdk still folds reasoning into `output_text` and emits no reasoning
  deltas — the close *trigger* on that harness path is inferred, not delta-driven.
- **`Reconnected { gap }` precedes all replayed history items** — ordering
  guaranteed. The state model must clear transient accumulators *before*
  history lands (per §7).
- **No text accumulation, no call/result pairing, no ordering changes** beyond
  the above. The crate is transparent except for the dedup and synthetic
  events listed here. Accumulation and pairing are the state model's job.

---

## 8. The contract-version gate

The semver lives at **`GET /api/version`** → `{"version": "<semver>"}`
(`app.py:1479-1491`, from `importlib.metadata.version("omnigent")`). **`GET /v1/info`
does NOT carry a version** — it is the unauthenticated runtime capability/auth
probe (`accounts_enabled, login_url, needs_setup, databricks_features,
managed_sandboxes_enabled, sandbox_provider`, `app.py:1493-1564`). `GET /health`
is liveness only. The crate pins a known-good omnigent version and **refuses to
start** if the server's reported version != the pinned one. A hard gate, not a
warning — a contract mismatch is a "things will break silently" condition.

**Ready-detection ladder** (used by the server lifecycle document on spawn):
`/health` (process is live) → `/api/version` (contract pin matches) → `/v1/info`
(capabilities / first-run / auth posture).

- The pin is a single `const PINNED_OMNIGENT_VERSION: &str` in the crate
  (currently `0.3.0.dev0`).
- On handshake, the crate calls `GET /api/version`, compares, and returns
  `Err(ContractMismatch { expected, actual })` if they differ.
- The crate then calls `GET /v1/info` to capture capabilities/auth posture
  (`accounts_enabled`, `login_url`, …) into `ServerInfo` for the connection.
- The server lifecycle document surfaces a mismatch as a visible "server down /
  wrong version" state — never lets a UI bug masquerade as a server bug.

**What if the server has no `/api/version`** (older or stripped build)? The crate
falls back to feature-detection: probe a few load-bearing endpoints
(`/v1/sessions`, `/v1/sessions/{id}/stream`) and fail loud if any are missing.
This branch is for robustness, not a supported config — the pin is the rule.

---

## 9. Contract pinning & codegen

The crate is the **single source of wire-shape knowledge** in Lens. Strategy:

1. **Vendor `openapi.json`.** The Lens repo vendors a copy of omnigent's
   `openapi.json` at the pinned version (`vendor/omnigent-0.3.0.dev0/openapi.json`),
   alongside an `OMNIGENT_PIN`/`VERSION` file recording the exact pin. A CI step
   diffs the vendored copy (path enumeration + SSE schema) against the sibling
   omnigent pin so the contract can't silently re-drift. Updates are an explicit
   bump: drop in the new file, run codegen, fix contract-test failures.
   (`openapi.json`'s own `info.version` is a stale `"0.1.0"` — pin against package
   semver, not that field.)
2. **Codegen the types.** Use `utoipa` or `openapi-codegen` to generate Rust
   structs for every schema in `openapi.json components/schemas`. These live in
   a `lens_client::generated` submodule; the hand-written `Client` layer wraps
   them.
3. **Hand-write the enum layer.** The typed `ServerStreamEvent` enum, the
   `SessionEventInput` enum (§6), and the `ChildSessionSummary` mirror (which
   openapi doesn't expose as a named schema) are hand-written on top of the
   generated structs. The canonical **harness list is the 19 in
   `OMNIGENT_HARNESSES`** (`omnigent/spec/_omnigent_compat.py:80-101`): antigravity,
   antigravity-native, claude-native, claude-sdk, codex, codex-native, copilot,
   cursor, cursor-native, goose, goose-native, hermes, openai-agents,
   open-responses, opencode-native, pi, pi-native, qwen, qwen-native. This is the
   validator-accepted set and what Lens should treat as canonical for the picker.
   `AgentObject.harness` is now a **free `string | null`** (no enum) in the schema
   (`openapi.json` ~62-71) — so the hand-written list is a Lens-side picker/
   validation aid, not a wire enum. Aliases are documented separately
   (`OMNIGENT_HARNESS_ALIASES`, `_omnigent_compat.py:104-118`: claude, opencode,
   github-copilot, …), normalized before dispatch. (`hermes` IS canonical — the
   old "16" list was stale and wrongly excluded it; `open-responses` is canonical
   but adapter-routed, with no dedicated runtime module wrap.)
4. **Contract-test suite.** Two test families:
   - **Golden SSE captures** — captured `.sse` files from a real `omnigent server`
     session; the parser deserializes them and asserts the typed event sequence.
     One capture per event family (text streaming, reasoning, tool calls,
     native tools, compaction, elicitation, usage, sub-agent, heartbeat,
     presence).
   - **Startup taxonomy diff** — on handshake, after `GET /api/version` passes, the
     crate diffs the server's emitted event discrimators (probed via a quick
     `/stream` sample + `openapi.json` schemas) against the pinned schema, and
     fails loud on unknown/changed shapes.
   - **Endpoint reachability** — every endpoint in §3 is pinged at least once in
     the test connection setup; missing endpoints fail the contract.

The suite runs against a local `omnigent server` (the server lifecycle
document's spawn path), started by the test harness. Failures here gate a
version bump, not a production release.

---

## 10. The typed Rust API surface (sketch)

> **Runtime note (decided 2026-06-25, see `typed-client-implementation.md`):** the
> public methods are **blocking/synchronous** `fn`, not `async fn`. The `async fn`
> in the sketch below is illustrative of the *shape*, not the signature — callers
> offload to background threads at the gpui seam (one thread per active session).
> `lens-client` pulls in no async runtime; SSE/WS use dedicated blocking OS
> threads → `std::sync::mpsc` → UI poller. Async/tokio/flume is a deferred,
> seam-local change to revisit only if stream fan-out reaches the thousands.

The crate's public surface to the rest of Lens:

```rust
pub struct Client {
    conn: Connection,
    http: reqwest::Client,
}

impl Client {
    pub async fn new(conn: Connection) -> Result<Self, ClientError>;  // handshakes + contract gate
    pub fn sessions(&self) -> Sessions;       // the session subservice
    pub fn hosts(&self) -> Hosts;             // hosts registry
    pub fn agents(&self) -> Agents;           // agent registry
    pub fn policies(&self) -> Policies;       // server-wide policies
    pub fn info(&self) -> Info;               // /v1/info, /v1/me
}

pub struct Sessions<'a> { /* ... */ }
impl<'a> Sessions<'a> {
    pub async fn list(&self, filter: SessionFilter) -> Result<Vec<SessionObject>>;
    pub async fn create(&self, req: SessionCreateRequest) -> Result<SessionObject>;
    pub async fn get(&self, id: SessionId, opts: GetOpts) -> Result<SessionObject>;
    pub async fn patch(&self, id: SessionId, req: UpdateSessionRequest) -> Result<SessionObject>;
    pub async fn delete(&self, id: SessionId, delete_branch: bool) -> Result<()>;
    pub async fn items(&self, id: SessionId, page: Page) -> Result<Vec<Item>>;
    pub async fn stream(&self, id: SessionId) -> Result<EventStream>;     // SSE
    pub async fn send_event(&self, id: SessionId, evt: SessionEventInput) -> Result<()>;
    pub async fn fork(&self, source: SessionId, req: SessionForkRequest) -> Result<SessionObject>;
    pub async fn switch_agent(&self, id: SessionId, bundle: Bundle) -> Result<()>;
    // ... resources (env-scoped fs/diff/search/shell), terminals, comments, elicitations,
    //     hooks, labels, permissions, owner, policies, child_sessions — all as typed fns
}

pub struct EventStream { /* SSE reader */ }
impl EventStream {
    pub fn next(&self) -> impl Future<Output = Option<Result<ServerStreamEvent>>>;
}

pub enum ServerStreamEvent {
    Session(SessionEvent),
    Response(ResponseEvent),
    Turn(TurnEvent),
    // ── synthetic stream-lifecycle (crate-generated, never on the wire) ──
    Reconnecting { attempt: u32 },     // entered backoff — drives ↻ now (§7 step 2)
    Reconnected  { gap: Option<u64> }, // success — precedes replayed history (§7a)
    Disconnected,                      // retry phase (~7s) expired — terminal
}

// ── Session events ───────────────────────────────────────────────────
pub enum SessionEvent {
    Status {
        status: SessionStatusValue,   // Idle | Launching | Running | Waiting | Failed
        // Launching = runner/harness booting. Waiting = parent parked on
        // async-work drain. Both distinct from Idle and from SessionInterrupted
        // (user explicit interrupt).
    },
    InputConsumed {
        item_id:   String,
        item_type: String,
    },
    Interrupted,                      // user explicitly interrupted
    Created,                          // session was just created
    Heartbeat {
        sequence_number: Option<i64>,
    },
    Usage {
        usage: SessionUsage,          // cumulative + usage_by_model
    },
    Model {
        model: String,
    },
    Todos {
        todos: serde_json::Value,     // the agent's per-session todos
    },
    ResourceCreated {
        resource: serde_json::Value,  // env|terminal|file
    },
    ResourceDeleted {
        resource_id: String,
    },
    ChildSessionUpdated {
        child: ChildSessionSummary,  // the richer mirror, see §13
    },
    ChangedFilesInvalidated {
        paths: Vec<String>,
    },
    TerminalActivity {
        terminal_id: String,          // notification only — PTY bytes via the WS
    },
    // ── 8 new since v0.1.0 (0.2.0 chrome events) ───────────────────────
    ModelOptions {
        options: serde_json::Value,    // drives the composer model picker
    },
    ReasoningEffort {
        effort: String,               // none..max
    },
    CollaborationMode {
        mode: Option<String>,          // codex-native Plan mode
    },
    AgentChanged,                     // fired by POST /switch-agent (§3)
    TerminalPending {
        terminal_id: String,
    },
    SandboxStatus {
        status: serde_json::Value,    // managed-sandbox provisioning state
    },
    Skills {
        skills: Vec<SkillSummary>,
    },
    Presence {
        viewers: Vec<PresenceViewer>,
    },
}

pub enum SessionStatusValue {
    Idle,
    Launching,    // runner/harness booting (SessionStatusEvent literal, schemas.py)
    Running,
    Waiting,
    Failed,
}

// ── Response events ──────────────────────────────────────────────────
pub enum ResponseEvent {
    // ── response lifecycle ───────────────────────────────────────────────
    Created { response: ResponseObject },
    Queued { response: ResponseObject },
    InProgress { response: ResponseObject },
    Completed { response: ResponseObject },
    Failed    { response: ResponseObject },
    Incomplete { response: ResponseObject },
    Cancelled  { response: ResponseObject },
    // The created/queued/in_progress variants are surfaced so the state model can
    // drive status lanes with their full status; a naive cut would drop them as
    // "intermediate states with no distinct UI action", but the card wave and the
    // "queued" badge both rely on the distinctions.

    // ── text streaming ──────────────────────────────────────────────────
    OutputTextDelta {
        delta: String,
        message_id: Option<String>,   // terminal-observed-streaming correlation
        index: Option<usize>,         // chunk order within the message
        final:  Option<bool>,         // true on the last chunk for message_id
    },
    // Accumulation into TextChunk / TextDone is the state model's job.

    // ── reasoning ────────────────────────────────────────────────────────
    ReasoningStarted,
    ReasoningTextDelta        { delta: String },
    ReasoningSummaryTextDelta { delta: String },
    ReasoningClosed {                     // SYNTHETIC — see §7a
        full_text:    String,
        summary_text: String,
    },

    // ── tool calls & results ─────────────────────────────────────────────
    // Emitted via OutputItemDone with item.type = function_call | function_call_output.
    // The crate suppresses literal (kind, call_id, status) re-fires (see §7a), preserving in_progress→completed progression.
    OutputItemDone {
        item: Item,                    // heterogeneous — see Item union below
    },
    OutputFileDone {
        file_id:      String,
        filename:     Option<String>,
        content_type: Option<String>,
    },

    // ── async client tool cancel ────────────────────────────────────────
    ClientTaskCancel {
        task_id: String,
        call_id: Option<String>,
    },

    // ── elicitation ─────────────────────────────────────────────────────
    ElicitationRequest {
        elicitation_id:  String,
        params:          ElicitationRequestParams,  // mode/form/url, schema, phase, policy_name, content_preview, target_session_id
    },
    ElicitationResolved {
        elicitation_id: String,
    },

    // ── compaction ──────────────────────────────────────────────────────
    CompactionInProgress,
    CompactionCompleted { total_tokens: Option<i64> },
    CompactionFailed,

    // ── errors & retries ────────────────────────────────────────────────
    Retry,                              // structure carries source/tool_name/attempt — see RetryEvent
    Error {
        source:    ErrorSource,         // llm | execution | tool
        tool_name: Option<String>,
        error:     RetryErrorDetail,    // code + message
    },

    // ── response heartbeat (gap detection pairs with session.heartbeat) ─
    Heartbeat(Heartbeat),
}

// ── Turn events ───────────────────────────────────────────────────────
// Emitted only on the NO_DBOS harness path (codex-native, specific forwarder
// bridges). Not universally emitted. The state model must not depend on these
// for correctness — treat as supplementary signals.
pub enum TurnEvent {
    Started,
    Completed,
    Failed    { error: String },
    Cancelled,
}

// ── Synthetic events (generated by the crate, not from the wire) ─────
// NOT a separate enum — folded into ServerStreamEvent above so the consumer
// drains ONE stream and reduce(&mut SessionState, ServerStreamEvent) is the
// single writer. The reconnect-lifecycle trio (Reconnecting / Reconnected /
// Disconnected) are top-level ServerStreamEvent variants; ReasoningClosed lives
// inside ResponseEvent (see §7a). Reconnected { gap } precedes all replayed
// history items on a resync (§7).
```

`Item` is the typed union mirroring omnigent conversation items
(the generated schemas + hand-written enum layer per §9):

```rust
pub enum Item {
    Message           { id, role, content_blocks },
    FunctionCall      { id, name, arguments, call_id, status /* completed|action_required|incomplete */, agent_name },
    FunctionCallOutput { id, call_id, output, arguments },
    Reasoning         { id, summary, encrypted_content: Option<…> },
    NativeTool        { id, kind /* web_search_call|mcp_call|code_interpreter_call|image_generation_call|computer_call|file_search_call */, data },
    Compaction        { id, summary, token_count },
    SlashCommand      { id, command, args },
    TerminalCommand   { id, command, args },
    ResourceEvent     { id, resource_id, kind },
}
```

**The contract-test suite (§9) treats `ChildSessionSummary` as a hand-written
mirror struct that participates in the normalization layer, not just a
codegen-lag workaround.** When the schema gains or changes a field, the typed
client's mirror is updated in lockstep, and the contract test pins the
round-trip. This is part of the "single source of wire-shape knowledge"
contract: the state model and the sub-agent topology document read
`ChildSessionSummary` exclusively through this crate's type, never reaching
into `serde_json::Value` for its fields.

All ids are branded newtypes: `SessionId`, `ElicitationId`, `HostId`,
`RunnerId`, `TerminalId`, `FileId`, `CommentId`, `PolicyId`, `ConnectionId`.
A string-compare bug is a compile-time error, not a runtime one.

---

## 11. Error handling

```rust
pub enum ClientError {
    Network(reqwest::Error),
    Auth { status: u16, /* ... */ },
    NotFound { what: String },
    Validation { detail: Vec<ValidationError> },
    Server { status: u16, body: serde_json::Value },
    ContractMismatch { expected: &'static str, actual: String },
    Parse(serde_json::Error),
    Ws(tungstenite::Error),
}
```

Notable: `ContractMismatch` is **loud** (fails the connection); `Auth` propagates
to the server lifecycle document for "re-login / re-token" UX. The SSE stream's
own errors go through the channel as synthetic `ServerStreamEvent` values
(`Reconnecting`/`Reconnected`/`Disconnected`), not synchronous `Result` returns:
the crate auto-reconnects an open stream **internally** (the consumer's pump just
keeps reading — app-arch state-model §8), so there is no `ClientError::Disconnected`
— give-up is the terminal `ServerStreamEvent::Disconnected`. (The "liveness
watcher" that governs *non-active* sessions is the separate app-arch state-model
§10 cross-session list poll, not this stream.)

---

## 12. Seams & contract churn

What would break if omnigent's contract changes, and where the cost lands:

| Change | Where it breaks in this crate | Rest of Lens |
|---|---|---|
| Event type renamed | `ServerStreamEvent` enum variant | Nothing — the state model consumes the enum, not the wire name |
| Endpoint path moved (e.g. env-scoping changes again) | one `Client` method body | Nothing |
| `SessionEventInput` gains a new discriminator | `SessionEventInput` enum variant + serialization | The app needs to *use* the new variant, but existing call sites don't break |
| `ChildSessionSummary` gains a field | hand-written mirror struct | Optional — the field is `Option<T>` if nullable; the state model adds a surface for it, the transcript sub-agent tree ignores it |
| Auth shape changes (e.g. new OIDC flow) | `Auth` enum + `Client` middleware | The server lifecycle document may surface a new "add connection" variant |
| `/api/version` semver bumps past the pin | contract gate | The gate fails loud → a crate bump is required |
| `/v1/info` capability shape changes | `ServerInfo` struct | The shell's first-run/auth chrome adapts; not a hard gate |
| Terminal WS path moves | terminal WS client one-liner | Nothing |

All contract knowledge lives in this crate. The application architecture & state
model document consumes a *typed* `ServerStreamEvent` enum and typed `Client`
methods; the wire never leaks past the seam.

---

## 13. What this spec is NOT

- Not the UI. The crate emits typed events; how they render is the surface
  documents' job.
- Not the server lifecycle. The crate is handed a `Connection`; spawning /
  supervising `omnigent server` is the server lifecycle document's job.
- Not the state model. The crate's `EventStream` produces `ServerStreamEvent`s;
  the state model reduces them to `StreamUpdate` and per-session state.
- Not a normalization across orchestrators. There is one orchestrator
  (omnigent). The seam is the contract pin, not a `Backend` trait. If a Rust
  sidecar or a second orchestrator emerges, the crate's `Client` could become a
  trait impl — but that's a future call, not a day-one design, and this
  document doesn't spec it.