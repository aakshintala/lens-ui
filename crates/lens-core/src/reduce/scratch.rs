//! Text + reasoning accumulation over `StreamScratch` (§4.2).

use crate::domain::ids::AccId;
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
    new_acc_id: Option<AccId>,
) -> Updates {
    let acc = scratch.open_message.get_or_insert_with(|| MessageAcc {
        acc_id: new_acc_id.expect("pre-minted acc_id required when opening message acc"),
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

    #[test]
    fn open_message_keeps_stable_acc_id_across_deltas() {
        let mut s = st();
        reduce(&mut s, &resp_text("hel", None, None), &clock());
        let first = s.stream.open_message.as_ref().unwrap().acc_id.clone();
        assert!(
            !first.as_str().is_empty(),
            "acc_id must be non-empty at open"
        );
        reduce(&mut s, &resp_text("lo", None, None), &clock());
        let second = s.stream.open_message.as_ref().unwrap().acc_id.clone();
        assert_eq!(
            first, second,
            "acc_id must be stable across streaming deltas"
        );
    }

    #[test]
    fn reasoning_and_message_accs_get_distinct_acc_ids() {
        let mut s = st();
        reduce(
            &mut s,
            &ServerStreamEvent::Response(ResponseEvent::ReasoningStarted),
            &clock(),
        );
        reduce(&mut s, &resp_text("hi", None, None), &clock());
        let r = s.stream.open_reasoning.as_ref().unwrap().acc_id.clone();
        let m = s.stream.open_message.as_ref().unwrap().acc_id.clone();
        assert!(!r.as_str().is_empty());
        assert!(!m.as_str().is_empty());
        assert_ne!(r, m);
    }
}
