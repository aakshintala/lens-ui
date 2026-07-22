//! Pure group aggregation (B-3, spec §3.1): fold member `SessionCard`s into the
//! header-lane rollup (spend / age / ✓N-completed) and format it. No gpui — pure,
//! deterministic, unit-tested. `completed_count` is Archive-side (B-6); B-3 carries
//! it as an input defaulting to 0.

use crate::card::model::SessionCard;
use crate::card::wave::Wave;

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

/// The only two member fields a group rollup needs. Projecting to this at the read
/// site avoids cloning the entire `SessionCard` (strings/vecs/todos/usage) per member
/// per frame just to fold cost + age (codex final-review I2).
#[derive(Clone, Copy, Debug)]
pub struct MemberCost {
    pub spend_usd: Option<f64>,
    pub created_at: Option<i64>,
}

impl MemberCost {
    /// Narrow projection from a member card (read only the two fields we fold).
    pub fn from_card(card: &SessionCard) -> Self {
        Self {
            spend_usd: card.cumulative_cost.total_cost_usd,
            created_at: card.created_at,
        }
    }
}

pub fn group_rollup(members: &[MemberCost], completed_count: u32) -> GroupRollup {
    let mut spend_usd: Option<f64> = None;
    let mut oldest_created_at: Option<i64> = None;
    for m in members {
        if let Some(c) = m.spend_usd {
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

/// The status-rollup body of a collapsed group (spec §7 / §4.1): one row per
/// non-empty wave, in `derive_wave` priority-ladder order. `Neutral` is excluded
/// (no meaningful status). Pure — the caller projects each member card to a `Wave`
/// via `derive_wave` and passes the slice; label + dot color are resolved at render.
#[derive(Clone, Debug, PartialEq)]
pub struct StatusRollup {
    pub rows: Vec<(Wave, u32)>,
}

/// Rollup rank for a wave: `Some(priority)` in `derive_wave`'s resolution order, or
/// `None` for waves excluded from the rollup (`Neutral` = no meaningful status). This
/// `match` is EXHAUSTIVE, so adding a `Wave` variant is a COMPILE ERROR here until it is
/// placed in (or explicitly excluded from) the ladder — the "self-maintaining" property
/// (codex final-review Minor; a `const` array silently omitted new variants).
fn wave_rank(w: Wave) -> Option<u8> {
    match w {
        Wave::NeedsInput => Some(0),
        Wave::Failed => Some(1),
        Wave::Working => Some(2),
        Wave::AwaitingReview => Some(3),
        Wave::Scheduled => Some(4),
        Wave::Ready => Some(5),
        Wave::Slept => Some(6),
        Wave::Neutral => None,
    }
}

pub fn status_rollup(member_waves: &[Wave]) -> StatusRollup {
    // Distinct ranked waves present, each with its count, in ladder order.
    let mut ranked: Vec<(u8, Wave, u32)> = Vec::new();
    for &w in member_waves {
        let Some(rank) = wave_rank(w) else { continue };
        match ranked.iter_mut().find(|(_, cw, _)| *cw == w) {
            Some((_, _, n)) => *n += 1,
            None => ranked.push((rank, w, 1)),
        }
    }
    ranked.sort_by_key(|(rank, _, _)| *rank);
    let rows = ranked.into_iter().map(|(_, w, n)| (w, n)).collect();
    StatusRollup { rows }
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
        let members = [MemberCost::from_card(&a), MemberCost::from_card(&b)];
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
        let members = [MemberCost::from_card(&a), MemberCost::from_card(&b)];
        let r = group_rollup(&members, 0);
        assert_eq!(r.spend_usd, Some(0.75));
        assert_eq!(r.oldest_created_at, Some(5000));
    }

    #[test]
    fn rollup_all_absent_is_none() {
        let a = card("s1", None, None);
        let members = [MemberCost::from_card(&a)];
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
    fn status_rollup_counts_orders_and_drops_empties() {
        use crate::card::wave::Wave;
        // 2 Working, 1 Failed, 1 Ready, 1 Neutral (excluded).
        let waves = [
            Wave::Working,
            Wave::Ready,
            Wave::Working,
            Wave::Failed,
            Wave::Neutral,
        ];
        let r = status_rollup(&waves);
        // Ladder order: Failed before Working before Ready; Neutral absent; no zero rows.
        assert_eq!(
            r.rows,
            vec![(Wave::Failed, 1), (Wave::Working, 2), (Wave::Ready, 1)]
        );
    }

    #[test]
    fn status_rollup_empty_and_all_neutral_are_empty() {
        use crate::card::wave::Wave;
        assert!(status_rollup(&[]).rows.is_empty());
        assert!(
            status_rollup(&[Wave::Neutral, Wave::Neutral])
                .rows
                .is_empty()
        );
    }
}
