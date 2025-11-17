use std::io;

use clap::{Parser, command};
use confy::ConfyError;
use ratatui::{
    Terminal, TerminalOptions,
    crossterm::{
        event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
        style::Color,
    },
    prelude::*,
    widgets::{Block, BorderType, Borders, Paragraph},
};
use ratatui_image::picker::ProtocolType;
use serde::{Deserialize, Serialize};

use crate::{Padding, error::Error};

#[derive(Parser)]
#[command(name = "mdfried")]
#[command(version = "0.1")]
#[command(about = "Deep fries Markdown", long_about = Some("You can cook a terminal. Can you *deep fry* a terminal?"))]
pub struct Cli {
    /// Optional name to operate on
    pub filename: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub font_family: Option<String>,
    pub padding: Padding,
    pub skin: ratskin::MadSkin,
    pub enable_mouse_capture: bool,
    pub max_image_height: u16,
    pub debug_override_protocol_type: Option<ProtocolType>,
}

impl Default for Config {
    fn default() -> Self {
        let mut skin = ratskin::MadSkin::default();

        skin.bold.set_fg(Color::AnsiValue(220));

        skin.inline_code.set_fg(Color::AnsiValue(203));
        skin.inline_code.set_bg(Color::AnsiValue(236));

        skin.code_block.set_fg(Color::AnsiValue(203));

        skin.quote_mark.set_fg(Color::AnsiValue(63));
        skin.bullet.set_fg(Color::AnsiValue(63));

        let enable_mouse_capture = false;

        let max_image_height = 30;

        let debug_override_protocol_type = None;

        Self {
            font_family: Default::default(),
            padding: Default::default(),
            skin,
            enable_mouse_capture,
            max_image_height,
            debug_override_protocol_type,
        }
    }
}

const CONFIG_APP_NAME: &str = "mdfried";
const CONFIG_CONFIG_NAME: &str = "config";

pub fn get_configuration_file_path() -> String {
    confy::get_configuration_file_path(CONFIG_APP_NAME, CONFIG_CONFIG_NAME)
        .map(|p| p.display().to_string())
        .unwrap_or("(unknown config file path)".into())
}

pub fn store(new_config: Config) -> Result<(), ConfyError> {
    confy::store(CONFIG_APP_NAME, CONFIG_CONFIG_NAME, new_config)
}

pub fn load_or_ask() -> Result<Config, Error> {
    match confy::load::<Config>(CONFIG_APP_NAME, CONFIG_CONFIG_NAME) {
        Ok(config) => Ok(config),
        Err(error) => interactive_resolve_config(error.into()),
    }
}

fn interactive_resolve_config(error: Error) -> Result<Config, Error> {
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
                .border_style(ratatui::style::Color::White)
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
            let buttons = vec![
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
