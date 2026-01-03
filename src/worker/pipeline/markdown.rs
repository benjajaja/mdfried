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
        MdIterator {
            source: &self.source,
            cursor: self.tree.walk(),
            done: false,
            inline_parser: MdDocument::inline_parser(),
        }
    }

    pub fn inline_parser() -> Parser {
        let mut inline_parser = Parser::new();
        #[expect(clippy::unwrap_used)]
        inline_parser
            .set_language(&tree_sitter_md::INLINE_LANGUAGE.into())
            .unwrap();
        inline_parser
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

                let Some(tree) = self.inline_parser.parse(text, None) else {
                    return Some(MdSection::Markdown(vec![MdSpan::new(
                        text.to_owned(),
                        MdModifier::default(),
                    )]));
                };

                let mdspans =
                    inline_node_to_spans(tree.root_node(), text, MdModifier::default(), 0);
                let mdspans = split_newlines(mdspans);
                // mdspans.push(MdSpan::new("".to_owned(), MdModifier::NewLine));
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
}

#[expect(clippy::string_slice)] // Let's hope tree-sitter is right
fn inline_node_to_spans(node: Node, source: &str, extra: MdModifier, _depth: usize) -> Vec<MdSpan> {
    let kind = node.kind();
    // print!(">{}", String::from("  ").repeat(_depth));
    // println!(" {kind} - `{}`", &source[node.byte_range()]);

    if kind.contains("delimiter") {
        // print!("{}", String::from("  ").repeat(depth));
        // println!("delimiter - early return");
        return vec![];
    }

    let current_extra = match kind {
        "emphasis" => MdModifier::Emphasis,
        "strong_emphasis" => MdModifier::StrongEmphasis,
        "code_span" => MdModifier::Code,
        "[" | "]" => MdModifier::LinkDescriptionWrapper,
        "(" | ")" => MdModifier::LinkURLWrapper,
        "link_text" => MdModifier::LinkDescription,
        "inline_link" => MdModifier::Link,
        "image" => MdModifier::Image,
        "link_destination" => {
            // TODO: can we go deeper like usual, now that we skip punctuation?
            // don't go deeper, it just has the URL parts
            // although we could highlight the parts
            return vec![MdSpan::new(
                // this also assumes no newline at beginning here
                source[node.byte_range()].to_owned(),
                extra.union(MdModifier::LinkURL),
            )];
        }
        _ => MdModifier::default(),
    };
    let extra = extra.union(current_extra);

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
        if is_punctuation(child.kind(), current_extra) {
            continue;
        }
        let mut ended_with_newline = false;
        if child.start_byte() > pos {
            spans.push(MdSpan::new(
                source[pos..child.start_byte()].to_owned(),
                extra,
            ));
            if source.as_bytes()[child.start_byte() - 1] == b'\n' {
                ended_with_newline = true;
            }
        }
        let extra = if ended_with_newline {
            extra.union(MdModifier::NewLine)
        } else {
            extra
        };
        // A node cannot possible start with \n, so we don't need to pass newline_offset down here.
        spans.extend(inline_node_to_spans(child, source, extra, _depth + 1));
        pos = child.end_byte();
    }

    if pos < node.end_byte() {
        spans.push(MdSpan::new(source[pos..node.end_byte()].to_owned(), extra));
    }

    spans
}

#[inline]
fn is_punctuation(kind: &str, parent_modifier: MdModifier) -> bool {
    match kind {
        // ()[], only if *direct* children of Link, should become separate spans.
        "(" | ")" | "[" | "]" if parent_modifier == MdModifier::Link => false,
        // Single character punctuation
        "!" | "\"" | "#" | "$" | "%" | "&" | "'" | "(" | ")" | "*" | "+" | "," | "-" | "."
        | "/" | ":" | ";" | "<" | "=" | ">" | "?" | "@" | "[" | "\\" | "]" | "^" | "_" | "`"
        | "{" | "|" | "}" | "~" => true,
        // Multi-character tokens
        // "-->" | "<!--" | "<![CDATA[" | "<?" | "?>" | "]]>" => ???
        // Named delimiter nodes
        // "code_span_delimiter" | "emphasis_delimiter" | "latex_span_delimiter" => ???
        _ => false,
    }
}

fn split_newlines(mdspans: Vec<MdSpan>) -> Vec<MdSpan> {
    mdspans
        .iter()
        .flat_map(|mdspan| {
            let mut first = true;
            mdspan
                .content
                .split('\n')
                .filter_map(|part| {
                    if part.is_empty() {
                        first = false;
                        None
                    } else {
                        Some(MdSpan {
                            content: part.to_owned(),
                            extra: if first {
                                first = false;
                                mdspan.extra
                            } else {
                                mdspan.extra.union(MdModifier::NewLine)
                            },
                        })
                    }
                })
                .collect::<Vec<MdSpan>>()
        })
        .collect()
}

#[cfg(test)]
#[expect(clippy::unwrap_used)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn split_no_empty_spans() {
        let mdspans = split_newlines(vec![
            MdSpan::new("one line".to_owned(), MdModifier::default()),
            MdSpan::new(".".to_owned(), MdModifier::default()),
            MdSpan::new("\nanother line".to_owned(), MdModifier::NewLine),
            MdSpan::new(".".to_owned(), MdModifier::default()),
        ]);
        assert_eq!(
            mdspans,
            vec![
                MdSpan::new("one line".to_owned(), MdModifier::default()),
                MdSpan::new(".".to_owned(), MdModifier::default()),
                MdSpan::new("another line".to_owned(), MdModifier::NewLine),
                MdSpan::new(".".to_owned(), MdModifier::default()),
            ]
        );

        let mdspans = split_newlines(vec![
            MdSpan::new("one line".to_owned(), MdModifier::default()),
            MdSpan::new("\nanother line".to_owned(), MdModifier::NewLine),
        ]);
        assert_eq!(
            mdspans,
            vec![
                MdSpan::new("one line".to_owned(), MdModifier::default()),
                MdSpan::new("another line".to_owned(), MdModifier::NewLine),
            ]
        );
    }

    #[test]
    fn inline_node_to_spans_then_split_newlines_simple() {
        // let mut parser = MdParser::new().unwrap();
        // let doc =
        // MdDocument::new("this *is* a test.\nAnother line.".to_owned(), &mut parser).unwrap();
        let source = "one\ntwo\nthree\n";
        let tree = MdDocument::inline_parser().parse(source, None).unwrap();
        let mdspans = inline_node_to_spans(tree.root_node(), source, MdModifier::default(), 0);
        let mdspans = split_newlines(mdspans);
        assert_eq!(
            mdspans,
            vec![
                MdSpan::new("one".to_owned(), MdModifier::default()),
                MdSpan::new("two".to_owned(), MdModifier::NewLine),
                MdSpan::new("three".to_owned(), MdModifier::NewLine),
            ]
        )
    }

    #[test]
    fn inline_node_to_spans_then_split_newlines() {
        let source = "This *is* a test.\nAnother line.";
        let tree = MdDocument::inline_parser().parse(source, None).unwrap();
        let mdspans = inline_node_to_spans(tree.root_node(), source, MdModifier::default(), 0);
        let mdspans = split_newlines(mdspans);
        assert_eq!(
            mdspans,
            vec![
                MdSpan::new("This ".to_owned(), MdModifier::default()),
                MdSpan::new("is".to_owned(), MdModifier::Emphasis),
                MdSpan::new(" a test.".to_owned(), MdModifier::default()),
                MdSpan::new("Another line.".to_owned(), MdModifier::NewLine),
            ]
        )
    }

    #[test]
    fn split_newlines_at_styled() {
        let source = "This\n*is* a test.";
        let tree = MdDocument::inline_parser().parse(source, None).unwrap();
        let mdspans = inline_node_to_spans(tree.root_node(), source, MdModifier::default(), 0);
        let mdspans = split_newlines(mdspans);
        assert_eq!(
            mdspans,
            vec![
                MdSpan::new("This".to_owned(), MdModifier::default()),
                MdSpan::new("is".to_owned(), MdModifier::Emphasis | MdModifier::NewLine),
                MdSpan::new(" a test.".to_owned(), MdModifier::default()),
            ]
        )
    }

    #[test]
    fn split_newlines_middle() {
        let source = "hello\nworld";
        let tree = MdDocument::inline_parser().parse(source, None).unwrap();
        let mdspans = inline_node_to_spans(tree.root_node(), source, MdModifier::default(), 0);
        let mdspans = split_newlines(mdspans);
        assert_eq!(
            mdspans,
            vec![
                MdSpan::new("hello".to_owned(), MdModifier::default()),
                MdSpan::new("world".to_owned(), MdModifier::NewLine),
            ]
        )
    }

    #[test]
    fn merges_punctuation() {
        let source = "one, two.";
        let tree = MdDocument::inline_parser().parse(source, None).unwrap();
        let mdspans = inline_node_to_spans(tree.root_node(), source, MdModifier::default(), 0);
        let mdspans = split_newlines(mdspans);
        assert_eq!(
            mdspans,
            vec![MdSpan::new("one, two.".to_owned(), MdModifier::default()),]
        )
    }
}
