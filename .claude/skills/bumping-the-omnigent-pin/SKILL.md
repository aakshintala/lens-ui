---
name: bumping-the-omnigent-pin
description: Use when a new omnigent release ships (omnigent tags ~weekly — v0.3.0, v0.3.1…) and Lens must advance its vendored contract pin, when `xtask drift` reports drift vs the sibling checkout, or when re-vendoring openapi.json + re-running codegen + fixing lens-client for a new omnigent version.
---

# Bumping the omnigent pin

Advance Lens's vendored omnigent contract to a new release tag: re-vendor
`openapi.json`, re-codegen `generated.rs`, fix the lens-client fallout, re-ground
docs, and re-verify the gate. A recurring ~weekly chore (ADR-0001).

This is a runbook, not a discipline rule. Work top-to-bottom; the gate at the end
is non-negotiable (AGENTS.md: tests + clippy + fmt + drift all clean).

## Before you start

- Sibling checkout at `../omnigent`, fetched (`git -C ../omnigent fetch --tags`).
- Know the current pin: `vendor/omnigent-<ver>/OMNIGENT_PIN` + README Source HEAD.
- **Pin to the release TAG, not `main`.** Annotated tags peel: the real commit is
  `git -C ../omnigent rev-parse v0.3.0^{commit}` (plain `rev-parse v0.3.0` gives
  the tag object). `openapi.json`'s own `info.version` is a stale `0.1.0` — ignore
  it; trust the package semver (`pyproject.toml` at the tag) + route source.

## Steps

1. **Checkout the tag** (detached HEAD expected): `git -C ../omnigent checkout vX.Y.Z`.
   Confirm the package semver: `grep '^version' ../omnigent/pyproject.toml`.
2. **Preview the delta** from the OLD pin, *before* vendoring:
   `cargo run -p xtask -- drift` (diffs old vendored vs `../omnigent/openapi.json`).
   Read every line — see "Interpreting drift" below. Then predict codegen breakage
   with a schema-set diff (old vs new `components/schemas` keys; a `python3 -c` json
   diff). Dropped schemas that lens-client *source* references are the real work.
3. **Vendor**: `mkdir vendor/omnigent-X.Y.Z`, copy `../omnigent/openapi.json` in,
   write `OMNIGENT_PIN` (the semver) + `README.md` (tag, Source HEAD commit, the
   stale-`info.version` caveat, and the hidden-routes note).
4. **Repoint paths** to the new dir: `xtask/src/main.rs` (`SPEC`, the `SKIPPED.md`
   path, the generated-header string) and `tests/taxonomy_drift.rs`. Grep
   `vendor/omnigent-` to catch them all.
5. **Codegen**: `cargo run -p xtask -- codegen` (rewrites `generated.rs` + its header).
6. **Build & fix** (`cargo build -p lens-client`) — the four recurring breakages:
   - **New regex dep.** A new schema with a `pattern` makes typify emit
     `::regress::Regex`. Add `regress = "0.10"` (typify's companion; already in
     `Cargo.lock`) to `lens-client/Cargo.toml`.
   - **Dropped hidden-route schemas.** A route flipped to `include_in_schema=False`
     drops its body schema from openapi (e.g. `SessionEventInput`, `ElicitationResult`).
     If lens-client *source* uses the generated type, hand-author it in lens-client —
     it's still live contract (ADR-0001). If only a comment references it, no-op.
   - **New SSE event(s).** For each added `ServerStreamEvent` member: model it in
     `stream/event.rs` (enum variant + `Raw*` deser struct + parse arm) **and** add
     the wire type to `MODELED_EVENT_TYPES` (or `DEFERRED_EVENT_TYPES` if punting) —
     the offline `taxonomy_drift` test fails until mapping == MODELED ∪ DEFERRED.
   - **Version gate.** Bump `PINNED_OMNIGENT_VERSION` in `lib.rs` to the new semver
     (exact-match gate in `http.rs::check_contract`) + the version literals in the
     `http.rs` / `error.rs` unit tests.
7. **Re-ground docs**: version/path strings in `AGENTS.md`, `.agents/principles.md`,
   `.agents/tooling.md`, and the `installing-omnigent-from-source` skill. Leave golden
   test fixtures (`tests/fixtures/**`) alone — captured bytes, not docs.
8. **Drop the old vendor dir** (`rm -rf vendor/omnigent-<oldver>`; git tracks history).
9. **Gate** (all must be clean): `cargo fmt --all --check`, `cargo clippy --all-targets`,
   `cargo test -p lens-client && cargo test -p xtask`, `cargo run -p xtask -- drift`
   → "no drift".
10. **Update the installed server** so local runs + live-tests exercise the new
    contract, not the stale one (`omnigent --version` otherwise lags the pin). The
    checkout is already on the tag from step 1; run `installing-omnigent-from-source`:
    `uv tool uninstall omnigent && uv tool install --editable ../omnigent`, confirm
    `omnigent --version` shows the new commit, and restart any running daemon
    (`omnigent server status && omnigent server stop && omnigent server start`).
11. **(Optional) live-verify**: run the gated `live-tests` (handshake + `live_taxonomy`
    + `live_reachability`) against the updated server.
12. **Document**: update `docs/STATUS.md` + memory.

## Interpreting drift

`xtask drift` diffs openapi **presence**. Its "upstream dropped" lines are the
trap: a route marked `include_in_schema=False` is **hidden, still live** — not
removed (ADR-0001; this is how elicitation/events/transfer routes behave). Before
treating any "dropped" path as a breaking change, confirm against
`omnigent/server/routes/*.py` (grep the path; look for `include_in_schema=False`).
Genuine additions and genuine SSE-shape changes are real; "removals" are
guilty-until-proven.

## Common breakages

| Symptom | Cause | Fix |
|---|---|---|
| `could not find regress` in `generated.rs` | new schema has a `pattern` | add `regress = "0.10"` dep |
| `cannot find type X in crate::generated` | hidden-route body schema dropped | hand-author X in lens-client |
| `taxonomy_drift` fails | new SSE type unaccounted | add to MODELED or DEFERRED + model it |
| live handshake `ContractMismatch` | `PINNED_OMNIGENT_VERSION` stale | bump to new semver + test literals |
| drift "upstream dropped /…" | route now `include_in_schema=False` | verify in route source; expected, not a removal |
