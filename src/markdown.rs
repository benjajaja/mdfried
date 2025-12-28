mod links;

use bitflags::bitflags;
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use tree_sitter::{Node, Parser, Tree, TreeCursor};
use unicode_width::UnicodeWidthStr as _;

use crate::{
    DocumentId, Event, WidgetSource,
    widget_sources::{LineExtra, WidgetSourceData},
};

pub struct MdParser(Parser);

impl Default for MdParser {
    fn default() -> Self {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_md::LANGUAGE.into())
            .unwrap();
        Self(parser)
    }
}

pub struct MdDocument {
    source: String,
    tree: Tree,
}

impl MdDocument {
    pub fn new(source: String, parser: &mut MdParser) -> Self {
        let tree = parser.0.parse(&source, None).unwrap();
        Self { source, tree }
    }

    pub fn iter(&self) -> MdIterator<'_> {
        let mut inline_parser = Parser::new();
        inline_parser
            .set_language(&tree_sitter_md::INLINE_LANGUAGE.into())
            .unwrap();

        MdIterator {
            source: &self.source,
            cursor: self.tree.walk(),
            done: false,
            inline_parser,
        }
    }

    pub fn parse(
        &self,
        document_id: DocumentId,
        width: u16,
        has_text_size_protocol: bool,
    ) -> Vec<Event> {
        let mut source_id = 0;
        self.iter()
            .flat_map(|section| {
                section.into_sources(document_id, width, has_text_size_protocol, &mut source_id)
            })
            .collect()
    }
}

pub struct MdIterator<'a> {
    source: &'a str,
    cursor: TreeCursor<'a>,
    done: bool,
    inline_parser: Parser,
}

impl Iterator for MdIterator<'_> {
    type Item = MdSection;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.done {
                return None;
            }

            let node = self.cursor.node();

            // Advance cursor
            if !self.cursor.goto_first_child() {
                while !self.cursor.goto_next_sibling() {
                    if !self.cursor.goto_parent() {
                        self.done = true;
                        break;
                    }
                }
            }

            let parsed = self.parse_node(node);
            if parsed.is_some() {
                return parsed;
            }
            // Only yield specific nodes
            // match node.kind() {
            // "atx_heading" | "paragraph" | "fenced_code_block" => return self.parse_node(node),
            // _ => continue,
            // }
        }
    }
}

impl<'a> MdIterator<'a> {
    #[expect(clippy::string_slice)] // In tree-sitter we trust
    fn parse_node(&mut self, node: Node<'a>) -> Option<MdSection> {
        match node.kind() {
            "paragraph" => {
                let text = &self.source[node.byte_range()];

                let cursor = &mut node.walk();
                let mut children = node.children(cursor);
                if children.len() == 1 {
                    // Try to catch paragraphs with only a single image.
                    // Horrible, yes, rip out later and improve to catch all images.
                    #[expect(clippy::unwrap_used)] // len check above
                    let node = children.next().unwrap();
                    if node.kind() == "inline" {
                        let inline_source = &self.source[node.byte_range()];
                        if let Some(inline_tree) = self.inline_parser.parse(inline_source, None) {
                            let inline_root = inline_tree.root_node();
                            if inline_root.kind() == "inline" {
                                let cursor = &mut inline_root.walk();
                                let mut children = inline_root.children(cursor);
                                if children.len() == 1 {
                                    #[expect(clippy::unwrap_used)] // len check above
                                    let inline_node = children.next().unwrap();
                                    if inline_node.kind() == "image" {
                                        let mut image_description = "";
                                        let mut link_destination = "";
                                        for child in inline_node.children(&mut inline_node.walk()) {
                                            match child.kind() {
                                                "image_description" => {
                                                    image_description =
                                                        &inline_source[child.byte_range()]
                                                }
                                                "link_destination" => {
                                                    link_destination =
                                                        &inline_source[child.byte_range()]
                                                }
                                                _ => {}
                                            }
                                        }
                                        return Some(MdSection::Image(
                                            image_description.to_owned(),
                                            link_destination.to_owned(),
                                        ));
                                        // return Some(self.image(
                                        // image_description.to_owned(),
                                        // link_destination.to_owned(),
                                        // String::new(),
                                        // ));
                                    }
                                }
                            }
                        }
                    }
                }

                let Some(tree) = self.inline_parser.parse(text, None) else {
                    return Some(MdSection::Markdown(vec![MdSpan::new(
                        text.to_owned(),
                        Style::default(),
                        MdModifier::default(),
                    )]));
                    // return Some(self.parsed(
                    // 1,
                    // WidgetSourceData::Line(Line::from(text.to_owned()), Vec::new()),
                    // ));
                };

                let mdspans = node_to_spans(
                    tree.root_node(),
                    text,
                    Style::default(),
                    MdModifier::default(),
                    0,
                );
                for span in &mdspans {
                    println!("SPAN: {span:?}");
                }
                Some(MdSection::Markdown(mdspans))
            }
            "atx_heading" => {
                let mut tier = 0;
                let mut text = "";
                for child in node.children(&mut node.walk()) {
                    match child.kind() {
                        "inline" => text = &self.source[child.byte_range()],
                        "atx_h1_marker" => tier = 1,
                        "atx_h2_marker" => tier = 2,
                        "atx_h3_marker" => tier = 3,
                        "atx_h4_marker" => tier = 4,
                        "atx_h5_marker" => tier = 5,
                        "atx_h6_marker" => tier = 6,
                        _ => {
                            debug_assert!(false, "heading greater than 6");
                        }
                    }
                }
                Some(MdSection::Header(text.to_owned(), tier))
            }
            _ => None,
        }
    }
}

bitflags! {
    #[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
    struct MdModifier: u32 {
        const Link = 1 << 0;
        const LinkURL = 1 << 1;
        const Image = 1 << 2;
    }
}

#[derive(Debug)]
pub struct MdSpan {
    content: String,
    style: Style,
    extra: MdModifier,
}

impl MdSpan {
    fn new(content: String, style: Style, extra: MdModifier) -> Self {
        MdSpan {
            content,
            style,
            extra,
        }
    }
}

impl From<MdSpan> for Span<'static> {
    fn from(span: MdSpan) -> Self {
        Span::styled(span.content, span.style)
    }
}

pub enum MdSection {
    Header(String, u8),
    Markdown(Vec<MdSpan>),
    Image(String, String),
}
impl MdSection {
    fn into_sources(
        self,
        document_id: DocumentId,
        width: u16,
        has_text_size_protocol: bool,
        source_id: &mut usize,
    ) -> Vec<Event> {
        match self {
            MdSection::Header(text, tier) => {
                if has_text_size_protocol {
                    vec![Event::Parsed(
                        document_id,
                        WidgetSource {
                            id: MdSection::incr_source_id(source_id),
                            height: 2,
                            data: WidgetSourceData::Header(text, tier),
                        },
                    )]
                } else {
                    vec![Event::ParseHeader(
                        document_id,
                        MdSection::incr_source_id(source_id),
                        tier,
                        text,
                    )]
                }
            }
            MdSection::Image(url, text) => {
                vec![Event::ParseImage(
                    document_id,
                    MdSection::incr_source_id(source_id),
                    url,
                    text,
                    String::new(),
                )]
            }
            MdSection::Markdown(mdspans) => {
                let mut line_events = Vec::new();
                fn push_line(
                    line_events: &mut Vec<Event>,
                    document_id: DocumentId,
                    source_id: &mut usize,
                    spans: &mut Vec<Span<'static>>,
                    extras: &mut Vec<LineExtra>,
                    starts_with_newline: bool,
                    had_image: &mut Option<String>,
                ) {
                    // let is_single_span = spans.len() == 1;
                    if starts_with_newline && let Some(span) = spans.get_mut(0) {
                        // if had_image {
                        // had_image = false;
                        // println!("starts_with_newline: {}", span.content);
                        // }
                        span.content = span.content.chars().skip(1).collect();
                    }
                    let line = Line::from(std::mem::take(spans));
                    if let Some(url) = had_image.take() {
                        // *had_image = None;
                        println!("image: {url:?}");
                        line_events.push(Event::ParseImage(
                            document_id,
                            MdSection::incr_source_id(source_id),
                            url,
                            String::from("Loading..."),
                            String::from("???"),
                        ));
                        return;
                    }
                    line_events.push(Event::Parsed(
                        document_id,
                        WidgetSource {
                            id: MdSection::incr_source_id(source_id),
                            height: 1,
                            data: WidgetSourceData::Line(line, std::mem::take(extras)),
                        },
                    ));
                }

                let mut line_width = 0;
                let mut spans = Vec::new();
                let mut extras = Vec::new();
                let mut link_offset = 0; // TODO this sucks
                let mut had_image = None;

                for mdspan in mdspans {
                    let span_width = mdspan.content.width();
                    let is_overflow = line_width + span_width as u16 > width;
                    let starts_with_newline = mdspan
                        .content
                        .chars()
                        .next()
                        .map(|c| c == '\n')
                        .unwrap_or_default();
                    if is_overflow || starts_with_newline {
                        line_width = 0;
                        push_line(
                            &mut line_events,
                            document_id,
                            source_id,
                            &mut spans,
                            &mut extras,
                            starts_with_newline,
                            &mut had_image,
                        );
                        // line_events.push(Event::Parsed(
                        // document_id,
                        // WidgetSource {
                        // id: MdSection::incr_source_id(source_id),
                        // height: 1,
                        // data: WidgetSourceData::Line(line, std::mem::take(&mut extras)),
                        // },
                        // ));
                        link_offset = 0;
                    }

                    if mdspan.extra.contains(MdModifier::LinkURL) {
                        if mdspan.extra.contains(MdModifier::Image) {
                            had_image = Some(mdspan.content.clone());
                        } else {
                            let url = mdspan.content.clone();
                            let url_width = url.width();
                            extras.push(LineExtra::Link(
                                url,
                                link_offset,
                                link_offset + (url_width as u16),
                            ));
                        }
                    }
                    link_offset += span_width as u16;
                    line_width += span_width as u16;
                    let span: Span<'static> = mdspan.into();
                    spans.push(span);
                }

                if let Some(span) = spans.first() {
                    let starts_with_newline = span
                        .content
                        .chars()
                        .next()
                        .map(|c| c == '\n')
                        .unwrap_or_default();
                    push_line(
                        &mut line_events,
                        document_id,
                        source_id,
                        &mut spans,
                        &mut extras,
                        starts_with_newline,
                        &mut had_image,
                    );
                }

                line_events

                // let mut line_events = Vec::new();
                // let mut line_width = 0;
                // let mut line = Line::default();
                // for mdspan in mdspans {
                // let span_width = mdspan.content.width();
                // let span: Span<'static> = mdspan.into();
                // line.push_span(span);
                // }
                //
                // line_events
            }
        }
    }

    // Increas source_id but return value before it was increased.
    fn incr_source_id(source_id: &mut usize) -> usize {
        let current = *source_id;
        *source_id += 1;
        current
    }
}

// impl MdSection {
// type Item = WidgetSourceData;
// type IntoIter = std::vec::IntoIter<WidgetSourceData>;

// fn to_events(self) -> std::vec::IntoIter<Event> {
// match self {
// MdSection::Header(text, tier) => vec![
// Event::Parsed((), ())
// WidgetSourceData::Header(text, tier)].into_iter(),
// MdSection::Image(alt, url) => vec![WidgetSourceData::Image(alt, url)].into_iter(),
// MdSection::Markdown(spans) => {
// let mut lines = Vec::new();
//
// let mut offset = 0;
// for span in spans {}
//
// lines.into_iter()
// }
// }
// }
// }

#[expect(clippy::string_slice)] // Let's hope tree-sitter is right
fn node_to_spans(
    node: Node,
    source: &str,
    style: Style,
    extra: MdModifier,
    depth: usize,
) -> Vec<MdSpan> {
    let kind = node.kind();
    print!("{}", String::from("  ").repeat(depth));
    println!("{kind} - `{}`", &source[node.byte_range()]);

    if kind.contains("delimiter") {
        return vec![];
    }

    let (style, extra) = match kind {
        "emphasis" => (style.add_modifier(Modifier::ITALIC), extra),
        "strong_emphasis" => (style.add_modifier(Modifier::BOLD), extra),
        "code_span" => (style.add_modifier(Modifier::DIM), extra),
        "[" | "]" | "(" | ")" => (style.fg(Color::Indexed(237)), extra),
        "link_text" => (style.fg(Color::Indexed(4)), extra),
        "inline_link" => (style, extra.union(MdModifier::Link)),
        "image" => (style, extra.union(MdModifier::Image)),
        "link_destination" => {
            // don't go deeper, it just has the URL parts
            // although we could highlight the parts
            return vec![MdSpan::new(
                source[node.byte_range()].to_owned(),
                style.fg(Color::Indexed(32)).underlined(),
                extra.union(MdModifier::LinkURL),
            )];
        }
        _ => (style, extra),
    };

    if node.child_count() == 0 {
        return vec![MdSpan::new(
            source[node.byte_range()].to_owned(),
            style,
            extra,
        )];
    }

    let mut spans = Vec::new();
    let mut pos = node.start_byte();

    for child in node.children(&mut node.walk()) {
        if child.start_byte() > pos {
            spans.push(MdSpan::new(
                source[pos..child.start_byte()].to_owned(),
                style,
                extra,
            ));
        }
        spans.extend(node_to_spans(child, source, style, extra, depth + 1));
        pos = child.end_byte();
    }

    if pos < node.end_byte() {
        spans.push(MdSpan::new(
            source[pos..node.end_byte()].to_owned(),
            style,
            extra,
        ));
    }

    spans
}

#[cfg(test)]
mod tests {
    use crate::{
        markdown::{
            MdDocument, MdParser, MdSection,
            links::{COLOR_DECOR, COLOR_LINK, COLOR_TEXT},
        },
        *,
    };
    use pretty_assertions::assert_eq;
    use ratskin::RatSkin;

    fn parse(
        text: String,
        skin: &RatSkin,
        document_id: DocumentId,
        width: u16,
        has_text_size_protocol: bool,
    ) -> Vec<Event> {
        let mut parser = MdParser::default();

        let sections: Vec<MdSection> = MdDocument::new(text, &mut parser).iter().collect();
        let mut source_id = 0;
        sections
            .into_iter()
            .flat_map(|section| {
                section.into_sources(document_id, width, has_text_size_protocol, &mut source_id)
            })
            .collect()
    }

    #[test]
    fn parse_one_basic_line() {
        let events: Vec<Event> = parse(
            "oh *ah* ha ha".into(),
            &RatSkin::default(),
            DocumentId::default(),
            80,
            true,
        );
        let expected = vec![Event::Parsed(
            DocumentId::default(),
            WidgetSource {
                id: 0,
                height: 1,
                data: WidgetSourceData::Line(
                    Line::from(vec![
                        Span::from("oh "),
                        Span::from("ah").italic(),
                        Span::from(" ha ha"),
                    ]),
                    Vec::new(),
                ),
            },
        )];
        assert_eq!(events, expected);
    }

    #[test]
    fn parse_link() {
        let events: Vec<Event> = parse(
            "[text](http://link.com)".to_owned(),
            &RatSkin::default(),
            DocumentId::default(),
            80,
            true,
        );
        let expected = vec![Event::Parsed(
            DocumentId::default(),
            WidgetSource {
                id: 0,
                height: 1,
                data: WidgetSourceData::Line(
                    Line::from(vec![
                        Span::from("[").fg(COLOR_DECOR),
                        Span::from("text").fg(COLOR_TEXT),
                        Span::from("]").fg(COLOR_DECOR),
                        Span::from("(").fg(COLOR_DECOR),
                        Span::from("http://link.com").fg(COLOR_LINK).underlined(),
                        Span::from(")").fg(COLOR_DECOR),
                    ]),
                    vec![LineExtra::Link("http://link.com".to_owned(), 7, 22)],
                ),
            },
        )];
        assert_eq!(events, expected);
    }

    #[test]
    fn parse_long_link() {
        let events: Vec<Event> = parse(
            "[text](http://link.com/veeeeeeeeeeeeeeeeery/long/tail)".to_owned(),
            &RatSkin::default(),
            DocumentId::default(),
            30,
            true,
        );
        let expected = vec![
            Event::Parsed(
                DocumentId::default(),
                WidgetSource {
                    id: 0,
                    height: 1,
                    data: WidgetSourceData::Line(
                        Line::from(vec![
                            Span::from("[").fg(COLOR_DECOR),
                            Span::from("text").fg(COLOR_TEXT),
                            Span::from("]").fg(COLOR_DECOR),
                            Span::from("(").fg(COLOR_DECOR),
                            Span::from("http://link.com/veeeeee")
                                .fg(COLOR_LINK)
                                .underlined(),
                        ]),
                        vec![LineExtra::Link(
                            "http://link.com/veeeeeeeeeeeeeeeeery/long/tail".to_owned(),
                            7,
                            30,
                        )],
                    ),
                },
            ),
            Event::Parsed(
                DocumentId::default(),
                WidgetSource {
                    id: 1,
                    height: 1,
                    data: WidgetSourceData::Line(
                        Line::from(vec![Span::from("eeeeeeeeeeery/long/tail)")]),
                        Vec::new(),
                    ),
                },
            ),
        ];
        assert_eq!(events, expected);
    }

    #[test]
    fn parse_long_linebroken_link() {
        let events: Vec<Event> = parse(
            "[a b](http://link.com/veeeeeeeeeeeeeeeeery/long/tail)".to_owned(),
            &RatSkin::default(),
            DocumentId::default(),
            30,
            true,
        );

        let str_lines: Vec<String> = events
            .iter()
            .map(|ev| {
                if let Event::Parsed(_, source) = ev {
                    return source.to_string();
                }
                "<unrelated event>".into()
            })
            .collect();
        assert_eq!(
            vec![
                "[a ",
                "b](http://link.com/veeeeeeeeee",
                "eeeeeeery/long/tail)"
            ],
            str_lines,
            "breaks into 3 lines",
        );

        let urls: Vec<String> = events
            .iter()
            .flat_map(|ev| {
                if let Event::Parsed(
                    _,
                    WidgetSource {
                        data: WidgetSourceData::Line(_, links),
                        ..
                    },
                ) = ev
                {
                    let urls: Vec<String> = links
                        .iter()
                        .flat_map(|extra| {
                            if let LineExtra::Link(url, _, _) = extra {
                                vec![url.to_owned()]
                            } else {
                                Vec::new()
                            }
                        })
                        .collect();
                    return urls;
                }
                vec![]
            })
            .collect();
        assert_eq!(
            vec!["http://link.com/veeeeeeeeeeeeeeeeery/long/tail"],
            urls,
            "finds the full URL"
        );

        let expected = vec![
            Event::Parsed(
                DocumentId::default(),
                WidgetSource {
                    id: 0,
                    height: 1,
                    data: WidgetSourceData::Line(
                        Line::from(vec![Span::from("[a"), Span::from(" ")]),
                        Vec::new(),
                    ),
                },
            ),
            Event::Parsed(
                DocumentId::default(),
                WidgetSource {
                    id: 1,
                    height: 1,
                    data: WidgetSourceData::Line(
                        Line::from(vec![
                            Span::from("b]("),
                            Span::from("http://link.com/veeeeeeeeee")
                                .fg(COLOR_LINK)
                                .underlined(),
                        ]),
                        vec![LineExtra::Link(
                            "http://link.com/veeeeeeeeeeeeeeeeery/long/tail".to_owned(),
                            3,
                            30,
                        )],
                    ),
                },
            ),
            Event::Parsed(
                DocumentId::default(),
                WidgetSource {
                    id: 2,
                    height: 1,
                    data: WidgetSourceData::Line(
                        Line::from(vec![Span::from("eeeeeeery/long/tail)")]),
                        Vec::new(),
                    ),
                },
            ),
        ];
        assert_eq!(
            events, expected,
            "stylizes the part of the URL that starts on one line"
        );
    }

    #[test]
    fn parse_multiple_links_same_line() {
        let events: Vec<Event> = parse(
            "http://a.com http://b.com".to_owned(),
            &RatSkin::default(),
            DocumentId::default(),
            80,
            true,
        );

        let urls: Vec<String> = events
            .iter()
            .flat_map(|ev| {
                if let Event::Parsed(
                    _,
                    WidgetSource {
                        data: WidgetSourceData::Line(_, links),
                        ..
                    },
                ) = ev
                {
                    let urls: Vec<String> = links
                        .iter()
                        .flat_map(|extra| {
                            if let LineExtra::Link(url, _, _) = extra {
                                vec![url.to_owned()]
                            } else {
                                Vec::new()
                            }
                        })
                        .collect();
                    return urls;
                }
                vec![]
            })
            .collect();
        assert_eq!(vec!["http://a.com", "http://b.com"], urls, "finds all URLs");
    }

    #[test]
    fn parse_header_wrapping_tier_1() {
        let events: Vec<Event> = parse(
            "# 1234567890".to_owned(),
            &RatSkin::default(),
            DocumentId::default(),
            10,
            true,
        );
        assert_eq!(2, events.len());

        let Event::Parsed(
            _,
            WidgetSource {
                data: WidgetSourceData::Header(text, tier),
                ..
            },
        ) = &events[0]
        else {
            panic!("expected Header");
        };
        assert_eq!(1, *tier);
        assert_eq!("12345", text);

        let Event::Parsed(
            _,
            WidgetSource {
                data: WidgetSourceData::Header(text, tier),
                ..
            },
        ) = &events[1]
        else {
            panic!("expected Header");
        };
        assert_eq!(1, *tier);
        assert_eq!("67890", text);
    }

    #[test]
    fn parse_header_wrapping_tier_4() {
        let events: Vec<Event> = parse(
            "#### 1234567890".to_owned(),
            &RatSkin::default(),
            DocumentId::default(),
            10,
            true,
        );
        assert_eq!(2, events.len());

        let Event::Parsed(
            _,
            WidgetSource {
                data: WidgetSourceData::Header(text, tier),
                ..
            },
        ) = &events[0]
        else {
            panic!("expected Header");
        };
        assert_eq!(4, *tier);
        assert_eq!("1234567", text);

        let Event::Parsed(
            _,
            WidgetSource {
                data: WidgetSourceData::Header(text, tier),
                ..
            },
        ) = &events[1]
        else {
            panic!("expected Header");
        };
        assert_eq!(4, *tier);
        assert_eq!("890", text);
    }
}
