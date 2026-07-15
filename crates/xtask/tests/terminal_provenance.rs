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
