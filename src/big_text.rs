use std::fmt::Write as _;

use ratatui::{layout::Rect, widgets::Widget};

pub struct BigText<'a> {
    text: &'a str,
    tier: u8,
}

impl<'a> BigText<'a> {
    pub fn new(text: &'a str, tier: u8) -> Self {
        BigText { text, tier }
    }

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

    #[expect(clippy::unwrap_used)]
    #[inline]
    fn text_sizing_sequence(&self, area_width: u16) -> String {
        let (n, d) = BigText::size_ratio(self.tier);

        let chars: Vec<char> = self.text.chars().collect();
        let chunk_count = chars.len().div_ceil(d as usize);
        let width_digits = area_width.checked_ilog10().unwrap_or(0) as usize + 1;
        let capacity = 19 + 2 * width_digits + self.text.len() + chunk_count * 24;
        let mut symbol = String::with_capacity(capacity);

        // Erase-character dance.
        // We must erase anything inside area, which is 2 lines high and `area.width` wide.
        // This must be done before we write the text.
        // Also disable DECAWM, unsure if really necessary.
        write!(symbol, "\x1b[{}X\x1B[?7l", area_width).expect("write to string");
        write!(symbol, "\x1b[1B").expect("write to string");
        write!(symbol, "\x1b[{}X\x1B[?7l", area_width).expect("write to string");
        write!(symbol, "\x1b[1A").expect("write to string");

        for chunk in chars.chunks(d as usize) {
            write!(symbol, "\x1b]66;s=2:n={n}:d={d}:w={n};").unwrap();
            symbol.extend(chunk);
            write!(symbol, "\x1b\\").unwrap(); // Could also use BEL, but this seems safer.
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
