use std::io;

use ratatui::{
    Terminal, TerminalOptions,
    crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    prelude::*,
    style::Color,
    widgets::{Block, BorderType, Borders, Paragraph},
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
            viewport: ratatui::Viewport::Inline(4),
        },
    )?;
    terminal.clear()?;

    let mut focus: i8 = 0;
    loop {
        terminal.draw(|f| {
            let area = f.area();

            // Create the main block with border
            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Plain)
                .border_style(Color::White)
                .bg(ratatui::style::Color::DarkGray);

            // Calculate inner area after border
            let inner = block.inner(area);

            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(1), Constraint::Length(1)])
                .split(inner);

            f.render_widget(block, area);

            f.render_widget(Paragraph::new("What would you like to do?"), chunks[0]);

            // Create buttons
            let button_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Percentage(33),
                    Constraint::Percentage(33),
                    Constraint::Percentage(34),
                ])
                .split(chunks[1]);

            // Render buttons (you'd want to track which is selected)
            let buttons = [
                "Overwrite config file",
                "Ignore and use defaults",
                "Abort and quit",
            ];
            for (i, &button_text) in buttons.iter().enumerate() {
                let style = if i as i8 == focus {
                    Style::default().bg(ratatui::style::Color::Blue)
                } else {
                    Style::default()
                };
                f.render_widget(
                    Paragraph::new(format!("[ {} ]", button_text))
                        .alignment(Alignment::Center)
                        .style(style),
                    button_chunks[i],
                );
            }
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
