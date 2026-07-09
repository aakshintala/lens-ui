//! Text + reasoning accumulation over `StreamScratch` (§4.2).

use crate::domain::item::{MessageAcc, StreamScratch};
use crate::reduce::{StreamUpdate, Updates};
use smallvec::smallvec;

pub(crate) fn accumulate_text(
    scratch: &mut StreamScratch,
    delta: &str,
    message_id: Option<&str>,
    index: Option<usize>,
) -> Updates {
    let acc = scratch.open_message.get_or_insert_with(|| MessageAcc {
        message_id: message_id.map(str::to_string),
        text: String::new(),
        block_index: index.unwrap_or(0),
    });
    acc.text.push_str(delta);
    if let Some(i) = index {
        acc.block_index = i;
    }
    smallvec![StreamUpdate::ScratchChanged]
}

#[cfg(test)]
mod tests {
    use crate::clock::ManualClock;
    use crate::domain::{AgentId, ConnectionId, SessionId, SessionState};
    use crate::reduce::{StreamUpdate, reduce};
    use lens_client::stream::{ResponseEvent, ServerStreamEvent};

    fn st() -> SessionState {
        SessionState::new(
            ConnectionId::new("c"),
            SessionId::new("conv"),
            AgentId::new("ag"),
        )
    }

    fn clock() -> ManualClock {
        ManualClock::new(1_700_000_000_000)
    }

    fn resp_text(delta: &str, message_id: Option<&str>, index: Option<usize>) -> ServerStreamEvent {
        ServerStreamEvent::Response(ResponseEvent::OutputTextDelta {
            delta: delta.into(),
            message_id: message_id.map(str::to_string),
            index,
            last: None,
        })
    }

    #[test]
    fn text_deltas_accumulate_in_scratch() {
        let mut s = st();
        reduce(&mut s, &resp_text("Hel", None, None), &clock());
        let u = reduce(&mut s, &resp_text("lo", None, None), &clock());
        let acc = s.stream.open_message.as_ref().unwrap();
        assert_eq!(acc.text, "Hello");
        assert_eq!(&u[..], &[StreamUpdate::ScratchChanged]);
    }
}
