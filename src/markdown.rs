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
    List(NodeList, usize),
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
    let mut node_stack: Vec<Block> = vec![];

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
                #[allow(clippy::single_match)]
                match node_value {
                    NodeValue::List(ref nodelist) => {
                        node_stack.push(Block::List(*nodelist, 0));
                    }
                    _ => {}
                }
                style_stack.push(new_style);
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
                    NodeValue::Paragraph => {
                        let mut is_list = false;

                        // TODO: should be done in the `if` block, fight the borrow checker.
                        let indent = node_stack
                            .iter()
                            .filter(|block| matches!(block, Block::List(_, _)))
                            .collect::<Vec<_>>()
                            .len();

                        if let Some(Block::List(nodelist, i)) = node_stack.last_mut() {
                            is_list = true;
                            let mut prefix = if indent > 0 {
                                // This "zero-width space" just keeps wrap_spans from
                                // collapsing whitespace, but this should be fixed there not
                                // here.
                                vec![Span::from("\u{200B} ".repeat(indent))]
                            } else {
                                vec![]
                            };
                            prefix.extend(match nodelist.list_type {
                                ListType::Bullet => {
                                    let char: char = nodelist.bullet_char.into();
                                    vec![Span::from(String::from(char)).yellow(), Span::from(" ")]
                                }
                                ListType::Ordered => {
                                    vec![
                                        Span::from((nodelist.start + *i).to_string()).yellow(),
                                        (match nodelist.delimiter {
                                            ListDelimType::Period => Span::from(". "),
                                            ListDelimType::Paren => Span::from(") "),
                                        })
                                        .dark_gray(),
                                    ]
                                }
                            });
                            prefix.extend(spans);
                            spans = prefix;
                            *i += 1;
                        };
                        let wrapped_lines = wrap_spans(spans, width as usize)?;
                        for line in wrapped_lines {
                            sender.send_parse(WidgetSourceData::Line(line), 1)?;
                        }

                        if !is_list {
                            sender.send_parse(WidgetSourceData::Line(Line::default()), 1)?;
                        }
                        spans = vec![];
                    }
                    NodeValue::LineBreak | NodeValue::SoftBreak => {
                        let wrapped_lines = wrap_spans(spans, width as usize)?;
                        for line in wrapped_lines {
                            sender.send_parse(WidgetSourceData::Line(line), 1)?;
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
                    NodeValue::List(_) => {
                        debug_assert!(matches!(node_stack.last().unwrap(), Block::List(_, _)));
                        node_stack.pop();
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

    #[test]
    fn test_simple_bold() -> Result<(), Error> {
        let lines = text_to_lines("Some **bold** and _italics_ and `c0de`.")?;

        assert_eq!(
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

        assert_eq!(
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

        assert_eq!(
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
}
