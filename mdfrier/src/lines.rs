use unicode_width::UnicodeWidthStr;

use crate::{
    markdown::{MdContainer, MdContent, MdNode, MdSection, TableAlignment},
    wrap::wrap_md_spans,
};

/// A single output line from the markdown parser.
#[derive(Debug, Clone, PartialEq)]
pub struct MdLine {
    /// The text spans making up this line.
    pub spans: Vec<MdNode>,
    /// Metadata about this line.
    pub meta: LineMeta,
}

impl MdLine {
    /// Create a blank line.
    pub fn blank() -> Self {
        Self {
            spans: Vec::new(),
            meta: LineMeta {
                kind: LineKind::Blank,
                nesting: Vec::new(),
            },
        }
    }

    /// Create a blank line with blockquote nesting.
    pub fn blockquote_blank(depth: usize) -> Self {
        Self {
            spans: Vec::new(),
            meta: LineMeta {
                kind: LineKind::Blank,
                nesting: vec![Container::Blockquote; depth],
            },
        }
    }
}

/// Metadata about a markdown line.
#[derive(Debug, Clone, PartialEq)]
pub struct LineMeta {
    /// The kind of line content.
    pub kind: LineKind,
    /// Nesting containers (blockquotes and list items).
    pub nesting: Vec<Container>,
}

impl LineMeta {
    /// Get the blockquote nesting depth.
    pub fn blockquote_depth(&self) -> usize {
        self.nesting
            .iter()
            .filter(|c| matches!(c, Container::Blockquote))
            .count()
    }

    /// Calculate the display width of the prefix.
    pub fn prefix_width(&self) -> usize {
        self.nesting
            .iter()
            .map(|c| match c {
                Container::Blockquote => 2, // "▌ "
                Container::ListItem { marker, .. } => marker.width(),
            })
            .sum()
    }
}

/// A nesting container (blockquote or list item).
#[derive(Debug, Clone, PartialEq)]
pub enum Container {
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

/// The kind of content a line represents.
#[derive(Debug, Clone, PartialEq)]
pub enum LineKind {
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

/// Convert a markdown section to output lines.
pub fn section_to_lines(width: u16, section: &MdSection) -> Vec<MdLine> {
    let nesting = convert_nesting(&section.nesting, section.is_list_continuation);

    match &section.content {
        MdContent::Paragraph(mdspans) if mdspans.is_empty() => {
            vec![MdLine {
                spans: Vec::new(),
                meta: LineMeta {
                    kind: LineKind::Blank,
                    nesting,
                },
            }]
        }
        MdContent::Paragraph(mdspans) => {
            let prefix_width = nesting
                .iter()
                .map(|c| match c {
                    Container::Blockquote => 2,
                    Container::ListItem { marker, .. } => marker.width(),
                })
                .sum();
            let wrapped_lines = wrap_md_spans(width, mdspans.clone(), prefix_width);
            wrapped_lines_to_mdlines(wrapped_lines, nesting)
        }
        MdContent::Header { tier, text } => {
            vec![MdLine {
                spans: vec![MdNode::from(text.clone())],
                meta: LineMeta {
                    kind: LineKind::Header(*tier),
                    nesting,
                },
            }]
        }
        MdContent::CodeBlock { language, code } => code
            .lines()
            .map(|line| MdLine {
                spans: vec![MdNode::from(line.to_owned())],
                meta: LineMeta {
                    kind: LineKind::CodeBlock {
                        language: language.clone(),
                    },
                    nesting: nesting.clone(),
                },
            })
            .collect(),
        MdContent::HorizontalRule => {
            vec![MdLine {
                spans: Vec::new(),
                meta: LineMeta {
                    kind: LineKind::HorizontalRule,
                    nesting,
                },
            }]
        }
        MdContent::Table {
            header,
            rows,
            alignments,
        } => table_to_mdlines(width, header, rows, alignments, nesting),
    }
}

/// Convert MdContainer nesting to Container nesting.
fn convert_nesting(md_nesting: &[MdContainer], is_list_continuation: bool) -> Vec<Container> {
    let mut nesting = Vec::new();

    // Find the index of the last ListItem to mark it as continuation if needed
    let last_list_item_idx = md_nesting
        .iter()
        .rposition(|c| matches!(c, MdContainer::ListItem(_)));

    for (idx, c) in md_nesting.iter().enumerate() {
        match c {
            MdContainer::Blockquote(_) => {
                nesting.push(Container::Blockquote);
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
                    // Parse ordered list number
                    let num: u32 = marker
                        .original
                        .chars()
                        .take_while(|c| c.is_ascii_digit())
                        .collect::<String>()
                        .parse()
                        .unwrap_or(1);
                    ListMarker::Ordered(num)
                } else {
                    ListMarker::Unordered(bullet)
                };

                // Only the innermost list item can be a continuation
                let continuation = is_list_continuation && last_list_item_idx == Some(idx);

                nesting.push(Container::ListItem {
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

fn wrapped_lines_to_mdlines(
    wrapped_lines: Vec<crate::wrap::WrappedLine>,
    nesting: Vec<Container>,
) -> Vec<MdLine> {
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
                    Container::Blockquote => Container::Blockquote,
                    Container::ListItem { marker, .. } => Container::ListItem {
                        marker: marker.clone(),
                        continuation: true,
                    },
                })
                .collect()
        };

        // Create text line
        if !wrapped_line.spans.is_empty() {
            lines.push(MdLine {
                spans: wrapped_line.spans,
                meta: LineMeta {
                    kind: LineKind::Paragraph,
                    nesting: line_nesting.clone(),
                },
            });
        }

        // Create image lines
        for img in wrapped_line.images {
            lines.push(MdLine {
                spans: Vec::new(),
                meta: LineMeta {
                    kind: LineKind::Image {
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

fn table_to_mdlines(
    width: u16,
    header: &[Vec<MdNode>],
    rows: &[Vec<Vec<MdNode>>],
    alignments: &[TableAlignment],
    nesting: Vec<Container>,
) -> Vec<MdLine> {
    let mut lines = Vec::new();

    let prefix_width: usize = nesting
        .iter()
        .map(|c| match c {
            Container::Blockquote => 2,
            Container::ListItem { marker, .. } => marker.width(),
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
        widths: col_widths,
        alignments: alignments.to_vec(),
    };

    // Top border
    lines.push(MdLine {
        spans: Vec::new(),
        meta: LineMeta {
            kind: LineKind::TableBorder {
                column_info: column_info.clone(),
                position: BorderPosition::Top,
            },
            nesting: nesting.clone(),
        },
    });

    // Header row
    lines.push(MdLine {
        spans: Vec::new(),
        meta: LineMeta {
            kind: LineKind::TableRow {
                cells: header.to_vec(),
                column_info: column_info.clone(),
                is_header: true,
            },
            nesting: nesting.clone(),
        },
    });

    // Header separator
    lines.push(MdLine {
        spans: Vec::new(),
        meta: LineMeta {
            kind: LineKind::TableBorder {
                column_info: column_info.clone(),
                position: BorderPosition::HeaderSeparator,
            },
            nesting: nesting.clone(),
        },
    });

    // Data rows
    for row in rows {
        lines.push(MdLine {
            spans: Vec::new(),
            meta: LineMeta {
                kind: LineKind::TableRow {
                    cells: row.clone(),
                    column_info: column_info.clone(),
                    is_header: false,
                },
                nesting: nesting.clone(),
            },
        });
    }

    // Bottom border
    lines.push(MdLine {
        spans: Vec::new(),
        meta: LineMeta {
            kind: LineKind::TableBorder {
                column_info,
                position: BorderPosition::Bottom,
            },
            nesting,
        },
    });

    lines
}
