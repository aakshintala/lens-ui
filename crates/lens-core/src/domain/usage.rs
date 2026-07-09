//! Token/cost accounting and presence (§2.5).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Per-model rollup (§2.5). Field set mirrors the pinned wire contract
/// (`lens_client::generated::ModelUsage`, generated.rs:3467) so no server data is
/// lost before P1/P2: cache buckets + per-model `total_cost_usd`. All optional —
/// "priced ⟺ key present" (a `None` cost means the model was unpriced), and the
/// harness may omit any token bucket.
///
/// NOT `Eq` — it holds `f64` (`total_cost_usd`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ModelUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub cache_creation_input_tokens: Option<u64>,
    pub cache_read_input_tokens: Option<u64>,
    pub total_cost_usd: Option<f64>,
}

/// NOT `Eq` — transitively holds `ModelUsage`'s `f64` via `usage_by_model`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub reasoning_tokens: Option<u64>,
    pub context_tokens: Option<u64>,
    /// Per-model rollup (0.2.0). Empty when the server reports no breakdown.
    pub usage_by_model: BTreeMap<String, ModelUsage>,
}

/// Accumulated client-side from `session.usage` events; the USD figure is
/// SERVER-computed (`total_cost_usd`) — Lens keeps no price table (§2.5).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Cost {
    pub cumulative_usage: Usage,
    pub total_cost_usd: Option<f64>,
}

/// `session.presence` — wire-faithful shape (§2.5). Transient/RAM-only; never
/// persisted. Carries NO display_name/is_owner/last_seen_at (those are derived
/// separately and joined by user_id).
///
/// P1 HANDOFF NOTE: the current `lens_client::stream::PresenceViewer` wrapper
/// exposes ONLY `user_id` (event.rs:146) — it drops `joined_at`/`idle` that the
/// generated contract (generated.rs:4220) carries. P1 cannot populate this domain
/// type from `ServerStreamEvent::Presence` until lens-client's stream wrapper is
/// widened (or P1 reads the generated type). Flagged for the P1 spec — NOT a P0 fix.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresenceViewer {
    pub user_id: String,
    /// ISO 8601 UTC; stable across reconnect within the leave-grace window.
    pub joined_at: String,
    pub idle: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_default_is_zeroed_and_roundtrips() {
        let u = Usage::default();
        assert_eq!(u.total_tokens, 0);
        assert!(u.usage_by_model.is_empty());
        let back: Usage = serde_json::from_str(&serde_json::to_string(&u).unwrap()).unwrap();
        assert_eq!(back, u);
    }

    #[test]
    fn usage_with_per_model_rollup_roundtrips() {
        let mut by_model = BTreeMap::new();
        by_model.insert(
            "claude-opus".to_string(),
            ModelUsage {
                input_tokens: Some(10),
                output_tokens: Some(20),
                total_tokens: Some(30),
                cache_creation_input_tokens: Some(2),
                cache_read_input_tokens: Some(8),
                total_cost_usd: Some(0.42),
            },
        );
        let u = Usage {
            input_tokens: 10,
            output_tokens: 20,
            total_tokens: 30,
            reasoning_tokens: Some(5),
            context_tokens: None,
            usage_by_model: by_model,
        };
        let back: Usage = serde_json::from_str(&serde_json::to_string(&u).unwrap()).unwrap();
        assert_eq!(back, u);
    }

    #[test]
    fn cost_roundtrips_with_and_without_usd() {
        let c = Cost {
            cumulative_usage: Usage::default(),
            total_cost_usd: Some(1.25),
        };
        let back: Cost = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        assert_eq!(back, c);
        let c0 = Cost::default();
        assert_eq!(c0.total_cost_usd, None);
    }

    #[test]
    fn presence_viewer_roundtrips() {
        let p = PresenceViewer {
            user_id: "u_1".into(),
            joined_at: "2026-07-08T00:00:00Z".into(),
            idle: false,
        };
        let back: PresenceViewer =
            serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(back, p);
    }
}
