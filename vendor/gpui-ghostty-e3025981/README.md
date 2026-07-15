# WP0 terminal provenance — audit artifacts

This directory holds **audit-only** provenance for the pinned upstream terminal
inputs that a later work package (WP2) may adopt. **No upstream source is
vendored here** — only pins, hashes, an adopt/adapt/exclude inventory, a GPUI
reconciliation, a Zig build probe, and licenses.

## Pins

| Component | Pin |
| --- | --- |
| gpui-ghostty (wrapper) | `e3025981c6211dd7db2a825dc364ffb5d342f45e` — <https://github.com/Xuanwo/gpui-ghostty> |
| Ghostty (submodule) | `6d2dd585a5d87fa745d48188dd096ca6e63014d0` — <https://github.com/ghostty-org/ghostty>, tag `v1.2.3` |
| Zig | `0.14.1` |

## Hard gate

- No Ghostty / gpui-ghostty VT or render source enters `crates/` or any Lens
  production crate in WP0.
- `source-archives.sha256` records **hashes only** — no archive blobs, no trees.
- **WP2 is the first package allowed to import approved (`adopt`/`adapt`) rows,
  and only after this directory is committed and Opus-approved.**

## Regenerating the archive hashes

```bash
UPSTREAM=/tmp/gpui-ghostty-e3025981   # detached checkout at the pin (not committed)
{
  git -C "$UPSTREAM" archive --format=tar e3025981c6211dd7db2a825dc364ffb5d342f45e \
    | shasum -a 256 | awk '{print $1"  gpui-ghostty-e3025981.tar"}'
  git -C "$UPSTREAM/vendor/ghostty" archive --format=tar 6d2dd585a5d87fa745d48188dd096ca6e63014d0 \
    | shasum -a 256 | awk '{print $1"  ghostty-6d2dd585.tar"}'
} > source-archives.sha256
```

## Verifying this directory

```bash
cargo run -p xtask -- terminal-provenance \
  --root vendor/gpui-ghostty-e3025981 \
  --upstream /tmp/gpui-ghostty-e3025981
```
