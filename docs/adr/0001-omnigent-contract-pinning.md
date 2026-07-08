---
status: accepted
date: 2026-06-25
---

# Pin the omnigent contract to a commit, advance per feature-slice

## Decision

Lens keeps **vendoring** the omnigent contract, with three refinements:

1. **The canonical pin is the source commit, not the version string.** The
   package version (`0.3.0.dev0`) is a moving dev label that does not bump per
   commit, so it cannot anchor anything. `vendor/omnigent-<ver>/README.md`'s
   **Source HEAD** is the real pin. `openapi.json` is a *generated convenience
   artifact* derived from that commit — not the sole ground truth.
2. **Advance the pin per feature-slice, not in batches.** Re-vendor when we
   start building a surface (transcript, terminals, elicitations, …) so that
   slice is built against current truth. Do not chase HEAD; do not re-vendor on
   a fixed clock.
3. **Lock to official release tags going forward; `0.3.0` is the first one.**
   Upstream has an active release cadence (`v0.1.0 → v0.1.1 → v0.2.0`, each with
   rc's; `v0.2.0` shipped ~2026-06-18). The moment `0.3.0` tags, that becomes the
   pin and we stop tracking dev HEAD. Until then the current `0.3.0.dev0` commit
   pin (`36b2a11c`) is the bridge — **do not regress to `v0.2.0`**, which is an
   *older* contract than the current dev pin (different SSE schemas, missing
   routes).

## Context

omnigent ships releases (`v0.1.0 → v0.1.1 → v0.2.0`, with rc's), but the latest
tag `v0.2.0` is a **different, older** contract than the unreleased `0.3.0.dev0`
work we are grounded on — building against `v0.2.0` would regress. So
release-pinning is the committed end-state, gated on `0.3.0` actually tagging;
until then we bridge on a frozen `0.3.0.dev0` commit.

The trigger for this ADR was a 74-commit gap (pin `36b2a11c` → HEAD) that *looked*
catastrophic — `openapi.json` diffed +4082/−1945 with elicitation/permission/
hook routes apparently deleted. Investigation showed the capability was **fully
intact**: commit `e182b050` (#1249) merely marked those runtime routes
`include_in_schema=False`, hiding them from the published reference. Measuring
churn by `openapi.json` line count badly overstates real contract churn, and
proves `openapi.json` was **never the whole client contract**. Hence the commit,
not the spec file, is canonical.

> **Naming note (2026-07-06):** the first-party client dir was renamed
> `ap-web/` → `web/` upstream (PR #1333). References to `ap-web` below (and the
> "does `ap-web` call it?" heuristic) now mean `web/`; paths like
> `sessionsApi.ts` moved accordingly.

`include_in_schema=False` is not a single signal — it splits by caller:

- `/hooks/{permission,codex,antigravity,cursor,native}-*-request` are
  **runner→server callbacks** (posted by `*_native_hook.py` /
  `*_native_permissions.py`). `ap-web` never calls them. These are **not client
  API**; lens must ignore them. Hidden here means "internal plumbing."
- `/sessions/{id}/elicitations/{id}[/resolve]` are the **human-facing**
  read/resolve surface. `ap-web` calls them at HEAD (`sessionsApi.ts`,
  `pages/ApprovePage.tsx`). Hidden here means "kept out of the public reference
  docs," **not** "deprecated" — they are the live client contract.

The discriminator for whether lens depends on a non-public route is therefore
**"does the first-party client (`ap-web`) call it?"** — not "does it exist in
source." `openapi.json` is the *public subset* of the client contract; `ap-web`
is the *executable spec* for the whole client surface. WS channels
(`terminal_attach`, `/sessions/updates`) sit outside `openapi.json` for a
different reason — OpenAPI cannot express WebSocket routes at all (a tooling
limit, not a hiding choice).

The pain was never *vendoring* — it was letting drift accumulate into one
big-bang advance instead of a continuous small-diff process.

## Considered options

- **Generate live from the sibling `../omnigent` checkout at build time.**
  Rejected: makes builds depend on mutable external state (checkout HEAD + dirty
  tree), non-reproducible across machines/CI, and breaks the AGENTS.md
  "cite the pinned contract" discipline. CI would force us to re-invent
  vendoring anyway.
- **Track HEAD / rebuild against master daily.** Rejected *for now*: buys
  small-increment breakage but costs continuous maintenance on a client with no
  code yet — we'd fix codegen breaks for surfaces lens doesn't consume. The
  value is the *signal*, not forced *action* (see Consequences).
- **Anchor to released/stable versions only.** Adopted as the forward policy,
  gated on `0.3.0` tagging. Cannot anchor to the current latest release `v0.2.0`
  — it predates the `0.3.0.dev0` contract lens is grounded on.

## Consequences

- The vendored contract must capture the **out-of-band surface lens actually
  depends on** — the hidden-but-`ap-web`-used elicitation read/resolve routes
  plus the WS channels — documented alongside `openapi.json`. It must **not**
  include the runner-side `/hooks/*-request` routes; those are not client API.
  When in doubt about a non-public route, the test is "does `ap-web` call it?"
- A re-vendor is an **owned task**: `git pull` the checkout, regenerate
  `openapi.json`, bump README Source HEAD, re-run the drift check, fold in any
  new capability (e.g. `native-permission-request`, `agent/mcp-servers`).
- **When CI exists**, add the drift-check the vendor README already calls for
  (vendored vs sibling HEAD, path enumeration + SSE schema). This converts
  "rebuild daily" from manual toil into a *passive alarm* — divergence on
  surfaces we consume becomes visible without forcing action on churn we don't.
- Flipping to release-pinning later changes the pin string in ~10 places across
  the docs (tracked in the spec-review handoff judgment calls).
