use clap::{command, Parser};
use serde::{Deserialize, Serialize};

// #[command(version, about, long_about = None)]
#[derive(Parser)]
#[command(name = "mdfried")]
#[command(version = "0.1")]
#[command(about = "Deep fries Markdown", long_about = Some("You can cook a terminal. Can you *deep fry* a terminal?"))]
pub struct Cli {
    /// Optional name to operate on
    pub filename: Option<String>,
}

#[derive(Default, Debug, Serialize, Deserialize)]
pub struct Config {
    pub font_family: Option<String>,
}
