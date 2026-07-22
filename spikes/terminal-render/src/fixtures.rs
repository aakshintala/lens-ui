//! Fixed VT byte fixtures for the render-viability spike.
//!
//! Do not change the patterns — measurement comparability depends on them.

/// Worst case, all-dirty. Clear + fill every cell with rotating ASCII and
/// truecolor SGR foreground changing every ~8 cells. Re-emit every frame
/// to keep `Dirty::Full`.
pub fn full_redraw(cols: u16, rows: u16) -> Vec<u8> {
    let mut out = Vec::with_capacity((cols as usize) * (rows as usize) * 12);
    out.extend_from_slice(b"\x1b[2J\x1b[H");
    const GLYPHS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789@#$%&*";
    for row in 0..rows {
        for col in 0..cols {
            let gi = ((row as usize) * 3 + (col as usize)) % GLYPHS.len();
            let r = ((col.wrapping_mul(3) + row) % 256) as u8;
            let g = ((col.wrapping_mul(5) + row.wrapping_mul(2)) % 256) as u8;
            let b = ((col.wrapping_mul(7) + 40) % 256) as u8;
            if col % 8 == 0 {
                out.extend_from_slice(
                    format!("\x1b[38;2;{r};{g};{b}m").as_bytes(),
                );
            }
            out.push(GLYPHS[gi]);
        }
        if row + 1 < rows {
            out.extend_from_slice(b"\r\n");
        }
    }
    out
}

/// Typical case. Caller paints `full_redraw` once for the static base, then
/// feeds this each frame to rewrite only 3 rows (cursor-addressed).
/// Expect `Dirty::Partial` with ~3 dirty rows.
pub fn partial_update(cols: u16, rows: u16, frame_n: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(3 * (cols as usize + 32));
    let targets = partial_target_rows(rows);
    const GLYPHS: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    for &row in &targets {
        // 1-based CUP
        out.extend_from_slice(format!("\x1b[{};1H", row + 1).as_bytes());
        out.extend_from_slice(b"\x1b[0m");
        for col in 0..cols {
            let r = ((frame_n as u16).wrapping_mul(13).wrapping_add(col) % 256) as u8;
            let g = ((frame_n as u16).wrapping_mul(7).wrapping_add(row * 3) % 256) as u8;
            let b = ((col.wrapping_mul(11) + 80) % 256) as u8;
            if col % 8 == 0 {
                out.extend_from_slice(
                    format!("\x1b[38;2;{r};{g};{b}m").as_bytes(),
                );
            }
            let gi = ((frame_n as usize)
                .wrapping_add(col as usize)
                .wrapping_add(row as usize * 5))
                % GLYPHS.len();
            out.push(GLYPHS[gi]);
        }
    }
    out
}

/// Rows rewritten by [`partial_update`] (0-based).
pub fn partial_target_rows(rows: u16) -> [u16; 3] {
    let mid = rows / 2;
    let last = rows.saturating_sub(1);
    [0, mid, last]
}

/// Shaping stress + correctness: CJK + emoji interleaved with narrow ASCII,
/// plus dense truecolor SGR (color change every cell).
pub fn wide_and_sgr(cols: u16, rows: u16) -> Vec<u8> {
    let mut out = Vec::with_capacity((cols as usize) * (rows as usize) * 16);
    out.extend_from_slice(b"\x1b[2J\x1b[H");

    let cjk = "日本語漢字テスト寬";
    let emoji = "😀🚀✨🎉🌟";
    let ascii = "HelloWideGrid!";

    for row in 0..rows {
        out.extend_from_slice(format!("\x1b[{};1H", row + 1).as_bytes());
        match row % 4 {
            0 => {
                // CJK + ASCII mix
                let mut col = 0u16;
                let mut chars = cjk.chars().cycle();
                while col < cols {
                    let ch = chars.next().unwrap();
                    let s = ch.to_string();
                    out.extend_from_slice(s.as_bytes());
                    // CJK is typically width 2; leave room for spacer
                    col = col.saturating_add(2);
                    if col >= cols {
                        break;
                    }
                    if col < cols {
                        let a = ascii.as_bytes()[(col as usize) % ascii.len()];
                        out.push(a);
                        col += 1;
                    }
                }
            }
            1 => {
                // Emoji + ASCII mix
                let mut col = 0u16;
                let mut chars = emoji.chars().cycle();
                while col < cols {
                    let ch = chars.next().unwrap();
                    out.extend_from_slice(ch.to_string().as_bytes());
                    col = col.saturating_add(2);
                    if col >= cols {
                        break;
                    }
                    if col < cols {
                        out.push(b'.');
                        col += 1;
                    }
                }
            }
            2 => {
                // Dense truecolor SGR — color change every cell
                for col in 0..cols {
                    let r = ((col * 3) % 256) as u8;
                    let g = ((row * 5 + col) % 256) as u8;
                    let b = ((255 - col) % 256) as u8;
                    out.extend_from_slice(
                        format!("\x1b[38;2;{r};{g};{b}m").as_bytes(),
                    );
                    out.push(b'X');
                }
            }
            _ => {
                // Alternating wide CJK block then dense ASCII
                let half = cols / 2;
                let mut col = 0u16;
                let mut chars = cjk.chars().cycle();
                while col + 1 < half {
                    let ch = chars.next().unwrap();
                    out.extend_from_slice(ch.to_string().as_bytes());
                    col += 2;
                }
                while col < cols {
                    out.push(b'#');
                    col += 1;
                }
            }
        }
    }
    out
}
