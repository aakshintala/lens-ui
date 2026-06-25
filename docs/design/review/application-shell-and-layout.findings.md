# Review findings — `application-shell-and-layout.md`

**Reviewed:** 2026-06-24  
**Spec:** `docs/design/application-shell-and-layout.md` (Draft, 2026-06-23)  
**Ground truth:** omnigent @ **0.3.0.dev0** (`openapi.json`, 59 REST paths; `schemas.py`, `routes/`)  
**Spec baseline:** omnigent **v0.2.0** (capability map header)

---

## TL;DR

- **Bridge rail entry is internally contradictory** and **undefined when no session is focused** — §6 says Bridge “replaces the main area” while §10 requires a working-area tab (which needs a focused session).
- **Card wave derivation assumes five `session.status` values** (`launching` / `waiting` included), but **REST snapshot and list poll expose only three** (`idle` | `running` | `failed`); slept/archived cards can mis-classify busy agents unless Lens persists fine-grained status locally.
- **Lens Archive (`stop_session` + Lens-local hide) is not reconciled with omnigent’s server `archived` flag** (`PATCH /v1/sessions/{id}`) — two parallel archive semantics will diverge on multi-client fleets.
- **Decision I (global time-windowed spend) is marked resolved but only appears in §21 open/deferred**, not in the header/chrome sections that should host it; keyboard shortcuts **drift from the capability map** (⌘\, ⌘F vs ⌘⇧F, missing working-area tab chords).
- **0.3.0 adds `WS /v1/sessions/updates`** (fleet push) in code/tests but it is **absent from `openapi.json`**; the poll-based background model remains valid but the spec should note the drift and whether Lens will adopt it.

---

## Findings

### OpenAPI grounding & version drift

**[HIGH] [1-openapi]** Session status granularity mismatch — wave vs REST poll  
**Location:** §5.1 (wave ladder), §17.4 (background poll)  
**Evidence:** Shell derives Working from `status ∈ {running, launching, waiting}` (`application-shell-and-layout.md` §5.1). SSE `session.status` events carry five values (`openapi.json` → `SessionStatusEvent.status`: `idle`, `launching`, `running`, `waiting`, `failed`). **`SessionResponse.status` and `SessionListItem.status` are only `idle` | `running` | `failed`** (`omnigent/server/schemas.py:1604`, `:1869`). The fleet poll (`GET /v1/sessions`, state model §10) updates card-summary fields for slept/archived sessions without a live stream.  
**Recommendation:** Pin the reducer rule: Active sessions fold SSE five-state; poll-only cards use three-state coarse mapping with explicit degradation (`running` subsumes launching/waiting, or persist last fine-grained status in Lens SQLite). Document the mapping in §5.1 and cross-ref state model §10.

**[HIGH] [1-openapi]** Lens Archive vs omnigent `archived` — conflated semantics  
**Location:** §4.6 (Archive)  
**Evidence:** Lens Archive = Lens-local hide + `stop_session` (reclaim harness). omnigent exposes **`archived: bool` on snapshot/list** toggled via **`PATCH /v1/sessions/{id}`** and filtered by **`include_archived`** on `GET /v1/sessions` (`schemas.py:1560–1563`, `:1712–1727`; `openapi.json` `/v1/sessions` `include_archived` param). These are independent mechanisms — server archive does not imply `stop_session`.  
**Recommendation:** Add an explicit reconciliation table: when to PATCH server `archived`, when to Lens-archive, what happens on restore, and how poll/`search_query` treats server-archived sessions that Lens still shows on a board.

**[MEDIUM] [1-openapi]** `WS /v1/sessions/updates` added in 0.3.0.dev0, missing from OpenAPI  
**Location:** §17.4 (background poll), state model §10  
**Evidence:** Tests reference `WS /v1/sessions/updates` (`omnigent/tests/server/routes/test_session_updates_ws.py`). **`openapi.json` lists 59 paths; this WS route is not among them.** `SessionListItem` docstring references it for `comments_count` invalidation (`schemas.py:1854–1855`). Shell/state model rely on **`GET /v1/sessions` poll** for background needs-attention detection.  
**Recommendation:** Note 0.3.0 drift in §17.4; decide whether Lens v1 stays poll-only or adopts the WS push (would affect poll cadence and menu-bar badge latency). Re-verify on each omnigent release per capability map §0.8.

**[MEDIUM] [1-openapi]** `session.todos` / `activeForm` is harness-scoped, not universal  
**Location:** §5.2 (activity line priority ②)  
**Evidence:** Snapshot `todos` is sourced from Claude Code todo cache; “Empty list for non-claude-native sessions” (`schemas.py:1564–1568`). `session.todos` SSE event description matches (`openapi.json` → `SessionTodosEvent`).  
**Recommendation:** Keep “degrades gracefully” but make priority ② explicitly conditional (Claude-native / forwarders that populate todos). Specify fallback ordering for codex-native, SDK harnesses, etc.

**[LOW] [1-openapi]** OpenAPI metadata version stale (`info.version: 0.1.0`)  
**Location:** N/A (grounding)  
**Evidence:** `openapi.json` `"version": "0.1.0"` while package is `0.3.0.dev0` (`omnigent/pyproject.toml`). Path count still 59 (matches capability map claim).  
**Recommendation:** Treat semver from package/tests, not openapi `info.version`; add to verification checklist.

**[LOW] [1-openapi]** Terminal WS attach remains out-of-band (known seam)  
**Location:** §14 (Terminals tray), §8 (Terminal tabs)  
**Evidence:** Attach path is `WS /sessions/{id}/resources/terminals/{id}/attach` (no `/v1`; `terminal_attach.py:130`), documented in `typed-client.md` §5, not in `openapi.json`. `POST …/terminals/{id}/transfer` and `DELETE` are in openapi.  
**Recommendation:** No change required if typed-client owns it; add a one-line §19 seam pointer so shell readers know attach is not in openapi.

**[INFO] [1-openapi]** Core shell dependencies present in 0.3.0.dev0  
**Location:** §5, §7, §10, §11  
**Evidence:** Verified present: `GET /v1/sessions` (+ `search_query`, `pending_elicitations_count` on list items), `GET /v1/sessions/{id}` (+ `total_cost_usd`, `pending_elicitations`, `todos`, `git_branch`, `workspace`), `POST /v1/sessions/{id}/events` (`stop_session`, `approval`, `interrupt`), `POST /v1/sessions/{id}/comments/send`, `GET …/child_sessions`, `POST …/switch-agent`, `GET …/elicitations/{id}`, `GET …/stream` + `session.presence`, env-scoped `POST …/search`.  
**Recommendation:** None — note as grounded.

**[INFO] [1-openapi]** Bridge Inbox elicitations grounded; Log/Knowledge are Lens-local by design  
**Location:** §10  
**Evidence:** Elicitations: `response.elicitation_request` + `pending_elicitations_count` + `/elicitations/{id}/resolve` (state model §11). Relays: `POST /comments/send` (`openapi.json`). Planning todos, deferred notes, Log rollups, Knowledge wiki — **no omnigent REST object** (state model §11; capability map §0.6 Bridge).  
**Recommendation:** Add a short §10.6 “Wire vs Lens-local” table so implementers do not search openapi for Log/Knowledge endpoints.

---

### Cross-doc consistency (decisions A–J, shell vs content)

**[HIGH] [2-cross-doc]** Bridge rail navigation contradicts itself; board-only entry undefined  
**Location:** §6, §10.1  
**Evidence:** §6: “Boards, Bridge, and Archive **replace the main area**” then “Bridge opens as a **shrinking working-area tab**.” §10.1 Global scope: “opened from the rail; a **working-area tab** that shrinks the board” — presumes focused-session layout. Decision **H** (capability map §0.7-H): “Bridge **rail destination**” with Inbox/Log/Knowledge. When user is on **board-only state** (§3, no focused session), there is no working-area tab bar.  
**Recommendation:** Resolve explicitly: (a) board-only → Bridge replaces main area as full-page `Bridge(scope=all, container=window|main)`; (b) focused-session → singleton working-area tab; (c) fix §6 bullet to stop saying Bridge “replaces the main area.” Align §8.1 tab-bar 📓 Bridge with rail entry (same surface, two mounts).

**[MEDIUM] [2-cross-doc]** Decision I resolved but not integrated into chrome spec  
**Location:** §21 (decision I), §7.4 (header)  
**Evidence:** Capability map §0.7-I **resolved**: cumulative per-card/per-group + **global today/7d/30d** from `cost_samples` (state model §6.2). Shell §21 lists this under “Open / deferred” despite “resolved” label. §7.4 header specifies per-session “live cost” only — no global windowed readout placement.  
**Recommendation:** Move decision I into §7.4 or §17.1 (health popover / status chrome): control placement, toggle UX, and behavior when `total_cost_usd` is `None` (unpriced — `schemas.py:1496–1500`).

**[MEDIUM] [2-cross-doc]** Keyboard shortcut drift vs capability map §0.6  
**Location:** §9.2  
**Evidence:** Shell: **⌘\\** collapses **boards column** (§3, §7.1). Capability map: **⌘\\ toggles side pane** (working area). Capability map: **⌘F** project content search; shell: **⌘⇧F**. Capability map lists **⌘[ / ⌘] / ⌘s\\** for working-area tabs; shell §9 omits them. Capability map: **⌘K** “palette (agents + board-switch + **actions**)”; shell: “quick-nav … (**navigation only**).”  
**Recommendation:** Single keyboard table in §9.2 reconciled with capability map §0.6; assign ⌘\\ to one behavior or split (e.g. ⌘\\ boards, ⌥\\ working area).

**[MEDIUM] [2-cross-doc]** “Right rail” vocabulary persists in sibling docs  
**Location:** Spec set index (capability map §0.1 table, README)  
**Evidence:** Decision **F** resolved: chat + **collapsible working area** beside chat (not a dedicated right icon-rail — shell §8.1). Capability map table still says “focused-session window (chat + **collapsible right rail**)”. Shell itself is mostly consistent (§8: “no separate right icon-rail”).  
**Recommendation:** Shell §19 or §2 glossary note: “working area” replaces “right rail” in sibling doc indexes (out of scope for shell-only edit — flag for capability map/README sync).

**[LOW] [2-cross-doc]** State model still marks Bridge placement “open”  
**Location:** `app-architecture-and-state-model.md` §11 (Bridge router)  
**Evidence:** “Placement decision … is **open** (capability map §0.7-H)” — but §0.7-H is **resolved** (rail destination + Inbox mockup). Shell §11 implements resolved H.  
**Recommendation:** Update state model §11 when that doc is next edited; shell should cross-ref resolved H without “open” qualifier.

**[LOW] [2-cross-doc]** Plan tab orphaned in launcher taxonomy  
**Location:** §8.1 (diagram), §8.2 (content list), §8.3 (singleton/multi)  
**Evidence:** §8.2 lists **Plan** as content tab; §8.1 launcher clusters omit Plan; §8.3 does not classify Plan as singleton or multi-instance. Composer mentions **collaboration-mode** (Plan mode, §7.5) tied to codex-native (`session.collaboration_mode` in openapi).  
**Recommendation:** Add Plan to §8.1 (likely singleton, peer to Review) or fold Plan into Review/Canvas; classify in §8.3.

**[INFO] [2-cross-doc]** Decisions A, B, C, D, E, F, G, H, J align where specified  
**Location:** §4, §5.3, §7, §11, §14, §20  
**Evidence:** A: Group grouping, single-root tree default (§8.2). B: sub-agents in tray, child window drill-in, no board cards (§14, §5.4). C: ring buffer (§7.3). D: gpui (§20). E: presence/ownership chrome (§7.4). F: working area + ⌘D deep-focus (§7.1). G: multi-window detach (§3, §6). H: Inbox band + ⌘I/⌘⇧I (§10–§11). J: switch-agent kebab, disabled when busy (§5.3).  
**Recommendation:** None beyond Bridge rail fix above.

**[INFO] [2-cross-doc]** Shell vs content split largely respected  
**Location:** §1, §19  
**Evidence:** Owns containers/chrome; defers transcript rendering, workspace data, permission widget lifecycle, sub-agent semantics, server bootstrap. Seams table (§19) matches surface docs. Minor leak: §16 annotation engine specifies cross-surface behavior (acceptable as shared primitive).  
**Recommendation:** None.

---

### Completeness & gaps

**[HIGH] [3-completeness]** Global search modal — data source and scope unspecified  
**Location:** §9.1–§9.3  
**Evidence:** “Global search = transient modal” + ⌘⇧P “global palette (app actions + the global-search modal).” No index definition. Partial server hook: `GET /v1/sessions?search_query=` (title + conversation item text, `openapi.json` `/v1/sessions`). State model §15 mentions cross-session search over local `items` table (Lens-side). Workspace doc: session file search is **`POST …/environments/{env_id}/search`**, not global.  
**Recommendation:** Specify global modal sources (session list search_query per connection, local SQLite transcript index, board/group names, Bridge items?) and result routing. Clarify relationship to ⌘K (nav-only) vs ⌘⇧P.

**[MEDIUM] [3-completeness]** Bridge entry from rail when no session focused  
**Location:** §3, §6, §10  
**Evidence:** Board state anatomy (§3) has no working area. §10.1 global Bridge assumes “shrinks the board” (focused layout).  
**Recommendation:** Define board-only Bridge UX (full-page fleet Inbox? force-focus Concierge session tab?) and keyboard paths (⌘⇧I, ⌘I) in that state.

**[MEDIUM] [3-completeness]** Loading / reconnect / skeleton states thin beyond errors  
**Location:** §17.2–§17.3  
**Evidence:** Empty states (onboarding §17.2) and three-altitude errors (§17.3) are specified. Missing: board loading first poll, Bridge queue hydration, focused-session open while snapshot/items in flight, bridge badge stale state, multi-connection partial outage.  
**Recommendation:** Add a short “Loading & stale” subsection under §17: per-surface placeholders, optimistic card state, “data as of {poll_time}” for slept cards.

**[MEDIUM] [3-completeness]** Multi-window edge cases deferred  
**Location:** §21 (Concierge 📌 + multi-window)  
**Evidence:** “Default: per-main-window; refine at build” for pinned Concierge across detached windows (decision G). Detach semantics for Bridge tab, board column, and ⌘I focus across windows not specified.  
**Recommendation:** Minimum rules: which window owns menu-bar badge, whether ⌘I raises existing child window or main window, detached session window layout (full focused-session chrome without board column?).

**[LOW] [3-completeness]** “Ready” wave depends on Lens-local sticky state  
**Location:** §5.1  
**Evidence:** Ready = `idle` + “unacknowledged turn completion” — not an omnigent field; requires Lens persistence of last-seen completion vs focus event.  
**Recommendation:** Cross-ref state model derived-attention flags; define clear on focus, view transcript, or explicit dismiss.

**[LOW] [3-completeness]** Accessibility / keyboard board navigation unspecified  
**Location:** §4, §5  
**Evidence:** ⌘1–9 positional jump (§9.2); no arrow-key focus order, group collapse, or screen-reader labels for wave states.  
**Recommendation:** Optional v1 note or backlog item — not blocking if intentional power-user focus.

---

### Clarity & structure

**[MEDIUM] [4-clarity]** Duplicate wave derivation paragraph  
**Location:** §5.1 lines ~245–254  
**Evidence:** The “whole-card glow / corrected ladder / five session statuses” block appears **twice** verbatim before the wave table.  
**Recommendation:** Delete duplicate; keep table + “Scheduled reserved” callout once.

**[MEDIUM] [4-clarity]** §6 Bridge behavior needs a decision table  
**Location:** §6  
**Evidence:** Three behaviors interleaved: destinations replace main area, Bridge exception, Concierge avatar, health dot, detach.  
**Recommendation:** Replace prose with a small matrix: Destination × Window state (board-only | focused) → Main-area behavior | Tab behavior.

**[LOW] [4-clarity]** §17.3 error routing references “§14.3” — section is §17.3  
**Location:** §17.3  
**Evidence:** Capability map cites “shell §14.3” for three-altitude errors; shell places this in §17.3 (§14 is volatile tray).  
**Recommendation:** Fix cross-refs across spec set to §17.3.

**[LOW] [4-clarity]** Theme §18 vs capability map “§15” token reference  
**Location:** §18, capability map §0.6  
**Evidence:** Capability map says “gpui Theme struct (shell §15)”; shell theme is §18, §15 is header/composer summary.  
**Recommendation:** Fix capability map pointer when edited.

---

### GPUI feasibility

**[LOW] [5-gpui]** Layout/windowing model is feasible; spikes named correctly  
**Location:** §20  
**Evidence:** gpui multi-window (§20, decision G). Ordinal slot board explicitly **simpler than gpui-flow** free-form canvas (§20). Tab+split working area aligns with Paneflow/editor-group patterns cited in framework doc. Hover-expand nav rail, floating Concierge panel, menu-bar resident app are standard macOS + gpui patterns.  
**Recommendation:** Proceed; optional spike only for tab+split resize performance with many terminal/webview tiles.

**[MEDIUM] [5-gpui]** Embedded web views + Canvas HTML/SVG block need framework seam  
**Location:** §12, §8  
**Evidence:** Canvas allows “embedded HTML/SVG” (§12.1); Web⁺ multi-instance tabs (§8.1). gpui native rendering vs webview for arbitrary agent HTML not decided in shell (framework doc owns substrate).  
**Recommendation:** Shell §12.2 seam: native shapes first; HTML/SVG via webview or sanitized subset — pin in framework doc to avoid layout rework.

**[LOW] [5-gpui]** Adaptive board packing unspecified but bounded  
**Location:** §4.3  
**Evidence:** “Count-aware balanced packing” as behavior without algorithm; ordinal slots avoid drag physics.  
**Recommendation:** Acceptable for spec; implementation can use deterministic row-major packing. No gpui blocker.

---

## Summary counts

| Severity | Count |
|----------|------:|
| HIGH     | 4 |
| MEDIUM   | 11 |
| LOW      | 8 |
| INFO     | 3 |

**Suggested fix order:** Bridge rail/board-only UX → session status poll mapping → Archive vs server `archived` → global search scope → keyboard reconciliation → decision I chrome placement → editorial dedup.
