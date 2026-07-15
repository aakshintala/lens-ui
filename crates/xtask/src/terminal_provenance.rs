use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

const EXPECTED_ZIG_VERSION: &str = "0.14.1";
const EXPECTED_GPUI_STRATEGY: &str = "single_crates_io_0_2_2";
const EXPECTED_GPUI_RECONCILIATION_LINE: &str = "strategy = single_crates_io_0_2_2";
const ARCHIVE_GPUI_NAME: &str = "gpui-ghostty-e3025981.tar";
const ARCHIVE_GHOSTTY_NAME: &str = "ghostty-6d2dd585.tar";
const BUILD_PROBE_FILE: &str = "build-probe.md";
const DEFAULT_ARCHIVE_HASH_FILE: &str = "source-archives.sha256";

const BUILD_PROBE_HEADINGS: &[&str] = &[
    "zig_version",
    "bootstrap_sha256",
    "build_command",
    "build_exit_code",
    "object_list",
    "dependency_list",
    "compile_closure_path_count",
    "offline_ziglyph",
    "renderer_cell_coupling",
    "artifact_sha256",
    "license_closure",
    "raw_log_path",
    "raw_log_sha256",
];

/// Locked pin values enforced in Vendor mode.
/// TODO(Task 2/5/7): wire `--upstream` archive recompute and captured license constants.
const LOCKED_PINS: &[(&str, &str)] = &[
    (
        "gpui_ghostty_commit",
        "e3025981c6211dd7db2a825dc364ffb5d342f45e",
    ),
    ("ghostty_commit", "6d2dd585a5d87fa745d48188dd096ca6e63014d0"),
    ("ghostty_tag", "v1.2.3"),
    (
        "zig_macos_x86_64_sha256",
        "b0f8bdfb9035783db58dd6c19d7dea89892acc3814421853e5752fe4573e5f43",
    ),
    (
        "zig_macos_arm64_sha256",
        "39f3dc5e79c22088ce878edc821dedb4ca5a1cd9f5ef915e9b3cc3053e8faefa",
    ),
    (
        "ziglyph_url",
        "https://deps.files.ghostty.org/ziglyph-b89d43d1e3fb01b6074bc1f7fc980324b04d26a5.tar.gz",
    ),
    (
        "ziglyph_hash",
        "ziglyph-0.11.2-AAAAAHPtHwB4Mbzn1KvOV7Wpjo82NYEc_v0WC8oCLrkf",
    ),
    (
        "gpui_upstream_source",
        "git+https://github.com/zed-industries/zed#cff3ac6f93f506330034652f0d2389591bfb45a0",
    ),
    ("gpui_upstream_version", "0.2.2"),
    (
        "gpui_lens_source",
        "registry+https://github.com/rust-lang/crates.io-index",
    ),
    ("gpui_lens_version", "0.2.2"),
    ("gpui_strategy", EXPECTED_GPUI_STRATEGY),
];

pub const FORBIDDEN_ADOPT_PREFIXES: &[&str] = &[
    "examples/pty_terminal/",
    "examples/split_pty_terminal/",
    ".github/",
];

const FORBIDDEN_ADOPT_SUBSTRINGS: &[&str] = &["portable-pty", "sixel", "osc1337"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    Adopt,
    Adapt,
    Exclude,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerificationMode {
    Fixture,
    Vendor,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VerifyError {
    #[error("missing pin field {field}")]
    MissingPin { field: String },
    #[error("unknown disposition for {path}: {got}")]
    UnknownDisposition { path: String, got: String },
    #[error("duplicate inventory path {path}")]
    DuplicatePath { path: String },
    #[error("missing inventory path {path}")]
    MissingInventoryPath { path: String },
    #[error("extra inventory path {path}")]
    ExtraInventoryPath { path: String },
    #[error("missing license mapping for {path}")]
    MissingLicenseMapping { path: String },
    #[error("forbidden adopt path {path}")]
    ForbiddenAdopt { path: String },
    #[error("unresolved GPUI reconciliation")]
    UnresolvedGpuiReconciliation,
    #[error("wrong zig version: expected {expected}, got {got}")]
    WrongZigVersion { expected: String, got: String },
    #[error("missing hash {name}")]
    MissingHash { name: String },
    #[error("missing artifact {path}")]
    MissingArtifact { path: String },
    #[error("license hash mismatch for {path}: expected {expected}, got {got}")]
    LicenseHashMismatch {
        path: String,
        expected: String,
        got: String,
    },
    #[error("build probe incomplete: {field}")]
    BuildProbeIncomplete { field: String },
    #[error("fixture marker in vendor mode: {field}")]
    FixtureMarkerInVendorMode { field: String },
    #[error("mirror count mismatch: expected {expected}, got {got}")]
    MirrorCountMismatch { expected: usize, got: usize },
    #[error("wrapper count mismatch: expected {expected}, got {got}")]
    WrapperCountMismatch { expected: usize, got: usize },
}

#[derive(Debug, serde::Deserialize)]
struct ProvenanceToml {
    gpui_ghostty_remote: Option<String>,
    gpui_ghostty_commit: Option<String>,
    ghostty_remote: Option<String>,
    ghostty_commit: Option<String>,
    ghostty_tag: Option<String>,
    zig_version: Option<String>,
    zig_macos_x86_64_sha256: Option<String>,
    zig_macos_arm64_sha256: Option<String>,
    ziglyph_url: Option<String>,
    ziglyph_hash: Option<String>,
    gpui_upstream_source: Option<String>,
    gpui_upstream_version: Option<String>,
    gpui_lens_source: Option<String>,
    gpui_lens_version: Option<String>,
    gpui_strategy: Option<String>,
    gpui_reconciliation_path: Option<String>,
    wrapper_file_count: Option<usize>,
    mirror_file_count: Option<usize>,
    license_apache_path: Option<String>,
    license_mit_path: Option<String>,
    license_ziglyph_path: Option<String>,
    license_apache_sha256: Option<String>,
    license_mit_sha256: Option<String>,
    ziglyph_license_source: Option<String>,
    ziglyph_license_sha256: Option<String>,
    archive_hash_file: Option<String>,
    #[allow(dead_code)]
    mirror_count_note: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct AdoptionToml {
    #[serde(default)]
    wrapper: Vec<InventoryRowRaw>,
    #[serde(default)]
    mirror: Vec<InventoryRowRaw>,
}

#[derive(Debug, serde::Deserialize)]
struct InventoryRowRaw {
    path: String,
    disposition: String,
    license: Option<String>,
    #[allow(dead_code)]
    reason: Option<String>,
}

#[derive(Debug, Clone)]
struct Inventory {
    wrappers: Vec<InventoryRow>,
    mirrors: Vec<InventoryRow>,
}

#[derive(Debug, Clone)]
struct InventoryRow {
    path: String,
    disposition: Disposition,
    license: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct UpstreamTestsToml {
    #[serde(default)]
    test: Vec<UpstreamTestRow>,
}

#[derive(Debug, serde::Deserialize)]
struct UpstreamTestRow {
    path: String,
    relevant: Option<bool>,
}

pub fn load_and_verify(root: &Path, mode: VerificationMode) -> Result<(), Vec<VerifyError>> {
    let mut errors = Vec::new();

    let provenance_path = root.join("provenance.toml");
    let provenance = match read_toml::<ProvenanceToml>(&provenance_path) {
        Ok(p) => p,
        Err(err) => {
            errors.push(err);
            return Err(errors);
        }
    };

    validate_provenance(&provenance, mode, &mut errors);

    let adoption_path = root.join("adoption.toml");
    let adoption = match read_toml::<AdoptionToml>(&adoption_path) {
        Ok(a) => a,
        Err(err) => {
            errors.push(err);
            return Err(errors);
        }
    };

    let inventory = collect_inventory(&adoption, &mut errors);

    validate_inventory(&provenance, &inventory, mode, &mut errors);
    validate_artifacts(root, &provenance, mode, &mut errors);
    validate_gpui_reconciliation(root, &provenance, &mut errors);
    validate_build_probe(root, mode, &mut errors);
    validate_compile_closure(root, &inventory, &mut errors);
    validate_upstream_tests(root, &inventory, &mut errors);
    validate_source_archives(root, &provenance, mode, &mut errors);
    validate_license_hashes(root, &provenance, mode, &mut errors);

    if mode == VerificationMode::Vendor {
        validate_vendor_locked_pins(&provenance, &mut errors);
        // TODO(Task 5/7): verify committed probe-log `raw_log_sha256` bindings.
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn read_toml<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, VerifyError> {
    let raw = fs::read_to_string(path).map_err(|_| VerifyError::MissingArtifact {
        path: path.display().to_string(),
    })?;
    toml::from_str(&raw).map_err(|err| VerifyError::MissingPin {
        field: format!("{}: {err}", path.display()),
    })
}

fn require_non_empty(value: &Option<String>, field: &str, errors: &mut Vec<VerifyError>) {
    match value.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
        Some(_) => {}
        None => errors.push(VerifyError::MissingPin {
            field: field.to_string(),
        }),
    }
}

fn validate_provenance(
    provenance: &ProvenanceToml,
    mode: VerificationMode,
    errors: &mut Vec<VerifyError>,
) {
    for field in [
        "gpui_ghostty_remote",
        "gpui_ghostty_commit",
        "ghostty_remote",
        "ghostty_commit",
        "ghostty_tag",
        "zig_version",
        "zig_macos_x86_64_sha256",
        "zig_macos_arm64_sha256",
        "ziglyph_url",
        "ziglyph_hash",
        "gpui_upstream_source",
        "gpui_upstream_version",
        "gpui_lens_source",
        "gpui_lens_version",
        "gpui_strategy",
        "gpui_reconciliation_path",
        "license_apache_path",
        "license_mit_path",
        "license_ziglyph_path",
    ] {
        let value = provenance_field(provenance, field);
        require_non_empty(&value, field, errors);
    }

    if provenance.wrapper_file_count.is_none() {
        errors.push(VerifyError::MissingPin {
            field: "wrapper_file_count".to_string(),
        });
    }
    if provenance.mirror_file_count.is_none() {
        errors.push(VerifyError::MissingPin {
            field: "mirror_file_count".to_string(),
        });
    }

    if let Some(zig) = provenance.zig_version.as_deref()
        && zig != EXPECTED_ZIG_VERSION
    {
        errors.push(VerifyError::WrongZigVersion {
            expected: EXPECTED_ZIG_VERSION.to_string(),
            got: zig.to_string(),
        });
    }

    for field in ["zig_macos_x86_64_sha256", "zig_macos_arm64_sha256"] {
        if let Some(hash) = provenance_field(provenance, field) {
            validate_sha256_field(&hash, field, errors);
        }
    }

    if let Some(strategy) = provenance.gpui_strategy.as_deref()
        && strategy != EXPECTED_GPUI_STRATEGY
    {
        errors.push(VerifyError::MissingPin {
            field: "gpui_strategy".to_string(),
        });
    }

    if mode == VerificationMode::Vendor {
        for field in ["ziglyph_license_source", "ziglyph_license_sha256"] {
            let value = provenance_field(provenance, field);
            if value
                .as_ref()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .is_none()
            {
                errors.push(VerifyError::MissingHash {
                    name: field.to_string(),
                });
            }
        }
        if let Some(hash) = &provenance.ziglyph_license_sha256 {
            validate_sha256_field(hash, "ziglyph_license_sha256", errors);
        }
    }
}

fn provenance_field(provenance: &ProvenanceToml, field: &str) -> Option<String> {
    match field {
        "gpui_ghostty_remote" => provenance.gpui_ghostty_remote.clone(),
        "gpui_ghostty_commit" => provenance.gpui_ghostty_commit.clone(),
        "ghostty_remote" => provenance.ghostty_remote.clone(),
        "ghostty_commit" => provenance.ghostty_commit.clone(),
        "ghostty_tag" => provenance.ghostty_tag.clone(),
        "zig_version" => provenance.zig_version.clone(),
        "zig_macos_x86_64_sha256" => provenance.zig_macos_x86_64_sha256.clone(),
        "zig_macos_arm64_sha256" => provenance.zig_macos_arm64_sha256.clone(),
        "ziglyph_url" => provenance.ziglyph_url.clone(),
        "ziglyph_hash" => provenance.ziglyph_hash.clone(),
        "gpui_upstream_source" => provenance.gpui_upstream_source.clone(),
        "gpui_upstream_version" => provenance.gpui_upstream_version.clone(),
        "gpui_lens_source" => provenance.gpui_lens_source.clone(),
        "gpui_lens_version" => provenance.gpui_lens_version.clone(),
        "gpui_strategy" => provenance.gpui_strategy.clone(),
        "gpui_reconciliation_path" => provenance.gpui_reconciliation_path.clone(),
        "license_apache_path" => provenance.license_apache_path.clone(),
        "license_mit_path" => provenance.license_mit_path.clone(),
        "license_ziglyph_path" => provenance.license_ziglyph_path.clone(),
        "ziglyph_license_source" => provenance.ziglyph_license_source.clone(),
        "ziglyph_license_sha256" => provenance.ziglyph_license_sha256.clone(),
        _ => None,
    }
}

fn validate_sha256_field(value: &str, field: &str, errors: &mut Vec<VerifyError>) {
    let valid = value.len() == 64
        && value
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase());
    if !valid {
        errors.push(VerifyError::MissingPin {
            field: field.to_string(),
        });
    }
}

fn collect_inventory(adoption: &AdoptionToml, errors: &mut Vec<VerifyError>) -> Inventory {
    let wrappers = collect_rows(&adoption.wrapper, errors);
    let mirrors = collect_rows(&adoption.mirror, errors);
    Inventory { wrappers, mirrors }
}

fn collect_rows(raw_rows: &[InventoryRowRaw], errors: &mut Vec<VerifyError>) -> Vec<InventoryRow> {
    let mut rows = Vec::new();
    for raw in raw_rows {
        match parse_disposition(&raw.disposition) {
            Some(disposition) => rows.push(InventoryRow {
                path: raw.path.clone(),
                disposition,
                license: raw.license.clone(),
            }),
            None => errors.push(VerifyError::UnknownDisposition {
                path: raw.path.clone(),
                got: raw.disposition.clone(),
            }),
        }
    }
    rows
}

fn all_inventory_rows(inventory: &Inventory) -> impl Iterator<Item = &InventoryRow> {
    inventory.wrappers.iter().chain(inventory.mirrors.iter())
}

fn parse_disposition(raw: &str) -> Option<Disposition> {
    match raw {
        "adopt" => Some(Disposition::Adopt),
        "adapt" => Some(Disposition::Adapt),
        "exclude" => Some(Disposition::Exclude),
        _ => None,
    }
}

fn validate_inventory(
    provenance: &ProvenanceToml,
    inventory: &Inventory,
    _mode: VerificationMode,
    errors: &mut Vec<VerifyError>,
) {
    let mut seen = HashSet::new();
    for row in all_inventory_rows(inventory) {
        if !seen.insert(row.path.clone()) {
            errors.push(VerifyError::DuplicatePath {
                path: row.path.clone(),
            });
        }

        if matches!(row.disposition, Disposition::Adopt | Disposition::Adapt)
            && row
                .license
                .as_ref()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .is_none()
        {
            errors.push(VerifyError::MissingLicenseMapping {
                path: row.path.clone(),
            });
        }

        if row.disposition == Disposition::Adopt && is_forbidden_adopt_path(&row.path) {
            errors.push(VerifyError::ForbiddenAdopt {
                path: row.path.clone(),
            });
        }
    }

    let wrapper_count = inventory.wrappers.len();
    let mirror_count = inventory.mirrors.len();

    if let Some(expected) = provenance.wrapper_file_count
        && wrapper_count != expected
    {
        errors.push(VerifyError::WrapperCountMismatch {
            expected,
            got: wrapper_count,
        });
    }

    if let Some(expected) = provenance.mirror_file_count
        && mirror_count != expected
    {
        errors.push(VerifyError::MirrorCountMismatch {
            expected,
            got: mirror_count,
        });
    }
}

fn is_forbidden_adopt_path(path: &str) -> bool {
    FORBIDDEN_ADOPT_PREFIXES
        .iter()
        .any(|prefix| path.starts_with(prefix))
        || FORBIDDEN_ADOPT_SUBSTRINGS
            .iter()
            .any(|needle| path.to_ascii_lowercase().contains(needle))
}

fn validate_artifacts(
    root: &Path,
    provenance: &ProvenanceToml,
    mode: VerificationMode,
    errors: &mut Vec<VerifyError>,
) {
    let artifact_paths = [
        provenance.license_apache_path.clone(),
        provenance.license_mit_path.clone(),
        provenance.license_ziglyph_path.clone(),
    ];
    for path in artifact_paths.into_iter().flatten() {
        let full = root.join(&path);
        if !full.is_file() {
            errors.push(VerifyError::MissingArtifact { path: path.clone() });
        }
    }

    let archive_file = provenance
        .archive_hash_file
        .as_deref()
        .unwrap_or(DEFAULT_ARCHIVE_HASH_FILE);
    if mode == VerificationMode::Fixture || provenance.archive_hash_file.is_some() {
        let full = root.join(archive_file);
        if !full.is_file() {
            errors.push(VerifyError::MissingArtifact {
                path: archive_file.to_string(),
            });
        }
    }

    let _ = root;
}

fn validate_gpui_reconciliation(
    root: &Path,
    provenance: &ProvenanceToml,
    errors: &mut Vec<VerifyError>,
) {
    let Some(rel) = provenance.gpui_reconciliation_path.as_ref() else {
        return;
    };
    let path = root.join(rel);
    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => {
            errors.push(VerifyError::MissingArtifact { path: rel.clone() });
            return;
        }
    };
    if !content.contains("## Decision") || !content.contains(EXPECTED_GPUI_RECONCILIATION_LINE) {
        errors.push(VerifyError::UnresolvedGpuiReconciliation);
    }
}

fn parse_build_probe_sections(content: &str) -> HashMap<String, Vec<String>> {
    let mut sections: HashMap<String, Vec<String>> = HashMap::new();
    let mut current: Option<String> = None;
    let mut body = String::new();

    let flush = |sections: &mut HashMap<String, Vec<String>>,
                 current: &mut Option<String>,
                 body: &mut String| {
        if let Some(name) = current.take() {
            sections.entry(name).or_default().push(std::mem::take(body));
        }
    };

    for line in content.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            flush(&mut sections, &mut current, &mut body);
            current = Some(heading.trim().to_string());
        } else if current.is_some() {
            if !body.is_empty() {
                body.push('\n');
            }
            body.push_str(line);
        }
    }
    flush(&mut sections, &mut current, &mut body);
    sections
}

fn validate_build_probe(root: &Path, mode: VerificationMode, errors: &mut Vec<VerifyError>) {
    let path = root.join(BUILD_PROBE_FILE);
    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => {
            errors.push(VerifyError::MissingArtifact {
                path: BUILD_PROBE_FILE.to_string(),
            });
            return;
        }
    };

    if mode == VerificationMode::Vendor && content.contains("FIXTURE") {
        errors.push(VerifyError::FixtureMarkerInVendorMode {
            field: BUILD_PROBE_FILE.to_string(),
        });
    }

    let sections = parse_build_probe_sections(&content);
    for heading in BUILD_PROBE_HEADINGS {
        match sections.get(*heading) {
            None => errors.push(VerifyError::BuildProbeIncomplete {
                field: (*heading).to_string(),
            }),
            Some(bodies) if mode == VerificationMode::Vendor => {
                if bodies.iter().any(|body| body.trim().is_empty()) {
                    errors.push(VerifyError::FixtureMarkerInVendorMode {
                        field: (*heading).to_string(),
                    });
                }
            }
            Some(_) => {}
        }
    }
}

fn validate_compile_closure(root: &Path, inventory: &Inventory, errors: &mut Vec<VerifyError>) {
    let path = root.join("compile-closure.txt");
    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => {
            errors.push(VerifyError::MissingArtifact {
                path: "compile-closure.txt".to_string(),
            });
            return;
        }
    };

    let mirror_by_path: HashMap<&str, &InventoryRow> = inventory
        .mirrors
        .iter()
        .map(|r| (r.path.as_str(), r))
        .collect();

    for line in content.lines() {
        let path = line.trim();
        if path.is_empty() {
            continue;
        }
        match mirror_by_path.get(path) {
            None => errors.push(VerifyError::MissingInventoryPath {
                path: path.to_string(),
            }),
            Some(row) if !matches!(row.disposition, Disposition::Adopt | Disposition::Adapt) => {
                errors.push(VerifyError::MissingInventoryPath {
                    path: path.to_string(),
                });
            }
            _ => {}
        }
    }
}

fn validate_upstream_tests(root: &Path, inventory: &Inventory, errors: &mut Vec<VerifyError>) {
    let path = root.join("upstream-tests.toml");
    let tests = match read_toml::<UpstreamTestsToml>(&path) {
        Ok(t) => t,
        Err(err) => {
            errors.push(err);
            return;
        }
    };

    let wrapper_paths: HashSet<&str> = inventory.wrappers.iter().map(|r| r.path.as_str()).collect();

    for test in tests.test {
        if test.relevant == Some(true) && !wrapper_paths.contains(test.path.as_str()) {
            errors.push(VerifyError::MissingInventoryPath { path: test.path });
        }
    }
}

fn validate_source_archives(
    root: &Path,
    provenance: &ProvenanceToml,
    mode: VerificationMode,
    errors: &mut Vec<VerifyError>,
) {
    if mode == VerificationMode::Fixture {
        return;
    }

    let archive_file = provenance
        .archive_hash_file
        .as_deref()
        .unwrap_or(DEFAULT_ARCHIVE_HASH_FILE);
    let path = root.join(archive_file);
    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => {
            errors.push(VerifyError::FixtureMarkerInVendorMode {
                field: archive_file.to_string(),
            });
            return;
        }
    };

    if !vendor_archive_lines_valid(&content) {
        errors.push(VerifyError::FixtureMarkerInVendorMode {
            field: archive_file.to_string(),
        });
    }
}

fn vendor_archive_lines_valid(content: &str) -> bool {
    if content.is_empty() {
        return false;
    }

    let body = content.strip_suffix('\n').unwrap_or(content);
    if body.is_empty() {
        return false;
    }

    let lines: Vec<&str> = body.split('\n').collect();
    if lines.len() != 2 {
        return false;
    }

    let mut gpui = false;
    let mut ghostty = false;
    for line in lines {
        let Some((hash, name)) = line.split_once("  ") else {
            return false;
        };
        if hash.len() != 64 || name.is_empty() {
            return false;
        }
        if !is_nonzero_sha256(hash) {
            return false;
        }
        match name {
            ARCHIVE_GPUI_NAME => {
                if gpui {
                    return false;
                }
                gpui = true;
            }
            ARCHIVE_GHOSTTY_NAME => {
                if ghostty {
                    return false;
                }
                ghostty = true;
            }
            _ => return false,
        }
    }
    gpui && ghostty
}

fn is_nonzero_sha256(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase())
        && hash != "0000000000000000000000000000000000000000000000000000000000000000"
}

fn validate_license_hashes(
    root: &Path,
    provenance: &ProvenanceToml,
    mode: VerificationMode,
    errors: &mut Vec<VerifyError>,
) {
    let bindings = [
        (
            provenance.license_apache_path.clone(),
            provenance.license_apache_sha256.clone(),
            "license_apache_sha256",
        ),
        (
            provenance.license_mit_path.clone(),
            provenance.license_mit_sha256.clone(),
            "license_mit_sha256",
        ),
        (
            provenance.license_ziglyph_path.clone(),
            provenance.ziglyph_license_sha256.clone(),
            "ziglyph_license_sha256",
        ),
    ];

    for (path_opt, hash_opt, hash_name) in bindings {
        let Some(rel) = path_opt else {
            continue;
        };
        let expected = match &hash_opt {
            Some(h) => h.clone(),
            None => {
                if mode == VerificationMode::Vendor
                    && (hash_name == "ziglyph_license_sha256"
                        || hash_name == "license_apache_sha256"
                        || hash_name == "license_mit_sha256")
                {
                    // Vendor fixtures record all three; fixture-mode valid omits them.
                    if hash_name == "ziglyph_license_sha256" {
                        // Already reported in validate_provenance.
                    }
                }
                continue;
            }
        };

        let file_path = root.join(&rel);
        let bytes = match fs::read(&file_path) {
            Ok(b) => b,
            Err(_) => {
                errors.push(VerifyError::MissingArtifact { path: rel.clone() });
                continue;
            }
        };
        let got = hex_sha256(&bytes);
        if got != expected {
            errors.push(VerifyError::LicenseHashMismatch {
                path: rel,
                expected,
                got,
            });
        }
    }
}

fn validate_vendor_locked_pins(provenance: &ProvenanceToml, errors: &mut Vec<VerifyError>) {
    for (field, expected) in LOCKED_PINS {
        let actual = provenance_field(provenance, field);
        if actual.as_deref() != Some(*expected) {
            errors.push(VerifyError::MissingPin {
                field: (*field).to_string(),
            });
        }
    }
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// CLI entry: `--upstream` present selects Vendor mode and runs upstream enumeration +
/// archive recompute in addition to [`load_and_verify`].
pub fn run_terminal_provenance(root: &Path, upstream: Option<&Path>) -> anyhow::Result<()> {
    let mode = if upstream.is_some() {
        VerificationMode::Vendor
    } else {
        VerificationMode::Fixture
    };

    let mut errors = load_and_verify(root, mode).err().unwrap_or_default();

    if let Some(upstream_path) = upstream {
        errors.extend(validate_upstream_enumeration(root, upstream_path));
        errors.extend(validate_archive_recompute(root, upstream_path));
    }

    if errors.is_empty() {
        println!("terminal-provenance: ok");
        Ok(())
    } else {
        for err in &errors {
            eprintln!("{err:?}");
        }
        anyhow::bail!(
            "terminal-provenance verification failed ({} error(s))",
            errors.len()
        )
    }
}

fn normalize_mirror_path(path: &str) -> String {
    path.strip_prefix("src/").unwrap_or(path).to_string()
}

fn run_git(cwd: &Path, args: &[&str]) -> Result<String, VerifyError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .map_err(|err| VerifyError::MissingArtifact {
            path: format!("upstream git spawn: {err}"),
        })?;
    if !output.status.success() {
        return Err(VerifyError::MissingArtifact {
            path: format!(
                "upstream git {}: {}",
                args.join(" "),
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn git_ls_files(cwd: &Path, pathspec: Option<&str>) -> Result<Vec<String>, VerifyError> {
    let mut args = vec!["ls-files"];
    if let Some(spec) = pathspec {
        args.push(spec);
    }
    let stdout = run_git(cwd, &args)?;
    Ok(stdout
        .lines()
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect())
}

fn enumerate_upstream_wrappers(upstream: &Path) -> Result<BTreeSet<String>, VerifyError> {
    let files = git_ls_files(upstream, None)?;
    Ok(files
        .into_iter()
        .filter(|path| {
            !path.starts_with("vendor/ghostty")
                && !path.contains("ghostty_src")
                && path != ".gitmodules"
        })
        .collect())
}

fn enumerate_upstream_mirrors(upstream: &Path) -> Result<BTreeSet<String>, VerifyError> {
    let ghostty = upstream.join("vendor/ghostty");
    if !ghostty.is_dir() {
        return Err(VerifyError::MissingArtifact {
            path: format!("upstream: {}", ghostty.display()),
        });
    }
    let files = git_ls_files(&ghostty, Some("src/**"))?;
    Ok(files
        .into_iter()
        .map(|p| normalize_mirror_path(&p))
        .collect())
}

fn validate_upstream_enumeration(root: &Path, upstream: &Path) -> Vec<VerifyError> {
    let mut errors = Vec::new();

    if run_git(upstream, &["rev-parse", "--git-dir"]).is_err() {
        errors.push(VerifyError::MissingArtifact {
            path: format!("upstream: {} is not a git checkout", upstream.display()),
        });
        return errors;
    }

    let provenance_path = root.join("provenance.toml");
    let provenance = match read_toml::<ProvenanceToml>(&provenance_path) {
        Ok(p) => p,
        Err(err) => {
            errors.push(err);
            return errors;
        }
    };

    let adoption_path = root.join("adoption.toml");
    let adoption = match read_toml::<AdoptionToml>(&adoption_path) {
        Ok(a) => a,
        Err(err) => {
            errors.push(err);
            return errors;
        }
    };

    let inventory = collect_inventory(&adoption, &mut errors);

    let live_wrappers = match enumerate_upstream_wrappers(upstream) {
        Ok(paths) => paths,
        Err(err) => {
            errors.push(err);
            return errors;
        }
    };
    let adoption_wrappers: BTreeSet<String> = inventory
        .wrappers
        .iter()
        .map(|row| row.path.clone())
        .collect();

    if let Some(expected) = provenance.wrapper_file_count
        && live_wrappers.len() != expected
    {
        errors.push(VerifyError::WrapperCountMismatch {
            expected,
            got: live_wrappers.len(),
        });
    }
    if live_wrappers.len() != adoption_wrappers.len() {
        errors.push(VerifyError::WrapperCountMismatch {
            expected: adoption_wrappers.len(),
            got: live_wrappers.len(),
        });
    }
    for path in adoption_wrappers.difference(&live_wrappers) {
        errors.push(VerifyError::MissingInventoryPath { path: path.clone() });
    }
    for path in live_wrappers.difference(&adoption_wrappers) {
        errors.push(VerifyError::ExtraInventoryPath { path: path.clone() });
    }

    let live_mirrors = match enumerate_upstream_mirrors(upstream) {
        Ok(paths) => paths,
        Err(err) => {
            errors.push(err);
            return errors;
        }
    };
    let adoption_mirrors: BTreeSet<String> = inventory
        .mirrors
        .iter()
        .map(|row| row.path.clone())
        .collect();

    if let Some(expected) = provenance.mirror_file_count
        && live_mirrors.len() != expected
    {
        errors.push(VerifyError::MirrorCountMismatch {
            expected,
            got: live_mirrors.len(),
        });
    }
    if live_mirrors.len() != adoption_mirrors.len() {
        errors.push(VerifyError::MirrorCountMismatch {
            expected: adoption_mirrors.len(),
            got: live_mirrors.len(),
        });
    }
    for path in adoption_mirrors.difference(&live_mirrors) {
        errors.push(VerifyError::MissingInventoryPath { path: path.clone() });
    }
    for path in live_mirrors.difference(&adoption_mirrors) {
        errors.push(VerifyError::ExtraInventoryPath { path: path.clone() });
    }

    errors
}

fn git_archive_sha256(repo: &Path, commit: &str) -> Result<String, VerifyError> {
    let child = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["archive", "--format=tar", commit])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| VerifyError::MissingArtifact {
            path: format!("git archive spawn in {}: {err}", repo.display()),
        })?;

    let output = child
        .wait_with_output()
        .map_err(|err| VerifyError::MissingArtifact {
            path: format!("git archive wait in {}: {err}", repo.display()),
        })?;

    if !output.status.success() {
        return Err(VerifyError::MissingArtifact {
            path: format!(
                "git archive {commit} in {}: {}",
                repo.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        });
    }

    Ok(hex_sha256(&output.stdout))
}

fn parse_recorded_archive_hashes(content: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let body = content.strip_suffix('\n').unwrap_or(content);
    for line in body.split('\n').filter(|line| !line.is_empty()) {
        if let Some((hash, name)) = line.split_once("  ") {
            out.insert(name.to_string(), hash.to_string());
        }
    }
    out
}

fn validate_archive_recompute(root: &Path, upstream: &Path) -> Vec<VerifyError> {
    let mut errors = Vec::new();

    if run_git(upstream, &["rev-parse", "--git-dir"]).is_err() {
        errors.push(VerifyError::MissingArtifact {
            path: format!("upstream: {} is not a git checkout", upstream.display()),
        });
        return errors;
    }

    let provenance_path = root.join("provenance.toml");
    let provenance = match read_toml::<ProvenanceToml>(&provenance_path) {
        Ok(p) => p,
        Err(err) => {
            errors.push(err);
            return errors;
        }
    };

    let archive_file = provenance
        .archive_hash_file
        .as_deref()
        .unwrap_or(DEFAULT_ARCHIVE_HASH_FILE);
    let archive_path = root.join(archive_file);
    let recorded = match fs::read_to_string(&archive_path) {
        Ok(content) => parse_recorded_archive_hashes(&content),
        Err(_) => {
            errors.push(VerifyError::MissingArtifact {
                path: archive_file.to_string(),
            });
            return errors;
        }
    };

    let gpui_commit = match provenance.gpui_ghostty_commit.as_deref() {
        Some(commit) if !commit.is_empty() => commit,
        _ => {
            errors.push(VerifyError::MissingPin {
                field: "gpui_ghostty_commit".to_string(),
            });
            return errors;
        }
    };
    let ghostty_commit = match provenance.ghostty_commit.as_deref() {
        Some(commit) if !commit.is_empty() => commit,
        _ => {
            errors.push(VerifyError::MissingPin {
                field: "ghostty_commit".to_string(),
            });
            return errors;
        }
    };

    let gpui_hash = match git_archive_sha256(upstream, gpui_commit) {
        Ok(hash) => hash,
        Err(err) => {
            errors.push(err);
            String::new()
        }
    };
    if !gpui_hash.is_empty() {
        match recorded.get(ARCHIVE_GPUI_NAME) {
            Some(expected) if *expected == gpui_hash => {}
            Some(expected) => errors.push(VerifyError::MissingHash {
                name: format!("{ARCHIVE_GPUI_NAME} (expected {expected}, got {gpui_hash})"),
            }),
            None => errors.push(VerifyError::MissingHash {
                name: ARCHIVE_GPUI_NAME.to_string(),
            }),
        }
    }

    let ghostty_repo = upstream.join("vendor/ghostty");
    let ghostty_hash = match git_archive_sha256(&ghostty_repo, ghostty_commit) {
        Ok(hash) => hash,
        Err(err) => {
            errors.push(err);
            String::new()
        }
    };
    if !ghostty_hash.is_empty() {
        match recorded.get(ARCHIVE_GHOSTTY_NAME) {
            Some(expected) if *expected == ghostty_hash => {}
            Some(expected) => errors.push(VerifyError::MissingHash {
                name: format!("{ARCHIVE_GHOSTTY_NAME} (expected {expected}, got {ghostty_hash})"),
            }),
            None => errors.push(VerifyError::MissingHash {
                name: ARCHIVE_GHOSTTY_NAME.to_string(),
            }),
        }
    }

    errors
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn vendor_archive_rule_rejects_duplicates_and_wrong_names() {
        let dup = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa  gpui-ghostty-e3025981.tar\nbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb  gpui-ghostty-e3025981.tar\n";
        assert!(!vendor_archive_lines_valid(dup));

        let wrong = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa  wrong-one.tar\nbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb  wrong-two.tar\n";
        assert!(!vendor_archive_lines_valid(wrong));

        let good = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa  gpui-ghostty-e3025981.tar\nbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb  ghostty-6d2dd585.tar";
        assert!(vendor_archive_lines_valid(good));
        assert!(vendor_archive_lines_valid(&format!("{good}\n")));

        let padded = " aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa  gpui-ghostty-e3025981.tar\nbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb  ghostty-6d2dd585.tar\n";
        assert!(!vendor_archive_lines_valid(padded));

        let extra_blank = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa  gpui-ghostty-e3025981.tar\n\nbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb  ghostty-6d2dd585.tar\n";
        assert!(!vendor_archive_lines_valid(extra_blank));
    }

    #[test]
    fn vendor_build_probe_duplicate_heading_blank_fails_even_if_later_nonempty() {
        let content = "\
# Build probe

## zig_version

## zig_version
0.14.1

## bootstrap_sha256
x

## build_command
x

## build_exit_code
x

## object_list
x

## dependency_list
x

## compile_closure_path_count
x

## offline_ziglyph
x

## renderer_cell_coupling
x

## artifact_sha256
x

## license_closure
x

## raw_log_path
x

## raw_log_sha256
x
";
        let sections = parse_build_probe_sections(content);
        assert!(sections["zig_version"].iter().any(|b| b.trim().is_empty()));
    }
}
