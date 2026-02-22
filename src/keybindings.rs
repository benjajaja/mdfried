use std::{
    num::{NonZero, NonZeroU16},
    time::Duration,
};

use ratatui::{
    crossterm::event::{self, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEventKind},
    layout::Size,
};

use crate::{
    Error,
    cursor::{Cursor, CursorPointer},
    document::{LineExtra, SectionContent},
    model::{InputQueue, Model},
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
        match event::read()? {
            event::Event::Key(key) => {
                if key.kind == KeyEventKind::Press {
                    return match_keycode(key, model);
                }
            }
            event::Event::Resize(new_width, new_height) => {
                log::debug!("Resize {new_width},{new_height}");
                if model.screen_size.width != new_width || model.screen_size.height != new_height {
                    let screen_size = Size::new(new_width, new_height);
                    model.reload(screen_size)?;
                }
                return Ok(PollResult::HadInput);
            }
            event::Event::Mouse(mouse) => match mouse.kind {
                MouseEventKind::ScrollUp => {
                    model.scroll_by(-2);
                    return Ok(PollResult::HadInput);
                }
                MouseEventKind::ScrollDown => {
                    model.scroll_by(2);
                    return Ok(PollResult::HadInput);
                }
                _ => {}
            },
            _ => {}
        }
    }
    Ok(PollResult::None)
}

fn match_keycode(key: KeyEvent, model: &mut Model) -> Result<PollResult, Error> {
    // TODO: adjust padding or whatever properly
    let page_scroll_count = model.inner_height(model.screen_size.height) as i32 - 2;

    match key.code {
        // Search-input mode captures any `KeyCode::Char(_)`.
        KeyCode::Char(c) if matches!(model.input_queue, InputQueue::Search(_)) => {
            // We could also match over (key.code, model.input_queue).
            let InputQueue::Search(needle) = &mut model.input_queue else {
                panic!("invariant InputQueue::Search");
            };
            needle.push(c);
            let clone = needle.clone();
            model.add_searches(Some(&clone));
            model.cursor = Cursor::Search(clone, None);
        }
        // Digits start a movement-count.
        KeyCode::Char(x)
            if x.is_ascii_digit() && (model.input_queue != InputQueue::None || x != '0') =>
        {
            let x = x as u16 - '0' as u16;
            match &mut model.input_queue {
                InputQueue::None => {
                    model.input_queue =
                        InputQueue::MovementCount(NonZero::new(x).expect("is_ascii_digit"));
                }
                InputQueue::MovementCount(count) => {
                    *count = count
                        .saturating_mul(NonZero::new(10).expect("10 != 0"))
                        .saturating_add(x);
                }
                InputQueue::Search(_) => {
                    panic!("invariant is_ascii_digit while in search");
                }
            }
        }
        // Ways to quit
        KeyCode::Char('q') => {
            return Ok(PollResult::Quit);
        }
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            return Ok(PollResult::Quit);
        }
        // Movements
        KeyCode::Char('j') | KeyCode::Down => {
            let count = model.input_queue.take_count_or_unit_i32();
            model.scroll_by(count);
        }
        KeyCode::Char('k') | KeyCode::Up => {
            let count = model.input_queue.take_count_or_unit_i32();
            model.scroll_by(-count);
        }
        KeyCode::Char('d') => {
            let count = model.input_queue.take_count_or_unit_i32();
            model.scroll_by(((page_scroll_count + 1) / 2).saturating_mul(count));
        }
        KeyCode::Char('u') => {
            let count = model.input_queue.take_count_or_unit_i32();
            model.scroll_by((-(page_scroll_count + 1) / 2).saturating_mul(count));
        }
        KeyCode::Char('f' | ' ') | KeyCode::PageDown => {
            let count = model.input_queue.take_count_or_unit_i32();
            model.scroll_by(page_scroll_count.saturating_mul(count));
        }
        KeyCode::Char('b') | KeyCode::PageUp => {
            let count = model.input_queue.take_count_or_unit_i32();
            model.scroll_by((-page_scroll_count).saturating_mul(count));
        }
        KeyCode::Char('g') => {
            let scroll = if let InputQueue::MovementCount(count) = model.input_queue {
                model.input_queue = InputQueue::None;
                count.get()
            } else {
                0
            };
            model.scroll = scroll;
        }
        KeyCode::Char('G') => {
            let scroll = if let InputQueue::MovementCount(count) = model.input_queue {
                model.input_queue = InputQueue::None;
                count.get()
            } else {
                model.total_lines().saturating_sub(
                    page_scroll_count as u16 + 1, // Why +1?
                )
            };
            model.scroll = scroll;
        }
        // Cursor movements
        KeyCode::Char('n') => {
            let count = model.input_queue.take_count_or_unit_u16();
            model.cursor_next(count);
        }
        KeyCode::Char('N') => {
            let count = model.input_queue.take_count_or_unit_u16();
            model.cursor_prev(count);
        }
        // Others
        KeyCode::Char('r') => {
            model.reload(model.screen_size)?;
        }
        KeyCode::Char('/') => {
            model.input_queue = InputQueue::Search(String::new());
            model.cursor = Cursor::Search(String::new(), None);
        }
        KeyCode::F(11) => {
            model.log_snapshot = match model.log_snapshot {
                None => Some(flexi_logger::Snapshot::new()),
                Some(_) => None,
            };
        }
        KeyCode::Enter if matches!(model.input_queue, InputQueue::Search(_)) => {
            // Exit search...
            model.input_queue = InputQueue::None;
            // ...and jump to first match.
            model.cursor_next(1);
        }
        KeyCode::Enter => {
            // Open links with xdg-open
            if let Cursor::Links(CursorPointer { id, index }) = model.cursor {
                let url = model.sections().find_map(|section| {
                    if section.id == id {
                        let SectionContent::Lines(lines) = &section.content else {
                            return None;
                        };
                        let mut remaining = index;
                        for (_, extras) in lines {
                            if remaining < extras.len() {
                                return match &extras[remaining] {
                                    LineExtra::Link(url, _, _) => Some(url.clone()),
                                    _ => None,
                                };
                            }
                            remaining -= extras.len();
                        }
                        None
                    } else {
                        None
                    }
                });
                if let Some(url) = url {
                    log::debug!("open link_cursor {}", *url);
                    model.open_link(url.to_string())?;
                }
            }
        }
        KeyCode::Esc => match model.input_queue {
            InputQueue::None => {
                // This is not vim-canon: Esc is the equivalent of `:noh`. This works because we
                // don't have any real "modes".
                match &model.cursor {
                    Cursor::Search(_, _) | Cursor::Links(_) => {
                        model.cursor = Cursor::None;
                    }
                    _ => {}
                }
            }
            InputQueue::MovementCount(_) => {
                // Abort movement-count input.
                model.input_queue = InputQueue::None;
            }
            InputQueue::Search(_) => {
                // Abort search input.
                model.input_queue = InputQueue::None;
                model.cursor = Cursor::None;
            }
        },
        KeyCode::Backspace => match &mut model.input_queue {
            // Edit input queue.
            InputQueue::None => {}
            InputQueue::MovementCount(count) => {
                let value = count.get();
                if value > 10 {
                    *count = NonZeroU16::new(count.get() / 10).expect("checked >10");
                }
            }
            InputQueue::Search(needle) => {
                if needle.is_empty() {
                    model.input_queue = InputQueue::None;
                    model.cursor = Cursor::None;
                } else {
                    needle.pop();
                    let clone = needle.clone();
                    model.add_searches(Some(&clone));
                }
            }
        },
        _ => {
            return Ok(PollResult::None);
        }
    }
    Ok(PollResult::HadInput)
}
