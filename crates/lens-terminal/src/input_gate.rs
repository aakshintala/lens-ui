//! Read-only / input-enabled gate for encoded PTY input (keys, focus reports).

use crate::AccessMode;

/// Returns whether keyboard / focus-report bytes may be encoded and egressed.
pub fn write_input_allowed(access: AccessMode, input_enabled: bool) -> bool {
    matches!(access, AccessMode::Write) && input_enabled
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_input_allowed_requires_write_access_and_input_enabled() {
        assert!(!write_input_allowed(AccessMode::ReadOnly, true));
        assert!(!write_input_allowed(AccessMode::Write, false));
        assert!(write_input_allowed(AccessMode::Write, true));
    }
}
