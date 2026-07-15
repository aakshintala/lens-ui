# Terminal WP0 provenance plan — Opus 4.8 review

**Date:** 2026-07-15  
**Reviewer:** `claude-opus-4-8` through the local Claude CLI alias `opus`  
**Claude review session:** `e919f757-5190-4e99-b13e-df5789f8e583`  
**Roadmap base:** `6ebba06`  
**Reviewed plan:** `docs/plans/2026-07-15-terminal-wp0-provenance.md` (626 lines)  
**Reviewed plan SHA-256:** `15d4efcd840a7762fde7b079dc08dfbf435234889f84590c4e5eca2bf2460da9`  
**Pinned source checkout:** `/tmp/gpui-ghostty-e3025981`  
**gpui-ghostty pin:** `e3025981c6211dd7db2a825dc364ffb5d342f45e`  
**Ghostty submodule pin:** `6d2dd585a5d87fa745d48188dd096ca6e63014d0`

This document is the immutable provenance record for the review of the plan at the exact hash above. Corrections belong in the WP0 plan, not in this record. A later approval or review should be recorded separately.

## Verdict

**REVISE — seven blocking findings.**

Do not begin implementation until the findings below are corrected and a focused Opus re-review returns `APPROVE`.

## Review method and confirmed facts

The reviewer compared the plan against the repository's `AGENTS.md` and `.agents/` rules, the terminal workstream design and roadmap, the current `xtask` and Cargo structure, and the pinned upstream checkout.

The framework compatibility section was found valid. Lens resolves crates.io `gpui 0.2.2` with checksum prefix `979b45cf`, while the pinned upstream uses git `gpui 0.2.2` at Zed commit `cff3ac6...`.

The wrapper-file count check passed:

- The upstream tree contains 46 tracked non-`.git` files, plus the `vendor/ghostty` gitlink (`160000`) and `ghostty_src` symlink (`120000`).
- Removing `vendor/ghostty`, `ghostty_src`, and `.gitmodules` leaves the expected 45 wrapper files.

## Blocking findings

### B1. The `xtask` library and module layout contradicts the test imports

The plan specifies `[lib] path = "src/terminal_provenance.rs"`, which makes that file the crate root. The proposed tests import `xtask::terminal_provenance`, while Task 2 also shows a crate-root call. Those forms cannot all be correct simultaneously.

Required correction:

- Add `crates/xtask/src/lib.rs` with `pub mod terminal_provenance;`.
- Use Cargo's default library path.
- Make the CLI and tests consistently call `xtask::terminal_provenance`.
- Update all affected file lists and snippets.

### B2. The nominally valid fixture fails its own compile-closure rule

The fixture includes `terminal/Terminal.zig`, but the mirror-disposition row classifies it as `exclude`. The proposed verifier requires every compile-closure member to be classified as `adopt` or `adapt`, so the valid fixture is invalid by construction.

Required correction: classify `terminal/Terminal.zig` as `adopt` or `adapt` in the valid fixture.

### B3. Task 7's validation test also breaks the valid fixture

The test adds `relevant = "src/smoke.rs"` without adding a corresponding wrapper row. The verifier requires relevant entries to be represented in the wrapper classification, so this test fails for the wrong reason.

Required correction: add a wrapper row for `src/smoke.rs`, or make the relevant entry point to an already-classified wrapper file.

### B4. The forbidden-import verification command matches the verifier itself

The proposed repository-wide grep for names such as `portable-pty` searches `crates/`, where the `xtask` verifier necessarily contains those forbidden dependency names as constants. The check therefore self-hits.

Required correction: exclude `crates/xtask/**` from this check, or constrain the search to actual Rust import/dependency syntax rather than raw words.

### B5. The offline Zig proof depends on an unverified command-line flag

The plan relies on `zig build --offline`, but that flag was not established as supported by the pinned Zig toolchain. A nonexistent or ineffective flag cannot serve as the network-isolation proof.

Required correction:

- Inspect the pinned Zig version's help before naming an offline flag.
- Prefetch dependencies into a dedicated `ZIG_GLOBAL_CACHE_DIR`.
- Rebuild with networking actually disabled, or use another verified operating-system-level isolation mechanism.
- Treat successful compilation while network access is unavailable as the evidence.

### B6. The license closure omits statically linked `ziglyph`

The plan accounts for Apache-2.0 and Ghostty's MIT license, but not `ziglyph`, which is part of the static link closure.

Required correction:

- Add `licenses/MIT-ziglyph.txt`.
- Record its source URL, content hash, and upstream license path in provenance.
- Make the baseline license closure explicitly include Apache-2.0, MIT-Ghostty, and MIT-ziglyph.

### B7. Fixture and pinned-vendor build-probe modes are conflated

Fixtures need the synthetic `FIXTURE` archive marker, while a real pinned vendor audit must reject that marker. The proposed single verification path cannot enforce both requirements safely.

Required correction:

- Add an explicit mode such as `VerificationMode::{Fixture, Vendor}`.
- Permit the synthetic marker only in fixture mode.
- Reject `FIXTURE` and empty archive evidence in vendor mode.
- Make the CLI select vendor mode when `--upstream` is supplied.

## Non-blocking notes

### N1. Fixture and real-manifest schemas diverge

The real manifest adds `archive_hash_file` and may add `mirror_count_note`, while the fixture schema does not. Either make these fields explicitly optional or include them consistently in fixtures.

### N2. The Zig cache path is imprecise

Use Zig's local `.zig-cache` terminology and distinguish it from the global cache directory rather than referring generically to `zig-cache`.

### N3. `$UPSTREAM` command scope is implicit

Repeat or export the variable in each command block so the plan remains executable when snippets are run independently.

### N4. The canonical mirror count is ambiguous

Name the canonical source for the count and treat the other computation as an equality check, rather than leaving two possible authorities.

### N5. The `LICENSE` catch-all reason is inaccurate

Give `LICENSE` a dedicated classification and reason instead of reusing a generic non-Rust/build-input explanation.

## Required patch checklist

- [ ] Add `crates/xtask/src/lib.rs` and reconcile the library/module layout everywhere.
- [ ] Correct the valid fixture's `terminal/Terminal.zig` disposition.
- [ ] Correct Task 7's relevant-wrapper fixture setup.
- [ ] Prevent forbidden-import verification from matching `xtask`'s own policy constants.
- [ ] Replace the unverified Zig offline flag with a demonstrated network-isolation procedure.
- [ ] Add the `ziglyph` license artifact and provenance fields.
- [ ] Separate fixture verification from pinned-vendor verification.
- [ ] Reconcile optional and required manifest fields across fixtures and real data.
- [ ] Correct `.zig-cache` and global-cache terminology.
- [ ] Make `$UPSTREAM` scoping explicit in executable snippets.
- [ ] Declare one canonical mirror-count source and use the other as a cross-check.
- [ ] Give `LICENSE` a precise classification and reason.

## Approval gate

After revising the plan:

1. Run document diff checks and the plan's marker scan.
2. Request a focused Opus re-review of the corrected plan against this finding set.
3. Require an `APPROVE` verdict before committing the WP0 plan or beginning implementation.
