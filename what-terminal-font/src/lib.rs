//! # what-terminal-font
//!
//! Detect the terminal font in use, at least for some popular terminals.
//!
//! Inspired by [fastfetch]. Used by [mdfried].
//!
//! This crate attempts to detect what font is currently being used by the terminal, if the
//! terminal has an implementation in this crate, and either a font is configured, or the terminal
//! has some mechanism for querying the font via some command, or there is a known default font.
//!
//! **Note: the returned "font family" string might not necessarily match the font family
//! name exactly as reported by the different font systems elsewhere.**
//!
//! # Implementations
//!
//! * [x] `ghostty, kitty, wezterm` are queried via some command, and the output is parsed.
//! * [x] `foot, rio, xterm` use their configuration files (linux specific).
//! * [ ] konsole: doesn't set any `$TERM`-like env var, but config should be parseable.
//! * [ ] iterm2: macos hardware.
//! * [ ] others: PRs welcome, [fastfetch source] is a good reference.
//!
//! # Example
//!
//! ```rust
//! use what_terminal_font::{detect_terminal_font, WtfError};
//!
//! match detect_terminal_font() {
//!     Ok(font) => println!("Using font: {}", font),
//!     Err(WtfError::FontNotFound(err)) => println!("Font not detected ({err}), falling back to 'monospace'."),
//!     Err(WtfError::UnknownTerminal) => println!("Unknown terminal, falling back to 'monospace'."),
//!     Err(WtfError::Io(err)) => println!("{err}, aborting."),
//! }
//! ```
//!
//! [fastfetch]: https://github.com/fastfetch-cli/fastfetch
//! [mdfried]: https://crates.io/crates/mdfried
//! [fastfetch source]: https://github.com/fastfetch-cli/fastfetch/blob/master/src/detection/terminalfont/terminalfont_linux.c

use std::{
    env,
    fs::File,
    io::{self, BufRead, BufReader},
    process::Command,
    string::FromUtf8Error,
};

use thiserror::Error;

#[derive(Error, Debug)]
pub enum WtfError {
    #[error("I/O Error")]
    Io(io::Error),
    #[error("Terminal could not be determined or is unsupported")]
    UnknownTerminal,
    #[error("Font could not be detected: {0}")]
    FontNotFound(&'static str),
}

impl From<io::Error> for WtfError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<env::VarError> for WtfError {
    fn from(value: env::VarError) -> Self {
        Self::Io(io::Error::other(value))
    }
}

impl From<FromUtf8Error> for WtfError {
    fn from(value: FromUtf8Error) -> Self {
        Self::Io(io::Error::other(value))
    }
}

/// Try to detect the terminal font name.
///
/// Gets the font name by config file or command.
/// If there is a known fallback when the font is not configure, it will be returned. Otherwise
/// [`WtfError::FontNotFound`] will be returned.
///
/// If there is no implementation for the current terminal, then [`WtfError::UnknownTerminal`] will
/// be returned.
pub fn detect_terminal_font() -> Result<String, WtfError> {
    detect(RealTerminal, env::var("TERM_PROGRAM"), env::var("TERM"))
}

fn detect(
    config: impl TerminalConfig,
    term_program: Result<String, env::VarError>,
    term: Result<String, env::VarError>,
) -> Result<String, WtfError> {
    detect_with_term_program(term_program, &config).or(detect_with_term(term, &config))
}

fn detect_with_term_program(
    var: Result<String, env::VarError>,
    config: &impl TerminalConfig,
) -> Result<String, WtfError> {
    if let Ok(term_program) = var {
        match term_program.as_str() {
            "ghostty" => {
                let stdout = config.ghostty()?;
                if let Ok(font) = find_line(
                    stdout.as_bytes(),
                    "font-family = ",
                    "ghostty output had no font-family",
                ) {
                    return Ok(font);
                }
                // Default font of ghostty
                return Ok("JetBrainsMono Nerd Font".to_owned());
            }
            "WezTerm" => {
                let stdout = config.wezterm()?;
                if let Some(font) = stdout.split('"').nth(1)
                    && !font.is_empty()
                {
                    return Ok(font.to_owned());
                } else {
                    return Err(WtfError::FontNotFound(
                        "wezterm command had unexpected output",
                    ));
                }
            }
            "rio" => {
                if let Ok(reader) = config.rio() {
                    if let Ok(line) = find_line(reader, "family = \"", "rio config had no family")
                        && let Some(font_family) = line.split('"').next()
                        && !font_family.is_empty()
                    {
                        return Ok(font_family.to_owned());
                    }
                }
                // Default font of rio
                return Ok("Cascadia Code".to_owned());
            }
            _ => {}
        }
    }
    Err(WtfError::UnknownTerminal)
}

fn detect_with_term(
    var: Result<String, env::VarError>,
    config: &impl TerminalConfig,
) -> Result<String, WtfError> {
    if let Ok(term) = var {
        match term.as_str() {
            "xterm-kitty" => find_line(
                config.kitty()?.as_bytes(),
                "font_family: ",
                "kitten output had no font_family",
            ),
            "foot" => {
                if let Ok(reader) = config.foot() {
                    let line = find_line(reader, "font=", "foot config had not font")?;
                    if let Some(font_family) = line.split(':').next()
                        && !font_family.is_empty()
                    {
                        return Ok(font_family.to_owned());
                    }
                }
                Err(WtfError::FontNotFound("foot"))
            }
            "xterm" | "xterm-256color" => {
                if let Ok(reader) = config.xterm() {
                    if let Ok(font_line) = find_line(reader, "xterm*faceName:", "xterm*faceName")
                        && !font_line.is_empty()
                    {
                        return Ok(font_line);
                    }
                }
                if let Ok(reader) = config.xterm()
                    && let Ok(font_line) =
                        find_line(reader, "xterm.vt100.faceName:", "xterm.vt100.faceName")
                    && !font_line.is_empty()
                {
                    return Ok(font_line);
                }
                // "fixed" is the standard X11 built-in monospace font, used as fallback in
                // fastfetch
                Ok("fixed".to_owned())
            }
            _ => Err(WtfError::UnknownTerminal),
        }
    } else {
        Err(WtfError::UnknownTerminal)
    }
}

trait TerminalConfig {
    fn ghostty(&self) -> Result<String, WtfError>;
    fn kitty(&self) -> Result<String, WtfError>;
    fn rio(&self) -> Result<BufReader<File>, WtfError>;
    fn wezterm(&self) -> Result<String, WtfError>;
    fn foot(&self) -> Result<BufReader<File>, WtfError>;
    fn xterm(&self) -> Result<BufReader<File>, WtfError>;
}

struct RealTerminal;

impl TerminalConfig for RealTerminal {
    fn ghostty(&self) -> Result<String, WtfError> {
        command_get_stdout(Command::new("ghostty").arg("+show-config"))
    }

    fn kitty(&self) -> Result<String, WtfError> {
        command_get_stdout(Command::new("kitten").arg("query-terminal"))
    }

    fn rio(&self) -> Result<BufReader<File>, WtfError> {
        config_get_reader("rio/config.toml")
    }

    fn wezterm(&self) -> Result<String, WtfError> {
        command_get_stdout(
            Command::new("wezterm")
                .arg("ls-fonts")
                .arg("--text")
                .arg("a"),
        )
    }

    fn foot(&self) -> Result<BufReader<File>, WtfError> {
        config_get_reader("foot/foot.ini")
    }

    fn xterm(&self) -> Result<BufReader<File>, WtfError> {
        config_get_reader(".Xresources")
    }
}

fn config_get_reader(config_file: &'static str) -> Result<BufReader<File>, WtfError> {
    let config_home = env::var("XDG_CONFIG_HOME").unwrap_or_else(|_| {
        format!(
            "{}/.config",
            env::var("HOME").unwrap_or_else(|_| "~".to_owned())
        )
    });
    let path = format!("{}/{}", config_home, config_file);

    Ok(BufReader::new(File::open(path)?))
}

fn command_get_stdout(cmd: &mut Command) -> Result<String, WtfError> {
    let output = cmd.output()?;
    let stdout = String::from_utf8(output.stdout)?;
    Ok(stdout)
}

fn find_line(
    source: impl BufRead,
    prefix: &'static str,
    error_message: &'static str,
) -> Result<String, WtfError> {
    for line in source.lines() {
        let line = line?;
        if line.starts_with(prefix)
            && let Some(font_family) = line.get(prefix.len()..)
            && !font_family.is_empty()
        {
            return Ok(font_family.to_owned());
        }
    }
    Err(WtfError::FontNotFound(error_message))
}

#[cfg(test)]
mod tests;
