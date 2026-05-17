use std::{
    sync::{OnceLock, mpsc::Sender},
    thread,
    time::Duration,
};

use flexi_logger::{FlexiLoggerError, Logger, LoggerHandle};
use log::LevelFilter;

use crate::Event;

static LOGGER: OnceLock<LoggerHandle> = OnceLock::new();

pub fn init_logger(log_to_stderr: bool) -> Result<(), FlexiLoggerError> {
    let logger = if log_to_stderr {
        Logger::try_with_env_or_str("info")?
            .duplicate_to_stderr(flexi_logger::Duplicate::All)
            .start()
    } else {
        Logger::with(LevelFilter::Off).do_not_log().start()
    }?;
    if let Err(_logger) = LOGGER.set(logger) {
        panic!("error initializing global logger: already initialized.");
    }
    Ok(())
}

#[cfg(test)]
pub fn init_test_logger() {
    #[expect(clippy::let_underscore_untyped, clippy::unwrap_used)]
    let _ = Logger::try_with_env()
        .unwrap()
        .log_to_stderr()
        .start()
        .inspect_err(|err| eprintln!("test logger setup failed: {err}"));
}

#[cfg(not(windows))]
pub fn animate_recording(event_tx: Sender<Event>) {
    use std::sync::atomic::{AtomicBool, Ordering};

    static ANIMATE: AtomicBool = AtomicBool::new(false);

    extern "C" fn handle_sigusr1(_: libc::c_int) {
        ANIMATE.store(true, Ordering::Relaxed);
    }

    // SAFETY:
    // This is just for the demo video recording, under the `--animate` hidden flag.
    unsafe {
        libc::signal(libc::SIGUSR1, handle_sigusr1 as libc::sighandler_t);
    }

    thread::spawn(move || -> ! {
        loop {
            log::warn!("animate_recording thread waiting on SIGUSR1");
            while !ANIMATE.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(100));
            }

            // ease-in-out-ish movement curve
            let steps: &[i16] = &[1, 1, 2, 2, 3, 4, 5, 5, 5, 4, 3, 2, 1];

            for delta in steps {
                event_tx
                    .send(Event::Scroll(*delta))
                    .expect("can send Scroll");

                thread::sleep(Duration::from_millis(
                    20 * (7_u64.saturating_sub(*delta as u64)),
                ));
            }

            thread::sleep(Duration::from_millis(500));

            for delta in steps {
                event_tx
                    .send(Event::Scroll(-delta))
                    .expect("can send Scroll");

                thread::sleep(Duration::from_millis(
                    20 * (7_u64.saturating_sub(*delta as u64)),
                ));
            }

            ANIMATE.store(false, Ordering::Relaxed);
        }
    });
}
