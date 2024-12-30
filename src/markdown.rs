use std::sync::mpsc::Sender;

use comrak::{
    arena_tree::NodeEdge,
    nodes::{ListDelimType, ListType, NodeList, NodeValue},
    parse_document, Arena, ExtensionOptions, Options,
};
use ratatui::{
    style::{Modifier, Style, Stylize},
    text::{Line, Span},
};

use crate::{
    widget_sources::{SourceID, WidgetSourceData},
    wordwrap::wrap_spans,
    Error, Event, WidgetSource, WidthEvent,
};

enum Block {
    ListItem(ListItem),
    BlockQuote(bool),
}

struct ListItem {
    nodelist: NodeList,
    prefix_consumed: bool,
}

impl Block {
    fn insert_prefix_spans(&mut self, offset: usize, lines: &mut Vec<Line>) {
        for (i, line) in lines.iter_mut().enumerate() {
            line.spans.splice(offset..offset, self.prefix(i));
        }
    }
    fn insert_offset_spans(&mut self, offset: usize, lines: &mut Vec<Line>) {
        for line in lines {
            line.spans.splice(offset..offset, self.offset());
        }
    }
    fn width(&self) -> usize {
        match self {
            Self::ListItem(item) => match item.nodelist.list_type {
                ListType::Bullet => 2,
                ListType::Ordered => 3,
            },
            Self::BlockQuote(_) => 2,
        }
    }
    fn prefix<'a>(&mut self, line_index: usize) -> Vec<Span<'a>> {
        match self {
            Self::ListItem(item) => match item.nodelist.list_type {
                ListType::Bullet => {
                    let char: char = item.nodelist.bullet_char.into();
                    vec![Span::from(String::from(char)).yellow(), Span::from(" ")]
                }
                ListType::Ordered => {
                    if line_index == 0 && !item.prefix_consumed {
                        item.prefix_consumed = true;
                        vec![
                            Span::from((item.nodelist.start).to_string()).yellow(),
                            (match item.nodelist.delimiter {
                                ListDelimType::Period => Span::from("."),
                                ListDelimType::Paren => Span::from(")"),
                            })
                            .dark_gray(),
                            Span::from(" "),
                        ]
                    } else {
                        vec![Span::from("   ")]
                    }
                }
            },
            Self::BlockQuote(consumed) => {
                if *consumed {
                    vec![Span::from("\u{2502} ")] // vertical line
                } else {
                    *consumed = true;
                    // TODO: I would like to render this as ❝ \U{275D} Heavy double comma
                    // quotation, with an image like the headings.
                    vec![Span::from("\u{2502} ")]
                }
            }
        }
    }
    fn offset<'a>(&mut self) -> Vec<Span<'a>> {
        match self {
            Self::ListItem(_) => vec![Span::from(" ".repeat(self.width()))],
            Self::BlockQuote(_) => vec![Span::from("\u{2502} ")], // should also be able to render
                                                                  // the decoration?
        }
    }
}

fn insert_prefix_spans(block_stack: &mut [Block], wrapped_lines: &mut Vec<Line>) {
    let mut peek = block_stack.iter_mut().peekable();
    let mut inserted_span_count = 0;
    while let Some(block) = peek.next() {
        if peek.peek().is_some() {
            // for line in wrapped_lines.iter_mut() {
            // line.spans.insert(0, Span::from(" ".repeat(block.width())));
            // }
            block.insert_offset_spans(inserted_span_count, wrapped_lines);
            inserted_span_count += 1;
        } else {
            block.insert_prefix_spans(inserted_span_count, wrapped_lines);
        }
    }
}

pub fn parse(text: &str, width: u16, tx: &Sender<WidthEvent>) -> Result<(), Error> {
    let mut ext_options = ExtensionOptions::default();
    ext_options.strikethrough = true;

    let arena = Arena::new();

    let root = parse_document(
        &arena,
        text,
        &Options {
            extension: ext_options,
            ..Default::default()
        },
    );

    let mut spans = vec![];
    let mut style_stack = vec![Style::new()];
    let mut block_stack: Vec<Block> = vec![];

    let mut sender = SendTracker {
        width,
        tx,
        index: 0,
    };
    for edge in root.traverse() {
        match edge {
            NodeEdge::Start(node) => {
                let node_value = &node.data.borrow().value;
                let node_style = modifier(node_value);
                let new_style = match node_value {
                    NodeValue::Code(_) | NodeValue::CodeBlock(_) => node_style,
                    _ => (*style_stack.last().unwrap()).patch(node_style),
                };
                style_stack.push(new_style);

                #[allow(clippy::single_match)]
                match node_value {
                    NodeValue::Item(ref nodelist) => {
                        debug_assert!(spans.is_empty());
                        block_stack.push(Block::ListItem(ListItem {
                            nodelist: *nodelist,
                            prefix_consumed: false,
                        }));
                    }
                    NodeValue::BlockQuote => {
                        debug_assert!(spans.is_empty());
                        block_stack.push(Block::BlockQuote(false));
                    }
                    _ => {}
                }
            }
            NodeEdge::End(node) => {
                let node_value = &node.data.borrow().value;
                match node_value {
                    NodeValue::Text(ref literal) => {
                        let span = Span::from(literal.clone()).style(*style_stack.last().unwrap());
                        spans.push(span);
                    }
                    NodeValue::Heading(ref tier) => {
                        sender.send_event(Event::ParseHeader(sender.index, tier.level, spans))?;
                        spans = vec![];
                    }
                    NodeValue::Image(ref link) => {
                        let text = if spans.len() == 1 {
                            spans.first().map(|s| s.to_string())
                        } else {
                            None
                        }
                        .unwrap_or("".to_string());

                        sender.send_event(Event::ParseImage(
                            sender.index,
                            link.url.clone(),
                            text,
                            link.title.clone(),
                        ))?;
                        spans = vec![];
                    }
                    NodeValue::Paragraph | NodeValue::LineBreak | NodeValue::SoftBreak => {
                        let total_indent: usize = block_stack.iter().map(Block::width).sum();

                        let mut wrapped_lines = wrap_spans(spans, width as usize - total_indent)?;

                        insert_prefix_spans(&mut block_stack, &mut wrapped_lines);

                        for line in wrapped_lines {
                            sender.send_parse(WidgetSourceData::Line(line), 1)?;
                        }

                        if matches!(node_value, NodeValue::Paragraph) {
                            if block_stack.last().is_none() {
                                sender.send_parse(WidgetSourceData::Line(Line::default()), 1)?;
                            } else {
                                // let mut lines = vec![Line::from("")];
                                // insert_prefix_spans(&mut block_stack, &mut lines);
                                // let line = lines.remove(0);
                                // sender.send_parse(WidgetSourceData::Line(line), 1)?;
                            }
                        }
                        spans = vec![];
                    }
                    NodeValue::Link(ref link) => {
                        let inner = Line::from(spans);
                        spans = vec![
                            Span::from("[").dark_gray(),
                            Span::from(inner.to_string()).underlined(),
                            Span::from("](").dark_gray(),
                            Span::from(link.url.clone()).blue().underlined(),
                            Span::from(")").dark_gray(),
                        ];
                    }
                    NodeValue::Code(ref code) => {
                        let span =
                            Span::from(code.literal.clone()).style(*style_stack.last().unwrap());
                        spans.push(span);
                    }
                    NodeValue::CodeBlock(ref codeblock) => {
                        let mut splits: Vec<&str> = codeblock.literal.split("\n").collect();
                        if splits.last().map_or(false, |s| s.is_empty()) {
                            splits.pop();
                        }
                        for line in splits {
                            let line = Line::from(line.to_string())
                                .style(*style_stack.last().unwrap())
                                .on_dark_gray();
                            sender.send_parse(WidgetSourceData::CodeBlock(line), 1)?;
                        }
                        sender.send_parse(WidgetSourceData::Line(Line::default()), 1)?;
                        spans = vec![];
                    }
                    NodeValue::Item(_) => {
                        debug_assert!(matches!(block_stack.last().unwrap(), Block::ListItem(_)));
                        block_stack.pop();
                    }
                    NodeValue::BlockQuote => {
                        debug_assert!(matches!(block_stack.last().unwrap(), Block::BlockQuote(_)));
                        block_stack.pop();
                    }
                    _ => {}
                }
                style_stack.pop();
                debug_assert!(!style_stack.is_empty());
            }
        }
    }
    Ok(())
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

fn modifier(node_value: &NodeValue) -> Style {
    let style = Style::default();
    match node_value {
        NodeValue::Strong => style.bold(),
        NodeValue::Emph => style.italic(),
        NodeValue::Strikethrough => style.add_modifier(Modifier::CROSSED_OUT),
        NodeValue::Code(_) | NodeValue::CodeBlock(_) => style.on_dark_gray(),
        _ => style,
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
        let lines = text_to_lines("Some **bold** and _italics_ and `c0de`.")?;

        assert_lines_eq!(
            vec![
                Line::from(vec![
                    s("Some "),
                    s("bold").bold(),
                    s(" and "),
                    s("italics").italic(),
                    s(" and "),
                    s("c0de").on_dark_gray(),
                    s(".")
                ]),
                Line::default(),
            ],
            lines,
        );
        Ok(())
    }

    #[test]
    fn test_nested() -> Result<(), Error> {
        let lines = text_to_lines("_YES!_ You can have **cooked _and_ fried** widgets!")?;

        assert_lines_eq!(
            vec![
                Line::from(vec![
                    s("YES!").italic(),
                    s(" You can have "),
                    s("cooked ").bold(),
                    s("and").bold().italic(),
                    s(" fried").bold(),
                    s(" widgets!"),
                ]),
                Line::default(),
            ],
            lines,
        );
        Ok(())
    }

    #[test]
    fn test_nested_code() -> Result<(), Error> {
        let lines = text_to_lines("**bold surrounding `code`**")?;

        assert_lines_eq!(
            vec![
                Line::from(vec![
                    s("bold surrounding ").bold(),
                    s("code").on_dark_gray(),
                ]),
                Line::default(),
            ],
            lines,
        );
        Ok(())
    }

    #[test]
    fn test_list() -> Result<(), Error> {
        let lines = text_to_lines("1. one")?;

        assert_lines_eq!(
            vec![Line::from(vec![
                s("1").yellow(),
                s(".").dark_gray(),
                s(" "),
                s("one")
            ]),],
            lines,
        );
        Ok(())
    }

    #[test]
    fn test_indented_list() -> Result<(), Error> {
        let lines = text_to_lines("1. one\n   - subitem\n     - subsubitem\n2. two")?;

        assert_lines_eq!(
            vec![
                Line::from(vec![s("1").yellow(), s(".").dark_gray(), s(" "), s("one")]),
                Line::from(vec![s("   "), s("-").yellow(), s(" "), s("subitem"),]),
                Line::from(vec![
                    s("   "),
                    s("  "),
                    s("-").yellow(),
                    s(" "),
                    s("subsubitem"),
                ]),
                Line::from(vec![s("2").yellow(), s(".").dark_gray(), s(" "), s("two")]),
            ],
            lines,
        );
        Ok(())
    }

    #[test]
    #[ignore]
    fn test_list_vertical_space() -> Result<(), Error> {
        let lines = text_to_lines(
            r#"
* one

* two"#,
        )?;

        assert_lines_eq!(
            vec![
                Line::from(vec![s("*").yellow(), s(" "), s("one")]),
                Line::from(vec![s("")]),
                Line::from(vec![s("*").yellow(), s(" "), s("two")]),
            ],
            lines,
        );
        Ok(())
    }
}
