use std::collections::BTreeMap;

use cosmic_text::{FontSystem, SwashCache};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::{style::Stylize, text::Line, widgets::Paragraph};
use ratatui_image::{picker::Picker, protocol::Protocol, Image};

use crate::{
    setup::{BgColor, FontRenderer},
    widget_sources::{header_images, header_sources, WidgetSourceData},
    Error,
};

pub fn interactive_font_picker(
    picker: &mut Picker,
    bg: Option<BgColor>,
) -> Result<Option<String>, Error> {
    let mut terminal = ratatui::init_with_options(ratatui::TerminalOptions {
        viewport: ratatui::Viewport::Inline(6),
    });
    terminal.clear()?;

    let mut input = String::new();

    let mut font_system = FontSystem::new();
    let swash_cache = SwashCache::new();
    let db = font_system.db_mut();
    db.load_system_fonts();

    let lowercase_fonts: BTreeMap<String, String> = db
        .faces()
        .map(|faceinfo| {
            (
                faceinfo.families[0].0.to_ascii_lowercase(),
                faceinfo.families[0].0.clone(),
            )
        })
        .collect();

    let mut last_rendered: Option<(String, Protocol)> = None;
    let mut inner_width = 0;

    let mut renderer =
        FontRenderer::new(font_system, swash_cache, String::new(), picker.font_size());

    loop {
        let first_match = find_first_match(&lowercase_fonts, &input.to_ascii_lowercase());

        terminal.draw(|f| {
            let area = f.area();
            let block = ratatui::widgets::Block::default()
                .title("Enter font name (Tab: complete, Esc: abort, Enter: confirm):")
                .borders(ratatui::widgets::Borders::ALL);
            let inner_area = block.inner(area);
            inner_width = inner_area.width;
            f.render_widget(block, area);

            if let Some(first_match) = first_match.clone() {
                f.render_widget(
                    Paragraph::new(Line::from(first_match).dark_gray()),
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
            if let Some(first_match) = first_match {
                renderer.font_name = first_match.clone();
                let spans = vec!["The fox jumped over the goat or something".into()];
                let dyn_imgs = header_images(
                    bg,
                    &mut renderer,
                    inner_width,
                    Line::from(spans).to_string(),
                    1,
                    false,
                )?;
                let sources = header_sources(picker, inner_width, 0, dyn_imgs, false)?;

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
                        if let Some(family) =
                            find_first_match(&lowercase_fonts, &input.to_ascii_lowercase())
                        {
                            input = family;
                        }
                    }
                    KeyCode::Enter => {
                        if let Some(family) =
                            find_first_match(&lowercase_fonts, &input.to_ascii_lowercase())
                        {
                            terminal.clear()?;
                            ratatui::restore();
                            return Ok(Some(family));
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

fn find_first_match(all_fonts: &BTreeMap<String, String>, input: &str) -> Option<String> {
    let mut first_match = None;
    if !input.is_empty() {
        for (lowercase_pattern, pattern) in all_fonts {
            if lowercase_pattern.starts_with(input) {
                first_match = Some(pattern.clone());
                break;
            }
        }
    } else {
        first_match = all_fonts.first_key_value().map(|t| t.1).cloned();
    };
    first_match
}
