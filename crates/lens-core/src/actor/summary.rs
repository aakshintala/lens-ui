//! Coarse card-summary projection for background-warm sessions (§6, D10).

use crate::domain::SessionState;
use crate::domain::ids::HostId;
use crate::domain::scalars::SessionStatusValue;

/// Coarse card-summary — distinct from `StreamUpdate` (spec §6). Two producers
/// (actor here; §10 poll later). apply = copy-assignment of scalars.
#[derive(Clone, Debug, PartialEq)]
pub struct SummaryUpdate {
    pub status: SessionStatusValue,
    pub title: Option<String>,
    pub last_total_tokens: Option<u64>,
    pub host_id: Option<HostId>,
    pub needs_attention: bool,
    pub subagent_active: bool,
}

impl SummaryUpdate {
    pub fn from_state(s: &SessionState) -> Self {
        Self {
            status: s.status,
            title: s.title.clone(),
            last_total_tokens: s.last_total_tokens,
            host_id: s.host_id.clone(),
            needs_attention: !s.pending_elicitations.is_empty()
                || s.status == SessionStatusValue::Failed,
            // TODO(§9): derive from child-session registry once it exists.
            subagent_active: false,
        }
    }
}
