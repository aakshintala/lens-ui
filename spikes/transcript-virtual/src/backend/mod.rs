//! Backend A / B selector behind one harness API.

use gpui::{App, EntityId, ListOffset, Pixels, Window, px};

use crate::backend_a::BackendA;
use crate::backend_b::BackendB;
use crate::probe::AnchorSnapshot;
use crate::probe::ProbeHarness;
use crate::rowsource::{RowId, RowSource};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackendChoice {
    A,
    B,
}

impl BackendChoice {
    pub fn parse() -> Self {
        if let Ok(env) = std::env::var("TV_BACKEND")
            && env.eq_ignore_ascii_case("b")
        {
            return Self::B;
        }
        for arg in std::env::args().skip(1) {
            if arg.eq_ignore_ascii_case("--backend=b") {
                return Self::B;
            }
            if let Some(v) = arg.strip_prefix("--backend=")
                && v.eq_ignore_ascii_case("b")
            {
                return Self::B;
            }
        }
        Self::A
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::A => "A (gpui list)",
            Self::B => "B (gpui-component v_virtual_list)",
        }
    }
}

pub enum Backend {
    A(BackendA),
    B(BackendB),
}

impl Backend {
    pub fn new(choice: BackendChoice, n: usize, handoff: bool, cx: &mut App) -> Self {
        if handoff {
            Self::A(BackendA::new_handoff(n, cx))
        } else {
            match choice {
                BackendChoice::A => Self::A(BackendA::new(n, cx)),
                BackendChoice::B => Self::B(BackendB::new(n, cx)),
            }
        }
    }

    pub fn choice(&self) -> BackendChoice {
        match self {
            Self::A(_) => BackendChoice::A,
            Self::B(_) => BackendChoice::B,
        }
    }

    pub fn label(&self) -> &'static str {
        self.choice().label()
    }

    pub fn item_count(&self) -> usize {
        match self {
            Self::A(b) => b.item_count(),
            Self::B(b) => b.item_count(),
        }
    }

    pub fn fixture_mutable_id(&self) -> u64 {
        match self {
            Self::A(b) => b.fixture.mutable_offscreen_id,
            Self::B(b) => b.fixture.mutable_offscreen_id,
        }
    }

    pub fn scroll_anchor_ix(&self) -> usize {
        match self {
            Self::A(b) => b.scroll_anchor_ix(),
            Self::B(b) => b.scroll_anchor_ix(),
        }
    }

    pub fn logical_anchor(&self) -> AnchorSnapshot {
        match self {
            Self::A(b) => AnchorSnapshot::from(b.list_state.logical_scroll_top()),
            Self::B(b) => b.derived_anchor(),
        }
    }

    pub fn is_at_bottom(&self) -> bool {
        match self {
            Self::A(b) => {
                let count = b.item_count();
                let o = b.list_state.logical_scroll_top();
                o.item_ix >= count && o.offset_in_item == Pixels::ZERO
            }
            Self::B(b) => b.is_at_bottom(),
        }
    }

    pub fn scroll_to_bottom(&mut self) {
        match self {
            Self::A(b) => {
                let count = b.item_count();
                b.list_state.scroll_to(ListOffset {
                    item_ix: count,
                    offset_in_item: px(0.),
                });
            }
            Self::B(b) => b.scroll_to_bottom(),
        }
    }

    pub fn scroll_to_logical(&mut self, k: usize, o: Pixels) {
        match self {
            Self::A(b) => b.list_state.scroll_to(ListOffset {
                item_ix: k,
                offset_in_item: o,
            }),
            Self::B(b) => b.scroll_to_logical(k, o),
        }
    }

    pub fn scroll_to_top(&mut self) {
        match self {
            Self::A(b) => b.list_state.scroll_to(ListOffset {
                item_ix: 0,
                offset_in_item: px(0.),
            }),
            Self::B(b) => b.scroll_to_top(),
        }
    }

    pub fn scroll_to_reveal(&mut self, ix: usize) {
        match self {
            Self::A(b) => b.list_state.scroll_to_reveal_item(ix),
            Self::B(b) => b.scroll_to_reveal(ix),
        }
    }

    pub fn append_to_last(&mut self, chunk: &str, window: &mut Window, cx: &mut App) {
        match self {
            Self::A(b) => b.append_to_last(chunk, window, cx),
            Self::B(b) => b.append_to_last(chunk, window, cx),
        }
    }

    pub fn mutate_height(&mut self, id: RowId, delta: Pixels, window: &mut Window, cx: &mut App) {
        match self {
            Self::A(b) => b.mutate_height(id, delta, window, cx),
            Self::B(b) => b.mutate_height(id, delta, window, cx),
        }
    }

    pub fn reload(&mut self, n: usize, cx: &mut App) {
        match self {
            Self::A(b) => b.reload(n, cx),
            Self::B(b) => b.reload(n, cx),
        }
    }

    pub fn identity_target_entity_id(&self, ix: usize, cx: &App) -> Option<EntityId> {
        match self {
            Self::A(b) => b.identity_target_entity_id(ix, cx),
            Self::B(b) => b.identity_target_entity_id(ix, cx),
        }
    }

    pub fn identity_markdown_inits(&self, ix: usize, cx: &App) -> u32 {
        match self {
            Self::A(b) => b.identity_markdown_inits(ix, cx),
            Self::B(b) => b.identity_markdown_inits(ix, cx),
        }
    }

    pub fn bind_scroll_follow(&self, probes: &mut ProbeHarness) {
        if self.is_at_bottom() {
            probes.set_follow_mode(crate::probe::FollowMode::Following);
        } else {
            let anchor = self.logical_anchor();
            let count = self.item_count();
            if anchor.top_item_index < count.saturating_sub(3) {
                probes.set_follow_mode(crate::probe::FollowMode::Paused);
            }
        }
    }

    pub fn on_frame_start(&mut self) {
        if let Self::B(b) = self
            && b.pending_scroll_bottom
        {
            b.scroll_to_bottom();
            b.pending_scroll_bottom = false;
        }
    }

    pub fn anchor_1b_is_derived(&self) -> bool {
        matches!(self, Self::B(_))
    }

    pub fn set_follow_paused_b(&mut self) {
        if let Self::B(b) = self {
            b.follow_bottom = false;
        }
    }
}
