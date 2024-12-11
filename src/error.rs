use core::fmt;
use std::{io, sync::mpsc::SendError};

use confy::ConfyError;
use image::ImageError;
use tokio::task::JoinError;

use crate::{ImgCmd, ParseCmd, WidthEvent};

#[derive(Debug)]
#[allow(dead_code)]
pub enum Error {
    Cli(clap::error::Error),
    Config(ConfyError),
    Io(io::Error),
    Image(image::ImageError),
    Protocol(ratatui_image::errors::Errors),
    Download(reqwest::Error),
    Msg(String),
    NoFont,
    Thread,
    UnknownImage(usize, String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Cli(err) => write!(f, "Command line argument error: {err}"),
            Error::Config(err) => write!(f, "Configuration error: {err}"),
            Error::Io(err) => write!(f, "I/O error: {err}"),
            Error::Image(err) => write!(f, "Image manipulation error: {err}"),
            Error::Protocol(err) => write!(f, "Terminal graphics error: {err}"),
            Error::Download(err) => write!(f, "HTTP request error: {err}"),
            Error::Msg(err) => write!(f, "Error: {err}"),
            Error::NoFont => write!(f, "Error: no font available"),
            Error::Thread => write!(f, "Error: thread error"),
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

impl From<&str> for Error {
    fn from(value: &str) -> Self {
        Self::Msg(value.to_string())
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
