# Application architecture & state model

The layer between the typed client (events on a wire) and the rendering
surfaces. It pins the domain model, the reduction from raw `ServerStreamEvent`
to canonical `Item`s, per-session state and liveness, local persistence,
command flow, the Bridge router, presence/co-viewers, switch-agent &
fork flows, and the concurrency bridge. Written **framework-neutral** — as
data shapes, invariants, and flow that hold whether the rendering substrate is
gpui or something else. Framework-specific points are collected in §14.

**Status:** Draft, 2026-06-23.
**Depends on:** the typed client (`typed-client.md`).
**Feeds:** the application shell, conversation transcript, workspace &
terminals, agent definition, permissions & elicitations, sub-agent topology
— all surface documents read downstream contracts from §13.

---

## 1. Scope & boundaries

The typed client hands this layer, per session, an
`EventStream` (typed `ServerStreamEvent` + reconnect-safe) plus a `Sessions`
subservice, a WS terminal attach, and a per-connection auth envelope. It
guarantees dedup, synthetic `ReasoningClosed`, and `Reconnected`-precedes-
history ordering — nothing more (the typed client's §7a). Everything above
that is this layer.

**This layer owns:**

- The **domain model** — the Rust types the whole app reasons about (§2).
- The **reduction pipeline** — `ServerStreamEvent` → canonical `Item` list (§4).
- **Per-session state**, Lens lifecycle (Active / Slept / Deleted), and the
  server `archived` visibility flag (§3).
- **Local persistence** — the on-disk session/item store (§6).
- **Command flow** — UI actions → the typed client's `SessionEventInput` (§7).
- The **concurrency model** — `ActiveSession` actors, channels, the runtime bridge (§8).
- **App-wide structure** — per-connection and cross-connection registries,
  focus, board layout, navigation (§9).
- **Cross-session liveness** via the session-list poll (§10).
- **The Bridge router** — fleet-wide actionable queue + agent-to-agent
  relay + planning todos (§11).
- **Presence/co-viewers** — the per-session viewer list (§12.1).
- **Switch-agent & fork flows** — live agent swaps and conversation forks (§12.2).
- **The Concierge** — a persistent chief-of-staff agent (§12.3).
- **Error/lifecycle mapping** from `ClientError` to app state (§13).
- The **downstream contracts** the surface documents build on (§13).

**This layer does NOT own:** the wire, reconnect, or SSE normalization (the
typed client); per-surface rendering, layout, theming, or widget behavior
(the surface documents); agent execution (server/runner). The boundary
upstream is the `ServerStreamEvent` stream; the boundary downstream is the
canonical `Item` list plus session state that surfaces read.

The seven load-bearing decisions and where they live:

| # | Decision | Section | Status |
|---|----------|---------|--------|
| ① | Session lifecycle | §3 | Active / Slept / Deleted + server `archived`; Sleep sends best-effort `stop_session`; Archive mirrors server `archived`; auto-sleep requires quiescence; no stream cap |
| ② | Reduction pipeline | §4 | `ActiveSession` owns canonical reduction; `SessionStore` is a foreground replica |
| ③ | App structure | §9 | per-connection `ConnectionApp`; cross-connection `AppState` |
| ④ | Command flow | §7 | optimistic send + FIFO reconcile, via `SessionEventInput` |
| ⑤ | Runtime bridge | §8 | `ActiveSession` actor → bounded channel → foreground `SessionStore` replica |
| ⑥ | Bridge router | §11 | Lens-side, on omnigent comments + labels + elicitation aggregation |
| ⑦ | Multi-connection topology | §9 | one `Client` per omnigent server; sessions keyed by (connection, session_id) |

---

## 2. Domain model

The model is the omnigent model adapted for the UI — not a normalization across
backends. Field shapes below are grounded in `omnigent/server/schemas.py` +
`openapi.json` (the typed client generates the wire structs; this layer adapts
them).

### 2.1 Branded ids

Stringly-typed primitives are a carry-forward hazard. Every id is a newtype:

```rust
pub struct ConnectionId(String);  // Lens-local; one per omnigent server
pub struct SessionId(String);     // == conversation id, "conv_*"
pub struct ItemId(String);
pub struct CallId(String);        // tool call_id
pub struct ResponseId(String);
pub struct RunnerId(String);
pub struct AgentId(String);
pub struct TerminalId(String);
pub struct ElicitationId(String);
pub struct FileId(String);
pub struct CommentId(String);
pub struct HostId(String);
pub struct PolicyId(String);
pub struct BridgeItemId(String);  // Lens-local; identifies a Bridge queue item (§11)
```

A `SessionId` is the conversation id; the platform uses them interchangeably and
so does Lens. **Lens composite-keys a session by `(ConnectionId, SessionId)`**
internally — the same `SessionId` value can exist on two different omnigent
servers and Lens must not conflate them. The public `SessionId` stays the wire
value; the composite key is internal to the state store.

### 2.2 `SessionState`

The per-session view-model. Mirrors omnigent's `SessionResponse` (the typed
client's generated struct) plus Lens-local fields:

```rust
pub struct SessionState {
    // ── Identity & binding ──
    pub connection_id: ConnectionId,         // which omnigent server owns this
    pub id: SessionId,
    pub agent_id: AgentId,
    pub agent_name: Option<String>,           // None = orphaned/deleted agent row
    pub runner_id: Option<RunnerId>,
    pub parent_session_id: Option<SessionId>, // Some => sub-agent (sub-agent topology)

    // ── Status & lifecycle ──
    pub status: SessionStatusValue,          // Idle | Launching | Running | Waiting | Failed
                                             // FULL 5-state only from SSE (SessionStatusEvent).
                                             // REST poll (GET /v1/sessions[/{id}]) is COARSE 3-state:
                                             // idle|running|failed (waiting collapses to running,
                                             // launching→idle). Persist the last fine-grained value
                                             // so a poll-fed Slept card doesn't regress to Idle.
    pub last_task_error: Option<ErrorInfo>,   // present iff status == Failed
    pub created_at: i64,                      // epoch seconds

    // ── Model & controls (agent definition doc) ──
    pub llm_model: Option<String>,
    pub model_override: Option<String>,
    pub model_options: Option<Vec<ModelOption>>,  // 0.2.0 chrome: drives picker choices
    pub reasoning_effort: Option<String>,          // none|minimal|low|medium|high|xhigh|max
    pub collaboration_mode: Option<String>,        // 0.2.0 chrome: codex-native Plan
    pub context_window: Option<u64>,
    pub last_total_tokens: Option<u64>,
    pub cumulative_cost: Cost,                     // accumulated client-side from usage

    // ── Workspace & host (workspace doc + server lifecycle doc) ──
    pub workspace: Option<String>,                 // absolute or workspace path
    pub git_branch: Option<String>,
    pub host_type: HostType,                       // External | Managed (0.2.0)
    pub host_id: Option<HostId>,
    pub sandbox_status: Option<SandboxStatus>,     // 0.2.0 chrome: managed-sandbox provision

    // ── Content ──
    pub items: Vec<Item>,                          // canonical, ordered, durable (§4)
    pub todos: Vec<Todo>,                          // the agent's live todos — rendered inline in chat
    pub skills: Vec<SkillSummary>,                // 0.2.0 chrome

    // ── Display & policy ──
    pub title: Option<String>,
    pub labels: BTreeMap<String, String>,
    pub permission_level: Option<u8>,             // effective level: 1=read,2=edit,3=manage,4=owner
                                                  //   (READ side can be 4; GRANTS are 1-3 only — permissions doc)
    pub pending_elicitations: Vec<Elicitation>,   // PLURAL — SessionResponse.pending_elicitations is a
                                                  //   list (schemas.py:1630); fan-out parents mirror
                                                  //   multiple child prompts. Each Elicitation carries
                                                  //   target_session_id for resolve routing. Composer
                                                  //   docks one; card/Bridge badge uses the count.
    pub owner: Option<String>,                    // GET /v1/sessions/{id}/owner

    // ── chrome: presence & co-viewers ──
    pub presence: Vec<PresenceViewer>,             // session.presence events; drives header chrome
                                                  //   (wire shape is {user_id, joined_at, idle} ONLY — §2.5)

    // ── Lens-local transient (RAM only, never persisted) ──
    pub stream: StreamScratch,                    // in-progress accumulators (§4.2)
    pub pending_user: Vec<PendingUserMessage>,    // optimistic, pre-consumed (§7)

    // ── Lens-local persisted metadata ──
    pub archived: bool,                           // server `archived` flag (§3.1)
    pub lifecycle: SessionLifecycle,              // Active | Slept | Deleted (Lens-local)
    pub last_focused_at: i64,                     // active-set LRU (§3.3)
    pub last_seen_seq: Option<u64>,               // reconcile cursor (§6, typed client §7)
}

pub enum HostType { External, Managed }
pub enum SessionLifecycle { Active, Slept, Deleted }
```

`pending_elicitations`, `todos`, `skills`, `permission_level`, `owner`,
`presence`, `collaboration_mode`, `model_options`, `sandbox_status` are mirrored
here but **owned by their surface documents** — this layer stores the state;
those documents define how it's rendered and acted on.

### 2.3 `Item` — the canonical conversation unit

The durable, reduced unit the transcript and disk store hold. Typed union
mirroring omnigent conversation items. Distinct from *streaming blocks*: those
are transient render units; `Item` is what survives a turn and lands on disk.

```rust
pub struct Item {
    pub id: ItemId,          // THE dedup/identity key — persisted ConversationItem has
                             //   only `id`, no sequence_number (entities/conversation.py:644)
    pub seq: Option<u64>,    // SSE sequence_number when seen live; None for items loaded
                             //   from GET /items. Live-overlap dedup only — never a storage key.
    pub ctx: BlockContext,   // attribution, stamped by the reducer (§4)
    pub kind: ItemKind,
}

pub enum ItemKind {
    Message { role: Role, content: Vec<ContentBlock> },
    FunctionCall      { call_id: CallId, name: String, arguments: Value, status: String, agent_name: Option<String> },
    FunctionCallOutput { call_id: CallId, output: String, arguments: Value },
    Reasoning { full_text: String, summary_text: String, encrypted: bool },
    NativeTool { tool_type: String, data: Value },  // web_search_call, mcp_call, …
    Compaction { summary: String, token_count: Option<u64> },
    SlashCommand { name: String, raw: String },
    TerminalCommand { command: String },
    Error { source: ErrorSource, code: String, message: String }, // persisted error banner
                                                                  //   (ErrorData, mirrors response.error)
    ResourceEvent { resource: SessionResourceObject }, // env|terminal|file (workspace doc)
    AgentChanged { from: AgentId, to: AgentId, at: i64 }, // switch-agent marker; `from` is
                                                          //   SYNTHESIZED — agent_changed carries
                                                          //   only agent_id/agent_name (no `from`)
}
```

**`function_call.status` wire enum is `in_progress | completed | action_required |
incomplete`** (`schemas.py:2648`), not `pending`/`running`/`error`.

`FunctionCall` and `FunctionCallOutput` are separate items paired by `call_id`
at render time (a tool span), not merged in storage — this keeps the durable
model 1:1 with the server's item stream and makes reconcile-on-wake a straight
**`id`** comparison (NOT `seq` — persisted items carry no sequence_number).
The `AgentChanged` item is a transcript marker that
acknowledges a switch-agent mid-session; the transcript surface owns its
visual.

### 2.4 `BlockContext`

Attribution stamped onto every `Item` by the reducer:

```rust
pub struct BlockContext {
    pub agent: Option<String>,  // "coder" | "coder.researcher"; None = root
    pub depth: u32,             // 0 = root, 1 = sub-agent, …
    pub turn: u32,              // turn within the response
    pub timestamp: f64,         // monotonic, when reduced
}
```

This answers the typed client's open question: attribution is a **field on the
canonical item**, set at reduce time — not a separate stream wrapper.

### 2.5 `Usage`, `Cost`, `ErrorInfo`, `PresenceViewer`

```rust
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub reasoning_tokens: Option<u64>,
    pub context_tokens: Option<u64>,
    pub usage_by_model: BTreeMap<String, ModelUsage>,  // 0.2.0: per-model rollup
}

pub struct Cost {
    pub cumulative_usage: Usage,
    pub total_cost_usd: Option<f64>,  // SERVER-computed USD (session.usage / total_cost_usd);
                                      // Lens needs no price table. None only if the server
                                      // hasn't priced the session (e.g. no model price on file).
}

pub struct ErrorInfo {
    pub code: String,
    pub message: String,
}

pub struct PresenceViewer {  // session.presence events — WIRE-FAITHFUL shape (schemas.py:2787-2804)
    pub user_id: String,
    pub joined_at: String,   // ISO 8601 UTC; stable across reconnect within the leave-grace window
    pub idle: bool,          // every stream the user holds reports an idle/backgrounded tab
}
```

**`PresenceViewer` carries no `display_name`/`is_owner`/`last_seen_at`** — those
were invented. Owner identity is derived separately (`GET /v1/sessions/{id}/owner`
+ `GET /v1/me` + `permission_level`) and joined to the viewer list by `user_id`;
a display name comes from `/v1/me`/owner, not presence. Presence is **transient/
RAM-only — never persisted** (it's a live observation; on reconnect it re-derives
from holding the SSE stream open, §6.2).

`cumulative_cost` accumulates from each `session.usage` event;
`last_total_tokens` + `context_window` drive the context meter. The platform
reports tokens + `usage_by_model` **and a server-computed `total_cost_usd`**
(per-model in `ModelUsage` and per-session, summed over the subtree). Lens
reads that USD figure directly — **no client-side price table** — so per-card
and per-project cumulative spend (decision I) is exact and free. The
time-windowed global readout (today/7d/30d) is the one thing Lens derives
itself, by sampling cumulative `total_cost_usd` into the `cost_samples` table
(§6.2) and differencing per window (the server's per-owner daily rollup is
internal and exposed by no endpoint).

---

## 3. State model & lifecycle — ① (LOCKED)

> **Reshaped 2026-06-27.** Sleep and Archive are no longer overloaded. **Sleep**
> is Lens's lifecycle declaration that it is done actively observing a quiesced
> session for now: close the stream/actor, flush durable state, drop heavy RAM,
> and send `stop_session` as a best-effort server-side reclamation request.
> The server owns the exact runner/process semantics of that request. **Archive**
> is the server `archived` flag (`PATCH /v1/sessions/{id}`) and controls
> visibility/listing only. There is **no stream cap**: the stream count
> self-bounds via strict auto-sleep.

### 3.1 The session lifecycle

A session has one Lens-local lifecycle state, plus the server-owned `archived`
flag. Lens-local lifecycle controls whether Lens is actively observing and
holding in-memory state. Server `archived` controls whether the session is hidden
from the default server listing.

| State | Stream | Server action | UI | Wake-back |
|---|---|---|---|---|
| **Active** | open, `ActiveSession` running | none | normal card | — |
| **Slept** | closed | best-effort `stop_session` after quiescence | dimmed, stays on board | reconnect/reconcile; resume/rebind only if needed |
| **Archived** | independent of stream | `PATCH archived=true` | hidden from default board/list | unarchive; wake only if also Slept |
| **Deleted** | closed | **deleted server-side** | removed | — (gone) |

- **Active** — SSE stream open, `ActiveSession` actor running off the foreground
  thread, the reduction pipeline live. Canonical `SessionState` is owned by the
  actor and write-through persisted as items finalize (§6, §8).
- **Slept** — Lens closes the stream, stops the `ActiveSession`, flushes durable
  state, drops heavy canonical RAM state, and leaves the foreground
  `SessionStore` replica painted from disk/list-poll summaries. Entering Sleep
  sends `SessionEventInput::StopSession` by default as a best-effort server-side
  reclamation request. The result is recorded for introspection/debug, but is
  not card chrome unless it later blocks wake/rehydration.
- **Archived** — Lens mirrors the server `archived` flag. Archive is visibility
  and organization, not resource lifecycle: `PATCH /v1/sessions/{id}
  {archived:true}` hides the session from the default listing, and `GET
  /v1/sessions?include_archived=true` brings it back for the Archive drawer.
  Archive may be combined with Sleep by UI policy, but the two states are
  orthogonal.
- **Deleted** — `DELETE /v1/sessions/{id}` removes the session server-side;
  the registry row becomes a read-only local tombstone (history viewable, never
  re-streamed) until pruned.

**Wake from Slept** = paint instantly from the disk snapshot → open a fresh
typed-client stream (bootstrap `SnapshotRestored` + `GET /items` + live tail) →
reconcile by item `id`. If the server reports that no runner/resource path is
available, Lens surfaces a resume/rebind action then; wake does not assume every
Sleep requires a runner relaunch. The runner relaunch/rebind sequence remains
the server-lifecycle fallback (§6): optional `GET /v1/runners/{runner_id}/status`
→ `POST /v1/hosts/{host_id}/runners` → `PATCH /v1/sessions/{id}` with the new
`runner_id` → reopen the stream. Disk-paint keeps wake flash-free; live typing
deltas emitted while slept are gone, and committed state is recovered through
`GET /items`.

`archived: bool` is server-owned and persisted on `SessionState`. `lifecycle:
Slept` is Lens-local and persists so a poll-fed card survives restart. There is
no separate storage tier and no "cold" rehydration policy — the server remains
the source of truth for session existence and server-owned fields.

### 3.2 Auto-sleep

A session is **auto-slept** once it has **genuinely quiesced** for a period
(default ~10 min). Elapsed time alone is not enough; the `ActiveSession` must
know the turn is over and no transient work is outstanding.

Auto-sleep **excludes**:
- **Pinned** sessions (held Active by intent).
- Sessions with a **pending elicitation** (Needs-input — you still must act).
- Sessions with a pending user message or unconsumed optimistic send.
- Sessions with open `StreamScratch` accumulators (message/reasoning), pending
  tool calls without outputs, reconnect/disconnected state that has not
  reconciled, or any status other than authoritative `idle`.
- Sessions with **live or recent terminal activity** — because Sleep sends
  `stop_session` best-effort, terminal-awareness is load-bearing: a live terminal
  counts as not-quiesced.

### 3.3 No stream cap (self-bounding)

There is **no connection cap.** A session streams iff it is **Active**. The
10-minute auto-sleep is the natural bound on concurrent streams — quiesced
sessions close their local streams and request server-side stop. A zoomed-out
board showing 50 cards opens a stream only for the genuinely-active ones; the
rest are Slept (dimmed) or Archived. An optional **soft, informational** warning
may surface past a high threshold of
concurrently-live sessions; nothing is force-slept against intent.

### 3.4 Sleep sends best-effort `stop_session`

Sleep sends `SessionEventInput::StopSession` after the strict quiescence
predicate passes. Lens's guarantee is local: it closes the stream, stops the
actor, persists canonical state, and drops heavy RAM. The server's exact runner,
harness, and PTY behavior is owned by the server. A failed/unknown stop result
is recorded in introspection and the state-transition ring, not shown on the
card by default. It becomes user-facing only if a later wake cannot rehydrate or
resume.

The explicit **Stop session** command (§7) uses the same wire event but carries
different intent: the user is deliberately stopping a still-visible session.
Archive does not imply stop; UI may offer "Archive and Sleep" as a composed
command.

### 3.5 Lifecycle transitions

```
focus / pin / resume ─▶ ACTIVE
    (from Slept: disk-paint + stream bootstrap/reconcile; resume/rebind if needed;
     from new:            typed client cold open: snapshot + items + stream)

quiesced ≥10min & not pinned & no pending-elicit & no terminal activity
    ─▶ SLEPT     (best-effort stop_session; dim card; stop actor; flush; free RAM)

user archives ─▶ ARCHIVED  (PATCH server archived=true; hide from default board/list)
user deletes  ─▶ DELETED   (DELETE server-side; local tombstone)
```

---

## 4. Reduction pipeline — ② (LOCKED)

Two layers: a **pure canonical reducer** for durable truth, invoked by the
off-foreground `ActiveSession` actor (§8), and **pure render-time transforms**
for per-surface views.

### 4.1 Canonical reducer (pure; invoked by `ActiveSession`)

`reduce(&mut SessionState, &ServerStreamEvent) -> SmallVec<[StreamUpdate; 2]>`
mutates the canonical state it is handed and returns semantic deltas for
persistence/replication. It does no I/O, does not touch gpui, and does not write
SQLite itself. The **single writer** is the `ActiveSession` actor (§8), which is
the only production caller holding `&mut SessionState`. The reducer is
deterministic and replayable (the same event sequence yields the same state),
which is what makes disk write-through and reconcile-on-wake sound.

Responsibilities:

- **Text accumulation** — `OutputTextDelta` folds into the in-progress message
  accumulator in `StreamScratch` (§4.2); finalized into a `Message` item on
  `ResponseCompleted`. The `message_id`/`index`/`final` fields from 0.2.0 let
  the reducer scope an in-flight buffer to one assistant message and reconcile
  it against the final item — used when the session's harness emits
  terminal-observed streaming.
- **Tool pairing** — `OutputItemDone` with `item.type = function_call` creates
  a `FunctionCall` item; the matching `function_call_output` (same `call_id`)
  creates a `FunctionCallOutput` item. The typed client already deduped both,
  so each fires once. They remain separate canonical items paired by `call_id`
  at render time (§2.3); the reducer does not merge them into one stored item.
- **Reasoning bracketing** — `ReasoningStarted` opens a reasoning accumulator;
  `ReasoningTextDelta`/`ReasoningSummaryTextDelta` append; the synthetic
  `ReasoningClosed` (typed client §7a) finalizes a `Reasoning` item.
- **`BlockContext` attribution** — the reducer tracks the current
  `agent`/`depth`/`turn` from the stream (the `FunctionCall.agent_name`, child-
  session events bump depth, response boundaries bump turn) and **stamps each
  item at creation** (§2.4).
- **Identity, ordering, dedup** — items keyed by **`id`** (persisted items have
  no `sequence_number`); `seq` is an SSE-only live-overlap dedup hint, not a
  storage key. Dedup against hydrated/disk items by `id` so a reconnect or wake
  never double-inserts.
- **Session-field folds** — `session.status`, `session.usage`,
  `session.todos`, `session.model`, `session.model_options`,
  `session.reasoning_effort`, `session.collaboration_mode`, `session.skills`,
  `response.elicitation_request/resolved`, `session.child_session.updated`,
  `session.presence`, `session.sandbox_status`, `session.terminal_pending`,
  `session.agent_changed` all fold into `SessionState` scalar/collection
  fields, not the item list.
- **`AgentChanged` item insertion** — when `session.agent_changed` fires, the
  reducer (a) updates `agent_id`/`agent_name` on `SessionState`, (b) pushes an
  `AgentChanged` item to mark the transition in the transcript. The card and
  composer re-render from the updated scalar; the transcript keeps its
  history with the marker visible (decision J, capability map §0.7).

- **`SnapshotRestored` fold (reconnect & bootstrap chrome)** — on
  `ServerStreamEvent::SnapshotRestored(SessionSnapshot)` (typed client §7, A2
  decision) the reducer bulk-folds the snapshot's bucket-B chrome
  scalars/collections into `SessionState` — status/usage/model/todos/
  model_options/reasoning_effort/collaboration_mode/skills/archived/
  presence-count/`agent_id`+`agent_name`. **Scalar restore only: no transcript
  side-effects** — unlike the live `session.agent_changed` fold above, this
  arm does NOT push an `AgentChanged` item (no agent transition happened, just a
  wake) and emits no presence marker. Ordering is guaranteed by the crate:
  `Reconnected{gap}` (clears `StreamScratch` when `gap != Some(0)`) →
  `SnapshotRestored` → replayed `GET /items` history. **The same arm also folds
  the bootstrap prelude** (first stream open, typed client §7 "Bootstrap"):
  the crate emits `SnapshotRestored` + replayed items on first connect too (no
  `Reconnected` marker — no gap), so initial state is folded through the same
  reducer arm as reconnect — the consumer no longer applies the opening
  snapshot/items through a separate path.

What the reducer finalizes becomes durable via `ActiveSession` write-through
effects (§6, §8). In-progress accumulators in `StreamScratch` are actor-local
RAM and never persisted — exactly the persisted/transient split the typed
client §7 defines.

### 4.2 `StreamScratch` — transient accumulators

```rust
pub struct StreamScratch {
    pub open_message: Option<MessageAcc>,           // accumulating assistant text
    pub open_reasoning: Option<ReasoningAcc>,        // accumulating reasoning
    pub unpaired_calls: HashMap<CallId, ItemId>,     // calls awaiting results
}

pub struct MessageAcc {
    pub message_id: Option<String>,   // 0.2.0: terminal-observed correlation
    pub text: String,
    pub block_index: usize,
}
```

Cleared on `Reconnected { gap }` when `gap != Some(0)`: mid-stream text that
was never persisted is gone, and the transcript shows a `↻` break.

### 4.3 Render-time transforms (pure, per-surface)

Read-only projections over the canonical `Item` list, composed per surface as
`Iterator`/`Stream` combinators:

- `hide_reasoning` — drop `Reasoning` items.
- `flatten_sub_agents` — inline child-session items in place of the spawn
  span (sub-agent topology doc).
- `merge_text_for_display` — coalesce adjacent message fragments.
- `only_agent(name)` / `by_depth` — filter/group for multi-agent panels
  (sub-agent topology doc).
- `with_agent_changed_markers` — keep `AgentChanged` items visible in the
  transcript; drop them from the review/diff tab where they're not relevant.

They never mutate stored state. The transcript, board card preview, and
review tab each `pipe` the set they want. Being pure functions over a plain
`Vec<Item>`, they are framework-neutral.

---

## 5. The multi-connection topology — ⑦ (LOCKED)

Lens is a **multi-connection client** (capability map §0.2). The state model
holds one `Client` (the typed client) per omnigent server Lens is attached to
— a local spawned one + zero or more remote ones. Each connection has its own:

- `Client` instance (the typed client; carries `Connection { base_url, auth,
  info }`).
- `ConnectionApp` — the per-connection session registry, active-set, and poll
  task (§9).
- Its own set of Active (streaming) sessions — self-bounded by auto-sleep, no cap (§3.3).
- Poll cadence (§10).
- Health state (up / reconnecting / down / contract-mismatch).

The cross-connection `AppState` (§9) holds the board layout, the focused
session (which is cross-connection; you focus one session at a time, regardless
of which server owns it), the Bridge router (§11), the Concierge
session handle (§12.3), and the global navigation funnel.

**A session is composite-keyed by `(connection_id, session_id)`** internally.
The user-facing `SessionId` stays the wire value, but Lens must never conflate
two sessions with the same wire id on different servers. The composite key is
a state-model internal; surfaces see a flat registry with a `connection` badge
on each card.

---

## 6. Local persistence — (LOCKED: SQLite v1, portable schema)

A local store is a **new component** relative to the typed client, which assumes
no client history cache. This layer owns it. Buys instant startup (paint cards
from disk before the network is up), offline history, restart survival, and
bounds RAM to ≈ the Active set.

### 6.1 Engine & portability

- **SQLite for v1** — single local file, transactional, low setup.
- Access goes through an abstract **`SessionPersistence`** trait; SQLite is the
  v1 impl. A later move to a remote backing store (per-connection or shared) is
  a backing swap behind the same trait, not a rewrite.
- The schema is kept **portable**: standard SQL types, JSON payloads in a
  column that maps cleanly to Postgres `jsonb`, text ids, epoch timestamps,
  no SQLite-only features on the critical path.
- The schema is a **documented, stable read contract**, not an opaque blob,
  so external tools — notably Bridge — can read Lens's session/item history.
  Meaningful tables, a stable `item_kind` enum, and denormalized `BlockContext`
  columns are a design constraint, not an accident.

### 6.2 Schema sketch

```sql
CREATE TABLE connections (
  id          TEXT PRIMARY KEY,        -- Lens-local ConnectionId
  base_url    TEXT NOT NULL,
  auth_kind   TEXT NOT NULL,            -- none|bearer|cookie|forwarded_email
  label       TEXT,                     -- user-given ("Local", "Internal dev")
  server_info TEXT,                     -- json from GET /v1/info
  created_at  INTEGER NOT NULL
);

CREATE TABLE sessions (
  connection_id   TEXT NOT NULL REFERENCES connections(id),
  id              TEXT NOT NULL,        -- conv_*
  agent_id        TEXT NOT NULL,
  agent_name      TEXT,
  runner_id       TEXT,
  parent_session_id TEXT,
  status          TEXT NOT NULL,        -- idle|launching|running|waiting|failed
  last_task_error TEXT,                 -- json {code,message}
  llm_model       TEXT,
  model_override  TEXT,
  reasoning_effort TEXT,
  collaboration_mode TEXT,
  context_window  INTEGER,
  last_total_tokens INTEGER,
  cumulative_cost REAL,                 -- latest server total_cost_usd (USD, subtree)
  usage_by_model  TEXT,                 -- json: per-model {tokens, total_cost_usd}
  workspace       TEXT,
  git_branch      TEXT,
  host_type       TEXT NOT NULL,        -- external|managed
  host_id         TEXT,
  title           TEXT,
  labels          TEXT,                 -- json, jsonb-mappable
  permission_level INTEGER,
  owner           TEXT,
  todos           TEXT,                 -- json (the agent's live todos)
  skills          TEXT,                 -- json
  -- NOTE: `presence` is transient/RAM-only (§2.5) and is intentionally NOT a
  -- persisted column — it re-derives from holding the SSE stream open.
  created_at      INTEGER NOT NULL,
  -- Server-owned visibility + Lens-local lifecycle.
  archived        INTEGER NOT NULL DEFAULT 0,      -- server `archived` flag
  lifecycle       TEXT NOT NULL DEFAULT 'active',  -- active|slept|deleted
  pinned          INTEGER NOT NULL DEFAULT 0,      -- was RAM-only; persist it
  tombstoned_at   INTEGER,              -- set when server-deleted; row kept read-only until pruned
  last_focused_at INTEGER,
  last_status     TEXT,                 -- last FINE-GRAINED status (so poll's coarse 3-state
                                        --   doesn't regress a Slept card to idle, §2.2)
  last_seen_seq   INTEGER,
  updated_at      INTEGER NOT NULL,
  PRIMARY KEY (connection_id, id)
);

CREATE TABLE items (
  connection_id TEXT NOT NULL,
  session_id   TEXT NOT NULL,
  item_id      TEXT NOT NULL,
  live_seq     INTEGER,                    -- SSE-only overlap hint; NULL for GET /items
  ordinal      INTEGER NOT NULL,           -- local canonical order within the session
  kind         TEXT NOT NULL,           -- message|function_call|function_call_output|
                                        -- reasoning|native_tool|compaction|
                                        -- slash_command|terminal_command|resource_event|
                                        -- agent_changed
  payload      TEXT NOT NULL,           -- json, jsonb-mappable
  agent        TEXT,
  depth        INTEGER NOT NULL DEFAULT 0,
  turn         INTEGER NOT NULL DEFAULT 0,
  created_at   INTEGER NOT NULL,
  PRIMARY KEY (connection_id, session_id, item_id),
  UNIQUE (connection_id, session_id, ordinal),
  FOREIGN KEY (connection_id, session_id) REFERENCES sessions(connection_id, id)
);

-- Cost time-series for the time-windowed global readout (decision I). Lens
-- samples each session's cumulative server total_cost_usd on usage events
-- (Active) and on the list poll (Slept), and differences per window
-- (today / 7d / 30d). The server's per-owner daily rollup is internal and
-- exposed by no REST endpoint, so Lens keeps its own observed series.
CREATE TABLE cost_samples (
  connection_id TEXT NOT NULL,
  session_id    TEXT NOT NULL,
  sampled_at    INTEGER NOT NULL,        -- epoch seconds
  total_cost_usd REAL NOT NULL,          -- cumulative server figure at sample time
  PRIMARY KEY (connection_id, session_id, sampled_at)
);

CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT);   -- schema_version, …
```

> **Cost windowing caveat (decision I):** the windowed global figure counts only
> spend Lens *observed*. A cumulative jump that happened while Lens was closed is
> captured on the next sample and attributed to that day, not the day it was
> actually spent. Per-card/per-project **cumulative** spend reads the server's
> `total_cost_usd` directly and is exact regardless.

### 6.3 Write-through & reconcile

- **Write-through while Active:** as `ActiveSession` finalizes each canonical
  item, upsert it into `items` by `(connection_id, session_id, item_id)`; fold
  session-field updates into `sessions`.
  In-progress accumulators are RAM-only (§4.2) — a crash mid-stream loses only
  unpersisted deltas, which the server still has.
- **On sleep:** flush actor-local canonical state and persistence, send
  best-effort `stop_session`, then stop the actor and drop heavy RAM state.
- **On wake:** load the disk snapshot into the foreground replica and paint
  immediately, then start a fresh `ActiveSession` stream bootstrap
  (`SnapshotRestored` + `GET /items` + live tail) and **reconcile by item
  `id`** (persisted items carry no `sequence_number`) — the disk may lag the server (compaction rewrote history,
  items edited, new items committed while sleeping), so reconnect/dedup is what
  makes the card correct, not the disk read alone.
- **Schema versioning:** `meta.schema_version` gates migrations; an unrecognized
  future version is read-only-degraded, never silently corrupted.

---

## 7. Command flow — ④ (decided)

Commands exit through the typed client's `Sessions::send_event`, which takes a
`SessionEventInput` (the typed Rust enum the client serializes into
`{type, data}` — the typed client's §6). The discriminator set admits, at
0.2.0: `message`, `function_call_output`, `approval`, `interrupt`,
`stop_session`, and others the typed client enumerates.

UI actions never mutate `SessionState` directly. Commands route to the
`ActiveSession` actor for that session. The actor records any optimistic local
state, performs or offloads the typed-client call, and reconciles from echoed
stream events so the single-writer invariant remains intact.

Command semantics, kept distinct:

- **send** — `SessionEventInput::Message { … }`. **Optimistic**: the
  `ActiveSession` actor adds the user bubble to its RAM-only `pending_user`
  buffer and emits a `StreamUpdate` to the foreground replica, then reconciles
  FIFO when `session.input.consumed` arrives — promote into the canonical list
  (*then* it hits disk). Roll back the buffer entry on POST failure. FIFO is
  safe because client posts and server consumed-events are both ordered within a
  session. Because you can only send to an Active session (focus makes it
  Active), optimistic bubbles always sit on a live stream.
- **interrupt** — `SessionEventInput::Interrupt { … }`. No optimism. The echoed
  `session.interrupted` + `response.incomplete` drive state.
- **compact** — `SessionEventInput::Compact` (`_COMPACT_TYPE`, `sessions.py:294`).
  Requests context compaction; drives `response.compaction.in_progress/completed/
  failed` (a failed compaction leaves **no** permanent marker — transcript §10).
  (The route's full `_ALLOWED_EVENT_TYPES` accepts ~20+ types incl. the
  `external_*` forwarder family; Lens *sends* only this subset but the parser
  must accept all — typed client §6.)
- **approve** — `SessionEventInput::Approval { elicitation_id, result }`. The
  typed client also exposes the RESTful `POST /elicitations/{id}/resolve`
  counterpart (preferred when an `elicitation_id` is on hand; permissions doc).
- **stop_session** — request server-side stop (owner-only); conversation
  preserved. The explicit Stop command is user-facing and immediate. Sleep uses
  the same wire event only after the strict quiescence predicate passes (§3.4),
  and treats failure/unknown as introspection state unless wake later fails.
- **fork** — `POST /v1/sessions/{source_id}/fork` with `SessionForkRequest`
  — creates a new session from a fork point. Not an `events` dispatch; a
  dedicated endpoint. The new session arrives in the registry via the list poll
  (§10) or by immediate create-response; the user can focus it independently.
- **switch_agent** — `POST /v1/sessions/{id}/switch-agent` with
  `SessionSwitchAgentRequest` (bundle upload goes to `PUT /agent` first if a
  new bundle is needed). The `session.agent_changed` event drives the
  in-place handoff (§12.2). The card + composer re-render in place; the
  transcript stays with an `AgentChanged` item marker.

**Steering** falls out for free: a `send` while a turn is running just queues
another `pending_user` entry; the server steers it into the running turn and
emits its own consumed event, which reconciles the same way.

The **bidirectional** path (a `client_os_*` tool dispatch where the server
requests client-side tool execution) is reserved here as a forward extension of
`SessionEventInput`; behavior belongs to the workspace & agent-definition
documents, not pinned here.

---

## 8. Runtime & concurrency — ⑤ (decided)

```
background OS thread, one per Active session
┌──────────────────────────────────────────────────────────────────────────┐
│ ActiveSession — single writer of canonical SessionState                  │
│ EventStream ─▶ reduce(&mut SessionState, &event) ─▶ canonical state      │
│                  │                                                       │
│                  ├─▶ SessionPersistence write-through (SQLite)           │
│                  └─▶ StreamUpdate ───────────────┐                       │
│ SessionCommand ◀──────────────────────────────┐  │                       │
└────────────────────────────────────────────────┼──┼──────────────────────┘
                                                 │  │ bounded channels
foreground (gpui)                                │  ▼
┌────────────────────────────────────────────────┼─────────────────────────┐
│ UI action ─▶ SessionCommand ───────────────────┘                         │
│ SessionStore = Entity<SessionState> replica: apply StreamUpdate + notify │
│ surfaces observe SessionStore; surfaces never receive &mut SessionState   │
└──────────────────────────────────────────────────────────────────────────┘
```

- Each Active session owns one **`ActiveSession` actor** on a dedicated
  background OS thread. This matches the typed client's blocking stream reader
  model; no tokio runtime is required for the core data path. The actor drives
  the typed client's `EventStream`, calls the pure reducer, write-throughs
  durable effects to `SessionPersistence`, and sends reduced `StreamUpdate`s to
  the foreground.
- **Single-writer invariant:** for each Active session, canonical
  `SessionState` is owned and mutated by exactly one `ActiveSession` actor,
  running off the foreground thread. Every canonical mutation flows through
  `reduce(&mut SessionState, &ServerStreamEvent)` called by that actor's
  run-loop and nowhere else. The foreground `SessionStore`
  (`Entity<SessionState>` in gpui) is a read-only replica: it applies
  self-contained `StreamUpdate`s and notifies subscribers, but never runs the
  reducer and never originates state changes. UI commands go to the actor over
  a `SessionCommand` channel; even optimistic `pending_user` entries are written
  by the actor.
- **Replica contract:** `SessionStore` exposes read/observe access to surfaces,
  never mutable access. `StreamUpdate::apply` is cheap assignment/insert work
  only; no wire parsing, reduction, persistence, search, or diffing happens on
  the gpui foreground thread. Debug/introspection may keep a bounded raw event
  ring, but raw `ServerStreamEvent`s are not the app's source of truth.
- **Lifecycle:** spawn the `ActiveSession` actor on transition to Active; stop it
  when the session is Slept/Deleted, and keep only the foreground replica plus
  disk state. Reconnect within an Active session is entirely the typed client's
  `EventStream` (the actor just keeps reading); waking from Slept starts a fresh
  actor and stream bootstrap (§3.5).
- **Backpressure/coalescing:** the bounded update channel applies backpressure
  if the foreground cannot keep up. The actor coalesces bursty text deltas where
  safe, and the foreground poller drains all available `StreamUpdate`s for a
  session before a single `cx.notify()` so token streaming does not become a
  per-byte redraw path.
- **One poll task per connection** drives the cross-session list poll (§10).

The foreground hand-off is isolated to one seam with two message directions:
`StreamUpdate` out of the actor, `SessionCommand` into it. See §14 for the
framework divergence there.

---

## 9. App structure & navigation — ③

```rust
pub struct AppState {
    pub connections: HashMap<ConnectionId, ConnectionApp>,  // one per omnigent server
    pub focused: Option<(ConnectionId, SessionId)>,        // cross-connection focus
    pub board: BoardLayout,                                // card positions + grouping (capability map §0.6)
    pub bridge: BridgeRouter,                // §11
    pub concierge: Option<ConciergeHandle>,                 // §12.3
    pub derived: CrossSessionSignals,                      // running / needs-attention badges (§10)
}

pub struct ConnectionApp {
    pub conn: Connection,                  // the typed-client Connection
    pub client: Arc<Client>,               // the typed-client Client
    pub sessions: HashMap<SessionId, SessionHandle>,  // registry (Active + Slept + server-archived)
    pub pinned: HashSet<SessionId>,
    pub active_set: ActiveSet,             // which sessions are Active (streaming); no cap (§3.3)
    pub health: ConnectionHealth,         // up / reconnecting / down / contract-mismatch
    pub poll_task: Option<JoinHandle<()>>, // the list-poll task (§10)
}

pub struct SessionHandle {
    pub session_id: SessionId,
    pub store: SessionStore,              // foreground replica; exists in every lifecycle state
    pub active: Option<ActiveSessionHandle>, // Some iff Active
}

pub struct ActiveSessionHandle {
    pub thread: JoinHandle<()>,
    pub commands: Sender<SessionCommand>,
}
```

- **Registry** — every known session has a `SessionHandle`. Its `SessionStore`
  is the foreground read/observe replica, hydrated from disk/poll state and
  present even when the session is Slept/Archived. Active sessions additionally
  have an `ActiveSessionHandle`, whose background actor owns canonical state and
  emits `StreamUpdate`s into the replica.
- **Subscription scopes** — the side pane subscribes **deeply** to the focused
  session's store; board cards subscribe to a **coarse summary** (status,
  title, cost, badge), never the focused session's fine-grained transcript.
  This is the invariant that prevents a background delta burst from
  invalidating the foreground render. The per-session store beats the
  alternative single global store, which only ever streams one session.
- **Navigation** — one `navigate_to_session(connection_id, session_id)` funnel:
  sets focus, wakes the target if Slept (fresh actor + stream bootstrap, with
  resume/rebind only if the server requires it, §3.5), routes the side pane. No
  ad-hoc focus mutation.
- **Board vs drawer** — `BoardLayout` tracks Lens-local board/group placement;
  server `archived` controls the Archive drawer filter (§3.1). Both draw from
  the same registry.
- **Multi-window** is a capability-map decision G — `AppState` is
  framework-shared; per-window `WindowState` lives in the application shell.

---

## 10. Cross-session liveness (the list poll)

There is **no global/aggregate event stream** — the only SSE endpoint is
per-session (`GET /v1/sessions/{id}/stream`; verified). So Slept sessions, and
archived sessions that are not currently Active, cannot stream their status.
Instead:

- A periodic **`GET /v1/sessions` poll** (default ~10s, configurable) refreshes
  the **coarse** state of all known sessions — `status`, `title`,
  `last_total_tokens`, `host_id`, and a derived needs-attention flag (`Waiting`
  / pending elicitation / `pending_elicitations_count`). When the window is
  backgrounded the poll **throttles to a slower cadence rather than stopping**,
  so needs-input on slept/remote sessions is still detected and can fire a
  native notification (shell §17.4 residency); it stops only when the app is
  fully quit (⌘Q).
- The poll runs **per connection** (each `ConnectionApp` owns one poll task).
- It updates only card-summary fields, never the transcript. Active sessions
  ignore polled status for fields their live stream owns (the stream is
  authoritative); the poll exists for **Slept**, non-active archived, and
  externally-created sessions.
- `CrossSessionSignals` aggregates these into board badges ("3 running, 1
  needs you") **across connections** — the rollup is cross-connection; the
  per-connection health dot is per-connection (application shell owns the
  surface).
- The poll is also what surfaces **new sessions** created outside Lens (e.g. a
  fork via the CLI, a session someone else shared with you) so they appear in
  the registry without a Lens restart.

---

## 11. The Bridge router — ⑥ (LOCKED)

The Bridge is a **Lens-side router** — not a single omnigent surface.
It aggregates three input streams into one fleet-wide actionable queue:

1. **Pending elicitations** from every session (the `response.elicitation_request`
   events across all Active streams + the polled `pending_elicitations_count`
   for Slept/non-active archived sessions).
2. **Agent-to-agent relay messages** — Lens-side, built on omnigent's
   per-session `POST /comments` + `POST /comments/send`. An agent (e.g. the
   Concierge) can file a message to another session's comments and label it as
   a "relay" — the Bridge indexes on labels to surface it.
3. **Planning todos** — long-term, cross-session todos distinct from the
   agent's own `session.todos`. Stored Lens-side (in the Bridge's own
   SQLite tables, keyed by `connection_id + session_id + label`); the agent's
   live `session.todos` render inline in the chat (capability map §0.3) and
   are NOT routed here.

```rust
pub struct BridgeRouter {
    pub queue: Vec<BridgeItem>,
    pub filters: BridgeFilters,  // All | You | Projects | Agents | Deferred
    pub badge_counts: BadgeCounts,       // per-filter counts; drives the rail dot
}

pub struct BridgeItem {
    pub id: BridgeItemId,                // Lens-local (branded)
    pub kind: BridgeItemKind,         // Elicitation | Relay | PlanningTodo | DeferredNote
    pub connection_id: ConnectionId,
    pub session_id: Option<SessionId>,    // None = floating planning todo
    pub from_agent: Option<AgentId>,     // Some = agent→agent relay; the Concierge uses this
    pub to_agent: Option<AgentId>,       // Some = directed; None = broadcast/posted
    pub received_at: i64,
    pub deferred: bool,                  // user deferred; not gone, just postponed
    pub body: String,                    // the rendered text + structured payload
    pub actions: Vec<MessageAction>,     // Resolve, Reply, Undefer, Discuss with [agent], Send to…
}

pub enum BridgeItemKind { Elicitation, Relay, PlanningTodo, DeferredNote }
```

**Routing fabric sources:**

- **Elicitations** come from the typed client's `ElicitationRequest` event;
  Lens indexes them by `(connection_id, session_id, elicitation_id)` and
  resolves them via the typed client's `Approval` input or `/resolve` endpoint.
- **Relays** are encoded as omnigent session-scoped comments with a
  Lens-conventional label (e.g. `bridge:relay`) — the Bridge
  indexes the comments stream by label to surface them. `POST /comments/send`
  is the mechanism to deliver a relay *to* an agent; the agent receives it as
  structured feedback in its session. This keeps omnigent's "no cross-session
  messages object" constraint honored — Lens is *the* router, omnigent just
  carries labeled comments.
- **Planning todos** are Lens-side state, not carried over the omnigent wire
  at all. They live in the Bridge's own SQLite tables. An agent (e.g.
  the Concierge) can read them via a tool (an MCP server Lens exposes
  locally), so the agent's planner and the user's planning todos stay in sync.

**Bridge badges** drive the left rail's Bridge dot count +
pop through ⌘I ("jump to next-needs-input"). Placement decision (left-rail
destination vs tray vs modal) is open (capability map §0.7-H); **the router
itself is framework-neutral and spec'd here regardless of placement**.

---

## 12. 0.2.0-specific flows

### 12.1 Presence / co-viewers

`session.presence` events carry the per-session viewer list
(`Vec<PresenceViewer>`, wire shape `{user_id, joined_at, idle}` only — §2.5).
The reducer folds them into `SessionState.presence` **as RAM-only state — never
persisted** (it re-derives by holding the SSE stream open). The application shell
reads it for the focused-session header's "X, Y also viewing" chrome (shell §7.4).
**Owner/display chrome is NOT in the presence payload** — it's joined in from
`GET /v1/sessions/{id}/owner` + `GET /v1/me` + `permission_level` by `user_id`,
which also drives "you don't own this session" affordances when the session
belongs to someone else on a shared remote server (permissions doc).

**Lens does not broadcast its own presence** unless the user opts in (v1
privacy default: receive-only). The mechanism to broadcast (if built later)
would be a `POST /events` carrying a `presence`-shaped payload — the typed
client's enum reservation covers it.

### 12.2 Switch-agent handoff (decision J)

Triggered by `POST /v1/sessions/{id}/switch-agent` (route at
`omnigent/server/routes/sessions.py:14214`, body `SessionSwitchAgentRequest`).
`PUT /v1/sessions/{id}/agent` is used first if a **new bundle** is needed
(bundle storage only — it fires nothing); `POST /switch-agent` swaps the live
session's binding. `session.agent_changed` fires (`sessions.py:14353`).

**Guards — corrected grounding.** The server's **API floor is `LEVEL_EDIT` (2),
NOT owner** (`_require_access_and_level(..., LEVEL_EDIT, ...)`, `sessions.py:14214`;
docstring "403 if the caller lacks edit access"). The idle guard rejects when the
**cached** status is `running` (and `waiting`, which the cache collapses to
`running`) — but **not `launching`**, which falls through to `idle` and is NOT
rejected. The server also rejects sub-agents (400) and no-op swaps (400).
**Owner-only + idle-only is therefore a Lens UI policy (decision J), stricter than
the API** — the earlier "caller is owner, verified in source" was wrong. The
application shell disables the kebab's "Switch agent ▸" for non-owners and when
busy (client-preflighting `launching`, since the server won't reject it) and
hides it for sub-agents
(agent definition §7). The switch also fires the server's
`_reset_runner_resources_after_switch` — **runner resources reset**, so any open
terminals on the session drop and must re-attach. The transcript itself is
untouched.

**The flow this layer owns:**

1. On `session.agent_changed`: the event carries **only `agent_id` +
   `agent_name`** (`schemas.py:2218-2221`) — no model/skills. Update
   `SessionState.agent_id`/`agent_name` from it, then **refetch the snapshot**
   (`GET /v1/sessions/{id}`) for the new `llm_model`, `model_options`,
   `reasoning_effort`, `skills`.
2. Insert an `AgentChanged { from, to, at }` item into `SessionState.items` —
   **synthesize `from` from the prior reducer state** (it is not on the wire) and
   allocate a synthetic local item id (this marker is not in `GET /items`, so on
   a later reconnect re-synthesize it from the snapshot's current agent).
3. Notify subscribers — the card re-renders (correct harness badge, correct
   model label), the composer re-renders (correct per-session controls), the
   transcript keeps its history with the `AgentChanged` item visible. **The
   transcript does not remount** — it's the same session; the marker just
   acknowledges a handoff.
4. The state model does NOT clear `items` — the conversation continues across
   the swap. Earlier items keep their original `BlockContext.agent`; items
   after the swap carry the new agent. This makes the transcript's
   agent-attribution story coherent without a remount.

### 12.3 The Concierge

A **long-standing chief-of-staff agent** (capability map §0.6). Configured via
`~/.omnigent/agents/concierge.yaml` and onboarded automatically on first Lens
run. Behaviors:

- **Lives on the always-on local server.** The Concierge is **local-only** by
  three independent constraints: Lens can only write `~/.omnigent/agents/` on
  the server it spawns; its runner must reach the **Bridge MCP server Lens
  exposes locally** (a remote runner can't reach the laptop); and it must be a
  real omnigent session (Lens never orchestrates its own loop). So Lens runs a
  **local omnigent server as always-on baseline infrastructure** regardless of
  which work-connections the user adds (server lifecycle §3, §10) — the
  Concierge always has a home.
- **Persistent session** — the Concierge's session is `parent_session_id ==
  None` and persists across Lens restarts (Lens stores its `SessionId` in
  `meta` and re-attaches on launch, à la `--resume` semantics).
- **Triage the Bridge** — the Concierge has a tool (an MCP server Lens
  exposes) that reads the Bridge queue. It can `Resolve`, `Undefer`,
  or `Reply` to items; routing through Lens keeps the Bridge badge
  counters in lockstep.
- **File knowledge into Bridge** — another tool the Concierge reads/writes is
  the Bridge notebook (per-session/per-project knowledge — the application
  shell owns Bridge's surface).
- **Orchestrate cross-session follow-ups** — the Concierge uses
  `POST /comments/send` to file agent-to-agent relays; the Bridge
  indexes them (§11).

The state model holds a `ConciergeHandle` (an ordinary `SessionHandle` on the
local connection, pinned Active by default). If the Concierge's session dies
or is deleted (404), Lens surfaces a "Concierge offline" state in the rail
and re-creates it on next launch. The Concierge is **single-user** — one
Concierge per Lens, never per-connection.

---

## 13. Error & lifecycle mapping + downstream contracts

### 13.1 Error & lifecycle mapping

`ClientError` (the typed client's §11) maps to app state:

| `ClientError` / signal | App-state effect |
|---|---|
| `ServerStreamEvent::Reconnecting { attempt }` (typed client §7) | Active → health `Reconnecting`; raise the amber `↻` immediately; record `since`/`attempts`. |
| `ServerStreamEvent::Disconnected` (retry phase expired, typed client §7) | Active → "hard disconnected" UI; offer user-retry (reopens via `Sessions::stream`). Session stays in registry. (A stream signal, not a `ClientError` variant — see typed client §11.) |
| `ServerStreamEvent::Reconnected { gap }` (typed client §7) | `gap == Some(0)`: keep state. Else: clear `StreamScratch` (§4.2), show `↻` break, reconcile. |
| `Auth { 401 }` | Prompt re-auth (permissions + server-lifecycle docs); do not drop sessions. |
| `Auth { 403 }` | Lost access → remove session from registry + UI. |
| `NotFound` (404) | Session deleted server-side → remove from registry; any disk rows remain as a read-only local tombstone (history viewable, never re-streamed). |
| snapshot `status == Failed` | Surface `last_task_error`; no retry. |
| `ContractMismatch` | Connection goes to "wrong version" state; the user is prompted to upgrade Lens or downgrade omnigent. **Never silently continue.** |
| `Network` / `Parse` / `Ws` | Log; surface a non-fatal transcript error marker. Unknown event types are already dropped by the typed client. |

### 13.2 Downstream contracts (the seams)

What each rendering document reads from this layer. These surfaces are fixed
here so the surface documents build on a stable model.

- **Conversation transcript** — reads `SessionState.items` through render-time
  transforms (§4.3); the context meter from `context_window` +
  `last_total_tokens`; status/usage for lanes; `todos` rendered inline (the
  agent's live per-session todos, capability map §0.3); `AgentChanged` items
  kept visible in the transcript but dropped from non-conversation surfaces.
- **Workspace & terminals** — reads `ResourceEvent` items + session workspace
  fields (`workspace`, `git_branch`, `host_type`, `host_id`, `sandbox_status`);
  terminal byte streams come **directly** from the typed client's WS client,
  *not* through the reducer — this layer only carries the
  `session.terminal.activity`/`terminal_pending` notifications that tell a
  card a terminal moved or is about to be created.
- **Agent definition** — reads `agent_id`/`agent_name`, `llm_model`/
  `model_override`, `reasoning_effort`, `skills`, `model_options`,
  `collaboration_mode`; issues model controls + switch-agent via the command
  flow (§7, §12.2).
- **Permissions & elicitations** — owns `pending_elicitations` (plural) +
  `permission_level` + `owner` + `presence`; replies via `Approval` (§7) or
  the `/resolve` endpoint, routed by `target_session_id` for mirrored child
  prompts. Records the verdict locally (the `elicitation_resolved` event carries
  none) and clears Bridge badges idempotently when the poll shows
  `pending_elicitations_count: N→0`.
- **Sub-agent topology** — reads `parent_session_id`, child refs (from
  `ChildSessionUpdated` folds), and `BlockContext.{agent,depth}`; child
  sessions are ordinary registry entries with their own stores (so the
  liveness/cap model, §3, applies to them too).
- **Application shell** — reads `presence` (co-viewer header), `host_type` +
  `host_id` (card host pill), `sandbox_status` (card sandbox badge),
  `cumulative_cost` + `usage_by_model` (board rollup, capability map §0.7-I),
  `status` + `last_task_error` + the derived needs-attention flag (card wave);
  dispatches navigation, focus, archive toggles, ⌘I (into the Bridge),
  ⌘D (deep-focus mode).
- **Bridge router** — owned here (§11); surfaces read `queue` +
  `badge_counts`. The application shell owns placement (capability map §0.7-H)
  but this layer owns the data + the Lens-side router.

---

## 14. Framework divergence notes

Almost everything above is plain Rust — the reducer, transforms, stores,
persistence, command logic, the Bridge router, presence folds,
switch-agent flow — are framework-neutral. The framework choice touches
exactly three points:

1. **State primitive / observation.** The foreground `SessionStore` replica and
   `AppState` need a reactive container with **per-session subscription
   granularity** (§9).
   - *gpui:* each foreground `SessionStore` is an `Entity<SessionState>`;
     subscribe via `cx.observe`. Per-entity notify gives the granularity for
     free. Canonical reduction happens in `ActiveSession`, which is plain
     off-thread Rust and is not a gpui entity.
   - *Alternative (React/TS over Tauri):* a store-per-session (Zustand instance
     per id, or a Jotai atom family); selectors give granularity but require
     discipline. Re-introduces the IPC seam this whole design set out to avoid
     (capability map §0.1) — a primary input to the framework decision.
2. **The runtime bridge (§8).** One seam with two message directions:
   `StreamUpdate` from `ActiveSession` to the foreground replica, and
   `SessionCommand` from UI to `ActiveSession`.
   - *gpui:* `cx.spawn` drains update channels and batches entity updates on the
     foreground executor.
   - *Alternative:* a Tauri IPC hop — reduced state deltas and commands must
     cross the JS boundary. The typed client's all-Rust event path becomes JSON
     at the seam — loses the all-Rust win.
3. **Transcript re-render** (progressive re-render semantics) — a
   conversation-transcript doc concern, noted here only because the substrate
   constrains it.

The local persistence layer (§6), the Bridge router (§11), presence
(§12.1), switch-agent (§12.2), and the Concierge (§12.3) are all
framework-independent (Rust + SQLite + typed-client).

---

## 15. Open questions

- **Auto-sleep quiescence threshold & poll cadence** — §3.2 / §10 give starting
  values (~10 min quiet, ~10s poll); tune against real usage in the
  verification pass (capability map §0.8). (The old hard stream cap `N` was
  dropped — §3.3.)
- **Disk retention policy** — how long Slept/server-archived sessions keep their `items`
  rows before pruning to a summary tombstone; whether the user controls it.
  The schema (§6.2) supports either.
- **Bridge integration depth** — §6.1 keeps the schema readable; whether Bridge
  reads the SQLite file directly vs. a small export API is a Bridge-side
  decision.
- **`client_os_*` inbound tools** — the bidirectional command path (§7) is
  reserved but unspecced pending the server-side feature (workspace &
  agent-definition docs).
- **Cross-session search** — board-as-grouping implies a fuzzy/search story
  over the registry + disk; its UX is the workspace doc's call, but the
  `items` table (§6.2) is the index it would query.
- **Bridge storage** — the router's own SQLite tables (planning todos,
  relay index) are sketched (§11) but their exact schema is deferred to the
  first build pass; the router's contract with omnigent (labels +
  `comments/send`) is pinned here.
- **Concierge MCP contract** — the Lens-exposed MCP server the Concierge reads
  (Bridge Inbox + Knowledge) is a forward spec; the boundary is sketched (§12.3)
  but the tool schema is not pinned here.
