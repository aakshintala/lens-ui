# Handoff — Terminal VT via libghostty-rs (2026-07-15)

> **✅ UPDATE (2026-07-15, later same-day session): Tasks 1 + 2 + the doc-supersede
> are DONE** — commits `ae1f385`/`014f9a9`/`e155230` on `terminal-ws` (unpushed).
> **One reversal:** the Ghostty source is **crates-only, NOT vendored** (its ~57-152M
> tree deferred to the CI trigger; pin stays in `build.rs`). Wiring notes (EXCLUDE-not-
> member for cap-lints, `ZIG` override) in memory [[terminal-vt-vendored-executed]] +
> `vendor/libghostty-rs/README.md`. **Remaining = Tasks 3-4 (the design pass).** Current
> SSOT is `docs/STATUS.md`; the checklist below is retained as historical record.

Resume artifact for the shared terminal workstream after this session's architecture pivot.
Read memories `[[terminal-vt-adoption-model]]` + `[[zig-ghostty-macos26-scissor]]` first — they hold the full decision rationale. This doc is the "start executing" checklist.

## Decided model (one line)

**Vendor [libghostty-rs](https://github.com/Uzaaft/libghostty-rs) + build from source** (patched `zig@0.15`) against a **pinned Ghostty DEV commit**, then build a GPUI render layer + omnigent PTY-attach on its safe `Terminal`/`RenderState`/`Cell` API. No shim, no source port, no prebuilt. Validated on macOS 26.6 (builds in 33s, 29 tests pass, drives a real terminal).

## Exact pins / environment (verified this session)

- **libghostty-rs** validated at rev `46a9d2ac941ed600cf43c5e6299c8dfd1d3a1ef0` (tag `release: 0.2.1`, 2026-07-15). MIT OR Apache-2.0. Workspace: `crates/libghostty-vt-sys` (raw FFI, **bindings checked in** at `src/bindings.rs` — no bindgen/libclang at build time) + `crates/libghostty-vt` (safe API).
- **Ghostty dev** pinned by libghostty-rs `build.rs`: `a887df42c56f6de86c0fe6da9c4eeca37931e083` (2026-07-11). This is where the terminal C API lives (`terminal.h`/`screen.h`/`render.h`/`grid_ref*.h`); the v1.3.1 release `vt.h` is parsers-only.
- **Zig**: patched Homebrew `zig@0.15` = `0.15.2` at `/opt/homebrew/opt/zig@0.15/bin/zig` (keg-only). **Must** be on PATH for the build — vanilla ziglang.org Zig ≤0.15.2 fails to link on macOS 26 (ziglang/zig#31658). Install: `brew install zig@0.15`.
- **Build knobs** (libghostty-rs `-sys/build.rs`): `GHOSTTY_SOURCE_DIR` (point at a pinned local Ghostty tree → no runtime fetch), `LIBGHOSTTY_VT_SYS_OPTIMIZE`, static link of `ghostty-vt`. Built `.a` ≈ 12 MB (debug).

## Repo state (branch `terminal-ws`)

This session committed `73738d5..35299e4` (8 commits):
- `73738d5` WP0 plan + Opus review record — **for the DEAD source-port model**.
- `db5a0b4`/`bf721f1`/`354d405` xtask verifier + CLI (17 tests) — **dead model** (pin-agnostic mechanism only).
- `d9b2194`/`5bb16ec` archive hashes + 45/742 adoption inventory — **dead model**.
- `1d813a3`/`35299e4` STATUS pivot + final-model — KEEP.

## First tasks in the new session

1. **Discard the dead WP0 artifacts** (they audit a model we abandoned):
   - `rm -rf vendor/gpui-ghostty-e3025981/`
   - remove `crates/xtask/src/terminal_provenance.rs`, `crates/xtask/src/lib.rs` (the terminal-provenance mod), the `terminal-provenance` CLI wiring in `crates/xtask/src/main.rs`, `crates/xtask/tests/terminal_provenance.rs` + `tests/fixtures/terminal-provenance/`, `scripts/generate-terminal-adoption.sh`, and the WP0 plan + review docs under `docs/plans/2026-07-15-terminal-wp0-provenance*`. Revert the `xtask` `Cargo.toml` deps added for it (serde/toml/thiserror/sha2) if unused elsewhere. Keep `xtask`'s pre-existing codegen/drift/gate commands working; run `cargo test -p xtask` + workspace clippy after.
2. **Vendor the VT dependency**: copy libghostty-rs's two crates (at rev `46a9d2ac`) into the workspace (e.g. `vendor/libghostty-rs/` or `crates/`), + a pinned Ghostty dev source tree (commit `a887df42`) for `GHOSTTY_SOURCE_DIR`. Record provenance (both upstream URLs+commits, licenses: MIT/Apache + Ghostty MIT + ziglyph). Wire the build (PATH to `zig@0.15`; consider a `.cargo/config.toml` or an `xtask` helper) and **verify a Lens crate builds+links it** (feed bytes → read a cell).
3. **Design + build the GPUI render layer** on `RenderState`/`Row`/`Cell` (grid → GPUI paint), reusing the transcript-virtualization + markdown-streaming spike learnings.
4. **Omnigent PTY-attach**: terminal WS bytes → `vt_write`; `on_pty_write` → back to the WS.

Follow the project's design→plan cycle (brainstorming → writing-plans) for 2–4; 1 is mechanical.

## Gotchas

- `zig@0.15` is keg-only → NOT on PATH by default. Every build path (dev + eventual CI) must add `$(brew --prefix zig@0.15)/bin`.
- libghostty-rs is young (v0.2.1, 2 authors) + tracks unstable Ghostty dev → expect churn on bumps; vendoring + pinning is the mitigation. Its examples aren't workspace members (a `grid_ref_tracked_rs` nesting quirk) — trivial.
- Prebuilt `.a` is deliberately deferred until CI exists (no CI today → prebuilt would mean committing a 12 MB binary for no payoff). The vendored `-sys` build.rs is a one-file change to flip later.
- Throwaway `/tmp` clones used this session (will be GC'd; a fresh session re-fetches): `/tmp/libghostty-rs`, `/tmp/ghostty-v1.3.1`, `/tmp/zig-0.15.2`, `/tmp/zig-0.16.0`. `/tmp/libghostty-rs` was locally edited (example added to workspace) — don't vendor from it; re-clone at `46a9d2ac`.
- Nothing pushed. Original design docs (`docs/specs/2026-07-14-terminal-workstream-design.md`, roadmap) still describe the old source-port model — update or supersede them as part of the new design pass.
