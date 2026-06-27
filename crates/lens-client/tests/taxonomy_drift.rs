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
