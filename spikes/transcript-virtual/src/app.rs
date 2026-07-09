//! gpui window wiring, keybindings, live probe readout.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use gpui::{
    App, Context, FocusHandle, Focusable, KeyBinding, ListScrollEvent, Styled, Window, actions,
    div, list, prelude::*, px,
};
use gpui_component::{ActiveTheme as _, v_virtual_list};

use crate::backend::{Backend, BackendChoice};
use crate::probe::{AnchorSnapshot, FollowMode, ProbeHarness, ProbeVerdict};
use crate::rowsource::RowId;

actions!(
    harness,
    [
        ProbeWindowing,
        ProbeVariableHeights,
        ProbeAnchor1a,
        ProbeAnchor1bSetup,
        ProbeAnchor1bMutate,
        ProbeIdentityScrollOff,
        ProbeIdentityScrollBack,
        ProbeUxAppend,
        ProbeUxEvaluate,
        Reload200,
        Reload2000,
    ]
);

pub struct HarnessView {
    focus_handle: FocusHandle,
    pub backend: Backend,
    probes: ProbeHarness,
    render_calls: Rc<RefCell<u32>>,
    frame_started: Option<Instant>,
    checked_initial_bottom: bool,
    arm_windowing_sample: u8,
    arm_variable_heights_sample: u8,
    identity_arm_record: bool,
    identity_arm_assert: bool,
    identity_scrolled_off: bool,
    last_scroll_y: Option<gpui::Pixels>,
}

impl HarnessView {
    pub fn new(
        choice: BackendChoice,
        n: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();
        focus_handle.focus(window);
        let identity_ix = {
            let f = crate::fixture::Fixture::synthetic(n);
            f.rows
                .iter()
                .position(|r| r.kind == crate::fixture::RowKind::CodeBlock)
                .unwrap_or(n / 4)
        };
        Self {
            focus_handle,
            probes: ProbeHarness::new(identity_ix),
            backend: Backend::new(choice, n, cx),
            render_calls: Rc::new(RefCell::new(0)),
            frame_started: None,
            checked_initial_bottom: false,
            arm_windowing_sample: 0,
            arm_variable_heights_sample: 0,
            identity_arm_record: false,
            identity_arm_assert: false,
            identity_scrolled_off: false,
            last_scroll_y: None,
        }
    }

    fn on_probe_windowing(&mut self, _: &ProbeWindowing, _: &mut Window, cx: &mut Context<Self>) {
        self.arm_windowing_sample = 1;
        self.probes.frame_timer.reset_samples();
        cx.notify();
    }

    fn on_probe_variable_heights(
        &mut self,
        _: &ProbeVariableHeights,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.arm_variable_heights_sample = 1;
        cx.notify();
    }

    fn on_probe_anchor_1a(
        &mut self,
        _: &ProbeAnchor1a,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.backend.is_at_bottom() {
            self.probes.anchor_1a = ProbeVerdict::Fail(
                "not pinned to bottom — scroll to bottom before pressing 3".into(),
            );
            cx.notify();
            return;
        }
        self.backend.append_to_last(" …stream", window, cx);
        self.probes.note_append_while_paused();
        match self.backend.choice() {
            BackendChoice::A => {
                if let crate::backend::Backend::A(ref b) = self.backend {
                    let count = b.item_count();
                    let after = b.list_state.logical_scroll_top();
                    self.probes.assert_anchor_1a(count, after);
                }
            }
            BackendChoice::B => {
                self.probes
                    .assert_anchor_1a_bottom(self.backend.is_at_bottom());
            }
        }
        cx.notify();
    }

    fn on_anchor_1b_setup(
        &mut self,
        _: &ProbeAnchor1bSetup,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let k = self.backend.scroll_anchor_ix();
        let o = px(16.);
        self.backend.scroll_to_logical(k, o);
        self.backend.set_follow_paused_b();
        let anchor = self.backend.logical_anchor();
        self.probes.anchor_before = Some(anchor);
        let derived = if self.backend.anchor_1b_is_derived() {
            " [derived anchor — PARTIAL qualifier if table≠layout]"
        } else {
            ""
        };
        self.probes.anchor_1b_detail = format!(
            "setup: scrolled to k={k} o={o:?}; recorded ({}, {:?}){derived}",
            anchor.top_item_index, anchor.sub_offset
        );
        cx.notify();
    }

    fn on_anchor_1b_mutate(
        &mut self,
        _: &ProbeAnchor1bMutate,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = RowId(self.backend.fixture_mutable_id());
        self.backend.mutate_height(id, px(80.), window, cx);
        let anchor = self.backend.logical_anchor();
        self.probes.anchor_after = Some(anchor);
        self.probes.assert_anchor_1b();
        if self.backend.anchor_1b_is_derived() {
            self.probes.anchor_1b_detail.push_str(
                " | B: derived from offset.y + harness height table (no scroll compensation)",
            );
        }
        cx.notify();
    }

    fn on_identity_scroll_off(
        &mut self,
        _: &ProbeIdentityScrollOff,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ix = self.probes.identity_target_ix;
        self.backend.scroll_to_reveal(ix);
        self.identity_arm_record = true;
        self.probes.identity_detail =
            format!("revealing markdown row ix={ix} — recording next frame, then scrolling off");
        cx.notify();
    }

    fn on_identity_scroll_back(
        &mut self,
        _: &ProbeIdentityScrollBack,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.identity_scrolled_off {
            self.probes.identity = ProbeVerdict::Fail("press 6 first to scroll off".into());
            cx.notify();
            return;
        }
        let ix = self.probes.identity_target_ix;
        self.backend.scroll_to_reveal(ix);
        self.identity_arm_assert = true;
        self.probes.identity_detail = format!("scrolling back to ix={ix} — asserting next frame");
        cx.notify();
    }

    fn on_ux_append(&mut self, _: &ProbeUxAppend, window: &mut Window, cx: &mut Context<Self>) {
        self.backend.append_to_last(" [new]", window, cx);
        self.probes.note_append_while_paused();
        cx.notify();
    }

    fn on_ux_evaluate(&mut self, _: &ProbeUxEvaluate, _: &mut Window, cx: &mut Context<Self>) {
        self.probes.evaluate_ux();
        cx.notify();
    }

    fn on_reload(&mut self, n: usize, cx: &mut Context<Self>) {
        self.backend.reload(n, cx);
        self.checked_initial_bottom = false;
        self.probes = ProbeHarness::new(self.probes.identity_target_ix);
        self.arm_windowing_sample = 0;
        self.arm_variable_heights_sample = 0;
        cx.notify();
    }

    fn on_reload_200(&mut self, _: &Reload200, _: &mut Window, cx: &mut Context<Self>) {
        self.on_reload(200, cx);
    }

    fn on_reload_2000(&mut self, _: &Reload2000, _: &mut Window, cx: &mut Context<Self>) {
        self.on_reload(2000, cx);
    }

    fn poll_scroll_b(&mut self) {
        if let Backend::B(b) = &mut self.backend {
            let y = b.scroll_handle.offset().y;
            if let Some(prev) = self.last_scroll_y {
                if y > prev + px(1.) {
                    self.probes.set_follow_mode(FollowMode::Paused);
                    b.follow_bottom = false;
                }
            }
            self.last_scroll_y = Some(y);
            if b.is_at_bottom() {
                self.probes.set_follow_mode(FollowMode::Following);
                b.follow_bottom = true;
            }
        }
    }

    fn advance_probe_arms(&mut self, cx: &App) {
        let n = self.backend.item_count();
        let renders = *self.render_calls.borrow();
        self.probes.render_counter.this_frame = renders;

        if self.arm_windowing_sample == 2 {
            self.probes.assert_windowing(n);
            self.arm_windowing_sample = 0;
        } else if self.arm_windowing_sample == 1 {
            self.arm_windowing_sample = 2;
        }

        if self.arm_variable_heights_sample == 2 {
            self.probes.assert_variable_heights(n);
            self.arm_variable_heights_sample = 0;
        } else if self.arm_variable_heights_sample == 1 {
            self.arm_variable_heights_sample = 2;
        }

        let ix = self.probes.identity_target_ix;
        if self.identity_arm_record && renders > 0 {
            if let Some(eid) = self.backend.identity_target_entity_id(ix, cx) {
                self.probes.identity_entity_before = Some(eid);
                self.probes.identity_markdown_inits_before =
                    self.backend.identity_markdown_inits(ix, cx);
            }
            self.backend.scroll_to_top();
            self.identity_scrolled_off = true;
            self.identity_arm_record = false;
            self.probes.identity_detail =
                format!("recorded row ix={ix} — press b to scroll back + assert");
        }
        if self.identity_arm_assert && renders > 0 {
            let before = self.probes.identity_entity_before;
            let md_before = self.probes.identity_markdown_inits_before;
            if let (Some(before), Some(after)) =
                (before, self.backend.identity_target_entity_id(ix, cx))
            {
                let md_after = self.backend.identity_markdown_inits(ix, cx);
                self.probes
                    .assert_identity(before, after, md_before, md_after);
            }
            self.identity_arm_assert = false;
            self.identity_scrolled_off = false;
        }
    }

    fn finish_pending_assertions(&mut self, frame_elapsed: Option<std::time::Duration>) {
        if let Some(d) = frame_elapsed {
            self.probes.frame_timer.record_sample(d);
        }
        self.backend.on_frame_start();
        self.poll_scroll_b();

        if frame_elapsed.is_some() && !self.checked_initial_bottom {
            let n = self.backend.item_count();
            match self.backend.choice() {
                BackendChoice::A => {
                    if let crate::backend::Backend::A(ref b) = self.backend {
                        let offset = b.list_state.logical_scroll_top();
                        self.probes.assert_jump_to_bottom(n, offset);
                    }
                }
                BackendChoice::B => {
                    self.probes
                        .assert_jump_to_bottom_bottom(self.backend.is_at_bottom(), n);
                }
            }
            self.checked_initial_bottom = true;
        }
        self.backend.bind_scroll_follow(&mut self.probes);
    }

    fn render_readout(&self) -> impl IntoElement {
        let p = &self.probes;
        let line = |name: &str, v: &ProbeVerdict, detail: &str| {
            let color = match v {
                ProbeVerdict::Pass => gpui::rgb(0x22c55e),
                ProbeVerdict::Fail(_) => gpui::rgb(0xef4444),
                ProbeVerdict::Pending => gpui::rgb(0xeab308),
            };
            div()
                .text_sm()
                .text_color(color)
                .child(format!("{name}: {} — {detail}", v.label()))
        };
        div()
            .flex()
            .flex_col()
            .gap_1()
            .p_2()
            .bg(gpui::rgb(0x111111))
            .text_color(gpui::rgb(0xeeeeee))
            .child(div().text_sm().child(format!(
                "{} | N={} | renders/frame={} | frame={}µs | follow={:?} | N-new={}",
                self.backend.label(),
                self.backend.item_count(),
                p.render_counter.this_frame,
                p.frame_timer.last_frame.as_micros(),
                p.follow_mode,
                p.new_while_paused
            )))
            .child(line("1 windowing", &p.windowing, &p.windowing_detail))
            .child(line(
                "2 var-heights",
                &p.variable_heights,
                &p.variable_heights_detail,
            ))
            .child(line("3 anchor-1a", &p.anchor_1a, &p.anchor_1a_detail))
            .child(line("4 anchor-1b", &p.anchor_1b, &p.anchor_1b_detail))
            .child(line(
                "5 jump-bottom",
                &p.jump_to_bottom,
                &p.jump_to_bottom_detail,
            ))
            .child(line("6 identity", &p.identity, &p.identity_detail))
            .child(line("7 ux-demo", &p.ux_demo, &p.ux_demo_detail))
            .child(div().text_xs().text_color(gpui::rgb(0x888888)).child(
                "Keys: 1=windowing 2=var-heights 3=anchor-1a 4=1b-setup 5=1b-mutate \
                     6=identity-off b=identity-back 7=ux-append 8=ux-eval \
                     | 9=N200 0=N2000",
            ))
    }
}

impl Render for HarnessView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = Instant::now();
        let frame_elapsed = self
            .frame_started
            .replace(now)
            .map(|start| now.duration_since(start));

        self.advance_probe_arms(cx);
        self.finish_pending_assertions(frame_elapsed);

        *self.render_calls.borrow_mut() = 0;

        let entity = cx.entity().clone();
        let render_calls = self.render_calls.clone();

        let list_el = match &self.backend {
            Backend::A(b) => {
                let list_state = b.list_state.clone();
                let probes_entity = entity.clone();
                list_state.set_scroll_handler(move |event: &ListScrollEvent, _, app| {
                    probes_entity.update(app, |view, _| {
                        let count = view.backend.item_count();
                        let at_bottom = event.visible_range.end >= count.saturating_sub(1)
                            && !event.is_scrolled;
                        if at_bottom {
                            view.probes.set_follow_mode(FollowMode::Following);
                        } else if event.is_scrolled {
                            view.probes.set_follow_mode(FollowMode::Paused);
                        }
                    });
                });
                list(list_state, move |ix, window, app| {
                    entity
                        .update(app, |view, cx| {
                            if let Backend::A(ref backend) = view.backend {
                                backend.render_list_item(
                                    ix,
                                    window,
                                    cx,
                                    &mut render_calls.borrow_mut(),
                                )
                            } else {
                                div().into_any_element()
                            }
                        })
                        .into_any_element()
                })
                .size_full()
                .into_any_element()
            }
            Backend::B(b) => {
                let sizes = b.item_sizes.clone();
                let handle = b.scroll_handle.clone();
                v_virtual_list(
                    cx.entity(),
                    "transcript-virtual-b",
                    sizes,
                    move |this, range, window, cx| {
                        range
                            .map(|ix| {
                                if let Backend::B(ref backend) = this.backend {
                                    backend
                                        .render_list_item(
                                            ix,
                                            window,
                                            cx,
                                            &mut render_calls.borrow_mut(),
                                        )
                                        .into_any_element()
                                } else {
                                    div().into_any_element()
                                }
                            })
                            .collect()
                    },
                )
                .track_scroll(&handle)
                .size_full()
                .into_any_element()
            }
        };

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(self.render_readout())
            .child(div().flex_grow().overflow_hidden().child(list_el))
            .key_context("Harness")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_probe_windowing))
            .on_action(cx.listener(Self::on_probe_variable_heights))
            .on_action(cx.listener(Self::on_probe_anchor_1a))
            .on_action(cx.listener(Self::on_anchor_1b_setup))
            .on_action(cx.listener(Self::on_anchor_1b_mutate))
            .on_action(cx.listener(Self::on_identity_scroll_off))
            .on_action(cx.listener(Self::on_identity_scroll_back))
            .on_action(cx.listener(Self::on_ux_append))
            .on_action(cx.listener(Self::on_ux_evaluate))
            .on_action(cx.listener(Self::on_reload_200))
            .on_action(cx.listener(Self::on_reload_2000))
    }
}

impl Focusable for HarnessView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

pub fn register_keybindings(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("1", ProbeWindowing, Some("Harness")),
        KeyBinding::new("2", ProbeVariableHeights, Some("Harness")),
        KeyBinding::new("3", ProbeAnchor1a, Some("Harness")),
        KeyBinding::new("4", ProbeAnchor1bSetup, Some("Harness")),
        KeyBinding::new("5", ProbeAnchor1bMutate, Some("Harness")),
        KeyBinding::new("6", ProbeIdentityScrollOff, Some("Harness")),
        KeyBinding::new("b", ProbeIdentityScrollBack, Some("Harness")),
        KeyBinding::new("7", ProbeUxAppend, Some("Harness")),
        KeyBinding::new("8", ProbeUxEvaluate, Some("Harness")),
        KeyBinding::new("9", Reload200, Some("Harness")),
        KeyBinding::new("0", Reload2000, Some("Harness")),
    ]);
}
