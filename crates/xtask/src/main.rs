use anyhow::{Context, Result, bail};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

const SPEC: &str = "vendor/omnigent-0.5.1/openapi.json";
const OUT: &str = "crates/lens-client/src/generated.rs";
const SIBLING_DEFAULT: &str = "../omnigent/openapi.json";

/// Published client path set, excluding runner-callback `/hooks/*` routes
/// (`runner→server` callbacks `ap-web` never calls — not client API; ADR-0001).
fn client_paths(doc: &serde_json::Value) -> BTreeSet<String> {
    doc.get("paths")
        .and_then(|p| p.as_object())
        .map(|m| {
            m.keys()
                .filter(|p| !p.contains("/hooks/"))
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

#[derive(Debug, Default)]
struct SetDiff {
    added: Vec<String>,   // in sibling, not vendored (upstream gained)
    removed: Vec<String>, // in vendored, not sibling (upstream dropped)
}

impl SetDiff {
    fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty()
    }
}

fn diff_sets(vendored: &BTreeSet<String>, sibling: &BTreeSet<String>) -> SetDiff {
    SetDiff {
        added: sibling.difference(vendored).cloned().collect(),
        removed: vendored.difference(sibling).cloned().collect(),
    }
}

fn sse_mapping(doc: &serde_json::Value) -> Option<&serde_json::Map<String, serde_json::Value>> {
    doc.pointer("/components/schemas/ServerStreamEvent/discriminator/mapping")
        .and_then(|m| m.as_object())
}

fn sse_event_types(doc: &serde_json::Value) -> BTreeSet<String> {
    sse_mapping(doc)
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default()
}

/// wire-type → property-name set of its mapped member schema.
fn member_shapes(doc: &serde_json::Value) -> BTreeMap<String, BTreeSet<String>> {
    let Some(mapping) = sse_mapping(doc) else {
        return BTreeMap::new();
    };
    let mut out = BTreeMap::new();
    for (wire, ref_val) in mapping {
        let Some(name) = ref_val.as_str().and_then(|r| r.rsplit('/').next()) else {
            continue;
        };
        let props = doc
            .pointer(&format!("/components/schemas/{name}/properties"))
            .and_then(|p| p.as_object())
            .map(|p| p.keys().cloned().collect())
            .unwrap_or_default();
        out.insert(wire.clone(), props);
    }
    out
}

#[derive(Debug, Default)]
struct SseDiff {
    types: SetDiff,
    changed_shapes: Vec<(String, SetDiff)>, // shared types whose property sets differ
}

impl SseDiff {
    fn is_empty(&self) -> bool {
        self.types.is_empty() && self.changed_shapes.is_empty()
    }
}

fn diff_sse(vendored: &serde_json::Value, sibling: &serde_json::Value) -> SseDiff {
    let types = diff_sets(&sse_event_types(vendored), &sse_event_types(sibling));
    let (vshapes, sshapes) = (member_shapes(vendored), member_shapes(sibling));
    let mut changed_shapes = Vec::new();
    for (wire, vprops) in &vshapes {
        if let Some(sprops) = sshapes.get(wire) {
            let shape = diff_sets(vprops, sprops);
            if !shape.is_empty() {
                changed_shapes.push((wire.clone(), shape));
            }
        }
    }
    SseDiff {
        types,
        changed_shapes,
    }
}

fn main() -> Result<()> {
    let cmd = std::env::args().nth(1).unwrap_or_default();
    match cmd.as_str() {
        "codegen" => codegen(),
        "drift" => drift(std::env::args().skip(2)),
        "gate" => gate(),
        other => bail!("unknown xtask command: {other:?} (expected: codegen | drift | gate)"),
    }
}

fn codegen() -> Result<()> {
    let root = workspace_root()?;
    let spec_path = root.join(SPEC);
    let raw = std::fs::read_to_string(&spec_path)
        .with_context(|| format!("read {}", spec_path.display()))?;
    let doc: serde_json::Value = serde_json::from_str(&raw)?;

    let schemas = doc
        .get("components")
        .and_then(|c| c.get("schemas"))
        .and_then(|s| s.as_object())
        .context("openapi.json has no components.schemas")?;

    let mut settings = typify::TypeSpaceSettings::default();
    settings.with_derive("PartialEq".into());
    let mut type_space = typify::TypeSpace::new(&settings);

    let mut parsed: Vec<(String, schemars::schema::Schema)> = Vec::new();
    for (name, schema) in schemas {
        let schema: schemars::schema::Schema = serde_json::from_value(schema.clone())
            .with_context(|| format!("schema {name} is not valid JSON Schema"))?;
        parsed.push((name.clone(), schema));
    }

    if let Err(e) = type_space.add_ref_types(parsed.iter().map(|(n, s)| (n.as_str(), s.clone()))) {
        let mut failures: Vec<(String, String)> = Vec::new();
        for (name, schema) in &parsed {
            let mut probe = typify::TypeSpace::new(&settings);
            if let Err(err) = probe.add_ref_types(std::iter::once((name.as_str(), schema.clone())))
            {
                failures.push((name.clone(), err.to_string()));
            }
        }
        let skipped_path = root.join("vendor/omnigent-0.5.1/SKIPPED.md");
        let body: String = failures
            .iter()
            .map(|(name, reason)| format!("- {name}: {reason}"))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        std::fs::write(&skipped_path, &body)
            .with_context(|| format!("write {}", skipped_path.display()))?;
        let failed_names: Vec<&str> = failures.iter().map(|(n, _)| n.as_str()).collect();
        bail!(
            "typify batch add_ref_types failed: {e}; individually-failing schemas: {}",
            failed_names.join(", ")
        );
    }

    let tokens = type_space.to_stream();
    let file: syn::File = syn::parse2(tokens).context("parse generated tokens")?;
    let pretty = prettyplease::unparse(&file);

    let header = "// @generated by `cargo run -p xtask -- codegen` from \
vendor/omnigent-0.5.1/openapi.json — DO NOT EDIT BY HAND.\n\
#![allow(clippy::all)]\n\n";
    let out_path = root.join(OUT);
    std::fs::write(&out_path, format!("{header}{pretty}"))
        .with_context(|| format!("write {OUT}"))?;

    // prettyplease gives us readable output, but `cargo fmt` (rustfmt) formats
    // differently — run rustfmt as the final pass so the committed file is
    // rustfmt-canonical and `cargo fmt --check` stays a no-op on it.
    let status = std::process::Command::new("rustfmt")
        .args(["--edition", "2024"])
        .arg(&out_path)
        .status()
        .context("run rustfmt on generated.rs (is the rustfmt component installed?)")?;
    if !status.success() {
        bail!("rustfmt failed on {OUT} (exit {:?})", status.code());
    }

    println!("wrote {OUT} ({} schemas)", schemas.len());
    Ok(())
}

fn workspace_root() -> Result<PathBuf> {
    // xtask is invoked from the workspace root via `cargo run -p xtask`.
    Ok(std::env::current_dir()?)
}

/// Diff the vendored contract against the sibling omnigent pin — the ADR-0001
/// "passive alarm." Path enumeration now; SSE taxonomy/shape in Task 2.
fn drift(mut args: impl Iterator<Item = String>) -> Result<()> {
    let mut against = PathBuf::from(SIBLING_DEFAULT);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--against" => {
                against = PathBuf::from(args.next().context("--against needs a path argument")?);
            }
            other => bail!("unknown drift arg: {other:?} (expected: --against <path>)"),
        }
    }

    let root = workspace_root()?;
    let vendored: serde_json::Value = read_json(&root.join(SPEC))?;
    let sibling: serde_json::Value = read_json(&against)?;

    let paths = diff_sets(&client_paths(&vendored), &client_paths(&sibling));

    let mut drifted = false;
    if !paths.is_empty() {
        drifted = true;
        eprintln!("PATH DRIFT (vendored {SPEC} vs {}):", against.display());
        for p in &paths.added {
            eprintln!("  + {p}  (upstream gained)");
        }
        for p in &paths.removed {
            eprintln!("  - {p}  (upstream dropped)");
        }
    }

    let sse = diff_sse(&vendored, &sibling);
    if !sse.is_empty() {
        drifted = true;
        eprintln!("SSE TAXONOMY DRIFT:");
        for t in &sse.types.added {
            eprintln!("  + event type {t}  (upstream gained)");
        }
        for t in &sse.types.removed {
            eprintln!("  - event type {t}  (upstream dropped)");
        }
        for (ty, shape) in &sse.changed_shapes {
            for p in &shape.added {
                eprintln!("  ~ {ty}: + field {p}");
            }
            for p in &shape.removed {
                eprintln!("  ~ {ty}: - field {p}");
            }
        }
    }

    if drifted {
        bail!("contract drift detected — re-vendor + re-run codegen, or update the pin");
    }
    println!(
        "no drift: {} client paths match {}",
        client_paths(&vendored).len(),
        against.display()
    );
    Ok(())
}

/// Run one `cargo <args>` and fail loudly on a non-zero exit.
fn run(args: &[&str]) -> Result<()> {
    eprintln!("$ cargo {}", args.join(" "));
    let status = std::process::Command::new("cargo")
        .args(args)
        .status()
        .context("spawn cargo")?;
    if !status.success() {
        bail!(
            "`cargo {}` failed (exit {:?})",
            args.join(" "),
            status.code()
        );
    }
    Ok(())
}

/// The "CI = us running everything" wall (memory benchmark-validity-audit): fmt →
/// clippy (feature matrix) → test → bench compile-only → drift. Scoped to the
/// production crates; `spikes/*` opt out of the lint bar by design. Bench NUMBERS
/// are not pass/fail (no CI history to regress against yet) — `--no-run` only
/// guards the harness against bit-rot.
fn gate() -> Result<()> {
    run(&[
        "fmt",
        "-p",
        "lens-core",
        "-p",
        "lens-client",
        "-p",
        "lens-capture",
        "-p",
        "xtask",
        "--",
        "--check",
    ])?;

    // Default features across every production crate.
    run(&[
        "clippy",
        "-p",
        "lens-core",
        "-p",
        "lens-client",
        "-p",
        "lens-capture",
        "-p",
        "xtask",
        "--all-targets",
        "--",
        "-D",
        "warnings",
    ])?;
    // lens-client's feature combos compile different target sets; the live-tests
    // clippy pass catches lints the bench build misses (memory lens-client-benchmarks).
    run(&[
        "clippy",
        "-p",
        "lens-client",
        "--all-targets",
        "--features",
        "bench",
        "--",
        "-D",
        "warnings",
    ])?;
    run(&[
        "clippy",
        "-p",
        "lens-client",
        "--all-targets",
        "--features",
        "live-tests",
        "--",
        "-D",
        "warnings",
    ])?;

    // Tests: default features only (live-tests need a running server).
    run(&[
        "test",
        "-p",
        "lens-core",
        "-p",
        "lens-client",
        "-p",
        "lens-capture",
        "-p",
        "xtask",
    ])?;

    // Bench harness must still COMPILE (bit-rot guard); criterion sampling is
    // deliberately NOT run here (minutes of wall-clock).
    run(&[
        "bench",
        "-p",
        "lens-client",
        "--features",
        "bench",
        "--no-run",
    ])?;
    run(&["bench", "-p", "lens-core", "--no-run"])?;

    // Contract drift vs the sibling omnigent pin. A MISSING sibling is a setup
    // bug, not a pass — drift() bails on an unreadable spec and we propagate that.
    drift(std::iter::empty())?;

    println!("gate: all checks passed");
    Ok(())
}

fn read_json(path: &std::path::Path) -> Result<serde_json::Value> {
    let raw = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    serde_json::from_str(&raw).with_context(|| format!("parse {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn doc_with_paths(paths: &[&str]) -> serde_json::Value {
        let mut map = serde_json::Map::new();
        for p in paths {
            map.insert((*p).to_string(), json!({}));
        }
        json!({ "paths": map })
    }

    #[test]
    fn path_diff_ignores_hooks_and_reports_added_removed() {
        let vendored = doc_with_paths(&[
            "/v1/sessions",
            "/v1/sessions/{session_id}/hooks/permission-request", // runner callback — ignored
            "/v1/policies",
        ]);
        let sibling = doc_with_paths(&[
            "/v1/sessions",
            "/v1/sessions/{session_id}/hooks/permission-request",
            "/v1/agents", // upstream gained
                          // /v1/policies upstream dropped
        ]);

        let vp = client_paths(&vendored);
        let sp = client_paths(&sibling);
        assert!(
            !vp.iter().any(|p| p.contains("/hooks/")),
            "hooks must be filtered"
        );

        let diff = diff_sets(&vp, &sp);
        assert_eq!(diff.added, vec!["/v1/agents".to_string()]);
        assert_eq!(diff.removed, vec!["/v1/policies".to_string()]);
        assert!(!diff.is_empty());

        // Identical specs → no drift.
        let same = diff_sets(&vp, &vp);
        assert!(same.is_empty());
    }

    fn doc_with_events(members: &[(&str, &str, &[&str])]) -> serde_json::Value {
        // members: (wire_type, schema_name, property_names)
        let mut mapping = serde_json::Map::new();
        let mut schemas = serde_json::Map::new();
        for (wire, name, props) in members {
            mapping.insert(
                (*wire).to_string(),
                json!(format!("#/components/schemas/{name}")),
            );
            let mut props_obj = serde_json::Map::new();
            for p in *props {
                props_obj.insert((*p).to_string(), json!({}));
            }
            schemas.insert((*name).to_string(), json!({ "properties": props_obj }));
        }
        schemas.insert(
            "ServerStreamEvent".to_string(),
            json!({ "discriminator": { "propertyName": "type", "mapping": mapping } }),
        );
        json!({ "components": { "schemas": schemas } })
    }

    #[test]
    fn sse_diff_reports_type_and_shape_changes() {
        let vendored = doc_with_events(&[
            ("response.completed", "CompletedEvent", &["type", "seq"]),
            ("session.status", "SessionStatusEvent", &["type", "status"]),
        ]);
        let sibling = doc_with_events(&[
            (
                "response.completed",
                "CompletedEvent",
                &["type", "seq", "usage"],
            ), // field added
            ("turn.started", "TurnStartedEvent", &["type"]), // type added
                                                             // session.status dropped
        ]);

        let diff = diff_sse(&vendored, &sibling);
        assert_eq!(diff.types.added, vec!["turn.started".to_string()]);
        assert_eq!(diff.types.removed, vec!["session.status".to_string()]);
        assert_eq!(diff.changed_shapes.len(), 1);
        let (ty, shape) = &diff.changed_shapes[0];
        assert_eq!(ty, "response.completed");
        assert_eq!(shape.added, vec!["usage".to_string()]);
        assert!(shape.removed.is_empty());

        assert!(diff_sse(&vendored, &vendored).is_empty());
    }
}
