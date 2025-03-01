use std::collections::BTreeMap;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use font_kit::{
    family_name::FamilyName, handle::Handle, properties::Properties,
    sources::fontconfig::FontconfigSource,
};
use ratatui::{style::Stylize, text::Line, widgets::Paragraph};
use ratatui_image::{picker::Picker, protocol::Protocol, Image};

use crate::{
    setup::Renderer,
    widget_sources::{header_source, WidgetSourceData},
    Error,
};

pub fn interactive_font_picker(
    source: &FontconfigSource,
    picker: &mut Picker,
    bg: Option<[u8; 4]>,
) -> Result<Option<(Handle, String)>, Error> {
    let mut terminal = ratatui::init_with_options(ratatui::TerminalOptions {
        viewport: ratatui::Viewport::Inline(6),
    });
    terminal.clear()?;

    let mut input = String::new();

    let lowercase_fonts: BTreeMap<String, String> = source
        .all_families()?
        .into_iter()
        .map(|family_name| (family_name.to_ascii_lowercase(), family_name))
        .collect();

    let mut last_rendered: Option<(String, Protocol)> = None;
    let mut inner_width = 0;

    loop {
        let first_match = find_first_match(&source, &lowercase_fonts, &input.to_ascii_lowercase())?;

        let (font, first_match) = if let Some((font, pattern)) = first_match {
            (Some(font), Some(pattern))
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
                let name = format!("{first_match:?}");
                f.render_widget(Paragraph::new(Line::from(name).dark_gray()), inner_area);
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
                // let spans = vec!["The fox jumped over the goat or something".into()];
                let spans = vec!["The fox".into()];
                let sources = header_source(
                    bg,
                    picker.font_size(),
                    font.load().unwrap(),
                    picker,
                    // &Renderer::new(*picker, font.load()?, bg),
                    inner_width,
                    0,
                    Line::from(spans).to_string(),
                    1,
                    false,
                )?;

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
                        if let Some(handle) = find_first_match(
                            &source,
                            &lowercase_fonts,
                            &input.to_ascii_lowercase(),
                        )? {
                            input = format!("{handle:?}");
                        }
                    }
                    KeyCode::Enter => {
                        if let Some(font) = find_first_match(
                            &source,
                            &lowercase_fonts,
                            &input.to_ascii_lowercase(),
                        )? {
                            terminal.clear()?;
                            ratatui::restore();
                            return Ok(Some(font));
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

fn find_first_match<'a>(
    source: &FontconfigSource,
    all_fonts: &'a BTreeMap<String, String>,
    input: &str,
) -> Result<Option<(Handle, String)>, Error> {
    let mut first_match = None;
    if !input.is_empty() {
        for (lowercase_pattern, pattern) in all_fonts {
            if lowercase_pattern.starts_with(input) {
                first_match = Some((
                    source.select_best_match(
                        &[FamilyName::Title(pattern.clone())],
                        &Properties::new(),
                    )?,
                    pattern.clone(),
                ));
                break;
            }
        }
    } else {
        if let Some((_, pattern)) = all_fonts.first_key_value() {
            first_match = Some((
                source
                    .select_best_match(&[FamilyName::Title(pattern.clone())], &Properties::new())?,
                pattern.clone(),
            ))
        }
    };
    Ok(first_match)
}
