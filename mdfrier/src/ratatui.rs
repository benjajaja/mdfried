//! Ratatui integration for mdfrier.
//!
//! This module provides conversion from [`MdLine`] to styled ratatui `Line` widgets.

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span as RatatuiSpan},
};

use crate::{Line as MdLine, LineKind, Span, mapper::Mapper, markdown::Modifier as MdModifier};

// ============================================================================
// Theme trait - extends Mapper with styling
// ============================================================================

/// Theme trait for customizing markdown rendering styles.
///
/// Extends `Mapper` to inherit symbol definitions, then adds color/style methods.
/// All methods have default implementations providing a sensible default theme.
pub trait Theme: Mapper {
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
        Color::Indexed(222)
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
        Style::default()
            .fg(self.link_fg())
            .bg(self.link_bg())
            .underlined()
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

    /// Strikethrough text color (default: gray).
    fn strikethrough_color(&self) -> Color {
        Color::Indexed(245)
    }

    /// Style for strikethrough text.
    fn strikethrough_style(&self) -> Style {
        Style::default()
            .add_modifier(Modifier::CROSSED_OUT | Modifier::DIM)
            .fg(self.strikethrough_color())
    }
}

/// The default theme with all standard styling.
///
/// Delegates to [`crate::mapper::StyledMapper`] for symbols and removes text decorators
/// since styling (bold, italic, colors) replaces the textual markers.
#[derive(Debug, Clone, Copy, Default)]
pub struct DefaultTheme;

const STYLED: crate::mapper::StyledMapper = crate::mapper::StyledMapper;

impl Mapper for DefaultTheme {
    fn link_desc_open(&self) -> &str {
        STYLED.link_desc_open()
    }
    fn link_desc_close(&self) -> &str {
        STYLED.link_desc_close()
    }
    fn link_url_open(&self) -> &str {
        STYLED.link_url_open()
    }
    fn link_url_close(&self) -> &str {
        STYLED.link_url_close()
    }
    fn blockquote_bar(&self) -> &str {
        STYLED.blockquote_bar()
    }
    fn horizontal_rule_char(&self) -> &str {
        STYLED.horizontal_rule_char()
    }
    fn task_checked(&self) -> &str {
        STYLED.task_checked()
    }
    fn table_vertical(&self) -> &str {
        STYLED.table_vertical()
    }
    fn table_horizontal(&self) -> &str {
        STYLED.table_horizontal()
    }
    fn table_top_left(&self) -> &str {
        STYLED.table_top_left()
    }
    fn table_top_right(&self) -> &str {
        STYLED.table_top_right()
    }
    fn table_bottom_left(&self) -> &str {
        STYLED.table_bottom_left()
    }
    fn table_bottom_right(&self) -> &str {
        STYLED.table_bottom_right()
    }
    fn table_top_junction(&self) -> &str {
        STYLED.table_top_junction()
    }
    fn table_bottom_junction(&self) -> &str {
        STYLED.table_bottom_junction()
    }
    fn table_left_junction(&self) -> &str {
        STYLED.table_left_junction()
    }
    fn table_right_junction(&self) -> &str {
        STYLED.table_right_junction()
    }
    fn table_cross(&self) -> &str {
        STYLED.table_cross()
    }
    fn emphasis_open(&self) -> &str {
        STYLED.emphasis_open()
    }
    fn emphasis_close(&self) -> &str {
        STYLED.emphasis_close()
    }
    fn strong_open(&self) -> &str {
        STYLED.strong_open()
    }
    fn strong_close(&self) -> &str {
        STYLED.strong_close()
    }
    fn code_open(&self) -> &str {
        STYLED.code_open()
    }
    fn code_close(&self) -> &str {
        STYLED.code_close()
    }
    fn strikethrough_open(&self) -> &str {
        STYLED.strikethrough_open()
    }
    fn strikethrough_close(&self) -> &str {
        STYLED.strikethrough_close()
    }
}

impl Theme for DefaultTheme {}

// ============================================================================
// Main conversion function
// ============================================================================

/// Convert an `MdLine` to a styled ratatui `Line` and semantic `Tag`s.
///
/// With the new flat API, all content (including prefixes and decorators)
/// is already in `md_line.spans`. This function just applies styling based
/// on `MdModifier` flags.
///
/// # Arguments
///
/// * `md_line` - The markdown line to convert
/// * `theme` - The theme to use for styling
///
/// # Returns
///
/// A tuple of (styled Line, list of Tags)
pub fn render_line<T: Theme, F: FnMut(&Span)>(
    md_line: MdLine,
    theme: &T,
    mut node_cb: Option<F>,
) -> Line<'static> {
    let MdLine { spans, kind } = md_line;

    // Track blockquote depth by counting BlockquoteBar spans
    let mut bq_depth = 0;

    // Check if this is a header row for table styling
    let is_table_header = matches!(kind, LineKind::TableRow { is_header: true });
    // Check if this is a code block line
    let is_code_block = matches!(kind, LineKind::CodeBlock { .. });

    let line_spans: Vec<RatatuiSpan<'static>> = spans
        .into_iter()
        .map(|node| {
            if let Some(ref mut cb) = node_cb {
                cb(&node);
            }

            node_to_span(
                node,
                bq_depth,
                is_table_header,
                is_code_block,
                theme,
                &mut bq_depth,
            )
        })
        .collect();

    Line::from(line_spans)
}

/// Convert a Span to a styled ratatui Span.
fn node_to_span<T: Theme>(
    node: Span,
    current_bq_depth: usize,
    is_table_header: bool,
    is_code_block: bool,
    theme: &T,
    bq_depth_out: &mut usize,
) -> RatatuiSpan<'static> {
    let Span {
        content,
        modifiers,
        // TODO: make OSC links work when https://github.com/ratatui/ratatui/pull/1605
    } = node;

    if theme.hide_urls() && modifiers.is_link_url() {
        // TODO: this seems super hacky, can we not do this earlier?
        return RatatuiSpan::default();
    }

    // Handle special modifier-based styling
    if modifiers.contains(MdModifier::BlockquoteBar) {
        let style = theme.blockquote_style(current_bq_depth);
        *bq_depth_out = current_bq_depth + 1;
        return RatatuiSpan::styled(content, style);
    }

    if modifiers.contains(MdModifier::ListMarker) {
        return RatatuiSpan::styled(content, theme.prefix_style());
    }

    if modifiers.contains(MdModifier::TableBorder) {
        return RatatuiSpan::styled(content, theme.table_border_style());
    }

    if modifiers.contains(MdModifier::HorizontalRule) {
        return RatatuiSpan::styled(content, theme.hr_style());
    }

    // Handle inline code and code blocks
    if modifiers.contains(MdModifier::Code) || is_code_block {
        return RatatuiSpan::styled(content, theme.code_style());
    }

    // Handle link wrappers
    if modifiers.contains(MdModifier::LinkDescriptionWrapper)
        || modifiers.contains(MdModifier::LinkURLWrapper)
    {
        return RatatuiSpan::styled(content, theme.link_wrapper_style());
    }

    // Handle link URL
    if modifiers.contains(MdModifier::LinkURL) {
        return RatatuiSpan::styled(content, theme.link_url_style());
    }

    // Build style from modifiers
    let mut style = if is_table_header {
        theme.table_header_style()
    } else {
        Style::default()
    };

    if modifiers.contains(MdModifier::LinkDescription) {
        style = style.patch(theme.link_description_style());
    }

    if modifiers.contains(MdModifier::Emphasis) {
        style = style.patch(theme.emphasis_style());
    }
    if modifiers.contains(MdModifier::StrongEmphasis) {
        style = style.patch(theme.strong_emphasis_style());
    }
    if modifiers.contains(MdModifier::Strikethrough) {
        style = style.patch(theme.strikethrough_style());
    }
    if modifiers.contains(MdModifier::Link) {
        style = style.patch(theme.link_description_style());
    }

    if style == Style::default() {
        RatatuiSpan::from(content)
    } else {
        RatatuiSpan::styled(content, style)
    }
}
