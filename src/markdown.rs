use comrak::{arena_tree::NodeEdge, nodes::NodeValue};
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span, Text},
};
use reqwest::blocking::Client;

use crate::{
    widget_sources::{header_source, image_source, WidgetSourceData},
    Model, WidgetSource,
};

pub fn traverse<'a>(model: &mut Model<'a>, width: u16) -> Vec<WidgetSource<'a>> {
    let mut debug = vec![];
    let mut spans = vec![];
    let mut style = Style::new();

    let mut sources: Vec<WidgetSource<'a>> = vec![];

    let mut client = Client::new();

    for edge in model.root.traverse() {
        match edge {
            NodeEdge::Start(node) => {
                let node_value = &node.data.borrow().value;
                if let Some(modifier) = modifier(node_value) {
                    style = style.add_modifier(modifier);
                }
            }
            NodeEdge::End(node) => {
                debug.push(Line::from(format!("End {:?}", node.data.borrow().value)));
                match node.data.borrow().value {
                    NodeValue::Text(ref literal) => {
                        let span = Span::from(literal.clone()).style(style);
                        spans.push(span);
                    }
                    NodeValue::Heading(ref tier) => {
                        let source = header_source(
                            &mut model.picker,
                            &mut model.font,
                            model.bg,
                            width,
                            spans,
                            tier.level,
                            model.deep_fry,
                        )
                        .unwrap(); // TODO don't
                        sources.push(source);
                        spans = vec![];
                    }
                    NodeValue::Image(ref link) => {
                        match image_source(
                            &mut model.picker,
                            width,
                            model.basepath,
                            &mut client,
                            link.url.as_str(),
                            model.deep_fry,
                        ) {
                            Ok(source) => {
                                sources.push(source);
                            }
                            Err(err) => {
                                let text = Text::from(format!("[Image error: {err:?}]"));
                                let height = text.height() as u16;
                                sources.push(WidgetSource {
                                    height,
                                    source: WidgetSourceData::Text(text),
                                });
                            }
                        }
                        spans = vec![];
                    }
                    NodeValue::Paragraph => {
                        let mut wrapped_lines = wrap_spans(spans, width as usize);
                        wrapped_lines.push(Line::default());
                        for line in wrapped_lines {
                            let text = Text::from(line);
                            let height = text.height() as u16;
                            sources.push(WidgetSource {
                                height,
                                source: WidgetSourceData::Text(text),
                            });
                        }
                        spans = vec![];
                    }
                    NodeValue::LineBreak | NodeValue::SoftBreak => {
                        let wrapped_lines = wrap_spans(spans, width as usize);
                        for line in wrapped_lines {
                            let text = Text::from(line);
                            let height = text.height() as u16;
                            sources.push(WidgetSource {
                                height,
                                source: WidgetSourceData::Text(text),
                            });
                        }
                        spans = vec![];
                    }
                    _ => {
                        if let Some(modifier) = modifier(&node.data.borrow().value) {
                            style = style.remove_modifier(modifier);
                        }
                    }
                }
            }
        }
    }

    sources
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
