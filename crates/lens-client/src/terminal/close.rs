/// Classified WebSocket close cause — policy (stop/retry/reattach) is Slice 1d.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum CloseCause {
    TerminalNotFound,
    TerminalDetached,
    Internal,
    Unauthorized,
    Network,
}

pub(crate) fn classify_close(code: u16) -> CloseCause {
    match code {
        4404 => CloseCause::TerminalNotFound,
        4405 => CloseCause::TerminalDetached,
        4500 => CloseCause::Internal,
        1008 => CloseCause::Unauthorized,
        _ => CloseCause::Network,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_known_codes() {
        assert!(matches!(classify_close(4404), CloseCause::TerminalNotFound));
        assert!(matches!(classify_close(4405), CloseCause::TerminalDetached));
        assert!(matches!(classify_close(4500), CloseCause::Internal));
        assert!(matches!(classify_close(1008), CloseCause::Unauthorized));
        assert!(matches!(classify_close(1006), CloseCause::Network));
    }
}
