# Vendored omnigent contract

`openapi.json` is a verbatim copy of `omnigent`'s generated OpenAPI surface at the
pinned version.

- **Pin:** `0.3.0.dev0` (package semver — see `OMNIGENT_PIN`)
- **Source HEAD:** `36b2a11c` (`/Users/aakshintala/work/omnigent`)
- **Caveat:** the file's own `info.version` is a stale `"0.1.0"`. Trust the package
  semver / route source, not `info.version`.

This is the ground truth the `lens-client` crate codegens and contract-tests
against. Bumping the pin = drop in a new `openapi.json`, update `OMNIGENT_PIN`,
re-run codegen, fix contract-test failures. CI should diff this against the
sibling omnigent pin (path enumeration + SSE schema) so the contract can't
silently drift.

WebSocket terminal-attach paths are **not** in `openapi.json` — they live in
`omnigent/server/routes/terminal_attach.py` and are mounted under `/v1` by
`create_app` (`omnigent/server/app.py:1635-1642`).
