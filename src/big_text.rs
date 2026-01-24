use std::fmt::Write as _;

use ratatui::{layout::Rect, widgets::Widget};
use unicode_width::UnicodeWidthChar as _;

/// Yields slices of chars where each chunk has unicode width <= max_width.
/// Wide characters (width > 1) are always in their own chunk due to Kitty limitations (or perhaps
/// this is problem with the implementation - in any case this solves it).
fn unicode_chunks(chars: &[char], max_width: u8) -> impl Iterator<Item = (&[char], u8)> {
    let mut start = 0;
    std::iter::from_fn(move || {
        if start >= chars.len() {
            return None;
        }
        let mut end = start;
        let mut width = 0;
        while end < chars.len() {
            let char_width = chars[end].width().unwrap_or(1) as u8;
            if char_width > 1 {
                if width > 0 {
                    break; // end current chunk, wide char starts next chunk
                }
                // wide char alone
                width = char_width;
                end += 1;
                break;
            }
            if width + char_width > max_width {
                break;
            }
            width += char_width;
            end += 1;
        }
        let chunk = &chars[start..end];
        start = end;
        Some((chunk, width))
    })
}

pub struct BigText<'a> {
    text: &'a str,
    tier: u8,
}

impl<'a> BigText<'a> {
    pub fn new(text: &'a str, tier: u8) -> Self {
        BigText { text, tier }
    }

    /// When wrapping with text-wrap, the final line width must be known in advance before
    /// rendering.
    #[inline]
    pub fn size_ratio(tier: u8) -> (u8, u8) {
        match tier {
            1 => (7, 7),
            2 => (5, 6),
            3 => (3, 4),
            4 => (2, 3),
            5 => (3, 5),
            _ => (1, 3),
        }
    }

    #[inline]
    fn text_sizing_sequence(&self, area_width: u16) -> String {
        let (n, d) = BigText::size_ratio(self.tier);

        let mut symbol = String::new();

        // Erase-character dance.
        // We must erase anything inside area, which is 2 lines high and `area.width` wide.
        // This must be done before we write the text.
        // Also disable DECAWM, unsure if really necessary.
        write!(symbol, "\x1b[{}X\x1B[?7l", area_width).expect("write to string");
        write!(symbol, "\x1b[1B").expect("write to string");
        write!(symbol, "\x1b[{}X\x1B[?7l", area_width).expect("write to string");
        write!(symbol, "\x1b[1A").expect("write to string");

        let chars: Vec<char> = self.text.chars().collect();
        for (chunk, chunk_width) in unicode_chunks(&chars, d) {
            // w=n for full chunks (width == d), round up for partial chunks to avoid clipping
            let w = if chunk_width == d {
                n
            } else {
                (chunk_width * n).div_ceil(d)
            };

            write!(symbol, "\x1b]66;s=2:n={n}:d={d}:w={w};").expect("write to string");
            symbol.extend(chunk);
            write!(symbol, "\x1b\\").expect("write to string");
        }
        symbol
    }
}

impl Widget for BigText<'_> {
    fn render(self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let symbol = self.text_sizing_sequence(area.width);

        // Skip entire text area except first cell
        let mut skip_first = false;

        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                if skip_first {
                    buf.cell_mut((x, y)).map(|cell| cell.set_skip(true));
                } else {
                    skip_first = true;
                    buf.cell_mut((x, y)).map(|cell| cell.set_symbol(&symbol));
                }
            }
        }
    }
}
