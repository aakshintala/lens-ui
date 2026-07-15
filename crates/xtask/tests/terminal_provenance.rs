use std::path::PathBuf;
use xtask::terminal_provenance::{VerificationMode, VerifyError, load_and_verify};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/terminal-provenance")
        .join(name)
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
    assert!(
        e.iter()
            .any(|x| matches!(x, VerifyError::FixtureMarkerInVendorMode { .. }))
    );
}

#[test]
fn missing_pin_fails() {
    let e = load_and_verify(&fixture("missing-pin"), VerificationMode::Fixture).unwrap_err();
    assert!(
        e.iter()
            .any(|x| matches!(x, VerifyError::MissingPin { .. }))
    );
}
#[test]
fn unknown_disposition_fails() {
    let e =
        load_and_verify(&fixture("unknown-disposition"), VerificationMode::Fixture).unwrap_err();
    assert!(
        e.iter()
            .any(|x| matches!(x, VerifyError::UnknownDisposition { .. }))
    );
}
#[test]
fn duplicate_path_fails() {
    let e = load_and_verify(&fixture("duplicate-path"), VerificationMode::Fixture).unwrap_err();
    assert!(
        e.iter()
            .any(|x| matches!(x, VerifyError::DuplicatePath { .. }))
    );
}
#[test]
fn missing_license_fails() {
    let e = load_and_verify(&fixture("missing-license"), VerificationMode::Fixture).unwrap_err();
    assert!(
        e.iter()
            .any(|x| matches!(x, VerifyError::MissingLicenseMapping { .. }))
    );
}
#[test]
fn forbidden_adopt_fails() {
    let e = load_and_verify(&fixture("forbidden-adopt"), VerificationMode::Fixture).unwrap_err();
    assert!(
        e.iter()
            .any(|x| matches!(x, VerifyError::ForbiddenAdopt { .. }))
    );
}

#[test]
fn cli_rejects_missing_pin_fixture() {
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["terminal-provenance", "--root"])
        .arg(fixture("missing-pin"))
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(1));
}

// B7 (R2/R3): the SAME valid fixture passes without --upstream (Fixture mode) but fails with
// --upstream (Vendor mode), AND the failure output NAMES the Vendor-mode marker — so mode selection,
// not incidental upstream enumeration, is the proven cause (assert on the variant, not just exit 1).
#[test]
fn cli_valid_fixture_passes_fixture_mode_but_fails_vendor_mode() {
    let ok = std::process::Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["terminal-provenance", "--root"])
        .arg(fixture("valid"))
        .output()
        .unwrap();
    assert_eq!(
        ok.status.code(),
        Some(0),
        "no --upstream ⇒ Fixture mode ⇒ pass"
    );

    let vendor = std::process::Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["terminal-provenance", "--root"])
        .arg(fixture("valid"))
        .arg("--upstream")
        .arg(fixture("valid"))
        .output()
        .unwrap();
    assert_eq!(vendor.status.code(), Some(1));
    let out = format!(
        "{}{}",
        String::from_utf8_lossy(&vendor.stdout),
        String::from_utf8_lossy(&vendor.stderr)
    );
    assert!(
        out.contains("FixtureMarkerInVendorMode"),
        "Vendor mode (not upstream enumeration) must be the proven cause; got:\n{out}"
    );
}

#[test]
fn vendor_blank_probe_section_fails() {
    let e = load_and_verify(&fixture("vendor-blank-probe"), VerificationMode::Vendor).unwrap_err();
    assert!(
        e.iter()
            .any(|x| matches!(x, VerifyError::FixtureMarkerInVendorMode { .. }))
    );
}

#[test]
fn vendor_single_archive_hash_fails() {
    let e = load_and_verify(&fixture("vendor-one-hash"), VerificationMode::Vendor).unwrap_err();
    assert!(
        e.iter()
            .any(|x| matches!(x, VerifyError::FixtureMarkerInVendorMode { .. }))
    );
}

#[test]
fn vendor_duplicate_archive_name_fails() {
    let e = load_and_verify(&fixture("vendor-dup-archive"), VerificationMode::Vendor).unwrap_err();
    assert!(
        e.iter()
            .any(|x| matches!(x, VerifyError::FixtureMarkerInVendorMode { .. }))
    );
}

#[test]
fn vendor_wrong_archive_name_fails() {
    let e =
        load_and_verify(&fixture("vendor-wrong-archive"), VerificationMode::Vendor).unwrap_err();
    assert!(
        e.iter()
            .any(|x| matches!(x, VerifyError::FixtureMarkerInVendorMode { .. }))
    );
}
