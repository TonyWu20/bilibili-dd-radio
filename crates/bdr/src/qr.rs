//! QR code rendering as Unicode block characters.
//!
//! Two module rows are packed into one block-character row: a full
//! block for two dark modules, an upper half block for the top one.
//! The result carries a quiet zone so a camera can read the code.

use qrcode::{Color, QrCode};

/// Render `url` as block-character lines.
///
/// Returns `None` when the code cannot be built. All lines are the
/// same width and the block is padded with a two-cell quiet zone.
pub fn render_block(url: &str) -> Option<Vec<String>> {
    let qr = QrCode::new(url.as_bytes()).ok()?;
    // A QR code is square. One block row covers two module rows.
    let width = qr.width();
    let height = width;
    let mut lines: Vec<String> = Vec::new();
    for row in (0..height).step_by(2) {
        let mut line = String::with_capacity(width);
        for col in 0..width {
            let top = qr[(row, col)] == Color::Dark;
            let bottom = row + 1 < height && qr[(row + 1, col)] == Color::Dark;
            let ch = match (top, bottom) {
                (true, true) => '█',
                (true, false) => '▀',
                (false, true) => '▄',
                (false, false) => ' ',
            };
            line.push(ch);
        }
        lines.push(line);
    }
    // Two-cell quiet zone, plus a blank row above and below.
    let total = width.saturating_add(4);
    let mut out: Vec<String> = Vec::with_capacity(lines.len() + 2);
    out.push(" ".repeat(total));
    for line in &lines {
        out.push(format!("  {line:<width$}  "));
    }
    out.push(" ".repeat(total));
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_block_gives_even_lines() {
        let lines =
            render_block("https://account.bilibili.com/h5/scan-web?qrcode_key=abcd").unwrap();
        let w = lines[0].chars().count();
        for l in &lines {
            assert_eq!(l.chars().count(), w, "ragged line: {l}");
        }
        assert!(lines.iter().any(|l| l.contains('█')));
        // A QR code is square: one row per two modules, so the
        // character block is half as tall as it is wide.
        assert!(lines.len() * 2 <= w + 4, "block not square-ish");
    }

    #[test]
    fn render_block_handles_a_long_url() {
        // The QR login URL is long. The code must still build.
        let url = "https://account.bilibili.com/h5/account-h5/auth/scan-web?navhide=1&callback=close&qrcode_key="
            .to_string();
        let lines = render_block(&url).unwrap();
        let w = lines[0].chars().count();
        assert!(lines.iter().all(|l| l.chars().count() == w));
        assert!(lines.len() >= 10, "too few rows: {}", lines.len());
    }
}
