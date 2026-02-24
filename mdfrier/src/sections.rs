use std::iter::Peekable;

use textwrap::{Options, wrap};
use unicode_width::UnicodeWidthStr as _;

use crate::{
    BulletStyle, Line, LineKind, Mapper, MdLineContainer, Span, convert_raw_to_mdline,
    lines::{ListMarker, table_to_raw_lines, wrapped_lines_to_raw_lines},
    markdown::{MdContainer, MdContent, MdIterator, MdSection as MdSectionInternal},
    wrap::wrap_md_spans,
};

#[derive(Default, Debug, Clone, PartialEq)]
pub struct Section {
    pub backend: String,
    pub lines: Vec<Line>,
    pub kind: SectionKind,
}

#[derive(Default, Debug, Clone, PartialEq)]
pub enum SectionKind {
    #[default]
    Text,
    Header,
}

pub struct SectionIterator<'a, M: Mapper> {
    inner: Peekable<MdIterator<'a>>,
    width: u16,
    mapper: &'a M,
}

impl<'a, M: Mapper> SectionIterator<'a, M> {
    pub(crate) fn new(inner: MdIterator<'a>, width: u16, mapper: &'a M) -> Self {
        SectionIterator {
            inner: inner.peekable(),
            width,
            mapper,
        }
    }

    /// Convert an MdSection to lines, returning (lines, backend, is_header).
    fn lines_from_md_section(&self, md_section: &MdSectionInternal) -> (Vec<Line>, String, bool) {
        let nesting = convert_nesting(&md_section.nesting, md_section.is_list_continuation);

        match &md_section.content {
            MdContent::Paragraph(p) if p.is_empty() => (Vec::new(), String::new(), false),
            MdContent::Paragraph(p) => {
                let prefix_width: usize = nesting
                    .iter()
                    .map(|c| match c {
                        MdLineContainer::Blockquote => 2,
                        MdLineContainer::ListItem { marker, .. } => marker.width(),
                    })
                    .sum();
                let wrapped_lines = wrap_md_spans(self.width, p.spans.clone(), prefix_width);
                let raw_lines = wrapped_lines_to_raw_lines(wrapped_lines, nesting);
                let lines = raw_lines
                    .into_iter()
                    .map(|raw| convert_raw_to_mdline(raw, self.width, self.mapper))
                    .collect();
                (lines, String::new(), false)
            }
            MdContent::Header { tier, text } => {
                let line = Line {
                    spans: vec![Span::from(text.clone())],
                    kind: LineKind::Header(*tier),
                };
                (vec![line], text.clone(), true)
            }
            MdContent::CodeBlock { language, code } => {
                let code_lines: Vec<&str> = code.lines().collect();
                let num_lines = code_lines.len();
                if num_lines == 0 {
                    return (Vec::new(), code.clone(), false);
                }

                let prefix_width: usize = nesting
                    .iter()
                    .map(|c| match c {
                        MdLineContainer::Blockquote => 2,
                        MdLineContainer::ListItem { marker, .. } => marker.width(),
                    })
                    .sum();
                let available_width = (self.width as usize).saturating_sub(prefix_width).max(1);

                let mut lines = Vec::new();

                for line in code_lines {
                    let line_width = line.width();

                    if line_width > available_width {
                        let options = Options::new(available_width)
                            .break_words(true)
                            .word_splitter(textwrap::word_splitters::WordSplitter::NoHyphenation);
                        let parts: Vec<_> = wrap(line, options).into_iter().collect();

                        for part in parts {
                            let part_str = part.into_owned();
                            let part_width = part_str.width();
                            let padding = available_width.saturating_sub(part_width);
                            let mut spans = vec![Span::from(part_str)];
                            if padding > 0 {
                                spans.push(Span::from(" ".repeat(padding)));
                            }
                            lines.push(Line {
                                spans,
                                kind: LineKind::CodeBlock {
                                    language: language.clone(),
                                },
                            });
                        }
                    } else {
                        let padding = available_width.saturating_sub(line_width);
                        let mut spans = vec![Span::from(line.to_owned())];
                        if padding > 0 {
                            spans.push(Span::from(" ".repeat(padding)));
                        }
                        lines.push(Line {
                            spans,
                            kind: LineKind::CodeBlock {
                                language: language.clone(),
                            },
                        });
                    }
                }
                (lines, code.clone(), false)
            }
            MdContent::HorizontalRule => {
                let line = Line {
                    spans: Vec::new(),
                    kind: LineKind::HorizontalRule,
                };
                (vec![line], String::new(), false)
            }
            MdContent::Table {
                header,
                rows,
                alignments,
            } => {
                let raw_lines = table_to_raw_lines(self.width, header, rows, alignments, nesting);
                let lines = raw_lines
                    .into_iter()
                    .map(|raw| convert_raw_to_mdline(raw, self.width, self.mapper))
                    .collect();
                (lines, String::new(), false)
            }
        }
    }

}

/// Check if the next MdSection should be aggregated with the current one.
/// Returns true if they share the same top-level container (for lists only).
fn should_aggregate(current_top: Option<&MdContainer>, peeked: &MdSectionInternal) -> bool {
    // Headers are never aggregated
    if matches!(peeked.content, MdContent::Header { .. }) {
        return false;
    }

    // Compare top-level containers
    // Only aggregate within lists, not blockquotes (blockquotes don't have distinguishable markers)
    match (current_top, peeked.nesting.first()) {
        (Some(MdContainer::List(a)), Some(MdContainer::List(b))) => {
            // For lists: aggregate if same top-level list
            a == b
        }
        (Some(MdContainer::Blockquote(_)), Some(MdContainer::Blockquote(_))) => {
            // Don't aggregate blockquotes - each becomes its own section
            // (parser doesn't distinguish same vs different blockquotes)
            false
        }
        _ => false, // Don't aggregate standalone content or across different container types
    }
}

impl<M: Mapper> Iterator for SectionIterator<'_, M> {
    type Item = Section;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let first = self.inner.next()?;

            // Get lines from first MdSection
            let (mut lines, backend, is_header) = self.lines_from_md_section(&first);

            // Headers are always their own section
            if is_header {
                return Some(Section {
                    lines,
                    backend,
                    kind: SectionKind::Header,
                });
            }

            let current_top = first.nesting.first().cloned();

            // Aggregate consecutive MdSections with the same top-level container
            while let Some(peeked) = self.inner.peek() {
                if !should_aggregate(current_top.as_ref(), peeked) {
                    break;
                }

                // Add blank line before continuation paragraphs within lists
                let needs_blank = peeked.is_list_continuation;

                let next = self.inner.next().expect("peeked value should exist");
                let (next_lines, _, _) = self.lines_from_md_section(&next);

                if needs_blank && !lines.is_empty() && !next_lines.is_empty() {
                    lines.push(Line {
                        spans: Vec::new(),
                        kind: LineKind::Blank,
                    });
                }

                lines.extend(next_lines);
            }

            // Skip empty sections (e.g., blank blockquote lines)
            if lines.is_empty() {
                continue;
            }

            // Add trailing blank line if there are more sections after this one
            // But not if this section is only blank lines (don't double-up blanks)
            let has_content = lines.iter().any(|l| !matches!(l.kind, LineKind::Blank));
            if self.inner.peek().is_some() && has_content {
                lines.push(Line {
                    spans: Vec::new(),
                    kind: LineKind::Blank,
                });
            }

            return Some(Section {
                lines,
                backend,
                kind: SectionKind::Text,
            });
        }
    }
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
