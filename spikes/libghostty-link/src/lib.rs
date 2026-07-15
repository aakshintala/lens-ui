//! Link smoke-test for the vendored `libghostty-vt` (task-2 verification).
//!
//! Proves the vendored crate builds-from-source (zig@0.15) and links from
//! inside the Lens workspace, and that the safe API drives a real terminal:
//! feed bytes -> read the cell back.

#[cfg(test)]
mod tests {
    use libghostty_vt::{
        Terminal, TerminalOptions,
        terminal::{Point, PointCoordinate},
    };

    /// Read the Unicode scalar at an active-screen coordinate via a tracked
    /// grid ref (the same path the example uses).
    fn codepoint_at(terminal: &Terminal<'_, '_>, x: u16, y: u32) -> char {
        let tracked = terminal
            .track_grid_ref(Point::Active(PointCoordinate { x, y }))
            .expect("track grid ref");
        let snapshot = tracked
            .snapshot(terminal)
            .expect("snapshot")
            .expect("tracked cell has a value");
        let cell = snapshot.cell().expect("cell");
        let cp = cell.codepoint().expect("codepoint");
        char::from_u32(cp).expect("valid scalar")
    }

    #[test]
    fn feed_bytes_then_read_cells() {
        let mut terminal = Terminal::new(TerminalOptions {
            cols: 8,
            rows: 3,
            max_scrollback: 100,
        })
        .expect("construct terminal");

        terminal.vt_write(b"hi");

        assert_eq!(codepoint_at(&terminal, 0, 0), 'h');
        assert_eq!(codepoint_at(&terminal, 1, 0), 'i');
    }
}
