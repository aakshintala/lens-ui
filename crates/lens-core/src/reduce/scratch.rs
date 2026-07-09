//! Text + reasoning accumulation over `StreamScratch` (§4.2).

use crate::domain::item::{MessageAcc, StreamScratch};
use crate::reduce::{StreamUpdate, Updates};
use smallvec::smallvec;
use std::sync::Arc;

pub(crate) enum ReasoningKind {
    Full,
    Summary,
}

pub(crate) fn accumulate_reasoning(
    scratch: &mut StreamScratch,
    kind: ReasoningKind,
    delta: &str,
) -> Updates {
    let acc = scratch.open_reasoning.get_or_insert_with(Default::default);
    match kind {
        ReasoningKind::Full => acc.full_text.push_str(delta),
        ReasoningKind::Summary => acc.summary_text.push_str(delta),
    }
    smallvec![StreamUpdate::ScratchChanged(Arc::new(scratch.clone()))]
}

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
    smallvec![StreamUpdate::ScratchChanged(Arc::new(scratch.clone()))]
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
    fn reasoning_deltas_accumulate() {
        let mut s = st();
        reduce(
            &mut s,
            &ServerStreamEvent::Response(ResponseEvent::ReasoningStarted),
            &clock(),
        );
        reduce(
            &mut s,
            &ServerStreamEvent::Response(ResponseEvent::ReasoningTextDelta { delta: "be".into() }),
            &clock(),
        );
        reduce(
            &mut s,
            &ServerStreamEvent::Response(ResponseEvent::ReasoningTextDelta {
                delta: "cause".into(),
            }),
            &clock(),
        );
        assert_eq!(
            s.stream.open_reasoning.as_ref().unwrap().full_text,
            "because"
        );
    }

    #[test]
    fn text_deltas_accumulate_in_scratch() {
        let mut s = st();
        reduce(&mut s, &resp_text("Hel", None, None), &clock());
        let u = reduce(&mut s, &resp_text("lo", None, None), &clock());
        let acc = s.stream.open_message.as_ref().unwrap();
        assert_eq!(acc.text, "Hello");
        assert!(matches!(&u[..], [StreamUpdate::ScratchChanged(_)]));
    }
}
