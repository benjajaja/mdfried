use std::sync::mpsc::Sender;

use crossterm::style::Attribute;
use ratatui::{
    style::Stylize,
    text::{Line, Span},
};
use regex::Regex;
use termimad::{
    minimad::parse_text, CompositeKind, CompoundStyle, FmtComposite, FmtLine, FmtText,
    ListItemsIndentationMode, MadSkin, RelativePosition, Spacing, StyledChar,
};

use crate::{
    widget_sources::{SourceID, WidgetSourceData},
    Error, Event, WidgetSource, WidthEvent,
};

// Crude "pre-parsing" of markdown by lines.
// Headers are always on a line of their own.
// Images are only processed if it appears on a line by itself, to avoid having to deal with text
// wrapping around some area.
#[derive(Debug, PartialEq)]
enum Block {
    Header(u8, String),
    Image(String, String),
    Markdown(String),
}

fn split_headers_and_images(text: &str) -> Vec<Block> {
    // Regex to match lines starting with 1-6 `#` characters
    let header_re = Regex::new(r"^(#+)\s*(.*)").unwrap();
    // Regex to match standalone image lines: ![alt](url)
    let image_re = Regex::new(r"^!\[(.*?)\]\((.*?)\)$").unwrap();

    let mut blocks = Vec::new();
    let mut current_block = String::new();

    for line in text.lines() {
        if let Some(captures) = header_re.captures(line) {
            // If there's an ongoing block, push it as a plain text block
            if !current_block.is_empty() {
                blocks.push(Block::Markdown(current_block.clone()));
                current_block.clear();
            }
            // Push the header as (level, text)
            let level = captures[1].len().min(6) as u8;
            let text = captures[2].to_string();
            blocks.push(Block::Header(level, text));
        } else if let Some(captures) = image_re.captures(line) {
            // If there's an ongoing block, push it as a plain text block
            if !current_block.is_empty() {
                blocks.push(Block::Markdown(current_block.clone()));
                current_block.clear();
            }
            // Push the image as (alt_text, url)
            let alt_text = captures[1].to_string();
            let url = captures[2].to_string();
            blocks.push(Block::Image(alt_text, url));
        } else {
            // Accumulate lines that are neither headers nor images
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

pub fn parse(text: &str, skin: &MadSkin, width: u16, tx: &Sender<WidthEvent>) -> Result<(), Error> {
    let mut sender = SendTracker {
        width,
        tx,
        index: 0,
    };

    let text_blocks = split_headers_and_images(text);

    let mut needs_space = false;
    for block in text_blocks {
        if needs_space {
            // Send a newline after Markdowns and Images, but not after the last block.
            sender.send_line(WidgetSourceData::Line(Line::default()), 1)?;
        }

        match block {
            Block::Header(tier, text) => {
                needs_space = false;
                sender.send_event(Event::ParseHeader(sender.index, tier, text))?;
            }
            Block::Image(alt, url) => {
                needs_space = true;
                sender.send_event(Event::ParseImage(sender.index, url, alt, "".to_string()))?;
            }
            Block::Markdown(text) => {
                needs_space = true;
                let text = parse_text(&text, termimad::minimad::Options::default());

                let fmt_text = FmtText::from_text(skin, text, Some(width as usize));

                for line in fmt_text.lines {
                    match line {
                        FmtLine::Normal(fmtcomp) => {
                            let spans = fmt_composite_to_spans(skin, fmtcomp, false, None, false);
                            sender.send_line(WidgetSourceData::Line(Line::from(spans)), 1)?;
                        }
                        FmtLine::HorizontalRule => {
                            sender.send_line(
                                WidgetSourceData::Line(Line::from(
                                    skin.horizontal_rule
                                        .nude_char()
                                        .to_string()
                                        .repeat(width as usize),
                                )),
                                1,
                            )?;
                        }
                        FmtLine::TableRow(fmt) => {
                            let mut spans = vec![];
                            let tbl_width = 1 + fmt.cells.iter().fold(0, |sum, cell| {
                                if let Some(spacing) = cell.spacing {
                                    sum + spacing.width + 1
                                } else {
                                    sum + cell.visible_length + 1
                                }
                            });
                            let (lpo, rpo) = Spacing::optional_completions(
                                skin.table.align,
                                tbl_width,
                                Some(width as usize),
                            );
                            spans.push(Span::from(" ".repeat(lpo)));

                            for cell in fmt.cells {
                                spans.push(compoundstyle_to_span(
                                    skin.table_border_chars.vertical.to_string(),
                                    &skin.table.compound_style,
                                ));

                                let cell_spans =
                                    fmt_composite_to_spans(skin, cell, false, None, false);
                                spans.extend(cell_spans);
                            }
                            spans.push(compoundstyle_to_span(
                                skin.table_border_chars.vertical.to_string(),
                                &skin.table.compound_style,
                            ));

                            spans.push(Span::from(" ".repeat(rpo)));

                            sender.send_line(WidgetSourceData::Line(Line::from(spans)), 1)?;
                        }
                        FmtLine::TableRule(rule) => {
                            let mut chars = String::with_capacity(width as usize);
                            let tbl_width = 1 + rule.widths.iter().fold(0, |sum, w| sum + w + 1);
                            let (lpo, rpo) = Spacing::optional_completions(
                                skin.table.align,
                                tbl_width,
                                Some(width as usize),
                            );
                            chars.push_str(&" ".repeat(lpo));

                            chars.push(match rule.position {
                                RelativePosition::Top => skin.table_border_chars.top_left_corner,
                                RelativePosition::Other => skin.table_border_chars.left_junction,
                                RelativePosition::Bottom => {
                                    skin.table_border_chars.bottom_left_corner
                                }
                            });

                            for (idx, &width) in rule.widths.iter().enumerate() {
                                if idx > 0 {
                                    chars.push(match rule.position {
                                        RelativePosition::Top => {
                                            skin.table_border_chars.top_junction
                                        }
                                        RelativePosition::Other => skin.table_border_chars.cross,
                                        RelativePosition::Bottom => {
                                            skin.table_border_chars.bottom_junction
                                        }
                                    });
                                }
                                chars.push_str(
                                    &skin.table_border_chars.horizontal.to_string().repeat(width),
                                );
                            }

                            chars.push(match rule.position {
                                RelativePosition::Top => skin.table_border_chars.top_right_corner,
                                RelativePosition::Other => skin.table_border_chars.right_junction,
                                RelativePosition::Bottom => {
                                    skin.table_border_chars.bottom_right_corner
                                }
                            });
                            chars.push_str(&" ".repeat(rpo));

                            let mut span = Span::from(chars);
                            span = style_to_span(&skin.table.compound_style, span);
                            sender.send_line(WidgetSourceData::Line(Line::from(span)), 1)?;
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

// This is duplicated from MadSkin::write_fmt_composite, but with ratatui Spans.
fn fmt_composite_to_spans<'a>(
    skin: &MadSkin,
    fc: FmtComposite<'_>,
    with_margins: bool,
    outer_width: Option<usize>,
    with_right_completion: bool,
) -> Vec<Span<'a>> {
    let mut spans = vec![];

    let ls = skin.line_style(fc.kind);
    let (left_margin, right_margin) = if with_margins {
        ls.margins_in(outer_width)
    } else {
        (0, 0)
    };
    let (lpi, rpi) = fc.completions(); // inner completion
    let inner_width = fc.spacing.map_or(fc.visible_length, |sp| sp.width);
    let (lpo, rpo) = Spacing::optional_completions(
        ls.align,
        inner_width + left_margin + right_margin,
        outer_width,
    );
    spans.push(space(skin, lpo + left_margin));
    spans.push(compoundstyle_to_span(
        " ".repeat(lpi),
        &skin.line_style(fc.kind).compound_style,
    ));

    if let CompositeKind::ListItem(depth) = fc.kind {
        spans.push(space(skin, depth as usize));
        spans.push(styled_char_to_span(&skin.bullet));
        spans.push(space(skin, 1));
    }
    if skin.list_items_indentation_mode == ListItemsIndentationMode::Block {
        if let CompositeKind::ListItemFollowUp(depth) = fc.kind {
            spans.push(space(skin, (depth + 2) as usize));
        }
    }
    if fc.kind == CompositeKind::Quote {
        spans.push(styled_char_to_span(&skin.quote_mark));
        spans.push(space(skin, 1));
    }
    // #[cfg(feature = "special-renders")]
    // for c in &fmtcomp.compounds {
    // if let Some(replacement) = skin.special_chars.get(c) {
    // spans.push(styled_char_to_span(replacement));
    // } else {
    // let os = skin.compound_style(ls, c);
    // comp_style_to_span(c.as_str().to_string(), &os);
    // }
    // }
    // #[cfg(not(feature = "special-renders"))]
    for c in &fc.compounds {
        let os = skin.compound_style(ls, c);
        spans.push(compoundstyle_to_span(c.as_str().to_string(), &os));
    }
    spans.push(space(skin, rpi));
    if with_right_completion {
        spans.push(space(skin, rpo + right_margin));
    }
    spans
}

fn space<'a>(skin: &MadSkin, repeat: usize) -> Span<'a> {
    style_to_span(
        &skin.paragraph.compound_style,
        Span::from(" ".repeat(repeat)),
    )
}

fn styled_char_to_span<'a>(ch: &StyledChar) -> Span<'a> {
    style_to_span(ch.compound_style(), Span::from(ch.nude_char().to_string()))
}

// Make a ratatui Span from a termimad Compound, using the skin.
fn compoundstyle_to_span<'a>(src: String, style: &CompoundStyle) -> Span<'a> {
    style_to_span(style, Span::from(src))
}

// Convert from crossterm style to ratatui generic style, and set it on the span.
fn style_to_span<'a>(style: &CompoundStyle, mut span: Span<'a>) -> Span<'a> {
    if let Some(color) = style.object_style.foreground_color {
        span = span.fg(color);
    }
    if let Some(color) = style.object_style.background_color {
        span = span.bg(color);
    }
    if style.object_style.attributes.has(Attribute::Underlined) {
        span = span.underlined();
    }
    if style.object_style.attributes.has(Attribute::Bold) {
        span = span.bold();
    }
    if style.object_style.attributes.has(Attribute::Italic) {
        span = span.italic();
    }
    if style.object_style.attributes.has(Attribute::CrossedOut) {
        span = span.crossed_out();
    }
    span
}

// Just so that we don't miss an `index += 1`.
struct SendTracker<'a, 'b> {
    width: u16,
    index: SourceID,
    tx: &'a Sender<WidthEvent<'b>>,
}

impl<'b> SendTracker<'_, 'b> {
    fn send_line(&mut self, source: WidgetSourceData<'b>, height: u16) -> Result<(), Error> {
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
    use termimad::MadSkin;

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
        let mut skin = MadSkin::default();
        skin.inline_code.set_bg(crossterm::style::Color::DarkGrey);
        skin.inline_code.object_style.foreground_color = None;

        parse(text, &skin, TERM_WIDTH, &event_tx)?;
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
    fn test_split_headers_and_images() {
        let blocks = markdown::split_headers_and_images(
            r#"
# header

paragraph

paragraph

# header

paragraph
paragraph

# header

paragraph

# header
"#,
        );
        assert_eq!(
            blocks,
            vec![
                markdown::Block::Header(1, "header".to_string()),
                markdown::Block::Markdown("paragraph\n\nparagraph\n".to_string()),
                markdown::Block::Header(1, "header".to_string()),
                markdown::Block::Markdown("paragraph\nparagraph\n".to_string()),
                markdown::Block::Header(1, "header".to_string()),
                markdown::Block::Markdown("paragraph\n".to_string()),
                markdown::Block::Header(1, "header".to_string()),
            ]
        );
    }

    #[test]
    fn test_split_headers_and_images_without_space() {
        let blocks = markdown::split_headers_and_images(
            r#"
# header
paragraph
# header
# header
paragraph
# header
"#,
        );
        assert_eq!(6, blocks.len());
        assert_eq!(
            blocks,
            vec![
                markdown::Block::Header(1, "header".to_string()),
                markdown::Block::Markdown("paragraph".to_string()),
                markdown::Block::Header(1, "header".to_string()),
                markdown::Block::Header(1, "header".to_string()),
                markdown::Block::Markdown("paragraph".to_string()),
                markdown::Block::Header(1, "header".to_string()),
            ]
        );
    }
}
