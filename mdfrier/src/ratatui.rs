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
    lines::{BorderPosition, LineKind, ListMarker, MdLine, MdLineContainer, TableColumnInfo},
    markdown::{MdModifier, MdNode, TableAlignment},
};

// ============================================================================
// Theme trait - provides all styling options
// ============================================================================

/// Theme trait for customizing markdown rendering styles.
///
/// All methods have default implementations providing a sensible default theme.
/// Implement only the methods you want to customize.
pub trait Theme {
    // ========================================================================
    // Symbols
    // ========================================================================

    /// Blockquote bar character (default: "▌").
    fn blockquote_bar(&self) -> &str {
        "▌"
    }

    /// Link description opening bracket replacement (default: "▐").
    fn link_desc_open(&self) -> &str {
        "▐"
    }

    /// Link description closing bracket replacement (default: "▌").
    fn link_desc_close(&self) -> &str {
        "▌"
    }

    /// Link URL opening paren replacement (default: "◖").
    fn link_url_open(&self) -> &str {
        "◖"
    }

    /// Link URL closing paren replacement (default: "◗").
    fn link_url_close(&self) -> &str {
        "◗"
    }

    /// Horizontal rule character (default: "─").
    fn horizontal_rule_char(&self) -> &str {
        "─"
    }

    /// Task checkbox checked mark (default: "✓").
    fn task_checked_mark(&self) -> &str {
        "✓"
    }

    // ========================================================================
    // Table border characters
    // ========================================================================

    /// Table vertical border (default: "│").
    fn table_vertical(&self) -> &str {
        "│"
    }

    /// Table horizontal border (default: "─").
    fn table_horizontal(&self) -> &str {
        "─"
    }

    /// Table top-left corner (default: "┌").
    fn table_top_left(&self) -> &str {
        "┌"
    }

    /// Table top-right corner (default: "┐").
    fn table_top_right(&self) -> &str {
        "┐"
    }

    /// Table bottom-left corner (default: "└").
    fn table_bottom_left(&self) -> &str {
        "└"
    }

    /// Table bottom-right corner (default: "┘").
    fn table_bottom_right(&self) -> &str {
        "┘"
    }

    /// Table top junction (default: "┬").
    fn table_top_junction(&self) -> &str {
        "┬"
    }

    /// Table bottom junction (default: "┴").
    fn table_bottom_junction(&self) -> &str {
        "┴"
    }

    /// Table left junction (default: "├").
    fn table_left_junction(&self) -> &str {
        "├"
    }

    /// Table right junction (default: "┤").
    fn table_right_junction(&self) -> &str {
        "┤"
    }

    /// Table cross junction (default: "┼").
    fn table_cross(&self) -> &str {
        "┼"
    }

    // ========================================================================
    // Colors
    // ========================================================================

    /// Blockquote color for given nesting depth (0-indexed, cycles through palette).
    fn blockquote_color(&self, depth: usize) -> Color {
        const COLORS: [Color; 6] = [
            Color::Indexed(202),
            Color::Indexed(203),
            Color::Indexed(204),
            Color::Indexed(205),
            Color::Indexed(206),
            Color::Indexed(207),
        ];
        COLORS[depth % COLORS.len()]
    }

    /// Link background color (default: dark gray).
    fn link_bg(&self) -> Color {
        Color::Indexed(237)
    }

    /// Link foreground color (default: blue).
    fn link_fg(&self) -> Color {
        Color::Indexed(4)
    }

    /// Prefix color for list markers, etc. (default: light blue/gray).
    fn prefix_color(&self) -> Color {
        Color::Indexed(189)
    }

    /// Emphasis/italic color (default: yellow/gold).
    fn emphasis_color(&self) -> Color {
        Color::Indexed(220)
    }

    /// Code background color (default: dark gray).
    fn code_bg(&self) -> Color {
        Color::Indexed(236)
    }

    /// Code foreground color (default: salmon/coral).
    fn code_fg(&self) -> Color {
        Color::Indexed(203)
    }

    /// Horizontal rule color (default: gray).
    fn hr_color(&self) -> Color {
        Color::Indexed(240)
    }

    /// Table border color (default: gray).
    fn table_border_color(&self) -> Color {
        Color::Indexed(240)
    }

    /// Table header text color (default: bright white).
    fn table_header_color(&self) -> Color {
        Color::Indexed(255)
    }

    // ========================================================================
    // Composite styles
    // ========================================================================

    /// Style for inline code spans.
    fn code_style(&self) -> Style {
        Style::default().fg(self.code_fg()).bg(self.code_bg())
    }

    /// Style for emphasized (italic) text.
    fn emphasis_style(&self) -> Style {
        Style::default()
            .add_modifier(Modifier::ITALIC)
            .fg(self.emphasis_color())
    }

    /// Style for strong emphasis (bold) text.
    fn strong_emphasis_style(&self) -> Style {
        Style::default()
            .add_modifier(Modifier::BOLD)
            .fg(self.emphasis_color())
    }

    /// Style for link URL text.
    fn link_url_style(&self) -> Style {
        Style::default()
            .fg(self.link_fg())
            .bg(self.link_bg())
            .underlined()
    }

    /// Style for link description text.
    fn link_description_style(&self) -> Style {
        Style::default().fg(self.link_fg()).bg(self.link_bg())
    }

    /// Style for link bracket wrappers.
    fn link_wrapper_style(&self) -> Style {
        Style::default().fg(self.link_bg())
    }

    /// Style for horizontal rules.
    fn hr_style(&self) -> Style {
        Style::default().fg(self.hr_color())
    }

    /// Style for table borders.
    fn table_border_style(&self) -> Style {
        Style::default().fg(self.table_border_color())
    }

    /// Style for table header cells.
    fn table_header_style(&self) -> Style {
        Style::default()
            .add_modifier(Modifier::BOLD)
            .fg(self.table_header_color())
    }

    /// Style for list markers and prefixes.
    fn prefix_style(&self) -> Style {
        Style::default().fg(self.prefix_color())
    }

    /// Style for blockquote bar at given depth.
    fn blockquote_style(&self, depth: usize) -> Style {
        Style::default().fg(self.blockquote_color(depth))
    }
}

/// The default theme with all standard styling.
#[derive(Debug, Clone, Copy, Default)]
pub struct DefaultTheme;

impl Theme for DefaultTheme {}

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

/// Convert an `MdLine` to a styled ratatui `Line` and semantic `Tag`s using a custom theme.
///
/// # Arguments
///
/// * `md_line` - The markdown line to convert
/// * `width` - Terminal width (used for horizontal rules)
/// * `theme` - The theme to use for styling
///
/// # Returns
///
/// A tuple of (styled Line, list of Tags)
pub fn render_line<T: Theme>(md_line: MdLine, width: u16, theme: &T) -> (Line<'static>, Vec<Tag>) {
    let MdLine { spans, meta } = md_line;

    match meta.kind {
        LineKind::Paragraph => render_paragraph(spans, &meta.nesting, theme),
        LineKind::Header(tier) => render_header(spans, tier, meta.blockquote_depth(), theme),
        LineKind::CodeBlock { ref language } => {
            render_code_line(spans, language, meta.blockquote_depth(), width, theme)
        }
        LineKind::HorizontalRule => render_horizontal_rule(meta.blockquote_depth(), width, theme),
        LineKind::TableRow {
            ref cells,
            ref column_info,
            is_header,
        } => render_table_row(
            cells,
            column_info,
            is_header,
            meta.blockquote_depth(),
            theme,
        ),
        LineKind::TableBorder {
            ref column_info,
            position,
        } => render_table_border(column_info, position, meta.blockquote_depth(), theme),
        LineKind::Image { url, description } => {
            // Images are handled specially by the application
            let line = Line::default();
            let tags = vec![Tag::Image(url, description)];
            (line, tags)
        }
        LineKind::Blank => render_blank(meta.blockquote_depth(), theme),
    }
}

// ============================================================================
// Render functions for each line kind
// ============================================================================

fn render_paragraph<T: Theme>(
    spans: Vec<MdNode>,
    nesting: &[MdLineContainer],
    theme: &T,
) -> (Line<'static>, Vec<Tag>) {
    let mut line_spans = Vec::new();
    let mut tags = Vec::new();

    // Add styled prefix from nesting containers
    line_spans.extend(render_nesting_prefix(nesting, theme));

    for mdspan in spans {
        // Track links for tags - store span index (URL span position in line_spans)
        if mdspan.extra.contains(MdModifier::LinkURL)
            && !mdspan.extra.contains(MdModifier::LinkURLWrapper)
        {
            tags.push(Tag::Link(line_spans.len(), mdspan.content.clone()));
        }
        line_spans.push(span_from_mdnode(mdspan, theme));
    }

    (Line::from(line_spans), tags)
}

fn render_header<T: Theme>(
    spans: Vec<MdNode>,
    tier: u8,
    blockquote_depth: usize,
    theme: &T,
) -> (Line<'static>, Vec<Tag>) {
    let mut line_spans = Vec::new();

    // Add blockquote prefix if inside blockquote
    if blockquote_depth > 0 {
        line_spans.extend(build_blockquote_prefix(blockquote_depth, theme));
    }

    // Headers are rendered as plain text (application handles big text rendering)
    for span in spans {
        line_spans.push(Span::from(span.content));
    }

    let tags = vec![Tag::Header(tier)];
    (Line::from(line_spans), tags)
}

fn render_code_line<T: Theme>(
    spans: Vec<MdNode>,
    language: &str,
    blockquote_depth: usize,
    width: u16,
    theme: &T,
) -> (Line<'static>, Vec<Tag>) {
    let mut line_spans = Vec::new();

    let prefix_width = blockquote_depth * 2;
    let available_width = (width as usize).saturating_sub(prefix_width);

    // Add blockquote prefix
    if blockquote_depth > 0 {
        line_spans.extend(build_blockquote_prefix(blockquote_depth, theme));
    }

    let code_style = theme.code_style();

    let content = if spans.is_empty() {
        String::new()
    } else if spans.len() == 1 {
        spans.into_iter().next().unwrap().content
    } else {
        let total_len: usize = spans.iter().map(|s| s.content.len()).sum();
        let mut result = String::with_capacity(total_len);
        for span in spans {
            result.push_str(&span.content);
        }
        result
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

fn render_horizontal_rule<T: Theme>(
    blockquote_depth: usize,
    width: u16,
    theme: &T,
) -> (Line<'static>, Vec<Tag>) {
    let mut line_spans = Vec::new();

    let prefix_width = blockquote_depth * 2;
    let available_width = (width as usize).saturating_sub(prefix_width);

    // Add blockquote prefix
    if blockquote_depth > 0 {
        line_spans.extend(build_blockquote_prefix(blockquote_depth, theme));
    }

    let rule_char = theme.horizontal_rule_char();
    line_spans.push(Span::styled(
        rule_char.repeat(available_width),
        theme.hr_style(),
    ));

    let tags = vec![Tag::HorizontalRule];
    (Line::from(line_spans), tags)
}

fn render_blank<T: Theme>(blockquote_depth: usize, theme: &T) -> (Line<'static>, Vec<Tag>) {
    let mut line_spans = Vec::new();

    if blockquote_depth > 0 {
        line_spans.extend(build_blockquote_prefix(blockquote_depth, theme));
    }

    let tags = vec![Tag::Blank];
    (Line::from(line_spans), tags)
}

fn render_table_row<T: Theme>(
    cells: &[Vec<MdNode>],
    column_info: &TableColumnInfo,
    is_header: bool,
    blockquote_depth: usize,
    theme: &T,
) -> (Line<'static>, Vec<Tag>) {
    let mut line_spans = Vec::new();

    // Add blockquote prefix
    if blockquote_depth > 0 {
        line_spans.extend(build_blockquote_prefix(blockquote_depth, theme));
    }

    let border_style = theme.table_border_style();
    let header_style = theme.table_header_style();
    let vertical_border: String = theme.table_vertical().into();

    line_spans.push(Span::styled(vertical_border.clone(), border_style));

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
        let mut left_padding = String::with_capacity(1 + left_pad);
        left_padding.push(' ');
        for _ in 0..left_pad {
            left_padding.push(' ');
        }
        line_spans.push(Span::from(left_padding));

        // Cell content
        for node in cell_spans {
            let mut style = if is_header {
                header_style
            } else {
                Style::default()
            };

            if node.extra.contains(MdModifier::Emphasis) {
                style = style.patch(theme.emphasis_style());
            }
            if node.extra.contains(MdModifier::StrongEmphasis) {
                style = style.patch(theme.strong_emphasis_style());
            }
            if node.extra.contains(MdModifier::Code) {
                style = style.patch(theme.code_style());
            }

            line_spans.push(Span::styled(node.content.clone(), style));
        }

        // Right padding
        let mut right_padding = String::with_capacity(right_pad + 1);
        for _ in 0..right_pad {
            right_padding.push(' ');
        }
        right_padding.push(' ');
        line_spans.push(Span::from(right_padding));
        line_spans.push(Span::styled(vertical_border.clone(), border_style));
    }

    // Fill missing columns
    let num_cols = column_info.widths.len();
    for i in cells.len()..num_cols {
        let col_width = column_info.widths.get(i).copied().unwrap_or(3);
        line_spans.push(Span::from(" ".repeat(col_width)));
        line_spans.push(Span::styled(vertical_border.clone(), border_style));
    }

    let tags = vec![Tag::TableRow];
    (Line::from(line_spans), tags)
}

fn render_table_border<T: Theme>(
    column_info: &TableColumnInfo,
    position: BorderPosition,
    blockquote_depth: usize,
    theme: &T,
) -> (Line<'static>, Vec<Tag>) {
    let mut line_spans = Vec::new();

    // Add blockquote prefix
    if blockquote_depth > 0 {
        line_spans.extend(build_blockquote_prefix(blockquote_depth, theme));
    }

    let border_style = theme.table_border_style();
    let num_cols = column_info.widths.len();

    let (left, mid, right) = match position {
        BorderPosition::Top => (
            theme.table_top_left(),
            theme.table_top_junction(),
            theme.table_top_right(),
        ),
        BorderPosition::HeaderSeparator => (
            theme.table_left_junction(),
            theme.table_cross(),
            theme.table_right_junction(),
        ),
        BorderPosition::Bottom => (
            theme.table_bottom_left(),
            theme.table_bottom_junction(),
            theme.table_bottom_right(),
        ),
    };

    let horizontal = theme.table_horizontal();
    let mid_owned: String = mid.into();

    line_spans.push(Span::styled(String::from(left), border_style));
    for (i, &col_w) in column_info.widths.iter().enumerate() {
        line_spans.push(Span::styled(horizontal.repeat(col_w), border_style));
        if i < num_cols - 1 {
            line_spans.push(Span::styled(mid_owned.clone(), border_style));
        }
    }
    line_spans.push(Span::styled(String::from(right), border_style));

    let tags = vec![Tag::TableRow];
    (Line::from(line_spans), tags)
}

// ============================================================================
// Helper functions
// ============================================================================

/// Build blockquote prefix spans.
fn build_blockquote_prefix<T: Theme>(depth: usize, theme: &T) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for d in 0..depth {
        spans.push(Span::styled(
            theme.blockquote_bar().to_owned(),
            theme.blockquote_style(d),
        ));
        spans.push(Span::from(" "));
    }
    spans
}

/// Render prefix spans from nesting containers.
fn render_nesting_prefix<T: Theme>(nesting: &[MdLineContainer], theme: &T) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut blockquote_idx = 0;

    // Find the last (innermost) list item - only render that one with a marker
    let last_list_item_idx = nesting
        .iter()
        .rposition(|c| matches!(c, MdLineContainer::ListItem { .. }));

    for (i, container) in nesting.iter().enumerate() {
        match container {
            MdLineContainer::Blockquote => {
                spans.push(Span::styled(
                    theme.blockquote_bar().to_owned(),
                    theme.blockquote_style(blockquote_idx),
                ));
                spans.push(Span::from(" "));
                blockquote_idx += 1;
            }
            MdLineContainer::ListItem {
                marker,
                continuation,
            } => {
                // Render marker only for innermost non-continuation list item
                if Some(i) == last_list_item_idx && !continuation {
                    let marker_text = match marker {
                        ListMarker::Unordered(bullet) => format!("{} ", bullet.char()),
                        ListMarker::Ordered(n) => format!("{}. ", n),
                        ListMarker::TaskUnchecked(bullet) => format!("{} [ ] ", bullet.char()),
                        ListMarker::TaskChecked(bullet) => {
                            format!("{} [{}] ", bullet.char(), theme.task_checked_mark())
                        }
                    };
                    spans.push(Span::styled(marker_text, theme.prefix_style()));
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
fn span_from_mdnode<T: Theme>(mdnode: MdNode, theme: &T) -> Span<'static> {
    let mut style = Style::default();

    if mdnode.extra.contains(MdModifier::Emphasis) {
        style = style.patch(theme.emphasis_style());
    }
    if mdnode.extra.contains(MdModifier::StrongEmphasis) {
        style = style.patch(theme.strong_emphasis_style());
    }
    if mdnode.extra.contains(MdModifier::Code) {
        style = style.patch(theme.code_style());
    }

    if mdnode.extra.contains(MdModifier::LinkURLWrapper) {
        let bracket = if mdnode.content == "(" {
            theme.link_url_open()
        } else {
            theme.link_url_close()
        };
        return Span::styled(bracket.to_owned(), style.patch(theme.link_wrapper_style()));
    }
    if mdnode.extra.contains(MdModifier::LinkURL) {
        style = style.patch(theme.link_url_style());
    }

    if mdnode.extra.contains(MdModifier::LinkDescriptionWrapper) {
        let bracket = if mdnode.content == "[" {
            theme.link_desc_open()
        } else {
            theme.link_desc_close()
        };
        return Span::styled(bracket.to_owned(), style.patch(theme.link_wrapper_style()));
    }
    if mdnode.extra.contains(MdModifier::LinkDescription) {
        style = style.patch(theme.link_description_style());
    }

    Span::styled(mdnode.content, style)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MdFrier, lines::LineMeta};
    use pretty_assertions::assert_eq;

    /// Parse and render markdown, returning string output.
    fn parse_and_render(input: &str, width: u16) -> String {
        let mut frier = MdFrier::new().unwrap();
        let lines: Vec<_> = frier.parse(width, input.to_owned()).collect();
        lines
            .into_iter()
            .map(|md_line| {
                let (rendered, _) = render_line(md_line, width, &DefaultTheme::default());
                rendered.to_string()
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

        let (rendered, tags) = render_line(line, 80, &DefaultTheme::default());
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

        let (rendered, _tags) = render_line(line, 80, &DefaultTheme::default());
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
                nesting: vec![MdLineContainer::Blockquote],
            },
        };

        let (rendered, _tags) = render_line(line, 80, &DefaultTheme::default());
        // Should have blockquote bar + space + text
        assert!(rendered.spans.len() >= 2);
        assert_eq!(rendered.spans[0].content, DefaultTheme.blockquote_bar());
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

        let (_rendered, tags) = render_line(line, 80, &DefaultTheme::default());
        assert_eq!(tags.len(), 1);
        assert!(matches!(tags[0], Tag::Header(1)));
    }

    #[test]
    fn integration_parse_and_render() {
        let mut frier = MdFrier::new().unwrap();
        let lines: Vec<_> = frier.parse(80, "Hello *world*!".to_owned()).collect();

        let (rendered, _tags) = render_line(lines[0].clone(), 80, &DefaultTheme::default());
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

        let (rendered, tags) = render_line(lines[0].clone(), 80, &DefaultTheme::default());

        // Check rendered content includes link decorations
        let content = rendered.to_string();
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

    #[test]
    fn render_bare_url() {
        let mut frier = MdFrier::new().unwrap();
        let lines: Vec<_> = frier
            .parse(80, "Check https://example.com for info.".to_owned())
            .collect();
        assert_eq!(lines.len(), 1);

        let (line, tags) = render_line(lines[0].to_owned(), 80, &DefaultTheme::default());

        // Check rendered content includes the URL
        let content = line.to_string();
        assert_eq!(content, "Check ◖https://example.com◗ for info.");

        assert_eq!(tags, vec![Tag::Link(2, "https://example.com".to_owned())]);

        let url_span = &line.spans[2];
        assert_eq!(url_span.content, "https://example.com");
        assert!(url_span.style.add_modifier.contains(Modifier::UNDERLINED));
    }

    #[test]
    #[ignore]
    fn tag_linebroken_url() {
        let mut frier = MdFrier::new().unwrap();
        let lines: Vec<_> = frier
            .parse(15, "Check https://example.com for info.".to_owned())
            .collect();
        assert_eq!(
            lines
                .iter()
                .map(MdLine::to_string)
                .collect::<Vec<String>>()
                .join("\n"),
            r#"Check 
https://
example.com
for info."#
        );

        for i in 1..=2 {
            let (line, tags) = render_line(lines[i].to_owned(), 10, &DefaultTheme::default());
            assert_eq!(tags, vec![Tag::Link(1, "https://example.com".to_owned())]);
            let url_span = &line.spans[0];
            assert!(url_span.style.add_modifier.contains(Modifier::UNDERLINED));
        }
    }
}
