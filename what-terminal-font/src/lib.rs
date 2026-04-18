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
    #[error("Command {0} {1:?} expected line with prefix '{2}'")]
    CommandOutput(String, Vec<String>, &'static str),
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
/// match what_terminal_font::get_terminal_font() {
///     Ok(font) => println!("Using font: {}", font),
///     Err(e) => eprintln!("Error detecting font: {}", e),
/// }
/// ```
pub fn detect_terminal_font() -> Result<String, WtfError> {
    if let Ok(term_program) = env::var("TERM_PROGRAM") {
        match term_program.as_str() {
            "ghostty" => {
                if let Ok(font) = command_get_line(
                    Command::new("ghostty").arg("+show-config"),
                    "font-family = ",
                ) {
                    return Ok(font);
                } else {
                    return Ok("JetBrainsMono Nerd Font".to_owned());
                }
            }
            "WezTerm" => {
                let stdout = command_get_stdout(
                    Command::new("wezterm")
                        .arg("ls-fonts")
                        .arg("--text")
                        .arg("a"),
                )?;
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
                if let Ok(line) = config_get_line("rio/config.toml", "family = \"")
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
    if let Ok(term) = env::var("TERM") {
        match term.as_str() {
            "xterm-kitty" => {
                return command_get_line(
                    Command::new("kitten").arg("query-terminal"),
                    "font_family: ",
                );
            }
            "foot" => {
                let line = config_get_line("foot/foot.ini", "font=")?;
                if let Some(font_family) = line.split(':').next()
                    && !font_family.is_empty()
                {
                    return Ok(font_family.to_owned());
                }
                return Err(WtfError::FontNotFound(
                    "foot config expected font line to contain colon separating font-family and size",
                ));
            }
            _ => {}
        }
    }
    Err(WtfError::UnknownTerminal)
}

fn config_get_line(
    config_file: &'static str,
    line_prefix: &'static str,
) -> Result<String, WtfError> {
    let config_home = env::var("XDG_CONFIG_HOME").unwrap_or_else(|_| {
        format!(
            "{}/.config",
            env::var("HOME").unwrap_or_else(|_| "~".to_owned())
        )
    });
    let path = format!("{}/{}", config_home, config_file);

    let reader = BufReader::new(File::open(path)?);
    for line in reader.lines() {
        let line = line?;
        if line.starts_with(line_prefix)
            && let Some(font_family_line) = line.get(line_prefix.len()..)
        {
            return Ok(font_family_line.to_owned());
        }
    }
    Err(WtfError::ConfigFile(config_file, line_prefix))
}

fn command_get_line(cmd: &mut Command, prefix: &'static str) -> Result<String, WtfError> {
    let stdout = command_get_stdout(cmd)?;
    for line in stdout.lines() {
        if line.starts_with(prefix)
            && let Some(font_family) = line.get(prefix.len()..)
            && !font_family.is_empty()
        {
            return Ok(font_family.to_owned());
        }
    }
    Err(WtfError::CommandOutput(
        cmd.get_program().to_str().unwrap_or("[missing]").to_owned(),
        cmd.get_args()
            .map(|arg| arg.to_str().unwrap_or("[missing]").to_owned())
            .collect(),
        prefix,
    ))
}

fn command_get_stdout(cmd: &mut Command) -> Result<String, WtfError> {
    let output = cmd.output()?;
    let stdout = String::from_utf8(output.stdout)?;
    Ok(stdout)
}
