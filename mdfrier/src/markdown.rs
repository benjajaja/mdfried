use std::borrow::Cow;

use bitflags::bitflags;
use tree_sitter::{Node, Parser, Tree, TreeCursor};
use unicode_width::UnicodeWidthStr;

use crate::Error;

/// Default list marker when none can be determined.
const DEFAULT_LIST_MARKER: &str = "-";

pub struct MdDocument<'a> {
    source: String,
    tree: Tree,
    inline_parser: &'a mut Parser,
}

impl<'a> MdDocument<'a> {
    pub fn new(
        source: String,
        parser: &mut Parser,
        inline_parser: &'a mut Parser,
    ) -> Result<Self, Error> {
        // Ensure source ends with newline for proper tree-sitter-md parsing
        let source = if source.ends_with('\n') {
            source
        } else {
            source + "\n"
        };
        let tree = parser.parse(&source, None).ok_or(Error::MarkdownParse)?;

        Ok(Self {
            source,
            tree,
            inline_parser,
        })
    }

    pub fn sections(&mut self) -> MdIterator<'_> {
        MdIterator {
            source: &self.source,
            cursor: self.tree.walk(),
            done: false,
            inline_parser: self.inline_parser,
            context: Vec::new(),
            depth: 0,
            list_item_content_depth: None,
        }
    }
}

pub struct MdIterator<'a> {
    source: &'a str,
    cursor: TreeCursor<'a>,
    done: bool,
    inline_parser: &'a mut Parser,
    /// Current container ancestry with depth for tracking when to pop.
    context: Vec<(usize, MdContainer)>,
    /// Current depth in the tree.
    depth: usize,
    /// Depth of the last ListItem that has emitted content (for continuation detection).
    list_item_content_depth: Option<usize>,
}

impl Iterator for MdIterator<'_> {
    type Item = MdSection;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.done {
                return None;
            }

            let node = self.cursor.node();

            // Check if current node is a container and push to context
            if let Some(container) = self.node_to_container(node) {
                self.context.push((self.depth, container));
            }

            // Advance cursor
            if self.cursor.goto_first_child() {
                self.depth += 1;
            } else {
                while !self.cursor.goto_next_sibling() {
                    if self.cursor.goto_parent() {
                        self.depth -= 1;
                        // Pop containers that are no longer ancestors
                        while self.context.last().is_some_and(|(d, _)| *d >= self.depth) {
                            let popped = self.context.pop();
                            // If we're leaving a ListItem, reset the content tracking
                            if let Some((d, MdContainer::ListItem(_))) = popped {
                                if self.list_item_content_depth == Some(d) {
                                    self.list_item_content_depth = None;
                                }
                            }
                        }
                    } else {
                        self.done = true;
                        break;
                    }
                }
            }

            if let Some(content) = self.parse_node(node) {
                // Build nesting from container ancestry
                let nesting: Vec<MdContainer> =
                    self.context.iter().map(|(_, c)| c.clone()).collect();

                // Check if this is a continuation within a list item
                let list_item_depth = self
                    .context
                    .iter()
                    .filter(|(_, c)| matches!(c, MdContainer::ListItem(_)))
                    .map(|(d, _)| *d)
                    .next_back();

                let is_list_continuation = if let Some(depth) = list_item_depth {
                    if self.list_item_content_depth == Some(depth) {
                        // We've already emitted content for this list item
                        true
                    } else {
                        // First content for this list item
                        self.list_item_content_depth = Some(depth);
                        false
                    }
                } else {
                    false
                };

                return Some(MdSection {
                    content,
                    nesting,
                    is_list_continuation,
                });
            }
        }
    }
}

impl<'a> MdIterator<'a> {
    /// Parses a node and returns its content if it's a section.
    #[expect(clippy::string_slice)] // In tree-sitter we trust
    fn parse_node(&mut self, node: Node<'a>) -> Option<MdContent> {
        match node.kind() {
            "paragraph" => {
                let text = &self.source[node.byte_range()];

                // Skip empty/whitespace-only paragraphs
                if text.trim().is_empty() {
                    return None;
                }

                let Some(tree) = self.inline_parser.parse(text, None) else {
                    return Some(MdContent::Paragraph(vec![MdNode::new(
                        text.to_owned(),
                        MdModifier::default(),
                    )]));
                };

                // Count blockquote depth for stripping markers from content
                let blockquote_depth = self
                    .context
                    .iter()
                    .filter(|(_, c)| matches!(c, MdContainer::Blockquote(_)))
                    .count();

                let mdspans =
                    inline_node_to_spans(tree.root_node(), text, MdModifier::default(), 0);
                let mdspans = split_newlines(mdspans);
                // Strip blockquote markers from spans
                let mdspans: Vec<MdNode> = mdspans
                    .into_iter()
                    .map(|mut s| {
                        if s.extra.contains(MdModifier::NewLine) {
                            s.content =
                                strip_blockquote_prefix(&s.content, blockquote_depth).into_owned();
                        }
                        s
                    })
                    .filter(|s| {
                        // Keep spans with content, or empty spans with NewLine (hard line breaks)
                        (!s.content.is_empty() && !is_blockquote_marker_only(s.content.trim()))
                            || s.extra.contains(MdModifier::NewLine)
                    })
                    .collect();
                if mdspans.is_empty() {
                    return None;
                }
                Some(MdContent::Paragraph(mdspans))
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
                Some(MdContent::Header {
                    tier,
                    text: text.to_owned(),
                })
            }
            "block_continuation" => {
                // Blank line inside blockquote
                if let Some(parent) = node.parent() {
                    if parent.kind() == "block_quote" {
                        return Some(MdContent::Paragraph(Vec::new()));
                    }
                }
                None
            }
            "fenced_code_block" => {
                let mut language = String::new();
                let mut code = String::new();

                for child in node.children(&mut node.walk()) {
                    match child.kind() {
                        "info_string" => {
                            language = self.source[child.byte_range()].trim().to_owned();
                        }
                        "code_fence_content" => {
                            code = self.source[child.byte_range()].to_owned();
                        }
                        _ => {}
                    }
                }

                if code.ends_with('\n') {
                    code.pop();
                }

                Some(MdContent::CodeBlock { language, code })
            }
            "indented_code_block" => {
                let code = self.source[node.byte_range()]
                    .lines()
                    .map(|line| line.strip_prefix("    ").unwrap_or(line))
                    .collect::<Vec<_>>()
                    .join("\n");
                Some(MdContent::CodeBlock {
                    language: String::new(),
                    code,
                })
            }
            "thematic_break" => Some(MdContent::HorizontalRule),
            "pipe_table" => Some(self.parse_table(node)),
            _ => None,
        }
    }

    fn parse_table(&mut self, node: Node<'a>) -> MdContent {
        let mut header: Vec<Vec<MdNode>> = Vec::new();
        let mut rows: Vec<Vec<Vec<MdNode>>> = Vec::new();
        let mut alignments: Vec<TableAlignment> = Vec::new();

        for child in node.children(&mut node.walk()) {
            match child.kind() {
                "pipe_table_header" => {
                    header = self.parse_table_row(child);
                }
                "pipe_table_delimiter_row" => {
                    alignments = self.parse_table_alignments(child);
                }
                "pipe_table_row" => {
                    rows.push(self.parse_table_row(child));
                }
                _ => {}
            }
        }

        while alignments.len() < header.len() {
            alignments.push(TableAlignment::default());
        }

        MdContent::Table {
            header,
            rows,
            alignments,
        }
    }

    #[expect(clippy::string_slice)]
    fn parse_table_row(&mut self, row_node: Node<'a>) -> Vec<Vec<MdNode>> {
        let mut cells: Vec<Vec<MdNode>> = Vec::new();

        for child in row_node.children(&mut row_node.walk()) {
            if child.kind() == "pipe_table_cell" {
                let cell_text = self.source[child.byte_range()].trim();
                if cell_text.is_empty() {
                    cells.push(Vec::new());
                } else if let Some(tree) = self.inline_parser.parse(cell_text, None) {
                    let mdspans =
                        inline_node_to_spans(tree.root_node(), cell_text, MdModifier::default(), 0);
                    cells.push(mdspans);
                } else {
                    cells.push(vec![MdNode::new(
                        cell_text.to_owned(),
                        MdModifier::default(),
                    )]);
                }
            }
        }

        cells
    }

    #[expect(clippy::string_slice)]
    fn parse_table_alignments(&self, delimiter_node: Node<'a>) -> Vec<TableAlignment> {
        let mut alignments = Vec::new();

        for child in delimiter_node.children(&mut delimiter_node.walk()) {
            if child.kind() == "pipe_table_delimiter_cell" {
                let cell_text = &self.source[child.byte_range()];
                let starts_colon = cell_text.starts_with(':');
                let ends_colon = cell_text.ends_with(':');
                let alignment = match (starts_colon, ends_colon) {
                    (true, true) => TableAlignment::Center,
                    (false, true) => TableAlignment::Right,
                    _ => TableAlignment::Left,
                };
                alignments.push(alignment);
            }
        }

        alignments
    }

    fn node_to_container(&self, node: Node<'a>) -> Option<MdContainer> {
        match node.kind() {
            "list" => {
                for child in node.children(&mut node.walk()) {
                    if child.kind() == "list_item" {
                        return Some(MdContainer::List(self.extract_list_marker(child)));
                    }
                }
                Some(MdContainer::List(ListMarker::new(
                    DEFAULT_LIST_MARKER.into(),
                    0,
                )))
            }
            "list_item" => Some(MdContainer::ListItem(self.extract_list_marker(node))),
            "block_quote" => Some(MdContainer::Blockquote(BlockquoteMarker)),
            _ => None,
        }
    }

    #[expect(clippy::string_slice)]
    fn extract_list_marker(&self, list_item: Node<'a>) -> ListMarker {
        let mut marker_text: Cow<'_, str> = Cow::Borrowed(DEFAULT_LIST_MARKER);
        let mut indent = 0;
        let mut task: Option<bool> = None;

        for child in list_item.children(&mut list_item.walk()) {
            match child.kind() {
                "list_marker_minus"
                | "list_marker_plus"
                | "list_marker_star"
                | "list_marker_dot"
                | "list_marker_parenthesis" => {
                    marker_text = Cow::Owned(self.source[child.byte_range()].trim().to_owned());
                    indent = child.start_position().column;
                }
                "task_list_marker_checked" => {
                    task = Some(true);
                }
                "task_list_marker_unchecked" => {
                    task = Some(false);
                }
                _ => {}
            }
        }
        ListMarker::with_task(marker_text.into_owned(), indent, task)
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

#[derive(Debug, Clone, PartialEq)]
pub struct MdNode {
    pub content: String,
    pub extra: MdModifier,
}

impl MdNode {
    pub fn new(content: String, extra: MdModifier) -> Self {
        MdNode { content, extra }
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

impl From<String> for MdNode {
    fn from(value: String) -> Self {
        Self::new(value, MdModifier::default())
    }
}

#[cfg(test)]
impl From<&str> for MdNode {
    fn from(value: &str) -> Self {
        Self::from(value.to_owned())
    }
}

impl UnicodeWidthStr for MdNode {
    fn width(&self) -> usize {
        self.content.width()
    }

    fn width_cjk(&self) -> usize {
        self.content.width_cjk()
    }
}

/// A container in the document structure.
#[derive(Debug, Clone, PartialEq)]
pub enum MdContainer {
    List(ListMarker),
    ListItem(ListMarker),
    Blockquote(BlockquoteMarker),
}

/// Column alignment for tables.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum TableAlignment {
    #[default]
    Left,
    Center,
    Right,
}

/// Content of a markdown section.
#[derive(Debug, Clone, PartialEq)]
pub enum MdContent {
    Paragraph(Vec<MdNode>),
    Header {
        tier: u8,
        text: String,
    },
    CodeBlock {
        language: String,
        code: String,
    },
    HorizontalRule,
    Table {
        header: Vec<Vec<MdNode>>,
        rows: Vec<Vec<Vec<MdNode>>>,
        alignments: Vec<TableAlignment>,
    },
}

impl MdContent {
    pub fn is_blank(&self) -> bool {
        matches!(self, MdContent::Paragraph(nodes) if nodes.is_empty())
    }
}

/// Marker style for list items.
#[derive(Debug, Clone, PartialEq)]
pub struct ListMarker {
    pub original: String,
    pub indent: usize,
    pub task: Option<bool>,
}

impl ListMarker {
    pub fn new(original: String, indent: usize) -> Self {
        Self {
            original,
            indent,
            task: None,
        }
    }

    pub fn with_task(original: String, indent: usize, task: Option<bool>) -> Self {
        Self {
            original,
            indent,
            task,
        }
    }
}

/// Marker style for blockquotes.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct BlockquoteMarker;

/// A markdown section with its content and nesting path.
#[derive(Debug)]
pub struct MdSection {
    pub content: MdContent,
    pub nesting: Vec<MdContainer>,
    /// True if this is a continuation paragraph within a list item (not the first content).
    pub is_list_continuation: bool,
}

fn strip_blockquote_prefix(s: &str, depth: usize) -> Cow<'_, str> {
    if depth == 0 {
        return Cow::Borrowed(s);
    }
    let mut remaining = s;
    for _ in 0..depth {
        if let Some(rest) = remaining.strip_prefix("> ") {
            remaining = rest;
        } else if let Some(rest) = remaining.strip_prefix(">") {
            remaining = rest;
        } else {
            break;
        }
    }
    if remaining.len() == s.len() {
        Cow::Borrowed(s)
    } else {
        Cow::Owned(remaining.to_owned())
    }
}

fn is_blockquote_marker_only(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '>' {
            if chars.peek() == Some(&' ') {
                chars.next();
            }
        } else {
            return false;
        }
    }
    true
}

#[expect(clippy::string_slice)]
fn inline_node_to_spans(node: Node, source: &str, extra: MdModifier, _depth: usize) -> Vec<MdNode> {
    let kind = node.kind();

    if kind.contains("delimiter") {
        return vec![];
    }

    let current_extra = match kind {
        "emphasis" => MdModifier::Emphasis,
        "strong_emphasis" => MdModifier::StrongEmphasis,
        "code_span" => {
            return vec![MdNode::new(
                source[node.byte_range()].to_owned(),
                extra.union(MdModifier::Code),
            )];
        }
        "hard_line_break" | "soft_break" => {
            // GFM hard line break (two trailing spaces + newline) or soft break
            return vec![MdNode::new(String::new(), extra.union(MdModifier::NewLine))];
        }
        "[" | "]" => MdModifier::LinkDescriptionWrapper,
        "(" | ")" => MdModifier::LinkURLWrapper,
        "link_text" => MdModifier::LinkDescription,
        "inline_link" => MdModifier::Link,
        "image" => MdModifier::Image,
        "link_destination" => {
            return vec![MdNode::new(
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
        return vec![MdNode::new(
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
            spans.push(MdNode::new(
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
        spans.extend(inline_node_to_spans(child, source, extra, _depth + 1));
        pos = child.end_byte();
    }

    if pos < node.end_byte() {
        spans.push(MdNode::new(source[pos..node.end_byte()].to_owned(), extra));
    }

    spans
}

#[inline]
fn is_punctuation(kind: &str, parent_modifier: MdModifier) -> bool {
    match kind {
        "(" | ")" | "[" | "]" if parent_modifier == MdModifier::Link => false,
        "!" | "\"" | "#" | "$" | "%" | "&" | "'" | "(" | ")" | "*" | "+" | "," | "-" | "."
        | "/" | ":" | ";" | "<" | "=" | ">" | "?" | "@" | "[" | "\\" | "]" | "^" | "_" | "`"
        | "{" | "|" | "}" | "~" => true,
        _ => false,
    }
}

fn split_newlines(mdspans: Vec<MdNode>) -> Vec<MdNode> {
    let mut result = Vec::with_capacity(mdspans.len());
    for mdspan in mdspans {
        // Preserve empty spans that have NewLine flag (from hard_line_break)
        if mdspan.content.is_empty() && mdspan.extra.contains(MdModifier::NewLine) {
            result.push(mdspan);
            continue;
        }

        // Check if there are any newlines to split
        if !mdspan.content.contains('\n') {
            result.push(mdspan);
            continue;
        }

        let mut first = true;
        for part in mdspan.content.split('\n') {
            if part.is_empty() {
                first = false;
                continue;
            }
            result.push(MdNode {
                content: part.to_owned(),
                extra: if first {
                    first = false;
                    mdspan.extra
                } else {
                    mdspan.extra.union(MdModifier::NewLine)
                },
            });
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn split_no_empty_spans() {
        let mdspans = split_newlines(vec![
            MdNode::new("one line".to_owned(), MdModifier::default()),
            MdNode::new(".".to_owned(), MdModifier::default()),
            MdNode::new("\nanother line".to_owned(), MdModifier::NewLine),
            MdNode::new(".".to_owned(), MdModifier::default()),
        ]);
        assert_eq!(
            mdspans,
            vec![
                MdNode::new("one line".to_owned(), MdModifier::default()),
                MdNode::new(".".to_owned(), MdModifier::default()),
                MdNode::new("another line".to_owned(), MdModifier::NewLine),
                MdNode::new(".".to_owned(), MdModifier::default()),
            ]
        );
    }

    fn make_parser() -> Parser {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_md::LANGUAGE.into())
            .unwrap();
        parser
    }

    fn make_inline_parser() -> Parser {
        let mut inline_parser = Parser::new();
        inline_parser
            .set_language(&tree_sitter_md::INLINE_LANGUAGE.into())
            .unwrap();
        inline_parser
    }

    #[test]
    fn blockquote_blank_lines() {
        let mut parser = make_parser();
        let mut inline_parser = make_inline_parser();
        let source = r#"> First paragraph
>
> Second paragraph"#;
        let mut doc = MdDocument::new(source.to_owned(), &mut parser, &mut inline_parser).unwrap();

        let sections: Vec<_> = doc.sections().collect();
        assert_eq!(sections.len(), 3);
        assert!(!sections[0].content.is_blank());
        assert!(sections[1].content.is_blank());
        assert!(!sections[2].content.is_blank());
    }

    #[test]
    fn parse_header() {
        let mut parser = make_parser();
        let mut inline_parser = make_inline_parser();
        let mut doc =
            MdDocument::new("# Hello".to_owned(), &mut parser, &mut inline_parser).unwrap();
        let sections: Vec<_> = doc.sections().collect();
        assert_eq!(sections.len(), 1);
        assert!(matches!(
            sections[0].content,
            MdContent::Header { tier: 1, .. }
        ));
    }
}
