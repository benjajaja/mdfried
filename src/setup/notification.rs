use std::io;

use ratatui::{
    Terminal, TerminalOptions,
    crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    prelude::*,
    style::Color,
    widgets::{Block, Borders, Gauge, Padding},
};

use crate::error::Error;
pub fn interactive_notification(message: &'static str) -> Result<(), Error> {
    ratatui::crossterm::terminal::enable_raw_mode()?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::with_options(
        backend,
        TerminalOptions {
            viewport: ratatui::Viewport::Inline(7),
        },
    )?;
    terminal.clear()?;

    let mut progress: i16 = 0;

    loop {
        terminal.draw(|f| {
            let area = f.area();

            let block = Block::default()
                .title(message)
                .borders(Borders::ALL)
                .border_style(Color::Yellow)
                .padding(Padding::proportional(1));

            let mut inner_area = block.inner(area);
            inner_area.height -= 2;
            inner_area.y += 2;

            f.render_widget(block, area);

            if progress as u16 >= inner_area.width {
                progress = -1;
            } else {
                let gauge = Gauge::default()
                    .gauge_style(Color::Blue)
                    .ratio(f64::from(progress) / f64::from(inner_area.width))
                    .label("Continue...");
                f.render_widget(gauge, inner_area);
            }
        })?;

        if progress == -1 {
            terminal.clear()?;
            ratatui::restore();
            return Ok(());
        }

        if event::poll(std::time::Duration::from_millis(10))? {
            if let Event::Key(KeyEvent {
                code, modifiers, ..
            }) = event::read()?
            {
                match code {
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
        } else {
            progress += 1;
        }
    }
}
