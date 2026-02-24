#![cfg_attr(docsrs, feature(doc_auto_cfg))]

//! mdfrier - Deep fry markdown for [mdfried](https://crates.io/crates/mdfried).
//!
//! This crate parses markdown with tree-sitter-md into wrapped lines for a fixed width styled
//! output.
//!
//! This isn't as straightforward as wrapping the source and then highlighting syntax, because the
//! wrapping relies on markdown context. The process is:
//!
//! 1. Parse into raw lines with nodes
//! 2. Map the node's markdown symbols (optionally, because we want to strip e.g. `*` when
//!    highlighting with color later)
//! 3. Wrap the lines of nodes to a maximum width
//! 4. ???
//!
//! At step 4, the users of this library will typically convert the wrapped lines of nodes with
//! their style information to whatever the target is: ANSI escape sequences, or whatever some
//! their library expects.
//!
//! There is a `ratatui` feature that enables the [`ratatui`] module, which does exactly this, for
//! [ratatui](https://ratatui.rs).
//!
//! The [`Mapper`] trait controls decorator symbols (e.g., blockquote bar, link brackets).
//! The optional `ratatui` feature provides the [`ratatui::Theme`] trait that combines [`Mapper`]
//! with [`ratatui::style::Style`](https://docs.rs/ratatui/latest/ratatui/style/struct.Style.html) conversion.
//!
//! # Examples
//!
//! [`StyledMapper`] is the default goal of this crate. It heavily maps markdown symbols, and
//! strips many, with the intention of adding syles (color, bold, italics...) later, after wrapping.
//! That is, it does not "stylize" the markdown, but is intented *for* stylizing later.
//!
//! The styles should be applied when iterating over the [`Line`]'s [`Span`]s.
//! ```
//! use mdfrier::{MdFrier, Line, Span, Mapper, DefaultMapper, StyledMapper};
//!
//! let mut frier = MdFrier::new().unwrap();
//!
//! // StyledMapper removes decorators (for use with colors/bold/italic styling)
//! let lines = frier.parse(80, "*emphasis* and **strong**", &StyledMapper);
//! let text: String = lines.iter()
//!     .flat_map(|l: &Line| l.spans.iter().map(|s: &Span|
//!         // We should really add colors from `s.modifiers` here!
//!         s.content.as_str()
//!     ))
//!     .collect();
//! assert_eq!(text, "emphasis and strong");
//! ```
//!
//! A custom mapper should implement the [`Mapper`] trait. For example, here we replace some
//! markdown delimiters with fancy symbols.
//! ```
//! use mdfrier::{MdFrier, Mapper};
//!
//! struct FancyMapper;
//! impl Mapper for FancyMapper {
//!     fn emphasis_open(&self) -> &str { "♥" }
//!     fn emphasis_close(&self) -> &str { "♥" }
//!     fn strong_open(&self) -> &str { "✦" }
//!     fn strong_close(&self) -> &str { "✦" }
//!     fn blockquote_bar(&self) -> &str { "➤ " }
//! }
//!
//! let mut frier = MdFrier::new().unwrap();
//!
//! let lines = frier.parse(80, "Hello *world*!\n\n> Quote\n\n**Bold**", &FancyMapper);
//! let mut output = String::new();
//! for line in lines {
//!     for span in line.spans {
//!         output.push_str(&span.content);
//!     }
//!     output.push('\n');
//! }
//! assert_eq!(output, "Hello ♥world♥!\n\n➤ Quote\n\n✦Bold✦\n");
//! ```
//!
//! A [`DefaultMapper`] exists, which could be used only style, preserving the markdown content.
//! Note that it would be much more efficient to use the
//! [`tree-sitter-md`](https://crates.io/crates/tree-sitter-md) crate directly instead,
//! since it operates with byte-ranges of the original text. Think editor syntax highlighting.
//! ```
//! use mdfrier::{MdFrier, DefaultMapper};
//!
//! let mut frier = MdFrier::new().unwrap();
//!
//! let lines = frier.parse(80, "*emphasis* and **strong**", &DefaultMapper);
//! let text: String = lines.iter()
//!     .flat_map(|l| l.spans.iter().map(|s| s.content.as_str()))
//!     .collect();
//! assert_eq!(text, "*emphasis* and **strong**");
//!
//! ```

mod lines;
pub mod mapper;
mod markdown;
pub mod sections;
mod wrap;

#[cfg(feature = "ratatui")]
pub mod ratatui;

use std::collections::VecDeque;

use tree_sitter::Parser;
use unicode_width::UnicodeWidthStr as _;

use lines::{RawLine, RawLineKind};
use markdown::{MdContainer, MdContent, MdDocument, MdSection, TableAlignment};

pub use lines::BulletStyle;
pub use mapper::{DefaultMapper, Mapper, StyledMapper};
pub use markdown::{Modifier, SourceContent, Span};

// Re-export for internal use by lines module
pub(crate) use lines::MdLineContainer;

use crate::{markdown::MdIterator, sections::SectionIterator};

// ============================================================================
// Public output types
// ============================================================================

/// A single output line from the markdown parser.
///
/// This is the final, flattened representation with all decorators applied
/// and nesting converted to prefix spans.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    /// The text spans making up this line, including any prefix spans
    /// (blockquote bars, list markers) that were added from nesting.
    pub spans: Vec<Span>,
    /// The kind of content this line represents.
    pub kind: LineKind,
}

/// The kind of content a line represents.
#[derive(Debug, Clone, PartialEq)]
pub enum LineKind {
    /// Regular text paragraph.
    Paragraph,
    /// Header line with tier (1-6).
    Header(u8),
    /// Code block line with language.
    CodeBlock { language: String },
    /// Horizontal rule (content is in spans).
    HorizontalRule,
    /// Table data row.
    TableRow { is_header: bool },
    /// Table border/separator.
    TableBorder,
    /// Image reference.
    Image { url: String, description: String },
    /// Blank line.
    Blank,
}

/// Failed to parse markdown.
#[derive(Debug)]
pub struct MarkdownParseError;

impl std::fmt::Display for MarkdownParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Failed to parse markdown")
    }
}

impl std::error::Error for MarkdownParseError {}

/// The main markdown parser struct.
///
/// Wraps tree-sitter parsers and provides a simple interface for parsing
/// markdown text into lines.
pub struct MdFrier {
    parser: Parser,
    inline_parser: Parser,
}

impl MdFrier {
    /// Create a new MdFrier instance.
    pub fn new() -> Result<Self, MarkdownParseError> {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_md::LANGUAGE.into())
            .ok()
            .ok_or(MarkdownParseError)?;

        let mut inline_parser = Parser::new();
        inline_parser
            .set_language(&tree_sitter_md::INLINE_LANGUAGE.into())
            .ok()
            .ok_or(MarkdownParseError)?;

        Ok(Self {
            parser,
            inline_parser,
        })
    }

    /// Parse markdown text and return a vector of `MdLine` items.
    ///
    /// The mapper controls how decorators are rendered (link brackets,
    /// blockquote bars, list markers, etc.). Use `DefaultMapper` for
    /// plain ASCII output, or implement your own `Mapper` for custom symbols.
    ///
    /// # Arguments
    ///
    /// * `width` - The terminal width for line wrapping
    /// * `text` - The markdown text to parse
    /// * `mapper` - The mapper to use for content transformation
    pub fn parse<M: Mapper>(&mut self, width: u16, text: &str, mapper: &M) -> Vec<Line> {
        let doc = match MdDocument::new(text, &mut self.parser, &mut self.inline_parser) {
            Ok(doc) => doc,
            Err(_) => return Vec::new(),
        };

        let mut processor = RawLineProcessor::new(doc, width);
        let raw_lines = processor.collect_all();

        raw_lines
            .into_iter()
            .map(|raw| convert_raw_to_mdline(raw, width, mapper))
            .collect()
    }

    pub fn parse_sections<'a, M: Mapper>(
        &'a mut self,
        width: u16,
        text: &'a str,
        mapper: &'a M,
    ) -> Result<SectionIterator<'a, M>, MarkdownParseError> {
        let tree = self.parser.parse(text, None).ok_or(MarkdownParseError)?;
        let iter = MdIterator::new(tree, &mut self.inline_parser, text);
        Ok(SectionIterator::new(iter, width, mapper))
    }
}

// ============================================================================
// Conversion from RawLine to MdLine
// ============================================================================

/// Convert a RawLine to MdLine by applying the mapper and flattening nesting.
pub(crate) fn convert_raw_to_mdline<M: Mapper>(raw: RawLine, width: u16, mapper: &M) -> Line {
    let RawLine { spans, meta } = raw;

    // Build prefix spans from nesting
    let prefix_spans = nesting_to_prefix_spans(&meta.nesting, mapper);
    let prefix_width: usize = prefix_spans.iter().map(|s| s.content.width()).sum();

    // Apply mapper to content spans (link decorators, etc.)
    let mapped_spans = apply_mapper_to_spans(spans, mapper);

    // Combine prefix and content
    let mut final_spans = prefix_spans;

    // Handle special line kinds
    match &meta.kind {
        RawLineKind::HorizontalRule => {
            let available = (width as usize).saturating_sub(prefix_width);
            final_spans.push(Span::new(
                mapper.horizontal_rule_char().repeat(available),
                Modifier::HorizontalRule,
            ));
            Line {
                spans: final_spans,
                kind: LineKind::HorizontalRule,
            }
        }
        RawLineKind::TableBorder {
            column_info,
            position,
        } => {
            // Build table border spans
            let border_spans = build_table_border_spans(column_info, *position, mapper);
            final_spans.extend(border_spans);
            Line {
                spans: final_spans,
                kind: LineKind::TableBorder,
            }
        }
        RawLineKind::TableRow {
            cells,
            column_info,
            is_header,
        } => {
            // Build table row spans
            let row_spans = build_table_row_spans(cells, column_info, mapper);
            final_spans.extend(row_spans);
            Line {
                spans: final_spans,
                kind: LineKind::TableRow {
                    is_header: *is_header,
                },
            }
        }
        RawLineKind::CodeBlock { language } => {
            // Pad code to fill width with background
            let available = (width as usize).saturating_sub(prefix_width);
            let content_width: usize = mapped_spans.iter().map(|s| s.content.width()).sum();
            let padding = available.saturating_sub(content_width);

            final_spans.extend(mapped_spans);
            if padding > 0 {
                final_spans.push(Span::new(" ".repeat(padding), Modifier::Code));
            }
            Line {
                spans: final_spans,
                kind: LineKind::CodeBlock {
                    language: language.clone(),
                },
            }
        }
        RawLineKind::Paragraph => {
            final_spans.extend(mapped_spans);
            Line {
                spans: final_spans,
                kind: LineKind::Paragraph,
            }
        }
        RawLineKind::Header(tier) => {
            final_spans.extend(mapped_spans);
            Line {
                spans: final_spans,
                kind: LineKind::Header(*tier),
            }
        }
        RawLineKind::Image { url, description } => {
            // TODO: helper fn
            let image_spans = vec![
                Span {
                    content: "![".to_owned(),
                    modifiers: Modifier::LinkDescriptionWrapper,
                    source_content: None,
                },
                Span {
                    content: "Loading...".to_owned(),
                    modifiers: Modifier::LinkURL,
                    source_content: None,
                },
                Span {
                    content: "]".to_owned(),
                    modifiers: Modifier::LinkDescriptionWrapper,
                    source_content: None,
                },
                Span {
                    content: "(".to_owned(),
                    modifiers: Modifier::LinkURLWrapper,
                    source_content: None,
                },
                Span {
                    content: url.to_owned(),
                    modifiers: Modifier::LinkDescription,
                    source_content: None,
                },
                Span {
                    content: ")".to_owned(),
                    modifiers: Modifier::LinkURLWrapper,
                    source_content: None,
                },
            ];
            Line {
                spans: image_spans,
                kind: LineKind::Image {
                    url: url.clone(),
                    description: description.clone(),
                },
            }
        }
        RawLineKind::Blank => Line {
            spans: final_spans,
            kind: LineKind::Blank,
        },
    }
}

/// Apply mapper transformations to content spans.
/// This handles link wrappers and inserts emphasis/strong/code/strikethrough decorators.
fn apply_mapper_to_spans<M: Mapper>(spans: Vec<Span>, mapper: &M) -> Vec<Span> {
    let mut result = Vec::with_capacity(spans.len() * 2);
    let mut prev_emphasis = false;
    let mut prev_strong = false;
    let mut prev_code = false;
    let mut prev_strikethrough = false;

    for mut span in spans {
        let has_emphasis = span.modifiers.contains(Modifier::Emphasis);
        let has_strong = span.modifiers.contains(Modifier::StrongEmphasis);
        let has_code = span.modifiers.contains(Modifier::Code);
        let has_strikethrough = span.modifiers.contains(Modifier::Strikethrough);

        // Close decorators that ended (in reverse order of nesting)
        if prev_code && !has_code {
            let close = mapper.code_close();
            if !close.is_empty() {
                result.push(Span::new(close.to_owned(), Modifier::CodeWrapper));
            }
        }
        if prev_strikethrough && !has_strikethrough {
            let close = mapper.strikethrough_close();
            if !close.is_empty() {
                result.push(Span::new(close.to_owned(), Modifier::StrikethroughWrapper));
            }
        }
        if prev_strong && !has_strong {
            let close = mapper.strong_close();
            if !close.is_empty() {
                result.push(Span::new(close.to_owned(), Modifier::StrongEmphasisWrapper));
            }
        }
        if prev_emphasis && !has_emphasis {
            let close = mapper.emphasis_close();
            if !close.is_empty() {
                result.push(Span::new(close.to_owned(), Modifier::EmphasisWrapper));
            }
        }

        // Open decorators that started
        if has_emphasis && !prev_emphasis {
            let open = mapper.emphasis_open();
            if !open.is_empty() {
                result.push(Span::new(open.to_owned(), Modifier::EmphasisWrapper));
            }
        }
        if has_strong && !prev_strong {
            let open = mapper.strong_open();
            if !open.is_empty() {
                result.push(Span::new(open.to_owned(), Modifier::StrongEmphasisWrapper));
            }
        }
        if has_strikethrough && !prev_strikethrough {
            let open = mapper.strikethrough_open();
            if !open.is_empty() {
                result.push(Span::new(open.to_owned(), Modifier::StrikethroughWrapper));
            }
        }
        if has_code && !prev_code {
            let open = mapper.code_open();
            if !open.is_empty() {
                result.push(Span::new(open.to_owned(), Modifier::CodeWrapper));
            }
        }

        // Transform link wrappers
        if span.modifiers.contains(Modifier::LinkDescriptionWrapper) {
            span.content = if span.content == "[" {
                mapper.link_desc_open().to_owned()
            } else {
                mapper.link_desc_close().to_owned()
            };
        } else if span.modifiers.contains(Modifier::LinkURLWrapper) {
            span.content = if span.content == "(" {
                mapper.link_url_open().to_owned()
            } else {
                mapper.link_url_close().to_owned()
            };
        }

        // Hide (omit) URL spans if configured so, is LinkURL or LinkURLWrapper (e.g. the
        // `(http://url)` part, and this is not a BareLink. Bare links have no description, so
        // there's nothing we can omit.
        let hide = mapper.hide_urls()
            && span
                .modifiers
                .intersects(Modifier::LinkURL | Modifier::LinkURLWrapper)
            && !span.modifiers.contains(Modifier::BareLink);
        if !hide {
            result.push(span);
        }

        prev_emphasis = has_emphasis;
        prev_strong = has_strong;
        prev_code = has_code;
        prev_strikethrough = has_strikethrough;
    }

    // Close any remaining open decorators at end
    if prev_code {
        let close = mapper.code_close();
        if !close.is_empty() {
            result.push(Span::new(close.to_owned(), Modifier::CodeWrapper));
        }
    }
    if prev_strikethrough {
        let close = mapper.strikethrough_close();
        if !close.is_empty() {
            result.push(Span::new(close.to_owned(), Modifier::StrikethroughWrapper));
        }
    }
    if prev_strong {
        let close = mapper.strong_close();
        if !close.is_empty() {
            result.push(Span::new(close.to_owned(), Modifier::StrongEmphasisWrapper));
        }
    }
    if prev_emphasis {
        let close = mapper.emphasis_close();
        if !close.is_empty() {
            result.push(Span::new(close.to_owned(), Modifier::EmphasisWrapper));
        }
    }

    result
}

/// Build prefix spans from nesting containers.
fn nesting_to_prefix_spans<M: Mapper>(nesting: &[MdLineContainer], mapper: &M) -> Vec<Span> {
    let mut spans = Vec::new();
    let last_list_idx = nesting
        .iter()
        .rposition(|c| matches!(c, MdLineContainer::ListItem { .. }));

    for (i, container) in nesting.iter().enumerate() {
        match container {
            MdLineContainer::Blockquote => {
                spans.push(Span::new(
                    mapper.blockquote_bar().to_owned(),
                    Modifier::BlockquoteBar,
                ));
            }
            MdLineContainer::ListItem {
                marker,
                continuation,
            } => {
                if Some(i) == last_list_idx && !*continuation {
                    let marker_text = match marker {
                        lines::ListMarker::Unordered(b) => mapper.unordered_bullet(*b).to_owned(),
                        lines::ListMarker::Ordered(n) => mapper.ordered_marker(*n),
                        lines::ListMarker::TaskChecked(b) => {
                            // "- [x] " or similar
                            format!("{}{}", mapper.unordered_bullet(*b), mapper.task_checked())
                        }
                        lines::ListMarker::TaskUnchecked(b) => {
                            // "- [ ] " or similar
                            format!("{}{}", mapper.unordered_bullet(*b), mapper.task_unchecked())
                        }
                    };
                    spans.push(Span::new(marker_text, Modifier::ListMarker));
                } else {
                    // Indentation for outer/continuation items
                    let indent_width = marker_width_for_mapper(marker, mapper);
                    spans.push(Span::new(" ".repeat(indent_width), Modifier::empty()));
                }
            }
        }
    }
    spans
}

/// Calculate marker width using mapper's symbols.
fn marker_width_for_mapper<M: Mapper>(marker: &lines::ListMarker, mapper: &M) -> usize {
    match marker {
        lines::ListMarker::Unordered(b) => mapper.unordered_bullet(*b).width(),
        lines::ListMarker::Ordered(n) => mapper.ordered_marker(*n).width(),
        lines::ListMarker::TaskChecked(b) => {
            // "- [x] " = bullet + checkbox
            mapper.unordered_bullet(*b).width() + mapper.task_checked().width()
        }
        lines::ListMarker::TaskUnchecked(b) => {
            // "- [ ] " = bullet + checkbox
            mapper.unordered_bullet(*b).width() + mapper.task_unchecked().width()
        }
    }
}

/// Build table border spans.
fn build_table_border_spans<M: Mapper>(
    column_info: &lines::TableColumnInfo,
    position: lines::BorderPosition,
    mapper: &M,
) -> Vec<Span> {
    let mut spans = Vec::new();

    let (left, mid, right) = match position {
        lines::BorderPosition::Top => (
            mapper.table_top_left(),
            mapper.table_top_junction(),
            mapper.table_top_right(),
        ),
        lines::BorderPosition::HeaderSeparator => (
            mapper.table_left_junction(),
            mapper.table_cross(),
            mapper.table_right_junction(),
        ),
        lines::BorderPosition::Bottom => (
            mapper.table_bottom_left(),
            mapper.table_bottom_junction(),
            mapper.table_bottom_right(),
        ),
    };

    let horizontal = mapper.table_horizontal();
    let num_cols = column_info.widths.len();

    spans.push(Span::new(left.to_owned(), Modifier::TableBorder));
    for (i, &col_w) in column_info.widths.iter().enumerate() {
        spans.push(Span::new(horizontal.repeat(col_w), Modifier::TableBorder));
        if i < num_cols - 1 {
            spans.push(Span::new(mid.to_owned(), Modifier::TableBorder));
        }
    }
    spans.push(Span::new(right.to_owned(), Modifier::TableBorder));

    spans
}

/// Build table row spans.
fn build_table_row_spans<M: Mapper>(
    cells: &[Vec<Span>],
    column_info: &lines::TableColumnInfo,
    mapper: &M,
) -> Vec<Span> {
    let mut spans = Vec::new();
    let vertical = mapper.table_vertical();

    spans.push(Span::new(vertical.to_owned(), Modifier::TableBorder));

    for (i, col_width) in column_info.widths.iter().enumerate() {
        let alignment = column_info
            .alignments
            .get(i)
            .copied()
            .unwrap_or(TableAlignment::Left);

        let cell_spans = cells.get(i).map_or(&[][..], |v| v.as_slice());
        let content_width: usize = cell_spans.iter().map(|s| s.content.width()).sum();
        let inner_width = col_width.saturating_sub(2);
        let padding_total = inner_width.saturating_sub(content_width);

        let (left_pad, right_pad) = match alignment {
            TableAlignment::Center => (padding_total / 2, padding_total - padding_total / 2),
            TableAlignment::Right => (padding_total, 0),
            TableAlignment::Left => (0, padding_total),
        };

        // Left padding + space
        spans.push(Span::new(
            format!("{}{}", " ", " ".repeat(left_pad)),
            Modifier::empty(),
        ));

        // Cell content (apply mapper to nested spans)
        for node in cell_spans {
            let mut mapped_node = node.clone();
            if mapped_node
                .modifiers
                .contains(Modifier::LinkDescriptionWrapper)
            {
                mapped_node.content = if mapped_node.content == "[" {
                    mapper.link_desc_open().to_owned()
                } else {
                    mapper.link_desc_close().to_owned()
                };
            } else if mapped_node.modifiers.contains(Modifier::LinkURLWrapper) {
                mapped_node.content = if mapped_node.content == "(" {
                    mapper.link_url_open().to_owned()
                } else {
                    mapper.link_url_close().to_owned()
                };
            }
            spans.push(mapped_node);
        }

        // Right padding + space
        spans.push(Span::new(
            format!("{}{}", " ".repeat(right_pad), " "),
            Modifier::empty(),
        ));

        spans.push(Span::new(vertical.to_owned(), Modifier::TableBorder));
    }

    // Fill missing columns
    let num_cols = column_info.widths.len();
    for i in cells.len()..num_cols {
        let col_width = column_info.widths.get(i).copied().unwrap_or(3);
        spans.push(Span::new(" ".repeat(col_width), Modifier::empty()));
        spans.push(Span::new(vertical.to_owned(), Modifier::TableBorder));
    }

    spans
}

// ============================================================================
// Raw line processing (internal)
// ============================================================================

/// Processor for collecting raw lines from parsed markdown.
struct RawLineProcessor {
    sections: Vec<MdSection>,
    section_idx: usize,
    width: u16,
    pending_lines: VecDeque<RawLine>,
    needs_blank: bool,
    prev_nesting: Vec<MdContainer>,
    prev_was_blank: bool,
    prev_in_list: bool,
}

impl RawLineProcessor {
    fn new(doc: MdDocument, width: u16) -> Self {
        let sections: Vec<_> = doc.into_sections().collect();
        Self {
            sections,
            section_idx: 0,
            width,
            pending_lines: VecDeque::new(),
            needs_blank: false,
            prev_nesting: Vec::new(),
            prev_was_blank: false,
            prev_in_list: false,
        }
    }

    fn collect_all(&mut self) -> Vec<RawLine> {
        let mut result = Vec::new();
        while self.process_next_section() {
            while let Some(line) = self.pending_lines.pop_front() {
                result.push(line);
            }
        }
        result
    }

    fn process_next_section(&mut self) -> bool {
        if self.section_idx >= self.sections.len() {
            return false;
        }

        let section = &self.sections[self.section_idx];
        self.section_idx += 1;

        let in_list = section
            .nesting
            .iter()
            .any(|c| matches!(c, MdContainer::ListItem(_)));

        let is_blank_line = section.content.is_blank();

        // Nesting change detection - compare container types, not exact values
        let container_type_matches = |a: &MdContainer, b: &MdContainer| -> bool {
            matches!(
                (a, b),
                (MdContainer::List(_), MdContainer::List(_))
                    | (MdContainer::ListItem(_), MdContainer::ListItem(_))
                    | (MdContainer::Blockquote(_), MdContainer::Blockquote(_))
            )
        };
        let is_type_prefix = |shorter: &[MdContainer], longer: &[MdContainer]| -> bool {
            !shorter.is_empty()
                && shorter.len() < longer.len()
                && shorter
                    .iter()
                    .zip(longer.iter())
                    .all(|(a, b)| container_type_matches(a, b))
        };
        let nesting_change = is_type_prefix(&self.prev_nesting, &section.nesting)
            || is_type_prefix(&section.nesting, &self.prev_nesting);

        // Count list nesting depth (number of List containers)
        let list_depth = |nesting: &[MdContainer]| -> usize {
            nesting
                .iter()
                .filter(|c| matches!(c, MdContainer::List(_)))
                .count()
        };
        let curr_list_depth = list_depth(&section.nesting);
        let prev_list_depth = list_depth(&self.prev_nesting);

        // Check if both sections are at the same top-level list (depth 1) with same List container
        let same_top_level_list =
            if in_list && self.prev_in_list && curr_list_depth == 1 && prev_list_depth == 1 {
                // Compare first List container only for top-level items
                let curr_list = section
                    .nesting
                    .iter()
                    .find(|c| matches!(c, MdContainer::List(_)));
                let prev_list = self
                    .prev_nesting
                    .iter()
                    .find(|c| matches!(c, MdContainer::List(_)));
                curr_list == prev_list
            } else {
                false
            };

        // For nested lists (depth > 1), treat all items at same depth as same context
        // to avoid blanks between items with different markers
        let same_nested_context =
            in_list && self.prev_in_list && curr_list_depth > 1 && prev_list_depth > 1;

        let same_list_context = same_top_level_list || same_nested_context;

        // Check if we're exiting to a new top-level list (not part of previous ancestry)
        let exiting_to_new_top_level =
            nesting_change && curr_list_depth == 1 && prev_list_depth > 1 && {
                // Check if the current top-level List was in the previous nesting
                let curr_list = section
                    .nesting
                    .iter()
                    .find(|c| matches!(c, MdContainer::List(_)));
                let was_in_prev = curr_list.is_none_or(|cl| self.prev_nesting.contains(cl));
                !was_in_prev
            };

        // Allow blank lines before continuation paragraphs or between different top-level lists,
        // but not during nesting changes (unless exiting to a new top-level list)
        let should_emit_blank = self.needs_blank
            && (!same_list_context || section.is_list_continuation)
            && !is_blank_line
            && !self.prev_was_blank
            && (!nesting_change || exiting_to_new_top_level);

        if should_emit_blank {
            self.pending_lines.push_back(RawLine::blank());
        }

        // Only headers don't need space after
        self.needs_blank = !matches!(section.content, MdContent::Header { .. });
        // Clone nesting for comparison in next iteration
        self.prev_nesting.clone_from(&section.nesting);
        self.prev_was_blank = is_blank_line;
        self.prev_in_list = in_list;

        let lines = lines::section_to_raw_lines(self.width, section);
        self.pending_lines.extend(lines);

        true
    }
}

#[cfg(test)]
#[expect(clippy::unwrap_used)]
mod tests {
    use crate::markdown::SourceContent;

    use super::*;
    use pretty_assertions::assert_eq;

    /// Convert MdLines to a string representation for testing.
    /// With the new flat API, all prefix spans are included in spans.
    fn lines_to_string(lines: &[Line]) -> String {
        lines
            .iter()
            .map(|line| {
                if matches!(line.kind, LineKind::Blank) {
                    String::new()
                } else {
                    line.spans.iter().map(|s| s.content.as_str()).collect()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn parse_simple_text() {
        let mut frier = MdFrier::new().unwrap();
        let lines = frier.parse(80, "Hello world!", &DefaultMapper);
        assert_eq!(lines.len(), 1);

        let line = &lines[0];
        assert_eq!(line.spans.len(), 1);
        assert_eq!(line.spans[0].content, "Hello world!");
    }

    #[test]
    fn parse_styled_text() {
        let mut frier = MdFrier::new().unwrap();
        // DefaultMapper preserves decorators around emphasis
        let lines = frier.parse(80, "Hello *world*!", &DefaultMapper);
        assert_eq!(lines.len(), 1);

        let line = &lines[0];
        // Spans: "Hello " + "*" (open) + "world" (emphasis) + "*" (close) + "!"
        assert_eq!(line.spans.len(), 5);
        assert_eq!(line.spans[0].content, "Hello ");
        assert_eq!(line.spans[1].content, "*");
        assert!(line.spans[1].modifiers.contains(Modifier::EmphasisWrapper));
        assert_eq!(line.spans[2].content, "world");
        assert!(line.spans[2].modifiers.contains(Modifier::Emphasis));
        assert_eq!(line.spans[3].content, "*");
        assert!(line.spans[3].modifiers.contains(Modifier::EmphasisWrapper));
        assert_eq!(line.spans[4].content, "!");
    }

    #[test]
    fn parse_header() {
        let mut frier = MdFrier::new().unwrap();
        let lines = frier.parse(80, "# Hello\n", &DefaultMapper);
        assert_eq!(lines.len(), 1);

        let line = &lines[0];
        assert!(matches!(line.kind, LineKind::Header(1)));
    }

    #[test]
    fn parse_code_block() {
        let mut frier = MdFrier::new().unwrap();
        let lines = frier.parse(80, "```rust\nlet x = 1;\n```\n", &DefaultMapper);
        assert_eq!(lines.len(), 1);

        let line = &lines[0];
        assert!(matches!(line.kind, LineKind::CodeBlock { .. }));
        // First span is the code content
        assert!(line.spans[0].content.starts_with("let x = 1;"));
    }

    #[test]
    fn parse_blockquote() {
        let mut frier = MdFrier::new().unwrap();
        let lines = frier.parse(80, "> Hello world", &DefaultMapper);
        assert_eq!(lines.len(), 1);

        let line = &lines[0];
        // With flat API, first span should be the blockquote bar
        assert!(line.spans[0].modifiers.contains(Modifier::BlockquoteBar));
        assert_eq!(line.spans[0].content, "> ");
    }

    #[test]
    fn parse_list() {
        let mut frier = MdFrier::new().unwrap();
        let lines = frier.parse(80, "- Item 1\n- Item 2", &DefaultMapper);
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn paragraph_breaks() {
        let mut frier = MdFrier::new().unwrap();
        let lines = frier.parse(10, "longline1\nlongline2", &DefaultMapper);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].spans[0].content, "longline1");
        assert_eq!(lines[1].spans[0].content, "longline2");
    }

    #[test]
    fn soft_break_with_styling() {
        let mut frier = MdFrier::new().unwrap();
        // DefaultMapper preserves decorators
        let lines = frier.parse(80, "This \n*is* a test.", &DefaultMapper);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].spans[0].content, "This");
        // Second line: "*" (open) + "is" (emphasis) + "*" (close) + " a test."
        assert_eq!(lines[1].spans[0].content, "*");
        assert!(
            lines[1].spans[0]
                .modifiers
                .contains(Modifier::EmphasisWrapper)
        );
        assert_eq!(lines[1].spans[1].content, "is");
        assert!(lines[1].spans[1].modifiers.contains(Modifier::Emphasis));
        assert_eq!(lines[1].spans[2].content, "*");
        assert!(
            lines[1].spans[2]
                .modifiers
                .contains(Modifier::EmphasisWrapper)
        );
    }

    #[test]
    fn code_block_spacing() {
        let input = "Paragraph before.
```rust
let x = 1;
```
Paragraph after.";

        let mut frier = MdFrier::new().unwrap();
        let lines = frier.parse(80, input, &DefaultMapper);
        let output = lines_to_string(&lines);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn code_block_before_list_spacing() {
        let input = "```rust
let x = 1;
```
- list item";

        let mut frier = MdFrier::new().unwrap();
        let lines = frier.parse(80, input, &DefaultMapper);
        let output = lines_to_string(&lines);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn separate_blockquotes_have_blank_lines() {
        let input = r#"> Blockquotes are very handy in email to emulate reply text.
> This line is part of the same quote.

Quote break.

> This is a very long line that will still be quoted properly when it wraps. Oh boy let's keep writing to make sure this is long enough to actually wrap for everyone. Oh, you can *put* **Markdown** into a blockquote.

> Blockquotes can also be nested...
>
> > ...by using additional greater-than signs right next to each other...
> >
> > > ...or with spaces between arrows."#;

        let mut frier = MdFrier::new().unwrap();
        let lines = frier.parse(80, input, &DefaultMapper);
        let output = lines_to_string(&lines);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn bare_url_line_broken() {
        let mut frier = MdFrier::new().unwrap();
        let lines = frier.parse(15, "See https://example.com/path ok?", &DefaultMapper);
        let spans: Vec<_> = lines.into_iter().flat_map(|l| l.spans).collect();
        let url_source = SourceContent::from("https://example.com/path");
        assert_eq!(
            spans,
            vec![
                Span::new("See ".into(), Modifier::empty()),
                Span::new("(".into(), Modifier::LinkURLWrapper),
                Span::source_link(
                    "https://".into(),
                    Modifier::LinkURL | Modifier::BareLink,
                    url_source.clone()
                ),
                Span::source_link(
                    "example.com/".into(),
                    Modifier::LinkURL | Modifier::BareLink,
                    url_source.clone()
                ),
                Span::source_link(
                    "path".into(),
                    Modifier::LinkURL | Modifier::BareLink,
                    url_source.clone()
                ),
                Span::new(")".into(), Modifier::LinkURLWrapper),
                Span::new(" ok?".into(), Modifier::empty()),
            ]
        );
    }

    #[test]
    fn list_preserve_formatting() {
        let input = r#"1. First ordered list item
2. Another item
   - Unordered sub-list.
3. Actual numbers don't matter, just that it's a number
   1. Ordered sub-list
4. And another item.

   You can have properly indented paragraphs within list items. Notice the blank line above, and the leading spaces (at least one, but we'll use three here to also align the raw Markdown).

   To have a line break without a paragraph, you will need to use two trailing spaces.
   Note that this line is separate, but within the same paragraph.
   (This is contrary to the typical GFM line break behaviour, where trailing spaces are not required.)

- Unordered list can use asterisks

* Or minuses

- Or pluses

1. Make my changes
   1. Fix bug
   2. Improve formatting
      - Make the headings bigger
2. Push my commits to GitHub
3. Open a pull request
   - Describe my changes
   - Mention all the members of my team
     - Ask for feedback

- Create a list by starting a line with `+`, `-`, or `*`
- Sub-lists are made by indenting 2 spaces:
  - Marker character change forces new list start:
    - Ac tristique libero volutpat at
    * Facilisis in pretium nisl aliquet
    - Nulla volutpat aliquam velit
  - Task lists
    - [x] Finish my changes
    - [ ] Push my commits to GitHub
    - [ ] Open a pull request
    - [x] @mentions, #refs, [links](), **formatting**, and <del>tags</del> supported
    - [x] list syntax required (any unordered or ordered list supported)
    - [ ] this is a complete item
    - [ ] this is an incomplete item
- Very easy!
"#;

        let mut frier = MdFrier::new().unwrap();
        let lines = frier.parse(80, input, &DefaultMapper);
        let output = lines_to_string(&lines);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn code_block_wrapping() {
        // Test that code blocks wrap at width boundary
        let input = "```\nabcdefghij\n```\n";

        let mut frier = MdFrier::new().unwrap();
        // Width of 5 should wrap "abcdefghij" into two lines
        let lines = frier.parse(5, input, &DefaultMapper);
        assert_eq!(lines.len(), 2);
        // First line should be 5 chars
        assert_eq!(lines[0].spans[0].content, "abcde");
        // Second line should be remaining 5 chars
        assert_eq!(lines[1].spans[0].content, "fghij");
    }

    #[test]
    fn code_block_no_wrap_when_fits() {
        // Test that code blocks don't wrap when they fit
        let input = "```\nabcde\n```\n";

        let mut frier = MdFrier::new().unwrap();
        let lines = frier.parse(5, input, &DefaultMapper);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].spans[0].content, "abcde");
    }

    #[test]
    fn hide_urls() {
        let mut frier = MdFrier::new().unwrap();
        struct HideUrlsMapper {}
        impl Mapper for HideUrlsMapper {
            fn hide_urls(&self) -> bool {
                true
            }
        }
        let mapper = HideUrlsMapper {};
        let lines = frier.parse(80, "[desc](https://url)", &mapper);
        assert_eq!(lines.len(), 1);

        let url_source = SourceContent::from("https://url");
        assert_eq!(
            lines[0].spans,
            vec![
                Span::new(
                    "[".into(),
                    Modifier::Link | Modifier::LinkDescriptionWrapper
                ),
                Span::source_link(
                    "desc".into(),
                    Modifier::Link | Modifier::LinkDescription,
                    url_source.clone()
                ),
                Span::new(
                    "]".into(),
                    Modifier::Link | Modifier::LinkDescriptionWrapper
                ),
            ]
        );
    }
}
