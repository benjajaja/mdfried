use core::fmt;
use std::{
    error::Error as _,
    io,
    sync::{PoisonError, mpsc::SendError},
};

use color_eyre::eyre::InstallError;
use confy::ConfyError;
use flexi_logger::FlexiLoggerError;
use image::ImageError;
use tokio::task::JoinError;

use crate::{Cmd, WidthEvent, config, setup::FontRenderer};

#[derive(Debug)]
pub enum Error {
    Usage(Option<&'static str>),
    UserAbort(&'static str),
    Cli(clap::error::Error),
    Logger(FlexiLoggerError),
    Config(String, ConfyError),
    Io(io::Error),
    Parse(&'static str),
    Image(ImageError),
    Protocol(ratatui_image::errors::Errors),
    Download(reqwest::Error),
    NoFont,
    Thread,
    UnknownImage(usize, String),
    // Do not overuse this one!
    Generic(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Usage(_) => write!(f, "Bad arguments"), // Never shown to user, just a signal.
            Error::UserAbort(msg) => write!(f, "Aborted by user ({msg})"),
            Error::Cli(err) => write!(f, "Command line argument error: {err}"),
            Error::Logger(err) => write!(f, "Logger error: {err}"),
            Error::Config(path, err) => {
                write!(
                    f,
                    "Configuration file {path} error: {err} ({})",
                    err.source()
                        .map_or("no additional info".into(), ToString::to_string)
                )
            }
            Error::Io(err) => write!(f, "I/O error: {err}"),
            Error::Parse(msg) => write!(f, "Parse error: {msg}"),
            Error::Image(err) => write!(f, "Image manipulation error: {err}"),
            Error::Protocol(err) => write!(f, "Terminal graphics error: {err}"),
            Error::Download(err) => write!(f, "HTTP request error: {err}"),
            Error::NoFont => write!(f, "No font available"),
            Error::Thread => write!(f, "Thread error"),
            Error::UnknownImage(_, url) => write!(f, "Unknown image format: {url}"),
            Error::Generic(msg) => write!(f, "Generic error: {msg}"),
        }
    }
}

impl From<Error> for io::Error {
    fn from(value: Error) -> Self {
        match value {
            Error::Io(io_err) => io_err,
            err => io::Error::other(format!("{err:?}")),
        }
    }
}

impl From<io::Error> for Error {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<ImageError> for Error {
    fn from(value: ImageError) -> Self {
        Self::Image(value)
    }
}

impl From<ratatui_image::errors::Errors> for Error {
    fn from(value: ratatui_image::errors::Errors) -> Self {
        Self::Protocol(value)
    }
}

impl From<ConfyError> for Error {
    fn from(value: ConfyError) -> Self {
        Self::Config(
            config::get_configuration_file_path()
                .map_or("(unknown config file path)".into(), |p| {
                    p.display().to_string()
                }),
            value,
        )
    }
}

impl From<clap::error::Error> for Error {
    fn from(value: clap::error::Error) -> Self {
        Self::Cli(value)
    }
}

impl From<reqwest::Error> for Error {
    fn from(value: reqwest::Error) -> Self {
        Self::Download(value)
    }
}

impl From<SendError<WidthEvent<'_>>> for Error {
    fn from(_: SendError<WidthEvent<'_>>) -> Self {
        Self::Thread
    }
}

impl From<SendError<Cmd>> for Error {
    fn from(_: SendError<Cmd>) -> Self {
        Self::Thread
    }
}

impl From<JoinError> for Error {
    fn from(_: JoinError) -> Self {
        Self::Thread
    }
}

impl From<PoisonError<std::sync::MutexGuard<'_, Box<FontRenderer>>>> for Error {
    fn from(_: PoisonError<std::sync::MutexGuard<'_, Box<FontRenderer>>>) -> Self {
        Self::Thread
    }
}

impl From<FlexiLoggerError> for Error {
    fn from(value: FlexiLoggerError) -> Self {
        Self::Logger(value)
    }
}

impl From<InstallError> for Error {
    fn from(value: InstallError) -> Self {
        Self::Generic(format!("{value}"))
    }
}
