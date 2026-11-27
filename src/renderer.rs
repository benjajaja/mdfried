use std::{
    io::{self, Write as _},
    sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
    thread,
    time::{Duration, Instant},
};

use ratatui::{
    DefaultTerminal,
    buffer::Buffer,
    layout::{Position, Rect},
    style::{Color as RColor, Modifier, Style},
};

use crate::{
    error::Error,
    keybindings::{self, PollResult},
    model::Model,
    view::view,
};

/// In dump mode, give up waiting for pending images and headers when no more of the document has
/// settled for this long.
const DUMP_STALL_LIMIT: Duration = Duration::from_secs(5);

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
/// Output is progressive: lines are written as soon as everything before them has settled, so a
/// slow image only holds back the lines after it. Gives up on pending content after
/// `DUMP_STALL_LIMIT` without progress.
///
/// Writes buffer rows sequentially to stdout — no cursor-position queries, no raw mode required.
/// Works with both TTY and redirected stdout.
pub fn dump(mut model: Model) -> Result<(), Error> {
    let mut out = io::BufWriter::new(io::stdout());
    let mut emitted = 0_u16;
    let mut parsed = false;
    let mut deadline = Instant::now() + DUMP_STALL_LIMIT;
    loop {
        let (_, had_done, _) = model.process_events()?;
        parsed |= had_done;
        let done = parsed && !model.has_pending_content();

        // Output is append-only, so only write lines that can no longer change, which is
        // everything before the first section the worker is still working on.
        let target = if done {
            model.total_lines()
        } else {
            model.settled_lines()
        };
        if target > emitted {
            dump_lines(&mut model, &mut out, emitted, target)?;
            emitted = target;
            deadline = Instant::now() + DUMP_STALL_LIMIT;
        }

        if done {
            break;
        }
        if Instant::now() > deadline {
            log::warn!("dump stalled for {DUMP_STALL_LIMIT:?}, giving up on pending content");
            // Write the rest as it is, including unloaded placeholders.
            let total = model.total_lines();
            if total > emitted {
                dump_lines(&mut model, &mut out, emitted, total)?;
                emitted = total;
            }
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }

    if emitted > 0 {
        use ratatui::crossterm::{queue, style::ResetColor};
        queue!(out, ResetColor)?;
        // Big headers disable autowrap (DECAWM) while they are written; don't leave the user's
        // shell like that. Harmless if it was already enabled.
        out.write_all(b"\x1b[?7h")?;
    }
    out.flush()?;
    Ok(())
}

/// Write document lines `from..to` (as a window of the document, by scroll offset) to `out`, one
/// screenful at a time, and flush.
fn dump_lines(
    model: &mut Model,
    out: &mut impl io::Write,
    from: u16,
    to: u16,
) -> Result<(), Error> {
    let width = model.screen_size.width;
    // Full buffer height lets view() place the status bar at the last row, which we skip.
    let full_height = model.screen_size.height.max(2);
    let content_height = model.inner_height().max(1);

    let mut scroll = from;
    while scroll < to {
        // Pages must not cut big headers in two, or they are not drawn at all.
        let page_end = model.page_end(scroll, content_height).min(to);
        let page_lines = page_end - scroll;
        let mut buf = Buffer::empty(Rect::new(0, 0, width, full_height));
        model.scroll = scroll;
        view(model, &mut buf);
        write_buffer_rows(&buf, page_lines, out)?;
        scroll += page_lines;
    }
    out.flush()?;
    Ok(())
}

/// Write the first `rows` rows of `buf` to `out` as styled ANSI lines, one `\n` per row.
///
/// Cells are written like the ratatui backend would write them when diffing: a `Skip` cell is not
/// written at all (it is covered by an escape sequence in an earlier cell, such as a big header
/// or an image), and a cell wider than one column (`ForcedWidth`, e.g. an OSC 8 link, or a wide
/// character) covers the following columns.
fn write_buffer_rows(buf: &Buffer, rows: u16, out: &mut impl io::Write) -> io::Result<()> {
    use ratatui::{
        buffer::{CellDiffOption, CellWidth as _},
        crossterm::{
            cursor::MoveToColumn,
            queue,
            style::{
                Attribute, Print, ResetColor, SetAttribute, SetAttributes, SetBackgroundColor,
                SetForegroundColor,
            },
        },
        prelude::IntoCrossterm as _,
    };

    let area = *buf.area();
    for y in 0..rows.min(area.height) {
        let mut prev_fg = RColor::Reset;
        let mut prev_bg = RColor::Reset;
        let mut prev_mod = Modifier::empty();
        // After skipping cells, the cursor is wherever the escape sequence left it, so the next
        // written cell must be positioned explicitly (see below).
        let mut skipped = false;
        let mut x = 0;
        while x < area.width {
            let cell = &buf[(x, y)];
            if matches!(cell.diff_option, CellDiffOption::Skip) {
                skipped = true;
                x += 1;
                continue;
            }
            if skipped {
                queue!(out, MoveToColumn(x))?;
                skipped = false;
            }
            if cell.modifier != prev_mod {
                // Attribute::Reset clears colors too, so re-apply them after.
                queue!(out, SetAttribute(Attribute::Reset))?;
                prev_fg = RColor::Reset;
                prev_bg = RColor::Reset;
                prev_mod = cell.modifier;
                let style = Style::new().add_modifier(cell.modifier);
                queue!(out, SetAttributes(style.into_crossterm().attributes))?;
            }
            if cell.fg != prev_fg {
                queue!(out, SetForegroundColor(cell.fg.into_crossterm()))?;
                prev_fg = cell.fg;
            }
            if cell.bg != prev_bg {
                queue!(out, SetBackgroundColor(cell.bg.into_crossterm()))?;
                prev_bg = cell.bg;
            }
            queue!(out, Print(cell.symbol()))?;
            let width = cell.cell_width().max(1);
            // Like the ratatui backend, don't trust where the cursor ends up after a cell that
            // is not exactly one column wide, because it may contain an escape sequence.
            skipped |= width != 1;
            x += width;
        }
        queue!(out, ResetColor, Print("\n"))?;
    }
    Ok(())
}

#[cfg(test)]
#[expect(clippy::unwrap_used)]
mod tests {
    use std::num::NonZeroU16;

    use ratatui::{
        buffer::{Buffer, CellDiffOption},
        layout::Rect,
    };

    use super::write_buffer_rows;

    fn rows(buf: &Buffer, rows: u16) -> String {
        let mut out = Vec::new();
        write_buffer_rows(buf, rows, &mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn plain_cells() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 4, 1));
        buf.set_string(0, 0, "abcd", ratatui::style::Style::new());
        assert_eq!(rows(&buf, 1), "abcd\x1b[0m\n");
    }

    /// The covered cells of a multi-column cell, such as an OSC 8 link, are not written again.
    #[test]
    fn forced_width_cell_covers_following_cells() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 8, 1));
        buf.set_string(0, 0, "Xeleaseyz", ratatui::style::Style::new());
        buf[(1, 0)].set_symbol("e");
        buf[(0, 0)]
            .set_symbol("<link>")
            .set_diff_option(CellDiffOption::ForcedWidth(NonZeroU16::new(7).unwrap()));
        assert_eq!(rows(&buf, 1), "<link>\x1b[8Gy\x1b[0m\n");
    }

    /// Skipped cells (covered by an escape sequence in an earlier cell, such as a big header) are
    /// not written, and a later cell is positioned explicitly.
    #[test]
    fn skipped_cells_are_not_written() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 6, 2));
        buf[(0, 0)].set_symbol("<big>");
        for y in 0..2 {
            for x in 0..6 {
                if (x, y) != (0, 0) {
                    buf[(x, y)]
                        .set_symbol("#")
                        .set_diff_option(CellDiffOption::Skip);
                }
            }
        }
        buf[(5, 0)]
            .set_symbol("z")
            .set_diff_option(CellDiffOption::None);
        assert_eq!(rows(&buf, 2), "<big>\x1b[6Gz\x1b[0m\n\x1b[0m\n");
    }

    /// A wide character occupies two columns, so the following column is not written.
    #[test]
    fn wide_char_covers_following_cell() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 4, 1));
        buf.set_string(0, 0, "コa", ratatui::style::Style::new());
        assert_eq!(rows(&buf, 1), "コ\x1b[3Ga \x1b[0m\n");
    }
}
