//! what-terminal-font - Detect the terminal font in use, for at least some popular terminals.
//!
//! This crate provides functionality to detect what font is currently being used
//! by the terminal, if the terminal is one of:
//!
//! * kitty
//! * ghostty
//! * foot
//! * wezterm
//! * rio
//!
//! Inspired by fastfetch (and clones') font detection: https://github.com/fastfetch-cli/fastfetch

use std::{
    env,
    fs::File,
    io::{self, BufRead as _, BufReader},
    process::Command,
    string::FromUtf8Error,
};

use thiserror::Error;

#[derive(Error, Debug)]
pub enum WtfError {
    #[error("Terminal could not be determined or is unsupported")]
    UnknownTerminal,
    #[error("Font could not be detected: {0}")]
    FontNotFound(&'static str),
    #[error("I/O Error")]
    Io(io::Error),
    #[error("Env Var Error")]
    EnvVar(env::VarError),
    #[error("Config file {0} expected line with prefix '{1}'")]
    ConfigFile(&'static str, &'static str),
    #[error("{0} output expected line with prefix '{1}'")]
    CommandOutput(&'static str, &'static str),
}

impl From<io::Error> for WtfError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<env::VarError> for WtfError {
    fn from(value: env::VarError) -> Self {
        Self::EnvVar(value)
    }
}

impl From<FromUtf8Error> for WtfError {
    fn from(value: FromUtf8Error) -> Self {
        Self::Io(io::Error::other(value))
    }
}

/// Try to detect the terminal font name.
///
/// ```rust
/// match what_terminal_font::detect_terminal_font() {
///     Ok(font) => println!("Using font: {}", font),
///     Err(e) => println!("Error detecting font: {}", e),
/// }
/// ```
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
                if let Ok(font) = stdout_get_line("ghostty", stdout, "font-family = ") {
                    return Ok(font);
                }
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
                let reader = config.rio()?;
                if let Ok(line) = reader_get_line("rio", reader, "family = \"")
                    && let Some(font_family) = line.split('"').next()
                    && !font_family.is_empty()
                {
                    return Ok(font_family.to_owned());
                }
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
            "xterm-kitty" => stdout_get_line("kitty", config.kitty()?, "font_family: "),
            "foot" => {
                let reader = config.foot()?;
                let line = reader_get_line("foot", reader, "font=")?;
                if let Some(font_family) = line.split(':').next()
                    && !font_family.is_empty()
                {
                    return Ok(font_family.to_owned());
                }
                Err(WtfError::FontNotFound(
                    "foot config expected font line to contain colon separating font-family and size",
                ))
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

fn reader_get_line(
    terminal: &'static str,
    reader: BufReader<File>,
    line_prefix: &'static str,
) -> Result<String, WtfError> {
    for line in reader.lines() {
        let line = line?;
        if line.starts_with(line_prefix)
            && let Some(font_family_line) = line.get(line_prefix.len()..)
        {
            return Ok(font_family_line.to_owned());
        }
    }
    Err(WtfError::ConfigFile(terminal, line_prefix))
}

fn stdout_get_line(
    terminal: &'static str,
    stdout: String,
    prefix: &'static str,
) -> Result<String, WtfError> {
    for line in stdout.lines() {
        if line.starts_with(prefix)
            && let Some(font_family) = line.get(prefix.len()..)
            && !font_family.is_empty()
        {
            return Ok(font_family.to_owned());
        }
    }
    Err(WtfError::CommandOutput(terminal, prefix))
}

fn command_get_stdout(cmd: &mut Command) -> Result<String, WtfError> {
    let output = cmd.output()?;
    let stdout = String::from_utf8(output.stdout)?;
    Ok(stdout)
}

#[cfg(test)]
mod tests;
