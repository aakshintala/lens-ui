# Review: `app-architecture-and-state-model.md`

**Reviewer:** design-spec review subagent  
**Grounding:** omnigent @ `0.3.0.dev0` (`openapi.json`, `schemas.py`, `routes/sessions.py`, `presence.py`, `routes/comments.py`)  
**Spec target:** v0.2.0  
**Date:** 2026-06-24

---

## TL;DR

- **`PresenceViewer` is wrong** — spec fields (`display_name`, `is_owner`, `last_seen_at`) do not exist on the wire; openapi defines `user_id`, `joined_at`, `idle` only (`openapi.json` `PresenceViewer`, `schemas.py:2787–2804`).
- **Decision J “owner-only” contradicts omnigent 0.3.0** — `POST /switch-agent` gates on `LEVEL_EDIT`, not `LEVEL_OWNER` (`routes/sessions.py:14214–14216`); only `stop_session` adds an owner check (`17081–17088`).
- **Typed-client lifecycle drift** — foundation doc still describes “Sleeping cap ~8 concurrent streams”; this spec reshaped to Slept/`stop_session`/no cap (§3.3) but does not call for a typed-client rewrite.
- **Lifecycle persistence is underspecified** — four lifecycle states (§3) but SQLite schema has only `archived`; no `slept`/tombstone columns; wake/resume API (`PATCH runner_id` / `POST /hosts/.../runners`) never pinned.
- **Bridge router gaps** — no `response.elicitation_resolved` intake (sticky badges); relay discovery mechanism unspecified; §11 still marks Bridge *placement* “open” despite resolved decision H.

---

## Findings

### **[SEVERITY: blocker] [DIMENSION 1 — OpenAPI grounding / drift] `PresenceViewer` wire shape mismatch**

**Location:** §2.5 (`PresenceViewer` struct), §12.1  
**Evidence:** Spec defines `{ user_id, display_name?, is_owner, last_seen_at }`. Openapi + `schemas.py` define `{ user_id, joined_at, idle }` — no display name, no owner flag, no last-seen epoch. Presence is registered by holding `GET /v1/sessions/{id}/stream` open (`presence.py:19–32`, `openapi.json` stream route ~7943); owner identity comes from `GET /v1/me` + `GET /v1/sessions/{id}/owner`, not the viewer list.  
**Recommendation:** Replace `PresenceViewer` with the wire-faithful struct (`joined_at: String`, `idle: bool`). Derive “also viewing” chrome from `user_id`; derive “you don’t own this” from `permission_level`/`owner` vs `/v1/me`. Drop persistence of invented fields in §6.2 `presence` JSON column or document them as Lens-enriched cache filled asynchronously.

---

### **[SEVERITY: major] [DIMENSION 1 — OpenAPI grounding / drift] Switch-agent permission guard ≠ decision J**

**Location:** §12.2 “Guards (verified in source): caller is **owner**”  
**Evidence:** Resolved decision J and capability map §0.7-J say **owner-only**. Omnigent 0.3.0 requires `LEVEL_EDIT` (level 2) at the route gate (`routes/sessions.py:14214–14216`). Owner-only applies to `stop_session` (`17081–17088`), not switch-agent. Editors with grant level 2 can switch per current server code.  
**Recommendation:** Either (a) amend decision J / §12.2 to “edit-or-higher (level ≥ 2)” and disable switch in UI based on `permission_level`, or (b) file an omnigent change to require `LEVEL_OWNER` on switch-agent and keep J as written. Do not label as “verified owner-only” until aligned.

---

### **[SEVERITY: major] [DIMENSION 1 — OpenAPI grounding / drift] Switch “idle-only” guard is narrower than spec claims**

**Location:** §12.2, decision J  
**Evidence:** Server rejects only when `_session_status_from_cache(session_id) == "running"` (`routes/sessions.py:14237–14243`). It does **not** reject `waiting` or `launching`, both valid `session.status` values (`openapi.json` `SessionStatusEvent`, `schemas.py:2015–2042`). A parent parked on async-work drain (`waiting`) or a harness still booting (`launching`) could be switched mid-flight.  
**Recommendation:** Define idle as `{ idle, launching, waiting }` exclusion set matching product intent; extend server guards or add client-side preflight using live `SessionState.status` before POST.

---

### **[SEVERITY: major] [DIMENSION 2 — Cross-doc consistency] Typed-client lifecycle model stale vs this spec**

**Location:** §3 (no stream cap, Slept = `stop_session`); typed-client §4 lines 185–187  
**Evidence:** This spec (reshaped 2026-06-24) locks Active/Slept/Archived with `stop_session` reclaim and **no stream cap** (§3.3). Typed-client still says “Active/**Sleeping** cap, default **~8** concurrent streams” and “Sleeping sessions are repoll-ed.” That is the pre-reshape model this spec explicitly dropped (§3 intro).  
**Recommendation:** Add explicit “depends on typed-client §4 rewrite” note here, or patch typed-client in the same design pass. Until then, implementers will build the wrong liveness layer.

---

### **[SEVERITY: major] [DIMENSION 2 — Cross-doc consistency] §7 `stop_session` blurb contradicts §3.4**

**Location:** §7 command-flow bullet for `stop_session`  
**Evidence:** §3.4 states Sleep/Archive **do** `stop_session` and reclaim server-side. §7 says stop_session is “Distinct from … **sleep (client-side disconnect, §3.4)**” — the parenthetical describes the **old** model §3 explicitly superseded.  
**Recommendation:** Rewrite §7 `stop_session` bullet: Sleep/Archive and explicit Stop all use the same `stop_session` event; they differ in UI visibility (`archived`, dimming) and user intent, not wire semantics.

---

### **[SEVERITY: major] [DIMENSION 2 — Cross-doc consistency] Bridge placement marked “open” vs resolved decision H**

**Location:** §11 final paragraph (“Placement decision … is **open**”)  
**Evidence:** Decision H and `application-shell-and-layout.md` §10–§11 lock Bridge as **one left-rail destination** (Inbox + Log + Knowledge; ⌘I / ⌘⇧I). Capability map §0.7-H: **Resolved.**  
**Recommendation:** Remove “placement is open”; point to shell §11 for surface placement. Keep §11 focused on router data plane only.

---

### **[SEVERITY: major] [DIMENSION 3 — Completeness] Lifecycle state not persistable across restart**

**Location:** §2.2 `SessionState`, §3.1, §6.2 `sessions` table  
**Evidence:** §3 defines four lifecycle states; §313 claims “`archived: bool` and the **Slept flag** live on `SessionState` and persist.” Struct (§2.2) and schema expose only `archived INTEGER`. No `slept`, `deleted`, or `lifecycle` column. `ConnectionApp.pinned` (§9) is also RAM-only — restart loses pin/auto-sleep exclusions. Deleted tombstones (§3.1, §13.1) have no schema (`deleted_at`, `tombstone` flag).  
**Recommendation:** Add explicit persisted fields: `lifecycle TEXT NOT NULL` (`active|slept|archived|deleted`) or `slept INTEGER` + `deleted_at`; persist `pinned` per `(connection_id, session_id)`. Document startup rehydration: disk lifecycle + server poll merge.

---

### **[SEVERITY: major] [DIMENSION 3 — Completeness] Wake/resume command path undefined**

**Location:** §3.1, §3.5, §9 `navigate_to_session`  
**Evidence:** Wake described as “resume + re-bind runner + reconnect” but no API sequence. Omnigent uses `PATCH /v1/sessions/{id}` with `runner_id` and/or `POST /v1/hosts/{host_id}/runners` (`openapi.json`, `server-lifecycle.md` §6). After `stop_session`, runner binding may be cleared — wake must relaunch. Fork path (§7) returns `SessionResponse` immediately; wake does not mirror that pattern.  
**Recommendation:** Pin wake as an ordered sub-flow in §7: (1) `POST /hosts/{id}/runners` or reuse last `host_id`, (2) `PATCH` bind `runner_id`, (3) typed-client reconnect. Include error surfaces (host offline, 409 runner busy).

---

### **[SEVERITY: major] [DIMENSION 3 — Completeness] Lens-local `archived` vs server `archived` dual model**

**Location:** §2.2 `archived: bool`, §3.2, §6.2  
**Evidence:** Spec treats `archived` as Lens UI drawer flag + `stop_session`. Server exposes its own `archived` on `SessionListItem` / `PATCH` (`schemas.py:1844–1848`, typed-client `PATCH` params). Poll uses `include_archived` query (`openapi.json` `/v1/sessions`). No sync rule: Lens drawer vs server archive can diverge across connections/clients.  
**Recommendation:** Choose one source of truth — either mirror server `archived` via `PATCH` on archive action, or rename Lens field to `hidden_in_drawer` and stop overloading “archived”. Document multi-client behavior.

---

### **[SEVERITY: major] [DIMENSION 3 — Completeness] List poll cannot both “refresh known” and “discover new” as written**

**Location:** §10  
**Evidence:** §10 says poll “refreshes the **coarse** state of **all known sessions**” and also “surfaces **new sessions** created outside Lens.” A registry-only refresh never discovers server-side creates (fork via CLI, share grant). `GET /v1/sessions` is cursor-paginated (limit 1–1000, default 20 — `openapi.json` `/v1/sessions`).  
**Recommendation:** Split poll into (a) **discovery pass** — paginate full server list (or incremental `after` cursor stored in `meta`) merge into registry; (b) **summary refresh** — update known rows. Specify pagination/`include_archived`/`kind=default` filters.

---

### **[SEVERITY: major] [DIMENSION 3 — Completeness] Bridge router missing `response.elicitation_resolved` path**

**Location:** §11 routing fabric  
**Evidence:** Bridge indexes `response.elicitation_request` + polled `pending_elicitations_count` (§10). Capability map §0.3 and permissions doc require `response.elicitation_resolved` to decrement badges idempotently when prompts die without a verdict (`openapi.json` `ElicitationResolvedEvent`). §11 never subscribes Active streams to resolved events; Slept cards only get count, not identity cleanup.  
**Recommendation:** Add Bridge intake for `elicitation_resolved` (and poll transition `pending_elicitations_count: N→0`) to remove queue items and refresh `badge_counts` without stale Inbox rows.

---

### **[SEVERITY: major] [DIMENSION 3 — Completeness] Agent-to-agent relay discovery unspecified**

**Location:** §11 “Relays … POST /comments + POST /comments/send”  
**Evidence:** Endpoints exist (`routes/comments.py:322`). Spec says Bridge “indexes the comments stream by label” but comments are not SSE events — no push feed. `SessionListItem` exposes `comments_count` / `comments_updated_at` (`schemas.py:1849–1863`) as invalidation fingerprint, not comment bodies. No poll of `GET /comments` (if it exists) or snapshot hook defined.  
**Recommendation:** Specify relay ingestion: e.g. on `comments_updated_at` change in list poll → fetch comments, filter `labels["bridge:relay"]`, upsert Bridge queue. Pin label key and dedup by `comment_id`.

---

### **[SEVERITY: major] [DIMENSION 3 — Completeness] `AgentChanged` durable item — seq/reconcile strategy missing**

**Location:** §2.3 `ItemKind::AgentChanged`, §4.1, §6.2 `items` PK `(connection_id, session_id, seq)`  
**Evidence:** `session.agent_changed` is **transient** SSE-only (`schemas.py:2213–2215`). Server does not persist a switch marker item. Spec reducer inserts client-side `AgentChanged { from, to, at }` item, but event carries only `agent_id`/`agent_name` (no `from`). Items table requires server `seq` for PK; synthetic items need allocation rule. Wake-from-disk after switch-while-slept loses marker unless persisted.  
**Recommendation:** Define synthetic seq namespace (e.g. negative or `local:*` ids excluded from server reconcile), capture `from` from pre-update `SessionState.agent_id`, persist on reduce. On wake, do not expect marker from `GET /items`.

---

### **[SEVERITY: major] [DIMENSION 5 — Feasibility] Auto-sleep “terminal activity” undefined for streamless sessions**

**Location:** §3.2 auto-sleep exclusions  
**Evidence:** `session.terminal.activity` is transient, stream-only (`openapi.json`, typed-client §7 transient list). Slept sessions have **no stream** (§3.1). “Recent terminal activity” exclusion requires a Lens-local `last_terminal_activity_at` updated while Active, plus a TTL — not specified. Without it, auto-sleep may kill PTY mid-output right after a false “idle” reading.  
**Recommendation:** Add `last_terminal_activity_at` to `SessionState`/SQLite; define TTL (e.g. activity within last 60s blocks sleep). Terminal WS attach activity should also bump this timestamp (workspace doc cross-ref).

---

### **[SEVERITY: major] [DIMENSION 5 — Feasibility] Cost-sample series — global rollup algorithm absent**

**Location:** §6.2 `cost_samples`, §2.5 decision I  
**Evidence:** Table stores per-session cumulative samples; caveat acknowledges attribution skew when Lens closed. No spec for cross-connection **global** aggregation (sum deltas per window across all `(connection_id, session_id)` pairs), dedup when same session moves connections, or retention/pruning (unbounded table growth). Shell §17 expects today/7d/30d readout.  
**Recommendation:** Add §6.4: sample on every `session.usage` + poll; retention window ≥ 30d; global query = `SUM(max(cost_at_end) - cost_at_start)` per session per window with connection grouping; document Concierge/local-server exclusion if any.

---

### **[SEVERITY: minor] [DIMENSION 1 — OpenAPI grounding / drift] `SessionListItem.status` enum narrower than `SessionStatusValue`**

**Location:** §2.2 `SessionStatusValue` (`Launching`, `Waiting`, …)  
**Evidence:** Poll refreshes Slept cards from `GET /v1/sessions`. `SessionListItem.status` in `schemas.py:1869` is `Literal["idle", "running", "failed"]` — no `launching`/`waiting`. Stream events carry full set (`SessionStatusEvent`). Slept-card badges driven by poll may show `idle` while server stream would show `waiting`.  
**Recommendation:** Treat poll status as coarse for Slept cards only; note stream authority for Active. On bump to 0.3.0, track whether openapi narrows or expands list status.

---

### **[SEVERITY: minor] [DIMENSION 1 — OpenAPI grounding / drift] Presence `idle` stream query param not reflected in state model**

**Location:** §12.1  
**Evidence:** Stream route accepts `idle` query param at connect; mid-view idle flip requires reconnect (`routes/sessions.py:17758–17761`). Spec says Lens is receive-only for presence (fine) but omits that **opening** a stream registers the viewer — Active sessions always broadcast presence unless typed-client passes `idle=true`.  
**Recommendation:** Document: Active pump opens stream with `idle=false` when focused, `idle=true` when window backgrounded (reconnect on focus change). Aligns with server presence semantics.

---

### **[SEVERITY: minor] [DIMENSION 1 — OpenAPI grounding / drift] Future presence broadcast via `POST /events` unverified**

**Location:** §12.1 “mechanism … would be `POST /events` carrying a `presence`-shaped payload”  
**Evidence:** No `presence` discriminator in `_ALLOWED_EVENT_TYPES` / route dispatch (`routes/sessions.py:16860+`). Presence is stream-registration only (`presence.py`).  
**Recommendation:** Mark as hypothetical; remove “typed client's enum reservation covers it” unless typed-client explicitly adds a non-wire enum variant labeled future.

---

### **[SEVERITY: minor] [DIMENSION 2 — Cross-doc consistency] Typed-client claims `PUT /agent` fires `session.agent_changed`**

**Location:** Cross-ref typed-client §3 line 90 vs this spec §12.2  
**Evidence:** This spec correctly separates bundle storage (`PUT /agent`) from swap trigger (`POST /switch-agent`). Typed-client table blames `PUT /agent` for `session.agent_changed`. Switch route emits the event (`routes/sessions.py:14353`).  
**Recommendation:** Fix typed-client table; add cross-doc note here that bundle upload alone must not expect `agent_changed`.

---

### **[SEVERITY: minor] [DIMENSION 3 — Completeness] `session.agent_changed` absent from typed-client persisted/transient table**

**Location:** §4.1 session-field folds; typed-client §7 classification  
**Evidence:** Typed-client §7 “Persisted” list omits `session.agent_changed`. `schemas.py` categorizes it **transient**. Spec folds it into scalars **and** synthesizes durable item — hybrid handling not mirrored in typed-client contract.  
**Recommendation:** Add `session.agent_changed` to typed-client **Mixed** or document state-model-only handling in §13.2 downstream contract.

---

### **[SEVERITY: minor] [DIMENSION 3 — Completeness] Hard-disconnect vs Slept lifecycle interaction**

**Location:** §13.1 `Disconnected` → “Active → hard disconnected UI”  
**Evidence:** No rule for Slept/Archived sessions (no pump). If user wakes during network outage, ordering of resume vs reconnect failures unstated. `ContractMismatch` affects whole connection — unclear if Slept disk snapshots remain browsable offline.  
**Recommendation:** Extend §13.1 matrix: Slept/Archived remain disk-readable offline; Active disconnect offers retry; wake during outage fails at resume step with distinct UX.

---

### **[SEVERITY: minor] [DIMENSION 3 — Completeness] Fork flow registry update underspecified**

**Location:** §7 `fork` bullet  
**Evidence:** `POST /v1/sessions/{source_id}/fork` returns 201 + `SessionResponse` (`routes/sessions.py:13982–14019`). Spec says new session “arrives via list poll or immediate create-response” but §7 doesn’t require optimistic registry insert from response (unlike send optimism).  
**Recommendation:** On fork success, insert `SessionHandle` from response body immediately; poll is backstop only.

---

### **[SEVERITY: minor] [DIMENSION 4 — Clarity] Switch-agent source line citation stale**

**Location:** §12.2 `routes/sessions.py:14176`  
**Evidence:** Line number matches 0.3.0 checkout but will drift; guard description incorrectly says owner-only (see major finding above).  
**Recommendation:** Cite symbol `@router.post("/sessions/{session_id}/switch-agent")` and link guard table to permission levels not line numbers.

---

### **[SEVERITY: minor] [DIMENSION 5 — Feasibility] Persisting transient `presence` to SQLite**

**Location:** §6.2 `presence TEXT`, §12.1  
**Evidence:** Typed-client classifies `session.presence` as **transient** (gone on reconnect). Persisting stale viewer lists to disk misleads header chrome after restart until stream reopens.  
**Recommendation:** Do not persist `presence`; keep RAM-only on `SessionState` or clear on sleep/wake.

---

### **[SEVERITY: nit] [DIMENSION 1 — OpenAPI grounding / drift] Version pin vs checkout**

**Location:** Header “0.2.0 chrome” throughout  
**Evidence:** Checked-out omnigent is `0.3.0.dev0` (`pyproject.toml`). Openapi info version string still `"0.1.0"` (`openapi.json:3786`) — metadata lag, not absence of features reviewed.  
**Recommendation:** Re-run contract pin against 0.3.0.dev0 before implementation; treat 0.2.0 labels as intent not guarantee.

---

### **[SEVERITY: nit] [DIMENSION 4 — Clarity] §7 `SessionEventInput` “0.2.0” discriminator list vague**

**Location:** §7 paragraph 613–614  
**Evidence:** Says “message, function_call_output, approval, interrupt, stop_session, and others the typed client enumerates” — incomplete vs typed-client §6 and growing external_* types in 0.3.0 (`routes/sessions.py:16802–16813`).  
**Recommendation:** Replace with “exhaustive set in typed-client §6; state model dispatches subset.”

---

### **[SEVERITY: nit] [DIMENSION 5 — Feasibility] Bounded channel backpressure vs reducer single-writer**

**Location:** §8  
**Evidence:** Pump awaits channel send when UI slow; reducer runs in pump task before send — correct. Not stated whether `SessionStore` applies deltas synchronously on UI thread or spawns — gpui §14 leaves executor choice open. Risk of UI-thread reducer work if mis-implemented.  
**Recommendation:** Add invariant: reducer always runs on pump/async side; channel carries already-reduced `SessionState` patches or small deltas, never raw `ServerStreamEvent`.

---

## Version drift summary (0.2.0 spec → 0.3.0.dev0 source)

| Area | Spec assumption | 0.3.0.dev0 observation |
|------|-----------------|------------------------|
| Presence viewer fields | `display_name`, `is_owner`, `last_seen_at` | `joined_at`, `idle` only |
| Switch-agent auth | Owner-only (decision J) | `LEVEL_EDIT` at route |
| Switch idle guard | Idle-only | Rejects `running` only |
| List session status | Full lifecycle enum in UI | `SessionListItem.status` truncated in `schemas.py` |
| Session archive | Lens-local bool | Server `archived` + `include_archived` poll param |
| Comments / relay | Label-indexed | `comments_count`/`comments_updated_at` on list items |
| Contract version | v0.2.0 pin | Package `0.3.0.dev0`; openapi metadata `0.1.0` |

---

## What checks out (brief)

- Core endpoints referenced (`/stream`, `/fork`, `/switch-agent`, `/comments/send`, `/events` `stop_session`) exist in openapi and routes.
- `session.agent_changed`, `session.presence`, `session.usage`/`total_cost_usd`, fork/switch request bodies match openapi components.
- Multi-connection `(ConnectionId, SessionId)` composite key, reducer/`StreamScratch` split, Bridge as Lens-side router over comments+elicitations, decision A (session=work unit), G (multi-window `AppState`), I (cost_samples for windowed spend) are structurally sound once gaps above are closed.
- Sub-agent drill-in (decision B), terminal ring buffer (C), gpui (D), ⌘D working area (F) are delegated to sibling docs appropriately in §13.2.
