use std::borrow::Cow;
use std::num::NonZeroU16;

use crossterm::Command as _;
use crossterm::style::{
    Attribute, Attributes, Print, ResetColor, SetAttributes, SetBackgroundColor, SetForegroundColor,
};
use ratatui::buffer::CellDiffOption;
use ratatui::style::Modifier;
use ratatui::text::Span;

use ratatui::prelude::IntoCrossterm as _;
use ratatui::widgets::Widget;
use unicode_width::UnicodeWidthStr as _;

#[derive(Debug)]
pub struct Osc8Link<'a> {
    spans: Vec<Span<'a>>,
    url: Cow<'a, str>,
}
impl<'a> Osc8Link<'a> {
    pub fn new<S: Into<Vec<Span<'a>>>, U: Into<Cow<'a, str>>>(spans: S, url: U) -> Self {
        let spans: Vec<Span> = spans
            .into()
            .into_iter()
            .filter(|s| !s.content.is_empty())
            .collect();
        Self {
            spans,
            url: url.into(),
        }
    }
}

impl Widget for Osc8Link<'_> {
    fn render(self, area: ratatui::prelude::Rect, buf: &mut ratatui::prelude::Buffer)
    where
        Self: Sized,
    {
        let Ok((sequence, width)) = render_osc8_link(&self.spans, &self.url) else {
            return;
        };
        let remaining = buf.area().right().saturating_sub(area.x);
        let Some(forced_width) = NonZeroU16::new((width as u16).min(remaining)) else {
            return;
        };
        let Some(cell) = buf.cell_mut(area) else {
            return;
        };
        cell.set_symbol(&sequence)
            .set_diff_option(CellDiffOption::ForcedWidth(forced_width));
    }
}

fn span_to_ansi(span: &Span) -> Result<String, std::fmt::Error> {
    let mut out = String::new();
    if let Some(fg) = span.style.fg {
        SetForegroundColor(fg.into_crossterm()).write_ansi(&mut out)?
    }
    if let Some(bg) = span.style.bg {
        SetBackgroundColor(bg.into_crossterm()).write_ansi(&mut out)?;
    }
    SetAttributes(modifier_to_attributes(span.style.add_modifier)).write_ansi(&mut out)?;
    Print(&span.content).write_ansi(&mut out)?;
    ResetColor.write_ansi(&mut out)?;
    Ok(out)
}

fn modifier_to_attributes(m: Modifier) -> Attributes {
    let mut attrs = Attributes::default();
    if m.contains(Modifier::BOLD) {
        attrs.set(Attribute::Bold);
    }
    if m.contains(Modifier::DIM) {
        attrs.set(Attribute::Dim);
    }
    if m.contains(Modifier::ITALIC) {
        attrs.set(Attribute::Italic);
    }
    if m.contains(Modifier::UNDERLINED) {
        attrs.set(Attribute::Underlined);
    }
    if m.contains(Modifier::SLOW_BLINK) {
        attrs.set(Attribute::SlowBlink);
    }
    if m.contains(Modifier::RAPID_BLINK) {
        attrs.set(Attribute::RapidBlink);
    }
    if m.contains(Modifier::REVERSED) {
        attrs.set(Attribute::Reverse);
    }
    if m.contains(Modifier::HIDDEN) {
        attrs.set(Attribute::Hidden);
    }
    if m.contains(Modifier::CROSSED_OUT) {
        attrs.set(Attribute::CrossedOut);
    }
    attrs
}

pub fn render_osc8_link(spans: &[Span], url: &str) -> Result<(String, usize), std::fmt::Error> {
    let mut inner = String::new();
    let mut width = 0;
    for span in spans {
        inner.push_str(&span_to_ansi(span)?);
        width += span.content.width();
    }
    Ok((format!("\x1b]8;;{url}\x1b\\{inner}\x1b]8;;\x1b\\"), width))
}

#[cfg(test)]
#[expect(clippy::unwrap_used)]
mod tests {
    use ratatui::{buffer::Buffer, layout::Rect, style::Stylize as _};

    use super::*;

    #[test]
    fn basic_link() {
        assert_eq!(
            "\x1b]8;;http://example.com\x1b\\plain\u{1b}[0m\x1b]8;;\x1b\\",
            render_osc8_link(&[Span::from("plain")], "http://example.com")
                .unwrap()
                .0,
        );
    }

    #[test]
    fn styled_link() {
        assert_eq!(
            "\x1b]8;;http://example.com\x1b\\\u{1b}[38;5;2mhello \u{1b}[0m\u{1b}[38;5;1m\u{1b}[1mworld\u{1b}[0m\x1b]8;;\x1b\\",
            render_osc8_link(
                &[
                    Span::from("hello ").green(),
                    Span::from("world").red().bold()
                ],
                "http://example.com"
            )
            .unwrap()
            .0,
        );
    }

    #[test]
    fn render_into_buffer() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 20));
        let link = Osc8Link::new(
            &[
                Span::from("hello ").green(),
                Span::from("world").red().bold(),
            ],
            "http://example.com",
        );

        link.render(Rect::new(0, 1, 80, 1), &mut buf);
        // TODO: test this, but how?
    }
}
