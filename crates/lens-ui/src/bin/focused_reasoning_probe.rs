//! Real-window live-reasoning stick-to-bottom probe — must run on the main thread
//! (`cargo run -p lens-ui --features probe --bin focused_reasoning_probe`).
//! `Application::new().run()`; not invokable from `#[gpui::test]` worker threads.

use std::cell::RefCell;
use std::process;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

use gpui::{
    App, Application, AsyncWindowContext, Context, ListOffset, Render, Styled, WeakEntity, Window,
    div, prelude::*, px,
};
use lens_ui::focused::reasoning::{ReasoningUiState, render_reasoning};
use lens_ui::focused::{ContentKey, RowContent};
use lens_ui::md::{
    init as md_init, markdown_probe_list_item_count, markdown_probe_logical_scroll_top,
    markdown_probe_scroll_list_to,
};

const GROWTH_STEPS: usize = 6;
const FRAMES_PER_GROWTH: usize = 12;
const SETTLE_FRAMES: usize = 10;

fn growth_text(steps: usize) -> String {
    let mut s = String::from("## Live reasoning\n\n");
    for i in 0..steps {
        s.push_str(&format!(
            "Paragraph {i}: Lorem ipsum dolor sit amet, consectetur adipiscing elit. \
             Sed do eiusmod tempor incididunt ut labore et dolore magna aliqua.\n\n"
        ));
    }
    s
}

fn md_at_bottom(offset: ListOffset, item_count: usize) -> bool {
    item_count == 0 || offset.item_ix + 1 >= item_count
}

#[derive(Default)]
struct ProbeState {
    failures: Vec<String>,
}

struct HarnessView {
    full_text: String,
    content_key: ContentKey,
    probe: Rc<RefCell<ProbeState>>,
    spawned: bool,
    exit_ok: Rc<RefCell<bool>>,
}

async fn wait_frames(wcx: &mut AsyncWindowContext, n: usize) {
    for _ in 0..n {
        let (tx, rx) = mpsc::sync_channel(1);
        wcx.on_next_frame(move |_, _| {
            let _ = tx.send(());
        });
        let _ = wcx.update(|window, _| window.refresh());
        loop {
            if rx.try_recv().is_ok() {
                break;
            }
            wcx.background_executor()
                .timer(Duration::from_millis(1))
                .await;
        }
    }
}

async fn drive_reasoning_probe(
    weak: WeakEntity<HarnessView>,
    mut wcx: AsyncWindowContext,
    md_id: String,
    probe: Rc<RefCell<ProbeState>>,
    exit_ok: Rc<RefCell<bool>>,
) {
    // Arm after the target's initial 0x0→real layout has settled (T3-1 P2 lesson).
    wait_frames(&mut wcx, SETTLE_FRAMES).await;

    let mut stick_ok = true;
    for step in 1..=GROWTH_STEPS {
        let text = growth_text(step);
        let _ = weak.update_in(&mut wcx, |view, _, cx| {
            view.full_text = text;
            cx.notify();
        });
        wait_frames(&mut wcx, FRAMES_PER_GROWTH).await;

        let (at_bottom, item_ix, count) = weak
            .update_in(&mut wcx, |_, window, cx| {
                let offset = markdown_probe_logical_scroll_top(&md_id, window, cx);
                let count = markdown_probe_list_item_count(&md_id, window, cx);
                let at_bottom = md_at_bottom(offset, count);
                (at_bottom, offset.item_ix, count)
            })
            .unwrap_or((false, 0, 0));

        if !at_bottom {
            stick_ok = false;
            probe.borrow_mut().failures.push(format!(
                "stick-to-bottom failed at step {step}: item_ix={item_ix} count={count}"
            ));
        }
    }

    let _ = weak.update_in(&mut wcx, |_, window, cx| {
        markdown_probe_scroll_list_to(
            &md_id,
            ListOffset {
                item_ix: 0,
                offset_in_item: px(0.),
            },
            window,
            cx,
        );
        cx.notify();
    });
    wait_frames(&mut wcx, SETTLE_FRAMES).await;

    let top_ix_before = weak
        .update_in(&mut wcx, |_, window, cx| {
            markdown_probe_logical_scroll_top(&md_id, window, cx).item_ix
        })
        .unwrap_or(0);

    let final_text = growth_text(GROWTH_STEPS + 1);
    let _ = weak.update_in(&mut wcx, |view, _, cx| {
        view.full_text = final_text;
        cx.notify();
    });
    wait_frames(&mut wcx, FRAMES_PER_GROWTH).await;

    let preserve_ok = weak
        .update_in(&mut wcx, |_, window, cx| {
            let offset = markdown_probe_logical_scroll_top(&md_id, window, cx);
            let count = markdown_probe_list_item_count(&md_id, window, cx);
            offset.item_ix == top_ix_before && offset.item_ix < count.saturating_sub(1)
        })
        .unwrap_or(false);

    if !preserve_ok {
        probe.borrow_mut().failures.push(format!(
            "scroll-preserve failed: expected near top (ix={top_ix_before}) after growth"
        ));
    }

    eprintln!(
        "REASONING STICK-TO-BOTTOM: {}",
        if stick_ok { "PASS" } else { "FAIL" }
    );
    eprintln!(
        "REASONING SCROLL-PRESERVE: {}",
        if preserve_ok { "PASS" } else { "FAIL" }
    );

    let ok = stick_ok && preserve_ok && probe.borrow().failures.is_empty();
    if !ok {
        eprintln!("FAILURES: {:?}", probe.borrow().failures);
    }
    *exit_ok.borrow_mut() = ok;
    process::exit(if ok { 0 } else { 1 });
}

impl Render for HarnessView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.spawned {
            self.spawned = true;
            let exit_ok = Rc::clone(&self.exit_ok);
            let probe = Rc::clone(&self.probe);
            let md_id = self.content_key.as_element_id().as_str().to_string();
            cx.spawn_in(window, move |weak, wcx: &mut AsyncWindowContext| {
                let wcx = wcx.clone();
                async move {
                    drive_reasoning_probe(weak, wcx, md_id, probe, exit_ok).await;
                }
            })
            .detach();
        }

        let content = RowContent::Reasoning {
            summary: String::new(),
            full: self.full_text.clone(),
            encrypted: false,
            duration_secs: None,
            content_key: self.content_key.clone(),
            live: true,
        };

        div()
            .size_full()
            .p_4()
            .child(render_reasoning(
                &content,
                ReasoningUiState::Collapsed { duration_secs: None },
                None,
                window,
                cx,
            ))
    }
}

fn main() {
    let exit_ok = Rc::new(RefCell::new(false));
    let exit_for_run = Rc::clone(&exit_ok);
    let probe = Rc::new(RefCell::new(ProbeState::default()));
    let content_key = ContentKey::from_label("reasoning-probe");

    Application::new().run(move |cx: &mut App| {
        gpui_component::init(cx);
        md_init(cx);
        lens_ui::theme::install_at_startup(cx);

        cx.open_window(gpui::WindowOptions::default(), move |_window, cx| {
            cx.new(|_| HarnessView {
                full_text: growth_text(1),
                content_key: content_key.clone(),
                probe: Rc::clone(&probe),
                spawned: false,
                exit_ok: Rc::clone(&exit_for_run),
            })
        })
        .expect("open window");
        cx.activate(true);
    });

    process::exit(if *exit_ok.borrow() { 0 } else { 1 });
}
