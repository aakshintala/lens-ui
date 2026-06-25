# Review: `permissions-and-elicitations.md` + `framework.md`

**Reviewed against:** omnigent @ `0.3.0.dev0` (`openapi.json` info.version `"0.1.0"`, 59 paths; `omnigent/server/auth.py`, `schemas.py`, `routes/sessions.py`, `runtime/policies/approval.py`)  
**Spec baseline:** v0.2.0 (2026-06-23)

---

## TL;DR

- **URL-mode elicitation is mis-specified:** server `params.url` is the in-server `/approve/…` approval page (or `/resolve` path), not an external OAuth URL — Lens must not blindly `open(url)` without scheme/host validation.
- **Grant API levels are wrong in §7:** `PUT /permissions` accepts **1–3 only** (read/edit/manage); owner (4) is implicit, not grantable — contradicts “level 1=read..4=owner”.
- **Singular `pending_elicitation` cannot model fan-out:** snapshot exposes `pending_elicitations[]` (multiple mirrored child prompts); state model + permissions lifecycle assume at most one.
- **0.3.0 hook drift:** spec lists Claude + Codex hooks only; openapi adds `antigravity-elicitation-request` and `cursor-permission-request`; `external_elicitation_resolved` event path is undocumented.
- **Framework “recon retired risk” is unsubstantiated:** README/capability map cite a load-bearing recon artifact that is **not in the repo**; markdown JSON-Schema form widgets remain un-spiked while decision D is locked.

---

## permissions-and-elicitations.md

### Blockers

- **[BLOCKER] [1-grounding + drift]** URL-mode semantics conflate OAuth with server approval-page URL  
  **Location:** §3 (URL widget), §4 (url-mode OAuth callback)  
  **Evidence:** Spec: “Authorize ↗” opens `url` in the OS browser for “OAuth out-of-band”; `/resolve` is “preferred for url-mode OAuth.” Server builds url-mode as `url = f"/approve/{session_id}/{elicitation_id}"` — a **relative** standalone approval page on the omnigent host (`omnigent/runtime/policies/approval.py:209-228`). Openapi `/resolve` description: url-mode carries “this endpoint's path as its `params.url`” (`openapi.json` ~5628-5630), not an arbitrary external OAuth redirect.  
  **Recommendation:** Split **server approval-page url-mode** (relative `/approve/…` → Lens renders inline or opens `{base_url}/approve/…` with auth) from **true external OAuth** (if ever present in `params.url`). Add `validate_elicitation_url` (allow only `https:` + same-origin relative paths; block `javascript:`, `file:`, `data:`). Never pass unvalidated URLs to the OS opener. Document redirect handling: after browser flow, deep-link or poll `GET …/elicitations/{id}` until `status != pending`.

- **[BLOCKER] [1-grounding + drift]** Sharing grant levels misstate the API  
  **Location:** §7 table (`PUT /permissions`)  
  **Evidence:** Spec: `{user_id, level 1=read..4=owner}`. Openapi `GrantPermissionRequest`: “`1` = read, `2` = edit, `3` = manage”, `maximum: 3.0` (`openapi.json` ~866-874). `schemas.py:GrantPermissionRequest`: `level: int = Field(ge=1, le=3)`. `LEVEL_OWNER = 4` exists in `auth.py:76-79` but is assigned to session creator/admin — not grantable; owner grants return 403 (“Cannot modify owner permissions”, `sessions.py:18097-18100`).  
  **Recommendation:** Document grantable **1=read, 2=edit, 3=manage**; owner (4) via creation only. Align shell gates: Share link ≥3 (manage) is correct; fix table prose and typed-client cross-ref.

### Major

- **[MAJOR] [2-cross-doc consistency]** Singular pending elicitation vs multi-prompt snapshot  
  **Location:** §2 lifecycle; depends on state model `pending_elicitation: Option<Elicitation>`  
  **Evidence:** Permissions doc folds one `response.elicitation_request` into `SessionState.pending_elicitation`. Server snapshot returns **`pending_elicitations: list[dict]`** (`schemas.py:1529-1630`); integration test `test_two_children_elicitations_isolated_on_parent_stream` asserts parent snapshot lists **both** child prompts keyed by `target_session_id` (`test_sessions_elicitation_resolve_url.py:1171-1180`). Bridge indexes `(connection_id, session_id, elicitation_id)` but not multi-queue semantics (`app-architecture-and-state-model.md:801-802`).  
  **Recommendation:** Model `pending_elicitations: Vec<Elicitation>` (or map by `elicitation_id` / `target_session_id`). Composer docks **one focused** prompt; Bridge/card badge uses count. Reconcile wake/reconnect from snapshot `pending_elicitations[]`.

- **[MAJOR] [1-grounding + drift]** `response.elicitation_resolved` carries no verdict — transcript marker logic is incomplete  
  **Location:** §2 (“✓ approved / ✗ denied / ↯ cancelled”)  
  **Evidence:** `ElicitationResolvedEvent` has only `type` + `elicitation_id` (`openapi.json` ~676-705); openapi explicitly: “no UI approval verdict was delivered.” Approvals do **not** persist as conversation items (`sessions.py:16868`, ap-web `sessionsApi.ts:47`).  
  **Recommendation:** On local submit, record verdict in Lens state **before** await resolve/approval; on `elicitation_resolved` without prior local verdict, marker = `↯ cancelled` (timeout/turn-end/TUI answer). Do not infer approve/deny from resolved alone. Persist verdict in SQLite for reconnect.

- **[MAJOR] [1-grounding + drift]** Native permission hooks incomplete for 0.3.0 harness set  
  **Location:** §6  
  **Evidence:** Spec lists `permission-request` + `codex-elicitation-request` only. Openapi adds `POST …/hooks/antigravity-elicitation-request` (~5739) and `POST …/hooks/cursor-permission-request` (~5821). Cursor hook documents TUI-side answer → `external_elicitation_resolved` race (`test_cursor_native_permissions.py`, `sessions.py:348`).  
  **Recommendation:** Extend §6: “all harness hooks converge on same elicitation UI”; typed client must parse `external_elicitation_resolved` (via `POST /events`) to clear UI when native TUI wins. Reference capability-map harness list (incl. cursor-native, antigravity-native).

- **[MAJOR] [3-completeness]** URL / resolve security and error paths underspecified  
  **Location:** §3-§5 (missing)  
  **Evidence:** `/resolve` gated at `LEVEL_EDIT` (`sessions.py:16682-16684`); cross-user → 403/404 (`test_resolve_url_cross_user_forbidden`). Cross-session POST returns 202 but does not resolve wrong session's Future (`test_resolve_url_cross_session_does_not_resolve`). Double-submit is idempotent 202 (`test_post_resolve_already_resolved_is_idempotent`). Spec has no 403 UX, stale-widget handling, or “another co-viewer already approved” messaging.  
  **Recommendation:** Add §4.1: on 403 → disable widget + “read-only / not owner”; on 202 after local clear → treat as success; on resolved race → poll `GET …/elicitations/{id}`; disable actions while in-flight; handle concurrent co-viewer (presence §9) with last-writer-wins + toast.

- **[MAJOR] [3-completeness]** Elicitation timeout / cancel / expiry behavior not specified  
  **Location:** §2 (one line), §10 (no entry)  
  **Evidence:** Openapi: server synthesizes `cancel` on timeout (`ElicitationResult` ~709). Runner emits `response.elicitation_resolved` on timeout/cancel/harness exit (`openapi.json` ~677). Codex/permission hooks have re-park grace + deferred clear (`test_sessions_permission_request_hook.py`). Spec mentions timeout/cancel/turn-end but not UI timing, hook empty-200 behavior, or badge decrement on grace.  
  **Recommendation:** Document: widget auto-dismiss on `elicitation_resolved`; optional countdown if server exposes TTL (today: none on wire — say so); codex hook may return empty 200 → treat as `↯ cancelled`; sync Bridge badge on resolved idempotently.

- **[MAJOR] [1-grounding + drift]** Wire field naming: `requestedSchema` vs Rust `requested_schema`; missing `policy_names`  
  **Location:** §2 struct  
  **Evidence:** Openapi MCP camelCase `requestedSchema` (`openapi.json` ~632-643). Server adds `policy_names` when len>1 (`approval.py:225-226`). Spec Rust struct uses snake_case only; no `policy_names`.  
  **Recommendation:** Typed client deserializes camelCase; document `policy_names` in renderer (multi-policy ASK). Note in §2 that wire is MCP-shaped.

- **[MAJOR] [3-completeness]** Policy editor endpoint table incomplete  
  **Location:** §8  
  **Evidence:** Spec: session policies `GET/POST` + `GET/DELETE` one. Openapi also has `PATCH /v1/sessions/{id}/policies/{policy_id}` (~6504), `GET` single policy (~6452). Session list includes spec-declared policies (`source=spec`, non-deletable) per openapi ~6261.  
  **Recommendation:** Add PATCH + GET-one; note spec-bound policies are read-only in UI. Clarify evaluate body is proto-compatible `EvaluationRequest` (~6357-6359), not hypothetical free text.

- **[MAJOR] [3-completeness]** Identity / auth model drift from 0.3.0  
  **Location:** §9 (`OMNIGENT_AUTH_ENABLED=1`)  
  **Evidence:** Spec collapses auth to `OMNIGENT_AUTH_ENABLED`. Current auth: `OMNIGENT_AUTH_PROVIDER` = `header` | `oidc` | **`accounts`** (default OSS CUJ v2, `auth.py:15-20`); `GET /v1/me` returns `login_url` on 401 in OIDC (`openapi.json` ~4372-4374); `GET /v1/info` exposes `accounts_enabled`, `needs_setup` (typed-client review). Permissions disabled → `PUT /permissions` → “Permissions not enabled” (`sessions.py:18081-18085`).  
  **Recommendation:** Replace env-var shorthand with provider matrix; document first-run `needs_setup` / invite-only accounts; Lens remote connect must handle 401 + `login_url` (or paste cookie/token per server-lifecycle §4). Edge case: `user_id: null` in header mode unauthenticated.

- **[MAJOR] [2-cross-doc consistency]** Bridge resolve routing vs index key  
  **Location:** §5; state model Bridge §11  
  **Evidence:** §5 correctly routes resolve to `{target_session_id}`. State model Bridge indexes `(connection_id, session_id, elicitation_id)` where `session_id` is the **stream** session (parent when mirrored) — does not store `target_session_id` on Bridge item (`app-architecture-and-state-model.md:800-802`). Sub-agent topology §6 aligns with permissions §5.  
  **Recommendation:** Add `target_session_id: Option<SessionId>` to Bridge elicitation items and `Elicitation` struct; resolve helper always uses target or falls back to stream session.

### Medium

- **[MEDIUM] [3-completeness]** Permission-denied / read-only flows thin  
  **Location:** §7 (composer `< 2`, switch-agent `< 4) — no elicitation-specific rules  
  **Evidence:** `/resolve` requires `LEVEL_EDIT` (2+). Read-only user may see mirrored elicitation on parent stream but cannot resolve. Manage (3) required for sharing grants.  
  **Recommendation:** Explicit: read-only sees marker, widget hidden/disabled with explanation; manage vs owner for Share vs Switch agent.

- **[MEDIUM] [3-completeness]** Multi-user co-viewer race on shared session  
  **Location:** §9 (presence mentioned, races not)  
  **Evidence:** `session.presence` in capability map; concurrent resolve idempotent (`test_sessions_elicitation_api.py:170-201`); two-user stream test exists (`test_sessions_permissions.py:2927`).  
  **Recommendation:** On presence update, refresh pending from snapshot; if elicitation id absent, collapse widget; optional “Resolved by another viewer” copy.

- **[MEDIUM] [3-completeness]** JSON Schema form renderer scope / feasibility gap  
  **Location:** §3 form mode; framework §189 hand-wave  
  **Evidence:** Spec assumes generic JSON Schema → form panel. No spike, widget library, or schema subset (objects, enums, arrays, `oneOf`). Framework: “one-off build” with no plan.  
  **Recommendation:** Define v1 schema subset or reuse a Rust form crate; spike before implementation. Binary + `{approve: boolean}` covers many policy ASKs — prioritize.

- **[MEDIUM] [4-clarity]** `/resolve` usable for form mode contradicts openapi primary doc  
  **Location:** §4 table (“also usable for form mode”)  
  **Evidence:** Openapi titles `/resolve` as “URL-based elicitation” only (~5628). Implementation accepts any elicitation via shared `_resolve_elicitation` — spec is technically right but diverges from openapi narrative.  
  **Recommendation:** Keep both paths; note openapi description lag; prefer `POST /events` approval for form in Lens unless deep-linking.

- **[MEDIUM] [1-grounding + drift]** `GET …/elicitations/{id}` response shape under-specified  
  **Location:** §4 deep-link row  
  **Evidence:** Returns `status`, `message`, `phase`, `policy_name`, `content_preview` when pending; missing `mode`, `requestedSchema`, `target_session_id` in openapi description (~5580).  
  **Recommendation:** Typed client should parse full pending event from index or extend GET usage; deep-link page needs same fields as SSE event.

- **[MEDIUM] [5-feasibility]** Public-read `lens://session/…` deep link unresolved  
  **Location:** §10  
  **Evidence:** Public grant is server-side `__public__` read; Lens URL scheme unpinned; no auth token in link model.  
  **Recommendation:** Defer or specify: public read still needs connection + server URL; link carries connection/session ids only.

### Minor

- **[MINOR] [4-clarity]** Status header still says “Written fresh against 0.2.0” with no drift banner  
  **Recommendation:** Add “Re-verify against 0.3.0.dev0” + link to this review.

- **[MINOR] [2-cross-doc consistency]** Capability map §0.3 permissions row duplicates grant-level error  
  **Location:** capability-map line 195  
  **Recommendation:** Fix to 1–3 grantable when editing capability map (out of scope here — flag only).

- **[MINOR] [1-grounding + drift]** `policy_name` vs plural `policy_names`  
  **Recommendation:** Render single or joined list in widget header.

---

## framework.md

### Blockers

- **[BLOCKER] [2-cross-doc consistency + 4-clarity]** Load-bearing recon artifact absent from repo  
  **Location:** §2, §1 item 3; `docs/design/README.md` “Companion artifacts” (~76-77); capability-map ~500-501  
  **Evidence:** README: “Two synthesized notes — the **GPUI reconnaissance** … are load-bearing grounding sources.” `Glob **/recon*` under `lens/` → **0 files**. Framework §2 is an inline summary only (clone dates, repo names) with no paths, commit SHAs, or checklists in repo.  
  **Recommendation:** Check in recon artifact (even a slim `docs/design/recon/gpui-recon-2026-06-04.md` with per-widget pass/fail) **or** downgrade claims from “retired most widget risk” to “hypothesis pending verification spike.” Update README companion list.

### Major

- **[MAJOR] [5-feasibility]** “Recon retired most widget risk” contradicts open markdown spike  
  **Location:** §1.3, §4.1  
  **Evidence:** §1: “remaining spike item is markdown (§4).” §4.1: markdown is “load-bearing,” may require **gpui fork** (Paneflow path). Transcript progressive streaming + safe-prefix is core UX — not retired. JSON Schema approval forms (permissions doc) also un-spiked.  
  **Recommendation:** Reframe decision D: gpui locked for architecture (Rust types, no IPC); **markdown + form renderer** remain go/no-go spikes. Do not treat D as removing widget schedule risk.

- **[MAJOR] [3-completeness]** No fallback if gpui blocks a surface  
  **Location:** §1, §4.1, §6  
  **Evidence:** Decision locked; rejected Tauri/React only mentioned for IPC rationale. §4.1 fallback = “fork gpui” only. No plan for: markdown failure, JSON Schema forms, webview-style OAuth (if needed after permissions fix), or perf collapse on large diffs.  
  **Recommendation:** Add §4.3 “escalation ladder”: (1) hand-roll, (2) gpui fork, (3) scoped webview **only** for hostile HTML/markdown sandbox (last resort — conflicts with Bridge-native decision). Permissions URL flow may force (3) if external OAuth appears.

- **[MAJOR] [2-cross-doc consistency]** Capability map still labels framework “gated” in doc index  
  **Location:** capability-map table line 56 vs §0.7-D “Resolved: gpui”  
  **Evidence:** `framework.md` header: “Locked at gpui per capability map decision D.” Index row: “gpui (recommended; established by recon) vs React/TS — **gated**.”  
  **Recommendation:** Change index row to “**Resolved: gpui**” to match §0.7-D and framework §1.

### Medium

- **[MEDIUM] [5-feasibility]** gpui version pin stale / fork trigger vague  
  **Location:** §3 (`gpui = "0.2.2"`), §7  
  **Evidence:** “Revisit at first build”; markdown-append fork trigger unspecified version. Paneflow GPL — ideas only, reimplement burden.  
  **Recommendation:** Pin probe task: build hello-gpui on crates.io **and** zed git SHA; record which API Lens needs.

- **[MEDIUM] [5-feasibility]** Residual gpui gaps acknowledged but mitigations unverified  
  **Location:** §4.2  
  **Evidence:** No granular subscriptions; manual `window.refresh()` on drag; custom scrollbar; `canvas()` prepaint — mitigations reference state model §14 / ordinal board but no verification pass listed.  
  **Recommendation:** Add to §4.1 spike checklist or §7: board drag smoke test, 500-card notify storm, terminal scroll perf.

- **[MEDIUM] [4-clarity]** GPL vs MIT attribution boundary easy to miss  
  **Location:** §2.3-§2.5  
  **Evidence:** Paneflow GPL — “ideas only, reimplement”; markdown security patterns must be reimplemented cleanly.  
  **Recommendation:** One-line legal guard in §2: no copy-paste from GPL repos; link to MIT templates (Arbor) preferred.

- **[MEDIUM] [3-completeness]** Hot-reload theme + resize behavior still open  
  **Location:** §7  
  **Evidence:** Assumed, not verified — shell §18 depends on runtime theme swap.  
  **Recommendation:** Fold into first-build verification pass alongside markdown spike.

### Minor

- **[MINOR] [2-cross-doc consistency]** Permissions row in framework seam table understates form widget  
  **Location:** §5 table (“JSON-schema form renderer is a one-off build”)  
  **Recommendation:** Cross-link permissions review JSON Schema gap; mark as spike dependency, not solved by recon.

- **[MINOR] [4-clarity]** Decision D presentation within doc is clear  
  **Location:** §1 “LOCKED”  
  **Evidence:** Consistent with capability-map §0.7-D. No contradiction inside `framework.md`.

---

## Cross-cutting (E / D / Bridge)

| Resolved decision | Status in these docs |
|---|---|
| **E** — full permissions/sharing/multi-user first-class | **Mostly aligned** in scope (§7-§9, sharing dialog, policy editor). **Drift** on grant levels, auth providers, and hook inventory undermines implementation. |
| **D** — gpui locked | **Aligned** in framework §1 and capability-map §0.7-D. **Index table + README** still imply gating; recon artifact missing weakens justification. |
| **Bridge + `target_session_id`** | **Permissions §5 + sub-agent §6 aligned** with omnigent tests. **Gap:** state model singular `pending_elicitation` and Bridge index omit `target_session_id` — fix in state model, not permissions doc alone. |
| **Pending-elicitation badges** | Shell/capability-map/sub-agent use `pending_elicitations_count`; permissions lifecycle assumes single pending — **inconsistent** for fan-out parents. |

**Suggested fix order (permissions path):** URL-mode/security correction → grant level table → `pending_elicitations[]` model → resolved-event verdict rules → hook + external_elicitation inventory → co-viewer/error UX.

**Suggested fix order (framework):** Check in or downgrade recon artifact → reframe “retired risk” vs markdown/form spikes → fix capability-map “gated” label → add gpui escalation ladder.
