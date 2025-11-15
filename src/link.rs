use ratatui::widgets::Widget;

pub struct Link<'a> {
    text: &'a str,
    url: &'a str,
}

impl<'a> Link<'a> {
    pub fn new(text: &'a str, url: &'a str) -> Link<'a> {
        Link { text, url }
    }
}

impl Widget for Link<'_> {
    fn render(self, area: ratatui::prelude::Rect, buf: &mut ratatui::prelude::Buffer)
    where
        Self: Sized,
    {
        for chunk in pairs_and_chunks(self.text) {
            match chunk {
                Chunk::First(f, s) => {
                    let Some(cell) = buf.cell_mut((area.x, area.y)) else {
                        panic!("cell not there");
                    };
                    let symbol = format!("\x1b]8;;{}\x1b\\{f}{s}", self.url);
                    cell.set_symbol(&symbol);
                    let Some(cell) = buf.cell_mut((area.x + 1, area.y)) else {
                        panic!("cell not there");
                    };
                    cell.set_skip(true);
                }
                Chunk::Middle(pos, f, s) => {
                    let Some(cell) = buf.cell_mut((area.x + pos, area.y)) else {
                        panic!("cell not there");
                    };
                    cell.set_symbol(&format!("{f}{s}"));
                    let Some(cell) = buf.cell_mut((area.x + pos + 1, area.y)) else {
                        panic!("cell not there");
                    };
                    cell.set_skip(true);
                }
                Chunk::MiddleSquezed(pos, f) => {
                    let Some(cell) = buf.cell_mut((area.x + pos, area.y)) else {
                        panic!("cell not there");
                    };
                    cell.set_symbol(&format!("{f}"));
                }
                Chunk::Last(pos, f, s) => {
                    let Some(cell) = buf.cell_mut((area.x + pos, area.y)) else {
                        panic!("cell not there");
                    };
                    cell.set_symbol(&format!("{f}{s}\x1b]8;;\x1b\\"));
                    let Some(cell) = buf.cell_mut((area.x + pos + 1, area.y)) else {
                        panic!("cell not there");
                    };
                    cell.set_skip(true);
                }
            }
        }
    }
}

#[derive(Debug, PartialEq)]
enum Chunk {
    First(char, char),
    Middle(u16, char, char),
    MiddleSquezed(u16, char),
    Last(u16, char, char),
}

fn pairs_and_chunks(text: &str) -> impl Iterator<Item = Chunk> {
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();

    (0..len).filter_map(move |i| {
        if len < 2 {
            None
        } else if i == 0 {
            // First chunk - chars 0 and 1
            Some(Chunk::First(chars[0], chars[1]))
        } else if i == len - 2 {
            // Last chunk - last 2 chars
            Some(Chunk::Last(i as u16, chars[i], chars[i + 1]))
        } else if i >= 2 && i < len - 2 {
            // Middle section (between first 2 and last 2)
            let middle_start = 2;
            let middle_end = len - 2;
            let middle_len = middle_end - middle_start;
            let pos_in_middle = i - middle_start;

            if middle_len % 2 == 1 && pos_in_middle == middle_len - 1 {
                // Odd middle length, and we're at the last middle char
                Some(Chunk::MiddleSquezed(i as u16, chars[i]))
            } else if pos_in_middle % 2 == 0 {
                // Even position in middle - start of a pair
                Some(Chunk::Middle(i as u16, chars[i], chars[i + 1]))
            } else {
                // Odd position - already handled as second char of previous pair
                None
            }
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use crate::link::{Chunk, Link, pairs_and_chunks};
    use pretty_assertions::assert_eq;
    use ratatui::{buffer::Cell, layout::Rect, prelude::Buffer, widgets::Widget};

    #[test]
    fn chunks() {
        assert_eq!(
            vec![Chunk::First('a', 'b')],
            pairs_and_chunks("ab").collect::<Vec<Chunk>>()
        );
        assert_eq!(
            vec![Chunk::First('a', 'b'), Chunk::Last(2, 'c', 'd')],
            pairs_and_chunks("abcd").collect::<Vec<Chunk>>()
        );
        assert_eq!(
            vec![
                Chunk::First('a', 'b'),
                Chunk::MiddleSquezed(2, 'c'),
                Chunk::Last(3, 'd', 'e')
            ],
            pairs_and_chunks("abcde").collect::<Vec<Chunk>>()
        );
        assert_eq!(
            vec![
                Chunk::First('a', 'b'),
                Chunk::Middle(2, 'c', 'd'),
                Chunk::Last(4, 'e', 'f')
            ],
            pairs_and_chunks("abcdef").collect::<Vec<Chunk>>()
        );
        assert_eq!(
            vec![
                Chunk::First('a', 'b'),
                Chunk::Middle(2, 'c', 'd'),
                Chunk::MiddleSquezed(4, 'e'),
                Chunk::Last(5, 'f', 'g')
            ],
            pairs_and_chunks("abcdefg").collect::<Vec<Chunk>>()
        );
        assert_eq!(
            vec![
                Chunk::First('a', 'b'),
                Chunk::Middle(2, 'c', 'd'),
                Chunk::Middle(4, 'e', 'f'),
                Chunk::Last(6, 'g', 'h')
            ],
            pairs_and_chunks("abcdefgh").collect::<Vec<Chunk>>()
        );
    }

    #[test]
    fn link_4() {
        let area = Rect::new(0, 0, 4, 1);
        let link = Link::new("link", "http://example.com");
        let mut buf = Buffer::empty(area);
        link.render(area, &mut buf);
        let cells = buf.content;
        let mut empty = Cell::EMPTY.clone();
        let expected = vec![
            Cell::new("\x1b]8;;http://example.com\x1b\\li"),
            empty.set_skip(true).clone(),
            Cell::new("nk\x1b]8;;\x1b\\"),
            empty.set_skip(true).clone(),
        ];
        assert_eq!(expected, cells);
    }

    #[test]
    fn link_5() {
        let area = Rect::new(0, 0, 5, 1);
        let link = Link::new("linky", "http://example.com");
        let mut buf = Buffer::empty(area);
        link.render(area, &mut buf);
        let cells = buf.content;
        let mut empty = Cell::EMPTY.clone();
        let expected = vec![
            Cell::new("\x1b]8;;http://example.com\x1b\\li"),
            empty.set_skip(true).clone(),
            Cell::new("n"),
            Cell::new("ky\x1b]8;;\x1b\\"),
            empty.set_skip(true).clone(),
        ];
        assert_eq!(expected, cells);
    }

    #[test]
    #[ignore]
    fn link_8() {
        let area = Rect::new(0, 0, 8, 1);
        let link = Link::new("linklink", "http://example.com");
        let mut buf = Buffer::empty(area);
        link.render(area, &mut buf);
        let cells = buf.content;
        let mut empty = Cell::EMPTY.clone();
        let expected = vec![
            Cell::new("\x1b]8;;http://example.com\x1b\\li"),
            empty.set_skip(true).clone(),
            Cell::new("nk"),
            empty.set_skip(true).clone(),
            Cell::new("li"),
            empty.set_skip(true).clone(),
            Cell::new("nk\x1b]8;;\x1b\\"),
            empty.set_skip(true).clone(),
        ];
        assert_eq!(expected, cells);
    }

    #[test]
    fn link_buffer_out() {
        let area = Rect::new(0, 0, 4, 1);
        let link = Link::new("link", "http://example.com");
        let mut buf = Buffer::empty(area);
        link.render(area, &mut buf);
        buf.fmt
    }

}
