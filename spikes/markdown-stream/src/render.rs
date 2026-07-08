// Streaming render view (Task 5) — replay → sanitize → gpui-component markdown,
// fed one accumulation step per frame tick into ONE retained TextView keyed by a
// stable ElementId ("md-stream"). Times the per-frame TextView build and reports
// via the probe. Selection + scroll are enabled so the adversarial scenario
// (Task 6) can stress them.

use std::time::{Duration, Instant};

use gpui::{div, prelude::*, Context, Window};
use gpui_component::text::TextView;

use crate::probe::Probe;
use crate::replay;
use crate::sanitize::sanitize;

const GFM_STRESS: &str = include_str!("../fixtures/gfm-stress.md");
/// Delta size in characters; small enough that ticks land mid-construct often.
const CHUNK_CHARS: usize = 8;
/// Frame-tick cadence for advancing the stream.
const TICK_MS: u64 = 16;
/// Stable id — the whole point: same id every frame ⇒ retained state, no remount.
const STREAM_ID: &str = "md-stream";

pub struct StreamView {
    accums: Vec<String>,
    idx: usize,
    probe: Probe,
    done: bool,
}

impl StreamView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let deltas = replay::deltas(GFM_STRESS, CHUNK_CHARS);
        let accums = replay::accumulate(&deltas);

        // Drive the stream off a background timer, advancing one accumulation
        // step per tick and notifying so the view re-renders.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(TICK_MS))
                    .await;
                let keep_going = this
                    .update(cx, |view, cx| {
                        if view.idx + 1 < view.accums.len() {
                            view.idx += 1;
                            cx.notify();
                            true
                        } else {
                            if !view.done {
                                view.done = true;
                                // Finalize swap: the last streamed frame already
                                // IS the full text, fed with the same id — so the
                                // StreamingMessage→Message finalize is a no-op by
                                // construction here (no separate re-mount).
                                println!("FINALIZE: full text fed with stable id {STREAM_ID:?} — no-op swap");
                                println!("{}", view.probe.summary());
                            }
                            false
                        }
                    })
                    .unwrap_or(false);
                if !keep_going {
                    break;
                }
            }
        })
        .detach();

        Self {
            accums,
            idx: 0,
            probe: Probe::new(STREAM_ID),
            done: false,
        }
    }
}

impl Render for StreamView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let text = sanitize(&self.accums[self.idx]);
        let bytes = text.len();

        // Time ONLY the component build (not sanitize): this is our per-frame
        // touch of the retained TextView. Flat vs. bytes ⇒ no sync reparse.
        let t = Instant::now();
        let md = TextView::markdown(STREAM_ID, text, window, cx)
            .selectable(true)
            .scrollable(true);
        self.probe.note_tick(bytes, t.elapsed());

        div().size_full().p_4().child(md)
    }
}
