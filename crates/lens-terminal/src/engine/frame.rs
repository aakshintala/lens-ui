/// Resolved 24-bit color. No Ghostty/gpui type crosses the Frame seam.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

/// Full SGR attribute set carried per cell (design: 1c renders the full set;
/// paint.rs today does only bold+selection). Mirrors libghostty `style::Style`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct CellStyle {
    pub bold: bool,
    pub italic: bool,
    pub faint: bool,
    pub blink: bool,
    pub inverse: bool,
    pub invisible: bool,
    pub strikethrough: bool,
    pub overline: bool,
    pub underline: UnderlineStyle,
    pub underline_color: Option<Rgb>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum UnderlineStyle {
    #[default]
    None,
    Single,
    Double,
    Curly,
    Dotted,
    Dashed,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FrameCell {
    /// Grid column (wide spacer tails/heads are dropped).
    pub col: u16,
    /// One grapheme cluster; `" "` for blank.
    pub grapheme: String,
    pub fg: Rgb,
    /// `None` = default bg.
    pub bg: Option<Rgb>,
    pub wide: bool,
    pub selected: bool,
    pub style: CellStyle,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FrameRow {
    pub cells: Vec<FrameCell>,
}

/// Immutable owned snapshot of the visible grid — the Send boundary.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub cols: u16,
    pub rows: u16,
    pub default_fg: Rgb,
    pub default_bg: Rgb,
    pub grid: Vec<FrameRow>,
}
