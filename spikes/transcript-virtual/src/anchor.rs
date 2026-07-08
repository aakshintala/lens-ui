//! Derived logical anchor for Backend B (pixel offset + harness height table).

use gpui::Pixels;

use crate::probe::AnchorSnapshot;

/// Padding applied inside `v_virtual_list` content bounds (harness uses none).
pub const VIRTUAL_LIST_PADDING_TOP: Pixels = gpui::px(0.);

/// Derive `(top_item_index, sub_offset)` from scroll pixel offset + per-row heights.
///
/// Mirrors gpui-component `v_virtual_list` prepaint walk
/// (`virtual_list.rs`, vertical branch ~656–664): the first visible row is the
/// first index whose cumulative height exceeds `-(scroll_y + padding_top)`.
pub fn derive_anchor(scroll_y: Pixels, heights: &[Pixels]) -> AnchorSnapshot {
    let threshold = (-scroll_y - VIRTUAL_LIST_PADDING_TOP).max(gpui::px(0.));
    let mut cumulative = gpui::px(0.);
    for (i, &h) in heights.iter().enumerate() {
        let next = cumulative + h;
        if next > threshold {
            return AnchorSnapshot {
                top_item_index: i,
                sub_offset: threshold - cumulative,
            };
        }
        cumulative = next;
    }
    AnchorSnapshot {
        top_item_index: heights.len(),
        sub_offset: gpui::px(0.),
    }
}

/// Scroll offset that places logical anchor `(k, o)` at the viewport top.
pub fn scroll_y_for_anchor(k: usize, o: Pixels, heights: &[Pixels]) -> Pixels {
    let mut content_top = gpui::px(0.);
    for (i, &h) in heights.iter().enumerate() {
        if i == k {
            content_top += o;
            break;
        }
        content_top += h;
    }
    -content_top - VIRTUAL_LIST_PADDING_TOP
}
