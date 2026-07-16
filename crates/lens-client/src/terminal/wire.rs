use super::close::CloseCause;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WsInbound {
    Vt(Vec<u8>),
    Text(String),
    Closed(CloseCause),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WsOutbound {
    Input(Vec<u8>),
    Resize { cols: u16, rows: u16 },
}

pub(crate) fn encode_outbound(_o: &WsOutbound) -> tokio_tungstenite::tungstenite::Message {
    todo!("Task 3")
}

pub(crate) fn classify_inbound(
    _msg: tokio_tungstenite::tungstenite::Message,
) -> Option<WsInbound> {
    todo!("Task 3")
}
