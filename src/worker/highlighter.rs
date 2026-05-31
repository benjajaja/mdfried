use ansi_to_tui::IntoText;
use arborium::AnsiHighlighter;
use itertools::Itertools as _;
use mdfrier::ratatui::Theme as _;
use ratatui::{
    style::{Color, Stylize as _},
    text::{Line, Text},
};

use crate::error::Error;

pub struct Highlighter {
    hl: AnsiHighlighter,
    bg: Color,
}

impl Highlighter {
    pub fn new(mdfried_theme: &crate::config::Theme) -> Self {
        let theme = arborium::theme::builtin::tokyo_night().clone();
        let bg = mdfried_theme.code_bg();
        let hl = AnsiHighlighter::new(theme);
        Self { hl, bg }
    }

    pub fn highlight(
        &mut self,
        language: &str,
        lines: Vec<Line<'_>>,
    ) -> Result<Text<'static>, Error> {
        let code = lines.into_iter().map(|line| line.to_string()).join("\n");
        self.hl
            .highlight(language, &code)
            .map_err(Into::<Error>::into)
            .and_then(|colored| IntoText::into_text(&colored).map_err(Into::<Error>::into))
            .map(|mut text| {
                text.lines = text
                    .lines
                    .into_iter()
                    .map(|mut line| {
                        line.spans = line
                            .spans
                            .into_iter()
                            .map(|span| span.bg(self.bg))
                            .collect();
                        line.bg(self.bg)
                    })
                    .collect();
                text
            })
    }
}

#[cfg(test)]
#[expect(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn highlight_languages() {
        let theme = crate::config::Theme::default();
        let mut hl = Highlighter::new(&theme);

        let result = hl
            .highlight(
                "ada",
                vec![
                    Line::from("with Ada.Text_IO;"),
                    Line::from("procedure Hello is"),
                    Line::from("begin"),
                    Line::from("   Ada.Text_IO.Put_Line (\"Hello, World!\");"),
                    Line::from("end Hello;"),
                ],
            )
            .unwrap();

        assert!(
            result
                .lines
                .into_iter()
                .flat_map(|line| line.spans)
                .any(|span| span.style.fg.is_some())
        );
    }
}
