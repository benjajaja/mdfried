use unicode_width::UnicodeWidthStr;

use crate::{
    markdown::{MdContainer, MdContent, MdNode, MdSection, TableAlignment},
    wrap::{wrap_md_spans, wrap_md_spans_lines},
};

/// Internal intermediate line type with nesting metadata.
/// This is converted to the public `MdLine` after applying the mapper.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RawLine {
    /// The text spans making up this line.
    pub spans: Vec<MdNode>,
    /// Metadata about this line.
    pub meta: LineMeta,
}

impl RawLine {
    /// Create a blank line.
    pub fn blank() -> Self {
        Self {
            spans: Vec::new(),
            meta: LineMeta {
                kind: RawLineKind::Blank,
                nesting: Vec::new(),
            },
        }
    }
}

#[cfg(test)]
impl std::fmt::Display for RawLine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            self.spans
                .iter()
                .map(|span| span.content.clone())
                .collect::<Vec<String>>()
                .join("")
        )
    }
}

/// Metadata about a raw markdown line (internal).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LineMeta {
    /// The kind of line content.
    pub kind: RawLineKind,
    /// Nesting containers (blockquotes and list items).
    pub nesting: Vec<MdLineContainer>,
}

/// A simplified nesting container.
#[derive(Debug, Clone, PartialEq)]
pub enum MdLineContainer {
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
pub enum ListMarker {
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
    /// Calculate the display width of this marker.
    pub fn width(&self) -> usize {
        match self {
            ListMarker::Unordered(_) => 2, // "- "
            ListMarker::Ordered(n) => {
                // "1. " = 3, "10. " = 4, etc.
                let digits = if *n == 0 {
                    1
                } else {
                    (*n as f64).log10().floor() as usize + 1
                };
                digits + 2
            }
            ListMarker::TaskUnchecked(_) => 6, // "- [ ] "
            ListMarker::TaskChecked(_) => 6,   // "- [x] "
        }
    }
}

/// The kind of content a raw line represents (internal).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum RawLineKind {
    /// Regular text paragraph.
    Paragraph,
    /// Header line with tier (1-6).
    Header(u8),
    /// Code block line with language.
    CodeBlock { language: String },
    /// Horizontal rule.
    HorizontalRule,
    /// Table data row with cells preserved.
    TableRow {
        cells: Vec<Vec<MdNode>>,
        column_info: TableColumnInfo,
        is_header: bool,
    },
    /// Table border/separator.
    TableBorder {
        column_info: TableColumnInfo,
        position: BorderPosition,
    },
    /// Image reference (rendered asynchronously).
    Image { url: String, description: String },
    /// Blank line.
    Blank,
}

/// Information about table columns for rendering.
#[derive(Debug, Clone, PartialEq)]
pub struct TableColumnInfo {
    pub widths: Vec<usize>,
    pub alignments: Vec<TableAlignment>,
}

/// Position of a table border.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BorderPosition {
    Top,
    HeaderSeparator,
    Bottom,
}

/// Convert a markdown section to raw lines (internal).
pub(crate) fn section_to_raw_lines(width: u16, section: &MdSection) -> Vec<RawLine> {
    let nesting = convert_nesting(&section.nesting, section.is_list_continuation);

    match &section.content {
        MdContent::Paragraph(mdspans) if mdspans.is_empty() => {
            vec![RawLine {
                spans: Vec::new(),
                meta: LineMeta {
                    kind: RawLineKind::Blank,
                    nesting,
                },
            }]
        }
        MdContent::Paragraph(mdspans) => {
            let prefix_width = nesting
                .iter()
                .map(|c| match c {
                    MdLineContainer::Blockquote => 2,
                    MdLineContainer::ListItem { marker, .. } => marker.width(),
                })
                .sum();
            let wrapped_lines = wrap_md_spans(width, mdspans.clone(), prefix_width);
            wrapped_lines_to_raw_lines(wrapped_lines, nesting)
        }
        MdContent::Header { tier, text } => {
            vec![RawLine {
                spans: vec![MdNode::from(text.clone())],
                meta: LineMeta {
                    kind: RawLineKind::Header(*tier),
                    nesting,
                },
            }]
        }
        MdContent::CodeBlock { language, code } => {
            let code_lines: Vec<&str> = code.lines().collect();
            let num_lines = code_lines.len();
            if num_lines == 0 {
                return vec![];
            }
            let mut result = Vec::with_capacity(num_lines);
            let last_idx = num_lines - 1;
            let mut nesting_owned = Some(nesting);
            for (i, line) in code_lines.into_iter().enumerate() {
                let is_last = i == last_idx;
                result.push(RawLine {
                    spans: vec![MdNode::from(line.to_owned())],
                    meta: LineMeta {
                        kind: RawLineKind::CodeBlock {
                            language: language.clone(),
                        },
                        nesting: if is_last {
                            nesting_owned.take().unwrap()
                        } else {
                            nesting_owned.as_ref().unwrap().clone()
                        },
                    },
                });
            }
            result
        }
        MdContent::HorizontalRule => {
            vec![RawLine {
                spans: Vec::new(),
                meta: LineMeta {
                    kind: RawLineKind::HorizontalRule,
                    nesting,
                },
            }]
        }
        MdContent::Table {
            header,
            rows,
            alignments,
        } => table_to_raw_lines(width, header, rows, alignments, nesting),
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
                        .fold(0u32, |acc, c| {
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

fn wrapped_lines_to_raw_lines(
    wrapped_lines: Vec<crate::wrap::WrappedLine>,
    nesting: Vec<MdLineContainer>,
) -> Vec<RawLine> {
    let mut lines = Vec::new();

    for (line_idx, wrapped_line) in wrapped_lines.into_iter().enumerate() {
        let has_content = wrapped_line
            .spans
            .iter()
            .any(|s| !s.content.trim().is_empty());
        if !has_content && wrapped_line.images.is_empty() {
            continue;
        }

        // For continuation lines (soft-wrapped), keep all containers but mark
        // ListItems as continuation so they render as indentation
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
            lines.push(RawLine {
                spans: wrapped_line.spans,
                meta: LineMeta {
                    kind: RawLineKind::Paragraph,
                    nesting: line_nesting.clone(),
                },
            });
        }

        // Create image lines
        for img in wrapped_line.images {
            lines.push(RawLine {
                spans: Vec::new(),
                meta: LineMeta {
                    kind: RawLineKind::Image {
                        url: img.url,
                        description: img.description,
                    },
                    nesting: line_nesting.clone(),
                },
            });
        }
    }

    lines
}

fn table_to_raw_lines(
    width: u16,
    header: &[Vec<MdNode>],
    rows: &[Vec<Vec<MdNode>>],
    alignments: &[TableAlignment],
    nesting: Vec<MdLineContainer>,
) -> Vec<RawLine> {
    let mut lines = Vec::new();

    let prefix_width: usize = nesting
        .iter()
        .map(|c| match c {
            MdLineContainer::Blockquote => 2,
            MdLineContainer::ListItem { marker, .. } => marker.width(),
        })
        .sum();
    let available_width = (width as usize).saturating_sub(prefix_width);

    let num_cols = header.len();
    if num_cols == 0 {
        return lines;
    }

    // Calculate cell width
    let cell_width = |cell: &[MdNode]| -> usize { cell.iter().map(|n| n.content.width()).sum() };

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
    let col_widths = if table_width > available_width && available_width > num_cols + 1 {
        let content_width = available_width - num_cols - 1;
        let total_content: usize = col_widths.iter().sum();
        col_widths
            .iter()
            .map(|w| (w * content_width / total_content).max(3))
            .collect()
    } else {
        col_widths
    };

    let column_info = TableColumnInfo {
        widths: col_widths.clone(),
        alignments: alignments.to_vec(),
    };

    // Wrap cells to fit column widths and emit rows
    let wrap_and_emit_row = |lines: &mut Vec<RawLine>,
                             row: &[Vec<MdNode>],
                             is_header: bool,
                             column_info: &TableColumnInfo,
                             nesting: &Vec<MdLineContainer>| {
        // Wrap each cell's content to fit its column's inner width
        let wrapped_cells: Vec<Vec<Vec<MdNode>>> = row
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

        // Find max lines in this row
        let max_lines = wrapped_cells.iter().map(|c| c.len()).max().unwrap_or(1);

        // Emit one TableRow per wrapped line
        for line_idx in 0..max_lines {
            let cells_for_line: Vec<Vec<MdNode>> = wrapped_cells
                .iter()
                .map(|cell_lines| cell_lines.get(line_idx).cloned().unwrap_or_default())
                .collect();

            lines.push(RawLine {
                spans: Vec::new(),
                meta: LineMeta {
                    kind: RawLineKind::TableRow {
                        cells: cells_for_line,
                        column_info: column_info.clone(),
                        is_header,
                    },
                    nesting: nesting.clone(),
                },
            });
        }
    };

    // Top border
    lines.push(RawLine {
        spans: Vec::new(),
        meta: LineMeta {
            kind: RawLineKind::TableBorder {
                column_info: column_info.clone(),
                position: BorderPosition::Top,
            },
            nesting: nesting.clone(),
        },
    });

    // Header row (wrapped)
    wrap_and_emit_row(&mut lines, header, true, &column_info, &nesting);

    // Header separator
    lines.push(RawLine {
        spans: Vec::new(),
        meta: LineMeta {
            kind: RawLineKind::TableBorder {
                column_info: column_info.clone(),
                position: BorderPosition::HeaderSeparator,
            },
            nesting: nesting.clone(),
        },
    });

    // Data rows (wrapped)
    for row in rows {
        wrap_and_emit_row(&mut lines, row, false, &column_info, &nesting);
    }

    // Bottom border
    lines.push(RawLine {
        spans: Vec::new(),
        meta: LineMeta {
            kind: RawLineKind::TableBorder {
                column_info,
                position: BorderPosition::Bottom,
            },
            nesting,
        },
    });

    lines
}
