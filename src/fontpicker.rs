use std::io::Stdout;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use font_loader::system_fonts;
use ratatui::{
    backend::CrosstermBackend, style::Stylize, text::Line, widgets::Paragraph, Terminal,
};
use ratatui_image::{picker::Picker, Image};
use rusttype::Font;

use crate::{
    widget_sources::{header_source, WidgetSourceData},
    Error,
};

pub fn pick_a_font(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    picker: &mut Picker,
    bg: Option<[u8; 4]>,
) -> Result<String, Error> {
    let mut input = String::new();

    let all_fonts: Vec<(String, String)> = system_fonts::query_all()
        .iter()
        .map(|f| (f.clone(), f.to_ascii_lowercase()))
        .collect();

    loop {
        let font = if !input.is_empty() {
            let fp_builder = system_fonts::FontPropertyBuilder::new().family(&input);
            let property = fp_builder.build();
            let (font_data, _) =
                system_fonts::get(&property).ok_or("Could not get system fonts property")?;

            let font = Font::try_from_vec(font_data).ok_or(Error::NoFont)?;
            Some(font)
        } else {
            None
        };

        terminal.draw(|f| {
            let area = f.area();
            let block = ratatui::widgets::Block::default()
                .title("Enter font name:")
                .borders(ratatui::widgets::Borders::ALL);
            let inner_area = block.inner(area);
            f.render_widget(block, area);

            if let Some(first_match) = find_first_match(&all_fonts, &input) {
                f.render_widget(
                    Paragraph::new(Line::from(first_match.0.clone()).dark_gray()),
                    inner_area,
                );
            }
            f.render_widget(
                Paragraph::new(Line::from(input.clone()).white()),
                inner_area,
            );

            if let Some(mut font) = font {
                let spans = vec!["The fox jumped over the goat or something".into()];
                if let Ok(source) =
                    header_source(picker, &mut font, bg, inner_area.width, spans, 1, false)
                {
                    if let WidgetSourceData::Image(mut proto) = source.source {
                        let img = Image::new(&mut proto);
                        let mut area = inner_area;
                        area.y = area.y + 1;
                        area.height = 2;
                        f.render_widget(img, area);
                    }
                }
            }
        })?;
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
                        // Finalize input on Enter
                        break;
                    }
                    KeyCode::Esc => {
                        // Exit on Escape
                        input.clear();
                        break;
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(input)
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
