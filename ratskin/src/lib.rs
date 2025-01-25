//! Ratskin is a wrapper around [termimad] that parses markdown into [ratatui::text::Line]s.
//!
//! ```rust
//! # use ratskin::RatSkin;
//! # use ratatui::{style::Stylize, text::{Line, Span}};
//! let rat_skin = RatSkin::default();
//! let text = RatSkin::parse_text("**cook it!**");
//! let lines: Vec<Line> = rat_skin.parse(text, 80);
//! assert_eq!(lines, vec![Line::from(Span::from("cook it!").bold())]);
//! ```
//!
//! This is all you need to know about Ratskin - for everything else, please see termimad:
//!
//! * <https://github.com/Canop/termimad>
//! * <https://crates.io/crates/termimad>
//! * <https://docs.rs/termimad/latest/termimad/>
//!
//! Because termimad is very streamlined for writing terminal output directly (for good reasons),
//! a small part of the logic had to be rewritten for ratatui Spans and Lines.
use crossterm::style::Attribute;
use ratatui::{
    style::Stylize,
    text::{Line, Span},
};
pub use termimad::MadSkin;
use termimad::{
    minimad::{parse_text, Text},
    CompositeKind, CompoundStyle, FmtComposite, FmtLine, FmtText, ListItemsIndentationMode,
    RelativePosition, Spacing, StyledChar,
};

/// Wrapper around [termimad::MadSkin].
///
/// ```rust
/// # use ratskin::RatSkin;
/// let mut rat_skin = RatSkin::default();
/// rat_skin.skin.bold.set_fg((255, 0, 0).into());
/// ```
///
/// Hint: [termimad::MadSkin] can be de/serialized.
#[derive(Default)]
pub struct RatSkin {
    /// See [termimad::MadSkin].
    pub skin: MadSkin,
}

impl RatSkin {
    /// Just a wrapper around termimad::parse_text. Returns an intermediate `Text`, which is just
    /// the text split into lines.
    pub fn parse_text(text: &str) -> Text {
        parse_text(text, termimad::minimad::Options::default())
    }

    // Parse the `Text` into ratatui Line widgets.
    pub fn parse<'a>(&self, text: Text, width: u16) -> Vec<Line<'a>> {
        let mut lines = vec![];

        let fmt_text = FmtText::from_text(&self.skin, text, Some(width as usize));

        for line in fmt_text.lines {
            match line {
                FmtLine::Normal(fmtcomp) => {
                    let spans = fmt_composite_to_spans(
                        &self.skin,
                        fmtcomp,
                        true,
                        Some(width as usize),
                        false,
                    );
                    lines.push(Line::from(spans));
                    // self.add_line(&mut lines, spans);
                }
                FmtLine::HorizontalRule => {
                    lines.push(Line::from(Span::from(
                        self.skin
                            .horizontal_rule
                            .nude_char()
                            .to_string()
                            .repeat(width as usize),
                    )));
                }
                FmtLine::TableRow(fmt) => {
                    let mut spans = vec![];
                    let tbl_width = 1 + fmt.cells.iter().fold(0, |sum, cell| {
                        if let Some(spacing) = cell.spacing {
                            sum + spacing.width + 1
                        } else {
                            sum + cell.visible_length + 1
                        }
                    });
                    let (lpo, rpo) = Spacing::optional_completions(
                        self.skin.table.align,
                        tbl_width,
                        Some(width as usize),
                    );
                    spans.push(Span::from(" ".repeat(lpo)));

                    for cell in fmt.cells {
                        spans.push(compoundstyle_to_span(
                            self.skin.table_border_chars.vertical.to_string(),
                            &self.skin.table.compound_style,
                        ));

                        let cell_spans = fmt_composite_to_spans(
                            &self.skin,
                            cell,
                            false,
                            Some(width as usize),
                            false,
                        );
                        spans.extend(cell_spans);
                    }
                    spans.push(compoundstyle_to_span(
                        self.skin.table_border_chars.vertical.to_string(),
                        &self.skin.table.compound_style,
                    ));

                    spans.push(Span::from(" ".repeat(rpo)));

                    lines.push(Line::from(spans));
                }
                FmtLine::TableRule(rule) => {
                    let mut chars = String::with_capacity(width as usize);
                    let tbl_width = 1 + rule.widths.iter().fold(0, |sum, w| sum + w + 1);
                    let (lpo, rpo) = Spacing::optional_completions(
                        self.skin.table.align,
                        tbl_width,
                        Some(width as usize),
                    );
                    chars.push_str(&" ".repeat(lpo));

                    chars.push(match rule.position {
                        RelativePosition::Top => self.skin.table_border_chars.top_left_corner,
                        RelativePosition::Other => self.skin.table_border_chars.left_junction,
                        RelativePosition::Bottom => self.skin.table_border_chars.bottom_left_corner,
                    });

                    for (idx, &width) in rule.widths.iter().enumerate() {
                        if idx > 0 {
                            chars.push(match rule.position {
                                RelativePosition::Top => self.skin.table_border_chars.top_junction,
                                RelativePosition::Other => self.skin.table_border_chars.cross,
                                RelativePosition::Bottom => {
                                    self.skin.table_border_chars.bottom_junction
                                }
                            });
                        }
                        chars.push_str(
                            &self
                                .skin
                                .table_border_chars
                                .horizontal
                                .to_string()
                                .repeat(width),
                        );
                    }

                    chars.push(match rule.position {
                        RelativePosition::Top => self.skin.table_border_chars.top_right_corner,
                        RelativePosition::Other => self.skin.table_border_chars.right_junction,
                        RelativePosition::Bottom => {
                            self.skin.table_border_chars.bottom_right_corner
                        }
                    });
                    chars.push_str(&" ".repeat(rpo));

                    let mut span = Span::from(chars);
                    span = style_to_span(&self.skin.table.compound_style, span);
                    lines.push(Line::from(vec![span]));
                }
            }
        }
        lines
    }
}

// This is duplicated from MadSkin::write_fmt_composite, but with ratatui Spans.
fn fmt_composite_to_spans<'a>(
    skin: &MadSkin,
    fc: FmtComposite<'_>,
    with_margins: bool,
    outer_width: Option<usize>,
    with_right_completion: bool,
) -> Vec<Span<'a>> {
    let mut spans = vec![];

    let ls = skin.line_style(fc.kind);
    let (left_margin, right_margin) = if with_margins {
        ls.margins_in(outer_width)
    } else {
        (0, 0)
    };
    let (lpi, rpi) = fc.completions(); // inner completion
    let inner_width = fc.spacing.map_or(fc.visible_length, |sp| sp.width);
    let (lpo, rpo) = Spacing::optional_completions(
        ls.align,
        inner_width + left_margin + right_margin,
        outer_width,
    );
    if lpo + left_margin > 0 {
        spans.push(space(skin, lpo + left_margin));
    }
    if lpi > 0 {
        spans.push(compoundstyle_to_span(
            " ".repeat(lpi),
            &skin.line_style(fc.kind).compound_style,
        ));
    }

    if let CompositeKind::ListItem(depth) = fc.kind {
        if depth > 0 {
            spans.push(space(skin, depth as usize));
        }
        spans.push(styled_char_to_span(&skin.bullet));
        spans.push(space(skin, 1));
    }
    if skin.list_items_indentation_mode == ListItemsIndentationMode::Block {
        if let CompositeKind::ListItemFollowUp(depth) = fc.kind {
            spans.push(space(skin, (depth + 2) as usize));
        }
    }
    if fc.kind == CompositeKind::Quote {
        spans.push(styled_char_to_span(&skin.quote_mark));
        spans.push(space(skin, 1));
    }
    // #[cfg(feature = "special-renders")]
    // for c in &fmtcomp.compounds {
    // if let Some(replacement) = skin.special_chars.get(c) {
    // spans.push(styled_char_to_span(replacement));
    // } else {
    // let os = skin.compound_style(ls, c);
    // comp_style_to_span(c.as_str().to_string(), &os);
    // }
    // }
    // #[cfg(not(feature = "special-renders"))]
    for c in &fc.compounds {
        let os = skin.compound_style(ls, c);
        spans.push(compoundstyle_to_span(c.as_str().to_string(), &os));
    }
    if rpi > 0 {
        spans.push(space(skin, rpi));
    }
    if with_right_completion && rpo + right_margin > 0 {
        spans.push(space(skin, rpo + right_margin));
    }
    spans
}

fn space<'a>(skin: &MadSkin, repeat: usize) -> Span<'a> {
    style_to_span(
        &skin.paragraph.compound_style,
        Span::from(" ".repeat(repeat)),
    )
}

fn styled_char_to_span<'a>(ch: &StyledChar) -> Span<'a> {
    style_to_span(ch.compound_style(), Span::from(ch.nude_char().to_string()))
}

// Make a ratatui Span from a termimad Compound, using the skin.
fn compoundstyle_to_span<'a>(src: String, style: &CompoundStyle) -> Span<'a> {
    style_to_span(style, Span::from(src))
}

// Convert from crossterm style to ratatui generic style, and set it on the span.
fn style_to_span<'a>(style: &CompoundStyle, mut span: Span<'a>) -> Span<'a> {
    if let Some(color) = style.object_style.foreground_color {
        span = span.fg(color);
    }
    if let Some(color) = style.object_style.background_color {
        span = span.bg(color);
    }
    if style.object_style.attributes.has(Attribute::Underlined) {
        span = span.underlined();
    }
    if style.object_style.attributes.has(Attribute::Bold) {
        span = span.bold();
    }
    if style.object_style.attributes.has(Attribute::Italic) {
        span = span.italic();
    }
    if style.object_style.attributes.has(Attribute::CrossedOut) {
        span = span.crossed_out();
    }
    span
}

#[cfg(test)]
mod tests {
    use crate::RatSkin;
    use ratatui::{
        style::Stylize,
        text::{Line, Span},
    };

    #[test]
    fn basic() {
        let rat_skin = RatSkin::default();
        let text = RatSkin::parse_text("**cook it!**");
        let lines: Vec<Line> = rat_skin.parse(text, 80);
        assert_eq!(lines, vec![Line::from(Span::from("cook it!").bold())]);
    }

    #[test]
    fn wrap() {
        let rat_skin = RatSkin::default();
        let text = RatSkin::parse_text("**cook it!**");
        let lines: Vec<Line> = rat_skin.parse(text, 4);
        assert_eq!(
            lines,
            vec![
                Line::from(Span::from("cook").bold()),
                Line::from(Span::from("it!").bold()),
            ]
        );
    }
}
