use std::{
    path::PathBuf,
    sync::{OnceLock, mpsc::Sender},
    thread,
    time::Duration,
};

use flexi_logger::{FileSpec, FlexiLoggerError, Logger, LoggerHandle};
use log::LevelFilter;

use crate::Event;

#[derive(Clone, Debug, Default)]
pub enum LogTarget {
    #[default]
    None,
    Stderr,
    Path(PathBuf),
}

impl From<Option<&String>> for LogTarget {
    fn from(value: Option<&String>) -> Self {
        match value {
            None => LogTarget::None,
            Some(s) if s.is_empty() => LogTarget::Stderr,
            Some(s) => LogTarget::Path(PathBuf::from(s)),
        }
    }
}

static LOGGER: OnceLock<LoggerHandle> = OnceLock::new();

pub fn init_logger(log_target: LogTarget) -> Result<(), FlexiLoggerError> {
    let logger = match log_target {
        LogTarget::None => Logger::with(LevelFilter::Off).do_not_log().start(),
        LogTarget::Stderr => Logger::try_with_env_or_str("debug")?
            .duplicate_to_stderr(flexi_logger::Duplicate::All)
            .start(),
        LogTarget::Path(path) => Logger::try_with_env()?
            .log_to_file(FileSpec::try_from(path)?)
            .start(),
    }?;
    if let Err(_logger) = LOGGER.set(logger) {
        panic!("error initializing global logger: already initialized.");
    }
    Ok(())
}

#[cfg(test)]
pub fn init_test_logger() {
    use flexi_logger::WriteMode;

    #[expect(clippy::let_underscore_untyped, clippy::unwrap_used)]
    let _ = Logger::try_with_env()
        .unwrap()
        .log_to_stderr()
        .write_mode(WriteMode::Direct)
        .start()
        .ok();
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
