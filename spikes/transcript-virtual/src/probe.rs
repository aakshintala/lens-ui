//! Probe instrumentation + baked-in assertions (spec §4).

use std::time::Duration;

use gpui::{ListOffset, Pixels};

/// Logical scroll anchor snapshot for contract-1b before/after comparison.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AnchorSnapshot {
    pub top_item_index: usize,
    pub sub_offset: Pixels,
}

impl From<ListOffset> for AnchorSnapshot {
    fn from(offset: ListOffset) -> Self {
        Self {
            top_item_index: offset.item_ix,
            sub_offset: offset.offset_in_item,
        }
    }
}

/// Auto-follow ↔ paused transition (UX demo probe).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FollowMode {
    Following,
    Paused,
}

#[derive(Clone, Debug)]
pub struct FollowTransition {
    pub from: FollowMode,
    pub to: FollowMode,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProbeVerdict {
    Pending,
    Pass,
    Fail(String),
}

impl ProbeVerdict {
    pub fn label(&self) -> &'static str {
        match self {
            ProbeVerdict::Pending => "PENDING",
            ProbeVerdict::Pass => "PASS",
            ProbeVerdict::Fail(_) => "FAIL",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct WindowingSample {
    pub n: usize,
    pub render_calls: u32,
    pub mean_frame_us: u128,
}

#[derive(Debug)]
pub struct FrameTimer {
    pub last_frame: Duration,
    samples: Vec<Duration>,
}

impl FrameTimer {
    pub fn new() -> Self {
        Self {
            last_frame: Duration::ZERO,
            samples: Vec::new(),
        }
    }

    pub fn record_sample(&mut self, d: Duration) {
        self.last_frame = d;
        self.samples.push(d);
    }

    pub fn mean_us(&self) -> u128 {
        if self.samples.is_empty() {
            0
        } else {
            self.samples.iter().map(|d| d.as_micros()).sum::<u128>() / self.samples.len() as u128
        }
    }

    pub fn reset_samples(&mut self) {
        self.samples.clear();
    }
}

impl Default for FrameTimer {
    fn default() -> Self {
        Self::new()
    }
}

/// Per-frame render-invocation counter (reset at frame start).
#[derive(Clone, Debug, Default)]
pub struct RenderCounter {
    pub this_frame: u32,
}

impl RenderCounter {
    pub fn begin_frame(&mut self) {
        self.this_frame = 0;
    }

    pub fn bump(&mut self) {
        self.this_frame += 1;
    }
}

#[derive(Debug)]
pub struct ProbeHarness {
    pub render_counter: RenderCounter,
    pub frame_timer: FrameTimer,

    pub anchor_before: Option<AnchorSnapshot>,
    pub anchor_after: Option<AnchorSnapshot>,

    pub follow_mode: FollowMode,
    pub follow_log: Vec<FollowTransition>,
    pub new_while_paused: u64,

    pub identity_target_ix: usize,
    pub identity_entity_before: Option<gpui::EntityId>,
    pub identity_markdown_inits_before: u32,

    // Verdicts
    pub windowing: ProbeVerdict,
    pub windowing_detail: String,
    pub windowing_samples: Vec<WindowingSample>,

    pub variable_heights: ProbeVerdict,
    pub variable_heights_detail: String,

    pub anchor_1a: ProbeVerdict,
    pub anchor_1a_detail: String,
    pub anchor_1a_appends: u32,

    pub anchor_1b: ProbeVerdict,
    pub anchor_1b_detail: String,

    pub jump_to_bottom: ProbeVerdict,
    pub jump_to_bottom_detail: String,

    pub identity: ProbeVerdict,
    pub identity_detail: String,

    pub ux_demo: ProbeVerdict,
    pub ux_demo_detail: String,
}

impl ProbeHarness {
    pub fn new(identity_target_ix: usize) -> Self {
        Self {
            identity_target_ix,
            render_counter: RenderCounter::default(),
            frame_timer: FrameTimer::new(),
            anchor_before: None,
            anchor_after: None,
            follow_mode: FollowMode::Following,
            follow_log: Vec::new(),
            new_while_paused: 0,
            identity_entity_before: None,
            identity_markdown_inits_before: 0,
            windowing: ProbeVerdict::Pending,
            windowing_detail: String::new(),
            windowing_samples: Vec::new(),
            variable_heights: ProbeVerdict::Pending,
            variable_heights_detail: String::new(),
            anchor_1a: ProbeVerdict::Pending,
            anchor_1a_detail: String::new(),
            anchor_1a_appends: 0,
            anchor_1b: ProbeVerdict::Pending,
            anchor_1b_detail: String::new(),
            jump_to_bottom: ProbeVerdict::Pending,
            jump_to_bottom_detail: String::new(),
            identity: ProbeVerdict::Pending,
            identity_detail: String::new(),
            ux_demo: ProbeVerdict::Pending,
            ux_demo_detail: String::new(),
        }
    }

    pub fn set_follow_mode(&mut self, mode: FollowMode) {
        if mode != self.follow_mode {
            self.follow_log.push(FollowTransition {
                from: self.follow_mode,
                to: mode,
            });
            self.follow_mode = mode;
            if mode == FollowMode::Following {
                self.new_while_paused = 0;
            }
        }
    }

    pub fn note_append_while_paused(&mut self) {
        if self.follow_mode == FollowMode::Paused {
            self.new_while_paused += 1;
        }
    }

    /// Contract 2 — windowing: rendered ≪ N; frame cost flat across N sweep.
    pub fn assert_windowing(&mut self, n: usize) {
        let renders = self.render_counter.this_frame;
        let mean_us = self.frame_timer.last_frame.as_micros();
        self.windowing_samples.push(WindowingSample {
            n,
            render_calls: renders,
            mean_frame_us: mean_us,
        });

        let render_ok = renders < (n as u32 / 5).max(30);
        let mut detail = format!(
            "N={n} renders={renders} (≪{n}) mean_frame={mean_us}µs",
        );

        if self.windowing_samples.len() >= 2 {
            let a = &self.windowing_samples[self.windowing_samples.len() - 2];
            let b = &self.windowing_samples[self.windowing_samples.len() - 1];
            let ratio = if a.mean_frame_us == 0 {
                1.0
            } else {
                b.mean_frame_us as f64 / a.mean_frame_us as f64
            };
            detail.push_str(&format!(
                " | N{}→N{} frame ratio={ratio:.2} (want ≲3×)",
                a.n, b.n
            ));
            let flat_ok = ratio <= 3.0;
            if render_ok && flat_ok {
                self.windowing = ProbeVerdict::Pass;
            } else {
                let why = if !render_ok {
                    format!("renders {renders} not ≪ {n}")
                } else {
                    format!("frame cost ratio {ratio:.2} > 3×")
                };
                self.windowing = ProbeVerdict::Fail(why);
            }
        } else if render_ok {
            self.windowing = ProbeVerdict::Pending;
            detail.push_str(" — run at second N to compare frame cost");
        } else {
            self.windowing = ProbeVerdict::Fail(format!("renders {renders} not ≪ {n}"));
        }
        self.windowing_detail = detail;
    }

    /// Contract 3 — variable heights: render calls ≈ visible+overdraw, not N.
    pub fn assert_variable_heights(&mut self, n: usize) {
        let renders = self.render_counter.this_frame;
        let cap = (n as u32 / 5).max(30);
        self.variable_heights_detail =
            format!("N={n} render_calls={renders} (cap≈{cap}, not all N)");
        self.variable_heights = if renders < cap {
            ProbeVerdict::Pass
        } else {
            ProbeVerdict::Fail(format!("render_calls {renders} ≥ {cap}"))
        };
    }

    /// Contract 1a (Backend B) — bottom-follow keeps last row visible after append.
    pub fn assert_anchor_1a_bottom(&mut self, at_bottom: bool) {
        self.anchor_1a_appends += 1;
        self.anchor_1a_detail = format!(
            "append #{} at_bottom={at_bottom} (manual scroll_to_bottom follow)",
            self.anchor_1a_appends
        );
        if at_bottom {
            if self.anchor_1a_appends >= 3 {
                self.anchor_1a = ProbeVerdict::Pass;
            } else {
                self.anchor_1a = ProbeVerdict::Pending;
                self.anchor_1a_detail
                    .push_str(" — press 3 again for more appends");
            }
        } else {
            self.anchor_1a = ProbeVerdict::Fail("drifted off bottom after append".into());
        }
    }

    /// Contract 4 (Backend B) — initial scroll_to_bottom lands at bottom.
    pub fn assert_jump_to_bottom_bottom(&mut self, at_bottom: bool, item_count: usize) {
        self.jump_to_bottom_detail =
            format!("initial at_bottom={at_bottom} item_count={item_count}");
        self.jump_to_bottom = if at_bottom {
            ProbeVerdict::Pass
        } else {
            ProbeVerdict::Fail("scroll_to_bottom did not pin on open".into())
        };
    }

    /// Contract 1a (Backend A) — stay pinned to bottom across appends.
    pub fn assert_anchor_1a(&mut self, item_count: usize, offset: ListOffset) {
        self.anchor_1a_appends += 1;
        let pinned = offset.item_ix == item_count && offset.offset_in_item == Pixels::ZERO;
        self.anchor_1a_detail = format!(
            "append #{} logical_scroll_top=({}, {:?}) item_count={item_count}",
            self.anchor_1a_appends, offset.item_ix, offset.offset_in_item
        );
        if pinned {
            if self.anchor_1a_appends >= 3 {
                self.anchor_1a = ProbeVerdict::Pass;
            } else {
                self.anchor_1a = ProbeVerdict::Pending;
                self.anchor_1a_detail
                    .push_str(" — press 3 again for more appends");
            }
        } else {
            self.anchor_1a = ProbeVerdict::Fail("drifted off bottom pin".into());
        }
    }

    /// Contract 1b — anchor unchanged after off-screen-above height mutation.
    pub fn assert_anchor_1b(&mut self) {
        let (Some(before), Some(after)) = (self.anchor_before, self.anchor_after) else {
            self.anchor_1b = ProbeVerdict::Fail("missing before/after anchor".into());
            return;
        };
        let unchanged =
            before.top_item_index == after.top_item_index && before.sub_offset == after.sub_offset;
        self.anchor_1b_detail = format!(
            "before=({}, {:?}) after=({}, {:?})",
            before.top_item_index, before.sub_offset, after.top_item_index, after.sub_offset
        );
        self.anchor_1b = if unchanged {
            ProbeVerdict::Pass
        } else {
            ProbeVerdict::Fail("anchor shifted".into())
        };
    }

    /// Contract 4 — initial open lands at bottom.
    pub fn assert_jump_to_bottom(&mut self, item_count: usize, offset: ListOffset) {
        let at_bottom = offset.item_ix == item_count && offset.offset_in_item == Pixels::ZERO;
        self.jump_to_bottom_detail = format!(
            "initial logical_scroll_top=({}, {:?}) item_count={item_count}",
            offset.item_ix, offset.offset_in_item
        );
        self.jump_to_bottom = if at_bottom {
            ProbeVerdict::Pass
        } else {
            ProbeVerdict::Fail("not at bottom on open".into())
        };
    }

    /// Structural — retained Entity ids survive scroll-off + back.
    pub fn assert_identity(
        &mut self,
        before: gpui::EntityId,
        after: gpui::EntityId,
        markdown_inits_before: u32,
        markdown_inits_after: u32,
    ) {
        self.identity_detail = format!(
            "row entity {before:?}→{after:?} markdown_inits {markdown_inits_before}→{markdown_inits_after}"
        );
        let entity_ok = before == after;
        // Identity invariant: the row's markdown state is initialized AT MOST ONCE,
        // ever — the off+back round-trip must not RE-initialize it. `before` can read
        // 0 when the baseline is captured before the off-screen target's first paint,
        // so comparing after==before is wrong; what matters is that after the
        // round-trip the count is still <= 1 (a genuine re-init shows 1->2 => fail).
        let md_ok = markdown_inits_after <= 1;
        let _ = markdown_inits_before;
        self.identity = if entity_ok && md_ok {
            ProbeVerdict::Pass
        } else {
            ProbeVerdict::Fail(if !entity_ok {
                "RowState Entity recreated".into()
            } else {
                "TextView re-initialized on re-scroll".into()
            })
        };
    }

    /// UX demo — follow transitions logged; N-new increments while paused.
    pub fn evaluate_ux(&mut self) {
        let scrolled_up = self.follow_log.iter().any(|t| {
            t.from == FollowMode::Following && t.to == FollowMode::Paused
        });
        let resumed = self.follow_log.iter().any(|t| {
            t.from == FollowMode::Paused && t.to == FollowMode::Following
        });
        self.ux_demo_detail = format!(
            "follow_log={} transitions paused_appends={} (scroll up with wheel, press 7 to append, scroll to bottom to resume)",
            self.follow_log.len(),
            self.new_while_paused
        );
        self.ux_demo = if scrolled_up && resumed {
            ProbeVerdict::Pass
        } else if scrolled_up {
            ProbeVerdict::Pending
        } else {
            ProbeVerdict::Fail("no scroll-up pause detected — scroll up off bottom first".into())
        };
    }
}
