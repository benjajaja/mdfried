use std::{
    num::{NonZero, NonZeroU16},
    time::Duration,
};

use ratatui::{
    crossterm::event::{self, KeyCode, KeyEventKind, KeyModifiers, MouseEventKind},
    layout::Size,
};

use crate::{
    Error,
    cursor::{Cursor, CursorPointer, SearchState},
    document::{LineExtra, SectionContent},
    model::Model,
};

pub enum PollResult {
    None,
    HadInput,
    Quit,
}

pub fn poll(had_events: bool, model: &mut Model) -> Result<PollResult, Error> {
    if event::poll(if had_events {
        Duration::ZERO
    } else {
        Duration::from_millis(100)
    })? {
        let page_scroll_count = model.inner_height(model.screen_size.height) as i32 - 2;
        match event::read()? {
            event::Event::Key(key) => {
                if key.kind == KeyEventKind::Press {
                    match model.cursor {
                        Cursor::Search(ref mut mode, _) if !mode.accepted => match key.code {
                            KeyCode::Char(c) => {
                                let mut needle = std::mem::take(&mut mode.needle);
                                needle.push(c);
                                model.add_searches(Some(&needle));
                                let Cursor::Search(mode, _) = &mut model.cursor else {
                                    unreachable!("model.add_searches should not modify cursor");
                                };
                                mode.needle = needle;
                            }
                            KeyCode::Backspace => {
                                let mut needle = std::mem::take(&mut mode.needle);
                                needle.pop();
                                model.add_searches(Some(&needle));
                                let Cursor::Search(mode, _) = &mut model.cursor else {
                                    unreachable!("model.add_searches should not modify cursor");
                                };
                                mode.needle = needle;
                            }
                            KeyCode::Esc if model.movement_count.is_none() => {
                                model.cursor = Cursor::None;
                            }
                            KeyCode::Esc => {
                                model.movement_count = None;
                            }
                            KeyCode::Enter => {
                                mode.accepted = true;
                                model.cursor_next();
                            }
                            _ => {}
                        },
                        _ => {
                            match key.code {
                                KeyCode::Char('q') => {
                                    return Ok(PollResult::Quit);
                                }
                                KeyCode::Char('c')
                                    if key.modifiers.contains(KeyModifiers::CONTROL) =>
                                {
                                    return Ok(PollResult::Quit);
                                }
                                KeyCode::Char('r') => {
                                    model.reload(model.screen_size)?;
                                }
                                KeyCode::Char('j') | KeyCode::Down => {
                                    model.scroll_by(1);
                                }
                                KeyCode::Char('k') | KeyCode::Up => {
                                    model.scroll_by(-1);
                                }
                                KeyCode::Char('d') => {
                                    model.scroll_by((page_scroll_count + 1) / 2);
                                }
                                KeyCode::Char('u') => {
                                    model.scroll_by(-(page_scroll_count + 1) / 2);
                                }
                                KeyCode::Char('f' | ' ') | KeyCode::PageDown => {
                                    model.scroll_by(page_scroll_count);
                                }
                                KeyCode::Char('b') | KeyCode::PageUp => {
                                    model.scroll_by(-page_scroll_count);
                                }
                                KeyCode::Char('G') if model.movement_count.is_none() => {
                                    model.scroll = model.total_lines().saturating_sub(
                                        page_scroll_count as u16 + 1, // Why +1?
                                    );
                                }
                                KeyCode::Char('g' | 'G') => {
                                    model.scroll =
                                        model.movement_count.take().map_or(1, NonZero::get);
                                    model.scroll_by(0);
                                }
                                KeyCode::Char('/') => {
                                    model.cursor = Cursor::Search(SearchState::default(), None);
                                }
                                KeyCode::Char('n') => {
                                    model.cursor_next();
                                }
                                KeyCode::Char('N') => {
                                    model.cursor_prev();
                                }
                                KeyCode::F(11) => {
                                    model.log_snapshot = match model.log_snapshot {
                                        None => Some(flexi_logger::Snapshot::new()),
                                        Some(_) => None,
                                    };
                                }
                                KeyCode::Enter => {
                                    if let Cursor::Links(CursorPointer { id, index }) = model.cursor
                                    {
                                        let url = model.sections().find_map(|section| {
                                            if section.id == id {
                                                let SectionContent::Line(_, extras) =
                                                    &section.content
                                                else {
                                                    return None;
                                                };

                                                match extras.get(index) {
                                                    Some(LineExtra::Link(url, _, _)) => {
                                                        Some(url.clone())
                                                    }
                                                    _ => None,
                                                }
                                            } else {
                                                None
                                            }
                                        });
                                        if let Some(url) = url {
                                            log::debug!("open link_cursor {url}");
                                            model.open_link(url)?;
                                        }
                                    }
                                }
                                KeyCode::Esc if model.movement_count.is_none() => {
                                    if let Cursor::Search(SearchState { accepted, .. }, _) =
                                        model.cursor
                                        && accepted
                                    {
                                        model.cursor = Cursor::None;
                                    } else if let Cursor::Links(_) = model.cursor {
                                        model.cursor = Cursor::None;
                                    }
                                }
                                KeyCode::Esc => {
                                    model.movement_count = None;
                                }
                                KeyCode::Backspace => {
                                    model.movement_count = model
                                        .movement_count
                                        .and_then(|x| NonZeroU16::new(x.get() / 10));
                                }
                                KeyCode::Char(x) if x.is_ascii_digit() => {
                                    let x = x as u16 - '0' as u16;
                                    model.movement_count = model
                                        .movement_count
                                        .map(|value| {
                                            value
                                                .saturating_mul(NonZero::new(10).expect("10 != 0"))
                                                .saturating_add(x)
                                        })
                                        .or(NonZero::new(x));
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
            event::Event::Resize(new_width, new_height) => {
                log::debug!("Resize {new_width},{new_height}");
                if model.screen_size.width != new_width || model.screen_size.height != new_height {
                    let screen_size = Size::new(new_width, new_height);
                    model.reload(screen_size)?;
                }
            }
            event::Event::Mouse(mouse) => match mouse.kind {
                MouseEventKind::ScrollUp => {
                    model.scroll_by(-2);
                }
                MouseEventKind::ScrollDown => {
                    model.scroll_by(2);
                }
                _ => {}
            },
            _ => {}
        }
        return Ok(PollResult::HadInput);
    }
    Ok(PollResult::None)
}

// pub enum KeypressEvent {
// Quit,
// Reload(Option<Size>),
// OpenDebug,
// Movement(KeypressEventMovement),
// Search(KeypressEventSearch),
// Move(KeypressEventMove),
// }
//
// pub enum KeypressEventMove {
// Down,
// Up,
// HalfPageDown,
// HalfPageUp,
// PageDown,
// PageUp,
// End,
// Home,
// }
//
// pub enum KeypressEventMovement {
// Push(u16),
// Pop,
// Clear,
// }
//
// pub enum KeypressEventSearch {
// Edit(KeypressEventEditSearch),
// Next,
// Prev,
// }
//
// pub enum KeypressEventEditSearch {
// Push(char),
// Pop,
// Clear,
// Accept,
// }
