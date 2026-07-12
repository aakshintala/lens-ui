# Handoff — state-model P3-3b: PLAN WRITTEN + double-reviewed → next is EXECUTE — 2026-07-12

## TL;DR

**The P3-3b plan is written, twice adversarially cross-family reviewed, and all 15 findings
folded. No open design questions. Next session: EXECUTE it** via
`superpowers:subagent-driven-development` — composer-2.5 per task, grok cross-family review on
each MANDATORY seam. Do NOT re-plan or re-review at the plan level (two independent passes already
converged); the per-task seam reviews are the remaining quality gate.

- **Plan (execution SSOT):** `docs/superpowers/plans/2026-07-12-state-model-p3-3b.md`.
  Read it top to bottom — the "Pre-build review ledger" (2 tables) lists all 15 folded findings.
- **Decisions SSOT:** spec `docs/superpowers/specs/2026-07-08-state-model-engine-design.md`
  §2.4 (D24–D31) + live-verify appendix. App-arch amended: §3.5, §13.1.
- **Grill memory:** `state-model-p3-3b-grilling`. This session's memory:
  `state-model-p3-3b-planning` (the review-caught corrections).

## What this session did

1. Reviewed the P3-3b grilling handoff for readiness — found it sound but surfaced pins.
2. Decided: full calibrated planning (heavy for D24/D28/D30, light for D27/C4/tool);
   Opus authors (context locality + Opus-level sequencing); diversity via review not authorship.
3. Decided the `lens-drive` binary: **new crate** `crates/lens-drive` (sibling to `lens-capture`,
   NOT merged — capture=observe vs drive=actuate are opposite verbs; umbrella is premature at N=2
   and must never be named `lens`, which is the product binary). Headless JSON-lines driver;
   bright line = **dumps state, never renders**.
4. Wrote the plan (7 tasks). Ran TWO cross-family adversarial reviews and folded everything.

## The two review rounds (why the plan is trustworthy)

- **Round 1 — grok-4.5-xhigh** (pre-build, on T5/T6 sections): 10 findings. **Refuted TWO author
  gap-analyses:** (F4) omnigent `pending_inputs` DOES carry `content` — lens-client stripped it;
  D28 is genuinely THREE-way (widen `PendingInput`), not two-way. (F5) the SCHEMA_VERSION bump
  UPGRADES old files (not read-only) and `CREATE TABLE IF NOT EXISTS` won't add columns → needs a
  real `ALTER TABLE` migration. Plus F1 (store-frontier vs `next_ordinal` collision), F2 (catch-up
  delta not materialized), F3 (D30-folded re-fetch = false landed-evidence → silent drop), F6 (PK
  UPDATE unsafe if store id exists), F7/F9/F10.
- **Round 2 — gpt-5.5 via codex** (adversarial-verify the round-1 fixes + T1–T4): 5 findings, all
  refinements to the round-1 fixes (converging, not new foundations). **OK-VERIFIED the highest-risk
  question: D28 is NOT dead code under D24** (Path 1 = transient reader reconnect, actor alive;
  park=exit only on terminal Disconnected). Refinements: R2-1 (plumb snapshot `pending_inputs` to
  the hook — `fold_snapshot` discards it), R2-2+R2-4 (spec's FIFO-min-match is unsafe → **unique +
  temporal match, ambiguity → visible SendLost**), R2-3 (F6 refresh surviving row), R2-5 (atomic
  `mark_parked` — racy bookkeeping), R2-7 (migration runs unconditionally per open).

## Execution notes for the next session

- **Skill:** `superpowers:subagent-driven-development`. Fresh composer-2.5 subagent per task
  (via `cursor-delegate`), grok cross-family review on each **[MANDATORY seam review]** task
  (T1, T2, T5, T6) + the lens-client contract-surface changes (T6 `PendingInput`/`SnapshotRestored`).
  Opus inline for architecture calls + final synthesis. Per CLAUDE.md.
- **Task order + calibration** is in the plan header table. T4 (`lens-drive`) is deliberately
  numbered after T1/T2 so it targets the final outcome shapes; there is no T3.
- **Green bar every task:** `cargo test -p lens-core` (+ `-p lens-drive` from T4), `cargo clippy
  --all-targets -- -D warnings`, `cargo fmt --check`. Zero warnings/dead-code = the integration gate.
- **Spec back-edits are a T7 deliverable:** app-arch §4.1/§6.2 (D30 amendment the spec §7.1 flagged
  as not-yet-applied) AND spec §2.4 D28 (correct the FIFO-min-match wording to unique+temporal, with
  the review rationale) AND the `state-model-p3-3b-contract-gaps` memory.
- **Live-verify riders** (T4 D24/D27, T5 D30) need a running omnigent 0.5.1 — use
  `installing-omnigent-from-source`; a probe server may or may not still be up from the grilling
  session (`omnigent stop` also kills the host daemon — relaunch host manually).
- **Deferred, do NOT build:** `lens-ui` (Bucket B viewport + arch-B composer-draft); omnigent
  **client-message-id** contract request (the robust idempotency fix D28's content-match approximates);
  C5 command-ordering; opportunistic provisional-promotion; frontier-anchoring.

## Integration

Solo project (memory `integration-workflow`): each task commits to `main` when green; the plan's
T5 splits D30/C3/C4 into 3 commits (bisectability, F7). No PRs. Push is a separate manual call.
