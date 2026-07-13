# Handoff — lens-client Plan 4 (pre-consumer hardening) execution

**Date:** 2026-06-26
**Branch:** `feat/lens-client-hardening` (base `3dfadd9`, off main `8a5a8b3`) → `8fe4dd5`
**Plan:** [`plan4`](../plans/2026-06-26-lens-client-plan4-pre-consumer-hardening.md)
**Ledger:** `.superpowers/sdd/progress.md` (PLAN 4 section)

## What & why

lens-client was feature-complete (Plans 1–3c). Before building the state-model on
it, ran a **consolidated whole-crate review** (gpt-5.5 cross-family + Opus
architecture synthesis) — the per-plan seam reviews couldn't see cross-plan
composition or consumer-seat ergonomics. The review found 1 real bug + systemic
pre-ossification issues; this branch fixes the cheap-now subset.

## Delivered (5 tasks, subagent-driven: composer-2.5 build + per-task gpt-5.5 review + Opus spot-check on Task 5 + 1 final whole-branch review)

1. **Phantom `ReasoningClosed` after mid-reasoning reconnect** (real bug) — `reset_seen_items`
   → `reset_transient` clears the open reasoning bracket too. (`a0bfe8d`)
2. **HTTP robustness** — `CONNECT_TIMEOUT`(10s) on the shared client + per-request
   `REST_TIMEOUT`(30s) on REST helpers only (NOT the SSE body — would kill a quiet stream);
   `get_bytes` panic-free (no `unwrap_err`). (`d761d71`)
3. **Bounded `sync_channel`** (1024) — blocking-send backpressure per impl-spec §6. (`79ab1e9`)
4. **`EventStream::stop()`** — cooperative shutdown; heartbeat-bounded; a parked blocking send
   is unblocked by dropping the `EventStream`. (`8d7c98d`)
5. **Bootstrap = reconnect parity** — first open emits `SnapshotRestored`+items (no
   `Reconnecting`/`Reconnected`) so the reducer is the single writer on first open too;
   `run` split into `bootstrap`+`read_loop`; typed-client §7 "Bootstrap" + app-arch §4.1
   reconciled. (docs `f66bc6f`, code `1d6c4a4`)
- Final-review fix wave: scoped `stop` into `bootstrap` + reconciled `stream()`/`SnapshotRestored`
  docs. (`8fe4dd5`)

**Gate:** 126 lib + offline `taxonomy_drift` + 2 xtask tests pass; clippy `--all-targets`/fmt
clean; `xtask drift` green (55 paths); `generated.rs` untouched; no `Value` to consumers.

## Deferred (tracked in STATUS "Deferred, with a clean seam")

- **#5 event-surface recapture (next, most consequential)** — `session.agent_changed`,
  `response.created`/`queued`, `turn.*` map to `Unknown` and are ABSENT from the golden corpus
  (claude-sdk emits none). Needs a live server + a harness that drives them; model from real
  bytes (decided), schema-model fallback if undrivable. Folds in the
  `ChildSessionUpdated`/`Terminal*` poke-only payload-loss family.
- Small hardening: `info.databricks_features: Value`; `ClientError::NotFound` rename + `Validation`/422;
  `gap==Some(0)` proof; `/items` pagination; gated live reconnect smoke; bootstrap-of-failed-session
  redundant `SnapshotRestored` (self-corrects, acceptable v1).
- Document for the reducer: two status (`SessionStatusValue`/`SessionStatus`) + two usage representations.
- WS terminal attach client (Plan 7).

## Next

Branch is ready to finish (merge/PR). After that, the natural next step is either the **#5 capture
spike** (when a live server + a multi-event harness are available) or the **state-model / gpui pump**
itself — which this hardening pass was meant to de-risk.
