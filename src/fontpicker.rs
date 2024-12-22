use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use font_loader::system_fonts;
use ratatui::{style::Stylize, text::Line, widgets::Paragraph};
use ratatui_image::{picker::Picker, protocol::Protocol, Image};
use rusttype::Font;

use crate::{
    setup::Renderer,
    widget_sources::{header_source, WidgetSourceData},
    Error,
};

pub async fn set_up_font(
    picker: &mut Picker,
    bg: Option<[u8; 4]>,
) -> Result<Option<String>, Error> {
    let mut terminal = ratatui::init_with_options(ratatui::TerminalOptions {
        viewport: ratatui::Viewport::Inline(6),
    });
    terminal.clear()?;

    let mut input = String::new();

    let all_fonts: Vec<(String, String)> = system_fonts::query_all()
        .iter()
        .map(|f| (f.clone(), f.to_ascii_lowercase()))
        .collect();

    let mut last_rendered: Option<(String, Protocol)> = None;
    let mut inner_width = 0;

    loop {
        let first_match = find_first_match(&all_fonts, &input);

        let (font, first_match) = if let Some((first_match, _)) = first_match {
            let fp_builder = system_fonts::FontPropertyBuilder::new().family(&first_match);
            let property = fp_builder.build();
            let (font_data, _) = system_fonts::get(&property).ok_or(Error::NoFont)?;

            let font = Font::try_from_vec(font_data).ok_or(Error::NoFont)?;
            (Some(font), Some(first_match))
        } else {
            (None, None)
        };

        terminal.draw(|f| {
            let area = f.area();
            let block = ratatui::widgets::Block::default()
                .title("Enter font name (Tab: complete, Esc: abort, Enter: confirm):")
                .borders(ratatui::widgets::Borders::ALL);
            let inner_area = block.inner(area);
            inner_width = inner_area.width;
            f.render_widget(block, area);

            if let Some(first_match) = first_match {
                f.render_widget(
                    Paragraph::new(Line::from(first_match.clone()).dark_gray()),
                    inner_area,
                );
            }
            f.render_widget(
                Paragraph::new(Line::from(input.clone()).white()),
                inner_area,
            );

            if let Some((_, ref mut proto)) = last_rendered {
                let img = Image::new(proto);
                let mut area = inner_area;
                area.y += 1;
                area.height = 2;
                f.render_widget(img, area);
            }
        })?;

        if inner_width > 0 && (last_rendered.is_none() || last_rendered.clone().unwrap().0 != input)
        {
            if let Some(font) = font {
                let spans = vec!["The fox jumped over the goat or something".into()];
                let sources = header_source(
                    &Renderer::new(*picker, font, bg),
                    inner_width,
                    0,
                    Line::from(spans).to_string(),
                    1,
                    false,
                )
                .await?;

                // Just render the first line if it got split.
                if let Some(source) = sources.into_iter().next() {
                    if let WidgetSourceData::Image(proto) = source.source {
                        last_rendered = Some((input.clone(), proto));
                    }
                }
            }
        }

        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(KeyEvent {
                code, modifiers, ..
            }) = event::read()?
            {
                match code {
                    KeyCode::Char(c) => {
                        // Append the character unless a control modifier is pressed
                        if modifiers.is_empty() {
                            input.push(c);
                        } else if modifiers == KeyModifiers::SHIFT {
                            input.push(c.to_ascii_uppercase());
                        }
                    }
                    KeyCode::Backspace => {
                        input.pop(); // Remove the last character
                    }
                    KeyCode::Tab => {
                        if let Some(first_match) = find_first_match(&all_fonts, &input) {
                            input = first_match.0;
                        }
                    }
                    KeyCode::Enter => {
                        if let Some(first_match) = find_first_match(&all_fonts, &input) {
                            terminal.clear()?;
                            ratatui::restore();
                            return Ok(Some(first_match.0));
                        }
                    }
                    KeyCode::Esc => {
                        // Exit on Escape
                        terminal.clear()?;
                        ratatui::restore();
                        return Ok(None);
                    }
                    _ => {}
                }
            }
        }
    }
}

fn find_first_match(all_fonts: &Vec<(String, String)>, input: &str) -> Option<(String, String)> {
    let mut first_match = None;
    if !input.is_empty() {
        for font in all_fonts {
            if font.1.starts_with(&input.to_ascii_lowercase()) {
                first_match = Some(font);
                break;
            }
        }
    } else {
        first_match = all_fonts.first();
    };
    first_match.cloned()
}
