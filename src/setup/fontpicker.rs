use std::{collections::BTreeMap, io};

use cosmic_text::{FontSystem, SwashCache};
use ratatui::{
    Terminal, TerminalOptions,
    crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    layout::Rect,
    prelude::CrosstermBackend,
    style::{Color, Stylize as _},
    text::Line,
    widgets::{Padding, Paragraph},
};
use ratatui_image::{Image, picker::Picker, protocol::Protocol};

use crate::{
    Error,
    document::{SectionContent, header_images, header_sections},
    setup::FontRenderer,
};

#[expect(clippy::too_many_lines)]
pub fn interactive_font_picker(picker: &mut Picker) -> Result<Option<String>, Error> {
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

    if lowercase_fonts.is_empty() {
        return Err(Error::NoFont);
    }

    let mut last_rendered: Option<(String, Protocol)> = None;
    let mut inner_width = 0;

    let mut renderer =
        FontRenderer::new(font_system, swash_cache, String::new(), picker.font_size());

    println!("{} system fonts detected.", lowercase_fonts.len());

    ratatui::crossterm::terminal::enable_raw_mode()?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::with_options(
        backend,
        TerminalOptions {
            viewport: ratatui::Viewport::Inline(7),
        },
    )?;
    terminal.clear()?;

    loop {
        let first_match = find_first_match(&lowercase_fonts, &input.to_ascii_lowercase());

        terminal.draw(|f| {
            let area = f.area();
            let block = ratatui::widgets::Block::default()
                .title("Enter font name")
                .borders(ratatui::widgets::Borders::ALL)
                .border_style(Color::Yellow)
                .padding(Padding::proportional(1));
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

            f.render_widget(
                Paragraph::new(Line::from("Tab: complete, Esc: abort, Enter: confirm").dark_gray()),
                Rect::new(1, f.area().y + f.area().height - 1, inner_area.width, 1),
            );

            f.set_cursor_position(((input.len()) as u16 + 3, f.area().y + 2));
        })?;

        if inner_width > 0 && first_match.is_some() {
            if let Some(first_match) = first_match {
                if last_rendered.is_none()
                    || last_rendered
                        .as_ref()
                        .is_none_or(|(m, _)| *m != first_match)
                {
                    renderer.font_name.clone_from(&first_match);
                    const SAMPLE: &str = "The fox jumped over the goat or something";
                    let spans = vec![SAMPLE.into()];
                    let dyn_imgs = header_images(
                        &mut renderer,
                        inner_width,
                        Line::from(spans).to_string(),
                        1,
                        false,
                    )?;
                    let sections = header_sections(picker, inner_width, 0, dyn_imgs, false)?;

                    // Just render the first line if it got split.
                    if let Some(section) = sections.into_iter().next() {
                        if let SectionContent::Image(_, proto) = section.content {
                            last_rendered = Some((first_match.clone(), proto));
                        }
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
                    KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                        // Exit on Ctrl-C too
                        terminal.clear()?;
                        ratatui::restore();
                        return Ok(None);
                    }
                    KeyCode::Char(c) => {
                        // Append the character unless a control modifier is pressed
                        if modifiers.is_empty() {
                            input.push(c);
                        } else if modifiers == KeyModifiers::SHIFT {
                            input.push(c.to_ascii_uppercase());
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

fn find_first_match(all_fonts: &BTreeMap<String, String>, input: &str) -> Option<String> {
    let mut first_match = None;
    if input.is_empty() {
        first_match = all_fonts.first_key_value().map(|t| t.1).cloned();
    } else {
        for (lowercase_pattern, pattern) in all_fonts {
            if lowercase_pattern.starts_with(input) {
                first_match = Some(pattern.clone());
                break;
            }
        }
    }
    first_match
}
