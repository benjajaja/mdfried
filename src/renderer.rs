use std::{
    io::{self, Write as _},
    sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
    thread,
};

use ratatui::{
    DefaultTerminal,
    buffer::Buffer,
    layout::{Position, Rect},
    style::{Color as RColor, Modifier},
};

use crate::{
    error::Error,
    keybindings::{self, PollResult},
    model::Model,
    view::view,
};

/// Elm / TEA / The Elm Architecture -like runtime loop.
///
/// 1. `model.process_events()`: The TEA "update".
/// 2. `keybindings::poll()`: Part 2 of TEA "update", mostly user input and some terminal events.
/// 3. `view(&model, &mut buf)`: ELM "view", just writes to a buffer.
/// 4. `terminal.draw()`: ELM runtime renderer.
///
/// Since mdfried makes heavy usage of images, `terminal.draw()` might be slower than usual, so we
/// offload it onto another thread. If the previous frame has not completed rendering and the
/// buffer has not been returned, the frame is dropped and events and inputs are processed again.
///
/// This means, for example, if the user holds down the "scroll down" key, the model's scroll
/// position continuously gets updated regardless of `terminal.draw()` slowness, and the scroll
/// movement speed is consistent, and there is also no other "input buildup".
pub fn run_loop(mut terminal: DefaultTerminal, mut model: Model) -> Result<(), Error> {
    // Quick, say hi!
    terminal.draw(|frame| {
        let cursor_position = view(&model, frame.buffer_mut());
        if let Some(cursor_position) = cursor_position {
            frame.set_cursor_position(cursor_position);
        }
    })?;

    // Send the buffer back and forth to avoid allocating every frame, also serves as "dropped"
    // signal, when not returned already.
    let (buf_in_tx, buf_in_rx) = mpsc::sync_channel::<(Buffer, Option<Position>)>(1);
    let (buf_out_tx, buf_out_rx) = mpsc::sync_channel::<Buffer>(1);
    buf_out_tx
        .send(Buffer::empty(model.screen_size.into()))
        .expect("unreachable: channel has capacity 1");

    let render_thread = thread::Builder::new()
        .name("render".into())
        .spawn(move || -> Result<(), Error> { render(terminal, buf_in_rx, buf_out_tx) })?;

    let mut dropped = false;
    loop {
        let (had_events, _, had_reload) = model.process_events()?;

        let (had_input, skip_render) = match keybindings::poll(had_events, &mut model)? {
            PollResult::Quit => break,
            PollResult::None => (false, false),
            PollResult::HadInput => (true, false),
            PollResult::SkipRender => (true, true),
        };

        let should_render = dropped || ((had_events || had_input) && !skip_render && !had_reload);

        if should_render {
            let mut buf = match buf_out_rx.try_recv() {
                Err(err) => match err {
                    TryRecvError::Disconnected => {
                        log::warn!("no buffer: disconnected");
                        break;
                    }
                    TryRecvError::Empty => {
                        log::warn!("dropping frame");
                        dropped = true;
                        continue;
                    }
                },
                Ok(mut buf) => {
                    buf.resize(model.screen_size.into());
                    buf
                }
            };

            let cursor_position = view(&model, &mut buf);
            if let Err(err) = buf_in_tx.try_send((buf, cursor_position)) {
                match err {
                    TrySendError::Full(_) => {
                        // How did we get here?
                        log::warn!("frame dropped!");
                        dropped = true;
                    }
                    TrySendError::Disconnected(_) => {
                        log::error!("render buffer channel disconnected");
                        break;
                    }
                }
            } else {
                dropped = false;
            }
        }
    }

    drop(buf_in_tx); // Must drop before joining the threads!
    render_thread
        .join()
        .map_err(|err| Error::Thread(format!("{err:?}")))??;

    Ok(())
}

fn render(
    mut terminal: DefaultTerminal,
    buf_in: Receiver<(Buffer, Option<Position>)>,
    buf_out: SyncSender<Buffer>,
) -> Result<(), Error> {
    while let Ok((mut buf, cursor_position)) = buf_in.recv() {
        let mut hide_cursor = false;
        terminal.draw(|frame| {
            std::mem::swap(frame.buffer_mut(), &mut buf);
            // frame.set_cursor_position(cursor_position);
            if let Some(cursor_position) = cursor_position {
                frame.set_cursor_position(cursor_position);
            } else {
                hide_cursor = true;
            }
        })?;
        if hide_cursor {
            terminal.hide_cursor()?;
        }

        // Guaranteed to be empty since we hold the buffer.
        buf_out
            .send(buf)
            .map_err(|err| Error::Thread(format!("could not return buffer: {err}")))?;
    }
    // Cursor might be in weird places, prompt or whatever should always show at the bottom now.
    Ok(terminal.set_cursor_position((0, terminal.size()?.height - 1))?)
}

/// Render the full document to the main screen (no alternate screen, no interaction) and return.
///
/// Writes buffer rows sequentially to stdout — no cursor-position queries, no raw mode required.
/// Works with both TTY and redirected stdout.
pub fn dump(mut model: Model) -> Result<(), Error> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let (_, had_done, _) = model.process_events()?;
        if had_done {
            break;
        }
        if std::time::Instant::now() > deadline {
            break;
        }
        thread::sleep(std::time::Duration::from_millis(10));
    }

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        model.process_events()?;
        if !model.has_pending_content() {
            break;
        }
        if std::time::Instant::now() > deadline {
            break;
        }
        thread::sleep(std::time::Duration::from_millis(100));
    }

    let total = model.total_lines();
    if total == 0 {
        return Ok(());
    }

    let width = model.screen_size.width;
    // Full buffer height lets view() place the status bar at the last row, which we skip.
    let full_height = model.screen_size.height.max(2);
    let content_height = model.inner_height().max(1);

    let mut out = io::BufWriter::new(io::stdout());
    let mut scroll = 0_u16;
    while scroll < total {
        let page_lines = content_height.min(total.saturating_sub(scroll));
        let mut buf = Buffer::empty(Rect::new(0, 0, width, full_height));
        model.scroll = scroll;
        view(&model, &mut buf);
        write_buffer_rows(&buf, page_lines, &mut out)?;
        scroll = scroll.saturating_add(content_height);
    }

    use ratatui::crossterm::{queue, style::ResetColor};
    queue!(out, ResetColor)?;
    out.flush()?;
    Ok(())
}

/// Write the first `rows` rows of `buf` to `out` as styled ANSI lines, one `\n` per row.
fn write_buffer_rows(buf: &Buffer, rows: u16, out: &mut impl io::Write) -> io::Result<()> {
    use ratatui::crossterm::{
        queue,
        style::{
            Attribute, Color, Print, ResetColor, SetAttribute, SetBackgroundColor,
            SetForegroundColor,
        },
    };

    fn to_ct(c: RColor) -> Color {
        match c {
            RColor::Reset => Color::Reset,
            RColor::Black => Color::Black,
            RColor::Red => Color::DarkRed,
            RColor::Green => Color::DarkGreen,
            RColor::Yellow => Color::DarkYellow,
            RColor::Blue => Color::DarkBlue,
            RColor::Magenta => Color::DarkMagenta,
            RColor::Cyan => Color::DarkCyan,
            RColor::Gray => Color::Grey,
            RColor::DarkGray => Color::DarkGrey,
            RColor::LightRed => Color::Red,
            RColor::LightGreen => Color::Green,
            RColor::LightBlue => Color::Blue,
            RColor::LightYellow => Color::Yellow,
            RColor::LightMagenta => Color::Magenta,
            RColor::LightCyan => Color::Cyan,
            RColor::White => Color::White,
            RColor::Indexed(i) => Color::AnsiValue(i),
            RColor::Rgb(r, g, b) => Color::Rgb { r, g, b },
        }
    }

    let area = *buf.area();
    for y in 0..rows.min(area.height) {
        let mut prev_fg = RColor::Reset;
        let mut prev_bg = RColor::Reset;
        let mut prev_mod = Modifier::empty();
        for x in 0..area.width {
            let cell = &buf[(x, y)];
            if cell.modifier != prev_mod {
                // Attribute::Reset clears colors too, so re-apply them after.
                queue!(out, SetAttribute(Attribute::Reset))?;
                prev_fg = RColor::Reset;
                prev_bg = RColor::Reset;
                prev_mod = cell.modifier;
                for (bit, attr) in &[
                    (Modifier::BOLD, Attribute::Bold),
                    (Modifier::DIM, Attribute::Dim),
                    (Modifier::ITALIC, Attribute::Italic),
                    (Modifier::UNDERLINED, Attribute::Underlined),
                    (Modifier::SLOW_BLINK, Attribute::SlowBlink),
                    (Modifier::RAPID_BLINK, Attribute::RapidBlink),
                    (Modifier::REVERSED, Attribute::Reverse),
                    (Modifier::CROSSED_OUT, Attribute::CrossedOut),
                ] {
                    if cell.modifier.contains(*bit) {
                        queue!(out, SetAttribute(*attr))?;
                    }
                }
            }
            if cell.fg != prev_fg {
                queue!(out, SetForegroundColor(to_ct(cell.fg)))?;
                prev_fg = cell.fg;
            }
            if cell.bg != prev_bg {
                queue!(out, SetBackgroundColor(to_ct(cell.bg)))?;
                prev_bg = cell.bg;
            }
            queue!(out, Print(cell.symbol()))?;
        }
        queue!(out, ResetColor, Print("\n"))?;
    }
    Ok(())
}
