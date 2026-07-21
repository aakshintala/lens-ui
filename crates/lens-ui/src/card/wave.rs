use super::model::{READY_DECAY_MS, SessionCard};
use crate::theme::LensTheme;
use gpui::Hsla;
use lens_core::domain::scalars::{SessionLifecycle, SessionStatusValue};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wave {
    NeedsInput,
    Ready,
    Working,
    Failed,
    Slept,
    Neutral,
    AwaitingReview,
    Scheduled,
}

/// Priority ladder for card status glow (shell §5.1). Ready uses `UiClock` window
/// math; the decay wake is a separate gpui executor timer (dual-clock — see poller).
pub fn derive_wave(card: &SessionCard, now_ms: i64, is_focused: bool) -> Wave {
    // 1. NeedsInput — hard block wins.
    if card.needs_attention
        && card.status != SessionStatusValue::Failed
        && card.last_task_error.is_none()
    {
        return Wave::NeedsInput;
    }
    // 2. Failed.
    if card.status == SessionStatusValue::Failed || card.last_task_error.is_some() {
        return Wave::Failed;
    }
    // 3. Working — active now (latent schedule/review is background).
    if matches!(
        card.status,
        SessionStatusValue::Running | SessionStatusValue::Launching | SessionStatusValue::Waiting
    ) {
        return Wave::Working;
    }
    // 4. AwaitingReview — soft async attention; settles here after the turn ends,
    //    above Ready so a just-finished review-parked turn does not flash Ready.
    if card.awaiting_review {
        return Wave::AwaitingReview;
    }
    // 5. Scheduled — Active + idle + a future wake (self-clears once now passes it).
    if card.lifecycle == SessionLifecycle::Active
        && card.status == SessionStatusValue::Idle
        && card.scheduled_wake_at.is_some_and(|t| t > now_ms)
    {
        return Wave::Scheduled;
    }
    // 6. Ready — just finished, glance (idle + recent completion, unfocused).
    if card.status == SessionStatusValue::Idle
        && card
            .last_completed_at
            .is_some_and(|t| now_ms.saturating_sub(t) < READY_DECAY_MS)
        && !is_focused
    {
        return Wave::Ready;
    }
    // 7. Slept.
    if card.lifecycle == SessionLifecycle::Slept {
        return Wave::Slept;
    }
    Wave::Neutral
}

impl Wave {
    /// The saturated status color for this wave (spec §7). Keeps `theme` a leaf — the
    /// Wave→status map lives here in `card`, not in `theme`.
    pub fn status_color(self, t: &LensTheme) -> Hsla {
        match self {
            Wave::Ready => t.status.ready,
            Wave::Working => t.status.working,
            Wave::NeedsInput => t.status.needs_input,
            Wave::Failed => t.status.failed,
            Wave::Slept => t.status.slept,
            Wave::Neutral => t.status.neutral,
            Wave::AwaitingReview => t.status.awaiting_review,
            Wave::Scheduled => t.status.scheduled,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lens_core::domain::ids::SessionId;

    #[test]
    fn wave_ladder_priority_needs_input_over_ready() {
        let mut card = SessionCard::new(SessionId::new("s"));
        card.status = SessionStatusValue::Idle;
        card.needs_attention = true;
        card.last_completed_at = Some(1_000);
        assert_eq!(derive_wave(&card, 1_100, false), Wave::NeedsInput);
    }

    #[test]
    fn ready_requires_idle_and_recent_completion_suppressed_when_focused() {
        let mut card = SessionCard::new(SessionId::new("s"));
        card.status = SessionStatusValue::Idle;
        card.last_completed_at = Some(1_000);
        assert_eq!(derive_wave(&card, 1_000 + 60_000, false), Wave::Ready);
        assert_eq!(derive_wave(&card, 1_000 + 60_000, true), Wave::Neutral);
        assert_eq!(
            derive_wave(&card, 1_000 + READY_DECAY_MS + 1, false),
            Wave::Neutral
        );
    }

    #[test]
    fn failed_status_with_needs_attention_derives_failed_not_needs_input() {
        let mut card = SessionCard::new(SessionId::new("s"));
        card.status = SessionStatusValue::Failed;
        card.needs_attention = true;
        assert_eq!(derive_wave(&card, 0, false), Wave::Failed);
    }

    #[test]
    fn failed_wave_from_last_task_error_even_when_idle() {
        let mut card = SessionCard::new(SessionId::new("s"));
        card.status = SessionStatusValue::Idle;
        card.last_task_error = Some(lens_core::domain::scalars::ErrorInfo {
            code: "e".into(),
            message: "boom".into(),
        });
        assert_eq!(derive_wave(&card, 0, false), Wave::Failed);
    }

    #[test]
    fn awaiting_review_below_needs_input() {
        let mut card = SessionCard::new(SessionId::new("s"));
        card.status = SessionStatusValue::Idle;
        card.needs_attention = true;
        card.awaiting_review = true;
        assert_eq!(derive_wave(&card, 0, false), Wave::NeedsInput);
    }

    #[test]
    fn awaiting_review_below_failed() {
        let mut card = SessionCard::new(SessionId::new("s"));
        card.status = SessionStatusValue::Failed;
        card.awaiting_review = true;
        assert_eq!(derive_wave(&card, 0, false), Wave::Failed);
    }

    #[test]
    fn working_beats_awaiting_review() {
        let mut card = SessionCard::new(SessionId::new("s"));
        card.status = SessionStatusValue::Running;
        card.awaiting_review = true;
        assert_eq!(derive_wave(&card, 0, false), Wave::Working);
    }

    #[test]
    fn settles_to_awaiting_review_after_turn_ends() {
        let mut card = SessionCard::new(SessionId::new("s"));
        card.status = SessionStatusValue::Idle;
        card.awaiting_review = true;
        card.last_completed_at = Some(1_000);
        assert_eq!(
            derive_wave(&card, 1_000 + 60_000, false),
            Wave::AwaitingReview
        );
    }

    #[test]
    fn scheduled_requires_future_wake() {
        let mut card = SessionCard::new(SessionId::new("s"));
        card.status = SessionStatusValue::Idle;
        card.scheduled_wake_at = Some(1_000);
        assert_ne!(derive_wave(&card, 2_000, false), Wave::Scheduled);
        card.scheduled_wake_at = Some(5_000);
        assert_eq!(derive_wave(&card, 2_000, false), Wave::Scheduled);
    }

    #[test]
    fn scheduled_beats_ready() {
        let now = 10_000_i64;
        let mut card = SessionCard::new(SessionId::new("s"));
        card.status = SessionStatusValue::Idle;
        card.scheduled_wake_at = Some(now + 10_000);
        card.last_completed_at = Some(now);
        assert_eq!(derive_wave(&card, now, false), Wave::Scheduled);
    }

    #[test]
    fn scheduled_below_working() {
        let now = 10_000_i64;
        let mut card = SessionCard::new(SessionId::new("s"));
        card.status = SessionStatusValue::Running;
        card.scheduled_wake_at = Some(now + 10_000);
        assert_eq!(derive_wave(&card, now, false), Wave::Working);
    }

    #[test]
    fn scheduled_requires_active_not_slept() {
        let now = 10_000_i64;
        let mut card = SessionCard::new(SessionId::new("s"));
        card.status = SessionStatusValue::Idle;
        card.lifecycle = SessionLifecycle::Slept;
        card.scheduled_wake_at = Some(now + 10_000);
        assert_eq!(derive_wave(&card, now, false), Wave::Slept);
    }

    #[test]
    fn status_color_total_over_all_waves() {
        let t: crate::theme::LensTheme =
            serde_json::from_str(include_str!("../theme/lens-dark.json")).unwrap();
        // Every variant resolves to its corresponding status token (compile-time exhaustiveness is
        // the real totality guard; this pins the mapping so a mis-wire is caught).
        assert_eq!(Wave::Ready.status_color(&t), t.status.ready);
        assert_eq!(Wave::Working.status_color(&t), t.status.working);
        assert_eq!(Wave::NeedsInput.status_color(&t), t.status.needs_input);
        assert_eq!(Wave::Failed.status_color(&t), t.status.failed);
        assert_eq!(Wave::Slept.status_color(&t), t.status.slept);
        assert_eq!(Wave::Neutral.status_color(&t), t.status.neutral);
        assert_eq!(
            Wave::AwaitingReview.status_color(&t),
            t.status.awaiting_review
        );
        assert_eq!(Wave::Scheduled.status_color(&t), t.status.scheduled);
    }
}
