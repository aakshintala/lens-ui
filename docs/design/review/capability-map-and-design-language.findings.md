# Review findings — `capability-map-and-design-language.md`

**Reviewed:** 2026-06-24  
**Ground truth:** `/Users/aakshintala/work/omnigent/openapi.json` (OpenAPI 3.2.0, **59 paths**, package **0.3.0.dev0**) + sibling omnigent source  
**Spec pin:** omnigent **v0.2.0** (capability map header, `docs/design/README.md`)

---

## TL;DR

- **~51/59 OpenAPI paths** are covered explicitly or by grouped rows; **8 paths are absent** from the keystone inventory, and **2 cited paths are wrong** (`POST/DELETE /v1/hosts`, policy evaluate URL).
- **§0.2 contradicts resolved decision E:** it still says Lens “does not implement the multi-user/sharing UI” while §0.7-E and `README.md` resolve full sharing/permissions in scope.
- **0.3.0 drift is unaddressed:** `session.status` includes **`waiting`**, two new elicitation hooks, **`/health`** batch liveness, **`hermes`/`goose`** harnesses, and a host-registration model that is **WS tunnel–based** (no REST `POST /v1/hosts`).
- **Pin-and-verify is not operable today:** README claims a “checked-in `openapi.json`”, but **no vendored copy exists** in the Lens repo (`vendor/omnigent-0.2.0/` absent); ground truth lives only in the sibling omnigent repo.
- **Event/`POST /events` inventory is incomplete** for native-terminal forwarding (`external_*` dispatch types, `stop_session`) and card-status semantics (`waiting`).

---

## OpenAPI coverage

| Category | Count |
|---|---|
| Total paths in `openapi.json` | **59** |
| Explicitly named or clearly grouped in §0.3 | **~47** |
| Implied by grouped capability (env list/get, terminal create/list, session PATCH/DELETE, permissions DELETE) | **~4** |
| **No keystone row** (see gaps below) | **8** |
| **Cited incorrectly** (path/method mismatch vs openapi) | **2** |

**Absent from §0.3 (no row, no grouped mention):**

1. `GET /api/version` — `openapi.json:3790`
2. `GET /health` — `openapi.json:3813` (batch `runner_online` / `host_online` liveness)
3. `GET /v1/runners` — `openapi.json:4620`
4. `GET /v1/runners/{runner_id}/status` — `openapi.json:4650`
5. `GET /v1/sessions/{session_id}/agent/contents` — `openapi.json:5152`
6. `POST /v1/sessions/{session_id}/mcp` — `openapi.json:6042`
7. `POST /v1/sessions/{session_id}/hooks/antigravity-elicitation-request` — `openapi.json:5739`
8. `POST /v1/sessions/{session_id}/hooks/cursor-permission-request` — `openapi.json:5821`

WS terminal attach (`/sessions/{id}/resources/terminals/{id}/attach`) is documented in §0.3 but **not in openapi** (expected — lives in `terminal_attach.py` per typed-client spec).

---

## Findings

### OpenAPI grounding, coverage, drift

**[CRITICAL] [DIMENSION 1+2]** §0.2 contradicts resolved decision E on sharing UI  
**Location:** `capability-map-and-design-language.md:117-124` vs `:506-526`; `docs/design/README.md:61`  
**Evidence:** §0.2: “Lens itself does not implement the multi-user/sharing *UI* (decision §0.7-E)”. §0.7-E **Resolved:** “Sharing/multi-user UI — in scope” with `PUT /permissions`, owner readout, `__public__`, policy editor. README decision table matches §0.7-E.  
**Recommendation:** Rewrite §0.2 auth posture to align with E: per-connection credentials for remote servers **and** first-class sharing/permissions UI on authed connections. Remove the stale “not a full identity/sharing surface” clause or qualify it (Lens is not an IdP; it **is** a sharing client).

---

**[CRITICAL] [DIMENSION 1]** Host registry cites non-existent REST `POST/DELETE /v1/hosts`  
**Location:** `capability-map-and-design-language.md:212`  
**Evidence:** Map claims `GET/POST/DELETE /v1/hosts`. Current openapi: `GET /v1/hosts` (`openapi.json:3967`), `GET /v1/hosts/{host_id}` (`openapi.json:3997`) only — no POST/DELETE on either path. Host registration is via outbound WS (`omnigent/server/routes/host_tunnel.py:13-15`: “registers the host in the HostRegistry”). `hosts.py` exposes only GET list, GET detail, POST runners/directories, GET filesystem (`omnigent/server/routes/hosts.py:318-836`).  
**Recommendation:** Replace “GET/POST/DELETE /v1/hosts” with the actual model: hosts appear via `omnigent host` WS tunnel + managed provisioning; REST surface is list/get/browse/launch-runner. Move host-tunnel WS to server-lifecycle + typed-client inventories.

---

**[HIGH] [DIMENSION 1]** Wrong policy evaluate path and method  
**Location:** `capability-map-and-design-language.md:194`  
**Evidence:** Map: `GET /v1/policies/{id}/evaluate`. Openapi: `POST /v1/sessions/{session_id}/policies/evaluate` (`openapi.json:6357`).  
**Recommendation:** Fix to `POST /v1/sessions/{session_id}/policies/evaluate` with `EvaluationRequest` body. Distinguish from server-wide `/v1/policies` CRUD.

---

**[HIGH] [DIMENSION 1]** Version pin frozen at v0.2.0 while ground truth is 0.3.0.dev0  
**Location:** Header `:8-9`, §0.3 title `:128`, §0.8 `:600`; `docs/design/README.md:3,9`  
**Evidence:** Map/README ground against v0.2.0. Sibling repo tests reference `0.3.0.dev0` (`omnigent/tests/runner/test_waiting_status_compat.py:25`). Openapi `info.version` is stale `"0.1.0"` (`openapi.json:3786`) — package semver ≠ openapi metadata.  
**Recommendation:** Bump pin to `0.3.0.dev0` (or next release tag), add a “contract delta since 0.2.0” subsection, and re-run the 2026-06-22 contract check against current openapi.

---

**[HIGH] [DIMENSION 1]** `session.status = "waiting"` (0.3.0) missing from card/lifecycle inventory  
**Location:** §0.3 Conversation (`:142`), §0.4 event families (`:251-255`)  
**Evidence:** `SessionStatusEvent.status` enum includes `"waiting"` (`openapi.json:3042-3047`); description ties it to parent agent blocked on async/sub-agent work. State model folds `session.status` (`app-architecture-and-state-model.md:398`) but keystone never lists `waiting` in status wave / card urgency ladder.  
**Recommendation:** Add `waiting` to §0.3 response/session lifecycle row and §0.6 status-lane vocabulary; specify card wave behavior distinct from `running` and `idle`.

---

**[HIGH] [DIMENSION 1]** Two elicitation hook paths omitted from permissions inventory  
**Location:** `capability-map-and-design-language.md:193`  
**Evidence:** Map lists only `hooks/permission-request` (claude-native) and `hooks/codex-elicitation-request`. Openapi also has `hooks/antigravity-elicitation-request` (`openapi.json:5739`) and `hooks/cursor-permission-request` (`openapi.json:5821`).  
**Recommendation:** Extend permissions row to “four native hooks” matching openapi; route all into shared elicitation UI (consistent with antigravity/cursor harnesses in agent-definition).

---

**[MEDIUM] [DIMENSION 1]** `/health` batch liveness omitted despite fleet-dashboard relevance  
**Location:** §0.3 Server/runner lifecycle (`:207-216`), §0.6 Fleet/status (`:331-334,383-396`)  
**Evidence:** `GET /health` supports optional `session_id` / comma-separated `session_ids` returning `runner_online` and `host_online` per session (`openapi.json:3813-3815`). Map relies on SSE + `GET /v1/sessions` poll only (`:238-242`).  
**Recommendation:** Add a ▮sm row under server lifecycle: batch liveness poll for slept/archived cards when SSE is closed; cite `GET /health?session_ids=…`.

---

**[MEDIUM] [DIMENSION 1]** Harness registry stale — count, paths, and new harnesses  
**Location:** `capability-map-and-design-language.md:180`  
**Evidence:** Map: “16 canonical” list ending at `copilot`. Current `omnigent/runtime/harnesses/__init__.py:34-124` registers **`goose`** (headless ACP) and **`hermes`** in addition to the 16. Citation path wrong: `harness_aliases.py` is at `omnigent/harness_aliases.py`, not under `runtime/harnesses/`. Openapi `AgentObject.harness` is now free-form `string | null` (`openapi.json:62-70`), not a closed enum — the “enum lags code” note is partially outdated.  
**Recommendation:** Re-count harnesses from `_HARNESS_MODULES`, fix file paths, note openapi harness is untyped string; add `goose` + `hermes` to picker matrix.

---

**[MEDIUM] [DIMENSION 1]** `POST /events` dispatch surface under-specified  
**Location:** §0.3 `:149`, §0.4 `:264-270`  
**Evidence:** Openapi documents many `body.type` values on `POST /v1/sessions/{session_id}/events` (`openapi.json:5690`): `stop_session`, `external_assistant_message`, `external_conversation_item`, `external_output_text_delta`, `external_output_reasoning_delta`, `external_session_interrupted`, `external_elicitation_resolved`, `external_session_status`, `external_model_change`, `external_reasoning_effort_change`, `external_codex_collaboration_mode_change`, etc. Map mentions interrupt/approval/compact/switch-agent only; §0.6 auto-sleep uses `stop_session` (`:393`) without listing it in §0.3.  
**Recommendation:** Add a “Session control & external observation events” row covering `stop_session` (auto-sleep/wake), native-terminal `external_*` types, and their emitted SSE (`session.model`, `session.interrupted`, …).

---

**[MEDIUM] [DIMENSION 1]** §0.4 `session.*` inventory incomplete vs openapi  
**Location:** `capability-map-and-design-language.md:251-255`  
**Evidence:** Openapi also defines `session.status`, `session.model`, `session.input.consumed`, `session.interrupted`, `session.resource` (generic) (`openapi.json:2078-3183` grep). Map lists 8 “new since v0.1.0” chrome events but omits `session.status` and `session.model` from the §0.4 bullet list (state model folds both).  
**Recommendation:** Expand §0.4 inventory to match openapi event union; cross-ref typed-client persisted/transient classification.

---

**[LOW] [DIMENSION 1]** `/api/version`, `GET /v1/runners`, `GET …/agent/contents`, `POST …/mcp` unmapped  
**Location:** §0.3 (no rows)  
**Evidence:** Paths exist at `openapi.json:3790`, `:4620`, `:5152`, `:6042`. Lens may intentionally skip MCP proxy and agent bundle download (runner concern), but the keystone claims “full 0.2.0 surface → Lens” parity.  
**Recommendation:** Either add explicit rows with cost (▮sm/▮md) and “Lens defers / N/A” rationale, or narrow the parity claim to “client-relevant surface” and enumerate exclusions.

---

**[LOW] [DIMENSION 1]** `ChildSessionSummary` still not a named openapi schema  
**Location:** `capability-map-and-design-language.md:203`  
**Evidence:** Caveat remains valid — type referenced in descriptions (`openapi.json:2130`, `:5196`) but absent from `components/schemas`.  
**Recommendation:** Keep hand-written mirror + contract-test list; no change to caveat wording.

---

### Internal consistency (decisions A–J, README)

**[MEDIUM] [DIMENSION 2]** Decision H omits Bridge rail placement resolved downstream  
**Location:** §0.7-H (`:543-563`); `application-shell-and-layout.md:82,889`  
**Evidence:** Shell resolves “Bridge = **left-rail** destination”; keystone says “one Bridge rail destination” without side. Not a conflict with README (README silent on side).  
**Recommendation:** Add “left rail” to §0.7-H so the keystone owns the placement decision shell already cites.

---

**[LOW] [DIMENSION 2]** README framework row still “gated” while D is resolved  
**Location:** `docs/design/README.md:27` vs capability map §0.7-D `:498-504`  
**Evidence:** Decision D resolved gpui; README table entry for `framework.md` still says “gated”.  
**Recommendation:** Update README framework row to “Resolved: gpui” (outside this doc’s ownership, but creates set-wide inconsistency).

---

**[INFO] [DIMENSION 2]** Decisions A–J otherwise align between keystone and README  
**Location:** §0.7 `:447-591`; `README.md:55-66`  
**Evidence:** Task=session, sub-agent tray hybrid, ring buffer, gpui, full permissions, collapsible area + ⌘D, multi-window, Bridge one rail, spend readouts, live switch-agent — all match.  
**Recommendation:** None beyond fixing §0.2/E contradiction above.

---

### Capability map completeness

**[MEDIUM] [DIMENSION 3]** Lens-only surfaces presented without omnigent grounding flags  
**Location:** §0.6 Bridge Knowledge/Log/Memories/Wiki (`:355-359`), Concierge (`:362-366`), Canvas (`:336-338,367-370`)  
**Evidence:** Map acknowledges Bridge Inbox routing is “Lens-side fabric” (`:351-354`, `:555-557`) but Log/Knowledge/Memories/Wiki and Concierge persistence have **no REST/SSE counterpart** in openapi.  
**Recommendation:** Tag these rows explicitly as **Lens-native** (out of openapi parity scope) or document the minimal omnigent hooks they consume (comments, labels, elicitations only).

---

**[MEDIUM] [DIMENSION 3]** Session lifecycle CRUD thinly covered for “full parity”  
**Location:** §0.3 (no dedicated session-admin row)  
**Evidence:** `PATCH /v1/sessions/{session_id}` (runner bind, archive, labels, model override, … — `openapi.json:4889+`), `DELETE /v1/sessions/{session_id}` (`?delete_branch=`), fleet list filters (`kind`, `include_archived`, `search_query` — `openapi.json:4704`) are spread across narrative but not inventoried as capabilities. Typed-client lists them (`typed-client.md:81-85`).  
**Recommendation:** Add a “Session admin & fleet poll” row: create/list/patch/delete/archive, cursor filters, runner bind via PATCH.

---

**[LOW] [DIMENSION 3]** `GET /v1/hosts/{host_id}` not named alongside host browse  
**Location:** `capability-map-and-design-language.md:212`  
**Evidence:** `openapi.json:3997` — single-host detail fetch.  
**Recommendation:** Mention alongside list + filesystem browse in server-lifecycle inventory.

---

### Clarity & design language

**[LOW] [DIMENSION 4]** “Full 0.2.0 surface → Lens” title vs deliberate Lens extensions  
**Location:** §0.3 heading `:128`, §0.6 `:426-435`  
**Evidence:** Section title promises omnigent surface mapping; §0.6 “Net-new” and Bridge/Concierge/Canvas are Lens inventions.  
**Recommendation:** Retitle §0.3 to “omnigent API surface → Lens capabilities” and add a short §0.3.1 “Lens-native extensions (not in openapi)” pointer to §0.6.

---

**[LOW] [DIMENSION 4]** Term retirement (“task”) is clear; collision note is helpful  
**Location:** §0.6 `:340-344`, §0.7-A `:458-464`  
**Evidence:** Consistent use of Group / turn / todos vs retired “Task” entity.  
**Recommendation:** None.

---

**[INFO] [DIMENSION 4]** Cost model (§0.7-I) clearly splits server vs Lens-computed axes  
**Location:** `:565-578`  
**Evidence:** Correctly notes `user_daily_cost` has no REST endpoint; Lens samples `total_cost_usd`.  
**Recommendation:** None.

---

### Verification posture (pin-and-verify)

**[CRITICAL] [DIMENSION 5]** “Checked-in openapi.json” claim is false in Lens repo  
**Location:** `docs/design/README.md:9`; `typed-client.md:410-411`; §0.8 `:600`  
**Evidence:** `Glob vendor/**` in Lens repo → **0 files**. Openapi exists only at sibling `../omnigent/openapi.json`. Typed-client specifies `vendor/omnigent-0.2.0/openapi.json` but directory absent.  
**Recommendation:** Vendor openapi into Lens (as typed-client already specifies), add CI diff against sibling pin, or rewrite README/§0.8 to “verify against omnigent submodule/path @ pin” with explicit relative path and pin file (`OMNIGENT_PIN` / `vendor/VERSION`).

---

**[HIGH] [DIMENSION 5]** Re-verify cadence unspecified for 0.3.0 deltas  
**Location:** §0.8 `:600-618`  
**Evidence:** Single 2026-06-22 check against 0.2.0; “re-verify on each lock-step release” with no automated gate. Current drift (waiting, hooks, hosts, harnesses) would fail a fresh diff.  
**Recommendation:** Block implementation on: (1) vendor current openapi, (2) path-enumeration test (59 paths ↔ typed-client method table), (3) SSE event schema diff script, (4) spike checklist from §0.8 run against **0.3.0.dev0**.

---

**[MEDIUM] [DIMENSION 5]** Contract gate cites `GET /v1/info` but not `/api/version`  
**Location:** §0.3 `:215`, server lifecycle row  
**Evidence:** Both exist (`/v1/info` `openapi.json:4339`, `/api/version` `openapi.json:3790`). ap-web uses `/api/version` for bug reports per openapi description.  
**Recommendation:** Document which endpoint Lens uses for semver gate; prefer `/api/version` for package semver if `/v1/info` returns build metadata only.

---

## Summary table (severity × dimension)

| Sev | Dim | Count |
|---|---|---|
| CRITICAL | 1, 2, 5 | 3 |
| HIGH | 1, 5 | 4 |
| MEDIUM | 1, 2, 3, 5 | 8 |
| LOW | 1, 2, 3, 4 | 6 |
| INFO | 2, 4 | 2 |

**Suggested fix order:** (1) §0.2/E contradiction, (2) vendor openapi + pin bump, (3) hosts + policy-eval path corrections, (4) 0.3.0 event/hook/health additions, (5) harness recount, (6) Lens-native vs API parity labeling.
