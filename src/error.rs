use core::fmt;
use std::{io, path::PathBuf, sync::mpsc::SendError};

use confy::ConfyError;
use image::ImageError;
use tokio::task::JoinError;

use crate::{ImgCmd, ParseCmd, WidthEvent};

#[derive(Debug)]
#[allow(dead_code)]
pub enum Error {
    Usage(Option<&'static str>),
    UserAbort(&'static str),
    Cli(clap::error::Error),
    Config(ConfyError),
    Io(io::Error),
    Parse(&'static str),
    Image(image::ImageError),
    Protocol(ratatui_image::errors::Errors),
    Download(reqwest::Error),
    Path(PathBuf),
    NoFont,
    Thread,
    UnknownImage(usize, String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Usage(_) => write!(f, "Bad arguments"), // Never shown to user, just a signal.
            Error::UserAbort(msg) => write!(f, "Aborted by user ({msg})"),
            Error::Cli(err) => write!(f, "Command line argument error: {err}"),
            Error::Config(err) => write!(f, "Configuration error: {err}"),
            Error::Io(err) => write!(f, "I/O error: {err}"),
            Error::Parse(msg) => write!(f, "Parse error: {msg}"),
            Error::Image(err) => write!(f, "Image manipulation error: {err}"),
            Error::Protocol(err) => write!(f, "Terminal graphics error: {err}"),
            Error::Download(err) => write!(f, "HTTP request error: {err}"),
            Error::Path(path_str) => write!(f, "Path error: \"{path_str:?}\""),
            Error::NoFont => write!(f, "No font available"),
            Error::Thread => write!(f, "Thread error"),
            Error::UnknownImage(_, url) => write!(f, "Unknown image format: {url}"),
        }
    }
}

impl From<Error> for io::Error {
    fn from(value: Error) -> Self {
        match value {
            Error::Io(io_err) => io_err,
            err => io::Error::new(io::ErrorKind::Other, format!("{err:?}")),
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<ImageError> for Error {
    fn from(value: image::ImageError) -> Self {
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
        Self::Config(value)
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

impl From<SendError<ImgCmd>> for Error {
    fn from(_: SendError<ImgCmd>) -> Self {
        Self::Thread
    }
}

impl From<SendError<ParseCmd>> for Error {
    fn from(_: SendError<ParseCmd>) -> Self {
        Self::Thread
    }
}

impl From<JoinError> for Error {
    fn from(_: JoinError) -> Self {
        Self::Thread
    }
}
