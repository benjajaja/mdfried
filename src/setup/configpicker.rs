use std::io;

use ratatui::{
    Terminal, TerminalOptions,
    crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    prelude::*,
    style::Color,
    widgets::{Block, Borders, Padding, Paragraph},
};

use crate::{
    config::{Config, store},
    error::Error,
};

pub fn interactive_resolve_config(error: Error) -> Result<Config, Error> {
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

    let mut focus: i8 = 0;
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

            let buttons = [
                "Overwrite config file",
                "Ignore and use defaults",
                "Abort and quit",
            ];

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
                let style = if i as i8 == focus {
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
                    KeyCode::Enter => match focus {
                        0 => {
                            terminal.clear()?;
                            ratatui::restore();
                            store(Config::default())?;
                            return Ok(Config::default());
                        }
                        1 => {
                            terminal.clear()?;
                            ratatui::restore();
                            return Ok(Config::default());
                        }
                        _ => {
                            terminal.clear()?;
                            ratatui::restore();
                            return Err(Error::UserAbort("q"));
                        }
                    },
                    KeyCode::Char('o') => {
                        focus = 0;
                    }
                    KeyCode::Char('i') => {
                        focus = 1;
                    }
                    KeyCode::Char('a') => {
                        focus = 2;
                    }
                    KeyCode::Tab => {
                        focus += 1;
                        if focus > 2 {
                            focus = 0;
                        }
                    }
                    KeyCode::Right | KeyCode::Char('l') => {
                        focus += 1;
                        if focus > 2 {
                            focus = 0;
                        }
                    }
                    KeyCode::Left | KeyCode::Char('h') => {
                        focus -= 1;
                        if focus < 0 {
                            focus = 2;
                        }
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
