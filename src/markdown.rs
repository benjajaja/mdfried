use bitflags::bitflags;
use tree_sitter::{Node, Parser, Tree, TreeCursor};
use unicode_width::UnicodeWidthStr;

use crate::error::Error;

pub struct MdParser(Parser);

impl MdParser {
    pub fn new() -> Result<Self, Error> {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_md::LANGUAGE.into())
            .ok()
            .ok_or(Error::MarkdownParse)?;
        Ok(Self(parser))
    }
}

pub struct MdDocument {
    source: String,
    tree: Tree,
}

impl MdDocument {
    pub fn new(source: String, parser: &mut MdParser) -> Result<Self, Error> {
        let tree = parser.0.parse(&source, None).ok_or(Error::MarkdownParse)?;
        Ok(Self { source, tree })
    }

    pub fn iter(&self) -> MdIterator<'_> {
        let mut inline_parser = Parser::new();

        #[expect(clippy::unwrap_used)]
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
                    // Horrible, yes, rip out later and improve to catch all images,
                    // including linewrapped stuff. To be done in worker.rs wrap_md_spans().
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
                                            link_destination.to_owned(),
                                            image_description.to_owned(),
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }

                let Some(tree) = self.inline_parser.parse(text, None) else {
                    return Some(MdSection::Markdown(vec![MdSpan::new(
                        text.to_owned(),
                        MdModifier::default(),
                    )]));
                };

                let mdspans =
                    inline_node_to_spans(tree.root_node(), text, MdModifier::default(), 0);
                let mdspans = mdspans
                    .iter()
                    .flat_map(|mdspan| {
                        let mut first = true;
                        mdspan
                            .content
                            .split('\n')
                            .map(|part| MdSpan {
                                content: part.to_owned(),
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
    pub struct MdModifier: u16 {
        const Emphasis = 1 << 0;
        const StrongEmphasis = 1 << 1;
        const Code = 1 << 2;
        const Link = 1 << 3;
        const LinkDescription = 1 << 4;
        const LinkDescriptionWrapper = 1 << 5;
        const LinkURL = 1 << 6;
        const LinkURLWrapper = 1 << 7;
        const Image = 1 << 8;
        const NewLine = 1 << 9;
    }
}

#[derive(Debug, PartialEq)]
pub struct MdSpan {
    pub content: String,
    pub extra: MdModifier,
}

impl MdSpan {
    pub fn new(content: String, extra: MdModifier) -> Self {
        MdSpan { content, extra }
    }

    #[cfg(test)]
    pub fn link(description: &str, url: &str) -> Vec<Self> {
        vec![
            Self::new("[".to_owned(), MdModifier::Link),
            Self::new(description.to_owned(), MdModifier::Link),
            Self::new("]".to_owned(), MdModifier::Link),
            Self::new("(".to_owned(), MdModifier::Link),
            Self::new(url.to_owned(), MdModifier::Link | MdModifier::LinkURL),
            Self::new(")".to_owned(), MdModifier::Link),
        ]
    }
}

impl From<String> for MdSpan {
    fn from(value: String) -> Self {
        Self::new(value, MdModifier::default())
    }
}

#[cfg(test)]
impl From<&str> for MdSpan {
    fn from(value: &str) -> Self {
        Self::from(value.to_owned())
    }
}

impl UnicodeWidthStr for MdSpan {
    // TODO: could this be deref or something magical?
    fn width(&self) -> usize {
        self.content.width()
    }

    fn width_cjk(&self) -> usize {
        self.content.width_cjk()
    }
}

pub enum MdSection {
    Header(String, u8),
    Markdown(Vec<MdSpan>),
    Image(String, String), // TODO used?
}

#[expect(clippy::string_slice)] // Let's hope tree-sitter is right
fn inline_node_to_spans(node: Node, source: &str, extra: MdModifier, _depth: usize) -> Vec<MdSpan> {
    let kind = node.kind();
    // print!("{}", String::from("  ").repeat(depth));
    // println!("{kind} - `{}`", &source[node.byte_range()]);

    if kind.contains("delimiter") {
        // print!("{}", String::from("  ").repeat(depth));
        // println!("delimiter - early return");
        return vec![];
    }

    let extra = match kind {
        "emphasis" => extra.union(MdModifier::Emphasis),
        "strong_emphasis" => extra.union(MdModifier::StrongEmphasis),
        "code_span" => extra.union(MdModifier::Code),
        "[" | "]" => extra.union(MdModifier::LinkDescriptionWrapper),
        "(" | ")" => extra.union(MdModifier::LinkURLWrapper),
        "link_text" => extra.union(MdModifier::LinkDescription),
        "inline_link" => extra.union(MdModifier::Link),
        "image" => extra.union(MdModifier::Image),
        "link_destination" => {
            // don't go deeper, it just has the URL parts
            // although we could highlight the parts
            return vec![MdSpan::new(
                // this also assumes no newline at beginning here
                source[node.byte_range()].to_owned(),
                extra.union(MdModifier::LinkURL),
            )];
        }
        _ => extra,
    };

    let (extra, newline_offset) = if source.as_bytes()[node.start_byte()] == b'\n' {
        (extra.union(MdModifier::NewLine), 1)
    } else {
        (extra, 0)
    };

    if node.child_count() == 0 {
        return vec![MdSpan::new(
            source[newline_offset + node.start_byte()..node.end_byte()].to_owned(),
            extra,
        )];
    }

    let mut spans = Vec::new();
    let mut pos = node.start_byte() + newline_offset;

    for child in node.children(&mut node.walk()) {
        if child.start_byte() > pos {
            spans.push(MdSpan::new(
                source[pos..child.start_byte()].to_owned(),
                extra,
            ));
        }
        // A node cannot possible start with \n, so we don't need to pass newline_offset down here.
        spans.extend(inline_node_to_spans(child, source, extra, _depth + 1));
        pos = child.end_byte();
    }

    if pos < node.end_byte() {
        spans.push(MdSpan::new(source[pos..node.end_byte()].to_owned(), extra));
    }

    spans
}
