//! Shared row rendering for both backends.

use gpui::{div, prelude::*, App, Pixels, SharedString, Window, px};
use gpui_component::text::TextView;

use crate::fixture::RowKind;
use crate::rowsource::RowState;

pub fn estimated_height(state: &RowState) -> Pixels {
    if let Some(h) = state.measured_height {
        return h;
    }
    let pad = px(4.) + state.height_delta;
    match state.kind {
        RowKind::ImagePlaceholder => px(120.) + state.height_delta,
        RowKind::CodeBlock => px(140.) + pad,
        RowKind::ToolSpan => px(72.) + pad,
        RowKind::OneLiner => px(28.) + pad,
    }
}

pub fn render_row(state: &mut RowState, window: &mut Window, cx: &mut App) -> gpui::AnyElement {
    if state.use_markdown && !state.markdown_initialized {
        state.markdown_initialized = true;
        state.markdown_init_count += 1;
    }
    let pad = px(4.) + state.height_delta;
    match state.kind {
        RowKind::CodeBlock => div()
            .w_full()
            .pb(pad)
            .child(
                TextView::markdown(
                    SharedString::from(format!("md-{}", state.id.0)),
                    state.text.clone(),
                    window,
                    cx,
                )
                .selectable(true)
                .scrollable(false),
            )
            .into_any_element(),
        RowKind::ImagePlaceholder => div()
            .w_full()
            .h(px(120.) + state.height_delta)
            .p_2()
            .child(state.text.clone())
            .into_any_element(),
        RowKind::ToolSpan => div()
            .w_full()
            .pb(pad)
            .p_2()
            .child(format!("{}\n(extra tool output line)\n(more output)", state.text))
            .into_any_element(),
        RowKind::OneLiner => div()
            .w_full()
            .pb(pad)
            .px_2()
            .child(state.text.clone())
            .into_any_element(),
    }
}
