//! Synthetic `Frame` builders for the real-window harness (`test-util`) and
//! Criterion Frame-construction benches (`bench`).

#[cfg(any(test, feature = "test-util"))]
use crate::UnderlineStyle;
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

/// A single narrow row exercising the full SGR set (bold, italic, faint,
/// inverse, every underline kind, overline, strikethrough, invisible, and a
/// selected cell). All narrow → per-row path + decoration quads.
#[cfg(any(test, feature = "test-util"))]
pub fn sgr_frame() -> Frame {
    let styled = |col: u16, ch: &str, style: CellStyle, selected: bool| FrameCell {
        col,
        grapheme: ch.to_owned(),
        fg: FG,
        bg: None,
        wide: false,
        selected,
        style,
    };
    let s = |f: fn(&mut CellStyle)| {
        let mut st = CellStyle::default();
        f(&mut st);
        st
    };
    let cells = vec![
        styled(0, "n", CellStyle::default(), false),
        styled(1, "b", s(|s| s.bold = true), false),
        styled(2, "i", s(|s| s.italic = true), false),
        styled(3, "f", s(|s| s.faint = true), false),
        styled(4, "v", s(|s| s.inverse = true), false),
        styled(5, "u", s(|s| s.underline = UnderlineStyle::Single), false),
        styled(6, "c", s(|s| s.underline = UnderlineStyle::Curly), false),
        styled(7, "d", s(|s| s.underline = UnderlineStyle::Double), false),
        styled(8, "o", s(|s| s.underline = UnderlineStyle::Dotted), false),
        styled(9, "a", s(|s| s.underline = UnderlineStyle::Dashed), false),
        styled(10, "l", s(|s| s.overline = true), false),
        styled(11, "s", s(|s| s.strikethrough = true), false),
        styled(12, "x", s(|s| s.invisible = true), false),
        styled(13, "S", CellStyle::default(), true),
    ];
    let cols = cells.len() as u16;
    Frame {
        cols,
        rows: 1,
        default_fg: FG,
        default_bg: BG,
        grid: vec![FrameRow { cells }],
    }
}

/// A representative dense wide/emoji frame, mirroring the spike's
/// `wide_and_sgr` density (`spikes/terminal-render/src/fixtures.rs`) that the
/// perf budgets were calibrated against. Rows cycle: CJK+ASCII, emoji+ASCII,
/// dense ASCII SGR, half-CJK-block + ASCII — so wide/emoji rows route per-cell
/// while ASCII rows route per-row, exactly as production does. Emoji (the
/// costly color-glyph path) appears in ~1/4 of rows, not all.
pub fn dense_wide_emoji_frame(cols: u16, rows: u16) -> Frame {
    let cjk = ['日', '本', '語', '漢', '字'];
    let emoji = ['😀', '🚀', '✨', '🎉', '🌟'];
    let mut grid = Vec::with_capacity(rows as usize);
    for r in 0..rows {
        let mut cells = Vec::new();
        match r % 4 {
            // CJK + ASCII: wide glyph then one narrow, repeating (~1 wide/3 cols).
            0 => {
                let (mut col, mut i) = (0u16, 0usize);
                while col + 1 < cols {
                    cells.push(wide_cell(col, &cjk[i % cjk.len()].to_string()));
                    col += 2;
                    if col < cols {
                        cells.push(narrow_cell(col, "a"));
                        col += 1;
                    }
                    i += 1;
                }
            }
            // Emoji + ASCII, same density.
            1 => {
                let (mut col, mut i) = (0u16, 0usize);
                while col + 1 < cols {
                    cells.push(wide_cell(col, &emoji[i % emoji.len()].to_string()));
                    col += 2;
                    if col < cols {
                        cells.push(narrow_cell(col, "."));
                        col += 1;
                    }
                    i += 1;
                }
            }
            // Dense ASCII (no wide) → routes per-row, like real text rows.
            2 => {
                for col in 0..cols {
                    cells.push(narrow_cell(col, "X"));
                }
            }
            // Half wide-CJK block, half dense ASCII.
            _ => {
                let half = cols / 2;
                let (mut col, mut i) = (0u16, 0usize);
                while col + 1 < half {
                    cells.push(wide_cell(col, &cjk[i % cjk.len()].to_string()));
                    col += 2;
                    i += 1;
                }
                while col < cols {
                    cells.push(narrow_cell(col, "#"));
                    col += 1;
                }
            }
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

/// The **pathological** worst case: every column pair is a wide CJK/emoji cell
/// (alternating), 50% emoji — so every row is per-cell with the maximum shaped
/// glyphs (`~cols/2 * rows`) and the costly color-glyph path everywhere. Not
/// representative of real terminals; kept purely as a regression guard so the
/// per-cell path can never silently degrade past a generous ceiling.
#[cfg(any(test, feature = "test-util"))]
pub fn pathological_wide_emoji_frame(cols: u16, rows: u16) -> Frame {
    let mut grid = Vec::with_capacity(rows as usize);
    for r in 0..rows {
        let mut cells = Vec::new();
        let mut col = 0u16;
        let mut i = 0usize;
        while col + 1 < cols {
            let g = if (i + r as usize).is_multiple_of(2) {
                "日"
            } else {
                "😀"
            };
            cells.push(wide_cell(col, g));
            col += 2;
            i += 1;
        }
        if col < cols {
            cells.push(narrow_cell(col, "a"));
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

/// Row 0 is all narrow ASCII; row 1 leads with one wide CJK cell (cols 0–1)
/// then narrow cells. Exercises per-row (row 0) vs per-cell (row 1) routing.
#[cfg(any(test, feature = "test-util"))]
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
