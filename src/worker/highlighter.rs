use ansi_to_tui::IntoText as _;
use arborium::AnsiHighlighter;
use itertools::Itertools as _;
use mdfrier::ratatui::Theme as _;
use ratatui::{
    style::{Color, Stylize as _},
    text::{Line, Text},
};

// use crate::config::Theme as _;
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
        lines: Vec<Line<'static>>,
    ) -> Result<Text<'static>, Error> {
        let code = lines.into_iter().map(|line| line.to_string()).join("\n");
        self.hl
            .highlight(language, &code)
            .map_err(Into::<Error>::into)
            .and_then(|colored| colored.into_text().map_err(Into::<Error>::into))
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
