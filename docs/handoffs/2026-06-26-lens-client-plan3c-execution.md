# Handoff — Plan 3c (contract-drift CI / B6) execution

**Date:** 2026-06-26
**Branch:** `feat/lens-client-streaming`
**Commits:** `087ef6f..8a7bb2e` (5 tasks + 2 live-caught fixes + 1 review fix)
**Plan:** [`2026-06-26-lens-client-plan3c-contract-drift.md`](../plans/2026-06-26-lens-client-plan3c-contract-drift.md)
**Mode:** subagent-driven (composer-2.5 build + Opus per-task review + one consolidated gpt-5.5 cross-family review)

## What shipped

The outstanding **B6** contract-drift "passive alarm," in three layers split by what each needs:

1. **`xtask drift`** (`087ef6f`, `7769a11`) — `cargo run -p xtask -- drift` diffs the vendored
   `openapi.json` vs a sibling pin (default `../omnigent/openapi.json`, `--against <path>` to
   override). Semantic: path-set diff excluding `/hooks/*` runner callbacks (ADR-0001) + SSE
   `ServerStreamEvent.discriminator.mapping` event-type diff + per-member property-name shape
   diff. Non-zero exit on drift. Verified **green vs the byte-identical sibling** and **red vs a
   synthetic fixture** (`crates/xtask/tests/fixtures/drifted-openapi.json`).

2. **Offline taxonomy-completeness** (`f68b9aa`, strengthened in `8a7bb2e`) — always-on
   `cargo test` (`crates/lens-client/tests/taxonomy_drift.rs`): the pinned openapi's
   `ServerStreamEvent` discriminator mapping must set-equal `MODELED_EVENT_TYPES` (33) ∪
   `DEFERRED_EVENT_TYPES` (14), and the two are disjoint. A new upstream event type fails here
   with **no server**. `parse_event` (event.rs) is the SSOT for which types are modeled.

3. **Gated live checks** (`df38779`, `b4c3a0f`, strengthened `8a7bb2e`) — `--features live-tests`,
   against a daemon (`LENS_OMNIGENT_URL` + claude-sdk `LENS_OMNIGENT_SESSION_ID`):
   - `live_taxonomy` — drive a turn; every wire event type must be modeled, or a **deferred** type
     legitimately surfacing as `Unknown`. A **modeled** type arriving as `Unknown` = drift.
   - `live_reachability` — ping every consumed read endpoint once via its typed method; reachable =
     typed `Ok` or a typed domain error, never transport/decode. (`ClientError::is_transport` /
     `is_decode` predicates added.)

**CI surface = local `xtask` only** (design D3). No `.github/workflows` — drift needs the sibling
checkout a hosted runner lacks.

## Live run (executed this session)

Server stood up via the `installing-omnigent-from-source` skill: omnigent `0.3.0.dev0` @ `36b2a11c`
at `http://127.0.0.1:6767`, ready ladder green. A runner-bound claude-sdk session was created via
`omnigent run --harness claude-sdk --server <url>` (held-open stdin keeps the local runner warm).

- **`live_reachability`: GREEN** — all 9 endpoints reachable (`/v1/sessions/{id}/resources` is a
  typed HTTP 409 "not bound to a runner" → correctly classified reachable).
- **`live_taxonomy`: GREEN** — real turn, saw `response.completed`, no modeled-as-`Unknown`, no
  non-deferred `Unknown`.

### Two real pre-existing bugs the reachability sweep caught (and we fixed)

- **`b5e4dad` — `HostObject` field name.** Deserialized `id` from wire key `id`, but `/v1/hosts`
  (HTTP 200) keys hosts by `host_id` (single-host GET too). `/v1/hosts` is openapi-untyped
  (free-form `{hosts:[obj]}`), so the live bytes are the contract. Also dropped a phantom `object`
  field/getter (not on the wire, no consumer). This was the 2e-deferred minimal wrapper whose field
  names were never byte-verified.
- **`d788e84` — `SessionSnapshot` null-collection intolerance.** The server emits explicit `null`
  (not `[]`/`{}`) for empty collections; `#[serde(default)]` covers a *missing* key but not a
  present `null`. `labels`/`usage_by_model`/`skills`/`items` now use a `de_null_default` helper (+
  null-tolerant `de_items`) + regression test. Resolves the 3b-2a-deferred `last_task_error`-class
  null ambiguity for these fields.

## Cross-family review (gpt-5.5, consolidated, ~$0.79)

**DON'T-MERGE → resolved.** 1 Important + 1 Minor.
- **Important (fixed, `8a7bb2e`):** `live_taxonomy` allowed `Unknown` for any *accounted* type, so a
  **modeled** event degrading to `Unknown{modeled type}` on payload drift would pass silently. Fixed
  by splitting `ACCOUNTED_EVENT_TYPES` → `MODELED`(33) + `DEFERRED`(14); only deferred types may be
  `Unknown`. Re-verified live. The offline test now also asserts the split is disjoint.
- **Minor (documented, not fixed — plan-scoped):** `xtask drift` member-shape diff compares property
  *names* only (not requiredness/type/nullability/nested). Deliberate scope bound; deepen to a
  canonicalized subtree compare if a field's *type* (vs presence) ever needs guarding.

Per `[[review-spend-policy]]`, the 2nd paid pass was forgone — the controller verified the fix
matches the reviewer's prescribed remedy and re-ran it live.

## Deferred / flagged

- `xtask drift` member-shape diff is property-names-only (above).
- `ResourceList` live decode not exercised (the idle test session wasn't runner-bound; resources
  returned a typed 409). Exercise when a runner-bound session with resources is available.
- The MODELED/DEFERRED consts are hand-maintained alongside `parse_event`; the offline test guards
  the union/disjointness against the contract, and review guards the modeled/deferred split — but
  promoting a deferred type to modeled is a two-site edit (move the const entry + add the dispatch
  arm).

## Verification

122 `lens-client` lib tests + 2 `xtask` unit tests, clippy `--all-targets` + rustfmt clean,
`generated.rs` untouched, no `serde_json::Value` exposed to consumers. Both gated live tests green
vs pinned `0.3.0.dev0`.

## Where next picks up

**Plan 3 / B6 is closed.** Remaining branch-level open threads are unchanged: WS terminal attach
(§5), the markdown renderer build risk, and the verification-pass tunables. The branch
`feat/lens-client-streaming` (off `main` @ `78fdaa3`) is ready to finish (merge/PR).
