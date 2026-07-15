# Vendored `libghostty-rs` — Ghostty VT binding (build-from-source)

Lens's terminal VT is a **vendored Rust binding built from source**. This
directory holds the two upstream crates at a pinned rev; the Ghostty C library
they wrap is built from source by the `-sys` build script (see below).

Decision rationale: memories `terminal-vt-adoption-model` +
`zig-ghostty-macos26-scissor`. Validated on macOS 26.6 (2026-07-15).

## What is and isn't vendored

| Component | Vendored here? | Pin |
| --- | --- | --- |
| `libghostty-vt` (safe API) | **yes** — `libghostty-vt/` | rev `46a9d2ac941ed600cf43c5e6299c8dfd1d3a1ef0` (tag `0.2.1`) |
| `libghostty-vt-sys` (raw FFI, checked-in `bindings.rs`) | **yes** — `libghostty-vt-sys/` | same rev |
| Ghostty VT C source | **no** — fetched at build time | commit `a887df42c56f6de86c0fe6da9c4eeca37931e083` (2026-07-11), pinned in `libghostty-vt-sys/build.rs` `GHOSTTY_COMMIT` |
| Ghostty's Zig deps (ziglyph, etc.) | **no** — fetched by `zig build` | resolved from `build.zig.zon` at the pinned Ghostty commit |
| Prebuilt `libghostty-vt.a` | **no** — built from source | — |

**Why crates-only** (2026-07-15 decision): the pinned Ghostty working tree is
~152M (a hand-pruned build closure is still ~57M). Committing that — and
re-vetting it on every pin bump — is the same "large artifact for zero payoff
before CI" anti-pattern we rejected for the prebuilt `.a`. The commit pin in
`build.rs` already makes the fetch reproducible. **Defer** both a vendored
Ghostty source tree (`GHOSTTY_SOURCE_DIR`) and a prebuilt `.a` to the **same
trigger: when CI lands** (both are a one-file `build.rs`/env change to flip).

## Upstream

- libghostty-rs — <https://github.com/Uzaaft/libghostty-rs>, rev `46a9d2ac` (tag `0.2.1`, 2026-07-15). License: SPDX `MIT OR Apache-2.0`.
- Ghostty — <https://github.com/ghostty-org/ghostty>, commit `a887df42`. License: MIT.

## Build toolchain (prereq)

The `-sys` build script runs `zig build -Demit-lib-vt=true` against the fetched
Ghostty source. On macOS 26 this **must** be the patched Homebrew/Nix
`zig@0.15` (0.15.2) — the vanilla ziglang.org tarball fails to link against the
macOS 26 SDK (ziglang/zig#31658). Homebrew installs it keg-only (not on PATH).

```
brew install zig@0.15
```

The workspace `.cargo/config.toml` points the build script at it via a `ZIG`
override, so plain `cargo build`/`cargo test` works with no per-shell PATH edit.
A one-time blobless Ghostty clone + zig build runs on first build (~25–33s),
cached in `OUT_DIR` thereafter (keyed by the commit stamp).

## Lens-local patches (re-apply on every pin bump)

Kept intentionally minimal:

1. **`libghostty-vt-sys/build.rs`** — honor a `ZIG` env override for the zig
   binary (default `zig`); add `rerun-if-env-changed=ZIG`; fix the
   `rerun-if-changed` path (was workspace-relative `crates/...`, now `build.rs`).
2. **Both `Cargo.toml`s** — flatten `*.workspace = true` package fields to
   upstream literals (they'd otherwise resolve against Lens's
   `[workspace.package]`), and repoint `libghostty-vt-sys` from a workspace dep
   to a `path` dep.

Nothing else is modified. `src/bindings.rs` is checked in upstream (no bindgen
/ libclang at build time).

## License note

Upstream declares SPDX `MIT OR Apache-2.0` but ships only a single MIT `LICENSE`
(no Apache-2.0 text). `LICENSE` here is that MIT file, verbatim. The dual SPDX
expression lets us use the MIT arm; no Apache text is required for MIT use.

## Provenance record

Structured pins in `provenance.toml` (for grep / future tooling). There is **no
machine verifier** for this directory — the dead WP0 `xtask terminal-provenance`
apparatus was removed; this collapses to ordinary dependency vetting.
