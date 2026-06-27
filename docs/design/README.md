# Lens — design spec set

**Lens** is a native macOS desktop client for the **omnigent** open-source AI agent framework (`omnigent-ai/omnigent`, pinned at `0.3.0.dev0`, Apache-2.0). It is a pure *client* of omnigent's HTTP + SSE + WS API — never an orchestrator itself. It spawns an `omnigent server` as a supervised subprocess on the local Mac, and connects as a client to remote omnigent servers (e.g. an internal dev workspace). It renders every agent event as a native GPUI widget instead of wrapping a web UI. 

The wedge: omnigent already solves orchestration (sub-agents, policies, worktrees, sandboxes, multi-harness) and exposes a rich SSE event taxonomy; its desktop story is a half-baked web wrapper. Lens is the supervisor surface omnigent doesn't have — diff review, Bridge (actionable queue + agent-to-agent relay + todos), fleet dashboard, mid-flight steering, a Canvas for agent-presented visuals, and a long-standing Concierge agent — all native, at fleet scale.

## Status of the whole set

Draft. Each document is drafts-then-review per the brainstorming flow. Specs are grounded in `openapi.json` + `0.3.0.dev0` code (the internal `designs/*.md` rationale docs are gone in the open-source release — only `CLI_CONTRACT.md`, `POLICIES.md`, and a handful of unrelated process docs remain). Every endpoint/event assertion is verifiable against the checked-in `openapi.json`, now **vendored at `vendor/omnigent-0.3.0.dev0/openapi.json`** (with an `OMNIGENT_PIN` file). Note: that file's own `info.version` is a stale `"0.1.0"` — the pin is the package semver `0.3.0.dev0`, not `info.version`.

## Document set

Each document is named by what it describes.

| Document                                | Describes                                                                                                                                                                     | Depends on         |
| --------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------ |
| `capability-map-and-design-language.md` | omnigent surface → Lens capability map; Lens's UX vocabulary; cross-cutting decisions; verification posture                                                                   | —                  |
| `typed-client.md`                       | the `lens-client` Rust crate — typed REST+SSE+WS client; full event taxonomy; no-replay reconnect; environment-scoped workspace endpoints; presence; terminal WS attach paths | —                  |
| `app-architecture-and-state-model.md`   | how the typed client feeds the view-model, state store, command flow; the Bridge router; presence/co-viewers; switch-agent & fork flows                                        | the typed client   |
| `application-shell-and-layout.md`       | board/home, focused-session window (chat + collapsible working area beside chat, ⌘D deep-focus), resource-rail navigator, global search/nav/palette, app-state chrome, Bridge (native, one rail destination), theme              | the state model    |
| `conversation-transcript.md`            | the transcript surface, full fidelity; markdown security boundary                                                                                                             | the state model    |
| `workspace-and-terminals.md`            | one-workspace-per-session ↔ task model; env-scoped fs/diff/search/shell; terminal WS attach + transfer                                                                        | the state model    |
| `agent-definition.md`                   | the net-new vocabulary; 19 harnesses (`OMNIGENT_HARNESSES`); filesystem YAML; bundle upload clone; live switch-agent                                                                                 | the state model    |
| `permissions-and-elicitations.md`       | form/URL elicitations; `/resolve`; target_session_id; the four harness permission/elicitation hooks; per-session sharing (grant levels 1–3) + policy editor + identity                                                   | the state model    |
| `sub-agent-topology.md`                 | child-session trees with richer summaries; pending-elicitation badges                                                                                                         | the state model    |
| `server-lifecycle.md`                   | Lens-supervised server+embedded-runner; managed sandbox hosts; hosts/policies/permissions/fork topology                                                                       | the typed client   |
| `framework.md`                          | gpui (**resolved/locked**, decision D) vs React/TS (rejected); recon summary; residual spikes (markdown + JSON-Schema form renderer)                                                                                                                  | the capability map |

Dependency shape: **the typed client → the state model → the application
shell**; the surface documents (transcript, workspace, agent definition,
permissions, sub-agent topology) depend on the state model and **dock their
content into the shell's slots**; **the typed client → the server lifecycle**;
**the framework** document is orthogonal.

## How these documents are organized

- **Behavior & contract first** for frontend surface documents (transcript,
  workspace, permissions, sub-agent topology) so they survive the framework
  decision; note gpui vs React/TS divergences only where they actually matter.
- **Shell vs content split** — the application shell document owns containers
  and chrome; surface documents own the content that fills the slots, written
  container-agnostic.
- **Ground truth is the openapi.** Every endpoint assertion cites the openapi
  path or schema. Internal design docs that vanished in the open-source
  release are NOT cited as grounding sources.
- **Pin-and-verify, not gospel.** omnigent is pinned at `0.3.0.dev0` (a moving
  dev target); the contract will move. Each document keeps a "what would break if X changes" margin in its
  seams section, and the typed client owns the contract-pinning layer for the
  whole set.

## Cross-cutting decisions (resolved across the set)

Carried in the capability map §0.7. Status snapshot:

| Decision                                                     | Status                                                                                                                                                                                                                           | Owner                                |
| ------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------ |
| A. Task ↔ session model (one-workspace-per-session)          | **Resolved** — task = session; the board **Group** is the grouping (no first-class Task entity); single-root file tree by default; "task" retired as a term                                                                                                                                | application shell                    |
| B. Sub-agent tree model (rail/tree vs flat list vs hybrid)   | **Resolved** — focused-parent home is the tray "Sub-agents" segment; children never become board cards; drill-in opens the child's own window                                                                                                                                                               | sub-agent topology                   |
| C. Terminal reconnect UX (ring buffer vs blank-on-reconnect) | **Resolved** — Lens-side ring buffer covers brief reconnects; deliberate Sleep closes Lens observation and sends best-effort `stop_session`, so terminal PTY survival is server-owned                                                                                                                                                                                           | workspace & terminals                |
| D. Framework (gpui vs React/TS)                              | **Resolved: gpui.** Recon retired the terminal/diff/board risk; markdown + JSON-Schema form renderer remain un-spiked (framework §4).                                                                                                                                              | framework                            |
| E. Auth & multi-user posture                                 | **Resolved:** omnigent supports sharing/permissions/multi-user natively — Lens surfaces them as first-class (no deferral). Connection-auth (per-connection token) for remote servers + full permissions/sharing/policy-editor UI | permissions + server lifecycle       |
| F. Focused-session layout (right rail vs tabs)               | **Resolved** — collapsible working area beside the chat column; ⌘D deep-focus is a third state                                                                                                                                                                      | application shell                    |
| G. Multi-window posture                                      | **Resolved** — both (gpui multi-window native)                                                                                                                                                                                 | application shell                    |
| H. Bridge — collapsed surface (Inbox + Log + Knowledge sub-panes; ⌘I jump-to-agent, ⌘⇧I open Inbox) | **Resolved** — collapsed into one rail destination; Inbox UI = pinned "Needs you" band + reverse-chron stream (mockup pass done) | state model + application shell      |
| I. Global/board rollup spend readout                         | **Resolved** — cumulative per-card/project (server `total_cost_usd`) + time-windowed global (today/7d/30d, Lens-computed from a cost-sample series)                                                                                                                                                                                         | state model + application shell      |
| J. Live switch-agent handoff                                 | **Resolved** — card + composer re-render in place; transcript stays (no remount); Lens applies owner-only **and** idle-only guards as a **UI policy** (the API floor is `LEVEL_EDIT`, and the server idle-guard misses `launching` — Lens preflights it)                                                                                                                                                                     | agent definition + application shell |

## Sequencing

Per the brainstorming sessions: **don't pick an MVP yet.** The whole map first;
the sequencing call is separate.

## Companion artifacts

- `capability-map-and-design-language.md` — the keystone; start here.
- `vendor/omnigent-0.3.0.dev0/openapi.json` — the vendored contract ground truth (with `OMNIGENT_PIN`); the typed client codegens + contract-tests against it.
- The **GPUI reconnaissance** is summarized inline in `framework.md` §2 (a read of three reference GPUI apps at HEAD 2026-06-04 — there is no separate recon file). The omnigent-as-stability-boundary rationale is carried in the typed-client + capability-map grounding notes.
