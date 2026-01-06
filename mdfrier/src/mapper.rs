//! Content mapping for markdown rendering.
//!
//! The `Mapper` trait defines how markdown elements are decorated/transformed
//! before wrapping. This affects the visual width of elements and must be applied
//! before line wrapping occurs.

use crate::lines::BulletStyle;

/// Trait for mapping markdown content to decorated output.
///
/// The Mapper controls the symbols/decorators used for various markdown elements.
/// This is applied before wrapping, so the decorator widths are accounted for
/// in line-width calculations.
///
/// For ratatui users, the [`crate::ratatui::Theme`] trait in the [`ratatui`] module extends this
/// with color/style information.
pub trait Mapper {
    // ========================================================================
    // Link decorators
    // ========================================================================

    /// Opening bracket for link description (default: "[").
    fn link_desc_open(&self) -> &str {
        "["
    }

    /// Closing bracket for link description (default: "]").
    fn link_desc_close(&self) -> &str {
        "]"
    }

    /// Opening paren for link URL (default: "(").
    fn link_url_open(&self) -> &str {
        "("
    }

    /// Closing paren for link URL (default: ")").
    fn link_url_close(&self) -> &str {
        ")"
    }

    // ========================================================================
    // Blockquote
    // ========================================================================

    /// Blockquote bar with trailing space (default: "> ").
    fn blockquote_bar(&self) -> &str {
        "> "
    }

    // ========================================================================
    // List markers
    // ========================================================================

    /// Unordered list bullet with trailing space.
    fn unordered_bullet(&self, style: BulletStyle) -> &str {
        match style {
            BulletStyle::Dash => "- ",
            BulletStyle::Star => "* ",
            BulletStyle::Plus => "+ ",
        }
    }

    /// Ordered list marker (e.g., "1. ", "2. ").
    fn ordered_marker(&self, num: u32) -> String {
        format!("{}. ", num)
    }

    /// Task list checked marker (default: "\[x\] ").
    fn task_checked(&self) -> &str {
        "[x] "
    }

    /// Task list unchecked marker (default: "[ ] ").
    fn task_unchecked(&self) -> &str {
        "[ ] "
    }

    // ========================================================================
    // Table borders
    // ========================================================================

    /// Vertical border character (default: "|").
    fn table_vertical(&self) -> &str {
        "|"
    }

    /// Horizontal border character (default: "-").
    fn table_horizontal(&self) -> &str {
        "-"
    }

    /// Top-left corner (default: "+").
    fn table_top_left(&self) -> &str {
        "+"
    }

    /// Top-right corner (default: "+").
    fn table_top_right(&self) -> &str {
        "+"
    }

    /// Bottom-left corner (default: "+").
    fn table_bottom_left(&self) -> &str {
        "+"
    }

    /// Bottom-right corner (default: "+").
    fn table_bottom_right(&self) -> &str {
        "+"
    }

    /// Top junction (default: "+").
    fn table_top_junction(&self) -> &str {
        "+"
    }

    /// Bottom junction (default: "+").
    fn table_bottom_junction(&self) -> &str {
        "+"
    }

    /// Left junction (default: "+").
    fn table_left_junction(&self) -> &str {
        "+"
    }

    /// Right junction (default: "+").
    fn table_right_junction(&self) -> &str {
        "+"
    }

    /// Cross junction (default: "+").
    fn table_cross(&self) -> &str {
        "+"
    }

    // ========================================================================
    // Horizontal rule
    // ========================================================================

    /// Horizontal rule character (default: "-").
    fn horizontal_rule_char(&self) -> &str {
        "-"
    }

    // ========================================================================
    // Emphasis decorators
    // ========================================================================

    /// Opening decorator for emphasis/italic text (default: "*").
    fn emphasis_open(&self) -> &str {
        "*"
    }

    /// Closing decorator for emphasis/italic text (default: "*").
    fn emphasis_close(&self) -> &str {
        "*"
    }

    /// Opening decorator for strong/bold text (default: "**").
    fn strong_open(&self) -> &str {
        "**"
    }

    /// Closing decorator for strong/bold text (default: "**").
    fn strong_close(&self) -> &str {
        "**"
    }

    // ========================================================================
    // Code decorators
    // ========================================================================

    /// Opening decorator for inline code (default: "`").
    fn code_open(&self) -> &str {
        "`"
    }

    /// Closing decorator for inline code (default: "`").
    fn code_close(&self) -> &str {
        "`"
    }

    // ========================================================================
    // Strikethrough decorators
    // ========================================================================

    /// Opening decorator for strikethrough text (default: "~~").
    fn strikethrough_open(&self) -> &str {
        "~~"
    }

    /// Closing decorator for strikethrough text (default: "~~").
    fn strikethrough_close(&self) -> &str {
        "~~"
    }
}

/// Default mapper preserving markdown decorators.
///
/// This mapper keeps the original markdown syntax (`*`, `**`, `` ` ``).
/// Use [`StyledMapper`] when applying visual styles (colors, bold, italic)
/// that replace the textual decorators.
#[derive(Debug, Clone, Copy, Default)]
pub struct DefaultMapper;

impl Mapper for DefaultMapper {}

/// Mapper for styled output with fancy Unicode symbols.
///
/// Removes emphasis/code/strikethrough decorators (replaced by visual styles)
/// and uses fancy Unicode symbols for structural elements.
#[derive(Debug, Clone, Copy, Default)]
pub struct StyledMapper;

impl Mapper for StyledMapper {
    // Fancy link decorators
    fn link_desc_open(&self) -> &str {
        "▐"
    }
    fn link_desc_close(&self) -> &str {
        "▌"
    }
    fn link_url_open(&self) -> &str {
        "◖"
    }
    fn link_url_close(&self) -> &str {
        "◗"
    }

    // Fancy blockquote bar
    fn blockquote_bar(&self) -> &str {
        "▌ "
    }

    // Fancy horizontal rule
    fn horizontal_rule_char(&self) -> &str {
        "─"
    }

    // Fancy task checkbox
    fn task_checked(&self) -> &str {
        "[✓] "
    }

    // Fancy table borders
    fn table_vertical(&self) -> &str {
        "│"
    }
    fn table_horizontal(&self) -> &str {
        "─"
    }
    fn table_top_left(&self) -> &str {
        "┌"
    }
    fn table_top_right(&self) -> &str {
        "┐"
    }
    fn table_bottom_left(&self) -> &str {
        "└"
    }
    fn table_bottom_right(&self) -> &str {
        "┘"
    }
    fn table_top_junction(&self) -> &str {
        "┬"
    }
    fn table_bottom_junction(&self) -> &str {
        "┴"
    }
    fn table_left_junction(&self) -> &str {
        "├"
    }
    fn table_right_junction(&self) -> &str {
        "┤"
    }
    fn table_cross(&self) -> &str {
        "┼"
    }

    // Remove text decorators - styling replaces them
    fn emphasis_open(&self) -> &str {
        ""
    }
    fn emphasis_close(&self) -> &str {
        ""
    }
    fn strong_open(&self) -> &str {
        ""
    }
    fn strong_close(&self) -> &str {
        ""
    }
    fn code_open(&self) -> &str {
        ""
    }
    fn code_close(&self) -> &str {
        ""
    }
    fn strikethrough_open(&self) -> &str {
        ""
    }
    fn strikethrough_close(&self) -> &str {
        ""
    }
}
