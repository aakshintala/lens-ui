# Plan 3c — Contract-Drift CI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stand up the outstanding **B6** contract-drift "passive alarm" — an `xtask drift` command + an always-on static taxonomy-completeness test + gated live taxonomy-diff/endpoint-reachability checks — so tracking the unreleased `0.3.0.dev0` pin is safe and silent re-drift becomes loud.

**Architecture:** Three layers, split by what they need. (1) **`xtask drift`** is a deterministic two-file diff of vendored `openapi.json` vs the sibling omnigent pin — *path enumeration + SSE discriminator/shape* per ADR-0001, ignoring runner-callback `/hooks/*` routes. (2) An **always-on `cargo test`** in `lens-client` asserts the pinned openapi's `ServerStreamEvent` discriminator mapping is fully *accounted for* by the crate (modeled or knowingly-deferred) — the offline half of §9.4, no server. (3) **Gated `--features live-tests`** checks close the loop against a running server: a wire-vs-contract taxonomy assertion and an endpoint-reachability sweep. Local-only; no GitHub Actions (design D3).

**Tech Stack:** Rust 2024, `serde_json` (already an `xtask` + `lens-client` dep), the existing `live-tests` feature flag and live-test harness pattern, the `installing-omnigent-from-source` skill for server bring-up.

## Global Constraints

- **Pin:** omnigent `0.3.0.dev0`, Source HEAD `36b2a11c`. Vendored ground truth: `vendor/omnigent-0.3.0.dev0/openapi.json`. The sibling checkout `../omnigent` is currently at the same commit `36b2a11c` and its `openapi.json` is **byte-identical** to the vendored copy → the clean zero-drift baseline. Cite the pinned contract, never memory (AGENTS.md ground-truth discipline).
- **No `serde_json::Value` to consumers** — `Value` is internal-only. Public surfaces stay typed.
- **`generated.rs` is untouched** — codegen artifact; not edited by hand in this plan.
- **`clippy --all-targets` clean + `rustfmt` clean** on every commit. `unsafe` needs a `// SAFETY:` note (none expected here).
- **UI never panics** — but `xtask` is local tooling, not shipped; it may `bail!`/exit-non-zero (anyhow) on drift. The `lens-client` crate code stays panic-free in non-test paths.
- **Local-only CI** — no `.github/workflows`. `xtask drift` is a hand/pre-commit-run command; live checks gate behind `--features live-tests` + `LENS_OMNIGENT_URL`.
- **Cross-family review at the seams** is MANDATORY (composer authored → review by `gpt-5.5` or `gemini-3.5`). Mind Cursor-credit spend (one consolidated end-of-plan review preferred).
- **Comments explain *why* / the non-obvious** — never narrate code.

---

## File Structure

| File | Responsibility | Action |
| --- | --- | --- |
| `crates/xtask/src/main.rs` | Add `drift` subcommand + pure diff fns (`client_paths`, `sse_event_types`, `member_shapes`, `diff_sets`, `diff_sse`) + `#[cfg(test)] mod tests`. | Modify |
| `crates/lens-client/src/stream/event.rs` | Add `pub const ACCOUNTED_EVENT_TYPES: &[&str]` (the 47 contract discriminators, each modeled-or-deferred). | Modify |
| `crates/lens-client/src/stream/mod.rs` | Re-export `ACCOUNTED_EVENT_TYPES`. | Modify |
| `crates/lens-client/tests/taxonomy_drift.rs` | Always-on test: vendored openapi `ServerStreamEvent` mapping == `ACCOUNTED_EVENT_TYPES`. | Create |
| `crates/lens-client/tests/live_taxonomy.rs` | Gated: drive a turn, assert every wire `Unknown.event_type` ∈ `ACCOUNTED_EVENT_TYPES` (wire ⊆ contract). | Create |
| `crates/lens-client/tests/live_reachability.rs` | Gated: ping every consumed read endpoint once; assert reachable (typed Ok / typed not-error), no transport failure. | Create |
| `crates/xtask/tests/fixtures/drifted-openapi.json` | Tiny synthetic mutated openapi proving the alarm fires (red path). | Create |
| `docs/design/typed-client-implementation.md` | §5 — mark `xtask drift` built; record the static-taxonomy-test addition. | Modify |
| `vendor/omnigent-0.3.0.dev0/README.md` | Document the `xtask drift` invocation as the drift-check the README already calls for. | Modify |
| `docs/STATUS.md` + `docs/handoffs/2026-06-26-lens-client-plan3c-execution.md` | End-of-session status + handoff. | Modify / Create |

---

## Task 1: `xtask drift` — path-set diff (the passive alarm scaffold)

**Files:**
- Modify: `crates/xtask/src/main.rs`
- Test: `crates/xtask/src/main.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: nothing (entry is `main`'s arg dispatch).
- Produces: `fn client_paths(doc: &serde_json::Value) -> std::collections::BTreeSet<String>`; `struct SetDiff { added: Vec<String>, removed: Vec<String> }` with `fn is_empty(&self) -> bool`; `fn diff_sets(vendored: &BTreeSet<String>, sibling: &BTreeSet<String>) -> SetDiff`; `fn drift(args: impl Iterator<Item = String>) -> anyhow::Result<()>`.

**Notes for the implementer:**
- `xtask` is invoked from the workspace root via `cargo run -p xtask`. The existing `codegen` reads paths relative to `std::env::current_dir()` (see `workspace_root()`). Reuse that for the vendored spec path `vendor/omnigent-0.3.0.dev0/openapi.json`.
- Per ADR-0001, **exclude runner-callback routes**: any path containing `/hooks/` (these are `runner→server` callbacks `ap-web` never calls — `/v1/sessions/{session_id}/hooks/{permission,*-elicitation,cursor-permission,...}-request`). Compare the full published path set otherwise — a *new* upstream client route is exactly the signal we want, even before Lens consumes it.
- `added` = in sibling, not vendored (upstream gained). `removed` = in vendored, not sibling (upstream dropped). Both are drift.
- Default `--against` target is the sibling pin `../omnigent/openapi.json`.

- [ ] **Step 1: Write the failing test**

Add to `crates/xtask/src/main.rs`:

```rust
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
        assert!(!vp.iter().any(|p| p.contains("/hooks/")), "hooks must be filtered");

        let diff = diff_sets(&vp, &sp);
        assert_eq!(diff.added, vec!["/v1/agents".to_string()]);
        assert_eq!(diff.removed, vec!["/v1/policies".to_string()]);
        assert!(!diff.is_empty());

        // Identical specs → no drift.
        let same = diff_sets(&vp, &vp);
        assert!(same.is_empty());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p xtask path_diff_ignores_hooks_and_reports_added_removed`
Expected: FAIL — `client_paths` / `diff_sets` / `SetDiff` not defined.

- [ ] **Step 3: Write minimal implementation**

Add to `crates/xtask/src/main.rs` (near the top-level fns; keep `use` additions tidy):

```rust
use std::collections::BTreeSet;

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
```

Wire the subcommand and the `drift` fn:

```rust
// in main()'s match:
        "drift" => drift(std::env::args().skip(2)),
// update the catch-all message:
        other => bail!("unknown xtask command: {other:?} (expected: codegen | drift)"),
```

```rust
const SIBLING_DEFAULT: &str = "../omnigent/openapi.json";

/// Diff the vendored contract against the sibling omnigent pin — the ADR-0001
/// "passive alarm." Path enumeration now; SSE taxonomy/shape in Task 2.
fn drift(mut args: impl Iterator<Item = String>) -> Result<()> {
    let mut against = PathBuf::from(SIBLING_DEFAULT);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--against" => {
                against = PathBuf::from(
                    args.next().context("--against needs a path argument")?,
                );
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

    if drifted {
        bail!("contract drift detected — re-vendor + re-run codegen, or update the pin");
    }
    println!("no drift: {} client paths match {}", client_paths(&vendored).len(), against.display());
    Ok(())
}

fn read_json(path: &std::path::Path) -> Result<serde_json::Value> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("read {}", path.display()))?;
    serde_json::from_str(&raw).with_context(|| format!("parse {}", path.display()))
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p xtask path_diff_ignores_hooks_and_reports_added_removed`
Expected: PASS.

- [ ] **Step 5: Run it green against the real sibling baseline**

Run: `cargo run -p xtask -- drift`
Expected: `no drift: 55 client paths match ../omnigent/openapi.json` (59 total − 4 `/hooks/*` routes), exit 0.

- [ ] **Step 6: clippy + fmt + commit**

```bash
cargo clippy -p xtask --all-targets && cargo fmt -p xtask -- --check
git add crates/xtask/src/main.rs
git commit -m "feat(xtask): drift subcommand — path-set diff vs sibling (B6 passive alarm)"
```

---

## Task 2: `xtask drift` — SSE taxonomy + shape diff

**Files:**
- Modify: `crates/xtask/src/main.rs`
- Test: `crates/xtask/src/main.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `SetDiff` / `diff_sets` (Task 1).
- Produces: `fn sse_event_types(doc: &serde_json::Value) -> BTreeSet<String>`; `fn member_shapes(doc: &serde_json::Value) -> std::collections::BTreeMap<String, BTreeSet<String>>` (event-type → its schema's property names); `struct SseDiff { types: SetDiff, changed_shapes: Vec<(String, SetDiff)> }` with `fn is_empty(&self)`; `fn diff_sse(vendored, sibling) -> SseDiff`.

**Notes for the implementer:**
- The `/v1/sessions/{session_id}/stream` response is `text/event-stream` with `itemSchema: $ref ServerStreamEvent`. `ServerStreamEvent` is a `oneOf` of 47 members with `discriminator.propertyName: "type"` and a `mapping` of the 47 wire type strings → member schema `$ref`s. The mapping keys ARE the SSE taxonomy.
- **Shape diff**: for each shared event type, resolve its mapped `$ref` (e.g. `#/components/schemas/SessionStatusEvent`) and collect that schema's `properties` key set. Compare the property-name sets — an added/removed field surfaces as a `SetDiff`. Keep to property *names* (not deep type compare) to bound scope; a field rename/add/drop is the drift we need to catch.
- A type present in only one spec is reported via `types` (added/removed), not `changed_shapes`.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `crates/xtask/src/main.rs`:

```rust
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
            ("response.completed", "CompletedEvent", &["type", "seq", "usage"]), // field added
            ("turn.started", "TurnStartedEvent", &["type"]),                     // type added
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
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p xtask sse_diff_reports_type_and_shape_changes`
Expected: FAIL — `sse_event_types` / `member_shapes` / `diff_sse` / `SseDiff` not defined.

- [ ] **Step 3: Write minimal implementation**

Add to `crates/xtask/src/main.rs`:

```rust
use std::collections::BTreeMap;

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
    SseDiff { types, changed_shapes }
}
```

Extend `drift()` to run the SSE diff alongside paths (insert before the `if drifted` block):

```rust
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
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p xtask sse_diff_reports_type_and_shape_changes`
Expected: PASS.

- [ ] **Step 5: Run drift green against the real sibling**

Run: `cargo run -p xtask -- drift`
Expected: `no drift: 55 client paths match …` — both path and SSE diffs empty, exit 0.

- [ ] **Step 6: Prove the alarm fires (red path) with a synthetic fixture**

Create `crates/xtask/tests/fixtures/drifted-openapi.json` — a minimal openapi that drops a path, adds an SSE type, and adds a field:

```json
{
  "paths": {
    "/v1/sessions": {},
    "/v1/agents": {}
  },
  "components": {
    "schemas": {
      "CompletedEvent": { "properties": { "type": {}, "seq": {}, "usage": {} } },
      "TurnStartedEvent": { "properties": { "type": {} } },
      "ServerStreamEvent": {
        "discriminator": {
          "propertyName": "type",
          "mapping": {
            "response.completed": "#/components/schemas/CompletedEvent",
            "turn.started": "#/components/schemas/TurnStartedEvent"
          }
        }
      }
    }
  }
}
```

Run: `cargo run -p xtask -- drift --against crates/xtask/tests/fixtures/drifted-openapi.json`
Expected: non-zero exit; stderr lists PATH DRIFT + SSE TAXONOMY DRIFT (the alarm fires).

- [ ] **Step 7: clippy + fmt + commit**

```bash
cargo clippy -p xtask --all-targets && cargo fmt -p xtask -- --check
git add crates/xtask/src/main.rs crates/xtask/tests/fixtures/drifted-openapi.json
git commit -m "feat(xtask): drift — SSE discriminator + member-shape diff (B6)"
```

---

## Task 3: Static taxonomy-completeness test (offline §9.4)

**Files:**
- Modify: `crates/lens-client/src/stream/event.rs`
- Modify: `crates/lens-client/src/stream/mod.rs`
- Test: `crates/lens-client/tests/taxonomy_drift.rs` (create)

**Interfaces:**
- Consumes: the vendored `openapi.json` (read at test time, path relative to `CARGO_MANIFEST_DIR`).
- Produces: `pub const lens_client::stream::ACCOUNTED_EVENT_TYPES: &[&str]` — every contract `ServerStreamEvent` discriminator, each accounted for (modeled by `parse_event`, or knowingly routed to `Unknown`).

**Notes for the implementer:**
- `parse_event` (`event.rs:470`) is the SSOT for which types are *modeled*. This const enumerates the full contract set so the test can assert nothing new slipped in. Group the 14 currently-`Unknown` (deferred — absent from the golden captures, not yet modeled) under a comment so review can see modeled vs deferred at a glance.
- The 14 deferred (route to `Unknown` today): `response.client_task.cancel`, `response.created`, `response.heartbeat`, `response.output_file.done`, `response.queued`, `response.retry`, `session.agent_changed`, `session.collaboration_mode`, `session.created`, `session.resource.deleted`, `turn.cancelled`, `turn.completed`, `turn.failed`, `turn.started`. (Computed from `discriminator.mapping` minus `parse_event`'s modeled arms.)
- Test reads `../../vendor/omnigent-0.3.0.dev0/openapi.json` relative to `env!("CARGO_MANIFEST_DIR")` (`crates/lens-client`). Deterministic, no server, always-on `cargo test`.

- [ ] **Step 1: Write the failing test**

Create `crates/lens-client/tests/taxonomy_drift.rs`:

```rust
//! Offline contract-drift alarm (Plan 3c / §9.4 static half): the pinned
//! openapi's `ServerStreamEvent` discriminator mapping must be exactly the set
//! the crate accounts for (modeled or knowingly-deferred-to-Unknown). A new
//! upstream event type fails here with no server. Pairs with `xtask drift`
//! (vendored-vs-sibling) and the gated `live_taxonomy` (wire-vs-contract).
use std::collections::BTreeSet;

#[test]
fn accounted_event_types_match_pinned_contract() {
    let spec = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../vendor/omnigent-0.3.0.dev0/openapi.json"
    );
    let raw = std::fs::read_to_string(spec).expect("read vendored openapi.json");
    let doc: serde_json::Value = serde_json::from_str(&raw).expect("parse openapi.json");

    let contract: BTreeSet<String> = doc
        .pointer("/components/schemas/ServerStreamEvent/discriminator/mapping")
        .and_then(|m| m.as_object())
        .expect("ServerStreamEvent discriminator mapping")
        .keys()
        .cloned()
        .collect();

    let accounted: BTreeSet<String> = lens_client::stream::ACCOUNTED_EVENT_TYPES
        .iter()
        .map(|s| s.to_string())
        .collect();

    let unaccounted: Vec<&String> = contract.difference(&accounted).collect();
    let phantom: Vec<&String> = accounted.difference(&contract).collect();
    assert!(
        unaccounted.is_empty(),
        "contract event types not accounted for (model them or add to ACCOUNTED_EVENT_TYPES): {unaccounted:?}"
    );
    assert!(
        phantom.is_empty(),
        "ACCOUNTED_EVENT_TYPES lists types the contract no longer declares: {phantom:?}"
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p lens-client --test taxonomy_drift`
Expected: FAIL — `lens_client::stream::ACCOUNTED_EVENT_TYPES` not defined.

- [ ] **Step 3: Write minimal implementation**

Add to `crates/lens-client/src/stream/event.rs` (near `parse_event`):

```rust
/// Every `ServerStreamEvent` wire discriminator the pinned `0.3.0.dev0` contract
/// declares, each accounted for. `parse_event` (above) is the SSOT for which are
/// *modeled*; the rest are knowingly routed to `Unknown` (deferred — absent from
/// the golden captures). The `taxonomy_drift` test asserts this equals the
/// vendored openapi `ServerStreamEvent` discriminator mapping, so a new upstream
/// event type fails offline. Keep in sync with `parse_event` when modeling a
/// deferred type (move its comment, not the entry).
pub const ACCOUNTED_EVENT_TYPES: &[&str] = &[
    // --- modeled by parse_event ---
    "response.cancelled",
    "response.compaction.completed",
    "response.compaction.failed",
    "response.compaction.in_progress",
    "response.completed",
    "response.elicitation_request",
    "response.elicitation_resolved",
    "response.error",
    "response.failed",
    "response.in_progress",
    "response.incomplete",
    "response.output_item.done",
    "response.output_text.delta",
    "response.reasoning.started",
    "response.reasoning_summary_text.delta",
    "response.reasoning_text.delta",
    "session.changed_files.invalidated",
    "session.child_session.updated",
    "session.heartbeat",
    "session.input.consumed",
    "session.interrupted",
    "session.model",
    "session.model_options",
    "session.presence",
    "session.reasoning_effort",
    "session.resource.created",
    "session.sandbox_status",
    "session.skills",
    "session.status",
    "session.terminal.activity",
    "session.terminal_pending",
    "session.todos",
    "session.usage",
    // --- deferred: routed to Unknown today (not in golden captures) ---
    "response.client_task.cancel",
    "response.created",
    "response.heartbeat",
    "response.output_file.done",
    "response.queued",
    "response.retry",
    "session.agent_changed",
    "session.collaboration_mode",
    "session.created",
    "session.resource.deleted",
    "turn.cancelled",
    "turn.completed",
    "turn.failed",
    "turn.started",
];
```

Re-export it in `crates/lens-client/src/stream/mod.rs`:

```rust
pub use event::{
    ACCOUNTED_EVENT_TYPES, DisconnectReason, Item, MessageContentBlock, PresenceViewer,
    ResponseEvent, ServerStreamEvent, SessionEvent, SessionStatusValue,
};
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p lens-client --test taxonomy_drift`
Expected: PASS (33 modeled + 14 deferred == 47 contract).

- [ ] **Step 5: Full suite + clippy + fmt + commit**

```bash
cargo test -p lens-client
cargo clippy -p lens-client --all-targets && cargo fmt -p lens-client -- --check
git add crates/lens-client/src/stream/event.rs crates/lens-client/src/stream/mod.rs crates/lens-client/tests/taxonomy_drift.rs
git commit -m "feat(lens-client): offline taxonomy-completeness test vs pinned contract (B6/§9.4)"
```

---

## Task 4: Gated live taxonomy-diff (wire ⊆ contract)

**Files:**
- Create: `crates/lens-client/tests/live_taxonomy.rs`

**Interfaces:**
- Consumes: `lens_client::stream::ACCOUNTED_EVENT_TYPES` (Task 3); the existing `Client` / `Sessions::stream` / `send_event` surface (mirrors `tests/live_stream.rs`).
- Produces: nothing (test only).

**Notes for the implementer:**
- This is the runtime half of §9.4: a wire event whose `type` is **not** in `ACCOUNTED_EVENT_TYPES` is drift the openapi doesn't even declare (the capture-spike found 3 such once — now folded into the schema). `Unknown.event_type ∈ ACCOUNTED` is fine (a known-but-deferred type legitimately appearing on the wire, e.g. `turn.started`).
- Gate `#![cfg(feature = "live-tests")]`; require `LENS_OMNIGENT_URL` + an idle claude-sdk `LENS_OMNIGENT_SESSION_ID`, exactly like `live_stream.rs`.

- [ ] **Step 1: Write the test**

Create `crates/lens-client/tests/live_taxonomy.rs`:

```rust
//! Live taxonomy-diff (Plan 3c / §9.4 runtime half): drive one turn and assert
//! every observed wire `Unknown` event type is at least *declared* by the pinned
//! contract (∈ ACCOUNTED_EVENT_TYPES). A wire type absent from the contract is
//! drift the openapi doesn't express — fail loud.
//! Run: LENS_OMNIGENT_URL=… LENS_OMNIGENT_SESSION_ID=… \
//!   cargo test -p lens-client --features live-tests --test live_taxonomy -- --nocapture
#![cfg(feature = "live-tests")]

use lens_client::ids::{ConnectionId, SessionId};
use lens_client::stream::{ResponseEvent, ServerStreamEvent, ACCOUNTED_EVENT_TYPES};
use lens_client::{Auth, Connection, SessionEventInput};

#[test]
fn live_wire_types_are_declared_by_contract() {
    let base = std::env::var("LENS_OMNIGENT_URL")
        .expect("set LENS_OMNIGENT_URL")
        .parse()
        .unwrap();
    let sid = SessionId::new(
        std::env::var("LENS_OMNIGENT_SESSION_ID").expect("set LENS_OMNIGENT_SESSION_ID"),
    );
    let client =
        lens_client::Client::new(Connection::new(ConnectionId::new("live"), base, Auth::None))
            .unwrap();

    let stream = client.sessions().stream(&sid).expect("open stream");
    client
        .sessions()
        .send_event(
            &sid,
            &SessionEventInput::Message {
                content: vec![
                    serde_json::json!({"type":"input_text","text":"Say hello in one word."}),
                ],
                model_override: None,
                tools: None,
            },
        )
        .expect("post message");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    let mut undeclared: Vec<String> = Vec::new();
    let mut saw_completed = false;
    while std::time::Instant::now() < deadline {
        match stream.try_recv() {
            Some(ServerStreamEvent::Response(ResponseEvent::Completed)) => {
                saw_completed = true;
                break;
            }
            Some(ServerStreamEvent::Unknown { event_type }) => {
                if !ACCOUNTED_EVENT_TYPES.contains(&event_type.as_str()) {
                    undeclared.push(event_type);
                }
            }
            Some(_) => {}
            None => std::thread::sleep(std::time::Duration::from_millis(50)),
        }
    }
    assert!(saw_completed, "never observed response.completed");
    assert!(
        undeclared.is_empty(),
        "wire event types absent from the pinned contract (re-vendor + model): {undeclared:?}"
    );
}
```

- [ ] **Step 2: Verify it compiles (gated, not run yet)**

Run: `cargo test -p lens-client --features live-tests --test live_taxonomy --no-run`
Expected: compiles clean. (Execution is Task 6, against a live server.)

- [ ] **Step 3: clippy (gated) + commit**

```bash
cargo clippy -p lens-client --all-targets --features live-tests
cargo fmt -p lens-client -- --check
git add crates/lens-client/tests/live_taxonomy.rs
git commit -m "feat(lens-client): gated live taxonomy-diff — wire types ⊆ contract (B6/§9.4)"
```

---

## Task 5: Gated endpoint-reachability sweep

**Files:**
- Create: `crates/lens-client/tests/live_reachability.rs`

**Interfaces:**
- Consumes: the typed read surface — `Client::new` (handshake covers `/health`, `/api/version`, `/v1/info`), `info().me()`, `agents().list()`, `hosts().list()`, `policies().list()`, `sessions().list(...)`, `sessions().get(...)`, `sessions().items(...)`, `sessions().child_sessions(...)`, `sessions().resources(...)`. Verify each method's exact name/signature against `crates/lens-client/src/{info,registries,sessions}.rs` before writing — use the real signatures, not these labels.
- Produces: nothing (test only).

**Notes for the implementer:**
- Every §3 endpoint must be pinged ≥ once (typed-client §9 "endpoint reachability"). Use **typed methods only** (no raw `Value`, no `/hooks/*`). "Reachable" = the call returns `Ok` (or a *typed, modeled* domain error like not-found) — never a transport/connection error and never a deserialize failure (which would itself be drift).
- Session-scoped endpoints need `LENS_OMNIGENT_SESSION_ID`. Read-only calls only — do **not** create/patch/delete here.
- Before writing, open `info.rs`, `registries.rs`, `sessions.rs` and copy the exact accessor + method names + argument types (e.g. `SessionFilter`, `GetOpts`, pagination args). Construct default/empty filter+opts values as those modules expect. If a method needs an argument type you can't default cleanly, ping the smallest valid call.

- [ ] **Step 1: Confirm the exact typed read signatures**

Run: `grep -n "pub fn " crates/lens-client/src/info.rs crates/lens-client/src/registries.rs crates/lens-client/src/sessions.rs`
Use these to write the calls below with real names/types.

- [ ] **Step 2: Write the test**

Create `crates/lens-client/tests/live_reachability.rs` (adjust method names/arg construction to the signatures from Step 1):

```rust
//! Live endpoint-reachability sweep (Plan 3c / typed-client §9): every consumed
//! read endpoint is pinged once through its typed method. Reachable = Ok or a
//! typed domain error; a transport error or a deserialize failure is a contract
//! problem and fails the test.
//! Run: LENS_OMNIGENT_URL=… LENS_OMNIGENT_SESSION_ID=… \
//!   cargo test -p lens-client --features live-tests --test live_reachability -- --nocapture
#![cfg(feature = "live-tests")]

use lens_client::ids::{ConnectionId, SessionId};
use lens_client::{Auth, Connection};

/// A typed Ok or a modeled domain error both prove reachability; only transport
/// / decode failures are fatal here.
fn reachable<T>(label: &str, r: lens_client::Result<T>) {
    match r {
        Ok(_) => eprintln!("  ok   {label}"),
        Err(e) if e.is_transport() => panic!("UNREACHABLE {label}: transport error: {e}"),
        Err(e) if e.is_decode() => panic!("CONTRACT DRIFT {label}: decode error: {e}"),
        Err(e) => eprintln!("  ok   {label} (typed domain error: {e})"),
    }
}

#[test]
fn consumed_read_endpoints_are_reachable() {
    let base = std::env::var("LENS_OMNIGENT_URL")
        .expect("set LENS_OMNIGENT_URL")
        .parse()
        .unwrap();
    let sid = SessionId::new(
        std::env::var("LENS_OMNIGENT_SESSION_ID").expect("set LENS_OMNIGENT_SESSION_ID"),
    );
    // Client::new exercises /health, /api/version, /v1/info on the ready ladder.
    let client =
        lens_client::Client::new(Connection::new(ConnectionId::new("live"), base, Auth::None))
            .unwrap();

    // Registries + info (no session needed).
    reachable("/v1/me", client.info().me());
    reachable("/v1/agents", client.agents().list());
    reachable("/v1/hosts", client.hosts().list());
    reachable("/v1/policies", client.policies().list());

    // Session-scoped reads.
    let s = client.sessions();
    reachable("/v1/sessions", s.list(Default::default()));
    reachable("/v1/sessions/{id}", s.get(&sid, Default::default()));
    reachable("/v1/sessions/{id}/items", s.items(&sid, Default::default()));
    reachable("/v1/sessions/{id}/child_sessions", s.child_sessions(&sid, Default::default()));
    reachable("/v1/sessions/{id}/resources", s.resources(&sid));
}
```

> If `ClientError` lacks `is_transport()` / `is_decode()` discriminators, add them as small `pub fn` predicates on `ClientError` in `error.rs` in this task (matching the existing variant names — check `error.rs` first), with a one-line test. They're genuinely useful beyond this sweep. Otherwise match the concrete variants inline.

- [ ] **Step 3: Verify it compiles (gated)**

Run: `cargo test -p lens-client --features live-tests --test live_reachability --no-run`
Expected: compiles clean. Fix method-name/arg mismatches against the real signatures until it builds.

- [ ] **Step 4: clippy (gated) + fmt + commit**

```bash
cargo clippy -p lens-client --all-targets --features live-tests
cargo fmt -p lens-client -- --check
git add crates/lens-client/tests/live_reachability.rs crates/lens-client/src/error.rs
git commit -m "feat(lens-client): gated endpoint-reachability sweep (B6/§9)"
```

---

## Task 6: Live run + documentation

**Files:**
- Modify: `docs/design/typed-client-implementation.md` (§5)
- Modify: `vendor/omnigent-0.3.0.dev0/README.md`
- Modify: `docs/STATUS.md`
- Create: `docs/handoffs/2026-06-26-lens-client-plan3c-execution.md`

**Notes for the implementer:**
- This is the verification + docs task. **Stand up a real server** and run the two gated tests (Tasks 4 + 5). The box runs the omnigent server fine (transport spike 2026-06-25); only non-`claude-sdk` runners are constrained, and `claude-sdk` is enough here.
- Use the `installing-omnigent-from-source` skill to get a pin-matched `0.3.0.dev0` server (`omnigent --version` must match; ground-truth discipline). Confirm via the ready ladder before testing.

- [ ] **Step 1: Bring up the server (skill)**

Invoke the `installing-omnigent-from-source` skill. Start the daemon per the server-lifecycle spawn path (transport spike §3). Verify: `GET /health` → `GET /api/version` reports `0.3.0.dev0`.

- [ ] **Step 2: Create an idle claude-sdk session, export env**

Create a runner-backed claude-sdk session (the spike path / `live_stream` setup). Export `LENS_OMNIGENT_URL` and `LENS_OMNIGENT_SESSION_ID`.

- [ ] **Step 3: Run the gated live checks**

```bash
cargo test -p lens-client --features live-tests --test live_taxonomy -- --nocapture
cargo test -p lens-client --features live-tests --test live_reachability -- --nocapture
```
Expected: both PASS. Capture the `--nocapture` output (declared-types confirmation; per-endpoint `ok` lines).

> **Fallback (only if bring-up genuinely fails):** record the *exact* blocker in the handoff (matching prior sessions' honesty discipline), mark the two gated tests `--no-run`-verified only, and leave a one-line "live run deferred" in STATUS. Do **not** claim a green live run that didn't happen.

- [ ] **Step 4: Update the design + vendor docs**

In `docs/design/typed-client-implementation.md` §5, mark `xtask drift` as built and add the static `taxonomy_drift` test to the "always green, no server" line. In `vendor/omnigent-0.3.0.dev0/README.md`, replace the "CI should diff this…" sentence with the concrete command: `cargo run -p xtask -- drift` (default `--against ../omnigent/openapi.json`).

- [ ] **Step 5: STATUS + handoff**

Update `docs/STATUS.md` (Plan 3c done; close the B6 / Plan 3 thread). Write `docs/handoffs/2026-06-26-lens-client-plan3c-execution.md` (what shipped, the live-run result or its deferral, deferred items). Per memory `end-of-session-status-update`.

- [ ] **Step 6: Commit**

```bash
git add docs/ vendor/omnigent-0.3.0.dev0/README.md
git commit -m "docs(status): Plan 3c contract-drift CI executed; B6 closed; §5 + vendor README reconciled"
```

---

## Self-Review

**Spec coverage (B6 / typed-client §9 / ADR-0001):**
- §9 "Vendor openapi.json + CI step diffs vendored vs sibling (path enumeration + SSE schema)" → Tasks 1 + 2 (`xtask drift`). ✓
- §9 "Startup taxonomy diff — fail loud on unknown/changed shapes" → split: offline static half (Task 3, always-on) + runtime half (Task 4, gated). ✓
- §9 "Endpoint reachability — every §3 endpoint pinged once" → Task 5. ✓
- ADR-0001 "ignore `/hooks/*-request` runner callbacks; passive alarm; commit-is-canonical" → Task 1 `/hooks/` filter; drift is local non-zero-exit alarm. ✓
- D3 "Local verification (no CI), xtask is the local-CI home" → no `.github/workflows`; drift is an xtask command; live gated. ✓
- "incl. live run" (user decision) → Task 6 stands up a real server and runs Tasks 4+5. ✓

**Placeholder scan:** every code step shows complete code. Task 5 deliberately defers exact method names to a Step-1 signature check (the typed read surface is real but its accessor names must be copied from source, not guessed) — flagged, not a blind placeholder. ✓

**Type consistency:** `SetDiff`/`diff_sets` defined in Task 1, reused in Task 2. `ACCOUNTED_EVENT_TYPES` defined in Task 3, consumed in Tasks 4 (live) and the Task 3 test. `SseDiff`/`diff_sse`/`member_shapes`/`sse_event_types` consistent within Task 2. Live tests mirror the verified `tests/live_stream.rs` shape (`Client::new` / `sessions().stream` / `send_event`). ✓

**Counts to expect at runtime:** 59 openapi paths − 4 `/hooks/*` = 55 client paths; 47 SSE discriminators = 33 modeled + 14 deferred. If these change, the contract drifted — investigate before "fixing" the tests.
