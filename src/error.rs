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

use crate::{Cmd, Event, config};

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
    ThreadClosed(ThreadClosedError),
    Thread(String),
    ImageLoad(String, String),
    Notify(notify::Error),
    MarkdownParse,
    UrlParse(Option<url::ParseError>),
    CodeHighlight(String),
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
            Error::Thread(err) => err.fmt(f),
            Error::ThreadClosed(err) => err.fmt(f),
            Error::ImageLoad(url, err) => write!(f, "Image error {url}: {err}"),
            Error::Notify(err) => write!(f, "Watch error: {err}"),
            Error::MarkdownParse => write!(f, "Markdown parsing failed"),
            Error::UrlParse(err) => match err {
                Some(err) => write!(f, "URL parsing failed: {err}"),
                None => write!(f, "URL parsing failed"),
            },
            Error::CodeHighlight(err) => write!(f, "Code highlight error: {err}"),
            Error::Generic(msg) => write!(f, "Generic error: {msg}"),
        }
    }
}

#[derive(Debug)]
pub enum ThreadClosedError {
    SendEvent(SendError<Event>),
    SendCmd(SendError<Cmd>),
}

impl fmt::Display for ThreadClosedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ThreadClosedError::SendEvent(err) => write!(f, "SendEvent: {err}"),
            ThreadClosedError::SendCmd(err) => write!(f, "SendCmd: {err}"),
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

impl From<SendError<Event>> for Error {
    fn from(err: SendError<Event>) -> Self {
        Self::ThreadClosed(ThreadClosedError::SendEvent(err))
    }
}

impl From<SendError<Cmd>> for Error {
    fn from(err: SendError<Cmd>) -> Self {
        Self::ThreadClosed(ThreadClosedError::SendCmd(err))
    }
}

impl From<JoinError> for Error {
    fn from(err: JoinError) -> Self {
        Self::Thread(format!("JoinError: {err}"))
    }
}

impl<T> From<PoisonError<T>> for Error {
    fn from(err: PoisonError<T>) -> Self {
        Self::Thread(format!(
            "PoisonError<{}>: {err}",
            std::any::type_name::<T>()
        ))
    }
}

impl From<FlexiLoggerError> for Error {
    fn from(value: FlexiLoggerError) -> Self {
        Self::Logger(value)
    }
}

impl From<notify::Error> for Error {
    fn from(value: notify::Error) -> Self {
        Self::Notify(value)
    }
}

impl From<InstallError> for Error {
    fn from(value: InstallError) -> Self {
        Self::Generic(format!("{value}"))
    }
}

impl From<mdfrier::MarkdownParseError> for Error {
    fn from(_value: mdfrier::MarkdownParseError) -> Self {
        Self::MarkdownParse
    }
}

impl From<url::ParseError> for Error {
    fn from(value: url::ParseError) -> Self {
        Self::UrlParse(Some(value))
    }
}

impl From<arborium::Error> for Error {
    fn from(value: arborium::Error) -> Self {
        Self::CodeHighlight(format!("{value}"))
    }
}

impl From<ansi_to_tui::Error> for Error {
    fn from(value: ansi_to_tui::Error) -> Self {
        Self::CodeHighlight(format!("{value}"))
    }
}
