//! Pure hit-testing helpers for hyperlink click gestures.

use gpui::{Pixels, Point};

use crate::Frame;
use crate::engine::presentation::{plain_url_covering_cell, validate_open_url};
use crate::render::metrics::CellMetrics;

pub fn pixel_to_cell(
    origin: Point<Pixels>,
    metrics: &CellMetrics,
    position: Point<Pixels>,
    cols: u16,
    rows: u16,
) -> Option<(u16, u16)> {
    let rel_x = position.x - origin.x;
    let rel_y = position.y - origin.y;
    if rel_x < Pixels::ZERO || rel_y < Pixels::ZERO {
        return None;
    }
    let col = (rel_x / metrics.cell_w).floor() as u16;
    let row = (rel_y / metrics.cell_h).floor() as u16;
    if col >= cols || row >= rows {
        return None;
    }
    Some((col, row))
}

fn char_index_at_col(frame_row: &crate::engine::frame::FrameRow, target_col: u16) -> Option<usize> {
    let mut sorted: Vec<_> = frame_row.cells.iter().collect();
    sorted.sort_by_key(|c| c.col);
    let mut idx = 0usize;
    for cell in sorted {
        if cell.col == target_col {
            return Some(idx);
        }
        idx += cell.grapheme.chars().count();
    }
    None
}

fn row_text_from_cells(frame_row: &crate::engine::frame::FrameRow) -> String {
    let mut sorted: Vec<_> = frame_row.cells.iter().collect();
    sorted.sort_by_key(|c| c.col);
    sorted.iter().flat_map(|c| c.grapheme.chars()).collect()
}

pub fn uri_for_gesture(frame: &Frame, col: u16, row: u16) -> Option<String> {
    let row_idx = row as usize;
    if row_idx >= frame.grid.len() {
        return None;
    }
    let frame_row = &frame.grid[row_idx];
    let cell = frame_row.cells.iter().find(|c| c.col == col)?;
    if let Some(uri) = &cell.hyperlink_uri {
        return validate_open_url(uri);
    }
    let char_idx = char_index_at_col(frame_row, col)?;
    plain_url_covering_cell(&row_text_from_cells(frame_row), char_idx)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::uri_for_gesture;
    use crate::engine::frame::{CellStyle, Frame, FrameCell, FrameRow, Rgb};

    #[test]
    fn uri_for_gesture_prefers_osc8_field() {
        let osc = Arc::<str>::from("https://osc.example/only");
        let plain = "見 https://plain.example/not-this";
        let mut cells = Vec::new();
        let mut col = 0u16;
        for ch in plain.chars() {
            let hyperlink_uri = if ch == '見' {
                Some(Arc::clone(&osc))
            } else {
                None
            };
            cells.push(FrameCell {
                col,
                grapheme: ch.to_string(),
                fg: Rgb {
                    r: 200,
                    g: 200,
                    b: 200,
                },
                bg: None,
                wide: false,
                selected: false,
                style: CellStyle::default(),
                hyperlink_uri,
            });
            col += 1;
        }
        let frame = Frame {
            cols: col,
            rows: 1,
            default_fg: Rgb {
                r: 200,
                g: 200,
                b: 200,
            },
            default_bg: Rgb { r: 0, g: 0, b: 0 },
            grid: vec![FrameRow { cells }],
            cursor: None,
        };
        assert_eq!(
            uri_for_gesture(&frame, 0, 0).as_deref(),
            Some("https://osc.example/only")
        );
        let h_col = plain
            .chars()
            .enumerate()
            .find(|(_, ch)| *ch == 'h')
            .expect("plain url in fixture")
            .0 as u16;
        assert_eq!(
            uri_for_gesture(&frame, h_col, 0).as_deref(),
            Some("https://plain.example/not-this")
        );
    }
}
