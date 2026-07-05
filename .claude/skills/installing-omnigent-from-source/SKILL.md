---
name: installing-omnigent-from-source
description: Use when you need a running omnigent server that matches the design's pinned 0.4.0 contract, when `omnigent --version` reports the wrong version (e.g. an older PyPI/uv release), or when (re)installing/updating omnigent for spikes, contract tests, or local runs.
---

# Installing omnigent from source

## Why this exists

Lens is grounded against a **specific** omnigent release tag. Two files in
`vendor/omnigent-<ver>/` record the pin: `OMNIGENT_PIN` holds the package
version (`0.4.0`); `README.md` records the **Source tag** (`v0.4.0`) and its
**Source HEAD commit** (`31669e1b`) — that commit is the real ground truth. A
mismatched release on PyPI (`uv tool install omnigent`) is a **different
contract**: different SSE schemas, REST paths, and it lacks the source-only
WebSocket channels (terminal-attach, session updates) that aren't in
`openapi.json`. Probing or testing against the wrong version yields misleading
results. The server must run from the source checkout at the pinned tag.

## Procedure (verified)

Sibling source checkout lives at `../omnigent` (i.e. `/Users/aakshintala/work/omnigent`).

```bash
# 0. Put the checkout ON the pinned tag (detached HEAD is expected).
git -C ../omnigent checkout v0.4.0     # the Source tag in vendor README

# 1. Remove any release install so the wrong version can't launch by accident.
uv tool uninstall omnigent            # no-op if not installed

# 2. Install editable from the source checkout (tracks the working tree).
uv tool install --editable ../omnigent

# 3. Verify the binary embeds the PINNED commit, not just a version string.
omnigent --version                    # -> omnigent 0.4.0 (31669e1b, built ...)
```

It exposes two executables: `omnigent` and `omni`. The shebang points into
`~/.local/share/uv/tools/omnigent/`.

```bash
# 4. A running background daemon caches the code it STARTED with. If one is up,
#    restart it or it keeps serving the old version (the install alone won't).
omnigent server status && omnigent server stop && omnigent server start
```

## Critical: do NOT `git pull` the checkout by reflex

The checkout should sit **on the pin tag**, not on the latest `main`. Moving it
forward puts the live server off the contract the vendored `openapi.json` was
generated from — silent contract drift. The pin advances on a deliberate,
owned cadence (ADR-0001; omnigent ships ~weekly), never by reflex.

- **Spike/test against the current contract:** stay frozen. Verify the checkout
  is on the pin first: `git -C ../omnigent rev-parse --short HEAD` must match the
  **Source HEAD** commit in `vendor/omnigent-<ver>/README.md` (`31669e1b`).
- **Deliberately advancing the pin:** that is a separate, owned task — run the
  `bumping-the-omnigent-pin` skill (re-vendor, re-codegen, fix lens-client,
  re-ground docs). Don't fold it into a routine reinstall.

## Quick reference

| Goal | Command |
|---|---|
| Check live version + commit | `omnigent --version` |
| Check checkout commit vs pin | `git -C ../omnigent rev-parse --short HEAD` (vs README Source HEAD) |
| Reinstall after a checkout change | `uv tool install --editable ../omnigent --reinstall` |
| Remove | `uv tool uninstall omnigent` |
