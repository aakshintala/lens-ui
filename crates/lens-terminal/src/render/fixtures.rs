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

fn narrow_cell(col: u16, grapheme: &str) -> FrameCell {
    FrameCell {
        col,
        grapheme: grapheme.to_owned(),
        fg: FG,
        bg: None,
        wide: false,
        selected: false,
        style: CellStyle::default(),
    }
}

fn wide_cell(col: u16, grapheme: &str) -> FrameCell {
    FrameCell {
        wide: true,
        ..narrow_cell(col, grapheme)
    }
}

/// A dense ASCII frame: every cell is `fill`, narrow, default style.
pub fn ascii_frame(cols: u16, rows: u16, fill: char) -> Frame {
    let grapheme = fill.to_string();
    let mut grid = Vec::with_capacity(rows as usize);
    for _ in 0..rows {
        let cells = (0..cols).map(|col| narrow_cell(col, &grapheme)).collect();
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

/// Row 0 is all narrow ASCII; row 1 leads with one wide CJK cell (cols 0–1)
/// then narrow cells. Exercises per-row (row 0) vs per-cell (row 1) routing.
pub fn mixed_ascii_wide_frame(cols: u16, rows: u16) -> Frame {
    let mut grid = Vec::with_capacity(rows as usize);
    for r in 0..rows {
        let cells: Vec<FrameCell> = if r == 1 && cols >= 2 {
            std::iter::once(wide_cell(0, "日"))
                .chain((2..cols).map(|col| narrow_cell(col, "b")))
                .collect()
        } else {
            (0..cols).map(|col| narrow_cell(col, "a")).collect()
        };
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
