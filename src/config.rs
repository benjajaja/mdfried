use clap::{command, Parser};
use crossterm::style::Color;
use serde::{Deserialize, Serialize};

use crate::Padding;

#[derive(Parser)]
#[command(name = "mdfried")]
#[command(version = "0.1")]
#[command(about = "Deep fries Markdown", long_about = Some("You can cook a terminal. Can you *deep fry* a terminal?"))]
pub struct Cli {
    /// Optional name to operate on
    pub filename: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub font_family: Option<String>,
    pub padding: Padding,
    pub skin: ratskin::MadSkin,
}

impl Default for Config {
    fn default() -> Self {
        let mut skin = ratskin::MadSkin::default();

        skin.bold.set_fg(Color::AnsiValue(220));

        skin.inline_code.set_fg(Color::AnsiValue(203));
        skin.inline_code.set_bg(Color::AnsiValue(236));

        skin.code_block.set_fg(Color::AnsiValue(203));

        skin.quote_mark.set_fg(Color::AnsiValue(63));
        skin.bullet.set_fg(Color::AnsiValue(63));

        Self {
            font_family: Default::default(),
            padding: Default::default(),
            skin,
        }
    }
}
