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

enum CookedModifier {
    None,
    Raw(Modifier),
}

pub fn traverse<'a>(model: &mut Model<'a>, width: u16) -> Vec<WidgetSource<'a>> {
    let mut debug = vec![];
    let mut lines = vec![];
    let mut spans = vec![];
    let mut style = Style::new();

    let mut sources: Vec<WidgetSource<'a>> = vec![];

    let mut client = Client::new();

    for edge in model.root.traverse() {
        match edge {
            NodeEdge::Start(node) => {
                let node_value = &node.data.borrow().value;
                if let CookedModifier::Raw(modifier) = modifier(node_value) {
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
                        lines = vec![];
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
                        lines = vec![];
                        spans = vec![];
                    }
                    NodeValue::Paragraph => {
                        lines.push(Line::from(spans));
                        lines.push(Line::default());
                        let text = Text::from(lines);
                        lines = vec![];
                        spans = vec![];
                        let height = text.height() as u16;
                        sources.push(WidgetSource {
                            height,
                            source: WidgetSourceData::Text(text),
                        });
                    }
                    NodeValue::LineBreak | NodeValue::SoftBreak => {
                        lines.push(Line::from(spans));
                        let text = Text::from(lines);
                        lines = vec![];
                        spans = vec![];
                        let height = text.height() as u16;
                        sources.push(WidgetSource {
                            height,
                            source: WidgetSourceData::Text(text),
                        });
                    }
                    _ => {
                        if let CookedModifier::Raw(modifier) = modifier(&node.data.borrow().value) {
                            style = style.remove_modifier(modifier);
                        }
                    }
                }
            }
        }
    }

    sources
}

fn modifier(node_value: &NodeValue) -> CookedModifier {
    match node_value {
        NodeValue::Strong => CookedModifier::Raw(Modifier::BOLD),
        NodeValue::Emph => CookedModifier::Raw(Modifier::ITALIC),
        NodeValue::Strikethrough => CookedModifier::Raw(Modifier::CROSSED_OUT),
        _ => CookedModifier::None,
    }
}
