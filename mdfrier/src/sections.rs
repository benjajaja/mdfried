use std::iter::Peekable;

use textwrap::{Options, wrap};
use unicode_width::UnicodeWidthStr as _;

use crate::{
    BulletStyle, Line, LineKind, Mapper, MdLineContainer, Span, convert_raw_to_mdline,
    lines::{ListMarker, table_to_raw_lines, wrapped_lines_to_raw_lines},
    markdown::{MdContainer, MdContent, MdIterator},
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
    Paragraph,
    Header,
    CodeBlock,
    HorizontalRule,
    Table,
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

    fn section_from_content(&self, nesting: Vec<MdLineContainer>, content: MdContent) -> Section {
        match content {
            MdContent::Paragraph(p) if p.is_empty() => Section::default(),
            MdContent::Paragraph(p) => {
                let prefix_width = nesting
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
                Section {
                    kind: SectionKind::Paragraph,
                    backend: String::new(),
                    lines,
                }
            }
            MdContent::Header { tier, text } => {
                let line = Line {
                    spans: vec![Span::from(text.clone())],
                    kind: LineKind::Header(tier),
                };
                Section {
                    kind: SectionKind::Header,
                    backend: text.clone(),
                    lines: vec![line],
                }
            }
            MdContent::CodeBlock { language, code } => {
                let code_lines: Vec<&str> = code.lines().collect();
                let num_lines = code_lines.len();
                if num_lines == 0 {
                    return Section::default();
                }

                // Calculate available width for wrapping
                let prefix_width: usize = nesting
                    .iter()
                    .map(|c| match c {
                        MdLineContainer::Blockquote => 2,
                        MdLineContainer::ListItem { marker, .. } => marker.width(),
                    })
                    .sum();
                let available_width = (self.width as usize).saturating_sub(prefix_width).max(1);

                let mut lines = Vec::new();
                let last_source_idx = num_lines - 1;
                let mut nesting_owned = Some(nesting);

                for (source_idx, line) in code_lines.into_iter().enumerate() {
                    let is_last_source = source_idx == last_source_idx;
                    let line_width = line.width();

                    if line_width > available_width {
                        // Wrap this line
                        let options = Options::new(available_width)
                            .break_words(true)
                            .word_splitter(textwrap::word_splitters::WordSplitter::NoHyphenation);
                        let parts: Vec<_> = wrap(line, options).into_iter().collect();
                        let num_parts = parts.len();
                        let last_part_idx = num_parts.saturating_sub(1);

                        for (part_idx, part) in parts.into_iter().enumerate() {
                            let is_last_part = part_idx == last_part_idx;
                            let is_last = is_last_source && is_last_part;
                            lines.push(Line {
                                spans: vec![Span::from(part.into_owned())],
                                kind: LineKind::CodeBlock {
                                    language: language.clone(),
                                },
                            });
                        }
                    } else {
                        // Line fits, no wrapping needed
                        lines.push(Line {
                            spans: vec![Span::from(line.to_owned())],
                            kind: LineKind::CodeBlock {
                                language: language.clone(),
                            },
                        });
                    }
                }
                Section {
                    kind: SectionKind::CodeBlock,
                    backend: code,
                    lines,
                }
            }
            MdContent::HorizontalRule => {
                let line = Line {
                    spans: Vec::new(),
                    kind: LineKind::HorizontalRule,
                };
                Section {
                    kind: SectionKind::HorizontalRule,
                    backend: String::new(),
                    lines: vec![line],
                }
            }
            MdContent::Table {
                header,
                rows,
                alignments,
            } => {
                let lines = table_to_raw_lines(self.width, &header, &rows, &alignments, nesting);
                let lines = lines
                    .into_iter()
                    .map(|raw| convert_raw_to_mdline(raw, self.width, self.mapper))
                    .collect();
                Section {
                    kind: SectionKind::Table,
                    backend: String::new(),
                    lines,
                }
            }
        }
    }
}

impl<M: Mapper> Iterator for SectionIterator<'_, M> {
    type Item = Section;

    fn next(&mut self) -> Option<Self::Item> {
        let md_section = self.inner.next()?;

        let is_blank = md_section.content.is_blank();

        let nesting = convert_nesting(&md_section.nesting, md_section.is_list_continuation);
        let mut section: Section = self.section_from_content(nesting, md_section.content);

        if self.inner.peek().is_some() && !is_blank {
            section.lines.push(Line {
                spans: Vec::new(),
                kind: LineKind::Blank,
            });
        }

        Some(section)
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
