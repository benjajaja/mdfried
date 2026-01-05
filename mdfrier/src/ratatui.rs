//! Ratatui integration for mdfrier.
//!
//! This module provides conversion from `MdLine` to styled ratatui `Line` widgets,
//! along with semantic `Tag`s that can be used by the application.

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use unicode_width::UnicodeWidthStr;

use crate::{
    lines::{BorderPosition, Container, LineKind, ListMarker, MdLine, TableColumnInfo},
    markdown::{MdModifier, MdNode, TableAlignment},
};

// ============================================================================
// Styling constants
// ============================================================================

/// Blockquote bar character.
pub const BLOCKQUOTE_BAR: &str = "▌";

/// Link description opening bracket replacement.
pub const LINK_DESC_OPEN: &str = "▐";
/// Link description closing bracket replacement.
pub const LINK_DESC_CLOSE: &str = "▌";
/// Link URL opening paren replacement.
pub const LINK_URL_OPEN: &str = "◖";
/// Link URL closing paren replacement.
pub const LINK_URL_CLOSE: &str = "◗";

/// Colors for nested blockquotes (cycles through these).
pub const BLOCKQUOTE_COLORS: [Color; 6] = [
    Color::Indexed(202),
    Color::Indexed(203),
    Color::Indexed(204),
    Color::Indexed(205),
    Color::Indexed(206),
    Color::Indexed(207),
];

/// Link background color.
pub const COLOR_LINK_BG: Color = Color::Indexed(237);
/// Link foreground color.
pub const COLOR_LINK_FG: Color = Color::Indexed(4);
/// Prefix color (list markers, etc.).
pub const COLOR_PREFIX: Color = Color::Indexed(189);
/// Emphasis/italic color.
pub const COLOR_EMPHASIS: Color = Color::Indexed(220);
/// Code background color.
pub const COLOR_CODE_BG: Color = Color::Indexed(236);
/// Code foreground color.
pub const COLOR_CODE_FG: Color = Color::Indexed(203);
/// Horizontal rule color.
pub const COLOR_HR: Color = Color::Indexed(240);
/// Table border color.
pub const COLOR_TABLE_BORDER: Color = Color::Indexed(240);
/// Table header color.
pub const COLOR_TABLE_HEADER: Color = Color::Indexed(255);

// ============================================================================
// Tag enum - semantic information about lines
// ============================================================================

/// Semantic tags that describe line content for application use.
#[derive(Debug, Clone, PartialEq)]
pub enum Tag {
    /// Image reference (url, description).
    Image(String, String),
    /// Link reference (span index in rendered Line, url).
    Link(usize, String),
    /// Header with tier (1-6).
    Header(u8),
    /// Code block with optional language.
    CodeBlock(Option<String>),
    /// Horizontal rule.
    HorizontalRule,
    /// Table row.
    TableRow,
    /// Blank line.
    Blank,
}

// ============================================================================
// Main conversion function
// ============================================================================

/// Convert an `MdLine` to a styled ratatui `Line` and semantic `Tag`s.
///
/// # Arguments
///
/// * `md_line` - The markdown line to convert
/// * `width` - Terminal width (used for horizontal rules)
///
/// # Returns
///
/// A tuple of (styled Line, list of Tags)
pub fn render_line(md_line: MdLine, width: u16) -> (Line<'static>, Vec<Tag>) {
    let MdLine { spans, meta } = md_line;

    match meta.kind {
        LineKind::Paragraph => render_paragraph(spans, &meta.nesting),
        LineKind::Header(tier) => render_header(spans, tier, meta.blockquote_depth()),
        LineKind::CodeBlock { ref language } => {
            render_code_line(spans, language, meta.blockquote_depth(), width)
        }
        LineKind::HorizontalRule => render_horizontal_rule(meta.blockquote_depth(), width),
        LineKind::TableRow {
            ref cells,
            ref column_info,
            is_header,
        } => render_table_row(cells, column_info, is_header, meta.blockquote_depth()),
        LineKind::TableBorder {
            ref column_info,
            position,
        } => render_table_border(column_info, position, meta.blockquote_depth()),
        LineKind::Image { url, description } => {
            // Images are handled specially by the application
            let line = Line::default();
            let tags = vec![Tag::Image(url, description)];
            (line, tags)
        }
        LineKind::Blank => render_blank(meta.blockquote_depth()),
    }
}

// ============================================================================
// Render functions for each line kind
// ============================================================================

fn render_paragraph(spans: Vec<MdNode>, nesting: &[Container]) -> (Line<'static>, Vec<Tag>) {
    let mut line_spans = Vec::new();
    let mut tags = Vec::new();

    // Add styled prefix from nesting containers
    line_spans.extend(render_nesting_prefix(nesting));

    for mdspan in spans {
        // Track links for tags - store span index (URL span position in line_spans)
        if mdspan.extra.contains(MdModifier::LinkURL)
            && !mdspan.extra.contains(MdModifier::LinkURLWrapper)
        {
            tags.push(Tag::Link(line_spans.len(), mdspan.content.clone()));
        }
        line_spans.push(span_from_mdnode(mdspan));
    }

    (Line::from(line_spans), tags)
}

fn render_header(
    spans: Vec<MdNode>,
    tier: u8,
    blockquote_depth: usize,
) -> (Line<'static>, Vec<Tag>) {
    let mut line_spans = Vec::new();

    // Add blockquote prefix if inside blockquote
    if blockquote_depth > 0 {
        line_spans.extend(build_blockquote_prefix(blockquote_depth));
    }

    // Headers are rendered as plain text (application handles big text rendering)
    for span in spans {
        line_spans.push(Span::from(span.content));
    }

    let tags = vec![Tag::Header(tier)];
    (Line::from(line_spans), tags)
}

fn render_code_line(
    spans: Vec<MdNode>,
    language: &str,
    blockquote_depth: usize,
    width: u16,
) -> (Line<'static>, Vec<Tag>) {
    let mut line_spans = Vec::new();

    let prefix_width = blockquote_depth * 2;
    let available_width = (width as usize).saturating_sub(prefix_width);

    // Add blockquote prefix
    if blockquote_depth > 0 {
        line_spans.extend(build_blockquote_prefix(blockquote_depth));
    }

    let code_style = Style::default().fg(COLOR_CODE_FG).bg(COLOR_CODE_BG);

    // Get code content
    let content = if spans.is_empty() {
        String::new()
    } else {
        spans.into_iter().map(|s| s.content).collect::<String>()
    };

    // Pad to fill width with background color
    let line_width = content.width();
    let padding = available_width.saturating_sub(line_width);
    let padded_content = format!("{}{}", content, " ".repeat(padding));

    line_spans.push(Span::styled(padded_content, code_style));

    let tags = vec![Tag::CodeBlock(if language.is_empty() {
        None
    } else {
        Some(language.to_owned())
    })];
    (Line::from(line_spans), tags)
}

fn render_horizontal_rule(blockquote_depth: usize, width: u16) -> (Line<'static>, Vec<Tag>) {
    let mut line_spans = Vec::new();

    let prefix_width = blockquote_depth * 2;
    let available_width = (width as usize).saturating_sub(prefix_width);

    // Add blockquote prefix
    if blockquote_depth > 0 {
        line_spans.extend(build_blockquote_prefix(blockquote_depth));
    }

    let rule_style = Style::default().fg(COLOR_HR);
    line_spans.push(Span::styled("─".repeat(available_width), rule_style));

    let tags = vec![Tag::HorizontalRule];
    (Line::from(line_spans), tags)
}

fn render_blank(blockquote_depth: usize) -> (Line<'static>, Vec<Tag>) {
    let mut line_spans = Vec::new();

    if blockquote_depth > 0 {
        line_spans.extend(build_blockquote_prefix(blockquote_depth));
    }

    let tags = vec![Tag::Blank];
    (Line::from(line_spans), tags)
}

fn render_table_row(
    cells: &[Vec<MdNode>],
    column_info: &TableColumnInfo,
    is_header: bool,
    blockquote_depth: usize,
) -> (Line<'static>, Vec<Tag>) {
    let mut line_spans = Vec::new();

    // Add blockquote prefix
    if blockquote_depth > 0 {
        line_spans.extend(build_blockquote_prefix(blockquote_depth));
    }

    let border_style = Style::default().fg(COLOR_TABLE_BORDER);
    let header_style = Style::default()
        .add_modifier(Modifier::BOLD)
        .fg(COLOR_TABLE_HEADER);

    line_spans.push(Span::styled("│", border_style));

    for (i, col_width) in column_info.widths.iter().enumerate() {
        let alignment = column_info
            .alignments
            .get(i)
            .copied()
            .unwrap_or(TableAlignment::Left);

        // Get cell spans
        let cell_spans = cells.get(i).map_or(&[][..], |v| v.as_slice());

        let content_width: usize = cell_spans.iter().map(|s| s.content.width()).sum();
        let inner_width = col_width.saturating_sub(2);
        let padding_total = inner_width.saturating_sub(content_width);

        let (left_pad, right_pad) = match alignment {
            TableAlignment::Center => (padding_total / 2, padding_total - padding_total / 2),
            TableAlignment::Right => (padding_total, 0),
            TableAlignment::Left => (0, padding_total),
        };

        // Left padding
        line_spans.push(Span::from(format!(" {}", " ".repeat(left_pad))));

        // Cell content
        for node in cell_spans {
            let mut style = if is_header {
                header_style
            } else {
                Style::default()
            };

            if node.extra.contains(MdModifier::Emphasis) {
                style = style.add_modifier(Modifier::ITALIC).fg(COLOR_EMPHASIS);
            }
            if node.extra.contains(MdModifier::StrongEmphasis) {
                style = style.add_modifier(Modifier::BOLD).fg(COLOR_EMPHASIS);
            }
            if node.extra.contains(MdModifier::Code) {
                style = style.fg(COLOR_CODE_FG).bg(COLOR_CODE_BG);
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

            line_spans.push(Span::styled(content, style));
        }

        // Right padding
        line_spans.push(Span::from(format!("{} ", " ".repeat(right_pad))));
        line_spans.push(Span::styled("│", border_style));
    }

    // Fill missing columns
    let num_cols = column_info.widths.len();
    for i in cells.len()..num_cols {
        let col_width = column_info.widths.get(i).copied().unwrap_or(3);
        line_spans.push(Span::from(" ".repeat(col_width)));
        line_spans.push(Span::styled("│", border_style));
    }

    let tags = vec![Tag::TableRow];
    (Line::from(line_spans), tags)
}

fn render_table_border(
    column_info: &TableColumnInfo,
    position: BorderPosition,
    blockquote_depth: usize,
) -> (Line<'static>, Vec<Tag>) {
    let mut line_spans = Vec::new();

    // Add blockquote prefix
    if blockquote_depth > 0 {
        line_spans.extend(build_blockquote_prefix(blockquote_depth));
    }

    let border_style = Style::default().fg(COLOR_TABLE_BORDER);
    let num_cols = column_info.widths.len();

    let (left, mid, right) = match position {
        BorderPosition::Top => ("┌", "┬", "┐"),
        BorderPosition::HeaderSeparator => ("├", "┼", "┤"),
        BorderPosition::Bottom => ("└", "┴", "┘"),
    };

    line_spans.push(Span::styled(left, border_style));
    for (i, &col_w) in column_info.widths.iter().enumerate() {
        line_spans.push(Span::styled("─".repeat(col_w), border_style));
        if i < num_cols - 1 {
            line_spans.push(Span::styled(mid, border_style));
        }
    }
    line_spans.push(Span::styled(right, border_style));

    let tags = vec![Tag::TableRow];
    (Line::from(line_spans), tags)
}

// ============================================================================
// Helper functions
// ============================================================================

/// Build blockquote prefix spans.
fn build_blockquote_prefix(depth: usize) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for d in 0..depth {
        let color = BLOCKQUOTE_COLORS[d.min(5)];
        spans.push(Span::styled(
            BLOCKQUOTE_BAR.to_owned(),
            Style::default().fg(color),
        ));
        spans.push(Span::from(" "));
    }
    spans
}

/// Render prefix spans from nesting containers.
fn render_nesting_prefix(nesting: &[Container]) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut blockquote_idx = 0;

    // Find the last (innermost) list item - only render that one with a marker
    let last_list_item_idx = nesting
        .iter()
        .rposition(|c| matches!(c, Container::ListItem { .. }));

    for (i, container) in nesting.iter().enumerate() {
        match container {
            Container::Blockquote => {
                let color = BLOCKQUOTE_COLORS[blockquote_idx.min(5)];
                spans.push(Span::styled(
                    BLOCKQUOTE_BAR.to_owned(),
                    Style::default().fg(color),
                ));
                spans.push(Span::from(" "));
                blockquote_idx += 1;
            }
            Container::ListItem { marker, continuation } => {
                // Render marker only for innermost non-continuation list item
                if Some(i) == last_list_item_idx && !continuation {
                    let marker_text = match marker {
                        ListMarker::Unordered(bullet) => format!("{} ", bullet.char()),
                        ListMarker::Ordered(n) => format!("{}. ", n),
                        ListMarker::TaskUnchecked(bullet) => format!("{} [ ] ", bullet.char()),
                        ListMarker::TaskChecked(bullet) => format!("{} [✓] ", bullet.char()),
                    };
                    spans.push(Span::styled(marker_text, Style::default().fg(COLOR_PREFIX)));
                } else {
                    // For outer list items or continuations, render indentation only
                    let indent = marker.width();
                    spans.push(Span::from(" ".repeat(indent)));
                }
            }
        }
    }

    spans
}

/// Convert an MdNode to a styled ratatui Span.
fn span_from_mdnode(mdnode: MdNode) -> Span<'static> {
    let mut style = Style::default();

    if mdnode.extra.contains(MdModifier::Emphasis) {
        style = style.add_modifier(Modifier::ITALIC).fg(COLOR_EMPHASIS);
    }
    if mdnode.extra.contains(MdModifier::StrongEmphasis) {
        style = style.add_modifier(Modifier::BOLD).fg(COLOR_EMPHASIS);
    }
    if mdnode.extra.contains(MdModifier::Code) {
        style = style.fg(COLOR_CODE_FG).bg(COLOR_CODE_BG);
    }

    if mdnode.extra.contains(MdModifier::LinkURLWrapper) {
        let bracket = if mdnode.content == "(" {
            LINK_URL_OPEN
        } else {
            LINK_URL_CLOSE
        };
        return Span::styled(bracket, style.fg(COLOR_LINK_BG));
    }
    if mdnode.extra.contains(MdModifier::LinkURL) {
        style = style.fg(COLOR_LINK_FG).bg(COLOR_LINK_BG).underlined();
    }

    if mdnode.extra.contains(MdModifier::LinkDescriptionWrapper) {
        let bracket = if mdnode.content == "[" {
            LINK_DESC_OPEN
        } else {
            LINK_DESC_CLOSE
        };
        return Span::styled(bracket, style.fg(COLOR_LINK_BG));
    }
    if mdnode.extra.contains(MdModifier::LinkDescription) {
        style = style.fg(COLOR_LINK_FG).bg(COLOR_LINK_BG);
    }

    Span::styled(mdnode.content, style)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MdFrier, lines::LineMeta};
    use pretty_assertions::assert_eq;

    /// Convert a ratatui Line to string for testing.
    fn line_to_string(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    /// Parse and render markdown, returning string output.
    fn parse_and_render(input: &str, width: u16) -> String {
        let mut frier = MdFrier::new().unwrap();
        let lines: Vec<_> = frier.parse(width, input.to_owned()).collect();
        lines
            .into_iter()
            .map(|md_line| {
                let (rendered, _) = render_line(md_line, width);
                line_to_string(&rendered)
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn render_simple_text() {
        let line = MdLine {
            spans: vec![MdNode::from("Hello world!")],
            meta: LineMeta {
                kind: LineKind::Paragraph,
                nesting: Vec::new(),
            },
        };

        let (rendered, tags) = render_line(line, 80);
        assert_eq!(rendered.spans.len(), 1);
        assert_eq!(rendered.spans[0].content, "Hello world!");
        assert!(tags.is_empty());
    }

    #[test]
    fn render_styled_text() {
        let line = MdLine {
            spans: vec![
                MdNode::from("Hello "),
                MdNode::new("world".to_owned(), MdModifier::Emphasis),
                MdNode::from("!"),
            ],
            meta: LineMeta {
                kind: LineKind::Paragraph,
                nesting: Vec::new(),
            },
        };

        let (rendered, _tags) = render_line(line, 80);
        assert_eq!(rendered.spans.len(), 3);
        assert_eq!(rendered.spans[1].content, "world");
        // Check styling is applied
        assert!(
            rendered.spans[1]
                .style
                .add_modifier
                .contains(Modifier::ITALIC)
        );
    }

    #[test]
    fn render_blockquote() {
        let line = MdLine {
            spans: vec![MdNode::from("Quoted text")],
            meta: LineMeta {
                kind: LineKind::Paragraph,
                nesting: vec![Container::Blockquote],
            },
        };

        let (rendered, _tags) = render_line(line, 80);
        // Should have blockquote bar + space + text
        assert!(rendered.spans.len() >= 2);
        assert_eq!(rendered.spans[0].content, BLOCKQUOTE_BAR);
    }

    #[test]
    fn render_header_produces_tag() {
        let line = MdLine {
            spans: vec![MdNode::from("My Header")],
            meta: LineMeta {
                kind: LineKind::Header(1),
                nesting: Vec::new(),
            },
        };

        let (_rendered, tags) = render_line(line, 80);
        assert_eq!(tags.len(), 1);
        assert!(matches!(tags[0], Tag::Header(1)));
    }

    #[test]
    fn integration_parse_and_render() {
        let mut frier = MdFrier::new().unwrap();
        let lines: Vec<_> = frier.parse(80, "Hello *world*!".to_owned()).collect();

        let (rendered, _tags) = render_line(lines[0].clone(), 80);
        let content: String = rendered.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(content, "Hello world!");
    }

    #[test]
    fn render_link() {
        let mut frier = MdFrier::new().unwrap();
        let lines: Vec<_> = frier
            .parse(80, "[text](http://link.com)".to_owned())
            .collect();
        assert_eq!(lines.len(), 1);

        let (rendered, tags) = render_line(lines[0].clone(), 80);

        // Check rendered content includes link decorations
        let content = line_to_string(&rendered);
        assert!(content.contains("text"));
        assert!(content.contains("http://link.com"));

        // Check Tag::Link is produced with correct span index
        // Spans: ▐(0), text(1), ▌(2), ◖(3), http://link.com(4), ◗(5)
        assert_eq!(tags.len(), 1);
        assert!(matches!(&tags[0], Tag::Link(4, url) if url == "http://link.com"));
    }

    #[test]
    fn render_list() {
        let input = r#"1. First ordered list item
2. Another item
   - Unordered sub-list.
3. Actual numbers don't matter, just that it's a number
   1. Ordered sub-list
4. And another item."#;

        let output = parse_and_render(input, 500);
        assert_eq!(output, input);
    }

    #[test]
    fn render_list_checkboxes() {
        let input = r#"- [x] Checked item
- [ ] Unchecked item"#;

        let expected = r#"- [✓] Checked item
- [ ] Unchecked item"#;

        let output = parse_and_render(input, 500);
        assert_eq!(output, expected);
    }

    #[test]
    fn render_nested_blockquotes() {
        let input = r#"> This is a blockquote.
> Continuation of blockquote.
> > Nested blockquote
> > Continuation of nested blockquote."#;

        let expected = r#"▌ This is a blockquote.
▌ Continuation of blockquote.
▌ ▌ Nested blockquote
▌ ▌ Continuation of nested blockquote."#;

        let output = parse_and_render(input, 500);
        assert_eq!(output, expected);
    }

    #[test]
    fn render_blockquote_with_blank_lines() {
        let input = r#"> First paragraph
>
> Second paragraph"#;

        let expected = "▌ First paragraph\n▌ \n▌ Second paragraph";

        let output = parse_and_render(input, 500);
        assert_eq!(output, expected);
    }

    #[test]
    fn render_table() {
        let input = r#"| Header 1 | Header 2 |
|----------|----------|
| Cell *1* | Cell 2   |
| Cell 3   | Cell 4   |"#;

        let output = parse_and_render(input, 80);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn render_table_with_alignment() {
        let input = r#"| Left | Center | Right |
|:-----|:------:|------:|
| L    |   C    |     R |"#;

        let output = parse_and_render(input, 80);
        insta::assert_snapshot!(output);
    }
}
