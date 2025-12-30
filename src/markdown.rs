mod links;

use bitflags::bitflags;
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use textwrap::{Options, wrap};
use tree_sitter::{Node, Parser, Tree, TreeCursor};
use unicode_width::UnicodeWidthStr as _;

use crate::{
    DocumentId, Event, WidgetSource,
    widget_sources::{BigText, LineExtra, WidgetSourceData},
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
                section.into_events(document_id, width, has_text_size_protocol, &mut source_id)
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

                let mdspans = inline_node_to_spans(
                    tree.root_node(),
                    text,
                    Style::default(),
                    MdModifier::default(),
                    0,
                );
                let mdspans = mdspans
                    .iter()
                    .flat_map(|mdspan| {
                        let mut first = true;
                        mdspan
                            .content
                            .split('\n')
                            .map(|part| MdSpan {
                                content: part.to_owned(),
                                style: mdspan.style,
                                extra: if first {
                                    first = false;
                                    mdspan.extra
                                } else {
                                    mdspan.extra.union(MdModifier::NewLine)
                                },
                            })
                            .collect::<Vec<MdSpan>>()
                    })
                    .collect();
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
        const NewLine = 1 << 3;
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
    Image(String, String), // TODO used?
}
impl MdSection {
    fn into_events(
        self,
        document_id: DocumentId,
        width: u16,
        has_text_size_protocol: bool,
        source_id: &mut usize,
    ) -> Vec<Event> {
        match self {
            MdSection::Header(text, tier) => {
                if has_text_size_protocol {
                    let (n, d) = BigText::size_ratio(tier);
                    let scaled_with = width as usize / 2 * usize::from(d) / usize::from(n);
                    let options = Options::new(scaled_with)
                        .break_words(true) // break long words/URLs if they exceed width
                        .word_splitter(textwrap::word_splitters::WordSplitter::NoHyphenation); // no hyphens when breaking
                    let lines = wrap(&text, options);
                    lines
                        .iter()
                        .map(|line| {
                            Event::Parsed(
                                document_id,
                                WidgetSource {
                                    id: MdSection::incr_source_id(source_id),
                                    height: 2,
                                    data: WidgetSourceData::Header(line.to_string(), tier),
                                },
                            )
                        })
                        .collect()
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

                // Do you remember that sound?
                fn carriage_return(
                    line_events: &mut Vec<Event>,
                    document_id: DocumentId,
                    source_id: &mut usize,
                    spans: &mut Vec<Span<'static>>,
                    extras: &mut Vec<LineExtra>,
                    had_image: &mut Option<String>,
                    max_width: u16,
                ) {
                    let line = if spans.len() == 1 && spans[0].width() > max_width as usize {
                        // println!("break it down");
                        let spans = std::mem::take(spans);
                        let span = &spans[0];
                        let options = Options::new(max_width as usize)
                            .break_words(true) // break long words/URLs if they exceed width
                            .word_splitter(textwrap::word_splitters::WordSplitter::NoHyphenation); // no hyphens when breaking
                        let parts = wrap(&span.content, options);

                        let part_spans: Vec<Span<'static>> = parts
                            .iter()
                            .map(|part| {
                                let mut part_span = Span::from(part.to_string());
                                part_span.style = span.style.clone();
                                // println!("part : {}", part);
                                // println!("part width: {}", part.width());
                                part_span
                            })
                            .collect();
                        // println!("parts: {part_spans:?}");

                        let last_index = part_spans.len().checked_sub(1).unwrap_or(0);
                        let mut last_line = Line::default();
                        for (i, part_span) in part_spans.into_iter().enumerate() {
                            if i != last_index {
                                let line = Line::from(part_span);
                                line_events.push(Event::Parsed(
                                    document_id,
                                    WidgetSource {
                                        id: MdSection::incr_source_id(source_id),
                                        height: 1,
                                        data: WidgetSourceData::Line(line, std::mem::take(extras)),
                                    },
                                ));
                            } else {
                                last_line = Line::from(part_span);
                            }
                        }
                        last_line
                    } else {
                        Line::from(std::mem::take(spans))
                    };

                    if let Some(url) = had_image.take() {
                        line_events.push(Event::ParseImage(
                            document_id,
                            MdSection::incr_source_id(source_id),
                            url,
                            String::from("XXX..."),
                            String::from("???"),
                        ));
                    } else {
                        line_events.push(Event::Parsed(
                            document_id,
                            WidgetSource {
                                id: MdSection::incr_source_id(source_id),
                                height: 1,
                                data: WidgetSourceData::Line(line, std::mem::take(extras)),
                            },
                        ));
                    }
                }

                let mut line_width = 0;
                let mut spans = Vec::new();
                let mut extras = Vec::new();
                let mut link_offset = 0; // TODO this sucks
                let mut had_image = None;

                for mdspan in mdspans {
                    let span_width = mdspan.content.width();
                    let would_overflow = line_width + span_width as u16 > width;

                    if mdspan.extra.contains(MdModifier::NewLine) || would_overflow {
                        // println!(
                        // "is_overflow {would_overflow} / starts_with_newline {starts_with_newline}"
                        // );
                        // push spans before this one into a line
                        line_width = 0;
                        // println!("push line: {spans:?}");
                        carriage_return(
                            &mut line_events,
                            document_id,
                            source_id,
                            &mut spans,
                            &mut extras,
                            &mut had_image,
                            width,
                        );
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
                    // println!("next: {mdspan:?}");
                    let span: Span<'static> = mdspan.into();
                    spans.push(span);
                }

                if !spans.is_empty() {
                    // println!("last");
                    carriage_return(
                        &mut line_events,
                        document_id,
                        source_id,
                        &mut spans,
                        &mut extras,
                        &mut had_image,
                        width,
                    );
                }
                debug_assert!(spans.len() == 0, "used up all spans");

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
fn inline_node_to_spans(
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
        print!("{}", String::from("  ").repeat(depth));
        println!("delimiter - early return");
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
                // this also assumes no newline at beginning here
                source[node.byte_range()].to_owned(),
                style.fg(Color::Indexed(32)).underlined(),
                extra.union(MdModifier::LinkURL),
            )];
        }
        _ => (style, extra),
    };

    let (extra, newline_offset) = if source.as_bytes()[node.start_byte()] == b'\n' {
        (extra.union(MdModifier::NewLine), 1)
    } else {
        (extra, 0)
    };

    if node.child_count() == 0 {
        return vec![MdSpan::new(
            source[newline_offset + node.start_byte()..node.end_byte()].to_owned(),
            style,
            extra,
        )];
    }

    let mut spans = Vec::new();
    let mut pos = node.start_byte() + newline_offset;

    for child in node.children(&mut node.walk()) {
        if child.start_byte() > pos {
            spans.push(MdSpan::new(
                source[pos..child.start_byte()].to_owned(),
                style,
                extra,
            ));
        }
        // A node cannot possible start with \n, so we don't need to pass newline_offset down here.
        spans.extend(inline_node_to_spans(child, source, style, extra, depth + 1));
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
        let doc = MdDocument::new(text, &mut parser);
        doc.parse(document_id, width, has_text_size_protocol)
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
                let Event::Parsed(_, source) = ev else {
                    panic!("unrelated event");
                };
                source.to_string()
            })
            .collect();
        assert_eq!(
            vec![
                "[a b](",
                "http://link.com/",
                "veeeeeeeeeeeeeeeeery/long/tail",
                ")",
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
                        Line::from(vec![
                            Span::from("[").fg(Color::Indexed(237)),
                            Span::from("a b").fg(Color::Indexed(4)),
                            Span::from("]").fg(Color::Indexed(237)),
                            Span::from("(").fg(Color::Indexed(237)),
                        ]),
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
                            Span::from("http://link.com/")
                                .fg(Color::Indexed(32))
                                .underlined(),
                        ]),
                        vec![LineExtra::Link(
                            "http://link.com/veeeeeeeeeeeeeeeeery/long/tail".to_owned(),
                            0,
                            15,
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
                        Line::from(vec![
                            Span::from("veeeeeeeeeeeeeeeeery/long/tail")
                                .fg(Color::Indexed(32))
                                .underlined(),
                        ]),
                        Vec::new(),
                    ),
                },
            ),
            Event::Parsed(
                DocumentId::default(),
                WidgetSource {
                    id: 3,
                    height: 1,
                    data: WidgetSourceData::Line(
                        Line::from(vec![Span::from(")").fg(Color::Indexed(237))]),
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
    #[ignore] // https://github.com/tree-sitter-grammars/tree-sitter-markdown/issues/171
    fn parse_bare_link() {
        let events: Vec<Event> = parse(
            "http://ratatui.rs".to_owned(),
            &RatSkin::default(),
            DocumentId::default(),
            80,
            true,
        );

        let Event::Parsed(
            _,
            WidgetSource {
                data: WidgetSourceData::Line(_, links),
                ..
            },
        ) = &events[0]
        else {
            panic!("expected one widget event");
        };
        assert_eq!(
            *links,
            vec![LineExtra::Link("http://ratatui.rs".to_string(), 0, 20)]
        );
    }

    #[test]
    fn parse_multiple_links_same_line() {
        let events: Vec<Event> = parse(
            "[a](http://a.com) [b](http://b.com)".to_owned(),
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

    #[test]
    fn long_line_break() {
        let events: Vec<Event> = parse(
            "longline1\nlongline2".into(),
            &RatSkin::default(),
            DocumentId::default(),
            10,
            true,
        );
        let expected = vec![
            Event::Parsed(
                DocumentId::default(),
                WidgetSource {
                    id: 0,
                    height: 1,
                    data: WidgetSourceData::Line(
                        Line::from(vec![Span::from("longline1")]),
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
                        Line::from(vec![Span::from("longline2")]),
                        Vec::new(),
                    ),
                },
            ),
        ];
        assert_eq!(events, expected);
    }

    #[test]
    fn line_break() {
        let events: Vec<Event> = parse(
            "line1\nline2".into(),
            &RatSkin::default(),
            DocumentId::default(),
            10,
            true,
        );
        let expected = vec![
            Event::Parsed(
                DocumentId::default(),
                WidgetSource {
                    id: 0,
                    height: 1,
                    data: WidgetSourceData::Line(Line::from(vec![Span::from("line1")]), Vec::new()),
                },
            ),
            Event::Parsed(
                DocumentId::default(),
                WidgetSource {
                    id: 1,
                    height: 1,
                    data: WidgetSourceData::Line(Line::from(vec![Span::from("line2")]), Vec::new()),
                },
            ),
        ];
        assert_eq!(events, expected);
    }
}
