use std::{
    sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
    thread,
};

use ratatui::{DefaultTerminal, buffer::Buffer, layout::Position};

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
    terminal.draw(|frame| view(&model, frame.buffer_mut()))?;

    // Send the buffer back and forth to avoid allocating every frame, also serves as "dropped"
    // signal, when not returned already.
    let (buf_in_tx, buf_in_rx) = mpsc::sync_channel::<Buffer>(1);
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

            view(&model, &mut buf);
            if let Err(err) = buf_in_tx.try_send(buf) {
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
        .map_err(|err| Error::Thread(format!("{err:?}")))?
}

fn render(
    mut terminal: DefaultTerminal,
    buf_in: Receiver<Buffer>,
    buf_out: SyncSender<Buffer>,
) -> Result<(), Error> {
    while let Ok(mut buf) = buf_in.recv() {
        terminal.draw(|frame| {
            let cursor_position = Position::from((0, buf.area.height - 1));
            std::mem::swap(frame.buffer_mut(), &mut buf);
            frame.set_cursor_position(cursor_position);
        })?;
        // Guaranteed to be empty since we hold the buffer.
        buf_out
            .send(buf)
            .map_err(|err| Error::Thread(format!("could not return buffer: {err}")))?;
    }
    // Cursor might be in weird places, prompt or whatever should always show at the bottom now.
    Ok(terminal.set_cursor_position((0, terminal.size()?.height - 1))?)
}
