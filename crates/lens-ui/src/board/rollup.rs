//! Pure group aggregation (B-3, spec §3.1): fold member `SessionCard`s into the
//! header-lane rollup (spend / age / ✓N-completed) and format it. No gpui — pure,
//! deterministic, unit-tested. `completed_count` is Archive-side (B-6); B-3 carries
//! it as an input defaulting to 0.

use crate::card::model::SessionCard;

/// Pure fold over a group's member cards (spec §3.1). `completed_count` is supplied
/// by the caller (Archive-side; B-6 wires the real source, B-3 passes 0).
#[derive(Clone, Debug, PartialEq)]
pub struct GroupRollup {
    /// Σ member `total_cost_usd`; `None` only when no member reports a cost.
    pub spend_usd: Option<f64>,
    /// Oldest member `created_at` (epoch SECONDS); `None` when no member has one.
    pub oldest_created_at: Option<i64>,
    /// Completed → Archive count (B-6 source); 0 in B-3.
    pub completed_count: u32,
}

pub fn group_rollup(members: &[&SessionCard], completed_count: u32) -> GroupRollup {
    let mut spend_usd: Option<f64> = None;
    let mut oldest_created_at: Option<i64> = None;
    for m in members {
        if let Some(c) = m.cumulative_cost.total_cost_usd {
            spend_usd = Some(spend_usd.unwrap_or(0.0) + c);
        }
        if let Some(ca) = m.created_at {
            oldest_created_at = Some(oldest_created_at.map_or(ca, |o| o.min(ca)));
        }
    }
    GroupRollup {
        spend_usd,
        oldest_created_at,
        completed_count,
    }
}

/// `~$X.XX`, or `—` when unknown. Mirrors `card::chrome::format_spend`.
pub fn format_group_spend(spend_usd: Option<f64>) -> String {
    match spend_usd {
        Some(usd) => format!("~${usd:.2}"),
        None => "—".into(),
    }
}

/// Coarse age bucket from the oldest member's `created_at` (epoch SECONDS) vs the
/// current UI clock (epoch MILLIS). `—` when no source. Never negative.
pub fn format_age(oldest_created_at_secs: Option<i64>, now_ms: i64) -> String {
    let Some(created) = oldest_created_at_secs else {
        return "—".into();
    };
    let now_s = now_ms / 1000;
    // saturating: a corrupt/extreme server-provided `created` must never overflow
    // (debug-build panic) — the UI is no-panic (codex review, B-3).
    let secs = now_s.saturating_sub(created).max(0);
    if secs < 3_600 {
        format!("{}m", secs / 60)
    } else if secs < 86_400 {
        format!("{}h", secs / 3_600)
    } else {
        format!("{}d", secs / 86_400)
    }
}

/// The header-lane text (spec §3): `name · spend · age · ✓N`. The colored dot and
/// the `⌄` caret are rendered as elements, not part of this string.
pub fn group_header_text(name: &str, rollup: &GroupRollup, now_ms: i64) -> String {
    format!(
        "{name} · {} · {} · ✓{}",
        format_group_spend(rollup.spend_usd),
        format_age(rollup.oldest_created_at, now_ms),
        rollup.completed_count
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use lens_core::domain::ids::SessionId;
    use lens_core::domain::usage::Cost;

    fn card(id: &str, cost: Option<f64>, created_at: Option<i64>) -> SessionCard {
        let mut c = SessionCard::new(SessionId::new(id));
        c.cumulative_cost = Cost {
            total_cost_usd: cost,
            ..Cost::default()
        };
        c.created_at = created_at;
        c
    }

    #[test]
    fn rollup_sums_spend_and_takes_oldest_created_at() {
        let a = card("s1", Some(1.50), Some(2000));
        let b = card("s2", Some(2.00), Some(1000));
        let members = [&a, &b];
        let r = group_rollup(&members, 3);
        assert_eq!(r.spend_usd, Some(3.50));
        assert_eq!(r.oldest_created_at, Some(1000)); // min, not max
        assert_eq!(r.completed_count, 3);
    }

    #[test]
    fn rollup_partial_data_is_tolerated() {
        // one member has cost, neither the other; one has created_at, not the other.
        let a = card("s1", Some(0.75), None);
        let b = card("s2", None, Some(5000));
        let members = [&a, &b];
        let r = group_rollup(&members, 0);
        assert_eq!(r.spend_usd, Some(0.75));
        assert_eq!(r.oldest_created_at, Some(5000));
    }

    #[test]
    fn rollup_all_absent_is_none() {
        let a = card("s1", None, None);
        let members = [&a];
        let r = group_rollup(&members, 0);
        assert_eq!(r.spend_usd, None);
        assert_eq!(r.oldest_created_at, None);
    }

    #[test]
    fn format_spend_matches_card_style() {
        assert_eq!(format_group_spend(Some(3.5)), "~$3.50");
        assert_eq!(format_group_spend(None), "—");
    }

    #[test]
    fn format_age_buckets_minutes_hours_days() {
        // created 1000s ago; now 1000s + 2600s = 3600s → 2600s = 43m
        assert_eq!(format_age(Some(1000), 3_600_000), "43m");
        // 2h: created at 0s, now 7200s
        assert_eq!(format_age(Some(0), 7_200_000), "2h");
        // 3d: created at 0s, now 3*86400s
        assert_eq!(format_age(Some(0), 259_200_000), "3d");
        // absent → em dash
        assert_eq!(format_age(None, 3_600_000), "—");
        // future/zero clamps to 0m, never negative
        assert_eq!(format_age(Some(10_000), 0), "0m");
        // extreme/corrupt created must not overflow-panic (saturating_sub) — no-panic UI.
        assert!(format_age(Some(i64::MIN), 0).ends_with('d'));
    }

    #[test]
    fn header_text_assembles_spec_order() {
        let r = GroupRollup {
            spend_usd: Some(3.5),
            oldest_created_at: Some(0),
            completed_count: 2,
        };
        assert_eq!(
            group_header_text("Refactor", &r, 7_200_000),
            "Refactor · ~$3.50 · 2h · ✓2"
        );
    }
}
