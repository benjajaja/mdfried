use std::{
    cell::RefCell,
    path::PathBuf,
    sync::{mpsc::Sender, Arc, RwLock},
};

use comrak::{
    arena_tree::{Node, NodeEdge},
    nodes::{Ast, NodeValue},
    parse_document, Arena, ExtensionOptions, Options,
};
use ratatui::{
    style::{Modifier, Style, Stylize},
    text::{Line, Span, Text},
};
use ratatui_image::picker::Picker;
use reqwest::blocking::Client;
use rusttype::Font;

use crate::{
    widget_sources::{header_source, image_source, WidgetSourceData},
    Error, WidgetSource,
};

pub struct Parser<'a> {
    arena: &'a Arena<Node<'a, RefCell<Ast>>>,
    picker: Arc<RwLock<Picker>>,
    font: Font<'a>,
    bg: Option<[u8; 4]>,
    basepath: Option<PathBuf>,
}

impl<'a> Parser<'a> {
    pub fn new(
        arena: &'a Arena<Node<'a, RefCell<Ast>>>,
        picker: Arc<RwLock<Picker>>,
        font: Font<'a>,
        bg: Option<[u8; 4]>,
        basepath: Option<PathBuf>,
    ) -> Self {
        Self {
            arena,
            picker,
            font,
            bg,
            basepath,
        }
    }
    pub fn parse<'b>(
        mut self,
        text: &str,
        width: u16,
        tx: &Sender<WidgetSource<'b>>,
    ) -> Result<(), Error> {
        let mut ext_options = ExtensionOptions::default();
        ext_options.strikethrough = true;

        let root = parse_document(
            &self.arena,
            &text,
            &Options {
                extension: ext_options,
                ..Default::default()
            },
        );

        let mut spans = vec![];
        let mut style = Style::new();

        let mut client = Client::new();
        let mut picker = self.picker.write().unwrap();

        for edge in root.traverse() {
            match edge {
                NodeEdge::Start(node) => {
                    let node_value = &node.data.borrow().value;
                    if let Some(modifier) = modifier(node_value) {
                        style = style.add_modifier(modifier);
                    }
                    match node.data.borrow().value {
                        NodeValue::Code(_) => {
                            style = style.on_dark_gray();
                        }
                        _ => {}
                    }
                }
                NodeEdge::End(node) => {
                    match node.data.borrow().value {
                        NodeValue::Text(ref literal) => {
                            let span = Span::from(literal.clone()).style(style);
                            spans.push(span);
                        }
                        NodeValue::Heading(ref tier) => {
                            let source = header_source(
                                &mut picker,
                                &mut self.font,
                                self.bg,
                                width,
                                spans,
                                tier.level,
                                false,
                            )
                            .unwrap(); // TODO don't
                            tx.send(source)?;
                            // sources.push(source);
                            spans = vec![];
                        }
                        NodeValue::Image(ref link) => {
                            match image_source(
                                &mut picker,
                                width,
                                self.basepath.clone(),
                                &mut client,
                                link.url.as_str(),
                                false,
                            ) {
                                Ok(source) => {
                                    // sources.push(source);
                                    tx.send(source)?;
                                }
                                Err(err) => {
                                    let text = Text::from(format!("[Image error: {err:?}]"));
                                    let height = text.height() as u16;
                                    tx.send(WidgetSource {
                                        height,
                                        source: WidgetSourceData::Text(text),
                                    })?;
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
                                tx.send(WidgetSource {
                                    height,
                                    source: WidgetSourceData::Text(text),
                                })?;
                            }
                            spans = vec![];
                        }
                        NodeValue::LineBreak | NodeValue::SoftBreak => {
                            let wrapped_lines = wrap_spans(spans, width as usize);
                            for line in wrapped_lines {
                                let text = Text::from(line);
                                let height = text.height() as u16;
                                tx.send(WidgetSource {
                                    height,
                                    source: WidgetSourceData::Text(text),
                                })?;
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
