use super::model::{READY_DECAY_MS, SessionCard};
use lens_core::domain::scalars::{SessionLifecycle, SessionStatusValue};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wave {
    NeedsInput,
    Ready,
    Working,
    Failed,
    Slept,
    Neutral,
}

/// Priority ladder for card status glow (shell §5.1). Ready uses `UiClock` window
/// math; the decay wake is a separate gpui executor timer (dual-clock — see poller).
pub fn derive_wave(card: &SessionCard, now_ms: i64, is_focused: bool) -> Wave {
    if card.needs_attention
        && card.status != SessionStatusValue::Failed
        && card.last_task_error.is_none()
    {
        return Wave::NeedsInput;
    }
    if card.status == SessionStatusValue::Failed || card.last_task_error.is_some() {
        return Wave::Failed;
    }
    if card.status == SessionStatusValue::Idle
        && card
            .last_completed_at
            .is_some_and(|t| now_ms.saturating_sub(t) < READY_DECAY_MS)
        && !is_focused
    {
        return Wave::Ready;
    }
    if matches!(
        card.status,
        SessionStatusValue::Running | SessionStatusValue::Launching | SessionStatusValue::Waiting
    ) {
        return Wave::Working;
    }
    if card.lifecycle == SessionLifecycle::Slept {
        return Wave::Slept;
    }
    Wave::Neutral
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
}
