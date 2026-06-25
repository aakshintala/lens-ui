# Tooling (reference)

Descriptive, not normative. The Rust workspace does not exist yet — command
placeholders are marked `TODO` and get filled when Cargo lands.

## Code intelligence

- **LSP:** `rust-analyzer`. Prefer LSP-driven navigation (defs, refs, types,
  diagnostics) over text search for semantic questions.
- **Tree-sitter / AST:** prefer AST-aware structural search and edits over regex
  for code-shaped changes; fall back to text tools for prose and config.

## Commands (TODO — fill when Cargo exists)

- Build: `cargo build` / release `cargo build --release`
- Test: `cargo test`
- Bench: `cargo bench` (criterion)
- Lint: `cargo clippy -- -D warnings`
- Format: `cargo fmt`

## Contract

- Ground truth: `vendor/omnigent-0.3.0.dev0/openapi.json` (pin: `OMNIGENT_PIN`).
- `lens-client` codegens + contract-tests against the vendored openapi.
