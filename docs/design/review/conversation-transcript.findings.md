# Review: `conversation-transcript.md`

**Reviewed against:** omnigent `0.3.0.dev0` (`omnigent/server/schemas.py`, `omnigent/entities/conversation.py`, `openapi.json`, `API.md`)  
**Spec version target:** v0.2.0  
**Date:** 2026-06-24

## TL;DR

- **Markdown security is under-specified** relative to `framework.md` §2.5: assistant policy is good (no HTML, artifact-only images) but user autolinks, `` ```markdown `` opt-in blocks, and file-path autolink lack an explicit threat model and sanitization contract.
- **Tool-span status vocabulary drifts from wire shapes** — spec uses `pending`/`running`/`error`; omnigent items use `in_progress`/`completed`/`action_required`/`incomplete` (`schemas.py:2648–2651`, typed-client §10).
- **Several persisted item kinds / SSE events have no render path** — `error` items, `is_meta` messages, multimodal user blocks, `response.output_file.done`, interrupted partial assistant messages, `response.client_task.cancel`, and `response.elicitation_resolved` (timeout/cancel without verdict).
- **`CompactionFailed` contradicts source semantics** — spec mandates an error marker; `CompactionFailedEvent` doc says dismiss the spinner with **no** permanent marker because history was not modified (`schemas.py:3207–3215`).
- **Cross-doc seam conflict on elicitation placement** — transcript + permissions dock at composer; application shell §19 still says “in-transcript + attention.”

---

## Findings

### Markdown & security

**[HIGH] [COMPLETENESS / SECURITY]** User-message autolinks lack link sanitization  
**Location:** §6.2 (“Paths/URLs autolinked”)  
**Evidence:** Assistant markdown excludes raw HTML (§6.1) and artifact-only images; `framework.md` §2.5 requires `validate_link_url` (blocks `javascript:`, `file:`, `data:`, etc.) before any click handler. §6.2 autolinks user paths/URLs with no equivalent rule.  
**Recommendation:** Apply the same link scheme allowlist to user autolinks; document that user content is lower-risk but not trusted (paste attacks, `javascript:` in pasted logs).

**[HIGH] [COMPLETENESS / SECURITY]** `` ```markdown `` / `` ```md `` blocks inherit assistant markdown without a stated security boundary  
**Location:** §6.2  
**Evidence:** §6.1 security rules (no HTML, artifact-only images, external URL images → link) apply to assistant prose; §6.2 explicitly renders fenced markdown as formatted markdown with no cross-reference to §6.1 or `framework.md` §2.5.  
**Recommendation:** State explicitly that opt-in markdown blocks use the **same** sanitization pipeline as assistant markdown (HTML escaped, images artifact-only, links validated).

**[HIGH] [COMPLETENESS / SECURITY]** File-path autolink threat model missing  
**Location:** §6.1 (“Bare paths … become clickable … `navigate_to_file`”)  
**Evidence:** `framework.md` §2.5 `validate_image_ref` guards path traversal and symlink escape; no analogous rule for path autolink (`../../../etc/passwd`, `~/.ssh/id_rsa`, workspace escape). Click handler is delegated to workspace doc but transcript owns detect+paint+emit.  
**Recommendation:** Add a client-side path normalization boundary: resolve relative to session workspace root, reject `..` escape and absolute paths outside allowed roots; fail closed to plain text.

**[MEDIUM] [COMPLETENESS / SECURITY]** Artifact inline-image loading needs explicit validation contract  
**Location:** §6.1 (“inline images = IN, artifact-sourced only … `file_id` / workspace file via authenticated API”)  
**Evidence:** Policy is correct (blocks external tracking pixels); `framework.md` §2.5 requires `validate_image_ref` at load time. Spec names the policy but not the implementation seam (who validates, what IDs are allowed, symlink/traversal on workspace files).  
**Recommendation:** Pin validation at the fetch boundary (whitelist `file_id` from known session resources or authenticated `GET …/files/{id}/content`; reject arbitrary path strings in `![](…)`).

**[MEDIUM] [CLARITY / SECURITY]** No consolidated markdown threat-model section  
**Location:** §6, §19 (framework-divergence note only)  
**Evidence:** Security rules are scattered (HTML exclude, image policy, framework.md cross-ref). Missing explicit treatment of: `data:`/`vbscript:` links, HTML entities, autolink parsers that bypass markdown, Unicode homoglyph URLs, and streaming safe-prefix holding partial `[text](javascript:…` until close.  
**Recommendation:** Add a short §6.3 “Markdown security boundary” listing allowed constructs, forbidden constructs, and which channels (assistant / user verbatim / user `` ```md ``) each rule applies to.

**[LOW] [CLARITY]** Safe-prefix streaming and incomplete link/image syntax  
**Location:** §5  
**Evidence:** Safe-prefix holds open constructs until close — good for flicker; does not state behavior for partially streamed `[label](http` or `![alt](` across chunk boundaries beyond “plain/pending.”  
**Recommendation:** One sentence: incomplete links/images render as literal text until the closing `)`; no prefetch, no click target until validated closed construct.

---

### Event / schema grounding & drift (0.2.0 → 0.3.0.dev0)

**[HIGH] [GROUNDING]** Tool-span status enum does not match wire / typed-client  
**Location:** §8.2  
**Evidence:** Spec: `pending / running / completed / error / action_required`. Wire example in `OutputItemDoneEvent` (`schemas.py:2648–2651`): `"status": "action_required"`. Executor tests observe `completed` and `in_progress` (`test_executor_adapter.py`). Typed-client §10: `completed | action_required | incomplete`. No wire `pending`, `running`, or `error` on `function_call` items.  
**Recommendation:** Map wire statuses literally; derive UI labels (`running` ← `in_progress` + awaiting output; `error` ← failed tool output or `response.error` with `source: tool`). Document the mapping table.

**[HIGH] [GROUNDING]** `CompactionFailed` render contradicts omnigent semantics  
**Location:** §10  
**Evidence:** Spec: “`CompactionFailed` → an error marker.” `CompactionFailedEvent` doc (`schemas.py:3207–3215`): “dismiss [spinner] **without leaving a permanent marker**, since the conversation history was not modified.”  
**Recommendation:** Align with source: transient failure toast or dismiss-only; no durable divider. If product wants a permanent marker, note it as a Lens-only UX extension.

**[HIGH] [GROUNDING / COMPLETENESS]** Persisted `error` item kind omitted from `ItemKind` / render pipeline  
**Location:** §3, §11; state model §2.3  
**Evidence:** `conversation.py` defines `ErrorData` and `"error"` in `ITEM_TYPE_TO_DATA_CLS` (`conversation.py:323–341`, `531–535`). Items mirror `response.error` for reconnect (`ErrorData` docstring). State model `ItemKind` and transcript §11 cover transient `response.error` / `RetryEvent` only — not persisted `type: "error"` items from `GET /items`.  
**Recommendation:** Add `Error` to canonical `ItemKind`; render persisted errors same as transient `response.error` (shell §17.3: source · code · message).

**[MEDIUM] [GROUNDING]** `TerminalCommand` shape drift  
**Location:** §11 (“`❯ !cmd` card”)  
**Evidence:** Wire `TerminalCommandData` has `kind: "input" | "output"` with separate `input` / `stdout` / `stderr` fields (`conversation.py:455–477`), not a single `{ command: String }` as in state model §2.3. Claude Code emits paired input+output items.  
**Recommendation:** Render input as `❯ !cmd`; optionally pair or stack stdout/stderr output item; update state model field names.

**[MEDIUM] [GROUNDING]** `SlashCommand` field drift  
**Location:** §11  
**Evidence:** Spec/state model: `{ name, raw }`. Wire: `SlashCommandData` with `kind: "skill" | "command"`, `name`, `arguments`, `output` (`conversation.py:485–515`).  
**Recommendation:** Render `kind` for icon/prefix (`/skill` vs `/model`); show `arguments` + optional `output`.

**[MEDIUM] [GROUNDING]** Reasoning item field names abstract wire shape  
**Location:** §7  
**Evidence:** Spec/state model: `full_text`, `summary_text`, `encrypted: bool`. Wire `ReasoningData`: `summary: [{type: "summary_text", text}]`, `content: [...] | None`, `encrypted_content: str | None` (`conversation.py:360–378`). Encrypted reasoning: `ReasoningStartedEvent` fires even when no deltas follow (`schemas.py:2587–2590`).  
**Recommendation:** Document reducer projection (`summary_text` ← summary blocks; `full_text` ← content blocks; `encrypted` ← `encrypted_content.is_some()` or absent content). Handle summary-only and encrypted-no-expand paths (already partially covered).

**[MEDIUM] [GROUNDING / COMPLETENESS]** `NativeTool` nesting  
**Location:** §9  
**Evidence:** Spec: `NativeTool { tool_type, data }`. Wire: item `type: "native_tool"`, data `{ item: { type: "web_search_call", … } }` (`conversation.py:412–425`). Typed-client uses inner `kind` from nested dict (typed-client §10).  
**Recommendation:** One line in §9: dispatch on `data.item.type` (or typed-client’s extracted `kind`); table keys remain valid.

**[MEDIUM] [COMPLETENESS]** Multimodal user message content not specified  
**Location:** §6.2  
**Evidence:** `MessageData.content` supports `input_text`, `input_image`, `input_file` (`conversation.py:234`, `API.md` upload flow). §6.2 covers text + backtick gates only; §6.1 mentions `input_image` only for assistant artifact reuse.  
**Recommendation:** Render `input_image` / `input_file` as inline attachment cards (authenticated fetch by `file_id`); fall back to placeholder on missing file.

**[MEDIUM] [COMPLETENESS]** `output_text` content blocks and `annotations`  
**Location:** §6  
**Evidence:** Assistant messages use `output_text` blocks, often with `annotations: []` (`API.md:294`, `conversation.py:687`). Spec treats assistant text as a single markdown stream; no mention of per-block rendering or annotation types (citations, file references).  
**Recommendation:** Define behavior: concatenate `output_text` blocks in order; if annotations carry file citations, render as footnotes or inline chips (even if v1 is “ignore unknown annotations”).

**[MEDIUM] [COMPLETENESS]** `response.output_file.done` not covered  
**Location:** §12 (resource events only)  
**Evidence:** `OutputFileDoneEvent` (`schemas.py:2687–2709`: `file_id`, `filename`, `content_type`). Capability map lists it alongside `output_item.done`. Spec covers persisted `ResourceEvent` file artifacts and §6.1 artifact images, not the transient file-completion SSE event during streaming.  
**Recommendation:** On `output_file.done`, show inline downloadable card (same as §12 file artifact) or coalesce into the streaming message’s attachment row.

**[MEDIUM] [COMPLETENESS]** `is_meta` messages should be hidden  
**Location:** §1 scope / §6  
**Evidence:** `MessageData.is_meta: true` = “hidden from user-facing transcripts” (`conversation.py:222–225`). Spec does not filter meta messages.  
**Recommendation:** Reducer or render transform drops `is_meta` messages from transcript (state model render-time transform list is the seam).

**[MEDIUM] [COMPLETENESS]** Interrupted partial assistant messages  
**Location:** §15  
**Evidence:** `MessageData.interrupted: true` for durable partial on interrupt (`conversation.py:226–230`). §15 covers `session.interrupted` + `response.incomplete` for control flow, not visual treatment of partial assistant text already finalized as an item.  
**Recommendation:** Render interrupted messages with a visible “interrupted” affordance (e.g. truncated marker); do not treat as a normal completed turn.

**[LOW] [COMPLETENESS]** New 0.3.0 events with no transcript mention  
**Location:** (gap — not in spec)  
**Evidence:** Present in `ServerStreamEvent` union (`schemas.py:3427–3482`): `turn.started|completed|failed|cancelled`, `response.client_task.cancel`, `session.status` with `waiting` + setup `error` on `failed` (`schemas.py:2045–2058`). Capability map notes `turn.*` as optional for card wave.  
**Recommendation:** Transcript should explicitly defer `turn.*` to card/shell OR define minimal inline markers for turn-level failures that never produce items. Document `client_task.cancel` if client-side tools are in v1 scope.

**[LOW] [GROUNDING]** `SessionAgentChangedEvent` wire vs `AgentChanged` item  
**Location:** §13  
**Evidence:** Wire event carries `agent_id`, `agent_name` only — no `from` (`schemas.py:2192–2221`). State model synthesizes `AgentChanged { from, to, at }` from prior scalar + event (state model §12.2). Spec marker `⇄ agent Y → Z` is consistent if reducer fills `from`.  
**Recommendation:** One clarifying sentence: marker fields come from reducer state, not the SSE payload alone.

**[LOW] [GROUNDING]** Function call `status: "error"`  
**Location:** §8.2  
**Evidence:** Tool failures surface via `response.error` (`source: tool`, `schemas.py:3143–3155`) or failed output text, not a dedicated `function_call` status literal in tests.  
**Recommendation:** Replace `error` status with explicit derivation rules from output + `response.error`.

---

### Cross-doc consistency

**[HIGH] [CONSISTENCY]** Elicitation widget placement conflicts with application shell  
**Location:** §18 vs `application-shell-and-layout.md` §19  
**Evidence:** Transcript §18 + permissions §3: widget **docks at composer**; transcript shows record marker only. Shell §19: permissions seam = “in-transcript + attention.” Shell §17.3 error routing aligns with transcript markers but adds **Retry / Edit & resend** not in transcript §11.  
**Recommendation:** Update shell §19 to “composer dock + in-transcript record marker”; add Retry/Edit affordance to transcript §11 or reference shell §17.3 explicitly.

**[LOW] [CONSISTENCY]** Decision J (transcript stays on switch) — aligned  
**Location:** §4, §13, §20  
**Evidence:** Matches state model §12.2, agent-definition §7, capability map §0.7-J, README decision J. `session.agent_changed` + non-remount consistently specified.  
**Recommendation:** None.

**[LOW] [CONSISTENCY]** Sub-agent drill-in (decision B) — aligned  
**Location:** §8.6  
**Evidence:** `open ↗` → `navigate_to_session` matches sub-agent topology §3 drill-in to child focused-session window; peek inline is depth-1 only.  
**Recommendation:** Cross-link sub-agent topology for window/breadcrumb behavior on `open ↗`.

**[LOW] [CONSISTENCY]** Chat slot ownership — aligned  
**Location:** §18, shell §19  
**Evidence:** Shell owns chat column position/composer container; transcript owns rendering. Concierge reuse (§18) matches shell §13.  
**Recommendation:** None.

---

### Completeness & gaps (streaming, tools, errors, scale)

**[MEDIUM] [COMPLETENESS]** `response.elicitation_resolved` without verdict  
**Location:** §18  
**Evidence:** Permissions doc: marker becomes `✓ approved / ✗ denied / ↯ cancelled` on `elicitation_resolved`. Transcript §18: only `✓ approved / ✗ denied`. `ElicitationResolvedEvent` fires on timeout/cancel/harness exit with no UI verdict (`schemas.py:2936–2962`).  
**Recommendation:** Add `↯ cancelled` / `timed out` permanent marker path for resolved-without-verdict.

**[MEDIUM] [COMPLETENESS]** Shell three-altitude error UX partially missing  
**Location:** §11 vs shell §17.3  
**Evidence:** Shell: turn errors get “Retry / Edit & resend”; retry is quiet inline. Transcript §11: code + message marker only.  
**Recommendation:** Add §11 affordances or defer table pointing to shell with transcript-owned actions list.

**[MEDIUM] [COMPLETENESS]** `session.todos` completion markers vs event-only updates  
**Location:** §14  
**Evidence:** Todo shape `{content, status, activeForm}` matches `SessionTodosEvent` (`schemas.py:2237–2242`). Completion markers on `status → completed` require diffing successive `session.todos` events; not stated.  
**Recommendation:** Specify reducer emits synthetic completion marker items (or transcript diffs todo snapshots) so markers survive hydration from snapshot-only state.

**[LOW] [COMPLETENESS]** Large transcript / nested peek depth  
**Location:** §8.6, §16  
**Evidence:** `flatten_sub_agents` depth-1 peek can expand large child histories inline — conflicts with head/tail truncation (§8.3) unless peek also truncates.  
**Recommendation:** Apply §8.3 tiers inside peek; default collapsed.

**[LOW] [COMPLETENESS]** `Bridge` comment blocks (`POST /comments/send`)  
**Location:** §18  
**Evidence:** Spec mentions structured feedback block; no render sketch. Acceptable deferral if Bridge owns shape — flag as open.  
**Recommendation:** Minimal block spec or pointer to Bridge doc when written.

---

### Clarity & structure

**[MEDIUM] [CLARITY]** §16 section numbering collision  
**Location:** §16 title “The scrolling surface — E” vs §19 “Framework-divergence notes” item 3 also “§16” virtualization  
**Evidence:** Decision E in capability map is working-area collapse (⌘D), not scrolling. Scrolling section labeled “E” conflates two “§16” references.  
**Recommendation:** Renumber decision letter for scrolling or rename framework-divergence cross-ref to “§16 scrolling” explicitly.

**[LOW] [CLARITY]** `ViewBlock` exhaustive `ItemKind` claim vs composites  
**Location:** §3  
**Evidence:** “Projection matches `ItemKind` exhaustively” but `ViewBlock` also includes synthetics (`StreamingMessage`, `ReconnectBreak`, `AgentChangedMarker`, …) not on wire.  
**Recommendation:** Clarify: exhaustive over `ItemKind` → passthrough + known composites; synthetics come from `StreamScratch`/reducer.

**[LOW] [CLARITY]** Work-section chip metadata sources  
**Location:** §4 chip example (`Sonnet-4.6 → Opus-4.8 · 12.4k tok · $0.04`)  
**Evidence:** Tokens/cost from `session.usage` / per-turn `usage` on `response.completed`; model switch marker from `session.model` (§4). Agent switch in chip from `AgentChanged` items. Aggregation rules not spelled out.  
**Recommendation:** One paragraph: chip fields = fold of items in turn + latest `session.usage` snapshot at turn end.

---

### GPUI rendering feasibility

**[HIGH] [FEASIBILITY]** Variable-height virtualization vs `uniform_list`  
**Location:** §16, §19 item 3  
**Evidence:** §16 contract 3: “measure-and-cache per item; collapse-by-default.” §19 cites gpui `uniform_list` (fixed row height). `WorkSection` expands to nested variable-height tool spans, diffs, images. Framework §4.1: markdown is the spike item; no variable-height virtualizer proven.  
**Recommendation:** Treat `uniform_list` as inadequate for v1 transcript; spec a **variable-height** virtualizer (measured cache + anchor indices) or flatten `ViewBlock` to a list of row descriptors with explicit height keys. Spike before locking §16.

**[MEDIUM] [FEASIBILITY]** Nested `WorkSection` + flat stream tension  
**Location:** §3–§4  
**Evidence:** “Flat stream” for virtualization (§4) but `WorkSection { blocks: Vec<ViewBlock> }` is a tree. Virtualizer must either (a) flatten with collapse bit in key, or (b) virtualize outer list only with inner scroll — inner scroll fights §8.3 “no max-height scrollbox.”  
**Recommendation:** Define flattening transform: collapsed `WorkSection` = one row; expanded = splice child rows into virtual index space with stable ids.

**[MEDIUM] [FEASIBILITY]** Progressive markdown in-place diff  
**Location:** §5, §19 item 1  
**Evidence:** Requires stable widget identity + pulldown-cmark incremental parse (`framework.md` §4.1). GPUI `cx.notify()` re-renders whole view (`framework.md` §4.2) — mitigated by per-session entities but markdown subtree diff is unproven.  
**Recommendation:** Keep as spike gate; fallback plan (swap text only in single `Label`/`div` without full remount) documented in §19.

**[LOW] [FEASIBILITY]** ANSI terminal fidelity in tool spans  
**Location:** §8.5  
**Evidence:** Framework pins terminal widget to `alacritty_terminal` for live PTY; static Bash capture reuse of ANSI parser is feasible but separate code path.  
**Recommendation:** Reuse terminal grid parser for static captures or shared ANSI→styled-runs layer.

---

## Summary table (severity × dimension)

| Severity | Count | Dominant themes |
|----------|-------|-----------------|
| HIGH | 8 | Markdown security gaps; status/compaction/error grounding; shell seam; GPUI virtualization |
| MEDIUM | 16 | Item-shape drift; missing render paths; elicitation/error UX |
| LOW | 10 | Clarifications; optional events; cross-links |

---

## Verdict

The spec’s architectural choices (pure `ViewBlock` projection, flat stream + work collapse, transcript-stays-on-switch, composer-docked approvals) are sound and mostly consistent with sibling docs. The largest risks before implementation are **(1) markdown/link/image sanitization** — must be specified at the same rigor as `framework.md` §2.5 for all channels, **(2) wire-accurate tool/status/item shapes**, and **(3) a variable-height virtualization plan** that survives expanded tool spans and nested work sections.
