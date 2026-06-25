# Handoff — spec-review open decisions

**Date:** 2026-06-25
**Context:** Resolved the autonomous findings from the design-spec review
(`docs/design/review/`) — all grounding/consistency blockers + majors are applied
across the 13 design docs, and `openapi.json` is vendored at
`vendor/omnigent-0.3.0.dev0/`. This doc captures what's **left for a human to
decide** — items that can't be closed by grounding against omnigent source.

Ground truth used: omnigent HEAD `36b2a11c`, package `0.3.0.dev0`.

---

## Needs discussion (genuine decisions)

### 1. Archive semantics — Lens local-hide vs server `archived` (M8 / T8)
The server has its own `archived: bool` on the session snapshot/list, toggled via
`PATCH /v1/sessions/{id}` and filtered by `include_archived` on `GET /v1/sessions`
— **independent** of `stop_session` (server archive does NOT stop the session).
Lens currently overloads "Archive" to mean *local drawer-hide + `stop_session`*.
On a multi-client fleet the two diverge.

- **Option A:** mirror the server `archived` flag via `PATCH` on the Archive
  action (single source of truth; honor `include_archived`/`kind`/`search_query`
  poll filters).
- **Option B:** rename the Lens field to `hidden_in_drawer` and stop overloading
  "archived."

Flagged inline at `app-architecture-and-state-model.md` §3.2. Decide A vs B (an
ADR would help — offered to draft one).

### 2. Fleet menu-bar badge — poll vs push (C7)
`WS /v1/sessions/updates` exists in omnigent source/tests but is **absent from
openapi** (correctly excluded from the 59 REST paths). Lens can stay poll-only for
v1 or adopt the push channel for the menu-bar/fleet badge. Currently the specs are
poll-only but the decision is left implicit. **Decide:** poll-only v1, or wire the
push channel? Make it explicit in typed-client + shell §17.4 + state-model §10.

### 3. Global spend rollup — algorithm + retention (M7 / decision I)
Decision-I chrome is resolved (per-card/project cumulative from `total_cost_usd`;
global today/7d/30d windowed). **Unresolved:** the cross-connection aggregation
algorithm and the `cost_samples` retention/bucketing policy (how long samples are
kept, how windows are bucketed, behavior across connections). State-model §6.2
owns it; needs a product + data-model call. (`None`-cost behavior is now specified:
show `—`/"unpriced", never `$0.00`.)

### 4. gpui spikes — un-spiked engineering risk (M19 / decision D)
Cannot be closed by doc edits — they need actual spikes before locking the
framework substrate:
- **Variable-height transcript virtualization** — `uniform_list` is uniform-height
  only; the transcript is variable-height. Needs a custom virtualizer (or `list`).
- **Incremental/streaming markdown** — `pulldown-cmark` → gpui element renderer
  with safe-prefix streaming; may force a gpui fork (Paneflow's path).
- **JSON-Schema elicitation form renderer** — hand-rolled; un-spiked.
Fallback ladders are written into `framework.md` §4.1/§4.3, but the spikes
themselves are outstanding.

### 5. Contract-drift CI (B6 / Phase 0)
`openapi.json` + an `OMNIGENT_PIN` file are vendored. **Still needed:** a CI job
that diffs the vendored copy (path enumeration + SSE schema) against the sibling
omnigent pin so the contract can't silently drift. Needs your CI. Also decide:
does the pin **track a moving `0.3.0.dev0`** or **freeze a specific commit**?

---

## Judgment calls made — sanity-check these

- **Vendored the full 8,100-line `openapi.json`** into the repo
  (`vendor/omnigent-0.3.0.dev0/`). Confirm that belongs in a docs repo.
- **Pin string `0.3.0.dev0`** used everywhere (package semver; the file's own
  `info.version` is a stale `0.1.0`). If you intend to pin a tagged release, the
  string changes in ~10 places across the docs.
- **Applied the synthesis's prescribed modeling**, not just facts: `Vec`
  pending-elicitations, three-bucket reconnect, persisted lifecycle columns,
  synthesized `AgentChanged.from`. These are the review's recommendations — flag
  any you'd model differently.
- **Bridge board-only entry (M6):** assessed as already consistent in the current
  `application-shell-and-layout.md` §10.1 (`scope=all` → working-area tab that
  shrinks the board). No edit made — confirm that's the intended behavior.
- **Recon artifact (D):** downgraded "recon retired most widget risk" language and
  clarified `framework.md` §2 *is* the recon record (no separate file). If you
  want a standalone vendored recon artifact, that's a separate task (I didn't have
  the raw artifact).

---

## Pointers
- Authoritative review: `docs/design/review/_SYNTHESIS-opus.md` (master list,
  6-phase fix plan).
- All blockers B1–B11 and majors M1–M21 grounding fixes are **applied**; what
  remains is only the 5 decisions above.
