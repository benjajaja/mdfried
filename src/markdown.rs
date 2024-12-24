use std::sync::mpsc::Sender;

use comrak::{
    arena_tree::NodeEdge, nodes::NodeValue, parse_document, Arena, ExtensionOptions, Options,
};
use ratatui::{
    style::{Modifier, Style, Stylize},
    text::{Line, Span},
};

use crate::{
    widget_sources::WidgetSourceData, wordwrap::wrap_spans, Error, Event, WidgetSource, WidthEvent,
};

pub async fn parse<'b>(text: &str, width: u16, tx: &Sender<WidthEvent<'b>>) -> Result<(), Error> {
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
    let mut style = Style::new();

    let mut sender = SendTracker {
        width,
        tx,
        index: 0,
    };
    for edge in root.traverse() {
        match edge {
            NodeEdge::Start(node) => {
                let node_value = &node.data.borrow().value;
                style = modifier(style, node_value);
            }
            NodeEdge::End(node) => {
                let node_value = &node.data.borrow().value;
                match node_value {
                    NodeValue::Text(ref literal) => {
                        let span = Span::from(literal.clone()).style(style);
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
                        let wrapped_lines = wrap_spans(spans, width as usize);
                        for line in wrapped_lines {
                            sender.send_parse(WidgetSourceData::Line(line), 1)?;
                        }
                        sender.send_parse(WidgetSourceData::Line(Line::default()), 1)?;
                        spans = vec![];
                    }
                    NodeValue::LineBreak | NodeValue::SoftBreak => {
                        let wrapped_lines = wrap_spans(spans, width as usize);
                        for line in wrapped_lines {
                            sender.send_parse(WidgetSourceData::Line(line), 1)?;
                        }
                        spans = vec![];
                    }
                    NodeValue::Link(ref link) => {
                        let inner = Line::from(spans);
                        let span = Span::from(format!("[{}]({})", inner, link.url))
                            .style(modifier(style, node_value));
                        spans = vec![span];
                    }
                    NodeValue::Code(ref code) => {
                        let span = Span::from(code.literal.clone()).style(style);
                        spans.push(span);
                    }
                    NodeValue::CodeBlock(ref codeblock) => {
                        let mut splits: Vec<&str> = codeblock.literal.split("\n").collect();
                        if splits.last().map_or(false, |s| s.is_empty()) {
                            splits.pop();
                        }
                        for line in splits {
                            let line = Line::from(line.to_string()).style(style).on_dark_gray();
                            sender.send_parse(WidgetSourceData::CodeBlock(line), 1)?;
                        }
                        sender.send_parse(WidgetSourceData::Line(Line::default()), 1)?;
                        spans = vec![];
                    }
                    _ => {}
                }
                style = Style::default();
            }
        }
    }
    Ok(())
}

// Just so that we don't miss an `index += 1`.
struct SendTracker<'a, 'b> {
    width: u16,
    index: usize,
    tx: &'a Sender<WidthEvent<'b>>,
}

impl<'b> SendTracker<'_, 'b> {
    fn send_parse(&mut self, source: WidgetSourceData<'b>, height: u16) -> Result<(), Error> {
        self.send_event(Event::Parsed(WidgetSource {
            index: self.index,
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

fn modifier(style: Style, node_value: &NodeValue) -> Style {
    match node_value {
        NodeValue::Strong => style.add_modifier(Modifier::BOLD),
        NodeValue::Emph => style.add_modifier(Modifier::ITALIC),
        NodeValue::Strikethrough => style.add_modifier(Modifier::CROSSED_OUT),
        NodeValue::Code(_) | NodeValue::CodeBlock(_) => style.on_dark_gray(),
        NodeValue::Link(_) => style.blue().underlined(),
        _ => style,
    }
}
