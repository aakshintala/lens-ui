# Vendored omnigent contract

`openapi.json` is a verbatim copy of `omnigent`'s generated OpenAPI surface at the
pinned version.

- **Pin:** `0.3.0` (package semver — see `OMNIGENT_PIN`)
- **Source tag:** `v0.3.0`
- **Source HEAD:** `4edb4d95` (`/Users/aakshintala/work/omnigent`)
- **Caveat:** the file's own `info.version` is a stale `"0.1.0"`. Trust the package
  semver / route source, not `info.version`.

This is the ground truth the `lens-client` crate codegens and contract-tests
against. Bumping the pin = drop in a new `openapi.json`, update `OMNIGENT_PIN`,
re-run codegen, fix contract-test failures.

**Hidden-but-live routes (`include_in_schema=False`).** `0.3.0` moved several
internal/runner-facing routes out of the public OpenAPI reference. They are
**still live contract** (ADR-0001), just absent from this file:
`POST …/events`, `…/elicitations/{eid}` + `/resolve`, `…/resources/terminals/{tid}/transfer`,
`…/resources/environments/{eid}/diff/{path}`, `…/mcp`. Their request/response
schemas (`SessionEventInput`, `ElicitationResult`) consequently drop out of
`components/schemas` even though the wire shape is unchanged — `lens-client`
hand-authors those types where it wraps the hidden routes. `xtask drift`'s
"upstream dropped" lines for these paths are **expected false alarms** against a
hidden route; verify against route source before treating one as a real removal.

**Drift check (Plan 3c):** `cargo run -p xtask -- drift` diffs this file against
the sibling pin (default `../omnigent/openapi.json`; override with `--against <path>`)
— path enumeration (excluding `/hooks/*` runner callbacks) + SSE discriminator/shape
— and exits non-zero on drift. The ADR-0001 "passive alarm," run locally. The
offline `cargo test` (`taxonomy_drift`) additionally fails if the SSE event taxonomy
gains/loses a type vs this file.

WebSocket terminal-attach paths are **not** in `openapi.json` — they live in
`omnigent/server/routes/terminal_attach.py` and are mounted under `/v1` by
`create_app` (`omnigent/server/app.py`).
