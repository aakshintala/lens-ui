use tokio_tungstenite::tungstenite::Message;

use super::close::{CloseCause, classify_close};

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

#[allow(dead_code)]
pub(crate) fn encode_outbound(o: &WsOutbound) -> Message {
    match o {
        WsOutbound::Input(bytes) => Message::Binary(bytes.clone().into()),
        WsOutbound::Resize { cols, rows } => {
            Message::Text(format!(r#"{{"type":"resize","cols":{cols},"rows":{rows}}}"#).into())
        }
    }
}

#[allow(dead_code)]
pub(crate) fn classify_inbound(msg: Message) -> Option<WsInbound> {
    match msg {
        Message::Binary(bin) => Some(WsInbound::Vt(bin.into())),
        Message::Text(text) => Some(WsInbound::Text(text.to_string())),
        Message::Close(frame) => {
            let code = frame.map(|f| f.code.into()).unwrap_or(1006);
            Some(WsInbound::Closed(classify_close(code)))
        }
        Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resize_encodes_exact_json_text_frame() {
        let m = encode_outbound(&WsOutbound::Resize {
            cols: 120,
            rows: 40,
        });
        match m {
            Message::Text(t) => {
                assert_eq!(t.as_str(), r#"{"type":"resize","cols":120,"rows":40}"#);
            }
            _ => panic!("expected text frame"),
        }
    }

    #[test]
    fn input_encodes_binary_frame_verbatim() {
        let m = encode_outbound(&WsOutbound::Input(vec![0x1b, b'[', b'A']));
        match m {
            Message::Binary(b) => assert_eq!(&b[..], &[0x1b, b'[', b'A']),
            _ => panic!("expected binary frame"),
        }
    }

    #[test]
    fn binary_inbound_is_vt_bytes_verbatim() {
        let got = classify_inbound(Message::Binary(vec![0x1b, b'c'].into()));
        assert!(matches!(got, Some(WsInbound::Vt(b)) if b == vec![0x1b, b'c']));
    }

    #[test]
    fn ping_pong_are_ignored() {
        assert!(classify_inbound(Message::Ping(vec![].into())).is_none());
    }
}
