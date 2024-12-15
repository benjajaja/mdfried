use std::sync::mpsc::Sender;

use comrak::{
    arena_tree::NodeEdge, nodes::NodeValue, parse_document, Arena, ExtensionOptions, Options,
};
use ratatui::{
    style::{Modifier, Style, Stylize},
    text::{Line, Span, Text},
};

use crate::{widget_sources::WidgetSourceData, Error, Event, WidgetSource, WidthEvent};

pub async fn parse<'b>(
    text: &str,
    width: u16,
    tx: &Sender<WidthEvent<'b>>,
    id: u16,
) -> Result<(), Error> {
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

    let mut sender = SendTracker { id, tx, index: 0 };
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
                            let text = Text::from(line);
                            let height = text.height() as u16;
                            sender.send_parsed(WidgetSourceData::Text(text), height)?;
                        }
                        sender.send_parsed(WidgetSourceData::Text(Text::default()), 1)?;
                        spans = vec![];
                    }
                    NodeValue::LineBreak | NodeValue::SoftBreak => {
                        let wrapped_lines = wrap_spans(spans, width as usize);
                        for line in wrapped_lines {
                            let text = Text::from(line);
                            let height = text.height() as u16;
                            sender.send_parsed(WidgetSourceData::Text(text), height)?;
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
                            let text = Text::from(Line::from(line.to_string())).style(style);
                            let height = text.height() as u16;
                            sender.send_parsed(WidgetSourceData::CodeBlock(text), height)?;
                        }
                        sender.send_parsed(WidgetSourceData::Text(Text::default()), 1)?;
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
    id: u16,
    index: usize,
    tx: &'a Sender<WidthEvent<'b>>,
}

impl<'a, 'b> SendTracker<'a, 'b> {
    fn send_parsed(&mut self, source: WidgetSourceData<'b>, height: u16) -> Result<(), Error> {
        self.send_event(Event::Parsed(WidgetSource {
            index: self.index,
            height,
            source,
        }))
    }
    fn send_event(&mut self, ev: Event<'b>) -> Result<(), Error> {
        self.tx.send((self.id, ev))?;
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

// This probably has bugs and doesn't handle multi-width characters properly. Generated with AI.
pub fn wrap_spans(spans: Vec<Span>, max_width: usize) -> Vec<Line> {
    let mut result_lines = Vec::new();
    let mut current_line = Vec::new();
    let mut current_line_width = 0;
    let mut current_style = Style::default();

    // Helper function to trim leading whitespace
    fn trim_leading_whitespace(s: &str) -> &str {
        s.trim_start()
    }

    for span in spans {
        // Split the span content into words
        let words: Vec<&str> = span.content.split_whitespace().collect();

        for word in words {
            let word_width = word.len();

            // If adding this word would exceed max width, start a new line
            if current_line_width + word_width + (if current_line_width > 0 { 1 } else { 0 })
                > max_width
            {
                // Finalize and add current line if not empty
                if !current_line.is_empty() {
                    result_lines.push(Line::from(current_line));
                    current_line = Vec::new();
                    current_line_width = 0;
                }
            }

            // Add word to current line (with space if not first word)
            if current_line_width > 0 {
                if span.style == current_style {
                    current_line.push(Span::style(" ".into(), current_style));
                } else {
                    current_line.push(Span::from(" "));
                }
            }
            current_line_width += word.len();
            current_line.push(Span::styled(word.to_string(), span.style));
            current_style = span.style;
        }
    }

    // Add any remaining line
    if !current_line.is_empty() {
        result_lines.push(Line::from(current_line));
    }

    // Remove leading whitespace from each line
    result_lines.iter_mut().for_each(|line| {
        if let Some(first_span) = line.spans.first_mut() {
            first_span.content = trim_leading_whitespace(&first_span.content)
                .to_string()
                .into();
        }
    });

    result_lines
}
