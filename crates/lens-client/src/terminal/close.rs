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

pub(crate) fn classify_close(_code: u16) -> CloseCause {
    todo!("Task 4")
}
