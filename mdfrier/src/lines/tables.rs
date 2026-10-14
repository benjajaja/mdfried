use unicode_width::UnicodeWidthStr as _;

use super::{MdLineContainer, apply_decorators, nesting_to_prefix_spans};
use crate::{
    Line, LineKind, Mapper,
    link_tracker::TrackedUrl,
    markdown::{Modifier, Span, TableAlignment},
    wrap::wrap_md_spans,
};

#[derive(Debug, Clone, Copy, PartialEq)]
enum BorderPosition {
    Top,
    HeaderSeparator,
    Bottom,
}

/// Convert a table to output lines.
pub(crate) fn table_to_lines<M: Mapper>(
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

    // Pre-apply decorators to all cells before computing column widths so that
    // mapper transformations (e.g. hide_urls dropping link URL text) are already
    // reflected in the width calculation. This avoids double-decoration later.
    let decorated_header: Vec<Vec<Span>> = header
        .iter()
        .map(|cell| apply_decorators(cell.clone(), mapper))
        .collect();
    let decorated_rows: Vec<Vec<Vec<Span>>> = rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|cell| apply_decorators(cell.clone(), mapper))
                .collect()
        })
        .collect();

    // Compute the visual width of a decorated cell, accounting for hide_urls
    // (link URL spans are invisible when hide_urls is true).
    // We need to query hide_urls here as a special case, because URLs are not hidden until later,
    // because LinkTracker needs the actual span content.
    let hide_urls = mapper.hide_urls();
    let cell_vis_width = |cell: &[Span]| -> usize {
        cell.iter()
            .map(|s| {
                if hide_urls && s.modifiers.is_link_url() {
                    0
                } else {
                    s.content.width()
                }
            })
            .sum()
    };

    // Find max visual width for each column across header and all rows.
    let mut col_widths: Vec<usize> = decorated_header.iter().map(|c| cell_vis_width(c)).collect();
    for drow in &decorated_rows {
        for (i, cell) in drow.iter().enumerate() {
            if i < col_widths.len() {
                col_widths[i] = col_widths[i].max(cell_vis_width(cell));
            }
        }
    }

    // Add padding (1 space on each side).
    let col_widths: Vec<usize> = col_widths.iter().map(|w| w + 2).collect();

    // Per-column floor: smallest cell content in that column + padding, min 3.
    let col_floors: Vec<usize> = (0..num_cols)
        .map(|i| {
            let min_content = std::iter::once(cell_vis_width(&decorated_header[i]))
                .chain(
                    decorated_rows
                        .iter()
                        .filter_map(|row| row.get(i))
                        .map(|c| cell_vis_width(c)),
                )
                .min()
                .unwrap_or(0);
            (min_content + 2).max(3)
        })
        .collect();

    // Per-column median: lower-median of all cell content widths + padding, min 3.
    let col_medians: Vec<usize> = (0..num_cols)
        .map(|i| {
            let mut widths: Vec<usize> = std::iter::once(cell_vis_width(&decorated_header[i]))
                .chain(
                    decorated_rows
                        .iter()
                        .filter_map(|row| row.get(i))
                        .map(|c| cell_vis_width(c)),
                )
                .collect();
            widths.sort_unstable();
            let median_content = widths[(widths.len() - 1) / 2];
            (median_content + 2).max(3)
        })
        .collect();

    let table_width: usize = col_widths.iter().sum::<usize>() + num_cols + 1;
    let col_widths: Vec<usize> = if table_width > available_width && available_width > num_cols + 1
    {
        shrink_col_widths(
            col_widths,
            available_width,
            num_cols,
            &col_floors,
            &col_medians,
        )
    } else {
        col_widths
    };

    // Compute the character offset of each column's content-start within a rendered row,
    // assuming Left alignment (left_pad = 0). The URL positions from wrap_md_spans are
    // relative to 0, so we shift them by (col_base_offsets[i] + left_pad).
    //
    // Layout per row:
    //   <prefix> "|" (" " <left_pad> <content> <right_pad> " " "|") * num_cols
    //
    // base_offsets[i] = prefix_width + |"|"| + sum_{j<i}(col_widths[j] + |"|"|) + 1
    // where the trailing +1 is the mandatory leading " " of column i.
    let vertical_w = mapper.table_vertical().width();
    let col_base_offsets: Vec<usize> = {
        let mut offsets = Vec::with_capacity(num_cols);
        let mut pos = prefix_width + vertical_w + 1;
        for &col_w in &col_widths {
            offsets.push(pos);
            pos += col_w + vertical_w;
        }
        offsets
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
            urls: Vec::new(),
        }
    };

    // Helper to build row lines from pre-decorated cell spans.
    let build_row_lines = |row: &[Vec<Span>], is_header: bool| -> Vec<Line> {
        let vertical = mapper.table_vertical();

        // Wrap each already-decorated cell and track links via wrap_md_spans.
        let wrapped_cells: Vec<Vec<(Vec<Span>, Vec<TrackedUrl>)>> = row
            .iter()
            .enumerate()
            .map(|(i, cell)| {
                let col_width = col_widths.get(i).copied().unwrap_or(3);
                let inner_width = col_width.saturating_sub(2).max(1) as u16;
                // prefix_width=0 so available_width=inner_width and URL positions are
                // cell-relative (0-based); we shift them to row-absolute below.
                let wrapped = wrap_md_spans(inner_width, cell.clone(), 0, mapper);
                if wrapped.is_empty() {
                    vec![(Vec::new(), Vec::new())]
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
            let mut row_urls: Vec<TrackedUrl> = Vec::new();

            for (i, col_width) in col_widths.iter().enumerate() {
                let alignment = alignments.get(i).copied().unwrap_or(TableAlignment::Left);
                let (cell_spans, cell_urls) = wrapped_cells
                    .get(i)
                    .and_then(|c| c.get(line_idx))
                    .map_or((&[][..], &[][..]), |(s, u)| (s.as_slice(), u.as_slice()));

                // Compute visible content width for padding (same rule as wrap_md_spans).
                let content_width: usize = cell_spans
                    .iter()
                    .map(|s| {
                        if hide_urls && s.modifiers.is_link_url() {
                            0
                        } else {
                            s.content.width()
                        }
                    })
                    .sum();
                let inner_width = col_width.saturating_sub(2);
                let padding_total = inner_width.saturating_sub(content_width);

                let (left_pad, right_pad) = match alignment {
                    TableAlignment::Center => {
                        (padding_total / 2, padding_total - padding_total / 2)
                    }
                    TableAlignment::Right => (padding_total, 0),
                    TableAlignment::Left => (0, padding_total),
                };

                // Shift cell-relative URL positions to row-absolute positions.
                let base = col_base_offsets.get(i).copied().unwrap_or(0);
                let content_start = (base + left_pad) as u16;
                for url in cell_urls {
                    match url {
                        TrackedUrl::Link {
                            start,
                            lines,
                            end,
                            url,
                            is_reference,
                        } => {
                            row_urls.push(TrackedUrl::Link {
                                start: start + content_start,
                                lines: *lines,
                                end: end + content_start,
                                url: url.clone(),
                                is_reference: *is_reference,
                            });
                        }
                        TrackedUrl::Image { desc, url } => {
                            row_urls.push(TrackedUrl::Image {
                                desc: desc.clone(),
                                url: url.clone(),
                            });
                        }
                    }
                }

                // Left padding + space
                spans.push(Span::new(
                    format!(" {}", " ".repeat(left_pad)),
                    Modifier::empty(),
                ));

                // Cell content (already decorated)
                spans.extend(cell_spans.iter().cloned());

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
                urls: row_urls,
            });
        }

        result
    };

    // Top border
    lines.push(build_border(BorderPosition::Top));

    // Header row
    lines.extend(build_row_lines(&decorated_header, true));

    // Header separator
    lines.push(build_border(BorderPosition::HeaderSeparator));

    // Data rows
    for drow in &decorated_rows {
        lines.extend(build_row_lines(drow, false));
    }

    // Bottom border
    lines.push(build_border(BorderPosition::Bottom));

    lines
}

/// Shrink column widths to fit within `available_width` using a 3-pass greedy algorithm.
///
/// Each `col_width` includes 2 chars of padding (1 on each side). The table's rendered
/// width is `sum(col_widths) + num_cols + 1` (one `|` separator per column plus one at
/// the start).
///
/// All passes respect `col_floors` as a minimum width per column. If the table still
/// doesn't fit after all three passes, the algorithm repeats without floor protection.
fn shrink_col_widths(
    mut col_widths: Vec<usize>,
    available_width: usize,
    num_cols: usize,
    col_floors: &[usize],
    col_medians: &[usize],
) -> Vec<usize> {
    let no_floor = vec![1_usize; col_widths.len()];

    shrink_dominant(&mut col_widths, available_width, num_cols, col_floors);
    shrink_above_median(
        &mut col_widths,
        available_width,
        num_cols,
        col_floors,
        col_medians,
    );
    shrink_widest(&mut col_widths, available_width, num_cols, col_floors);

    if table_width(&col_widths, num_cols) <= available_width {
        return col_widths;
    }

    // Last resort: repeat without floor protection.
    shrink_dominant(&mut col_widths, available_width, num_cols, &no_floor);
    shrink_above_median(
        &mut col_widths,
        available_width,
        num_cols,
        &no_floor,
        col_medians,
    );
    shrink_widest(&mut col_widths, available_width, num_cols, &no_floor);

    col_widths
}

/// Shrink the column hogging more than half the current table width.
fn shrink_dominant(col_widths: &mut [usize], available: usize, num_cols: usize, floors: &[usize]) {
    loop {
        if table_width(col_widths, num_cols) <= available {
            return;
        }
        let t = table_width(col_widths, num_cols);
        let half = t / 2;
        let Some(i) = col_widths
            .iter()
            .enumerate()
            .filter(|&(i, &w)| w > half && w > floors[i])
            .max_by_key(|&(_, &w)| w)
            .map(|(i, _)| i)
        else {
            return;
        };
        col_widths[i] -= 1;
    }
}

/// Shrink the column most bloated relative to its typical (median) content width.
fn shrink_above_median(
    col_widths: &mut [usize],
    available: usize,
    num_cols: usize,
    floors: &[usize],
    medians: &[usize],
) {
    loop {
        if table_width(col_widths, num_cols) <= available {
            return;
        }
        let Some(i) = col_widths
            .iter()
            .enumerate()
            .filter(|&(i, &w)| w > floors[i] && w > medians[i])
            .max_by_key(|&(i, &w)| w.saturating_sub(medians[i]))
            .map(|(i, _)| i)
        else {
            return;
        };
        col_widths[i] -= 1;
    }
}

/// Shrink the widest column.
fn shrink_widest(col_widths: &mut [usize], available: usize, num_cols: usize, floors: &[usize]) {
    loop {
        if table_width(col_widths, num_cols) <= available {
            return;
        }
        let Some(i) = col_widths
            .iter()
            .enumerate()
            .filter(|&(i, &w)| w > floors[i])
            .max_by_key(|&(_, &w)| w)
            .map(|(i, _)| i)
        else {
            return;
        };
        col_widths[i] -= 1;
    }
}

fn table_width(col_widths: &[usize], num_cols: usize) -> usize {
    col_widths.iter().sum::<usize>() + num_cols + 1
}

#[cfg(test)]
#[expect(clippy::unwrap_used)]
mod tests {
    use pretty_assertions::assert_eq;
    use tree_sitter::Parser;

    use crate::link_tracker::TrackedUrl;
    use crate::markdown::MdIterator;
    use crate::{DefaultMapper, Line, LineIterator, LineKind, Mapper};

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
    fn table_link_width_with_hide_urls() {
        // A table cell containing a link must be sized by the *visible* text ("text"), not the
        // full raw span including the URL. With hide_urls the URL span is zero-width for layout
        // purposes, so the column width must reflect only the link description.
        //
        // "col" header = 3 visible chars. "[text](http://example.com)" cell = 4 visible chars.
        // → inner_width = 4, col_width = 6. Border: "+" + "------" + "+" = 8 chars total.
        // Without the fix, the raw cell spans (including the 18-char URL) would inflate
        // col_width to 28, producing a 30-char border line.
        let source = "| col |\n|-----|\n| [text](http://example.com) |\n";
        let mut parser = make_parser();
        let mut inline_parser = make_inline_parser();
        let tree = parser.parse(source, None).unwrap();
        let iter = MdIterator::new(tree, &mut inline_parser, source);

        struct HideUrlsMapper;
        impl Mapper for HideUrlsMapper {
            fn hide_urls(&self) -> bool {
                true
            }
            fn link_desc_open(&self) -> &str {
                ""
            }
            fn link_desc_close(&self) -> &str {
                ""
            }
        }

        let line_iter = LineIterator::new(iter, 80, &HideUrlsMapper {});
        let lines: Vec<Line> = line_iter.collect();

        // Lines: top border, header row, separator, data row, bottom border.
        assert_eq!(lines.len(), 5);

        // Border width reflects col_width. Expected: "+" + 6 chars + "+" = 8.
        // A URL-inflated column would produce "+" + 28 chars + "+" = 30.
        let top_border: String = lines[0].spans.iter().map(|s| s.content.as_str()).collect();
        assert_eq!(
            top_border.len(),
            8,
            "border must be sized for visible text only, got: {top_border:?}"
        );

        // Data row must track the link even with hide_urls.
        assert_eq!(lines[3].urls.len(), 1, "data row must track the cell link");
        assert!(
            matches!(&lines[3].urls[0], TrackedUrl::Link { url, .. } if url == "http://example.com"),
            "tracked URL must match the cell link"
        );
    }

    #[test]
    fn table_link_tracked() {
        // Links inside table cells must be tracked (non-empty urls on the row Line).
        let source = "| Header |\n|---------|\n| [click](http://example.com) |\n";
        let mut parser = make_parser();
        let mut inline_parser = make_inline_parser();
        let tree = parser.parse(source, None).unwrap();
        let iter = MdIterator::new(tree, &mut inline_parser, source);

        let line_iter = LineIterator::new(iter, 80, &DefaultMapper {});
        let lines: Vec<Line> = line_iter.collect();

        // Lines: top border, header row, separator, data row, bottom border.
        assert_eq!(lines.len(), 5);

        // Data row must have exactly one tracked link.
        assert_eq!(lines[3].urls.len(), 1, "data row must track the cell link");
        assert!(
            matches!(&lines[3].urls[0], TrackedUrl::Link { url, .. } if url == "http://example.com"),
            "tracked URL must match the cell link"
        );
    }

    #[test]
    fn table_narrow_columns_not_squished_by_wide_column() {
        // When a wide column forces the table to scale down, the proportional scaling
        // must not squish narrow columns below their natural content width. Previously,
        // all columns were scaled by the same ratio, so short columns like "step" (4 chars)
        // and "verdict" (7 chars) ended up with inner_width=1, causing each character to
        // wrap to its own line.
        //
        // Expected: short columns keep their natural width; only the wide column shrinks.
        // The header row must fit on a single visual line (no cell wraps).
        let source = "\
| scenario | step | verdict | what I saw |
|---|---|---|---|
| smoke-desktop | 1 | PASS | health: bodyChars 1425, greyWindow false, pageErrors 0. No tour dialog; a key-paste dialog covered the app instead (see doubts) |
";
        let mut parser = make_parser();
        let mut inline_parser = make_inline_parser();
        let tree = parser.parse(source, None).unwrap();
        let iter = MdIterator::new(tree, &mut inline_parser, source);

        let line_iter = LineIterator::new(iter, 80, &DefaultMapper {});
        let lines: Vec<Line> = line_iter.collect();

        let header_lines: Vec<_> = lines
            .iter()
            .filter(|l| matches!(l.kind, LineKind::TableRow { is_header: true }))
            .collect();

        assert_eq!(
            header_lines.len(),
            1,
            "header must fit on one visual line; narrow columns must not be squished below \
             their content width. Got {} header lines: {:#?}",
            header_lines.len(),
            header_lines
                .iter()
                .map(|l| l
                    .spans
                    .iter()
                    .map(|s| s.content.as_str())
                    .collect::<String>())
                .collect::<Vec<_>>(),
        );
    }
}
