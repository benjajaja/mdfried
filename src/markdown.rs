use std::sync::mpsc::Sender;

use ratatui::{
    style::Stylize,
    text::{Line, Span},
};
use regex::Regex;
use termimad::{minimad::parse_text, CompositeKind, FmtLine, FmtText, MadSkin};

use crate::{
    widget_sources::{SourceID, WidgetSourceData},
    Error, Event, WidgetSource, WidthEvent,
};

enum Block {
    Header(u8, String),
    // Image(&'a str, &'a str),
    Markdown(String),
}

pub fn parse(text: &str, width: u16, tx: &Sender<WidthEvent>) -> Result<(), Error> {
    let mut sender = SendTracker {
        width,
        tx,
        index: 0,
    };

    let text_blocks = parse_headers_and_images(text);

    let skin = MadSkin::default();

    for block in text_blocks {
        match block {
            Block::Header(tier, text) => {
                let spans = vec![Span::from(text)];
                sender.send_event(Event::ParseHeader(sender.index, tier, spans))?;
            }
            Block::Markdown(text) => {
                let text = parse_text(&text, termimad::minimad::Options::default());

                let fmt_text = FmtText::from_text(&skin, text, Some(width as usize));

                for line in fmt_text.lines {
                    match line {
                        FmtLine::Normal(fmtcomp) => {
                            // let dbg = format!("{fmtcomp:?}");
                            let mut spans = vec![];

                            for comp in fmtcomp.compounds {
                                let mut span = Span::from(comp.src.to_string());

                                if comp.code {
                                    // Don't apply any other styles to `code`.
                                    span = span.on_dark_gray();
                                } else {
                                    if comp.bold {
                                        span = span.bold();
                                    }
                                    if comp.italic {
                                        span = span.italic();
                                    }
                                    if comp.strikeout {
                                        span = span.crossed_out();
                                    }

                                    if matches!(fmtcomp.kind, CompositeKind::ListItem(_)) {
                                        span = span.yellow();
                                    }
                                }
                                spans.push(span);
                            }

                            match fmtcomp.kind {
                                CompositeKind::Header(tier) => {
                                    sender.send_event(Event::ParseHeader(
                                        sender.index,
                                        tier,
                                        spans,
                                    ))?;
                                }
                                _ => {
                                    let line = Line::from(spans);
                                    sender.send_parse(WidgetSourceData::Line(line), 1)?;
                                }
                            }
                            // sender.send_parse(WidgetSourceData::Line(Line::from(dbg)), 1)?;
                            // sender.send_parse(
                            // WidgetSourceData::Line(Line::from(format!("{is_header:?}"))),
                            // 1,
                            // )?;
                        }
                        FmtLine::HorizontalRule => {
                            sender.send_parse(
                                WidgetSourceData::Line(Line::from(
                                    "\u{2505}".repeat(width as usize),
                                )),
                                1,
                            )?;
                        }
                        _ => {
                            sender.send_parse(
                                WidgetSourceData::Line(Line::from(format!("{line:?}"))),
                                1,
                            )?;
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

fn parse_headers_and_images(text: &str) -> Vec<Block> {
    // Regex to match lines starting with 1-6 `#` characters
    let re = Regex::new(r"^(#+)\s*(.*)").unwrap();

    let mut blocks = Vec::new();
    let mut current_block = String::new();

    for line in text.lines() {
        if let Some(captures) = re.captures(line) {
            // If there's an ongoing block, push it as a plain text block
            if !current_block.is_empty() {
                blocks.push(Block::Markdown(current_block.clone()));
                current_block.clear();
            }
            // Push the header as (level, text)
            let level = captures[1].len().min(6) as u8;
            let text = captures[2].to_string();
            blocks.push(Block::Header(level, text));
        } else {
            // Accumulate lines that are not headers
            if !current_block.is_empty() {
                current_block.push('\n');
            }
            current_block.push_str(line);
        }
    }

    // Push the final block if there's remaining content
    if !current_block.is_empty() {
        blocks.push(Block::Markdown(current_block));
    }

    blocks
}

// Just so that we don't miss an `index += 1`.
struct SendTracker<'a, 'b> {
    width: u16,
    index: SourceID,
    tx: &'a Sender<WidthEvent<'b>>,
}

impl<'b> SendTracker<'_, 'b> {
    fn send_parse(&mut self, source: WidgetSourceData<'b>, height: u16) -> Result<(), Error> {
        self.send_event(Event::Parsed(WidgetSource {
            id: self.index,
            height,
            source,
        }))
    }
    fn send_event(&mut self, ev: Event<'b>) -> Result<(), Error> {
        self.tx.send((self.width, ev))?;
        self.index += 1;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use core::panic;

    use crate::*;

    fn events_to_lines(event_rx: Receiver<(u16, Event<'_>)>) -> Vec<Line> {
        let mut lines = vec![];
        for (_, ev) in event_rx {
            match ev {
                Event::Parsed(source) => match source.source {
                    WidgetSourceData::Line(line) => {
                        lines.push(line);
                    }
                    _ => panic!("expected Line"),
                },
                Event::ParseHeader(_, _, spans) => {
                    lines.push(Line::from(format!("# {}", Line::from(spans))));
                }
                _ => {}
            }
        }
        lines
    }

    fn text_to_lines(text: &str) -> Result<Vec<Line>, Error> {
        const TERM_WIDTH: u16 = 80;
        let (event_tx, event_rx) = mpsc::channel::<(u16, Event)>();
        parse(text, TERM_WIDTH, &event_tx)?;
        drop(event_tx);
        let lines = events_to_lines(event_rx);
        Ok(lines)
    }

    fn s(content: &str) -> Span {
        Span::from(content)
    }

    #[macro_export]
    macro_rules! assert_lines_eq {
        ($left:expr, $right:expr $(,)?) => {
            {
                use ratatui::text::Line;

                // Extract text content
                let left_text: Vec<String> = $left.iter().map(|line| line.spans.iter().map(|span| span.content.clone()).collect::<String>()).collect();
                let right_text: Vec<String> = $right.iter().map(|line| line.spans.iter().map(|span| span.content.clone()).collect::<String>()).collect();

                // Compare styles
                let left_styles: Vec<Vec<_>> = $left.iter().map(|line| line.spans.iter().map(|span| span.style).collect()).collect();
                let right_styles: Vec<Vec<_>> = $right.iter().map(|line| line.spans.iter().map(|span| span.style).collect()).collect();

                if left_styles != right_styles {
                    if left_text != right_text {
                        panic!(
                            "Text content differs:\nLeft:\n{:#?}\n\nRight:\n{:#?}",
                            left_text, right_text
                        );
                    }
                    panic!(
                        "Styles differ:\nLeft:\n{:#?}\n\nRight:\n{:#?}\n\nFull Left:\n{:#?}\n\nFull Right:\n{:#?}",
                        left_styles, right_styles, $left, $right
                    );
                }
                // Compare text content
                if left_text != right_text {
                    panic!(
                        "Text content differs:\nLeft:\n{:#?}\n\nRight:\n{:#?}",
                        left_text, right_text
                    );
                }

            }
        };
    }

    #[test]
    fn test_simple_bold() -> Result<(), Error> {
        let lines = text_to_lines("Some **bold** and *italics* and `c0de`.")?;

        assert_lines_eq!(
            vec![Line::from(vec![
                s("Some "),
                s("bold").bold(),
                s(" and "),
                s("italics").italic(),
                s(" and "),
                s("c0de").on_dark_gray(),
                s(".")
            ]),],
            lines,
        );
        Ok(())
    }

    #[test]
    fn test_nested() -> Result<(), Error> {
        let lines = text_to_lines("*YES!* You can have **cooked *and* fried** widgets!")?;

        assert_lines_eq!(
            vec![Line::from(vec![
                s("YES!").italic(),
                s(" You can have "),
                s("cooked ").bold(),
                s("and").bold().italic(),
                s(" fried").bold(),
                s(" widgets!"),
            ]),],
            lines,
        );
        Ok(())
    }

    #[test]
    fn test_nested_code() -> Result<(), Error> {
        let lines = text_to_lines("**bold surrounding `code`**")?;

        assert_lines_eq!(
            vec![Line::from(vec![
                s("bold surrounding ").bold(),
                s("code").on_dark_gray(),
            ]),],
            lines,
        );
        Ok(())
    }
}
