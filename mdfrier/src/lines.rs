use std::collections::VecDeque;
use std::iter::Peekable;

use textwrap::{Options, wrap};
use unicode_width::UnicodeWidthStr as _;

use crate::{
    Line, LineKind, Mapper,
    markdown::{MdContainer, MdContent, MdIterator, MdSection, Modifier, Span, TableAlignment},
    wrap::{wrap_md_spans, wrap_md_spans_lines},
};

/// A simplified nesting container.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum MdLineContainer {
    /// Blockquote level.
    Blockquote,
    /// List item with marker type.
    /// `continuation` is true for content after the first paragraph in a list item,
    /// which renders as indentation (spaces) instead of the marker.
    ListItem {
        marker: ListMarker,
        continuation: bool,
    },
}

/// Type of list marker.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ListMarker {
    Unordered(BulletStyle),
    Ordered(u32),
    TaskUnchecked(BulletStyle),
    TaskChecked(BulletStyle),
}

/// Bullet style for unordered lists.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BulletStyle {
    Dash,
    Star, // a.k.a Asterisk, *
    Plus,
}

impl BulletStyle {
    /// Get the character representation.
    pub fn char(&self) -> char {
        match self {
            BulletStyle::Dash => '-',
            BulletStyle::Star => '*',
            BulletStyle::Plus => '+',
        }
    }

    /// Parse from a character.
    pub fn from_char(c: char) -> Option<Self> {
        match c {
            '-' => Some(BulletStyle::Dash),
            '*' => Some(BulletStyle::Star),
            '+' => Some(BulletStyle::Plus),
            _ => None,
        }
    }
}

impl ListMarker {
    /// Calculate marker width using mapper's symbols.
    pub fn width<M: Mapper>(&self, mapper: &M) -> usize {
        match self {
            ListMarker::Unordered(b) => mapper.unordered_bullet(*b).width(),
            ListMarker::Ordered(n) => mapper.ordered_marker(*n).width(),
            ListMarker::TaskChecked(b) => {
                mapper.unordered_bullet(*b).width() + mapper.task_checked().width()
            }
            ListMarker::TaskUnchecked(b) => {
                mapper.unordered_bullet(*b).width() + mapper.task_unchecked().width()
            }
        }
    }
}

/// Position of a table border.
#[derive(Debug, Clone, Copy, PartialEq)]
enum BorderPosition {
    Top,
    HeaderSeparator,
    Bottom,
}

/// Iterator that produces `Line` items from parsed markdown.
///
/// This handles the blank line logic between sections, producing lines
/// one at a time with proper spacing.
pub struct LineIterator<'a, M: Mapper> {
    inner: Peekable<MdIterator<'a>>,
    width: u16,
    mapper: &'a M,
    /// Buffer of pending lines to emit
    pending_lines: VecDeque<Line>,
    /// Whether we need a blank line before next content
    needs_blank: bool,
    /// Previous section's nesting for comparison
    prev_nesting: Vec<MdContainer>,
    /// Whether previous section was a blank line
    prev_was_blank: bool,
    /// Whether previous section was in a list
    prev_in_list: bool,
}

impl<'a, M: Mapper> LineIterator<'a, M> {
    pub(crate) fn new(inner: MdIterator<'a>, width: u16, mapper: &'a M) -> Self {
        LineIterator {
            inner: inner.peekable(),
            width,
            mapper,
            pending_lines: VecDeque::new(),
            needs_blank: false,
            prev_nesting: Vec::new(),
            prev_was_blank: false,
            prev_in_list: false,
        }
    }

    /// Process the next MdSection and queue its lines
    fn process_next_section(&mut self) -> bool {
        let section = match self.inner.next() {
            Some(s) => s,
            None => return false,
        };

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
        let same_nested_context =
            in_list && self.prev_in_list && curr_list_depth > 1 && prev_list_depth > 1;

        let same_list_context = same_top_level_list || same_nested_context;

        // Check if we're exiting to a new top-level list (not part of previous ancestry)
        let exiting_to_new_top_level =
            nesting_change && curr_list_depth == 1 && prev_list_depth > 1 && {
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
            self.pending_lines.push_back(Line {
                spans: Vec::new(),
                kind: LineKind::Blank,
            });
        }

        // Only headers don't need space after
        self.needs_blank = !matches!(section.content, MdContent::Header { .. });
        self.prev_nesting.clone_from(&section.nesting);
        self.prev_was_blank = is_blank_line;
        self.prev_in_list = in_list;

        let lines = section_to_lines(self.width, &section, self.mapper);
        self.pending_lines.extend(lines);

        true
    }
}

impl<M: Mapper> Iterator for LineIterator<'_, M> {
    type Item = Line;

    fn next(&mut self) -> Option<Self::Item> {
        // Return buffered line if available
        if let Some(line) = self.pending_lines.pop_front() {
            return Some(line);
        }

        // Process sections until we have a line to return
        while self.process_next_section() {
            if let Some(line) = self.pending_lines.pop_front() {
                return Some(line);
            }
        }

        None
    }
}

/// Convert a markdown section to output lines.
/// Applies mapper decorators before wrapping so widths are correct.
fn section_to_lines<M: Mapper>(width: u16, section: &MdSection, mapper: &M) -> Vec<Line> {
    let nesting = convert_nesting(&section.nesting, section.is_list_continuation);

    match &section.content {
        MdContent::Paragraph(p) if p.is_empty() => {
            vec![Line {
                spans: Vec::new(),
                kind: LineKind::Blank,
            }]
        }
        MdContent::Paragraph(p) => {
            // Apply decorators first, then wrap
            let decorated_spans = apply_decorators(p.spans.clone(), mapper);
            let prefix_width: usize = nesting
                .iter()
                .map(|c| match c {
                    MdLineContainer::Blockquote => mapper.blockquote_bar().width(),
                    MdLineContainer::ListItem { marker, .. } => marker.width(mapper),
                })
                .sum();
            let wrapped_lines = wrap_md_spans(width, decorated_spans, prefix_width);
            wrapped_to_lines(wrapped_lines, nesting, mapper)
        }
        MdContent::Header { tier, text } => {
            let mut spans = nesting_to_prefix_spans(&nesting, mapper);
            spans.push(Span::from(text.clone()));
            vec![Line {
                spans,
                kind: LineKind::Header(*tier),
            }]
        }
        MdContent::CodeBlock { language, code } => {
            code_block_to_lines(width, language, code, nesting, mapper)
        }
        MdContent::HorizontalRule => {
            let prefix_spans = nesting_to_prefix_spans(&nesting, mapper);
            let prefix_width: usize = prefix_spans.iter().map(|s| s.content.width()).sum();
            let available = (width as usize).saturating_sub(prefix_width);

            let mut spans = prefix_spans;
            spans.push(Span::new(
                mapper.horizontal_rule_char().repeat(available),
                Modifier::HorizontalRule,
            ));
            vec![Line {
                spans,
                kind: LineKind::HorizontalRule,
            }]
        }
        MdContent::Table {
            header,
            rows,
            alignments,
        } => table_to_lines(width, header, rows, alignments, nesting, mapper),
    }
}

/// Apply mapper decorators to spans (emphasis, code, links, etc).
/// This must happen before wrapping so decorator widths are included.
fn apply_decorators<M: Mapper>(spans: Vec<Span>, mapper: &M) -> Vec<Span> {
    let mut result: Vec<Span> = Vec::with_capacity(spans.len() * 2);
    let mut prev_emphasis = false;
    let mut prev_strong = false;
    let mut prev_code = false;
    let mut prev_strikethrough = false;

    for mut span in spans {
        let has_emphasis = span.modifiers.contains(Modifier::Emphasis);
        let has_strong = span.modifiers.contains(Modifier::StrongEmphasis);
        let has_code = span.modifiers.contains(Modifier::Code);
        let has_strikethrough = span.modifiers.contains(Modifier::Strikethrough);
        let is_newline = span.modifiers.contains(Modifier::NewLine);

        // If this span starts a new line, trim trailing whitespace from previous span
        // (This matches wrap.rs behavior but must happen before we insert decorators)
        if is_newline {
            if let Some(last) = result.last_mut() {
                last.content.truncate(last.content.trim_end().len());
            }
        }

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

        // Track if we need to transfer NewLine to the first opening decorator
        let mut newline_transferred = false;

        // Open decorators that started
        // If the content span has NewLine, transfer it to the first decorator we insert
        if has_emphasis && !prev_emphasis {
            let open = mapper.emphasis_open();
            if !open.is_empty() {
                let mods = if is_newline && !newline_transferred {
                    newline_transferred = true;
                    Modifier::EmphasisWrapper | Modifier::NewLine
                } else {
                    Modifier::EmphasisWrapper
                };
                result.push(Span::new(open.to_owned(), mods));
            }
        }
        if has_strong && !prev_strong {
            let open = mapper.strong_open();
            if !open.is_empty() {
                let mods = if is_newline && !newline_transferred {
                    newline_transferred = true;
                    Modifier::StrongEmphasisWrapper | Modifier::NewLine
                } else {
                    Modifier::StrongEmphasisWrapper
                };
                result.push(Span::new(open.to_owned(), mods));
            }
        }
        if has_strikethrough && !prev_strikethrough {
            let open = mapper.strikethrough_open();
            if !open.is_empty() {
                let mods = if is_newline && !newline_transferred {
                    newline_transferred = true;
                    Modifier::StrikethroughWrapper | Modifier::NewLine
                } else {
                    Modifier::StrikethroughWrapper
                };
                result.push(Span::new(open.to_owned(), mods));
            }
        }
        if has_code && !prev_code {
            let open = mapper.code_open();
            if !open.is_empty() {
                let mods = if is_newline && !newline_transferred {
                    newline_transferred = true;
                    Modifier::CodeWrapper | Modifier::NewLine
                } else {
                    Modifier::CodeWrapper
                };
                result.push(Span::new(open.to_owned(), mods));
            }
        }

        // If we transferred NewLine to an opening decorator, remove it from the content span
        if newline_transferred {
            span.modifiers.remove(Modifier::NewLine);
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

        // Hide URL spans if configured (but not Image spans - they're extracted during wrapping)
        let hide = mapper.hide_urls()
            && span
                .modifiers
                .intersects(Modifier::LinkURL | Modifier::LinkURLWrapper)
            && !span.modifiers.contains(Modifier::BareLink)
            && !span.modifiers.contains(Modifier::Image);
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
                        ListMarker::Unordered(b) => mapper.unordered_bullet(*b).to_owned(),
                        ListMarker::Ordered(n) => mapper.ordered_marker(*n),
                        ListMarker::TaskChecked(b) => {
                            format!("{}{}", mapper.unordered_bullet(*b), mapper.task_checked())
                        }
                        ListMarker::TaskUnchecked(b) => {
                            format!("{}{}", mapper.unordered_bullet(*b), mapper.task_unchecked())
                        }
                    };
                    spans.push(Span::new(marker_text, Modifier::ListMarker));
                } else {
                    // Indentation for outer/continuation items
                    let indent_width = marker.width(mapper);
                    spans.push(Span::new(" ".repeat(indent_width), Modifier::empty()));
                }
            }
        }
    }
    spans
}

/// Convert MdContainer nesting to Container nesting.
fn convert_nesting(md_nesting: &[MdContainer], is_list_continuation: bool) -> Vec<MdLineContainer> {
    let mut nesting = Vec::new();

    // Find the index of the last ListItem to mark it as continuation if needed
    let last_list_item_idx = md_nesting
        .iter()
        .rposition(|c| matches!(c, MdContainer::ListItem(_)));

    for (idx, c) in md_nesting.iter().enumerate() {
        match c {
            MdContainer::Blockquote(_) => {
                nesting.push(MdLineContainer::Blockquote);
            }
            MdContainer::ListItem(marker) => {
                let first_char = marker.original.chars().next().unwrap_or('-');
                let bullet = BulletStyle::from_char(first_char).unwrap_or(BulletStyle::Dash);

                let list_marker = if let Some(checked) = marker.task {
                    if checked {
                        ListMarker::TaskChecked(bullet)
                    } else {
                        ListMarker::TaskUnchecked(bullet)
                    }
                } else if first_char.is_ascii_digit() {
                    let num: u32 = marker
                        .original
                        .chars()
                        .take_while(|c| c.is_ascii_digit())
                        .fold(0_u32, |acc, c| {
                            acc.saturating_mul(10)
                                .saturating_add(c.to_digit(10).unwrap_or(0))
                        });
                    ListMarker::Ordered(if num == 0 { 1 } else { num })
                } else {
                    ListMarker::Unordered(bullet)
                };

                // Only the innermost list item can be a continuation
                let continuation = is_list_continuation && last_list_item_idx == Some(idx);

                nesting.push(MdLineContainer::ListItem {
                    marker: list_marker,
                    continuation,
                });
            }
            MdContainer::List(_) => {
                // List containers don't produce visual nesting
            }
        }
    }

    nesting
}

/// Convert a code block to output lines.
fn code_block_to_lines<M: Mapper>(
    width: u16,
    language: &str,
    code: &str,
    nesting: Vec<MdLineContainer>,
    mapper: &M,
) -> Vec<Line> {
    let code_lines: Vec<&str> = code.lines().collect();
    let num_lines = code_lines.len();
    if num_lines == 0 {
        return vec![];
    }

    // Calculate prefix and available width
    let prefix_spans = nesting_to_prefix_spans(&nesting, mapper);
    let prefix_width: usize = prefix_spans.iter().map(|s| s.content.width()).sum();
    let available_width = (width as usize).saturating_sub(prefix_width).max(1);

    let mut result = Vec::new();

    for line in code_lines {
        let line_width = line.width();

        if line_width > available_width {
            // Wrap this line
            let options = Options::new(available_width)
                .break_words(true)
                .word_splitter(textwrap::word_splitters::WordSplitter::NoHyphenation);
            let parts: Vec<_> = wrap(line, options).into_iter().collect();

            for part in parts {
                let content_width = part.width();
                let padding = available_width.saturating_sub(content_width);

                let mut spans = prefix_spans.clone();
                spans.push(Span::new(part.into_owned(), Modifier::Code));
                if padding > 0 {
                    spans.push(Span::new(" ".repeat(padding), Modifier::Code));
                }
                result.push(Line {
                    spans,
                    kind: LineKind::CodeBlock {
                        language: language.to_owned(),
                    },
                });
            }
        } else {
            // Line fits, pad to fill width
            let padding = available_width.saturating_sub(line_width);

            let mut spans = prefix_spans.clone();
            spans.push(Span::new(line.to_owned(), Modifier::Code));
            if padding > 0 {
                spans.push(Span::new(" ".repeat(padding), Modifier::Code));
            }
            result.push(Line {
                spans,
                kind: LineKind::CodeBlock {
                    language: language.to_owned(),
                },
            });
        }
    }

    result
}

/// Convert wrapped lines to output Lines with prefix spans.
fn wrapped_to_lines<M: Mapper>(
    wrapped_lines: Vec<crate::wrap::WrappedLine>,
    nesting: Vec<MdLineContainer>,
    mapper: &M,
) -> Vec<Line> {
    let mut lines = Vec::new();

    for (line_idx, wrapped_line) in wrapped_lines.into_iter().enumerate() {
        let has_content = wrapped_line
            .spans
            .iter()
            .any(|s| !s.content.trim().is_empty());
        if !has_content && wrapped_line.images.is_empty() {
            continue;
        }

        // For continuation lines (soft-wrapped), mark ListItems as continuation
        let line_nesting = if line_idx == 0 || wrapped_line.is_first {
            nesting.clone()
        } else {
            nesting
                .iter()
                .map(|c| match c {
                    MdLineContainer::Blockquote => MdLineContainer::Blockquote,
                    MdLineContainer::ListItem { marker, .. } => MdLineContainer::ListItem {
                        marker: marker.clone(),
                        continuation: true,
                    },
                })
                .collect()
        };

        // Create text line
        if !wrapped_line.spans.is_empty() {
            let mut spans = nesting_to_prefix_spans(&line_nesting, mapper);
            spans.extend(wrapped_line.spans);
            lines.push(Line {
                spans,
                kind: LineKind::Paragraph,
            });
        }

        // Create image lines
        for img in wrapped_line.images {
            let spans = vec![
                Span::new("![".to_owned(), Modifier::LinkDescriptionWrapper),
                Span::new("Loading...".to_owned(), Modifier::LinkURL),
                Span::new("]".to_owned(), Modifier::LinkDescriptionWrapper),
                Span::new("(".to_owned(), Modifier::LinkURLWrapper),
                Span::new(img.url.clone(), Modifier::LinkDescription),
                Span::new(")".to_owned(), Modifier::LinkURLWrapper),
            ];
            lines.push(Line {
                spans,
                kind: LineKind::Image {
                    url: img.url,
                    description: img.description,
                },
            });
        }
    }

    lines
}

/// Convert a table to output lines.
fn table_to_lines<M: Mapper>(
    width: u16,
    header: &[Vec<Span>],
    rows: &[Vec<Vec<Span>>],
    alignments: &[TableAlignment],
    nesting: Vec<MdLineContainer>,
    mapper: &M,
) -> Vec<Line> {
    let mut lines = Vec::new();

    let prefix_spans = nesting_to_prefix_spans(&nesting, mapper);
    let prefix_width: usize = prefix_spans.iter().map(|s| s.content.width()).sum();
    let available_width = (width as usize).saturating_sub(prefix_width);

    let num_cols = header.len();
    if num_cols == 0 {
        return lines;
    }

    // Calculate cell width
    let cell_width = |cell: &[Span]| -> usize { cell.iter().map(|n| n.content.width()).sum() };

    // Find max width for each column
    let mut col_widths: Vec<usize> = header.iter().map(|c| cell_width(c)).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if i < col_widths.len() {
                col_widths[i] = col_widths[i].max(cell_width(cell));
            }
        }
    }

    // Add padding
    let col_widths: Vec<usize> = col_widths.iter().map(|w| w + 2).collect();

    // Scale if too wide
    let table_width: usize = col_widths.iter().sum::<usize>() + num_cols + 1;
    let col_widths: Vec<usize> = if table_width > available_width && available_width > num_cols + 1 {
        let content_width = available_width - num_cols - 1;
        let total_content: usize = col_widths.iter().sum();
        col_widths
            .iter()
            .map(|w| (w * content_width / total_content).max(3))
            .collect()
    } else {
        col_widths
    };

    // Helper to build border line
    let build_border = |position: BorderPosition| -> Line {
        let (left, mid, right) = match position {
            BorderPosition::Top => (
                mapper.table_top_left(),
                mapper.table_top_junction(),
                mapper.table_top_right(),
            ),
            BorderPosition::HeaderSeparator => (
                mapper.table_left_junction(),
                mapper.table_cross(),
                mapper.table_right_junction(),
            ),
            BorderPosition::Bottom => (
                mapper.table_bottom_left(),
                mapper.table_bottom_junction(),
                mapper.table_bottom_right(),
            ),
        };
        let horizontal = mapper.table_horizontal();

        let mut spans = prefix_spans.clone();
        spans.push(Span::new(left.to_owned(), Modifier::TableBorder));
        for (i, &col_w) in col_widths.iter().enumerate() {
            spans.push(Span::new(horizontal.repeat(col_w), Modifier::TableBorder));
            if i < num_cols - 1 {
                spans.push(Span::new(mid.to_owned(), Modifier::TableBorder));
            }
        }
        spans.push(Span::new(right.to_owned(), Modifier::TableBorder));

        Line {
            spans,
            kind: LineKind::TableBorder,
        }
    };

    // Helper to build row lines
    let build_row_lines = |row: &[Vec<Span>], is_header: bool| -> Vec<Line> {
        let vertical = mapper.table_vertical();

        // Wrap each cell's content
        let wrapped_cells: Vec<Vec<Vec<Span>>> = row
            .iter()
            .enumerate()
            .map(|(i, cell)| {
                let col_width = col_widths.get(i).copied().unwrap_or(3);
                let inner_width = col_width.saturating_sub(2).max(1) as u16;
                let wrapped = wrap_md_spans_lines(inner_width, cell.clone());
                if wrapped.is_empty() {
                    vec![Vec::new()]
                } else {
                    wrapped
                }
            })
            .collect();

        let max_lines = wrapped_cells.iter().map(|c| c.len()).max().unwrap_or(1);
        let mut result = Vec::new();

        for line_idx in 0..max_lines {
            let mut spans = prefix_spans.clone();
            spans.push(Span::new(vertical.to_owned(), Modifier::TableBorder));

            for (i, col_width) in col_widths.iter().enumerate() {
                let alignment = alignments.get(i).copied().unwrap_or(TableAlignment::Left);
                let cell_spans = wrapped_cells
                    .get(i)
                    .and_then(|c| c.get(line_idx))
                    .map_or(&[][..], |v| v.as_slice());

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
                    format!(" {}", " ".repeat(left_pad)),
                    Modifier::empty(),
                ));

                // Cell content (apply decorators)
                for node in cell_spans {
                    let mut mapped_node = node.clone();
                    if mapped_node.modifiers.contains(Modifier::LinkDescriptionWrapper) {
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
                    format!("{} ", " ".repeat(right_pad)),
                    Modifier::empty(),
                ));
                spans.push(Span::new(vertical.to_owned(), Modifier::TableBorder));
            }

            // Fill missing columns
            for i in row.len()..num_cols {
                let col_width = col_widths.get(i).copied().unwrap_or(3);
                spans.push(Span::new(" ".repeat(col_width), Modifier::empty()));
                spans.push(Span::new(vertical.to_owned(), Modifier::TableBorder));
            }

            result.push(Line {
                spans,
                kind: LineKind::TableRow { is_header },
            });
        }

        result
    };

    // Top border
    lines.push(build_border(BorderPosition::Top));

    // Header row
    lines.extend(build_row_lines(header, true));

    // Header separator
    lines.push(build_border(BorderPosition::HeaderSeparator));

    // Data rows
    for row in rows {
        lines.extend(build_row_lines(row, false));
    }

    // Bottom border
    lines.push(build_border(BorderPosition::Bottom));

    lines
}
