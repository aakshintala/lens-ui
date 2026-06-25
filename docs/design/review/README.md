# Lens design-spec review

A multi-model review of the 11 design specs in `docs/design/`, grounded against the
sibling omnigent source (`/Users/aakshintala/work/omnigent`, HEAD `36b2a11c`, package
`0.3.0.dev0`, `openapi.json` OpenAPI 3.2.0, 59 REST paths).

## How to read this

1. **Start with [`_SYNTHESIS-opus.md`](./_SYNTHESIS-opus.md)** — the master cross-cutting
   report (exec summary, 20 root-caused systemic themes, A–J compliance table, 8
   adjudicated conflicts, prioritized master list of **11 blockers + 21 majors**, and a
   6-phase fix plan). This is the authoritative deliverable.
2. **[`_SECOND-OPINION-gpt55.md`](./_SECOND-OPINION-gpt55.md)** — independent verification
   table with source citations + corrections to the bulk reviewers.
3. **[`_grounding-baseline.md`](./_grounding-baseline.md)** — the 59 enumerated REST paths,
   harness registries, and version facts used as ground truth.
4. **Per-doc findings** — the detailed bulk pass, one file per doc.

## Process

| Pass | Model | Output |
|---|---|---|
| Bulk per-doc (×8) | `composer-2.5-fast` | the 8 `*.findings.md` files |
| Cross-cutting synthesis | `claude-opus-4-8-thinking-high` | `_SYNTHESIS-opus.md` |
| Independent second opinion | `gpt-5.5-medium` | `_SECOND-OPINION-gpt55.md` |

Dimensions: openapi grounding · cross-doc consistency (decisions A–J) · completeness ·
clarity · technical feasibility.

> Note: `composer-2.5` (non-fast) was requested but is unavailable in this harness;
> `composer-2.5-fast` was used for the bulk pass.

## Per-doc findings

- [`typed-client.findings.md`](./typed-client.findings.md)
- [`app-architecture-and-state-model.findings.md`](./app-architecture-and-state-model.findings.md)
- [`application-shell-and-layout.findings.md`](./application-shell-and-layout.findings.md)
- [`conversation-transcript.findings.md`](./conversation-transcript.findings.md)
- [`capability-map-and-design-language.findings.md`](./capability-map-and-design-language.findings.md)
- [`workspace-and-server-lifecycle.findings.md`](./workspace-and-server-lifecycle.findings.md)
- [`agent-definition-and-sub-agent-topology.findings.md`](./agent-definition-and-sub-agent-topology.findings.md)
- [`permissions-and-framework.findings.md`](./permissions-and-framework.findings.md)

## Headline resolutions (adjudicated against source)

- **Harness count = 19** canonical (`OMNIGENT_HARNESSES`, `_omnigent_compat.py:80-101`); spec's "16" is stale. Lens picker should use the 19; `harness` is now a free `string|null`.
- **Version gate is on the wrong endpoint** — `/v1/info` has no version; semver is at `GET /api/version`. (Wrong in 3 docs.)
- **switch-agent requires `LEVEL_EDIT`, not owner** (`sessions.py:14214`). "Owner-only verified in source" is false in 4 docs; owner-only is fine as a *Lens UI policy*. Idle guard rejects `waiting` but **not** `launching`.
- **`session.status` is 3-state over REST, 5-state over SSE** — cards/poll mapping must account for this.
- **`pending_elicitations` is plural on the wire** — the state model's singular `Option` can't model child fan-out.
- **`PresenceViewer` fields are invented**; **grant levels are 1–3** not 1–4; **terminal WS path needs the `/v1` prefix**.
- **Verification posture is broken** — no `openapi.json` and no GPUI recon artifact are vendored in this repo despite README claims; stated pin (`0.2.0`) lags source (`0.3.0.dev0`).

## Decision A–J status

A, B, C, G honored (C with a server/runner-restart caveat) · D, F partial (stale "gated"/"right rail" labels + missing recon) · **E, H, I, J have real violations** (sharing-UI scope contradiction; Bridge placement still "open"; spend readout not in chrome; switch-agent mis-grounded).
