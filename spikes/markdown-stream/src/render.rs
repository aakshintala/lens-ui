// Streaming render view (Task 5/6) — replay → sanitize → gpui-component markdown,
// fed one accumulation step per frame tick into ONE retained TextView keyed by a
// stable ElementId ("md-stream"). Times the per-frame TextView build and reports
// via the probe. Selection + scroll are enabled so the adversarial scenario can
// stress them.

use std::time::{Duration, Instant};

use gpui::{div, prelude::*, Context, Window};
use gpui_component::text::TextView;

use crate::probe::Probe;
use crate::replay;
use crate::sanitize::sanitize;

/// GFM stress fixture (Task 5) — every construct, modest length.
const GFM_STRESS: &str = include_str!("../fixtures/gfm-stress.md");
/// Adversarial fixture (Task 6) — dangerous links, external image, unterminated
/// constructs at EOF.
const ADVERSARIAL: &str = include_str!("../fixtures/adversarial.md");
/// A large REAL GFM doc (Task 5 flat-scaling rerun + a doc long enough to scroll
/// while streaming). ~15KB of headings/tables/lists/fences/links.
const BIG: &str = include_str!("../../../docs/design/framework.md");

/// Delta size in characters; small enough that ticks land mid-construct often.
const CHUNK_CHARS: usize = 6;
/// Frame-tick cadence — slow enough to watch stream in.
const TICK_MS: u64 = 30;
/// Delay before streaming starts, so the window is visible first.
const START_DELAY_MS: u64 = 800;
/// Pause at end of each loop before restarting, so the finalized doc is visible.
const LOOP_PAUSE_MS: u64 = 1500;
/// Stable id — the whole point: same id every frame ⇒ retained state, no remount.
const STREAM_ID: &str = "md-stream";

#[derive(Clone, Copy)]
pub enum Source {
    Stress,
    Adversarial,
    Big,
}

impl Source {
    fn text(self) -> &'static str {
        match self {
            Source::Stress => GFM_STRESS,
            Source::Adversarial => ADVERSARIAL,
            Source::Big => BIG,
        }
    }
    fn label(self) -> &'static str {
        match self {
            Source::Stress => "gfm-stress",
            Source::Adversarial => "adversarial",
            Source::Big => "framework.md (big)",
        }
    }
}

pub struct StreamView {
    accums: Vec<String>,
    idx: usize,
    probe: Probe,
    loops: usize,
    label: &'static str,
}

impl StreamView {
    pub fn new(source: Source, cx: &mut Context<Self>) -> Self {
        let deltas = replay::deltas(source.text(), CHUNK_CHARS);
        let accums = replay::accumulate(&deltas);
        println!(
            "STREAM: source={} deltas={} — window opens, streaming starts in {}ms",
            source.label(),
            accums.len(),
            START_DELAY_MS
        );

        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(START_DELAY_MS))
                .await;
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(TICK_MS))
                    .await;
                let at_end = this
                    .update(cx, |view, cx| {
                        if view.idx + 1 < view.accums.len() {
                            view.idx += 1;
                            cx.notify();
                            false
                        } else {
                            view.loops += 1;
                            if view.loops == 1 {
                                // Finalize swap: the last streamed frame already IS
                                // the full text, fed with the same id — so the
                                // StreamingMessage→Message finalize is a no-op by
                                // construction (no separate re-mount).
                                println!(
                                    "FINALIZE: full text fed with stable id {STREAM_ID:?} — no-op swap"
                                );
                                println!("{}", view.probe.summary());
                            }
                            println!(
                                "LOOP {} done — restarting stream in {}ms (same id, text resets → also tests shrink)",
                                view.loops, LOOP_PAUSE_MS
                            );
                            true
                        }
                    })
                    .unwrap_or(true);
                if at_end {
                    // Pause on the finalized doc, then restart so there is ALWAYS
                    // a live stream to scroll/select against.
                    cx.background_executor()
                        .timer(Duration::from_millis(LOOP_PAUSE_MS))
                        .await;
                    if this.update(cx, |view, cx| {
                        view.idx = 0;
                        cx.notify();
                    }).is_err() {
                        break;
                    }
                }
            }
        })
        .detach();

        Self {
            accums,
            idx: 0,
            probe: Probe::new(STREAM_ID),
            loops: 0,
            label: source.label(),
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

        let _ = self.label;
        div().size_full().p_4().child(md)
    }
}
