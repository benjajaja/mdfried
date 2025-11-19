use std::io;

use ratatui::{
    Terminal, TerminalOptions,
    crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    prelude::*,
    style::Color,
    widgets::{Block, Borders, Padding, Paragraph},
};

use crate::error::Error;

#[derive(PartialEq)]
pub enum ConfigResolution {
    Overwrite,
    Ignore,
    Abort,
}

impl ConfigResolution {
    fn next(&mut self) {
        use ConfigResolution::*;
        *self = match self {
            Overwrite => Ignore,
            Ignore => Abort,
            Abort => Overwrite,
        };
    }

    fn prev(&mut self) {
        use ConfigResolution::*;
        *self = match self {
            Overwrite => Abort,
            Ignore => Overwrite,
            Abort => Ignore,
        };
    }
}

impl From<usize> for ConfigResolution {
    fn from(value: usize) -> Self {
        use ConfigResolution::*;
        match value {
            0 => Overwrite,
            1 => Ignore,
            _ => Abort,
        }
    }
}

pub fn interactive_resolve_config(error: Error) -> Result<ConfigResolution, Error> {
    println!("{error}");
    ratatui::crossterm::terminal::enable_raw_mode()?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::with_options(
        backend,
        TerminalOptions {
            viewport: ratatui::Viewport::Inline(7),
        },
    )?;
    terminal.clear()?;

    let mut focus = ConfigResolution::Overwrite;
    loop {
        terminal.draw(|f| {
            let area = f.area();

            let block = Block::default()
                .title("Could not use configuration file")
                .borders(Borders::ALL)
                .border_style(Color::Yellow)
                .padding(Padding::proportional(1));

            let inner_area = block.inner(area);

            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1),
                    Constraint::Fill(1),
                    Constraint::Length(1),
                    Constraint::Fill(1),
                ])
                .split(inner_area);

            f.render_widget(block, area);

            f.render_widget(Paragraph::new("What would you like to do?"), chunks[0]);

            let buttons = ["Overwrite file with default config", "Ignore", "Abort"];

            let button_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Fill(1),
                    Constraint::Length(buttons[0].len() as u16 + 4),
                    Constraint::Length(buttons[1].len() as u16 + 4),
                    Constraint::Length(buttons[2].len() as u16 + 4),
                ])
                .split(chunks[2]);

            for (i, &button_text) in buttons.iter().enumerate() {
                let style = if ConfigResolution::from(i) == focus {
                    Style::default()
                        .bg(ratatui::style::Color::Blue)
                        .fg(ratatui::style::Color::Black)
                } else {
                    Style::default().fg(ratatui::style::Color::Blue)
                };
                let mut chars = button_text.chars();
                let first = chars.next().map(|c| c.to_string()).unwrap_or_default();
                let rest = chars.as_str();
                f.render_widget(
                    Paragraph::new(Line::from(vec![
                        Span::from(first).fg(Color::Yellow),
                        Span::from(rest),
                    ]))
                    .alignment(Alignment::Center)
                    .style(style),
                    button_chunks[i + 1],
                );
            }

            f.render_widget(
                Paragraph::new(Line::from(" Esc: abort, Enter: confirm ").dark_gray()),
                Rect::new(1, f.area().y + f.area().height - 1, inner_area.width, 1),
            );
        })?;

        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(KeyEvent {
                code, modifiers, ..
            }) = event::read()?
            {
                match code {
                    KeyCode::Enter => {
                        terminal.clear()?;
                        ratatui::restore();
                        return Ok(focus);
                    }
                    KeyCode::Char('o') => {
                        focus = ConfigResolution::Overwrite;
                    }
                    KeyCode::Char('i') => {
                        focus = ConfigResolution::Ignore;
                    }
                    KeyCode::Char('a') => {
                        focus = ConfigResolution::Abort;
                    }
                    KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => {
                        focus.next();
                    }
                    KeyCode::Left | KeyCode::Char('h') => {
                        focus.prev();
                    }
                    KeyCode::Char('q') => {
                        // Exit on q
                        terminal.clear()?;
                        ratatui::restore();
                        return Err(Error::UserAbort("q"));
                    }
                    KeyCode::Esc => {
                        // Exit on Escape
                        terminal.clear()?;
                        ratatui::restore();
                        return Err(Error::UserAbort("esc"));
                    }
                    KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                        // Exit on Ctrl-C too
                        terminal.clear()?;
                        ratatui::restore();
                        return Err(Error::UserAbort("ctrl-c"));
                    }
                    _ => {}
                }
            }
        }
    }
}
