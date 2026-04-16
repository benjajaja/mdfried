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
};

use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Terminal could not be determined or is unsupported")]
    UnknownTerminal,
    #[error("Font could not be detected")]
    FontNotFound,
    #[error("I/O Error")]
    Io(io::Error),
}

impl From<io::Error> for Error {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
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
pub fn detect_terminal_font() -> Result<String, Error> {
    if let Ok(term_program) = env::var("TERM_PROGRAM") {
        match term_program.as_str() {
            "ghostty" => {
                const GHOSTTY_DEFAULT_FONT: &str = "JetBrainsMono Nerd Font";
                const GHOSTTY_FONT_FAMILY_CONFIG_PREFIX: &str = "font-family = ";

                let output = Command::new("ghostty").arg("+show-config").output()?;
                let stdout = String::from_utf8_lossy(&output.stdout);
                for line in stdout.lines() {
                    if line.starts_with(GHOSTTY_FONT_FAMILY_CONFIG_PREFIX)
                        && let Some(font_family) =
                            line.get(GHOSTTY_FONT_FAMILY_CONFIG_PREFIX.len()..)
                        && font_family.len() > 0
                    {
                        return Ok(font_family.to_owned());
                    }
                }
                return Ok(GHOSTTY_DEFAULT_FONT.to_owned());
            }
            "WezTerm" => {
                let output = Command::new("wezterm")
                    .arg("ls-fonts")
                    .arg("--text")
                    .arg("a")
                    .output()?;
                let stdout = String::from_utf8_lossy(&output.stdout);
                let font = stdout.split('"').nth(1);
                if let Some(font) = font
                    && font.len() > 0
                {
                    return Ok(font.to_owned());
                }
                return Err(Error::FontNotFound);
            }
            "rio" => {
                const RIO_FONT_FAMILY_CONFIG_PREFIX: &str = "family = \"";
                let config_home = env::var("XDG_CONFIG_HOME")
                    .unwrap_or_else(|_| format!("{}/.config", env::var("HOME").unwrap()));

                let path = format!("{}/rio/config.toml", config_home);
                let reader = BufReader::new(File::open(path)?);
                for line in reader.lines() {
                    let line = line?;
                    if line.starts_with(RIO_FONT_FAMILY_CONFIG_PREFIX)
                        && let Some(font_family_line) =
                            line.get(RIO_FONT_FAMILY_CONFIG_PREFIX.len()..)
                    {
                        if let Some(font_family) = font_family_line.split('"').nth(1)
                            && font_family.len() > 0
                        {
                            return Ok(font_family.to_owned());
                        }
                    }
                }
                return Err(Error::FontNotFound);
            }
            _ => {}
        }
    }
    if let Ok(term) = env::var("TERM") {
        match term.as_str() {
            "xterm-kitty" => {
                const KITTY_FONT_FAMILY_CONFIG_PREFIX: &str = "font_family: ";
                let output = Command::new("kitten").arg("query-terminal").output()?;
                let stdout = String::from_utf8_lossy(&output.stdout);
                for line in stdout.lines() {
                    if line.starts_with(KITTY_FONT_FAMILY_CONFIG_PREFIX)
                        && let Some(font_family) = line.get(KITTY_FONT_FAMILY_CONFIG_PREFIX.len()..)
                        && font_family.len() > 0
                    {
                        return Ok(font_family.to_owned());
                    }
                }
                return Err(Error::FontNotFound);
            }
            "foot" => {
                const FOOT_FONT_FAMILY_CONFIG_PREFIX: &str = "font=";
                let config_home = env::var("XDG_CONFIG_HOME")
                    .unwrap_or_else(|_| format!("{}/.config", env::var("HOME").unwrap()));

                let path = format!("{}/foot/foot.ini", config_home);
                let reader = BufReader::new(File::open(path)?);
                for line in reader.lines() {
                    let line = line?;
                    if line.starts_with(FOOT_FONT_FAMILY_CONFIG_PREFIX)
                        && let Some(font_family) = line.get(FOOT_FONT_FAMILY_CONFIG_PREFIX.len()..)
                    {
                        if let Some(first) = font_family.split(":").nth(0)
                            && first.len() > 0
                        {
                            return Ok(first.to_owned());
                        }
                    }
                }
                return Err(Error::FontNotFound);
            }
            _ => {}
        }
    }
    Err(Error::UnknownTerminal)
}
