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
            context: Vec::new(),
            depth: 0,
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
    /// Current container ancestry with depth for tracking when to pop.
    /// Only contains container types (List, ListItem, Blockquote), not section types.
    context: Vec<(usize, MdContainer)>,
    /// Current depth in the tree.
    depth: usize,
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
                            self.context.pop();
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
                return Some(MdSection { content, nesting });
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
                // Strip blockquote markers from spans and filter out empty/marker-only spans
                let mdspans: Vec<MdNode> = mdspans
                    .into_iter()
                    .map(|mut s| {
                        // Strip blockquote markers from start of NewLine spans
                        if s.extra.contains(MdModifier::NewLine) {
                            s.content = strip_blockquote_prefix(&s.content, blockquote_depth);
                        }
                        s
                    })
                    .filter(|s| {
                        // Filter out completely empty spans or blockquote-marker-only spans
                        // But keep whitespace-only spans (they may be meaningful between styled text)
                        !s.content.is_empty() && !is_blockquote_marker_only(s.content.trim())
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
                // A block_continuation that's a direct child of block_quote
                // (not inside a paragraph) represents a blank line
                if let Some(parent) = node.parent() {
                    if parent.kind() == "block_quote" {
                        // Blank line = Paragraph with empty content
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

                // Remove trailing newline if present
                if code.ends_with('\n') {
                    code.pop();
                }

                Some(MdContent::CodeBlock { language, code })
            }
            "indented_code_block" => {
                // Indented code blocks don't have a language
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

    /// Parses a pipe_table node into MdContent::Table.
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

        // Ensure alignments match header column count
        while alignments.len() < header.len() {
            alignments.push(TableAlignment::default());
        }

        MdContent::Table {
            header,
            rows,
            alignments,
        }
    }

    /// Parses cells from a table row (header or data row).
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

    /// Parses column alignments from the delimiter row.
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

    /// Converts a node to a container type if it's a container (not a section).
    fn node_to_container(&self, node: Node<'a>) -> Option<MdContainer> {
        match node.kind() {
            "list" => {
                // Find the first list_item to determine marker type
                for child in node.children(&mut node.walk()) {
                    if child.kind() == "list_item" {
                        return Some(MdContainer::List(self.extract_list_marker(child)));
                    }
                }
                Some(MdContainer::List(ListMarker::new("-".to_owned(), 0)))
            }
            "list_item" => Some(MdContainer::ListItem(self.extract_list_marker(node))),
            "block_quote" => Some(MdContainer::Blockquote(BlockquoteMarker)),
            _ => None,
        }
    }

    /// Extracts the marker from a list_item node.
    #[expect(clippy::string_slice)]
    fn extract_list_marker(&self, list_item: Node<'a>) -> ListMarker {
        let mut marker_text = "-".to_owned();
        let mut indent = 0;
        let mut task: Option<bool> = None;

        for child in list_item.children(&mut list_item.walk()) {
            match child.kind() {
                "list_marker_minus"
                | "list_marker_plus"
                | "list_marker_star"
                | "list_marker_dot"
                | "list_marker_parenthesis" => {
                    marker_text = self.source[child.byte_range()].trim().to_owned();
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
        ListMarker::with_task(marker_text, indent, task)
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
    // TODO: could this be deref or something magical?
    fn width(&self) -> usize {
        self.content.width()
    }

    fn width_cjk(&self) -> usize {
        self.content.width_cjk()
    }
}

/// A container in the document structure (can contain other elements).
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

/// Content of a markdown section (leaf node with its content).
#[derive(Debug, Clone, PartialEq)]
pub enum MdContent {
    /// Paragraph with inline-formatted content. Empty vec for blank lines.
    Paragraph(Vec<MdNode>),
    /// Header with tier (1-6) and raw text.
    Header { tier: u8, text: String },
    /// Code block with language and raw code.
    CodeBlock { language: String, code: String },
    /// Horizontal rule (---, ***, ___).
    HorizontalRule,
    /// Table with header, rows, and column alignments.
    Table {
        /// Header row cells, each cell is inline-parsed.
        header: Vec<Vec<MdNode>>,
        /// Data rows, each row contains inline-parsed cells.
        rows: Vec<Vec<Vec<MdNode>>>,
        /// Column alignments extracted from delimiter row.
        alignments: Vec<TableAlignment>,
    },
}

impl MdContent {
    /// Returns true if this content represents a blank line (empty paragraph).
    pub fn is_blank(&self) -> bool {
        matches!(self, MdContent::Paragraph(nodes) if nodes.is_empty())
    }
}

/// Marker style for list items.
#[derive(Debug, Clone, PartialEq)]
pub struct ListMarker {
    /// The original marker from source (e.g., "-", "*", "1.", "2)")
    pub original: String,
    /// Column position (indentation) of the marker in the source.
    pub indent: usize,
    /// Task list marker: None = not a task, Some(true) = checked, Some(false) = unchecked
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
    /// Nesting path: container ancestry with leaf section type as last element.
    pub nesting: Vec<MdContainer>,
}

/// Strip blockquote markers from the start of a string.
/// Strips up to `depth` markers ("> " or ">").
fn strip_blockquote_prefix(s: &str, depth: usize) -> String {
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
    remaining.to_owned()
}

/// Checks if content is only blockquote markers (e.g., "> ", "> > ").
/// These appear in paragraph content when tree-sitter includes continuation markers.
fn is_blockquote_marker_only(s: &str) -> bool {
    if s.is_empty() {
        return false; // Empty string is not a blockquote marker
    }
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '>' {
            // Expect optional space after >
            if chars.peek() == Some(&' ') {
                chars.next();
            }
        } else {
            return false;
        }
    }
    true
}

#[expect(clippy::string_slice)] // Let's hope tree-sitter is right
fn inline_node_to_spans(node: Node, source: &str, extra: MdModifier, _depth: usize) -> Vec<MdNode> {
    let kind = node.kind();
    // eprint!(">{}", String::from("  ").repeat(_depth));
    // eprintln!(" {kind} {:?} - `{}`", node.byte_range(), &source[node.byte_range()]);

    if kind.contains("delimiter") {
        // eprint!("{}", String::from("  ").repeat(_depth));
        // eprintln!("delimiter - early return");
        return vec![];
    }

    let current_extra = match kind {
        "emphasis" => MdModifier::Emphasis,
        "strong_emphasis" => MdModifier::StrongEmphasis,
        "code_span" => {
            // Return full byte range including backticks to preserve source
            return vec![MdNode::new(
                source[node.byte_range()].to_owned(),
                extra.union(MdModifier::Code),
            )];
        }
        "[" | "]" => MdModifier::LinkDescriptionWrapper,
        "(" | ")" => MdModifier::LinkURLWrapper,
        "link_text" => MdModifier::LinkDescription,
        "inline_link" => MdModifier::Link,
        "image" => MdModifier::Image,
        "link_destination" => {
            // TODO: can we go deeper like usual, now that we skip punctuation?
            // don't go deeper, it just has the URL parts
            // although we could highlight the parts
            return vec![MdNode::new(
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
        // A node cannot possible start with \n, so we don't need to pass newline_offset down here.
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

fn split_newlines(mdspans: Vec<MdNode>) -> Vec<MdNode> {
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
                        Some(MdNode {
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
                .collect::<Vec<MdNode>>()
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

        let mdspans = split_newlines(vec![
            MdNode::new("one line".to_owned(), MdModifier::default()),
            MdNode::new("\nanother line".to_owned(), MdModifier::NewLine),
        ]);
        assert_eq!(
            mdspans,
            vec![
                MdNode::new("one line".to_owned(), MdModifier::default()),
                MdNode::new("another line".to_owned(), MdModifier::NewLine),
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
                MdNode::new("one".to_owned(), MdModifier::default()),
                MdNode::new("two".to_owned(), MdModifier::NewLine),
                MdNode::new("three".to_owned(), MdModifier::NewLine),
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
                MdNode::new("This ".to_owned(), MdModifier::default()),
                MdNode::new("is".to_owned(), MdModifier::Emphasis),
                MdNode::new(" a test.".to_owned(), MdModifier::default()),
                MdNode::new("Another line.".to_owned(), MdModifier::NewLine),
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
                MdNode::new("This".to_owned(), MdModifier::default()),
                MdNode::new("is".to_owned(), MdModifier::Emphasis | MdModifier::NewLine),
                MdNode::new(" a test.".to_owned(), MdModifier::default()),
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
                MdNode::new("hello".to_owned(), MdModifier::default()),
                MdNode::new("world".to_owned(), MdModifier::NewLine),
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
            vec![MdNode::new("one, two.".to_owned(), MdModifier::default()),]
        )
    }

    #[test]
    fn multiple_styled_sections() {
        let source = "you can _put_ **Markdown** into";
        let tree = MdDocument::inline_parser().parse(source, None).unwrap();
        let mdspans = inline_node_to_spans(tree.root_node(), source, MdModifier::default(), 0);
        let mdspans = split_newlines(mdspans);
        assert_eq!(
            mdspans,
            vec![
                MdNode::new("you can ".to_owned(), MdModifier::default()),
                MdNode::new("put".to_owned(), MdModifier::Emphasis),
                MdNode::new(" ".to_owned(), MdModifier::default()),
                MdNode::new("Markdown".to_owned(), MdModifier::StrongEmphasis),
                MdNode::new(" into".to_owned(), MdModifier::default()),
            ]
        );
    }

    #[test]
    fn blockquote_blank_lines() {
        let mut parser = MdParser::new().unwrap();
        let source = r#"> First paragraph
>
> Second paragraph"#;
        let doc = MdDocument::new(source.to_owned(), &mut parser).unwrap();

        let sections: Vec<_> = doc.iter().collect();
        assert_eq!(sections.len(), 3, "should have 3 sections");
        // First paragraph
        assert!(matches!(sections[0].content, MdContent::Paragraph(_)));
        assert!(!sections[0].content.is_blank());
        // Blank line (Paragraph with empty content)
        assert!(matches!(sections[1].content, MdContent::Paragraph(_)));
        assert!(sections[1].content.is_blank());
        // Second paragraph
        assert!(matches!(sections[2].content, MdContent::Paragraph(_)));
        assert!(!sections[2].content.is_blank());
    }

    #[test]
    fn blockquote_styled_preserves_whitespace() {
        let mut parser = MdParser::new().unwrap();
        let doc =
            MdDocument::new("> you can _put_ **Markdown** into".to_owned(), &mut parser).unwrap();

        let sections: Vec<_> = doc.iter().collect();
        assert_eq!(sections.len(), 1);

        if let MdContent::Paragraph(spans) = &sections[0].content {
            // Verify whitespace between styled sections is preserved
            let content: String = spans.iter().map(|s| s.content.clone()).collect();
            assert_eq!(content, "you can put Markdown into");

            // Verify the space between styled elements is present
            assert_eq!(
                spans,
                &vec![
                    MdNode::new("you can ".to_owned(), MdModifier::default()),
                    MdNode::new("put".to_owned(), MdModifier::Emphasis),
                    MdNode::new(" ".to_owned(), MdModifier::default()),
                    MdNode::new("Markdown".to_owned(), MdModifier::StrongEmphasis),
                    MdNode::new(" into".to_owned(), MdModifier::default()),
                ]
            );
        } else {
            panic!("Expected Nodes content");
        }
    }
}
