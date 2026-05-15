use std::sync::OnceLock;

use flexi_logger::{FlexiLoggerError, Logger, LoggerHandle};
use log::LevelFilter;

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
