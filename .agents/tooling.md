# Tooling (reference)

Descriptive, not normative. Reflects the live workspace (`Cargo.toml` at repo
root, edition 2024, resolver 3, toolchain pinned in `rust-toolchain.toml`).

## Code intelligence

- **LSP:** `rust-analyzer`. Prefer LSP-driven navigation (defs, refs, types,
  diagnostics) over text search for semantic questions.
- **Tree-sitter / AST:** `ast-grep` (`ast-grep run -l rust -p '<pattern>'`).
  Prefer AST-aware structural search and edits over regex for code-shaped
  changes; fall back to text tools for prose and config.

## Commands

- Build: `cargo build` / release `cargo build --release`
- Test: `cargo test`
- Bench: `cargo bench` (criterion)
- Lint: `cargo clippy -- -D warnings`
- Format: `cargo fmt`

## Workspace layout

- `crates/*` — production crates; opt into the lint bar with
  `lints.workspace = true`.
- `spikes/*` — throwaway probes; deliberately do **not** opt into
  `[workspace.lints]` and are not held to the definition-of-done. That opt-in is
  the throwaway/production wall.

## Running the omnigent server

Must be the pinned source build — see the `installing-omnigent-from-source`
skill. Two modes:

- **Background (what `run`/`claude`/clients use):** `omnigent server start` — runs
  detached on an **ephemeral port** (not 6767). Discover it with
  `omnigent server status`; never hardcode the port. `omnigent server stop` tears
  down the server + local host daemon.
- **Foreground:** bare `omnigent server` — binds `127.0.0.1:6767` (`-p` to
  override), Ctrl-C to stop. For deploys/Docker.

**Gotcha — stale daemon after a reinstall:** a running background daemon keeps the
code it *started with* in memory. After `uv tool install --editable` you must
`omnigent server stop && omnigent server start`, or it keeps serving the old
version (observed: a 0.2.0 daemon serving 57 paths after the package was already
0.3.0.dev0). Verify the live contract matches the pin by path set, not
`info.version` (which is a stale `0.1.0`):

```bash
PORT=$(omnigent server status | sed -n 's/.*port \([0-9]*\).*/\1/p')
curl -s "http://127.0.0.1:$PORT/openapi.json" \
  | python3 -c "import sys,json; print(len(json.load(sys.stdin)['paths']), 'paths')"   # expect 59
```

Default store: sqlite at `~/.omnigent/chat.db` (machine-global; `server` and `run`
share it).

## Contract

- Ground truth: `vendor/omnigent-0.3.0/openapi.json` (pin: `OMNIGENT_PIN`).
- `lens-client` codegens + contract-tests against the vendored openapi.
