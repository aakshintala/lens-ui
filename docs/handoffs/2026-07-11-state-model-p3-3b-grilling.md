# Handoff — state-model P3-3b GRILLED & CLOSED → next is `writing-plans` + execute — 2026-07-11

## TL;DR

**P3-3b is fully grilled and specified. No open design questions.** Every decision is
locked in **spec §2.4 (D24–D31)** — that block is the **SSOT for the plan**. App-arch
§3.5 + §13.1 were amended this session. Three live probes against **omnigent 0.5.1**
settled the behavioral unknowns. **Next session: `writing-plans` from spec §2.4, then
execute subagent-driven.** Do NOT re-grill — go straight to planning.

- **Decisions SSOT:** `docs/superpowers/specs/2026-07-08-state-model-engine-design.md`
  **§2.4 (D24–D31)** + the live-verify appendix + §7.1 amendment list.
- **Design source (amended):** `docs/design/app-architecture-and-state-model.md`
  §3.5 (Disconnected transition) + §13.1 (P3-3b amendment block).
- **Memory:** `state-model-p3-3b-grilling` (decisions + the 3 omnigent findings);
  `omnigent-two-id-space-reconciliation` (updated: messages-don't-split confirmed).
- **Grill trail** was `scratchpad/p3-3b-grill-notes.md` (ephemeral — the spec §2.4
  block supersedes it; don't rely on the scratchpad surviving).

## Scope (what P3-3b is)

- **Bucket A — recovery semantics:** D24–D29.
- **Bucket C — scaffold-id + tech-debt:** D30–D31.
- **Bucket B — viewport/render is NOT P3-3b.** It's a separate later plan that stands
  up a new **`lens-ui`** crate/cluster (gpui views: transcript virtualization, streaming
  markdown [vendored gpui-component], composer, elicitation forms, the board). Do not
  pull it in.

## Decision map (read spec §2.4 for the full text — this is the index)

| D | One-liner | Touches | Review seam |
|---|---|---|---|
| **D24** | Park = actor **EXITS** (terminal `Parked{reason}`, frees thread/reader) — dissolves the feeder-wedge | `actor/runloop.rs` parked loop | **MANDATORY cross-family** (edits merged P3-3a, subtractive) |
| **D25** | ONE **user-gated `reconnect`=respawn**; no auto-retry; **nothing auto-terminal** (403/404 rest Disconnected) | `actor/scheduler.rs` (+ `reconnect` entry, `parked` map) | — |
| **D26** | Slept persisted / **Parked = RAM fault**; re-read live status on attach | `scheduler`/`runloop` attach | — |
| **D27** | No silent send-text drop; 3 fates (`SendFailed/SendDenied{content}` / soft-pending / `SendLost{content}`) | `actor/outcome.rs` (+`content`), `runloop.rs` Err arm | cross-family (outcome enum shape) |
| **D28** | Held landed-detection = content-match vs catch-up delta + `pending_inputs`, conservative-landed bias, FIFO dup-match | `reduce/reconcile.rs`, `runloop.rs` catch-up | — |
| **D29** | Build NO survival persistence; `pending_user` RAM-only; defer path-2 to `lens-ui` arch B | (mostly a non-change + guardrails) | — |
| **D30** | Scaffold-id: dedup at PERSIST, uniform **`id → call_id`** + provisional flag + store-frontier cursor | `persist/` (provisional col), `runloop.rs` frontier+catch-up+commit | **MANDATORY cross-family** (durable-id reconcile) |
| **D31** | C2 frontier-fail-closed, C3 catch-up→iteration, C4 arg-bundle (DO); C5 command-ordering (DEFER) | `runloop.rs` | — |

## Task-order hints for the plan (not prescriptive — writing-plans decides)

C1/D30 rewrites the same catch-up/frontier/commit code that C2/C3/C4 clean and that
D24/D28 touch, so they cluster. A sane order: (1) D24 park=exit + D25/D26 scheduler
reconnect seam + deterministic park→reconnect test; (2) D27 outcome `content` + 3-fate
split; (3) D30 scaffold-id (provisional flag = a persist schema add — additive, P2
schema-version gate handles it; store-frontier cursor; `id→call_id` reconcile) folding
in C2/C3/C4; (4) D28 held landed-detection on the in-actor reconnect; (5) docs. Gate
D24/D27/D30 on cross-family seam review (subtractive/contract changes — the P3-3a
pattern earned its keep 3× there).

## Deferred (documented, non-blocking — do NOT build in P3-3b)

- **`lens-ui` composer-draft layer (arch B)** — owns durable unconfirmed-text; feeds the
  engine DOWN the spawn port (`SessionState.pending_user`) at respawn; verdict flows UP by
  emission (favor a fail-safe **"landed→clear"** outcome). Guardrails P3-3b must honor:
  the held-set is a *spawn input* (seed before autonomous catch-up), and don't design the
  outcome enum to preclude a "landed→clear" verdict.
- **omnigent client-message-id** — a contract feature-request (would make send/tool dedup
  fully robust; today content-match for held sends, `call_id` for tools). File upstream.
- **Bucket B viewport** — its own plan + `lens-ui` crate.
- **C5** fuller command-ordering; **opportunistic provisional-promotion** (D30 native
  re-fetch optimization); **frontier-anchoring** for D28 dup-match.

## Live-verify appendix (omnigent 0.5.1 — see spec §2.4 for detail)

`failed` = resumable-in-place (never 404s, heals on next `message` POST, resets to `idle`
on server restart, transcript byte-durable); organic-crash == `stop_session`; heal is
host-gated (503 `runner_unavailable` = retry-not-dead); stream is chrome-only (no
transcript snapshot — confirms D19); **messages do NOT split** (id-match, only tools do).

## Process notes

- A throwaway **omnigent 0.5.1 server may still be running** from the probes (`omnigent
  stop` to tear down). `omnigent stop` ALSO kills the host daemon and `server start` does
  NOT respawn it — relaunch the host manually (`omnigent host … --non-interactive`) for
  any future live-verify.
- Delegation per CLAUDE.md: composer-2.5 per task, Opus inline for the seam/architecture,
  cross-family review (grok-4.5 or codex/gpt-5.5) on the D24/D27/D30 subtractive seams.
- The `provisional` column is a P2-schema **add** — additive, the per-file
  `schema_version` gate covers it (no migration of existing rows needed; default false).
