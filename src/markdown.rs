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

    let mut index = 0;
    for edge in root.traverse() {
        match edge {
            NodeEdge::Start(node) => {
                let node_value = &node.data.borrow().value;
                if let Some(modifier) = modifier(node_value) {
                    style = style.add_modifier(modifier);
                }
                if let NodeValue::Code(_) = node.data.borrow().value {
                    style = style.on_dark_gray();
                }
            }
            NodeEdge::End(node) => {
                match node.data.borrow().value {
                    NodeValue::Text(ref literal) => {
                        let span = Span::from(literal.clone()).style(style);
                        spans.push(span);
                    }
                    NodeValue::Heading(ref tier) => {
                        tx.send((id, Event::ParseHeader(index, tier.level, spans)))?;
                        index += 1;
                        spans = vec![];
                    }
                    NodeValue::Image(ref link) => {
                        tx.send((
                            id,
                            Event::ParseImage(index, link.url.clone(), link.title.clone()),
                        ))?;
                        index += 1;
                        spans = vec![];
                    }
                    NodeValue::Paragraph => {
                        let mut wrapped_lines = wrap_spans(spans, width as usize);
                        wrapped_lines.push(Line::default());
                        for line in wrapped_lines {
                            let text = Text::from(line);
                            let height = text.height() as u16;
                            tx.send((
                                id,
                                Event::Parsed(WidgetSource {
                                    index,
                                    height,
                                    source: WidgetSourceData::Text(text),
                                }),
                            ))?;
                            index += 1;
                        }
                        spans = vec![];
                    }
                    NodeValue::LineBreak | NodeValue::SoftBreak => {
                        let wrapped_lines = wrap_spans(spans, width as usize);
                        for line in wrapped_lines {
                            let text = Text::from(line);
                            let height = text.height() as u16;
                            tx.send((
                                id,
                                Event::Parsed(WidgetSource {
                                    index,
                                    height,
                                    source: WidgetSourceData::Text(text),
                                }),
                            ))?;
                            index += 1;
                        }
                        spans = vec![];
                    }
                    NodeValue::Code(ref node_code) => {
                        let span = Span::from(node_code.literal.clone()).style(style);
                        spans.push(span);
                    }
                    _ => {
                        if let Some(modifier) = modifier(&node.data.borrow().value) {
                            style = style.remove_modifier(modifier);
                        }
                    }
                }
                style.bg = None;
            }
        }
    }
    Ok(())
}

fn modifier(node_value: &NodeValue) -> Option<Modifier> {
    match node_value {
        NodeValue::Strong => Some(Modifier::BOLD),
        NodeValue::Emph => Some(Modifier::ITALIC),
        NodeValue::Strikethrough => Some(Modifier::CROSSED_OUT),
        _ => None,
    }
}

// This probably has bugs and doesn't handle multi-width characters properly.
pub fn wrap_spans(spans: Vec<Span>, max_width: usize) -> Vec<Line> {
    let mut result_lines = Vec::new();
    let mut current_line = Vec::new();
    let mut current_line_width = 0;

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
            let word_to_add = if current_line_width > 0 {
                format!(" {}", word)
            } else {
                word.to_string()
            };

            current_line_width += word_to_add.len();
            current_line.push(Span::styled(word_to_add, span.style));
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
