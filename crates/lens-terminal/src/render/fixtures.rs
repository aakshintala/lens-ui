//! Synthetic `Frame` builders for the real-window harness (`test-util`) and
//! Criterion Frame-construction benches (`bench`).

use crate::{CellStyle, Frame, FrameCell, FrameRow, Rgb};

const FG: Rgb = Rgb {
    r: 220,
    g: 220,
    b: 220,
};
const BG: Rgb = Rgb {
    r: 12,
    g: 12,
    b: 12,
};

/// A dense ASCII frame: every cell is `fill`, narrow, default style.
pub fn ascii_frame(cols: u16, rows: u16, fill: char) -> Frame {
    let grapheme = fill.to_string();
    let mut grid = Vec::with_capacity(rows as usize);
    for _ in 0..rows {
        let mut cells = Vec::with_capacity(cols as usize);
        for col in 0..cols {
            cells.push(FrameCell {
                col,
                grapheme: grapheme.clone(),
                fg: FG,
                bg: None,
                wide: false,
                selected: false,
                style: CellStyle::default(),
            });
        }
        grid.push(FrameRow { cells });
    }
    Frame {
        cols,
        rows,
        default_fg: FG,
        default_bg: BG,
        grid,
    }
}
