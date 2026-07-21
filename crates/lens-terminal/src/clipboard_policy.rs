//! Session-scoped clipboard/paste policy seam. In-memory here; `lens-ui` injects
//! a persisted impl later (spec 2026-07-20 amendment — persistence deferred).

use std::collections::HashMap;

use crate::ClipboardLocation;
use crate::HostRequestDecision;

/// Foreground policy for permissioned clipboard writes (OSC 52) + paste warnings.
pub trait ClipboardPolicy {
    fn paste_warn_suppressed(&self) -> bool;
    fn suppress_paste_warn(&mut self);
    /// A remembered `Allow`/`Deny` for this location this session, if any.
    fn osc52_session_decision(&self, location: &ClipboardLocation) -> Option<HostRequestDecision>;
    fn remember_osc52(&mut self, location: ClipboardLocation, decision: HostRequestDecision);
}

/// Default in-memory policy: everything resets on process exit.
#[derive(Default)]
pub struct SessionClipboardPolicy {
    paste_warn_suppressed: bool,
    osc52: HashMap<ClipboardLocation, HostRequestDecision>,
}

impl ClipboardPolicy for SessionClipboardPolicy {
    fn paste_warn_suppressed(&self) -> bool {
        self.paste_warn_suppressed
    }
    fn suppress_paste_warn(&mut self) {
        self.paste_warn_suppressed = true;
    }
    fn osc52_session_decision(&self, location: &ClipboardLocation) -> Option<HostRequestDecision> {
        self.osc52.get(location).cloned()
    }
    fn remember_osc52(&mut self, location: ClipboardLocation, decision: HostRequestDecision) {
        self.osc52.insert(location, decision);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ClipboardLocation;
    use crate::HostRequestDecision;

    #[test]
    fn session_policy_defaults_to_no_suppression_no_decision() {
        let p = SessionClipboardPolicy::default();
        assert!(!p.paste_warn_suppressed());
        assert_eq!(p.osc52_session_decision(&ClipboardLocation::Standard), None);
    }

    #[test]
    fn remembering_osc52_allow_is_returned_for_same_location_only() {
        let mut p = SessionClipboardPolicy::default();
        p.remember_osc52(ClipboardLocation::Standard, HostRequestDecision::Allow);
        assert_eq!(
            p.osc52_session_decision(&ClipboardLocation::Standard),
            Some(HostRequestDecision::Allow)
        );
        assert_eq!(p.osc52_session_decision(&ClipboardLocation::Primary), None);
    }

    #[test]
    fn suppress_paste_warn_sticks() {
        let mut p = SessionClipboardPolicy::default();
        p.suppress_paste_warn();
        assert!(p.paste_warn_suppressed());
    }
}
