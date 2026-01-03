use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use unicode_width::UnicodeWidthStr as _;

use crate::{
    Event,
    model::DocumentId,
    widget_sources::{WidgetSource, WidgetSourceData},
    worker::pipeline::{
        line::{BLOCKQUOTE_BAR, BLOCKQUOTE_COLORS},
        markdown::{MdModifier, MdNode, TableAlignment},
        post_incr_source_id,
    },
};

pub fn table_to_events(
    document_id: DocumentId,
    source_id: &mut Option<usize>,
    width: u16,
    header: Vec<Vec<MdNode>>,
    rows: Vec<Vec<Vec<MdNode>>>,
    alignments: Vec<TableAlignment>,
    blockquote_depth: usize,
) -> Vec<Event> {
    let mut events = Vec::new();

    // Calculate prefix width for blockquotes
    let prefix_width = blockquote_depth * 2;
    let available_width = (width as usize).saturating_sub(prefix_width);

    // Calculate column widths based on content
    let num_cols = header.len();
    if num_cols == 0 {
        return events;
    }

    // Helper to calculate cell text width
    let cell_width = |cell: &[MdNode]| -> usize { cell.iter().map(|n| n.content.width()).sum() };

    // Find max width for each column
    let mut col_widths: Vec<usize> = header.iter().map(|c| cell_width(c)).collect();
    for row in &rows {
        for (i, cell) in row.iter().enumerate() {
            if i < col_widths.len() {
                col_widths[i] = col_widths[i].max(cell_width(cell));
            }
        }
    }

    // Add padding (1 space on each side of cell content)
    let col_widths: Vec<usize> = col_widths.iter().map(|w| w + 2).collect();

    // Calculate total table width: | col1 | col2 | ... |
    let table_width: usize = col_widths.iter().sum::<usize>() + num_cols + 1;

    // If table is wider than available, scale down proportionally
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

    let header_style = Style::default()
        .add_modifier(Modifier::BOLD)
        .fg(Color::Indexed(255));
    let border_style = Style::default().fg(Color::Indexed(240));
    let cell_style = Style::default();

    // Helper to build blockquote prefix spans
    let build_prefix = || -> Vec<Span<'static>> {
        let mut spans = Vec::new();
        for depth in 0..blockquote_depth {
            let color = BLOCKQUOTE_COLORS[depth.min(5)];
            spans.push(Span::styled(
                BLOCKQUOTE_BAR.to_owned(),
                Style::default().fg(color),
            ));
            spans.push(Span::from(" "));
        }
        spans
    };

    // Helper to render a row of cells
    let render_row = |cells: &[Vec<MdNode>], style: Style, is_header: bool| -> Vec<Span<'static>> {
        let mut spans = build_prefix();
        spans.push(Span::styled("│", border_style));

        for (i, cell) in cells.iter().enumerate() {
            let col_width = col_widths.get(i).copied().unwrap_or(3);
            let alignment = alignments.get(i).copied().unwrap_or_default();

            // Calculate cell content width
            let content_width: usize = cell.iter().map(|n| n.content.width()).sum();
            let inner_width = col_width.saturating_sub(2); // subtract padding
            let padding_total = inner_width.saturating_sub(content_width);

            let (left_pad, right_pad) = match alignment {
                TableAlignment::Center => (padding_total / 2, padding_total - padding_total / 2),
                TableAlignment::Right => (padding_total, 0),
                TableAlignment::Left => (0, padding_total),
            };

            // Add left padding
            spans.push(Span::from(format!(" {}", " ".repeat(left_pad))));

            // Add cell content with styling
            for node in cell {
                let mut node_style = if is_header { header_style } else { style };

                if node.extra.contains(MdModifier::Emphasis) {
                    node_style = node_style
                        .add_modifier(Modifier::ITALIC)
                        .fg(Color::Indexed(220));
                }
                if node.extra.contains(MdModifier::StrongEmphasis) {
                    node_style = node_style
                        .add_modifier(Modifier::BOLD)
                        .fg(Color::Indexed(220));
                }
                if node.extra.contains(MdModifier::Code) {
                    node_style = node_style.fg(Color::Indexed(203)).bg(Color::Indexed(236));
                }

                // Truncate if needed
                let content = if node.content.width() > inner_width {
                    let mut truncated = String::new();
                    let mut width = 0;
                    for c in node.content.chars() {
                        let cw = c.to_string().width();
                        if width + cw > inner_width.saturating_sub(1) {
                            truncated.push('…');
                            break;
                        }
                        truncated.push(c);
                        width += cw;
                    }
                    truncated
                } else {
                    node.content.clone()
                };

                spans.push(Span::styled(content, node_style));
            }

            // Add right padding
            spans.push(Span::from(format!("{} ", " ".repeat(right_pad))));
            spans.push(Span::styled("│", border_style));
        }

        // Fill in missing columns
        for i in cells.len()..num_cols {
            let col_width = col_widths.get(i).copied().unwrap_or(3);
            spans.push(Span::from(" ".repeat(col_width)));
            spans.push(Span::styled("│", border_style));
        }

        spans
    };

    // Render separator line
    let render_separator = |is_header: bool| -> Vec<Span<'static>> {
        let mut spans = build_prefix();
        let (left, mid, right, horiz) = if is_header {
            ("├", "┼", "┤", "─")
        } else {
            ("┌", "┬", "┐", "─")
        };

        spans.push(Span::styled(left, border_style));
        for (i, &col_w) in col_widths.iter().enumerate() {
            spans.push(Span::styled(horiz.repeat(col_w), border_style));
            if i < num_cols - 1 {
                spans.push(Span::styled(mid, border_style));
            }
        }
        spans.push(Span::styled(right, border_style));
        spans
    };

    // Render bottom border
    let render_bottom = || -> Vec<Span<'static>> {
        let mut spans = build_prefix();
        spans.push(Span::styled("└", border_style));
        for (i, &col_w) in col_widths.iter().enumerate() {
            spans.push(Span::styled("─".repeat(col_w), border_style));
            if i < num_cols - 1 {
                spans.push(Span::styled("┴", border_style));
            }
        }
        spans.push(Span::styled("┘", border_style));
        spans
    };

    // Top border
    events.push(Event::Parsed(
        document_id,
        WidgetSource {
            id: post_incr_source_id(source_id),
            height: 1,
            data: WidgetSourceData::Line(Line::from(render_separator(false)), Vec::new()),
        },
    ));

    // Header row
    events.push(Event::Parsed(
        document_id,
        WidgetSource {
            id: post_incr_source_id(source_id),
            height: 1,
            data: WidgetSourceData::Line(
                Line::from(render_row(&header, header_style, true)),
                Vec::new(),
            ),
        },
    ));

    // Header separator
    events.push(Event::Parsed(
        document_id,
        WidgetSource {
            id: post_incr_source_id(source_id),
            height: 1,
            data: WidgetSourceData::Line(Line::from(render_separator(true)), Vec::new()),
        },
    ));

    // Data rows
    for row in &rows {
        events.push(Event::Parsed(
            document_id,
            WidgetSource {
                id: post_incr_source_id(source_id),
                height: 1,
                data: WidgetSourceData::Line(
                    Line::from(render_row(row, cell_style, false)),
                    Vec::new(),
                ),
            },
        ));
    }

    // Bottom border
    events.push(Event::Parsed(
        document_id,
        WidgetSource {
            id: post_incr_source_id(source_id),
            height: 1,
            data: WidgetSourceData::Line(Line::from(render_bottom()), Vec::new()),
        },
    ));

    events
}
