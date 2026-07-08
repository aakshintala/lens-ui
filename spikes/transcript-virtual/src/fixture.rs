//! Synthetic row generator for the probe matrix (spec §3).

use gpui::Pixels;

/// Row shape in the height distribution sweep.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RowKind {
    /// Default one-liner transcript line.
    OneLiner,
    /// Tall fenced-code block placeholder.
    CodeBlock,
    /// Image attachment placeholder (fixed tall box).
    ImagePlaceholder,
    /// Expanded tool-span block.
    ToolSpan,
}

/// A single synthetic transcript row (no virtualization yet).
#[derive(Clone, Debug)]
pub struct FixtureRow {
    pub id: u64,
    pub kind: RowKind,
    pub text: String,
    /// Extra height layered on the kind's baseline (contract-1b mutation hook).
    pub height_delta: Pixels,
}

/// Parametrized synthetic fixture: N rows, periodic tall rows, streaming tail,
/// and a designated off-screen-above height-mutation target.
#[derive(Clone, Debug)]
pub struct Fixture {
    pub rows: Vec<FixtureRow>,
    /// Index of the row that grows on append (always the last row).
    pub growing_last_ix: usize,
    /// Id of the row designated for off-screen height mutation (contract 1b).
    pub mutable_offscreen_id: u64,
}

impl Fixture {
    /// Build `n` synthetic rows: mostly one-liners with periodic tall rows every
    /// 25 items (code → image → tool-span cycle). The last row is the streaming
    /// tail; row `mutable_offscreen_ix` is the 1b mutation target.
    pub fn synthetic(n: usize) -> Self {
        assert!(n > 0, "fixture needs at least one row");

        let mutable_offscreen_ix = (n / 3).max(1).min(n.saturating_sub(2));
        let mut mutable_offscreen_id = 0;

        let rows = (0..n)
            .map(|ix| {
                let kind = row_kind_at(ix);
                let id = ix as u64;
                if ix == mutable_offscreen_ix {
                    mutable_offscreen_id = id;
                }
                FixtureRow {
                    id,
                    kind,
                    text: default_text(kind, ix),
                    height_delta: gpui::px(0.),
                }
            })
            .collect::<Vec<_>>();

        Self {
            growing_last_ix: n - 1,
            mutable_offscreen_id,
            rows,
        }
    }

    /// Append text to the streaming last item (contract 1a).
    pub fn append_to_last(&mut self, chunk: &str) {
        if let Some(row) = self.rows.last_mut() {
            row.text.push_str(chunk);
        }
    }

    /// Mutate height of the designated off-screen-above item (contract 1b).
    pub fn mutate_offscreen_height(&mut self, delta: Pixels) {
        if let Some(row) = self
            .rows
            .iter_mut()
            .find(|r| r.id == self.mutable_offscreen_id)
        {
            row.height_delta += delta;
        }
    }

    pub fn row(&self, id: u64) -> Option<&FixtureRow> {
        self.rows.iter().find(|r| r.id == id)
    }
}

fn row_kind_at(ix: usize) -> RowKind {
    if ix % 25 == 0 {
        match (ix / 25) % 3 {
            0 => RowKind::CodeBlock,
            1 => RowKind::ImagePlaceholder,
            _ => RowKind::ToolSpan,
        }
    } else {
        RowKind::OneLiner
    }
}

fn default_text(kind: RowKind, ix: usize) -> String {
    match kind {
        RowKind::OneLiner => format!("line {ix}: short transcript utterance"),
        RowKind::CodeBlock => format!("line {ix}: ```\nfn example() {{\n    // ...\n}}\n```"),
        RowKind::ImagePlaceholder => format!("line {ix}: [image attachment placeholder]"),
        RowKind::ToolSpan => format!(
            "line {ix}: tool span expanded — read_file(path=\"src/main.rs\") → 42 lines"
        ),
    }
}
