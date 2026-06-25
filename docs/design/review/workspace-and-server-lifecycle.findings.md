# Review: `workspace-and-terminals.md` + `server-lifecycle.md`

**Reviewed against:** omnigent @ `0.3.0.dev0` (`pyproject.toml` version, `openapi.json` 59 paths)  
**Spec baseline:** v0.2.0 (2026-06-23)

## TL;DR

- **Blocker (both docs + typed-client):** Terminal WS attach is documented as `/sessions/…/attach` (no `/v1`), but the route is mounted under `/v1` — clients must use `/v1/sessions/{id}/resources/terminals/{terminal_id}/attach` (`app.py:1635-1642`, `ap-web` tests).
- **Blocker (server-lifecycle):** Local bootstrap supervises only `omnigent server`; omnigent 0.3 requires a **host daemon** (`omnigent host --local` / `_daemon_entry.py`) to register the machine and spawn runner tunnels — workspace/terminal APIs proxy to the runner and fail without it (`cli.py:2385-2436`, `host/_daemon_entry.py:8-66`).
- **Blocker (server-lifecycle §8):** Contract gate assumes `GET /v1/info` returns a semver; 0.3 returns capability flags only — version is at `GET /api/version` (`app.py:1479-1564`).
- **Major (workspace §3):** Filesystem write/edit verbs drift — openapi uses **PUT** (`content`) for write and **PATCH** (`old_text`/`new_text`) for search-replace edit, not a single PATCH “write/edit” (`openapi.json` ~6930-7158).
- **Major (server-lifecycle scope):** Capability map assigns policies/permissions/**fork** to this doc, but the spec omits all three — fork is only in state-model/typed-client; permissions live in a sibling doc with no cross-ref here.

---

## workspace-and-terminals.md

### Blockers

- **[SEVERITY: blocker] [endpoint/WS grounding + drift]** Terminal WS path missing `/v1` prefix
  - Location: §9.1 (lines 206–207)
  - Evidence: Spec: `WS /sessions/{id}/resources/terminals/{terminal_id}/attach` — **no `/v1` prefix**, citing `terminal_attach.py:130`. Router defines a relative path at line 130, but `create_app` mounts it with `prefix="/v1"` (`omnigent/server/app.py:1635-1642`). Runner proxy also targets `/v1/sessions/…/attach` (`terminal_attach.py:173-175`). `ap-web` connects to `/v1/sessions/conv_abc/resources/terminals/terminal_bash_s1/attach` (`ap-web/src/components/blocks/TerminalView.test.tsx:63`).
  - Recommendation: Fix wire path to `WS /v1/sessions/{session_id}/resources/terminals/{terminal_id}/attach?read_only=…`. Update typed-client cross-ref (same bug). Cite `app.py:1641`, not bare route decorator line.

- **[SEVERITY: blocker] [cross-doc consistency]** Same `/v1`-less terminal path propagated in typed-client contract
  - Location: §9.1; depends on typed-client §5
  - Evidence: `typed-client.md` lines 121-122, 211-212 repeat “no `/v1` prefix”. Both docs cite `terminal_attach.py:130` without mount prefix.
  - Recommendation: Fix both docs together; contract-test reachability must hit the mounted path.

---

### Major — grounding / drift

- **[SEVERITY: major] [endpoint grounding]** Filesystem HTTP verbs and body shapes wrong
  - Location: §3 table (lines 83–88)
  - Evidence: Spec lists `PATCH …/filesystem/{relative_path}` for “write/edit (old/new or batch)”. Openapi: **PUT** `…/filesystem/{relative_path}` with JSON `{content}` for write/replace; **PATCH** same path with `{old_text, new_text}` for search-replace edit; **DELETE** for delete (`openapi.json` ~6930-7158). No batch-edit path in openapi.
  - Recommendation: Split table into GET (read/list + pagination cursors), PUT (write), PATCH (old_text/new_text edit), DELETE. Drop “batch” unless a 0.3 route exists.

- **[SEVERITY: major] [endpoint grounding]** Managed-session workspace semantics misstated
  - Location: §10 (lines 286–289)
  - Evidence: Spec: for `managed`, “the server provisions the sandbox host + the workspace.” `SessionCreateRequest` for `host_type="managed"`: `host_id` and path-style `workspace` **must not** be set; optional workspace is a **repo URL** (`https://…#branch`) or null for empty sandbox workspace (`schemas.py:1069-1099`, tests in `test_schemas.py:test_session_create_managed_rejects_path_workspace`).
  - Recommendation: Distinguish external (path + optional `host_id`) from managed (repo URL or empty; server assigns path after clone). New-session UX must not offer host filesystem picker for managed.

- **[SEVERITY: major] [endpoint grounding]** `switch-agent` path abbreviated
  - Location: §9.3 (line 257)
  - Evidence: Spec: `POST /switch-agent`. Wire path is `POST /v1/sessions/{session_id}/switch-agent` (`openapi.json` ~7997, `sessions.py:14371` background `_reset_runner_resources_after_switch`).
  - Recommendation: Use full path; note post-switch `session.agent_changed` + `session.changed_files.invalidated` + terminal resource reset (terminals drop per §9.3 — grounded).

- **[SEVERITY: major] [completeness]** Terminal transfer request shape and failure modes undocumented
  - Location: §9.3 (lines 246–250), §12 open questions (lines 323–325)
  - Evidence: `POST /v1/sessions/{session_id}/resources/terminals/{terminal_id}/transfer` body: `{"target_session_id": "conv_new"}` (`openapi.json` ~7841, `sessions.py:15862-15868`). Server returns 409 on conflict (`sessions.py:15884-15888`). On success publishes `session.resource.deleted` on source and `session.resource.created` on target (`sessions.py:15898-15908`).
  - Recommendation: Document body + events. Pin UX: close/rebind WS to new `session_id`; handle 409 (concurrent transfer / stale tab). Ring buffer is keyed by `(connection, session_id, terminal_id)` — transfer changes session_id under same terminal resource id.

- **[SEVERITY: major] [completeness]** `session.changed_files.invalidated` is environment-scoped
  - Location: §3 file watch (lines 100–102)
  - Evidence: Event carries `environment_id` (default `"default"`) (`openapi.json` `SessionChangedFilesInvalidatedEvent` ~2092-2099). Review tab and file tree must refetch the correct env, not assume `"default"` when terminal-scoped envs exist (§2).
  - Recommendation: Invalidate per `(session_id, environment_id)`; terminal-scoped env changes should not blindly invalidate `"default"` Review data.

- **[SEVERITY: major] [completeness]** Environment inventory endpoint omitted
  - Location: §2 environment model
  - Evidence: `GET /v1/sessions/{session_id}/resources/environments` and `GET …/environments/{environment_id}` exist (`openapi.json` ~6625-6715). Spec assumes `"default"` implicitly but never documents discovery of terminal-scoped env ids from the server.
  - Recommendation: File tree + Review tab should list envs from REST (or `session.resource.created` events) rather than hard-code `"default"`.

- **[SEVERITY: major] [version drift]** Spec baseline 0.2.0 vs omnigent 0.3.0.dev0
  - Location: header (line 9)
  - Evidence: `pyproject.toml` version `0.3.0.dev0`. Terminal transfer, `session.terminal_pending`, managed `host_type`, env-scoped workspace tree, and permission-gated terminal attach are present in 0.3 openapi — align pin before implementation.
  - Recommendation: Bump contract pin to 0.3.x when Lens vendors openapi; re-verify harness/event drift (see typed-client review).

---

### Major — cross-doc / completeness

- **[SEVERITY: major] [cross-doc consistency]** Terminal reconnect (decision C) vs server crash recovery
  - Location: §9.2 vs server-lifecycle §9.1
  - Evidence: Workspace pins Lens ring buffer for brief WS blips (decision C, locked). Server-lifecycle §9.1: local server death loses in-flight SSE (no replay). Terminal WS also drops on server restart — ring buffer helps only if Lens process survives; server restart kills runner PTY regardless.
  - Recommendation: State explicitly: ring buffer covers **client-side WS reconnect to a live server**; server restart requires full terminal re-attach (likely new terminal resources after runner relaunch).

- **[SEVERITY: major] [completeness]** Switch-agent terminal reset — resource events not listed
  - Location: §9.3 (lines 256–261)
  - Evidence: `_reset_runner_resources_after_switch` clears runner terminals; tests expect `session.changed_files.invalidated` (`test_sessions_switch_agent.py`). Terminals should surface `session.resource.deleted` / recreation, not only a cosmetic `↻ re-attaching` state.
  - Recommendation: Subscribe to resource deleted/created during switch-agent; reconcile terminal tabs against `GET …/resources/terminals`.

- **[SEVERITY: major] [feasibility]** WS wire protocol details omitted for GPUI terminal widget
  - Location: §9.1
  - Evidence: Server → client: **binary** PTY bytes; client → server: **binary** input, **text JSON** `{"type":"resize","cols":N,"rows":M}` (`terminal_attach.py:35-47`). `read_only=true` query drops input (`terminal_attach.py:52-56`, attach route line 135).
  - Recommendation: Add wire-protocol subsection for the GPUI terminal implementation (framework doc alacritty path); document `read_only` query for non-owner attach.

---

### Minor — clarity / structure

- **[SEVERITY: minor] [clarity]** Filesystem listing pagination not mentioned
  - Location: §3
  - Evidence: Root and nested listings support `limit` (default 20, max 1000), `after`/`before` cursors, `order` (`openapi.json` ~6825-6990).
  - Recommendation: Note paginated tree provider; large repos need cursor fetch, not single GET.

- **[SEVERITY: minor] [clarity]** Search cap grounded but parameter names absent
  - Location: §5 (line 129)
  - Evidence: `limit` default 500, max 500 (`sessions.py:16341-16363`) — matches spec cap.
  - Recommendation: Optional: cite request body fields from openapi search schema for typed-client parity.

- **[SEVERITY: minor] [cross-doc consistency]** `POST /switch-agent` vs capability-map J
  - Location: §9.3
  - Evidence: Capability map §0.7-J and typed-client document full switch-agent contract; workspace doc is consistent on behavior (terminals drop) — only path abbreviation is wrong (above).

---

### Minor — feasibility

- **[SEVERITY: minor] [feasibility]** Ring buffer + binary WS on GPUI is feasible but thread-bridge required
  - Location: §9.2
  - Evidence: Same pattern as SSE in typed-client/framework (blocking WS reader → channel → UI poller). Terminal attach is long-lived; must not block GPUI thread.
  - Recommendation: Cross-ref framework §terminal; cap buffer on bytes not “10 MB” alone — ANSI-heavy output differs from raw size.

---

## server-lifecycle.md

### Blockers

- **[SEVERITY: blocker] [CLI grounding + feasibility]** Local bootstrap must supervise host daemon, not server alone
  - Location: §3.1 spawn command (lines 111–114), §6 “External + local” (lines 234–237)
  - Evidence: Spec: spawn `uv run omnigent server start` (or equivalent) and claims “local server embeds its own runner, no launch needed.” Omnigent 0.3 local stack: `_ensure_backend` always calls `_ensure_host_daemon` first; daemon in `--local` mode runs `ensure_local_omnigent_server()` **and** `run_host_process` to register host + spawn runners on demand (`cli.py:2385-2436`, `host/_daemon_entry.py:56-66`). Workspace/terminal/fs routes proxy to runner via tunnel (`sessions.py` `_proxy_*_to_runner`). `server start` alone does **not** start the host daemon (`cli.py:3220-3245` only calls `ensure_local_omnigent_server`).
  - Recommendation: Lens local bootstrap must supervise **both** (a) background `omnigent server` and (b) local host daemon — mirror `omnigent stop` teardown order (daemon first, then server — `cli.py:3254-3268`). Reconcile with capability-map “embedded runner” wording: runner is **daemon-spawned**, not in-process in the server.

- **[SEVERITY: blocker] [endpoint grounding + cross-doc]** Contract gate uses wrong endpoint for version
  - Location: §8 (lines 262–277), §3.1 ready detection (line 115)
  - Evidence: Spec: poll `GET /v1/info` and compare semver to `PINNED_OMNIGENT_VERSION` (0.2.0). `GET /v1/info` returns `accounts_enabled`, `login_url`, `needs_setup`, `databricks_features`, `managed_sandboxes_enabled`, `sandbox_provider` — **no version** (`app.py:1493-1564`). Version: `GET /api/version` → `{"version": "…"}` (`app.py:1479-1491`, `openapi.json` `/api/version` ~3790). Typed-client §8 repeats the same mistake.
  - Recommendation: Gate on `/api/version`; use `/v1/info` for capability probes (`managed_sandboxes_enabled`, `accounts_enabled`, `needs_setup` for first-run admin).

- **[SEVERITY: blocker] [CLI grounding]** Actual child spawn command differs from spec
  - Location: §3.1 (lines 111–114)
  - Evidence: Spec: `uv run omnigent server start`. `ensure_local_omnigent_server` spawns `sys.executable -m omnigent.cli server --host 127.0.0.1 --port {port} --database-uri … --artifact-location …` (`local_server.py:645-659`) — foreground **`server`**, not `server start`. `server start` is a CLI subcommand that *calls* `ensure_local_omnigent_server` (`cli.py:3231-3236`). Lens hermetic `uv` env is fine, but must pass explicit `--database-uri` / `--artifact-location` under Lens app-support paths (not default `~/.omnigent` unless intentional).
  - Recommendation: Document subprocess argv explicitly; decide whether Lens uses `~/.omnigent` or isolated `OMNIGENT_DATA_DIR` under app support. On quit, call `server stop` / `stop_local_omnigent_server` + terminate host daemon (matches omnigent `omnigent stop`).

---

### Major — grounding / drift

- **[SEVERITY: major] [version drift]** Pin `omnigent==0.2.0` vs repository 0.3.0.dev0
  - Location: §3.1 (line 107), §8 (lines 268-273)
  - Evidence: Spec pins `0.2.0`; omnigent `pyproject.toml` is `0.3.0.dev0`. New in 0.3: expanded sandbox providers (openshell, boxlite, e2b, cwsandbox in `managed_hosts.py` module doc), accounts/OIDC flows on `/v1/info`, session policies routes, additional hook paths, permission-gated terminal attach.
  - Recommendation: Plan explicit 0.3 vendor + contract-test pass before Lens ships; gate message should reference actual pin.

- **[SEVERITY: major] [endpoint grounding]** `sandbox_status` stage pipeline incomplete
  - Location: §7 (lines 249-250)
  - Evidence: Spec: “queued → provisioning → ready → failed.” Openapi `SessionSandboxStatusEvent.stage` enum: `provisioning`, `cloning`, `starting`, `connecting`, `ready`, `failed` — **no `queued`** (`openapi.json` ~2941-2948). Snapshot field `sandbox_status` seeds UI on reconnect (event description ~2911).
  - Recommendation: Match stage enum; surface intermediate stages on card badge. Cancel remains `DELETE /v1/sessions/{id}` (grounded).

- **[SEVERITY: major] [endpoint grounding]** `/v1/info` capability-driven managed sandbox UX
  - Location: §7 (lines 246-252), §10 managed row (line 332)
  - Evidence: Managed option must be hidden when `managed_sandboxes_enabled: false` on `/v1/info` (`app.py:1547-1556`, `managed_hosts.py:347`). Provider label from `sandbox_provider` (e.g. `"modal"`, `"islo"`).
  - Recommendation: First-run / new-session flows probe `/v1/info` before offering managed; don’t assume Modal/Daytona/Islo trinity from README alone (0.3 adds providers in `managed_hosts.py` header comment).

- **[SEVERITY: major] [CLI grounding]** Graceful shutdown should include host daemon
  - Location: §3.2 (lines 136-141)
  - Evidence: Spec: ⌘Q sends SIGTERM to server (or `omnigent server stop`). Omnigent `server stop` stops **local host daemon first**, then background server (`cli.py:3254-3268`, `test_server_lifecycle.py:test_server_stop_stops_server_and_local_daemon`).
  - Recommendation: Lens quit path must terminate host daemon + server; leaving daemon orphan causes zombie runners/listeners.

- **[SEVERITY: major] [CLI grounding]** Health probe endpoints incomplete
  - Location: §3.1 ready detection (line 115), §3.2 heartbeat (line 127)
  - Evidence: `local_server_status` / spawn readiness use `/health` (`cli.py:3277-3278` mentions `/health`). Contract gate in spec uses `/v1/info` only.
  - Recommendation: Ready detection: `/health` (liveness) then `/api/version` (contract) then `/v1/info` (capabilities). Document both probes.

---

### Major — completeness / scope gaps

- **[SEVERITY: major] [completeness vs capability map]** Policies, permissions, fork topology missing
  - Location: Doc scope (§1) vs `capability-map-and-design-language.md` row for `server-lifecycle.md`
  - Evidence: Capability map assigns “hosts/**policies/permissions/fork** topology” to this doc. Spec covers hosts + managed sandbox + runners but **no** `GET/POST /v1/policies`, `/v1/sessions/{id}/permissions`, or `POST /v1/sessions/{source_id}/fork` (`openapi.json` ~4389+, ~6124+, ~8048+). Permissions doc owns UI; fork lives in state-model §12 — server-lifecycle never links runner relaunch on fork (`LaunchRunnerRequest` after fork — ap-web `ForkSessionDialog.tsx` pattern).
  - Recommendation: Add §11 “Fork + shared-session topology” (fork creates session; Lens/`POST /runners` binds runner on chosen host) or narrow capability-map ownership. Cross-ref permissions doc for remote connection auth vs session sharing.

- **[SEVERITY: major] [completeness]** Local spawned auth model underspecified vs omnigent defaults
  - Location: §2 (line 81), §4 table (lines 164-166)
  - Evidence: Spec: local spawned connection uses `Auth::None`. `ensure_local_omnigent_server` comment: “server runs accounts mode (the default)” (`local_server.py:441-442`) when `OMNIGENT_AUTH_ENABLED=1`. Default auth source is **`header`** (`auth.py:7-9`) unless accounts/OIDC env set. `/v1/info.needs_setup` drives first-run admin (`app.py:1529-1531`).
  - Recommendation: Pin Lens local spawn env explicitly: either force header/single-user (`OMNIGENT_AUTH_ENABLED=0`) for true `Auth::None`, or implement accounts first-run (admin setup URL) and cookie auth for local connection. Align with decision E (remote = token/cookie).

- **[SEVERITY: major] [completeness]** Crash recovery — zombie/orphan server PIDs not covered
  - Location: §9.1 (lines 287-301)
  - Evidence: `stop_local_omnigent_server` handles orphan listeners on canonical port (`cli.py:3213-3216`, `local_server.py:509-513` port-contention respawn). Auto-restart “once” may collide with pidfile from foreign owner if Lens respawns naïvely.
  - Recommendation: Reuse omnigent’s pidfile + `server_config_signature` drift detection (`local_server.py:83-110`, `457-468`) rather than reinventing; on repeated failure surface log path from sidecar (`local_server.logpath`).

- **[SEVERITY: major] [completeness]** Managed-sandbox failure / reconnect underspecified
  - Location: §9.2 (lines 313-318)
  - Evidence: Managed host identity is **durable** across sandbox relaunch (`managed_hosts.py:11-15` — same `host_id`, new sandbox generation). Session may survive provider cap; `session.sandbox_status` stage `failed` carries `error` (`openapi.json` ~2917-2927).
  - Recommendation: Distinguish (a) network drop to server vs (b) sandbox provider failure vs (c) server-initiated relaunch. Poll snapshot `sandbox_status` + listen for `session.sandbox_status` after reconnect. Offer retry/create-new-session on terminal `failed`.

- **[SEVERITY: major] [completeness]** Remote runner launch race (host offline 409)
  - Location: §6 (lines 228-237)
  - Evidence: `POST /v1/hosts/{id}/runners` returns 409 when host offline (`openapi.json` ~4286). Daemon launch retries transient 409s ~16.5s (`daemon_launch.py:209-225`).
  - Recommendation: Lens new-session flow should retry launch while host tunnel registers; surface “host offline” from host liveness, not generic error.

---

### Major — cross-doc consistency

- **[SEVERITY: major] [cross-doc consistency]** Concierge requires local server **and** runnable sessions
  - Location: §3 intro (lines 94-101), §10 local row (line 330)
  - Evidence: Concierge lives on local server (state-model §12.3). Concierge session still needs a runner for tool execution — host daemon gap blocks Concierge if only server is spawned (see blocker above).
  - Recommendation: First-run wizard “always bootstrap local server” should read “local server + host daemon”.

- **[SEVERITY: major] [cross-doc consistency]** `LaunchRunnerRequest` shape grounded; PATCH bind omitted
  - Location: §6 (lines 218-226)
  - Evidence: Openapi `LaunchRunnerRequest`: `session_id`, `workspace`, optional `git` (`openapi.json` ~1027-1054) — matches spec. Atomic bind is server-side on launch; alternate bind via `PATCH /v1/sessions/{id} {runner_id}` (`openapi.json` ~3544) used by CLI fork/resume — not mentioned.
  - Recommendation: Note PATCH runner bind for fork/resume flows (shell §7.6 / fork dialog).

- **[SEVERITY: major] [cross-doc consistency]** Bearer auth exists for CLI
  - Location: §4 (lines 167-168)
  - Evidence: `UnifiedAuthProvider` accepts `Authorization: Bearer <jwt>` for CLI clients (`auth.py:352-382`). Spec lists Bearer for “internal dev workspace” — grounded.
  - Recommendation: Add note: accounts-mode local server may require session cookie **or** Bearer after login, not literally `Auth::None`, unless header mode.

---

### Minor — clarity / feasibility

- **[SEVERITY: minor] [clarity]** Host filesystem browse auth scope
  - Location: §5 (lines 201-205)
  - Evidence: `GET /v1/hosts/{id}/filesystem` is owner-scoped, exposes full host FS, not session-scoped (`openapi.json` ~4097-4186) — matches spec intent.
  - Recommendation: None beyond noting tilde paths are expanded server-side on host browse but **rejected** on session workspace create (`schemas.py:1089-1090`).

- **[SEVERITY: minor] [feasibility]** macOS subprocess supervision feasible with caveats
  - Location: §3.2
  - Evidence: Omnigent uses `start_new_session=True` detached spawn (`local_server.py:663`), SIGTERM escalation pattern in `stop_local_omnigent_server`. Lens menu-bar resident app matches spec ⌘W vs ⌘Q split.
  - Recommendation: Use `process_group` kill on macOS for daemon+server tree; capture logs from `LocalServerStartup.log_path`.

- **[SEVERITY: minor] [endpoint grounding]** `GET /v1/hosts` list shape
  - Location: §5 table (line 195)
  - Evidence: Path exists (`openapi.json` ~3967); host registration also via `omnigent host` out-of-band — spec grounded.
  - Recommendation: Document managed hosts appear in same registry with managed columns (not user-registered).

- **[SEVERITY: minor] [structure]** Open questions appropriately flagged
  - Location: §11
  - Evidence: Log rotation, SSH host install, provider picker — reasonable deferrals.
  - Recommendation: Add open question: isolated `OMNIGENT_DATA_DIR` vs shared `~/.omnigent` when Lens coexists with CLI omnigent on same machine.

---

### Positive grounding (no action required)

- Host registry paths `GET/POST/DELETE /v1/hosts`, `POST …/directories`, `GET …/filesystem`, `POST …/runners` — present in openapi 0.3.
- `host_type: "external" | "managed"` on session create — present (`schemas.py:1148`).
- `session.sandbox_status` event — present with snapshot fallback.
- Remote auth envelope shapes (Cookie, ForwardedEmail, Bearer) — align with `auth.py` UnifiedAuthProvider.
- Auto-restart once + session persistence via server DB — consistent with omnigent session store model.
