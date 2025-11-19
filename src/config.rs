use clap::{Parser, command};
use confy::ConfyError;
use ratatui::crossterm::style::Color;
use ratatui_image::picker::ProtocolType;
use serde::{Deserialize, Serialize};

use crate::{Padding, error::Error};

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
    pub enable_mouse_capture: bool,
    pub max_image_height: u16,
    pub debug_override_protocol_type: Option<ProtocolType>,
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

        let enable_mouse_capture = false;

        let max_image_height = 30;

        let debug_override_protocol_type = None;

        Self {
            font_family: Default::default(),
            padding: Default::default(),
            skin,
            enable_mouse_capture,
            max_image_height,
            debug_override_protocol_type,
        }
    }
}

const CONFIG_APP_NAME: &str = "mdfried";
const CONFIG_CONFIG_NAME: &str = "config";

pub fn get_configuration_file_path() -> String {
    confy::get_configuration_file_path(CONFIG_APP_NAME, CONFIG_CONFIG_NAME)
        .map(|p| p.display().to_string())
        .unwrap_or("(unknown config file path)".into())
}

pub fn store(new_config: Config) -> Result<(), ConfyError> {
    confy::store(CONFIG_APP_NAME, CONFIG_CONFIG_NAME, new_config)
}

pub fn load_or_ask() -> Result<Config, Error> {
    use crate::setup::configpicker::{ConfigResolution::*, interactive_resolve_config};
    match confy::load::<Config>(CONFIG_APP_NAME, CONFIG_CONFIG_NAME) {
        Ok(config) => Ok(config),
        Err(error) => match interactive_resolve_config(error.into())? {
            Overwrite => {
                store(Config::default())?;
                Ok(Config::default())
            }
            Ignore => Ok(Config::default()),
            Abort => Err(Error::UserAbort("aborted")),
        },
    }
}
