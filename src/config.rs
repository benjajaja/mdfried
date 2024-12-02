use clap::{command, Parser};
use serde::{Deserialize, Serialize};

// #[command(version, about, long_about = None)]
#[derive(Parser)]
#[command(name = "mdcooked")]
#[command(version = "0.1")]
#[command(about = "Cooks Markdown", long_about = Some("You can cook a terminal. Can you **deep fry** a terminal?"))]
pub struct Cli {
    /// Optional name to operate on
    pub filename: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub font_family: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Config { font_family: None }
    }
}
