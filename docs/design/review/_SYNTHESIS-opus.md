# Lens design-spec review — cross-cutting synthesis (Opus)

**Role:** Synthesis lead. This consolidates 8 per-doc bulk findings + the grounding baseline + the GPT-5.5 second opinion, adjudicated against omnigent source at HEAD `36b2a11c` (`/Users/aakshintala/work/omnigent`, package `0.3.0.dev0`, `openapi.json` info.version `"0.1.0"`, OpenAPI 3.2.0, 59 REST paths).
**Scope:** systemic clustering + adjudication, not per-line re-review. Nothing under `/omnigent` or any spec was modified.
**Settled decisions assumed (A–J):** A task=session · B sub-agent child window, never board cards · C Lens-side terminal ring buffer · D framework=gpui · E full permissions/sharing/multi-user + per-connection token auth · F collapsible working area beside chat + ⌘D deep-focus · G multi-window native · H Bridge = one rail destination (Inbox+Log+Knowledge) · I spend readouts (per-card cumulative + windowed global) · J live switch-agent in place, transcript stays, owner-only + idle-only.

---

## 1. Executive summary

- **The typed-client contract layer is the root of most blockers.** Three independent contract errors — version gate on the wrong endpoint, a 3-vs-5 state `session.status` enum mismatch, and a reconnect protocol keyed off a field that does not exist — propagate into server-lifecycle, app-architecture, and application-shell. Fix the contract layer first; ~6 downstream findings collapse with it.
- **Decision J is mis-grounded in 4 docs.** `POST /switch-agent` gates on `LEVEL_EDIT` (2), **not owner (4)** — confirmed in source (`routes/sessions.py:14214`, docstring "403 if the caller lacks edit access", `auth.py:76-79`). Owner-only is a legitimate *Lens UI policy* on top of the API, but every "verified in source: owner-only" claim is false and must be relabeled. The idle guard rejects `running`+`waiting` (collapsed) but **not** `launching`.
- **Harness count resolved authoritatively at 19.** `OMNIGENT_HARNESSES` (`spec/_omnigent_compat.py:80-101`) is the canonical validator-accepted set of **19** and is what Lens should treat as canonical. The "16", "11", "18", and "20" figures in circulation are all wrong or partial (explained in §2 / T4). `AgentObject.harness` is now a free `string|null`, not an enum.
- **Verification posture is broken.** README claims a "checked-in `openapi.json`" and a "load-bearing GPUI reconnaissance" artifact; **neither exists in the lens repo** (`**/openapi*.json` → 0 files; `**/recon*` → 0 files). Every "verifiable against the checked-in openapi" claim is, in this repo, unverifiable. This must be fixed before any contract claim can be trusted.
- **Decision E is internally contradicted.** Capability-map §0.2 still says "Lens does not implement the multi-user/sharing UI" while §0.7-E and README resolve full sharing in scope. Grant levels are also mis-stated as 1–4 (server accepts **1–3**; owner=4 is implicit/non-grantable).
- **The state model assumes a single pending elicitation; the wire is plural.** `SessionResponse.pending_elicitations` is a `list` (`schemas.py:1630`); fan-out parents mirror multiple child prompts. `PresenceViewer` fields are also invented (`display_name/is_owner/last_seen_at` vs wire `user_id/joined_at/idle`).
- **Three resolved decisions (D, H, I) are still labeled "open/gated" somewhere**, and Bridge rail behavior is internally contradictory in the shell. These are cheap edits but make the spec set look unsettled.
- **Two systemic security/grounding clusters in content docs:** markdown/link/image sanitization is specified at high rigor in `framework.md` §2.5 but not uniformly applied in transcript channels; and several wire item/status/event shapes (tool-span status, `CompactionFailed`, persisted `error` items, terminal WS `/v1` prefix) drift from source.

---

## 2. Systemic themes (clustered, root-caused)

Each theme states a single root cause and the **exact set of docs/sections that must change**. Themes are ordered roughly by blast radius.

### T1 — Contract gate probes the wrong endpoint for version *(root: `/v1/info` ≠ version)*

**Root cause.** Specs assume `GET /v1/info` carries the server semver for the `PINNED_OMNIGENT_VERSION` gate. Source: `/api/version` returns `{"version": "<semver>"}` (`app.py:1479-1491`); `/v1/info` is an **unauthenticated capability/auth probe** returning `accounts_enabled, login_url, needs_setup, databricks_features, managed_sandboxes_enabled, sandbox_provider` — **no version field** (`app.py:1493-1564`). Note `/v1/info` is not pure booleans (`login_url`, `sandbox_provider` are strings/nulls), so the "only capability flags" phrasing is slightly overstated but directionally correct.

**Affected docs:** `typed-client.md` §8 (+ §1 scope) · `server-lifecycle.md` §8 + §3.1 ready detection · `capability-map-and-design-language.md` §0.3 contract-gate row (~:215).
**Fix:** gate semver on `GET /api/version`; use `/v1/info` for capability/first-run probes; use `GET /health` for liveness. Ready-detection ladder = `/health` (live) → `/api/version` (contract pin) → `/v1/info` (capabilities).

### T2 — `session.status` enum drift: 3-state REST poll vs 5-state SSE

**Root cause.** `SessionResponse.status` and `SessionListItem.status` are `Literal["idle","running","failed"]` (`schemas.py:1604`, `:1869`). SSE `SessionStatusEvent.status` is `Literal["idle","launching","running","waiting","failed"]` (`schemas.py:2067`). The server cache helper collapses `waiting`→`running` and treats unknown as `idle` (`sessions.py:1792-1811`). So Active (SSE) cards see 5 states; Slept/Archived (poll-fed) cards see only 3.

**Affected docs (mis-state poll as 5-state):** `application-shell-and-layout.md` §5.1 wave ladder + §17.4 background poll · `capability-map-and-design-language.md` §0.3/§0.6 (omits `waiting`) · `app-architecture-and-state-model.md` §2.2 `SessionStatusValue` (must mark poll status coarse — minor).
**Fix:** pin reducer rule — Active sessions fold full 5-state from SSE; poll-only cards use 3-state coarse mapping (`running` subsumes launching/waiting) and **persist last fine-grained status in Lens SQLite** so Slept cards don't regress to `idle`. Add `waiting` to the card-wave/urgency vocabulary.

### T3 — switch-agent guard: `LEVEL_EDIT` not owner; idle guard rejects `waiting` not `launching`

**Root cause.** Route calls `_require_access_and_level(..., LEVEL_EDIT, ...)` (`sessions.py:14214`); docstring: "403 if the caller lacks edit access." `LEVEL_EDIT=2`, `LEVEL_OWNER=4` (`auth.py:76-79`). The idle guard rejects only when `_session_status_from_cache(session_id) == "running"` (`sessions.py:14239`), and that helper maps `waiting`→`running` (`sessions.py:1806-1808`) — so `waiting` **is** rejected, but `launching` falls through to `idle` and is **not** rejected.

**Reconciliation with decision J.** J ("owner-only + idle-only") is legitimate as a *Lens UI policy stricter than the API*. The error is the grounding claim, not the product choice. Every "verified in source / owner-only at the API" statement is false.
**Affected docs:** `agent-definition.md` §7 ("Requires LEVEL_OWNER … verified in source" — blocker) · `app-architecture-and-state-model.md` §12.2 ("Guards (verified in source): caller is owner") · `capability-map-and-design-language.md` §0.7-J ("owner-only and idle-only verified in source") · `permissions-and-elicitations.md` §7 (UI `< 4` disables — consistent with J as policy, but cross-ref must say API floor = edit) · `typed-client.md` §3 nit.
**Fix:** state API floor = `LEVEL_EDIT (2)`; Lens UI floor = owner (decision J, a Lens policy) and drop "verified in source." For idle: UI disable set = `{running, waiting, launching}`; server enforces `running`+`waiting` only, so Lens must client-preflight `launching` before POST.

### T4 — Harness count: authoritative answer is **19**

**Root cause.** Multiple registries with different purposes were conflated. Verified directly in source:

| Registry | Source | Count | What it is |
|---|---|---|---|
| `OMNIGENT_HARNESSES` | `spec/_omnigent_compat.py:80-101` | **19** | Validator-accepted canonical spec harness names. **Canonical for Lens.** |
| `OMNIGENT_HARNESS_ALIASES` | `_omnigent_compat.py:104-118` | 11 | Accepted aliases (claude, opencode, agy, github-copilot, …) normalized before dispatch. |
| `_HARNESS_MODULES` keys | `runtime/harnesses/__init__.py:34-124` | 20 | Runtime subprocess module map. Includes 2 alias keys (`claude`, `opencode`) → **18** canonical modules; **omits `open-responses`** (adapter-routed, no dedicated wrap). |
| `AgentHarnessType` Literal | `runtime/workflow.py:143` | 7 | Narrow ucode-gateway set — **not** the picker list. |

**Explanation of the drift:** "16" = stale spec list (missing `goose`, `hermes`); "11" = a stale intro typo; "18" = `_HARNESS_MODULES` minus aliases (correct module count but drops `open-responses`, which the validator still accepts); "20" = raw `_HARNESS_MODULES` key count including aliases. **Lens canonical = the 19 in `OMNIGENT_HARNESSES`**, with aliases documented separately and a note that `open-responses` is accepted-but-adapter-routed.
**Affected docs:** `agent-definition.md` §4 table + intro "11-harness" + §2 comment + `OMNIGENT_HARNESSES` "20" claim · `typed-client.md` §9 (claims 16; the "hermes not in canonical 16" line is wrong — hermes IS canonical) · `capability-map-and-design-language.md` §0.3 (~:180). Fix `harness_aliases.py` path → `omnigent/harness_aliases.py`; note `AgentObject.harness` is now free `string|null` (`openapi.json:62-71`).

### T5 — pending elicitation: singular Option vs plural wire

**Root cause.** Server snapshot `SessionResponse.pending_elicitations: list[dict]` (`schemas.py:1630`); `SessionListItem.pending_elicitations_count` (`:1882`) and `ChildSessionSummary.pending_elicitations_count` (`:664`). A fan-out parent mirrors multiple child prompts onto its own stream keyed by `target_session_id` (integration test `test_two_children_elicitations_isolated_on_parent_stream`).
**Docs using a singular Option / single-pending assumption:** `app-architecture-and-state-model.md` §2.2 `pending_elicitation: Option<Elicitation>` + §11 Bridge index (no `target_session_id`) · `permissions-and-elicitations.md` §2 lifecycle (folds one) · `sub-agent-topology.md` §6 (dedupe rule).
**Fix:** model `pending_elicitations: Vec<Elicitation>` (or map keyed by `elicitation_id`/`target_session_id`); composer docks **one focused** prompt; card/Bridge badges use the count; add `target_session_id: Option<SessionId>` to `Elicitation` and to the Bridge item so resolve routes to the correct (possibly child) session.

### T6 — Verification posture: nothing is actually vendored

**Root cause.** README:9 asserts "every endpoint/event assertion is verifiable against the checked-in `openapi.json`"; typed-client §11 specifies `vendor/omnigent-0.2.0/openapi.json`; framework §2 + README:76 cite a "load-bearing GPUI reconnaissance" note. In the lens repo: `**/openapi*.json` → **0 files**, `**/recon*` → **0 files**. Ground truth lives only in the sibling omnigent checkout. This makes the entire pin-and-verify story inoperable today and silently couples the specs to an unpinned moving target (`0.3.0.dev0`, not the claimed `0.2.0`).
**Affected docs:** `README.md` (companion-artifacts + ground-truth claims) · `typed-client.md` §11 + header · `capability-map-and-design-language.md` §0.8 · `framework.md` §1.3/§2.
**Fix:** vendor `openapi.json` into `vendor/omnigent-<pin>/` with a `VERSION`/`OMNIGENT_PIN` file + CI diff against the sibling pin; **either** check in a slim recon artifact (`docs/design/recon/gpui-recon-*.md`) **or** downgrade "recon retired most widget risk" to "hypothesis pending spike" (see T-D). Bump the stated pin from `0.2.0` to the actual `0.3.0.dev0` (note `openapi.json` info.version `"0.1.0"` is stale metadata — trust package semver/tests, not `info.version`).

### T7 — Elicitation placement conflict (composer vs in-transcript)

**Root cause.** `conversation-transcript.md` §18 + `permissions-and-elicitations.md` §3 dock the widget **at the composer** with an in-transcript record marker. `application-shell-and-layout.md` §19 seam still says "in-transcript + attention."
**Fix:** update shell §19 to "composer dock + in-transcript record marker"; carry Retry/Edit-and-resend affordances (shell §17.3) into transcript §11 or add an explicit pointer.

### T8 — Archive semantics: Lens local-hide vs server `archived`

**Root cause.** Lens Archive = local drawer-hide + `stop_session`. Server has its own `archived: bool` on snapshot/list, toggled via `PATCH /v1/sessions/{id}` and filtered by `include_archived` on `GET /v1/sessions` (`schemas.py:1885`; `openapi.json` `/v1/sessions`). These are independent; server archive does **not** imply `stop_session`. On multi-client fleets the two diverge.
**Affected docs:** `application-shell-and-layout.md` §4.6 · `app-architecture-and-state-model.md` §2.2/§3.2/§6.2.
**Fix:** add a reconciliation table — either mirror server `archived` via `PATCH` on the Archive action (single source of truth) or rename the Lens field to `hidden_in_drawer` and stop overloading "archived." Document restore + multi-client behavior and the `include_archived`/`kind`/`search_query` poll filters.

### T9 — Reconnect model: snapshot-restored chrome vs item replay, and a non-existent dedup key *(contract blocker)*

**Root cause.** typed-client §7 reconnect step 5 dedups `GET /items` rows by `sequence_number`, but persisted `ConversationItem` has **no** `sequence_number` (`entities/conversation.py:628-675`) — the protocol as written cannot run. Separately, §7 classifies most `session.*` chrome (`status`, `created`, `usage`, `model`, `todos`, `agent_changed`, …) as "persisted / replayable from `GET /items`", but openapi marks them **transient (SSE-only)**, with reconnect state restored from the **session snapshot**, not item replay.
**Affected docs:** `typed-client.md` §7 + §10 · `app-architecture-and-state-model.md` §4.1 (folds + synthesizes `AgentChanged` item) · `agent-definition.md` §7 step 4 (expects model/skills from `agent_changed` payload — must refetch snapshot).
**Fix:** three-bucket reconnect — (A) **item-backed** transcript from `GET /items` merged by **item id** (not seq); (B) **snapshot-restored** chrome (apply `GET /v1/sessions/{id}` scalars/collections); (C) **truly transient** (deltas/heartbeats/presence, never persisted). `sequence_number` dedup applies only to the live SSE overlap window. `session.agent_changed` carries `agent_id`/`agent_name` only (`schemas.py:2192-2221`) — synthesize the `from` from prior reducer state, allocate synthetic local item ids, and don't expect the marker from `GET /items` on wake.

### T10 — Permission grant levels 1–3, not 1–4

**Root cause.** `GrantPermissionRequest.level = Field(ge=1, le=3)` → 1=read, 2=edit, 3=manage (`schemas.py:1893-1905`); owner (4) is creator/admin-derived and grant attempts on owner 403 (`sessions.py:18054-18113`). Public access capped at read.
**Affected docs:** `permissions-and-elicitations.md` §7 table · `typed-client.md` §3 permissions row · `capability-map-and-design-language.md` §0.3 permissions row (line ~195).
**Fix:** document grantable 1–3; owner via creation only; Share-link ≥ manage(3) is correct.

### T11 — Native permission hooks: four, not two

**Root cause.** openapi exposes `permission-request`, `codex-elicitation-request`, **`antigravity-elicitation-request`** (`openapi.json:5739`), **`cursor-permission-request`** (`:5821`). Lens never POSTs these (server-initiated), but the elicitation UI + `external_elicitation_resolved` race handling must tolerate all harness sources.
**Affected docs:** `permissions-and-elicitations.md` §6 · `typed-client.md` §3 hooks table · `capability-map-and-design-language.md` §0.3 (~:193).

### T12 — `PresenceViewer` invented fields

**Root cause.** Wire shape is `{user_id, joined_at, idle}` (`schemas.py:2787-2804`); spec invents `{display_name, is_owner, last_seen_at}`. Owner identity comes from `GET /v1/sessions/{id}/owner` + `GET /v1/me`, not the viewer list; presence is registered by holding the SSE stream open.
**Affected docs:** `app-architecture-and-state-model.md` §2.5 + §12.1 + §6.2 (don't persist transient presence).
**Fix:** wire-faithful struct; derive owner chrome from `/owner` + `/v1/me` + `permission_level`; keep presence RAM-only.

### T13 — `SessionEventInput` discriminator set + missing `compact`

**Root cause.** typed-client §6 lists ~5 send types; route `_ALLOWED_EVENT_TYPES` unions ~20+ including `compact`, `slash_command`, `mcp_elicitation`, and the `external_*` forwarding family (`sessions.py:771-792`; `openapi.json` `POST …/events` ~5690). The parser/validator must accept the full dispatch table even though Lens only *sends* a subset. `compact` is also missing from state-model §7 command list.
**Affected docs:** `typed-client.md` §6 · `app-architecture-and-state-model.md` §7 · `capability-map-and-design-language.md` §0.3/§0.4.

### T14 — Terminal WS attach must include `/v1`

**Root cause.** Router defines `/sessions/.../attach` but `create_app` mounts it with `prefix="/v1"` (`app.py:1635-1642`); runner proxy + ap-web both use the `/v1`-prefixed URL. Citing `terminal_attach.py:130` alone (router-relative) is incomplete.
**Affected docs:** `workspace-and-terminals.md` §9.1 · `typed-client.md` §5. Wire path = `WS /v1/sessions/{session_id}/resources/terminals/{terminal_id}/attach?read_only=…`. Also document binary PTY frames + text JSON `{"type":"resize",…}` and `read_only` query.

### T15 — Host registration model: no REST `POST/DELETE /v1/hosts`

**Root cause.** Hosts appear via outbound WS tunnel (`omnigent host` / `host_tunnel.py`) + managed provisioning; REST is `GET /v1/hosts`, `GET /v1/hosts/{id}`, `POST …/directories`, `POST …/runners`, `GET …/filesystem` only. (See adjudicated conflict C5 — the workspace doc's "positive grounding" line listing `GET/POST/DELETE /v1/hosts` is wrong.)
**Affected docs:** `capability-map-and-design-language.md` (~:212) · `workspace-and-terminals.md`/`server-lifecycle.md` positive-grounding note.

### T16 — Local bootstrap must supervise the **host daemon**, not server alone

**Root cause.** omnigent 0.3 local stack: `_ensure_backend` → `_ensure_host_daemon` first; the daemon (`omnigent host --local`) runs `ensure_local_omnigent_server()` **and** registers the host + spawns runner tunnels (`cli.py:2385-2436`, `host/_daemon_entry.py`). `server start` alone does not start the daemon; workspace/terminal/fs routes proxy to the runner and fail without it. Teardown order is daemon-first then server.
**Affected docs:** `server-lifecycle.md` §3.1/§3.2/§6 + Concierge §3 intro (Concierge needs a runner) + local auth §2/§4. Also pin the actual argv (`sys.executable -m omnigent.cli server …`, not `omnigent server start`) and explicit `--database-uri`/`--artifact-location`.

### T17 — Content-doc wire shapes (transcript) drift from source

**Root cause.** A cluster of item/event grounding errors in `conversation-transcript.md`: tool-span status uses `pending/running/error` vs wire `in_progress/completed/action_required/incomplete` (`schemas.py:2648-2651`); `CompactionFailed` mandates an error marker but source says dismiss with **no** permanent marker (`schemas.py:3207-3215`); persisted `error` item kind is missing from `ItemKind` (`conversation.py:323-341`); `TerminalCommandData`, `SlashCommandData`, `ReasoningData`, `native_tool` nesting, multimodal `input_image`/`input_file`, `is_meta` hide, interrupted-partial messages, `output_file.done`, and `elicitation_resolved`-without-verdict (`↯ cancelled`) all need render paths.
**Affected docs:** `conversation-transcript.md` §6–§15 · `app-architecture-and-state-model.md` §2.3 `ItemKind` (add `Error`, fix `TerminalCommand`/`SlashCommand` field names).

### T18 — Markdown / link / image sanitization not uniform across channels

**Root cause.** `framework.md` §2.5 defines `validate_link_url`/`validate_image_ref` (block `javascript:`/`file:`/`data:`, path-traversal/symlink guards). Transcript §6.2 autolinks user paths/URLs and renders opt-in ` ```markdown ` blocks with no stated equivalent boundary. Also the elicitation `params.url` (T-permissions) is currently a relative `/approve/{session_id}/{elicitation_id}` page (`runtime/policies/approval.py:209-228`), **not** an external OAuth URL — Lens must validate scheme/origin and never blindly `open()` it.
**Affected docs:** `conversation-transcript.md` §6 (add a §6.3 security boundary) · `permissions-and-elicitations.md` §3/§4 (split server-approval-page url-mode from true external OAuth; add `validate_elicitation_url`).

### T19 — `response.elicitation_resolved` carries no verdict

**Root cause.** `ElicitationResolvedEvent` has only `type` + `elicitation_id` (`schemas.py:2936-2962`); approvals don't persist as conversation items. Verdict (`approve/decline/cancel`) is known only from the locally-submitted `ElicitationResult`.
**Affected docs:** `permissions-and-elicitations.md` §2 (record verdict locally before resolve; default `↯ cancelled` when resolved without a prior local verdict) · `conversation-transcript.md` §18 (add `↯ cancelled`/`timed out` marker path) · `app-architecture-and-state-model.md` §11 (Bridge must intake `elicitation_resolved` + poll `pending_elicitations_count: N→0` to clear sticky badges idempotently).

### T20 — Lifecycle persistence, wake/resume path, global spend rollup *(state-model completeness)*

**Root cause.** §3 defines four lifecycle states but SQLite has only `archived`; no `slept`/`deleted`/`lifecycle` column, `pinned` is RAM-only, tombstones unschematized. Wake = "resume + rebind runner + reconnect" with no API sequence (`POST /hosts/{id}/runners` → `PATCH {runner_id}` → reconnect). Global windowed spend (decision I) has a `cost_samples` table but no cross-connection aggregation/retention algorithm.
**Affected docs:** `app-architecture-and-state-model.md` §2.2/§3/§6.2/§7 · `application-shell-and-layout.md` §7.4/§17.1 (decision-I chrome placement) · `server-lifecycle.md` §6 (wake runner relaunch + 409 retry).

---

## 3. Decision A–J compliance table

| Dec | Statement | Status | Violations / notes |
|---|---|---|---|
| **A** | task = session (no Task entity) | ✅ Honored | Capability-map retires "Task"; consistent across set. (Code remnant: `ChildSessionSummary.agent_name`/`current_task_id` are `None` in 0.3.0 — affects sub-agent labels, not decision A.) |
| **B** | sub-agent drill-in → child window, never board cards | ✅ Honored | shell §14/§5.4, sub-agent-topology, transcript §8.6 aligned. Deep trees need multi-hop client fetch (`child_sessions` is direct-children only) — completeness gap, not a violation. |
| **C** | Lens-side terminal ring buffer | ✅ Honored | workspace §9.2, typed-client §5 aligned. **Caveat to add:** ring buffer covers client WS reconnect to a *live* server only; server/runner restart kills the PTY and needs full re-attach. |
| **D** | framework = gpui | ⚠️ Partial | framework §1 + capability-map §0.7-D say resolved, but **README:27 + capability-map index row still say "gated"**, and the load-bearing recon artifact is **missing** (T6). "Recon retired most widget risk" overstated — markdown + JSON-Schema-form renderers remain un-spiked. |
| **E** | full permissions/sharing/multi-user + per-connection token auth | ❌ Violated | **capability-map §0.2 contradicts E** ("Lens does not implement the sharing UI"). Grant levels mis-stated 1–4 (server 1–3, T10). Auth collapsed to `OMNIGENT_AUTH_ENABLED` vs real provider matrix `header|oidc|accounts` (`auth.py:15-20`); 401 + `login_url` handling missing. |
| **F** | collapsible working area beside chat + ⌘D | ⚠️ Partial | shell §7.1/§8.1 aligned ("no right icon-rail"). But **"collapsible right rail" vocabulary persists** in capability-map index + README; keyboard `⌘\`/`⌘F` drift between shell and capability-map §0.6. |
| **G** | multi-window native | ✅ Honored | shell §3/§6, app-arch §9. Multi-window edge cases (menu-bar badge owner, ⌘I raise behavior, detached Bridge tab) deferred — acceptable. |
| **H** | Bridge = one rail destination (Inbox+Log+Knowledge) | ❌ Violated | **app-architecture §11 still marks placement "open"**; capability-map §0.7-H lacks the resolved "left rail"; **shell §6 internally contradictory** ("replaces the main area" vs "shrinking working-area tab") and board-only (no focused session) Bridge entry undefined. |
| **I** | spend readouts (per-card cumulative + windowed global) | ❌ Violated | **shell §21 lists I under "open/deferred"** despite resolved; no chrome placement for global today/7d/30d in §7.4; state-model global rollup algorithm + retention absent; behavior when `total_cost_usd` is `None` unspecified. |
| **J** | live switch-agent in place; transcript stays; owner-only + idle-only | ❌ Mis-grounded | API floor is `LEVEL_EDIT` not owner (T3) — "verified in source: owner-only" false in agent-definition §7, app-arch §12.2, capability-map §0.7-J, typed-client §3. Idle guard rejects `waiting` (collapsed) but **not** `launching`. *Transcript-stays-on-switch* portion ✅ aligned (transcript §4/§13/§20). J as a Lens UI policy is fine; the grounding labels are not. |

**Summary:** A, B, C, G honored (C with one caveat). D, F partial (stale labels + missing recon). E, H, I, J have real violations requiring edits.

---

## 4. Adjudicated conflicts

Where reviewers disagreed or may have overstated, with a ground-truth ruling.

**C1 — Does switch-agent reject `waiting`?**
`app-architecture` review claims the route "does **not** reject `waiting` or `launching`." **Ruling: half wrong.** Source rejects `running`, and `_session_status_from_cache` collapses `waiting`→`running` (`sessions.py:1806-1808`, `14239`), so **`waiting` IS rejected**. `launching` is not a cache value and falls through to `idle` → **not rejected**. The second-opinion correction stands; the real gap is `launching`.

**C2 — Harness count (16 / 18 / 19 / 20).**
**Ruling: 19** canonical accepted harnesses (`OMNIGENT_HARNESSES`, `_omnigent_compat.py:80-101`), verified by direct read. 18 = `_HARNESS_MODULES` minus the two alias keys, but it **drops `open-responses`** (validator-accepted, adapter-routed) so it understates the *accepted* set. 20 = raw `_HARNESS_MODULES` keys incl. aliases. 16/11 = stale spec numbers. typed-client's "`hermes` not in the canonical 16" is wrong — `hermes` is canonical in both registries. **Lens picker/validation should use the 19.**

**C3 — Is `/v1/info` "only capability flags"?**
**Ruling: directionally correct, slightly overstated.** It returns no version (so it must not be the gate), but it is not pure booleans — `login_url` and `sandbox_provider` are strings/nulls (`app.py:1557-1564`). Describe it as an *unauthenticated runtime capability/auth probe*. Version gate = `/api/version`.

**C4 — `ElicitationRequestParams.url`: external OAuth or relative approval page?**
**Ruling: relative same-origin today, generic in schema prose.** Current producer sets `url = f"/approve/{session_id}/{elicitation_id}"` (`runtime/policies/approval.py:209-228`); the schema comments use MCP's generic "external URL" language. Lens must validate scheme/origin and treat current omnigent-generated cards as relative `{base_url}/approve/…`, while still guarding for a future external URL. Prior "not external OAuth" findings are correct for current omnigent.

**C5 — Are `POST/DELETE /v1/hosts` real REST paths?**
**Ruling: no.** Only `GET /v1/hosts` + `GET /v1/hosts/{id}` exist; host registration is WS-tunnel/daemon based (`host_tunnel.py`). The capability-map reviewer correctly flagged this; the **workspace-and-server-lifecycle "positive grounding" line that lists `GET/POST/DELETE /v1/hosts` as present is wrong** and should be corrected (creation = `POST …/directories`/`…/runners`, not host CRUD).

**C6 — Which endpoint fires `session.agent_changed`: `PUT /agent` or `POST /switch-agent`?**
**Ruling: `POST /switch-agent`** (`sessions.py:14353`). `PUT /v1/sessions/{id}/agent` is bundle upload only (same-name constraint, idempotent on unchanged content). typed-client §3 line 90 (and any doc blaming `PUT /agent`) is wrong; app-architecture §12.2 has it right.

**C7 — `WS /v1/sessions/updates` (fleet push).**
**Ruling: real in source/tests, absent from openapi (correctly excluded from the 59).** Lens may stay poll-only for v1, but the decision (poll vs push for the menu-bar badge) should be explicit in typed-client + shell §17.4 + state-model §10, not silent.

**C8 — `sandbox_status` stages.**
**Ruling:** enum is `provisioning, cloning, starting, connecting, ready, failed` (`openapi.json` ~2941) — **no `queued`**. server-lifecycle §7 "queued → provisioning → ready → failed" is wrong.

---

## 5. Prioritized master findings (de-duplicated)

Severity: **Blocker** (contract/feasibility wrong — implementation would be built incorrectly), **Major** (grounding/consistency error with real downstream cost), **Minor** (clarity/completeness).

### Blockers

| # | Finding (theme) | Docs + sections to edit |
|---|---|---|
| B1 | Version gate on `/v1/info` instead of `/api/version` (T1) | typed-client §8/§1; server-lifecycle §8/§3.1; capability-map §0.3 gate row |
| B2 | Reconnect dedup keyed off non-existent `sequence_number`; chrome misclassified as item-replayable vs snapshot-restored (T9) | typed-client §7/§10; app-architecture §4.1; agent-definition §7 step 4 |
| B3 | switch-agent "owner-only verified in source" false — API floor = `LEVEL_EDIT`; idle guard misses `launching` (T3, J) | agent-definition §7; app-architecture §12.2; capability-map §0.7-J; permissions §7; typed-client §3 |
| B4 | Grant levels stated 1–4; server accepts 1–3 (T10) | permissions §7; typed-client §3; capability-map §0.3 |
| B5 | Elicitation `params.url` treated as external OAuth; it's a relative `/approve/…` page — no scheme/origin validation (T18/C4) | permissions §3/§4; (cross-ref framework §2.5) |
| B6 | "Checked-in openapi.json" + recon artifact claimed but absent; pin says 0.2.0, source is 0.3.0.dev0 (T6) | README; typed-client §11; capability-map §0.8; framework §1.3/§2 |
| B7 | Local bootstrap supervises server only; 0.3 requires host daemon for runner/workspace/terminal proxy + Concierge (T16) | server-lifecycle §3.1/§3.2/§6/§3-intro |
| B8 | Terminal WS path missing `/v1` prefix (T14) | workspace §9.1; typed-client §5 |
| B9 | `PresenceViewer` invented fields (`display_name/is_owner/last_seen_at`) (T12) | app-architecture §2.5/§12.1/§6.2 |
| B10 | State model `pending_elicitation: Option` cannot model fan-out; wire is `pending_elicitations[]` (T5) | app-architecture §2.2/§11; permissions §2; sub-agent-topology §6 |
| B11 | Capability-map §0.2 contradicts decision E (sharing UI in/out of scope) (E) | capability-map §0.2 vs §0.7-E |

### Majors

| # | Finding (theme) | Docs + sections |
|---|---|---|
| M1 | `session.status` 3-state poll vs 5-state SSE; Slept cards may regress to `idle` (T2) | shell §5.1/§17.4; capability-map §0.3/§0.6; app-architecture §2.2 |
| M2 | Harness count → 19 canonical; add `goose`/`hermes`; `harness` now free string; fix alias path (T4/C2) | agent-definition §4/intro/§2; typed-client §9; capability-map §0.3 |
| M3 | `SessionEventInput` discriminator set incomplete; `compact` missing (T13) | typed-client §6; app-architecture §7; capability-map §0.3/§0.4 |
| M4 | Native hooks: 4 not 2; parse `external_elicitation_resolved` to clear UI on TUI win (T11) | permissions §6; typed-client §3; capability-map §0.3 |
| M5 | `response.elicitation_resolved` carries no verdict — marker/badge logic + Bridge intake (T19) | permissions §2; transcript §18; app-architecture §11 |
| M6 | Bridge rail self-contradiction + board-only entry undefined; placement still "open" (H) | shell §6/§10.1; app-architecture §11; capability-map §0.7-H |
| M7 | Decision I not integrated into chrome; global rollup algorithm + retention absent (I, T20) | shell §7.4/§17.1/§21; app-architecture §6.2/§6.4(new) |
| M8 | Lens vs server `archived` dual model unreconciled (T8) | shell §4.6; app-architecture §2.2/§3.2/§6.2 |
| M9 | Lifecycle not persistable (no `slept`/`deleted`/`lifecycle`/`pinned` columns; tombstones) (T20) | app-architecture §2.2/§3/§6.2 |
| M10 | Wake/resume API sequence undefined (`POST /runners`→`PATCH runner_id`→reconnect) (T20) | app-architecture §3/§7; server-lifecycle §6 |
| M11 | Agent YAML example doesn't match omnigent spec shape (`executor.type`, `executor.config.harness`, terminals map, dir-based sub-agents) | agent-definition §2 |
| M12 | switch-agent flow conflates PUT bundle iteration with `agent_id`-only switch; `agent_changed` carries no model/skills | agent-definition §6/§7 |
| M13 | `ChildSessionSummary.agent_name`/`current_task_id` are `None` in 0.3.0; recursive rollup is direct-children only | sub-agent-topology §2/§3/§6; (label fallback to `tool`/`title`) |
| M14 | Filesystem verbs: PUT `{content}` write vs PATCH `{old_text,new_text}` edit (not one PATCH); env-scoped, paginated; managed workspace = repo URL not path | workspace §3/§10; (typed-client `GET` `refresh_state`) |
| M15 | Terminal transfer body/events/409; switch-agent resets terminals (resource deleted/created); `changed_files.invalidated` is env-scoped | workspace §9.3/§3 |
| M16 | Host model: no `POST/DELETE /v1/hosts`; wrong policy-evaluate path (`POST …/policies/evaluate`); `/health` batch liveness omitted (T15/C5) | capability-map §0.3; server-lifecycle/workspace positive-grounding |
| M17 | Tool-span status enum + `CompactionFailed` marker contradict source; persisted `error` item + `is_meta`/interrupted/multimodal render paths missing (T17) | transcript §6–§15; app-architecture §2.3 |
| M18 | Markdown/link/image sanitization not applied to user autolinks / ` ```md ` blocks (T18) | transcript §6 (+ new §6.3) |
| M19 | Variable-height transcript virtualization vs gpui `uniform_list`; markdown incremental diff — unproven spikes | transcript §16/§19; framework §4.1 |
| M20 | Auth model collapsed to one env var vs `header|oidc|accounts` provider matrix; 401+`login_url`; local spawn auth (E) | permissions §9; server-lifecycle §2/§4 |
| M21 | `recon`/"retired widget risk" overstated; no fallback ladder if gpui blocks markdown/forms (D) | framework §1.3/§4.1 (+ §4.3 escalation) |

### Minors (representative — see per-doc findings for the full list)

- Keyboard-shortcut drift (`⌘\`, `⌘F` vs `⌘⇧F`, working-area chords) shell §9.2 ↔ capability-map §0.6 (F).
- "Right rail" vocabulary persists in capability-map index + README (F).
- README framework row "gated" + capability-map index "gated" vs resolved gpui (D).
- `sandbox_status` stage enum (no `queued`) server-lifecycle §7 (C8).
- Graceful shutdown must stop daemon-then-server; pin actual spawn argv server-lifecycle §3.1/§3.2 (T16).
- Duplicate wave-derivation paragraph shell §5.1; §17.3 cross-ref typo ("§14.3"→§17.3).
- `WS /v1/sessions/updates` poll-vs-push decision explicit (C7).
- openapi `info.version "0.1.0"` is stale metadata — trust package semver/tests.
- `/health`, `GET /v1/runners`, `GET /v1/runners/{id}/status`, `POST …/mcp`, `GET …/agent/contents` either mapped with "Lens defers/N-A" rationale or parity claim narrowed (capability-map §0.3).

---

## 6. Ordered fix plan

Dependencies first; later phases assume earlier ones landed. Each phase is independently reviewable.

**Phase 0 — Verification foundation (unblocks trusting every other fix).** B6.
Vendor `openapi.json` to `vendor/omnigent-0.3.0.dev0/` + `OMNIGENT_PIN`/`VERSION`; add a CI path-enumeration + SSE-schema diff against the sibling pin; bump the stated pin from 0.2.0 to 0.3.0.dev0 across headers; check in or downgrade the recon artifact. *Without this, no contract claim is reproducible.*

**Phase 1 — Typed-client contract layer (the keystone; unblocks B1, B2, M1, M3, M4, T14, harness).** B1, B2, B8, M1, M2, M3, M4, C6, C7.
Fix the version gate (`/api/version`), the 3-vs-5 `session.status` split + poll-coarse mapping, the reconnect three-bucket model (snapshot-restored chrome; item-id dedup; drop `sequence_number` from item replay), the full `SessionEventInput` discriminator set + `compact`, the four native hooks, harness registry → 19 (+ free-string `harness`), terminal WS `/v1` path, grant levels 1–3, and the `PUT /agent` vs `switch-agent` `agent_changed` attribution. **Everything downstream consumes these types.**

**Phase 2 — State model (depends on Phase 1 types).** B3(guard reconcile), B9, B10, M5, M8, M9, M10, M17(ItemKind), T12, T20.
`pending_elicitations: Vec` + `target_session_id` on Elicitation/Bridge; wire-faithful `PresenceViewer` (RAM-only); persisted lifecycle columns + tombstones + `pinned`; wake/resume sub-flow; Lens-vs-server `archived` reconciliation; Bridge `elicitation_resolved` intake + count→0; global cost rollup algorithm + retention; `ItemKind::Error` + corrected `TerminalCommand`/`SlashCommand` fields; switch-agent guard truth (API=edit; UI=owner-policy; idle set incl. launching preflight).

**Phase 3 — Application shell (depends on Phases 1–2).** M1(card mapping), M6, M7, M8(table), F/H/I labels.
Card wave 3-vs-5 mapping + persist fine-grained status; resolve Bridge rail contradiction + board-only entry (decision table: destination × window-state); move decision I into header/health chrome with `None`-cost behavior; archive reconciliation table; reconcile keyboard table + "right rail"/"gated" vocabulary with capability-map.

**Phase 4 — Permissions, transcript, agent-definition, workspace, server-lifecycle (doc-local, depend on Phase 1 contract + Phase 2 model).**
- Permissions: B4(grant 1–3), B5(url-mode split + `validate_elicitation_url`), M4, M5, M20(auth matrix), §7 read-only/co-viewer UX.
- Transcript: M17(status/compaction/error/multimodal/is_meta), M18(markdown §6.3), M19(virtualization spike gate), T7 placement, `↯ cancelled`.
- Agent-definition: M2, M11(YAML shape), M12(switch flow), M13(child fields), B3 labels.
- Workspace: M14(fs verbs/managed/env), M15(transfer/switch reset), T14, C8.
- Server-lifecycle: B7(daemon), B1, M10, M16(hosts/policy-eval/health), M20, C8, shutdown order + argv.

**Phase 5 — Cross-cutting decision reconciliation + capability-map keystone (last, once downstream truths are settled).** B11(E §0.2), D/F/H/I label fixes, M16 host/policy-eval paths, harness recount, "Lens-native vs API parity" labeling, J grounding language. The capability-map is the keystone index, so it ratifies the resolved truths after the surface docs converge.

**Why this order:** B1/B2 (contract gate + reconnect) are the highest-leverage blockers — they're shared by 3 docs and silently wrong. The typed-client crate is the single source of types for REST/SSE/WS, so its corrections must land before the state model can be made consistent, which in turn must precede the shell and the per-surface docs. Verification (Phase 0) comes first because, until openapi is vendored and the pin is honest, none of the contract fixes can be checked or kept from re-drifting.

