# Terminal WP0 Provenance and Reproducible Inputs Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the hard pre-port provenance gate for pinned gpui-ghostty/Ghostty inputs — pins, licenses, exhaustive adopt/adapt/exclude inventory, GPUI 0.2.2 reconciliation, Zig offline inputs, and a typed xtask verifier — without importing any terminal source into Lens crates.

**Architecture:** WP0 writes only audit artifacts under `vendor/gpui-ghostty-e3025981/` plus an xtask verifier. A temporary checkout outside the repo supplies live tree enumeration, git-archive hashes, and the Zig 0.14.1 build probe. **WP2 is the first package allowed to import approved sources, and only after this package is committed and Opus-approved.**

**Tech Stack:** Rust 2024 xtask (`serde`/`toml`/`anyhow`), TOML/Markdown provenance artifacts, git archive + SHA-256, Zig 0.14.1, macOS Apple Silicon.

**Delegated author model:** grok-4.5-xhigh owns pin/license/GPUI decisions; composer-2.5 may perform mechanical inventory generation. **Independent reviewer:** Codex orchestrator + Opus 4.8 via local Claude CLI (`--model opus` → `claude-opus-4-8`). Producing model never reviews its own output.

**Revision R1 (2026-07-15):** this plan incorporates the fixes for the seven blocking findings (B1–B7) and five notes (N1–N5) from `docs/plans/2026-07-15-terminal-wp0-provenance-opus-review.md`: default-path `src/lib.rs` crate root (B1); compile-closure-consistent valid fixture (B2); relevant-entry wrapper row (B3); xtask-excluded forbidden-import scan (B4); prefetch-then-network-denied Zig offline proof (B5); MIT-ziglyph in the baseline license closure (B6); `VerificationMode::{Fixture,Vendor}` gating the `FIXTURE` marker (B7); plus N1–N5. Inline `**Bn**`/`**Nn**` markers below tag each site.

## Global Constraints

- Hard gate: no Ghostty/gpui-ghostty VT/render source enters `crates/` or any Lens production crate in WP0.
- No source archives or upstream trees are added under Lens; `source-archives.sha256` records hashes only.
- Offline/reproducible normal-build inputs for everything WP2 will later adopt.
- Pins: gpui-ghostty `e3025981c6211dd7db2a825dc364ffb5d342f45e`; Ghostty `6d2dd585a5d87fa745d48188dd096ca6e63014d0`; Zig `0.14.1`; Ghostty tag `v1.2.3`.
- Baseline license closure = Apache-2.0 (wrapper) + MIT-Ghostty + MIT-ziglyph (ziglyph is statically linked into `libghostty_vt.a`, so its license is load-bearing, not "nested-if-required"); any further nested license added only if the compile closure pulls it in.
- Every candidate file classified adopt/adapt/exclude; forbidden paths must never be `adopt`.
- GPUI: single dependency = workspace crates.io `gpui 0.2.2`; reconcile upstream git `cff3ac6f…`; no unexamined second GPUI.
- Workspace gates: `cargo fmt`, `cargo clippy --workspace --all-targets -- -D warnings`; verifier errors are values (no panic).
- **Preflight before Task 1 (Important, R3):** `cargo fmt --all -- --check` and `cargo clippy --workspace --all-targets -- -D warnings` must be **green on the base tree first** — AGENTS.md (§ clippy gate) requires resolving a red gate *before* starting execution, never building on it. If red, it MUST be resolved before Task 1 begins.
- Exclusions: local PTY/`portable-pty`, Kitty inline images, Sixel, OSC 1337, Ghostty termio/pty/app shell, native renderer/image decode, packaging/CI.

---

## File structure (locked)

**Create:** `vendor/gpui-ghostty-e3025981/{README.md,provenance.toml,adoption.toml,compile-closure.txt,source-archives.sha256,upstream-tests.toml,gpui-reconciliation.md,build-probe.md,licenses/Apache-2.0.txt,licenses/MIT-Ghostty.txt,licenses/MIT-ziglyph.txt}`; `crates/xtask/src/lib.rs` (crate root: `pub mod terminal_provenance;`); `crates/xtask/src/terminal_provenance.rs`; `crates/xtask/tests/terminal_provenance.rs`; `vendor/gpui-ghostty-e3025981/probe-logs/` (committed Zig build/offline/fetch logs, Task 5); `crates/xtask/tests/fixtures/terminal-provenance/{valid,missing-pin,unknown-disposition,duplicate-path,missing-license,forbidden-adopt,vendor-blank-probe,vendor-one-hash,vendor-dup-archive,vendor-wrong-archive}/…`; `scripts/generate-terminal-adoption.sh`.

**Modify:** `crates/xtask/src/main.rs`, `crates/xtask/Cargo.toml`, `docs/design/framework.md` §2.2, `docs/STATUS.md`.

**Temporary (not committed):** `/tmp/gpui-ghostty-e3025981` + Zig under that tree’s `.context/zig/`.

## Locked pins and enumeration

```bash
set -euo pipefail   # Blocking (R3): a failed pin `test` or clone must HALT, not continue
UPSTREAM=/tmp/gpui-ghostty-e3025981
REPO=https://github.com/Xuanwo/gpui-ghostty
COMMIT=e3025981c6211dd7db2a825dc364ffb5d342f45e
GHOSTTY_COMMIT=6d2dd585a5d87fa745d48188dd096ca6e63014d0
# Wrapper = 45 paths (exclude vendor/ghostty, ghostty_src, .gitmodules):
git -C "$UPSTREAM" ls-files | grep -v '^vendor/ghostty' | grep -v 'ghostty_src' | grep -v '^\.gitmodules$' | sort
# Mirror — CANONICAL count source = `git ls-files src/**` in vendor/ghostty at 6d2dd585 (record its wc -l).
# Evidence claimed 685; the live pin is authoritative — never invent rows.
git -C "$UPSTREAM/vendor/ghostty" ls-files 'src/**' | sort | tee /tmp/ghostty-src-files.txt | wc -l   # canonical
# Cross-check ONLY (must equal the canonical count; not a second authority):
find -L "$UPSTREAM/crates/ghostty_vt_sys/zig/ghostty_src" -type f | sed "s|.*/ghostty_src/||" | sort | wc -l
```

Zig macOS SHA-256: x86_64 `b0f8bdfb9035783db58dd6c19d7dea89892acc3814421853e5752fe4573e5f43`; arm64 `39f3dc5e79c22088ce878edc821dedb4ca5a1cd9f5ef915e9b3cc3053e8faefa`. ziglyph URL `https://deps.files.ghostty.org/ziglyph-b89d43d1e3fb01b6074bc1f7fc980324b04d26a5.tar.gz`, hash `ziglyph-0.11.2-AAAAAHPtHwB4Mbzn1KvOV7Wpjo82NYEc_v0WC8oCLrkf`.

---

### Task 1: TDD typed xtask provenance verifier + fixtures

**Files:** Create `crates/xtask/src/lib.rs`, `crates/xtask/src/terminal_provenance.rs`, `crates/xtask/tests/terminal_provenance.rs`, fixtures under `crates/xtask/tests/fixtures/terminal-provenance/`. Modify `crates/xtask/Cargo.toml` (`serde`+derive, `toml = "0.8"`, `thiserror = "2"`, `sha2 = "0.10"` for the B6 license-hash binding).

**B1 — library layout (Cargo defaults, no custom `path`):** `src/lib.rs` is the crate root and declares the module; the binary stays `src/main.rs`. Tests and CLI both address `xtask::terminal_provenance::*`.

```rust
// crates/xtask/src/lib.rs
pub mod terminal_provenance;
```

```toml
# crates/xtask/Cargo.toml — default lib path (src/lib.rs); do NOT set [lib] path.
[[bin]]
name = "xtask"
path = "src/main.rs"
```

**Interfaces:**
- `pub enum Disposition { Adopt, Adapt, Exclude }`
- `pub enum VerificationMode { Fixture, Vendor }` — **B7**: `Fixture` permits the synthetic `FIXTURE` build-probe marker and `0{64}` archive hashes; `Vendor` rejects both and demands captured values.
- `pub enum VerifyError { MissingPin { field: String }, UnknownDisposition { path: String, got: String }, DuplicatePath { path: String }, MissingInventoryPath { path: String }, ExtraInventoryPath { path: String }, MissingLicenseMapping { path: String }, ForbiddenAdopt { path: String }, UnresolvedGpuiReconciliation, WrongZigVersion { expected: String, got: String }, MissingHash { name: String }, MissingArtifact { path: String }, LicenseHashMismatch { path: String, expected: String, got: String }, BuildProbeIncomplete { field: String }, FixtureMarkerInVendorMode { field: String }, MirrorCountMismatch { expected: usize, got: usize }, WrapperCountMismatch { expected: usize, got: usize } }` — the CLI renders each error's **Debug** form (variant name included) one per line, so a caller can assert on the variant.
- `pub fn load_and_verify(root: &Path, mode: VerificationMode) -> Result<(), Vec<VerifyError>>` — no panic; aggregate errors
- `pub const FORBIDDEN_ADOPT_PREFIXES: &[&str] = &["examples/pty_terminal/", "examples/split_pty_terminal/", ".github/"];` plus case-insensitive path substrings `portable-pty`, `sixel`, `osc1337`

- [ ] **Step 1: Write failing tests + fixtures**

`valid/provenance.toml` must set every field below (fixture hashes may be 64×`0`; real vendor uses captured values):

```toml
gpui_ghostty_remote = "https://github.com/Xuanwo/gpui-ghostty"
gpui_ghostty_commit = "e3025981c6211dd7db2a825dc364ffb5d342f45e"
ghostty_remote = "https://github.com/ghostty-org/ghostty"
ghostty_commit = "6d2dd585a5d87fa745d48188dd096ca6e63014d0"
ghostty_tag = "v1.2.3"
zig_version = "0.14.1"
zig_macos_x86_64_sha256 = "b0f8bdfb9035783db58dd6c19d7dea89892acc3814421853e5752fe4573e5f43"
zig_macos_arm64_sha256 = "39f3dc5e79c22088ce878edc821dedb4ca5a1cd9f5ef915e9b3cc3053e8faefa"
ziglyph_url = "https://deps.files.ghostty.org/ziglyph-b89d43d1e3fb01b6074bc1f7fc980324b04d26a5.tar.gz"
ziglyph_hash = "ziglyph-0.11.2-AAAAAHPtHwB4Mbzn1KvOV7Wpjo82NYEc_v0WC8oCLrkf"
gpui_upstream_source = "git+https://github.com/zed-industries/zed#cff3ac6f93f506330034652f0d2389591bfb45a0"
gpui_upstream_version = "0.2.2"
gpui_lens_source = "registry+https://github.com/rust-lang/crates.io-index"
gpui_lens_version = "0.2.2"
gpui_strategy = "single_crates_io_0_2_2"
gpui_reconciliation_path = "gpui-reconciliation.md"
wrapper_file_count = 2
mirror_file_count = 1
license_apache_path = "licenses/Apache-2.0.txt"
license_mit_path = "licenses/MIT-Ghostty.txt"
license_ziglyph_path = "licenses/MIT-ziglyph.txt"
archive_hash_file = "source-archives.sha256"
```

`valid/adoption.toml` (**B2**: the compile-closure member `terminal/Terminal.zig` must be `adopt`/`adapt`, never `exclude`; **B3**: `smoke.rs` is a wrapper row so `upstream-tests.toml`'s relevant entry resolves):

```toml
[[wrapper]]
path = "crates/ghostty_vt/src/lib.rs"
disposition = "adapt"
license = "Apache-2.0"
reason = "FFI-safe VT facade"

[[wrapper]]
path = "crates/ghostty_vt/tests/smoke.rs"
disposition = "adopt"
license = "Apache-2.0"
reason = "VT corpus (relevant upstream test)"

[[mirror]]
path = "terminal/Terminal.zig"
disposition = "adopt"
license = "MIT"
reason = "fixture VT compile-closure member"
```

Also stub in `valid/`: `README.md`; `compile-closure.txt` with `terminal/Terminal.zig`; `source-archives.sha256` (two `0{64}  name.tar` lines — accepted only because the fixtures load in `VerificationMode::Fixture`); `upstream-tests.toml` with one `[[test]] path = "crates/ghostty_vt/tests/smoke.rs" relevant = true` (resolves to the `smoke.rs` wrapper row above); `licenses/{Apache-2.0,MIT-Ghostty,MIT-ziglyph}.txt`; `gpui-reconciliation.md` containing `## Decision` and `strategy = single_crates_io_0_2_2`; `build-probe.md` with every Task-5 heading and body token `FIXTURE` (permitted only in `Fixture` mode; **B7** — a `Vendor`-mode load of this same file must fail with `FixtureMarkerInVendorMode`).

Negative fixtures: `missing-pin` omits `zig_version`; `unknown-disposition` uses `disposition = "maybe"`; `duplicate-path` repeats a path; `missing-license` omits `license` on adopt/adapt; `forbidden-adopt` adopts `examples/pty_terminal/src/main.rs`. **Vendor-mode negatives (B7, R3)** — both otherwise Vendor-clean (real pins, non-`FIXTURE` `build-probe.md` bodies, two non-zero 64-hex `source-archives.sha256` lines, `ziglyph_license_source`+`ziglyph_license_sha256` present, license files hash-matched) so only the target rule fires: `vendor-blank-probe` blanks one required `build-probe.md` section body; `vendor-one-hash` keeps a single `source-archives.sha256` line; **`vendor-dup-archive`** has two lines with the SAME expected name; **`vendor-wrong-archive`** has two non-zero lines with names that are neither `gpui-ghostty-e3025981.tar` nor `ghostty-6d2dd585.tar` (R4 — proves the exact-two-names rule, not just "two non-zero lines", is enforced).

```rust
// crates/xtask/tests/terminal_provenance.rs
use std::path::PathBuf;
use xtask::terminal_provenance::{load_and_verify, VerificationMode, VerifyError};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/terminal-provenance").join(name)
}

#[test]
fn valid_fixture_passes() {
    assert!(load_and_verify(&fixture("valid"), VerificationMode::Fixture).is_ok());
}

// B7: the SAME valid fixture must FAIL in Vendor mode — its build-probe body is `FIXTURE`
// and its archive hashes are all-zero.
#[test]
fn valid_fixture_rejected_in_vendor_mode() {
    let e = load_and_verify(&fixture("valid"), VerificationMode::Vendor).unwrap_err();
    assert!(e.iter().any(|x| matches!(x, VerifyError::FixtureMarkerInVendorMode { .. })));
}

#[test]
fn missing_pin_fails() {
    let e = load_and_verify(&fixture("missing-pin"), VerificationMode::Fixture).unwrap_err();
    assert!(e.iter().any(|x| matches!(x, VerifyError::MissingPin { .. })));
}
#[test]
fn unknown_disposition_fails() {
    let e = load_and_verify(&fixture("unknown-disposition"), VerificationMode::Fixture).unwrap_err();
    assert!(e.iter().any(|x| matches!(x, VerifyError::UnknownDisposition { .. })));
}
#[test]
fn duplicate_path_fails() {
    let e = load_and_verify(&fixture("duplicate-path"), VerificationMode::Fixture).unwrap_err();
    assert!(e.iter().any(|x| matches!(x, VerifyError::DuplicatePath { .. })));
}
#[test]
fn missing_license_fails() {
    let e = load_and_verify(&fixture("missing-license"), VerificationMode::Fixture).unwrap_err();
    assert!(e.iter().any(|x| matches!(x, VerifyError::MissingLicenseMapping { .. })));
}
#[test]
fn forbidden_adopt_fails() {
    let e = load_and_verify(&fixture("forbidden-adopt"), VerificationMode::Fixture).unwrap_err();
    assert!(e.iter().any(|x| matches!(x, VerifyError::ForbiddenAdopt { .. })));
}
```

- [ ] **Step 2: Run — expect FAIL**

```bash
cargo test -p xtask --test terminal_provenance -- --nocapture
```

Expected: compile/link failure (missing `load_and_verify` / lib), not PASS.

- [ ] **Step 3: Implement `src/lib.rs` (`pub mod terminal_provenance;`) + `terminal_provenance.rs`**

Parse with serde; require non-empty pins; `zig_version == "0.14.1"`; sha256 fields `^[0-9a-f]{64}$`; `gpui_strategy == "single_crates_io_0_2_2"` and reconciliation file contains that strategy line; unique paths; known dispositions; licenses on adopt/adapt; forbidden-adopt rules; all locked artifact filenames present (**including `license_ziglyph_path`, B6**); `build-probe.md` contains Task-5 headings; wrapper/mirror row counts match provenance counts; every `compile-closure.txt` line is a mirror row with adopt/adapt; every `upstream-tests.toml` `relevant` path resolves to a wrapper row (**B3**).

**B6 hash binding (GPT-5.6-Sol R2/R3):** the verifier reads `license_ziglyph_path`, `ziglyph_license_source`, and `ziglyph_license_sha256`, and asserts `sha256(<file at license_ziglyph_path>) == ziglyph_license_sha256` using the `sha2` crate — `MissingArtifact` if the file is absent, `MissingHash` if the field is absent (Vendor mode), and **`LicenseHashMismatch { path, expected, got }`** on divergence. Same binding applies to the Apache/MIT license artifacts if a hash is recorded.

**Mode (B7):** in `Vendor` mode, `FixtureMarkerInVendorMode` fires when `build-probe.md` contains `FIXTURE`, **any required build-probe section body is empty/whitespace**, or `source-archives.sha256` is not **exactly two lines, one `^<64-hex, non-zero>  gpui-ghostty-e3025981.tar$` and one `^<64-hex, non-zero>  ghostty-6d2dd585.tar$`** (Important, R3 — the two expected names must both be present and non-zero; two duplicate or unrelated entries must NOT pass); in `Fixture` mode all of these are permitted. `--upstream` ⇒ Vendor (tested at the CLI in Task 2).

**Vendor-mode truth binding (Blocking, R4) — the hard gate must reject a fabricated-but-well-shaped root, not merely a malformed one.** In `Vendor` mode the verifier asserts values EQUAL their locked constants, not just non-empty/regex-valid:
- **Exact pins/constants:** `gpui_ghostty_commit == e3025981c6211dd7db2a825dc364ffb5d342f45e`, `ghostty_commit == 6d2dd585a5d87fa745d48188dd096ca6e63014d0`, `ghostty_tag == v1.2.3`; both `zig_macos_*_sha256`, `ziglyph_url`, `ziglyph_hash`, and the four `gpui_*` source/version + `gpui_strategy` fields each equal their locked value (§ "Locked pins and enumeration"). Wrong-but-shaped values ⇒ `MissingPin { field }`.
- **Archive recompute (requires `--upstream`, hence Vendor):** recompute the `git archive` SHA-256 for both pinned commits from the upstream checkout and assert equality with `source-archives.sha256` — a hand-authored hash cannot pass (`MissingHash`/mismatch).
- **All baseline licenses bound:** `ziglyph_license_sha256` **and** recorded Apache/MIT license hashes are REQUIRED (not optional) and each must match its file (`LicenseHashMismatch` on drift).
- **Probe evidence retained:** `build-probe.md` must cite a COMMITTED log under `vendor/…/probe-logs/` whose recorded `raw_log_sha256` matches; a non-empty `offline_ziglyph = pass` backed only by an uncommitted `/tmp` log is rejected (`BuildProbeIncomplete`).

**Manifest optionality (N1):** `archive_hash_file` and `mirror_count_note` are optional in both modes; `ziglyph_license_source` and `ziglyph_license_sha256` are **optional in `Fixture` mode, required in `Vendor` mode** (fixtures ship only `license_ziglyph_path` + the stub file). Return `Err(Vec<VerifyError>)` — no `panic!` on input.

- [ ] **Step 4: Run — expect PASS** — `cargo test -p xtask --test terminal_provenance`

- [ ] **Step 5: Commit**

```bash
git add crates/xtask/Cargo.toml Cargo.lock crates/xtask/src/lib.rs crates/xtask/src/terminal_provenance.rs \
  crates/xtask/src/main.rs crates/xtask/tests/terminal_provenance.rs \
  crates/xtask/tests/fixtures/terminal-provenance   # Cargo.lock: serde/toml/thiserror added (R2)
git commit -m "$(cat <<'EOF'
feat(xtask): add terminal-provenance verifier and fixtures

EOF
)"
```

---

### Task 2: CLI `terminal-provenance`

**Files:** Modify `crates/xtask/src/main.rs`, `crates/xtask/src/terminal_provenance.rs`, tests.

**Interfaces:** `cargo run -p xtask -- terminal-provenance --root <path> [--upstream <path>]` → `pub fn run_terminal_provenance(root: &Path, upstream: Option<&Path>) -> anyhow::Result<()>` in `terminal_provenance.rs` (called as `xtask::terminal_provenance::run_terminal_provenance`). **B7 mode selection:** `--upstream` present ⇒ `VerificationMode::Vendor`; absent ⇒ `VerificationMode::Fixture`. With `--upstream` (⇒ Vendor), also (a) re-enumerate wrapper/mirror and emit count/path mismatch errors — **applying the same Task-4 normalization (strip leading `src/` from mirror paths) before comparing to the mirror rows (Important, R3), else raw `src/…` upstream paths spuriously reject the normalized inventory** — and (b) **recompute the two `git archive` SHA-256s from the checkout and assert equality with `source-archives.sha256` (Blocking truth-binding, R4)**. Exit 0 / `terminal-provenance: ok`; else exit 1 with one error per line (each the error's Debug form).

- [ ] **Step 1: Failing CLI test**

```rust
#[test]
fn cli_rejects_missing_pin_fixture() {
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["terminal-provenance", "--root"]).arg(fixture("missing-pin"))
        .status().unwrap();
    assert_eq!(status.code(), Some(1));
}

// B7 (R2/R3): the SAME valid fixture passes without --upstream (Fixture mode) but fails with
// --upstream (Vendor mode), AND the failure output NAMES the Vendor-mode marker — so mode selection,
// not incidental upstream enumeration, is the proven cause (assert on the variant, not just exit 1).
#[test]
fn cli_valid_fixture_passes_fixture_mode_but_fails_vendor_mode() {
    let ok = std::process::Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["terminal-provenance", "--root"]).arg(fixture("valid"))
        .output().unwrap();
    assert_eq!(ok.status.code(), Some(0), "no --upstream ⇒ Fixture mode ⇒ pass");

    let vendor = std::process::Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["terminal-provenance", "--root"]).arg(fixture("valid"))
        .arg("--upstream").arg(fixture("valid"))
        .output().unwrap();
    assert_eq!(vendor.status.code(), Some(1));
    let out = format!("{}{}", String::from_utf8_lossy(&vendor.stdout), String::from_utf8_lossy(&vendor.stderr));
    assert!(out.contains("FixtureMarkerInVendorMode"),
        "Vendor mode (not upstream enumeration) must be the proven cause; got:\n{out}");
}

// B7 (R3): the two non-FIXTURE Vendor rejections have dedicated fixtures + library tests, loaded in
// Vendor mode. Both fixtures are otherwise Vendor-clean (real pins, non-`FIXTURE` probe bodies, two
// non-zero 64-hex hashes, ziglyph source+hash present) so ONLY the target rule fires.
#[test]
fn vendor_blank_probe_section_fails() {
    let e = load_and_verify(&fixture("vendor-blank-probe"), VerificationMode::Vendor).unwrap_err();
    assert!(e.iter().any(|x| matches!(x, VerifyError::FixtureMarkerInVendorMode { .. })));
}
#[test]
fn vendor_single_archive_hash_fails() {
    let e = load_and_verify(&fixture("vendor-one-hash"), VerificationMode::Vendor).unwrap_err();
    assert!(e.iter().any(|x| matches!(x, VerifyError::FixtureMarkerInVendorMode { .. })));
}
// R4: two lines that are non-zero but NOT the two expected names must still fail — proves the rule is
// exact-two-names, not merely "two non-zero lines".
#[test]
fn vendor_duplicate_archive_name_fails() {
    let e = load_and_verify(&fixture("vendor-dup-archive"), VerificationMode::Vendor).unwrap_err();
    assert!(e.iter().any(|x| matches!(x, VerifyError::FixtureMarkerInVendorMode { .. })));
}
#[test]
fn vendor_wrong_archive_name_fails() {
    let e = load_and_verify(&fixture("vendor-wrong-archive"), VerificationMode::Vendor).unwrap_err();
    assert!(e.iter().any(|x| matches!(x, VerifyError::FixtureMarkerInVendorMode { .. })));
}
```

- [ ] **Step 2: Run — expect FAIL** — unknown command `terminal-provenance`.

```bash
cargo test -p xtask --test terminal_provenance cli_rejects_missing_pin_fixture -- --nocapture
```

- [ ] **Step 3: Wire CLI**

```rust
// main match:
"terminal-provenance" => terminal_provenance_cmd(std::env::args().skip(2)),
// help: codegen | drift | gate | terminal-provenance

fn terminal_provenance_cmd(mut args: impl Iterator<Item = String>) -> Result<()> {
    let mut root = None;
    let mut upstream = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--root" => root = Some(PathBuf::from(args.next().context("--root needs a path")?)),
            "--upstream" => upstream = Some(PathBuf::from(args.next().context("--upstream needs a path")?)),
            other => bail!("unknown terminal-provenance arg: {other}"),
        }
    }
    // run_terminal_provenance picks Vendor mode iff `upstream` is Some (B7).
    xtask::terminal_provenance::run_terminal_provenance(&root.context("missing --root")?, upstream.as_deref())
}
```

- [ ] **Step 4: Run — expect PASS**

```bash
cargo test -p xtask --test terminal_provenance
cargo run -p xtask -- terminal-provenance --root crates/xtask/tests/fixtures/terminal-provenance/valid
```

- [ ] **Step 5: Commit**

```bash
git add crates/xtask/src/main.rs crates/xtask/src/terminal_provenance.rs crates/xtask/tests/terminal_provenance.rs
git commit -m "$(cat <<'EOF'
feat(xtask): wire terminal-provenance CLI

EOF
)"
```

---

### Task 3: Safe temp checkout + archive hashes only

**Files:** Create `vendor/gpui-ghostty-e3025981/source-archives.sha256`, `README.md`.

- [ ] **Step 1: Clone or safely reuse (never `rm -rf` / `--hard`)**

```bash
set -euo pipefail   # Blocking (R3/R4): a failed pin `test`/clone/submodule step must HALT, not continue
UPSTREAM=/tmp/gpui-ghostty-e3025981
if [ -d "$UPSTREAM/.git" ]; then
  test "$(git -C "$UPSTREAM" rev-parse HEAD)" = "e3025981c6211dd7db2a825dc364ffb5d342f45e"
else
  git clone https://github.com/Xuanwo/gpui-ghostty "$UPSTREAM"
  git -C "$UPSTREAM" checkout --detach e3025981c6211dd7db2a825dc364ffb5d342f45e
fi
git -C "$UPSTREAM" submodule update --init vendor/ghostty
test "$(git -C "$UPSTREAM/vendor/ghostty" rev-parse HEAD)" = "6d2dd585a5d87fa745d48188dd096ca6e63014d0"
test "$(git -C "$UPSTREAM" config --file .gitmodules --get submodule.vendor/ghostty.url)" = \
  "https://github.com/ghostty-org/ghostty"
```

Expected: all tests exit 0; on mismatch stop and report.

- [ ] **Step 2: Hash git-archive streams into Lens (no archive blobs)**

```bash
# Blocking (R3): WITHOUT pipefail a failed `git archive` still feeds an empty stream to shasum,
# which happily records the well-known empty-input digest (e3b0c442…) and exits 0 — a silent
# provenance corruption. pipefail makes the pipeline inherit the archive's failure, and set -e halts.
set -euo pipefail
UPSTREAM=/tmp/gpui-ghostty-e3025981   # N3: standalone
mkdir -p vendor/gpui-ghostty-e3025981
{
  git -C "$UPSTREAM" archive --format=tar e3025981c6211dd7db2a825dc364ffb5d342f45e \
    | shasum -a 256 | awk '{print $1"  gpui-ghostty-e3025981.tar"}'
  git -C "$UPSTREAM/vendor/ghostty" archive --format=tar 6d2dd585a5d87fa745d48188dd096ca6e63014d0 \
    | shasum -a 256 | awk '{print $1"  ghostty-6d2dd585.tar"}'
} > vendor/gpui-ghostty-e3025981/source-archives.sha256
# Guard against the empty-stream digest slipping through anyway:
! rg -q 'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855' \
  vendor/gpui-ghostty-e3025981/source-archives.sha256
# Operational assertion (R4) — exactly the two expected named lines, each 64-hex (not just "two lines"):
test "$(grep -c . vendor/gpui-ghostty-e3025981/source-archives.sha256)" -eq 2
rg -q '^[0-9a-f]{64}  gpui-ghostty-e3025981\.tar$' vendor/gpui-ghostty-e3025981/source-archives.sha256
rg -q '^[0-9a-f]{64}  ghostty-6d2dd585\.tar$'      vendor/gpui-ghostty-e3025981/source-archives.sha256
cat vendor/gpui-ghostty-e3025981/source-archives.sha256
```

Expected: exactly two lines, one `^[0-9a-f]{64}  gpui-ghostty-e3025981\.tar$` and one `^[0-9a-f]{64}  ghostty-6d2dd585\.tar$` (neither the empty digest); re-run yields identical hashes.

- [ ] **Step 3: README** — state pins; directory is audit-only; no upstream source; WP2 imports after Opus approval; hash regeneration = commands above.

- [ ] **Step 4: No source import** (**B4**: never scan `crates/xtask/**` — the verifier legitimately holds these forbidden names as policy constants)

```bash
test ! -e vendor/gpui-ghostty-e3025981/gpui-ghostty-e3025981.tar
test ! -e vendor/gpui-ghostty-e3025981/ghostty-6d2dd585.tar
git ls-files crates | rg -v '^crates/xtask/' | rg -i 'ghostty|gpui.ghostty' && exit 1 || true
```

- [ ] **Step 5: Commit**

```bash
git add vendor/gpui-ghostty-e3025981/source-archives.sha256 vendor/gpui-ghostty-e3025981/README.md
git commit -m "$(cat <<'EOF'
chore(vendor): record gpui-ghostty source-archive hashes

EOF
)"
```

---

### Task 4: `provenance.toml` + exhaustive adoption generation

**Files:** Create `provenance.toml`, `adoption.toml`, `scripts/generate-terminal-adoption.sh`.

- [ ] **Step 1: Write known `provenance.toml`**

```toml
gpui_ghostty_remote = "https://github.com/Xuanwo/gpui-ghostty"
gpui_ghostty_commit = "e3025981c6211dd7db2a825dc364ffb5d342f45e"
ghostty_remote = "https://github.com/ghostty-org/ghostty"
ghostty_commit = "6d2dd585a5d87fa745d48188dd096ca6e63014d0"
ghostty_tag = "v1.2.3"
zig_version = "0.14.1"
zig_macos_x86_64_sha256 = "b0f8bdfb9035783db58dd6c19d7dea89892acc3814421853e5752fe4573e5f43"
zig_macos_arm64_sha256 = "39f3dc5e79c22088ce878edc821dedb4ca5a1cd9f5ef915e9b3cc3053e8faefa"
ziglyph_url = "https://deps.files.ghostty.org/ziglyph-b89d43d1e3fb01b6074bc1f7fc980324b04d26a5.tar.gz"
ziglyph_hash = "ziglyph-0.11.2-AAAAAHPtHwB4Mbzn1KvOV7Wpjo82NYEc_v0WC8oCLrkf"
gpui_upstream_source = "git+https://github.com/zed-industries/zed#cff3ac6f93f506330034652f0d2389591bfb45a0"
gpui_upstream_version = "0.2.2"
gpui_lens_source = "registry+https://github.com/rust-lang/crates.io-index"
gpui_lens_version = "0.2.2"
gpui_strategy = "single_crates_io_0_2_2"
gpui_reconciliation_path = "gpui-reconciliation.md"
wrapper_file_count = 45
mirror_file_count = 0
license_apache_path = "licenses/Apache-2.0.txt"
license_mit_path = "licenses/MIT-Ghostty.txt"
license_ziglyph_path = "licenses/MIT-ziglyph.txt"
ziglyph_license_source = "extracted from the pinned ziglyph tarball (see ziglyph_url / ziglyph_hash)"
ziglyph_license_sha256 = "<captured in Task 7 from the extracted license file>"
archive_hash_file = "source-archives.sha256"
```

Generator overwrites `mirror_file_count` from live `wc -l`. If live ≠ 685, keep live count and set `mirror_count_note = "live git ls-files src/** at 6d2dd585; prior evidence claimed 685 — Opus must acknowledge"`.

- [ ] **Step 2: Generator rules (`scripts/generate-terminal-adoption.sh`)**

Emit `[[wrapper]]` for all 45 paths and `[[mirror]]` for every Ghostty `src/**` path. **Mirror `path` normalization (Important, R3):** the enumeration yields `src/`-prefixed paths (e.g. `src/terminal/Terminal.zig`), but mirror rows and `compile-closure.txt` are **relative to `vendor/ghostty/src`** — the generator strips the leading `src/` (→ `terminal/Terminal.zig`), matching the fixtures and the closure convention, so closure-promotion path-matches. First-match wrapper rules:

| Match | disposition | reason |
| --- | --- | --- |
| `examples/pty_terminal/**`, `examples/split_pty_terminal/**` | exclude | local PTY example |
| `.github/**` | exclude | packaging/CI |
| `LICENSE`, `LICENSE-*` | exclude | upstream license retained verbatim under `vendor/…/licenses/`; not imported as source (**N5**) |
| `Cargo.lock`, `README.md`, `ROADMAP.md`, `AGENTS.md`, `.gitignore`, `docs/**` | exclude | lockfile/docs not imported |
| `crates/gpui_ghostty_terminal/src/view/mod.rs`, `crates/gpui_ghostty_terminal/src/font.rs`, `crates/gpui_ghostty_terminal/src/tests.rs`, `examples/basic_terminal/src/main.rs` | adapt | GPUI 0.2.2 reconciliation surface |
| `crates/ghostty_vt_sys/**`, `crates/ghostty_vt/src/lib.rs`, `crates/ghostty_vt/Cargo.toml` | adapt | FFI/VT boundary |
| `crates/ghostty_vt/tests/*.rs` | adopt | VT corpus for WP7 |
| `crates/gpui_ghostty_terminal/src/session.rs` | exclude | PTY host — omnigent owns PTY |
| remaining wrapper build/Rust/Zig/license/Cargo/example files | adapt | wrapper build/port surface |

**Path anchoring (New-Blocking, GPT-5.6-Sol R2):** wrapper rows are the exact `git ls-files` strings — VT/terminal crates carry the **`crates/` prefix** (`crates/gpui_ghostty_terminal/src/session.rs`, verified against the pin), while `examples/**`, `.github/**`, `LICENSE`, `docs/**` are **repo-root** (no prefix). Match patterns against the full path; first-match order keeps the `session.rs` exclude ahead of the catch-all so the PTY host never falls through to `adapt`.

Mirror defaults: `exclude`/`MIT`/`not in slim compile closure`. Pre-closure exclude overlays for paths matching `termio/`, `apprt/`, `renderer/`, `font/`, `cli/`, `shell-integration/`, `inspector/`, `benchmark/`, `stb/`, `sixel`, `kitty`, `osc1337`, `image`. Task 5 promotes compile-closure paths to adopt/adapt.

```bash
chmod +x scripts/generate-terminal-adoption.sh
UPSTREAM=/tmp/gpui-ghostty-e3025981 ./scripts/generate-terminal-adoption.sh vendor/gpui-ghostty-e3025981
test "$(rg -c '^\[\[wrapper\]\]' vendor/gpui-ghostty-e3025981/adoption.toml)" = "45"
```

- [ ] **Step 3: Completeness — empty `comm -3` between live wrapper list and adoption wrapper paths**

```bash
UPSTREAM=/tmp/gpui-ghostty-e3025981   # N3: re-declare so this block runs standalone
comm -3 \
  <(git -C "$UPSTREAM" ls-files | grep -v '^vendor/ghostty' | grep -v 'ghostty_src' | grep -v '^\.gitmodules$' | sort) \
  <(awk '/^\[\[wrapper\]\]/{w=1;next} w&&/^path = /{gsub(/"/,""); print $3; w=0}' vendor/gpui-ghostty-e3025981/adoption.toml | sort)
```

Expected: no output. **Mirror row count must equal `mirror_file_count`, which is the canonical `git ls-files 'src/**'` count at 6d2dd585 (N4)** — the generator writes that count and every `[[mirror]]` row derives from the same enumeration.

- [ ] **Step 4: Commit**

```bash
git add scripts/generate-terminal-adoption.sh vendor/gpui-ghostty-e3025981/provenance.toml vendor/gpui-ghostty-e3025981/adoption.toml
git commit -m "$(cat <<'EOF'
chore(vendor): generate terminal adoption inventory

EOF
)"
```

---

### Task 5: Zig 0.14.1 build probe + compile closure

**Files:** Create `build-probe.md`, `compile-closure.txt`; update `adoption.toml` (and provenance if nested licenses appear).

**Required `build-probe.md` headings** (verifier fails if any missing or body is `FIXTURE` on the real vendor root):

```markdown
# Build probe
## zig_version
## bootstrap_sha256
## build_command
## build_exit_code
## object_list
## dependency_list
## compile_closure_path_count
## offline_ziglyph
## renderer_cell_coupling
## artifact_sha256
## license_closure
## raw_log_path
## raw_log_sha256
```

- [ ] **Step 1: Bootstrap Zig + establish the offline mechanism (B5 — do NOT assume `--offline` exists)**

```bash
UPSTREAM=/tmp/gpui-ghostty-e3025981   # N3
( cd "$UPSTREAM" && ./scripts/bootstrap-zig.sh )
export ZIG="$UPSTREAM/.context/zig/zig"
"$ZIG" version   # expect: 0.14.1
# Inspect the pinned toolchain's ACTUAL flags before naming any offline switch:
"$ZIG" build --help 2>&1 | rg -n -i 'offline|fetch|--system|cache' || true
```

The offline proof is **prefetch-then-deny-network** (Step 3), not a bare `--offline` flag — record whichever mechanism the help text confirms in `build-probe.md → offline_ziglyph`.

- [ ] **Step 2: Slim build + capture (real outputs only)**

```bash
set -euo pipefail   # capture real failures, don't let a broken build masquerade as a closure
UPSTREAM=/tmp/gpui-ghostty-e3025981   # N3
PROBE_DIR=$(mktemp -d /tmp/lens-wp0-probe-XXXXXX)
export ZIG="$UPSTREAM/.context/zig/zig"
# Blocking (R4): use a FRESH per-probe LOCAL cache dir so its input manifest reflects ONLY this build
# (the checkout is reusable; its default `.zig-cache` can hold stale manifests from earlier builds).
LOCAL_CACHE="$PROBE_DIR/zig-local-cache"
( cd "$UPSTREAM/crates/ghostty_vt_sys/zig"
  "$ZIG" build -Doptimize=ReleaseFast --cache-dir "$LOCAL_CACHE" --prefix "$PROBE_DIR/zig-out" --verbose 2>&1
) | tee "$PROBE_DIR/zig-build.log"
test -f "$PROBE_DIR/zig-out/lib/libghostty_vt.a"
```

Expected: exit 0 + static lib. On failure: stop; do not invent closure.

- [ ] **Step 3: Closure / objects / offline / coupling**

```bash
set -euo pipefail
UPSTREAM=/tmp/gpui-ghostty-e3025981   # N3
# Blocking (R3/R4): the AUTHORITATIVE closure is every source Zig actually READ, taken from the
# compiler's own cache input manifests — NOT a grep of `--verbose` args (which lists roots/CLI paths,
# not the transitive `@import` graph). Because Step 2 used a FRESH per-probe LOCAL cache, that cache's
# manifests reflect ONLY this build's inputs → the enumeration is complete by construction (no
# stale-manifest contamination, no sampling). Zig writes input-file lists in the manifests under
# `$LOCAL_CACHE/h/`; enumerate the ghostty_src inputs and make them src-relative:
# NO extension allowlist (Blocking, R5): the VT path @embedFiles non-source assets
# (`src/terminal/res/rgb.txt`, `res/glitch.txt` — verified in the pin), so a `.zig|.h|.c` filter would
# silently drop real inputs and hand WP2 an incomplete closure. Take EVERY ghostty_src manifest input:
rg -oNI 'ghostty_src/[^ "]+' "$LOCAL_CACHE"/h 2>/dev/null | sed 's|^.*ghostty_src/||' | sort -u > "$PROBE_DIR/closure.txt"
test -s "$PROBE_DIR/closure.txt"   # a non-empty manifest closure is required
# Every closure path must exist as a mirror row in the canonical inventory (no orphan, no typo):
test -z "$(comm -23 "$PROBE_DIR/closure.txt" <(awk '/^\[\[mirror\]\]/{m=1;next} m&&/^path = /{gsub(/"/,"");print $3;m=0}' vendor/gpui-ghostty-e3025981/adoption.toml | sort -u))"
# The verbose log is only a SUBSET cross-check — every logged path must appear in the manifest closure:
rg -oNI 'ghostty_src/[^ "]+' "$PROBE_DIR/zig-build.log" | sed 's|^ghostty_src/||' | sort -u > "$PROBE_DIR/closure-from-log.txt"
test -z "$(comm -23 "$PROBE_DIR/closure-from-log.txt" "$PROBE_DIR/closure.txt")"   # log ⊆ manifest, else FAIL
# Optional spot-check (NOT the completeness authority — the fresh-cache manifest is): copy the closure
# into a scratch tree and confirm the offline rebuild still succeeds; a member absent from that copy
# breaking the build confirms it belongs. Record the exact `.zig-cache` manifest path used + input
# count under build-probe `## compile_closure_path_count`.
# N2: Zig 0.14's per-build cache is the LOCAL `.zig-cache/` (not `zig-cache`); the download cache is the GLOBAL dir.
find "$UPSTREAM/crates/ghostty_vt_sys/zig/.zig-cache" "$PROBE_DIR" -name '*.o' 2>/dev/null | sort > "$PROBE_DIR/objects.txt"

# B5 — prove the offline closure WITHOUT assuming a `--offline` flag:
# (a) prefetch every dependency into a DETERMINISTIC global cache while networking is up …
#     (fixed path, not under the random $PROBE_DIR, so Task 7 can bind to it — R3):
export ZIG_GLOBAL_CACHE_DIR=/tmp/lens-wp0-zig-global
( cd "$UPSTREAM/crates/ghostty_vt_sys/zig" && "$ZIG" build --fetch 2>&1 | tee "$PROBE_DIR/zig-fetch.log" )
# (b) … the denied build uses its OWN fresh local cache (not Step 2's $LOCAL_CACHE, whose objects are
#     already compiled), so it is a demonstrable FRESH compile drawing sources only from the prefetched
#     GLOBAL cache — a no-op incremental build would prove nothing (B5, R2/R4).
OFFLINE_CACHE="$PROBE_DIR/zig-local-cache-offline"
# (c) … then rebuild with the network DENIED at the OS level; success == the offline proof.
#     macOS: sandbox-exec deny-network; Linux fallback: `unshare -rn`. Use whichever the box supports.
( cd "$UPSTREAM/crates/ghostty_vt_sys/zig"
  sandbox-exec -p '(version 1)(allow default)(deny network*)' \
    "$ZIG" build -Doptimize=ReleaseFast --cache-dir "$OFFLINE_CACHE" --prefix "$PROBE_DIR/zig-out-offline" 2>&1 ) \
  | tee "$PROBE_DIR/zig-offline.log"
test -f "$PROBE_DIR/zig-out-offline/lib/libghostty_vt.a"   # fresh network-denied build must STILL produce the lib
```

Write `compile-closure.txt` = sorted `$PROBE_DIR/closure.txt` (the fresh-cache manifest closure above; paths relative to `vendor/ghostty/src`). Promote those mirror rows to `adopt` (pure VT) or `adapt` (parse-only Kitty/APC trim). **Retain probe evidence (Blocking, R4):** copy `zig-build.log` + `zig-offline.log` + `zig-fetch.log` into `vendor/gpui-ghostty-e3025981/probe-logs/` and record each one's `shasum -a 256` in `build-probe.md`; these committed logs (not `/tmp`) are the independently reviewable evidence. Fill each `build-probe.md` section with **captured** values: `zig version`; bootstrap sha used on this arch; exact build command; exit `0`; object list or `shasum` of `objects.txt` plus count; ziglyph URL/hash; `wc -l` of closure; `offline_ziglyph` = `pass`/`fail` where **`pass` means the fresh network-denied rebuild in step (c) succeeded** (fail fails the gate) and the body names the mechanism used (`sandbox-exec deny network` / `unshare -rn`); renderer/cell coupling conclusion (**no GPUI image path linked**; parse-only APC OK with WP2 blank-render note); `shasum -a 256` of `libghostty_vt.a`; `license_closure` = **`Apache-2.0 + MIT-Ghostty + MIT-ziglyph`** (plus any further nested license the fetch surfaced — **B6**); `raw_log_path=probe-logs/zig-build.log` (**committed**) + `raw_log_sha256`.

- [ ] **Step 4: Sanity**

```bash
test -s vendor/gpui-ghostty-e3025981/compile-closure.txt
rg -n '^## (zig_version|artifact_sha256|offline_ziglyph|license_closure)$' vendor/gpui-ghostty-e3025981/build-probe.md
rg -n 'MIT-ziglyph' vendor/gpui-ghostty-e3025981/build-probe.md   # B6: ziglyph in the license closure
rg -n 'FIXTURE' vendor/gpui-ghostty-e3025981/build-probe.md && exit 1 || true   # Vendor-mode marker guard (B7)
```

- [ ] **Step 5: Commit**

```bash
git add vendor/gpui-ghostty-e3025981/build-probe.md vendor/gpui-ghostty-e3025981/compile-closure.txt \
  vendor/gpui-ghostty-e3025981/probe-logs \
  vendor/gpui-ghostty-e3025981/adoption.toml vendor/gpui-ghostty-e3025981/provenance.toml
git commit -m "$(cat <<'EOF'
chore(vendor): capture Zig 0.14.1 terminal build probe

EOF
)"
```

---

### Task 6: GPUI 0.2.2 git-vs-registry reconciliation

**Files:** Create `gpui-reconciliation.md`.

- [ ] **Step 1: Collect evidence**

```bash
UPSTREAM=/tmp/gpui-ghostty-e3025981   # N3: standalone
rg -n 'gpui' "$UPSTREAM/Cargo.toml" "$UPSTREAM/crates/gpui_ghostty_terminal/Cargo.toml"
rg -n -A3 '^name = "gpui"$' "$UPSTREAM/Cargo.lock" | head -20
rg -n -A3 '^name = "gpui"$' Cargo.lock | head -20
```

Known: upstream git `zed#cff3ac6f93f506330034652f0d2389591bfb45a0` / `0.2.2`; Lens registry `0.2.2` checksum `979b45cfa6ec723b6f42330915a1b3769b930d02b2d505f9697f8ca602bee707`. Upstream README GPUI pin is stale — lockfile wins.

- [ ] **Step 2: Write `gpui-reconciliation.md`**

Must include:

```markdown
## Decision
strategy = single_crates_io_0_2_2
```

Plus Upstream/Lens tables; adaptation-risk rows for `view/mod.rs`, `font.rs`, `tests.rs`, `examples/basic_terminal/src/main.rs` with WP2 actions (rebind Canvas/text/input to registry; verify fonts; retarget tests; demo is WP3 rewrite). Duplicate-GPUI guard:

```bash
cargo tree -p lens-store | rg 'gpui v' | sort -u
# WP2 gate: cargo tree --workspace | rg 'gpui v' | sort -u  → FAIL if >1 distinct gpui source
```

Record real symbol mismatches found while reading the four risk files; if a symbol cannot be confirmed offline, mark `unverified_at_wp0 — WP2 must confirm before adopt compile` while keeping single-registry strategy.

- [ ] **Step 3: Commit**

```bash
git add vendor/gpui-ghostty-e3025981/gpui-reconciliation.md
git commit -m "$(cat <<'EOF'
docs(vendor): reconcile gpui-ghostty GPUI git pin to crates.io 0.2.2

EOF
)"
```

---

### Task 7: Licenses, upstream tests, framework/STATUS, forbidden-import checks

**Files:** Create licenses + `upstream-tests.toml`; modify `framework.md`, `STATUS.md`, `README.md`.

- [ ] **Step 1: Copy licenses verbatim (Apache-2.0 + MIT-Ghostty + MIT-ziglyph — B6)**

```bash
UPSTREAM=/tmp/gpui-ghostty-e3025981   # N3
mkdir -p vendor/gpui-ghostty-e3025981/licenses   # New-Blocking (R2): dir may not exist yet
cp "$UPSTREAM/LICENSE" vendor/gpui-ghostty-e3025981/licenses/Apache-2.0.txt
cp "$UPSTREAM/vendor/ghostty/LICENSE" vendor/gpui-ghostty-e3025981/licenses/MIT-Ghostty.txt

# ziglyph is statically linked → its license is load-bearing. Bind the copy to the EXACT pinned
# package by hash (R3): Zig 0.14 names the package cache dir by the `.hash` from build.zig.zon, which
# is our provenance `ziglyph_hash`. Use the deterministic Task-5 cache; no zsh brace-glob.
ZIG_GLOBAL_CACHE_DIR="${ZIG_GLOBAL_CACHE_DIR:-/tmp/lens-wp0-zig-global}"
ZIGLYPH_HASH="ziglyph-0.11.2-AAAAAHPtHwB4Mbzn1KvOV7Wpjo82NYEc_v0WC8oCLrkf"   # == provenance ziglyph_hash
ZIGLYPH_PKG="$ZIG_GLOBAL_CACHE_DIR/p/$ZIGLYPH_HASH"
test -d "$ZIGLYPH_PKG"   # the EXACT pinned package must be present — binds the copy to ziglyph_hash
ZIGLYPH_LICENSE=$(find "$ZIGLYPH_PKG" -maxdepth 1 -iname 'licen[cs]e*' -type f 2>/dev/null | head -1)
test -n "$ZIGLYPH_LICENSE"   # fail loudly if the license file is missing
cp "$ZIGLYPH_LICENSE" vendor/gpui-ghostty-e3025981/licenses/MIT-ziglyph.txt
test -s vendor/gpui-ghostty-e3025981/licenses/MIT-ziglyph.txt   # must exist and be non-empty

# Record ziglyph_license_source (the upstream path inside the pinned package) + ziglyph_license_sha256
# into provenance.toml, then confirm the recorded hash matches the copied file (verifier re-checks this):
shasum -a 256 vendor/gpui-ghostty-e3025981/licenses/*
# → set provenance.toml:
#     ziglyph_license_source = "<relative path of $ZIGLYPH_LICENSE inside the ziglyph package>"
#     ziglyph_license_sha256 = "<shasum of MIT-ziglyph.txt above>"
# Copy any further nested license the build-probe license_closure surfaced and add its provenance row too.
```

Expected: Apache-2.0 / MIT-Ghostty / MIT-ziglyph present; `ziglyph_license_sha256` in `provenance.toml` matches `shasum -a 256` of the copied `MIT-ziglyph.txt`.

- [ ] **Step 2: `upstream-tests.toml`** — all ten `crates/ghostty_vt/tests/{smoke,dirty_rows,full_redraw_dirty_rows,hyperlink,key_encoder,osc_palette,scroll_and_resize,viewport_scroll_delta,charset_dec_special,style_dump}.rs` with `relevant = true`; plus `crates/gpui_ghostty_terminal/src/tests.rs` and `…/view/mod.rs` (`relevant = true`, note GPUI adapt). Verifier: each relevant path exists as a wrapper row.

- [ ] **Step 3: Docs**

`framework.md` §2.2 add:

```markdown
**WP0 provenance artifacts:** `vendor/gpui-ghostty-e3025981/` (`provenance.toml`,
`adoption.toml`, `gpui-reconciliation.md`, `build-probe.md`). No upstream source is
vendored here; WP2 imports only adopt/adapt rows after Opus approval.
```

`STATUS.md` terminal thread: point at `vendor/gpui-ghostty-e3025981/` and
`cargo run -p xtask -- terminal-provenance --root vendor/gpui-ghostty-e3025981 --upstream /tmp/gpui-ghostty-e3025981`;
state WP1 may parallel; **WP2 must not start until WP0 is committed and Opus-approved.** Mark complete only after Task 8 APPROVE.

- [ ] **Step 4: Forbidden import checks** (**B4**: exclude `crates/xtask/**` — the verifier legitimately names these constants)

```bash
git ls-files 'crates/**' | rg -v '^crates/xtask/' | rg -i 'ghostty|zig/lib\.zig' && exit 1 || echo ok
rg -n 'portable-pty|gpui-ghostty|ghostty_vt' crates --glob '*.rs' --glob '!crates/xtask/**' && exit 1 || echo ok
test ! -d crates/lens-terminal
```

- [ ] **Step 5: Commit**

```bash
git add vendor/gpui-ghostty-e3025981/licenses vendor/gpui-ghostty-e3025981/upstream-tests.toml \
  vendor/gpui-ghostty-e3025981/provenance.toml \
  vendor/gpui-ghostty-e3025981/README.md docs/design/framework.md docs/STATUS.md
git commit -m "$(cat <<'EOF'
docs: retain terminal upstream licenses and point STATUS at WP0

EOF
)"
```

---

### Task 8: Full gates + Opus 4.8 review + self-review

- [ ] **Step 1: Verifier on real artifacts**

```bash
# --upstream selects VerificationMode::Vendor (B7): FIXTURE markers / all-zero hashes now fail.
cargo run -p xtask -- terminal-provenance \
  --root vendor/gpui-ghostty-e3025981 \
  --upstream /tmp/gpui-ghostty-e3025981
```

Expected: `terminal-provenance: ok` / exit 0.

- [ ] **Step 2: Tests / fmt / clippy / diff**

```bash
cargo test -p xtask
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
git diff --check
```

Expected: all exit 0.

- [ ] **Step 3: Opus 4.8 review (`--model opus` → `claude-opus-4-8`)**

```bash
claude --model opus -p "$(cat <<'EOF'
You are the independent high-risk reviewer for Lens WP0 terminal provenance.
Read-only. Review vendor/gpui-ghostty-e3025981/{provenance.toml,adoption.toml,gpui-reconciliation.md,build-probe.md,compile-closure.txt,source-archives.sha256,upstream-tests.toml,licenses/*,README.md}, crates/xtask/src/{lib.rs,main.rs,terminal_provenance.rs}, crates/xtask/tests/terminal_provenance.rs, crates/xtask/tests/fixtures/terminal-provenance/**, and docs/plans/2026-07-15-terminal-wp0-provenance.md.
Require: pins e3025981 + 6d2dd585; Zig 0.14.1; baseline license closure = Apache-2.0 + MIT-Ghostty + MIT-ziglyph, all retained verbatim; every wrapper+mirror path classified; forbidden PTY/graphics/packaging not adopted; single crates.io gpui 0.2.2 strategy; no upstream source in Lens crates; offline proof is a network-denied rebuild (not a bare flag); build-probe run in Vendor mode with no FIXTURE markers and no all-zero archive hashes; build-probe fields complete and not fabricated.
End with exactly one line: APPROVE or REJECT <reason>.
EOF
)"
```

**SHA-binding protocol (Blocking, R3):** the approval record must name the exact tree Opus approved, so
**review only a clean, committed tree and never fold fixes into the approval commit.** Precondition for
Step 3: Tasks 1–7 are all committed and `git status --porcelain` is empty. Then:

The Step 5 author self-review is a **precondition** of every review pass (run it before Step 3, not after).

- **APPROVE** → go to Step 3b. The reviewed SHA = current `HEAD` (unchanged since review).
- **REJECT** → **reject loop (Important, R4):** apply fixes, commit them as their own `fix(wp0): …`
  commit, then **re-run the FULL gate before the next review — Step 1 (verifier), Step 2 (tests / fmt /
  clippy / diff), and Step 5 (self-review) — against the new `HEAD`**, and only then re-run Step 3.
  Repeat until APPROVE. Never carry uncommitted fixes, a stale gate, or a stale self-review into a review
  or the approval commit.

- [ ] **Step 3b: Record the approval separately (R2/R3 Important)** — with the tree clean and APPROVED,
write `docs/plans/2026-07-15-terminal-wp0-provenance-opus-approval.md` (NOT inside the immutable review
doc) capturing: **the approved `HEAD` SHA (`git rev-parse HEAD`, which equals the reviewed tree because
no fix was applied after this review)**, the Claude review session id, the model (`claude-opus-4-8`), the
SHA-256 of the reviewed plan, and the verbatim final `APPROVE` line. Mirrors the provenance-record
convention of `…-opus-review.md`.

- [ ] **Step 4b: Commit ONLY the approval record** — `git add
docs/plans/2026-07-15-terminal-wp0-provenance-opus-approval.md` and commit
`docs(wp0): record Opus APPROVE of <approved-SHA>`. Because it is the sole change in this commit, the
approved-SHA it names is the parent commit — the exact tree Opus reviewed.

- [ ] **Step 5: Author self-review**

- [ ] Spec grounded-adoption + roadmap WP0 acceptance mapped to Tasks 3–8
- [ ] No unfinished markers or deferred hedges in this plan
- [ ] `VerifyError` / `VerificationMode` / CLI / artifact schemas consistent across tasks (B1, B7)
- [ ] Valid fixture is self-consistent: compile-closure member is adopt/adapt (B2), every `relevant` test has a wrapper row (B3)
- [ ] Forbidden-import scans exclude `crates/xtask/**` (B4)
- [ ] Offline proof is a network-denied rebuild after prefetch, not an unverified flag (B5)
- [ ] Baseline license closure = Apache-2.0 + MIT-Ghostty + MIT-ziglyph, all copied verbatim (B6)
- [ ] Fixtures load in `Fixture` mode; real vendor audit runs in `Vendor` mode and rejects `FIXTURE`/zero-hashes (B7)
- [ ] No `crates/lens-terminal` and no upstream source import in WP0
- [ ] Build-probe body is capture commands + required headings — plan invents no probe hashes/closures
- [ ] Explicit gate: WP2 is first package allowed to import approved sources after WP0 commit + Opus APPROVE

---

## Coverage map

| Obligation | Task |
| --- | --- |
| Pins + archive hashes | 3, 4 |
| Exhaustive adopt/adapt/exclude | 4, 5 |
| Zig probe / offline / closure | 5 |
| Licenses + upstream tests + docs | 7 |
| GPUI single-dep reconciliation | 6 |
| Typed verifier TDD + CLI | 1, 2 |
| Opus 4.8 + workspace gates | 8 |
| No source import until WP2 | Global, 3, 7, 8 |
