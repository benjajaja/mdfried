use std::{
    sync::mpsc::{self, TrySendError},
    thread,
};

use ratatui::{DefaultTerminal, buffer::Buffer, layout::Position};

use crate::{
    error::Error,
    keybindings::{self, PollResult},
    model::Model,
    view::view,
};

pub fn run_loop(mut terminal: DefaultTerminal, mut model: Model) -> Result<(), Error> {
    terminal.draw(|frame| view(&model, frame.buffer_mut()))?;
    let (buf_tx, buf_rx) = mpsc::sync_channel::<Buffer>(1);
    let mut terminal = terminal;
    let render_thread = thread::spawn(move || {
        while let Ok(buf) = buf_rx.recv() {
            if let Err(err) = terminal.draw(|frame| {
                let cursor_position = Position::from((0, buf.area.height - 1));
                *frame.buffer_mut() = buf;
                frame.set_cursor_position(cursor_position);
            }) {
                log::error!("draw error: {err}");
            }
        }
        // Cursor might be in wird places, prompt or whatever should always show at the bottom now.
        if let Ok(size) = terminal.size() {
            if let Err(err) = terminal.set_cursor_position((0, size.height - 1)) {
                log::error!("could not set_cursor_position on exit: {err}");
            }
        }
    });
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
            let mut buf = Buffer::empty(model.screen_size.into());
            view(&model, &mut buf);
            if let Err(err) = buf_tx.try_send(buf) {
                match err {
                    TrySendError::Full(_) => {
                        log::warn!("frame dropped");
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
    drop(buf_tx);
    render_thread
        .join()
        .map_err(|err| Error::Thread(format!("{err:?}")))
}
