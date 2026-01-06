//! Ratatui integration for mdfrier.
//!
//! This module provides conversion from `MdLine` to styled ratatui `Line` widgets,
//! along with semantic `Tag`s that can be used by the application.

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

use crate::{LineKind, MdLine, MdNode, mapper::Mapper, markdown::MdModifier};

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
/// Delegates to [`StyledMapper`] for symbols and removes text decorators
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
    /// Table border.
    TableBorder,
    /// Blank line.
    Blank,
}

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
pub fn render_line<T: Theme>(md_line: MdLine, theme: &T) -> (Line<'static>, Vec<Tag>) {
    let MdLine { spans, kind } = md_line;

    // Track blockquote depth by counting BlockquoteBar spans
    let mut bq_depth = 0;
    let mut line_spans = Vec::with_capacity(spans.len());
    let mut tags = Vec::new();

    // Check if this is a header row for table styling
    let is_table_header = matches!(kind, LineKind::TableRow { is_header: true });
    // Check if this is a code block line
    let is_code_block = matches!(kind, LineKind::CodeBlock { .. });

    for node in spans {
        // Track links for tags
        if node.modifiers.contains(MdModifier::LinkURL)
            && !node.modifiers.contains(MdModifier::LinkURLWrapper)
        {
            tags.push(Tag::Link(line_spans.len(), node.content.clone()));
        }

        let span = node_to_span(
            node,
            bq_depth,
            is_table_header,
            is_code_block,
            theme,
            &mut bq_depth,
        );
        line_spans.push(span);
    }

    // Add kind-based tags
    match kind {
        LineKind::Paragraph => {}
        LineKind::Header(tier) => tags.push(Tag::Header(tier)),
        LineKind::CodeBlock { ref language } => {
            tags.push(Tag::CodeBlock(if language.is_empty() {
                None
            } else {
                Some(language.clone())
            }));
        }
        LineKind::HorizontalRule => tags.push(Tag::HorizontalRule),
        LineKind::TableRow { .. } => tags.push(Tag::TableRow),
        LineKind::TableBorder => tags.push(Tag::TableBorder),
        LineKind::Image { url, description } => tags.push(Tag::Image(url, description)),
        LineKind::Blank => tags.push(Tag::Blank),
    }

    (Line::from(line_spans), tags)
}

/// Convert an MdNode to a styled Span.
fn node_to_span<T: Theme>(
    node: MdNode,
    current_bq_depth: usize,
    is_table_header: bool,
    is_code_block: bool,
    theme: &T,
    bq_depth_out: &mut usize,
) -> Span<'static> {
    let MdNode {
        content,
        modifiers: extra,
    } = node;

    // Handle special modifier-based styling
    if extra.contains(MdModifier::BlockquoteBar) {
        let style = theme.blockquote_style(current_bq_depth);
        *bq_depth_out = current_bq_depth + 1;
        return Span::styled(content, style);
    }

    if extra.contains(MdModifier::ListMarker) {
        return Span::styled(content, theme.prefix_style());
    }

    if extra.contains(MdModifier::TableBorder) {
        return Span::styled(content, theme.table_border_style());
    }

    if extra.contains(MdModifier::HorizontalRule) {
        return Span::styled(content, theme.hr_style());
    }

    // Handle inline code and code blocks
    if extra.contains(MdModifier::Code) || is_code_block {
        return Span::styled(content, theme.code_style());
    }

    // Handle link wrappers
    if extra.contains(MdModifier::LinkDescriptionWrapper)
        || extra.contains(MdModifier::LinkURLWrapper)
    {
        return Span::styled(content, theme.link_wrapper_style());
    }

    // Handle link URL
    if extra.contains(MdModifier::LinkURL) {
        return Span::styled(content, theme.link_url_style());
    }

    // Handle link description
    if extra.contains(MdModifier::LinkDescription) {
        return Span::styled(content, theme.link_description_style());
    }

    // Build style from modifiers
    let mut style = if is_table_header {
        theme.table_header_style()
    } else {
        Style::default()
    };

    if extra.contains(MdModifier::Emphasis) {
        style = style.patch(theme.emphasis_style());
    }
    if extra.contains(MdModifier::StrongEmphasis) {
        style = style.patch(theme.strong_emphasis_style());
    }
    if extra.contains(MdModifier::Strikethrough) {
        style = style.patch(theme.strikethrough_style());
    }
    if extra.contains(MdModifier::Link) {
        style = style.patch(theme.link_description_style());
    }

    if style == Style::default() {
        Span::from(content)
    } else {
        Span::styled(content, style)
    }
}
