use std::borrow::Cow;
use std::sync::Arc;

use bitflags::bitflags;
use regex::Regex;
use std::sync::LazyLock;
use tree_sitter::{Node, Parser, Tree, TreeCursor};
use unicode_width::UnicodeWidthStr;

use crate::Error;

/// Default list marker when none can be determined.
const DEFAULT_LIST_MARKER: &str = "-";

pub(crate) struct MdDocument<'a> {
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

pub(crate) struct MdIterator<'a> {
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
                    return Some(MdContent::Paragraph(vec![Span::new(
                        text.to_owned(),
                        Modifier::default(),
                    )]));
                };

                // Count blockquote depth for stripping markers from content
                let blockquote_depth = self
                    .context
                    .iter()
                    .filter(|(_, c)| matches!(c, MdContainer::Blockquote(_)))
                    .count();

                let mdspans = inline_node_to_spans(tree.root_node(), text, Modifier::default(), 0);
                let mdspans = split_newlines(mdspans);
                let mdspans = detect_bare_urls(mdspans);
                // Strip blockquote markers from line-start spans and filter empty/marker-only spans
                let mdspans: Vec<Span> = mdspans
                    .into_iter()
                    .map(|mut s| {
                        if s.modifiers.contains(Modifier::NewLine) {
                            s.content =
                                strip_blockquote_prefix(&s.content, blockquote_depth).into_owned();
                        }
                        s
                    })
                    .filter(|s| {
                        // Empty spans: only keep if they represent hard line breaks (NewLine)
                        if s.content.is_empty() {
                            return s.modifiers.contains(Modifier::NewLine);
                        }
                        // For line-start spans (NewLine), filter out blockquote-marker-only content
                        // that remains after stripping (e.g., a line that was just "> > ")
                        if s.modifiers.contains(Modifier::NewLine) {
                            return !is_blockquote_marker_only(s.content.trim());
                        }
                        // Mid-line spans are always kept (e.g., ">" from angle bracket URLs)
                        true
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
        let mut header: Vec<Vec<Span>> = Vec::new();
        let mut rows: Vec<Vec<Vec<Span>>> = Vec::new();
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
    fn parse_table_row(&mut self, row_node: Node<'a>) -> Vec<Vec<Span>> {
        let mut cells: Vec<Vec<Span>> = Vec::new();

        for child in row_node.children(&mut row_node.walk()) {
            if child.kind() == "pipe_table_cell" {
                let cell_text = self.source[child.byte_range()].trim();
                if cell_text.is_empty() {
                    cells.push(Vec::new());
                } else if let Some(tree) = self.inline_parser.parse(cell_text, None) {
                    let mdspans =
                        inline_node_to_spans(tree.root_node(), cell_text, Modifier::default(), 0);
                    let mdspans = detect_bare_urls(mdspans);
                    cells.push(mdspans);
                } else {
                    cells.push(vec![Span::new(cell_text.to_owned(), Modifier::default())]);
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
    pub struct Modifier: u32 {
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
        // Prefix/structural elements (added for mapper support)
        const BlockquoteBar = 1 << 10;
        const ListMarker = 1 << 11;
        const TableBorder = 1 << 12;
        const HorizontalRule = 1 << 13;
        // Wrapper elements for decorators
        const EmphasisWrapper = 1 << 14;
        const StrongEmphasisWrapper = 1 << 15;
        const CodeWrapper = 1 << 16;
        // Strikethrough
        const Strikethrough = 1 << 17;
        const StrikethroughWrapper = 1 << 18;
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Span {
    pub content: String,
    pub modifiers: Modifier,
    /// Original full content for spans that may be split (e.g., URLs that wrap across lines).
    /// When present, this should be used instead of `content` for semantic purposes like link targets.
    pub source_content: Option<Arc<str>>,
}

impl Span {
    pub fn new(content: String, extra: Modifier) -> Self {
        Span {
            content,
            modifiers: extra,
            source_content: None,
        }
    }

    /// Create a span with source content (for URLs that may be split across lines).
    pub fn with_source(content: String, modifiers: Modifier, source: Arc<str>) -> Self {
        Span {
            content,
            modifiers,
            source_content: Some(source),
        }
    }

    #[cfg(test)]
    pub fn link(description: &str, url: &str) -> Vec<Self> {
        vec![
            Self::new("[".to_owned(), Modifier::Link),
            Self::new(description.to_owned(), Modifier::Link),
            Self::new("]".to_owned(), Modifier::Link),
            Self::new("(".to_owned(), Modifier::Link),
            Self::new(url.to_owned(), Modifier::Link | Modifier::LinkURL),
            Self::new(")".to_owned(), Modifier::Link),
        ]
    }
}

impl From<String> for Span {
    fn from(value: String) -> Self {
        Span {
            content: value,
            modifiers: Modifier::default(),
            source_content: None,
        }
    }
}

#[cfg(test)]
impl From<&str> for Span {
    fn from(value: &str) -> Self {
        Self::from(value.to_owned())
    }
}

#[cfg(test)]
impl std::fmt::Display for Span {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.content)
    }
}

impl UnicodeWidthStr for Span {
    fn width(&self) -> usize {
        self.content.width()
    }

    fn width_cjk(&self) -> usize {
        self.content.width_cjk()
    }
}

/// A container in the document structure.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum MdContainer {
    List(ListMarker),
    ListItem(ListMarker),
    Blockquote(BlockquoteMarker),
}

/// Column alignment for tables.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) enum TableAlignment {
    #[default]
    Left,
    Center,
    Right,
}

/// Content of a markdown section.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum MdContent {
    Paragraph(Vec<Span>),
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
        header: Vec<Vec<Span>>,
        rows: Vec<Vec<Vec<Span>>>,
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
pub(crate) struct ListMarker {
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
pub(crate) struct BlockquoteMarker;

/// A markdown section with its content and nesting path.
#[derive(Debug)]
pub(crate) struct MdSection {
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
    let mut has_space = false;
    while let Some(c) = chars.next() {
        if c == '>' {
            if chars.peek() == Some(&' ') {
                chars.next();
                has_space = true;
            }
        } else {
            return false;
        }
    }
    // A proper blockquote marker has "> " pattern, not just ">" or ">>"
    // A standalone ">" is likely part of angle bracket URL syntax, not a blockquote marker
    has_space
}

#[expect(clippy::string_slice)]
fn inline_node_to_spans(node: Node, source: &str, extra: Modifier, _depth: usize) -> Vec<Span> {
    let kind = node.kind();

    if kind.contains("delimiter") {
        return vec![];
    }

    let current_extra = match kind {
        "emphasis" => Modifier::Emphasis,
        "strong_emphasis" => Modifier::StrongEmphasis,
        "strikethrough" => Modifier::Strikethrough,
        "code_span" => {
            // Strip the backtick delimiters from code span content
            let content = &source[node.byte_range()];
            let stripped = content.trim_start_matches('`').trim_end_matches('`').trim(); // Also trim inner whitespace that some code spans have
            return vec![Span::new(stripped.to_owned(), extra.union(Modifier::Code))];
        }
        "hard_line_break" | "soft_break" => {
            // GFM hard line break (two trailing spaces + newline) or soft break
            return vec![Span::new(String::new(), extra.union(Modifier::NewLine))];
        }
        "[" | "]" => Modifier::LinkDescriptionWrapper,
        "(" | ")" => Modifier::LinkURLWrapper,
        "link_text" => Modifier::LinkDescription,
        "inline_link" => Modifier::Link,
        "image" => Modifier::Image,
        "link_destination" => {
            let url = source[node.byte_range()].to_owned();
            let source_content = Arc::from(url.as_str());
            return vec![Span::with_source(
                url,
                extra.union(Modifier::LinkURL),
                source_content,
            )];
        }
        _ => Modifier::default(),
    };
    let extra = extra.union(current_extra);

    let (extra, newline_offset) = if source.as_bytes()[node.start_byte()] == b'\n' {
        (extra.union(Modifier::NewLine), 1)
    } else {
        (extra, 0)
    };

    if node.child_count() == 0 {
        return vec![Span::new(
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
            spans.push(Span::new(source[pos..child.start_byte()].to_owned(), extra));
            if source.as_bytes()[child.start_byte() - 1] == b'\n' {
                ended_with_newline = true;
            }
        }
        let extra = if ended_with_newline {
            extra.union(Modifier::NewLine)
        } else {
            extra
        };
        spans.extend(inline_node_to_spans(child, source, extra, _depth + 1));
        pos = child.end_byte();
    }

    if pos < node.end_byte() {
        spans.push(Span::new(source[pos..node.end_byte()].to_owned(), extra));
    }

    spans
}

#[inline]
fn is_punctuation(kind: &str, parent_modifier: Modifier) -> bool {
    match kind {
        "(" | ")" | "[" | "]" if parent_modifier == Modifier::Link => false,
        "!" | "\"" | "#" | "$" | "%" | "&" | "'" | "(" | ")" | "*" | "+" | "," | "-" | "."
        | "/" | ":" | ";" | "<" | "=" | ">" | "?" | "@" | "[" | "\\" | "]" | "^" | "_" | "`"
        | "{" | "|" | "}" | "~" => true,
        _ => false,
    }
}

/// Regex for detecting bare URLs (http:// or https://).
static URL_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"https?://[^\s<>\[\]()]+").unwrap());

/// Detect bare URLs in spans and mark them with LinkURL modifier.
/// Skips spans that already have link-related modifiers.
fn detect_bare_urls(mdspans: Vec<Span>) -> Vec<Span> {
    let mut result = Vec::with_capacity(mdspans.len());

    for span in mdspans {
        // Skip spans that are already part of a link or code
        if span
            .modifiers
            .intersects(Modifier::Link | Modifier::LinkURL | Modifier::Code)
        {
            result.push(span);
            continue;
        }

        // Find all URL matches in this span
        let mut last_end = 0;
        let content = &span.content;
        let mut found_urls = false;
        // Track whether we've emitted the first span (which keeps NewLine if present)
        let mut first_emitted = false;
        // Base modifiers without NewLine - we only want NewLine on the first span
        let base_modifiers = span.modifiers.difference(Modifier::NewLine);

        for mat in URL_REGEX.find_iter(content) {
            found_urls = true;

            // Text before the URL
            if mat.start() > last_end {
                let mods = if first_emitted {
                    base_modifiers
                } else {
                    first_emitted = true;
                    span.modifiers
                };
                result.push(Span::new(content[last_end..mat.start()].to_owned(), mods));
            }

            // Opening wrapper - only keep NewLine if this is the first span emitted
            let wrapper_mods = if first_emitted {
                base_modifiers | Modifier::LinkURLWrapper
            } else {
                first_emitted = true;
                span.modifiers | Modifier::LinkURLWrapper
            };
            result.push(Span::new("(".to_owned(), wrapper_mods));

            // The URL itself - marked as LinkURL (never first, wrapper is always before)
            let url = mat.as_str().to_owned();
            let source_content = Arc::from(url.as_str());
            result.push(Span::with_source(
                url,
                base_modifiers | Modifier::LinkURL,
                source_content,
            ));

            // Closing wrapper
            result.push(Span::new(
                ")".to_owned(),
                base_modifiers | Modifier::LinkURLWrapper,
            ));

            last_end = mat.end();
        }

        if found_urls {
            // Text after the last URL
            if last_end < content.len() {
                result.push(Span::new(content[last_end..].to_owned(), base_modifiers));
            }
        } else {
            // No URLs found, keep original span
            result.push(span);
        }
    }

    result
}

fn split_newlines(mdspans: Vec<Span>) -> Vec<Span> {
    let mut result = Vec::with_capacity(mdspans.len());
    for mdspan in mdspans {
        // Preserve empty spans that have NewLine flag (from hard_line_break)
        if mdspan.content.is_empty() && mdspan.modifiers.contains(Modifier::NewLine) {
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
            result.push(Span {
                content: part.to_owned(),
                modifiers: if first {
                    first = false;
                    mdspan.modifiers
                } else {
                    mdspan.modifiers.union(Modifier::NewLine)
                },
                // Preserve source_content across all split parts
                source_content: mdspan.source_content.clone(),
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
            Span::new("one line".to_owned(), Modifier::default()),
            Span::new(".".to_owned(), Modifier::default()),
            Span::new("\nanother line".to_owned(), Modifier::NewLine),
            Span::new(".".to_owned(), Modifier::default()),
        ]);
        assert_eq!(
            mdspans,
            vec![
                Span::new("one line".to_owned(), Modifier::default()),
                Span::new(".".to_owned(), Modifier::default()),
                Span::new("another line".to_owned(), Modifier::NewLine),
                Span::new(".".to_owned(), Modifier::default()),
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

    #[test]
    fn detect_bare_url() {
        let spans = vec![Span::new(
            "Check https://example.com for more.".to_owned(),
            Modifier::default(),
        )];
        let result = detect_bare_urls(spans);
        assert_eq!(result.len(), 5);
        assert_eq!(result[0].content, "Check ");
        assert!(!result[0].modifiers.contains(Modifier::LinkURL));
        assert_eq!(result[1].content, "(");
        assert!(result[1].modifiers.contains(Modifier::LinkURLWrapper));
        assert_eq!(result[2].content, "https://example.com");
        assert!(result[2].modifiers.contains(Modifier::LinkURL));
        assert_eq!(result[3].content, ")");
        assert!(result[3].modifiers.contains(Modifier::LinkURLWrapper));
        assert_eq!(result[4].content, " for more.");
        assert!(!result[4].modifiers.contains(Modifier::LinkURL));
    }

    #[test]
    fn detect_bare_url_preserves_existing_modifiers() {
        let spans = vec![Span::new(
            "See https://example.com now".to_owned(),
            Modifier::Emphasis,
        )];
        let result = detect_bare_urls(spans);
        assert_eq!(result.len(), 5);
        assert!(result[0].modifiers.contains(Modifier::Emphasis));
        assert!(result[1].modifiers.contains(Modifier::Emphasis));
        assert!(result[1].modifiers.contains(Modifier::LinkURLWrapper));
        assert!(result[2].modifiers.contains(Modifier::Emphasis));
        assert!(result[2].modifiers.contains(Modifier::LinkURL));
        assert!(result[3].modifiers.contains(Modifier::Emphasis));
        assert!(result[3].modifiers.contains(Modifier::LinkURLWrapper));
        assert!(result[4].modifiers.contains(Modifier::Emphasis));
    }

    #[test]
    fn detect_bare_url_skips_existing_links() {
        let spans = vec![Span::new(
            "https://example.com".to_owned(),
            Modifier::Link | Modifier::LinkURL,
        )];
        let result = detect_bare_urls(spans.clone());
        assert_eq!(result, spans);
    }

    #[test]
    fn detect_bare_url_skips_code() {
        let spans = vec![Span::new("https://example.com".to_owned(), Modifier::Code)];
        let result = detect_bare_urls(spans.clone());
        assert_eq!(result, spans);
    }

    #[test]
    fn angle_bracket_url_preserved() {
        // Angle bracket URLs like <http://example.com> should preserve both < and >
        let spans = vec![Span::new(
            "<http://www.example.com>".to_owned(),
            Modifier::default(),
        )];
        let result = detect_bare_urls(spans);
        assert_eq!(result.len(), 5);
        assert_eq!(result[0].content, "<");
        assert_eq!(result[1].content, "(");
        assert!(result[1].modifiers.contains(Modifier::LinkURLWrapper));
        assert_eq!(result[2].content, "http://www.example.com");
        assert!(result[2].modifiers.contains(Modifier::LinkURL));
        assert_eq!(result[3].content, ")");
        assert!(result[3].modifiers.contains(Modifier::LinkURLWrapper));
        assert_eq!(result[4].content, ">");
    }
}
