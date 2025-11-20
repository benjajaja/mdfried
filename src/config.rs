use std::path::PathBuf;

use clap::{Parser, command};
use confy::ConfyError;
use ratatui::crossterm::style::Color;
use ratatui_image::picker::ProtocolType;
use serde::{Deserialize, Serialize};

use crate::error::Error;

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
    pub padding: PaddingConfig,
    pub max_image_height: u16,
    pub enable_mouse_capture: Option<bool>,
    pub debug_override_protocol_type: Option<ProtocolType>,
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
            max_image_height: 30,
            enable_mouse_capture: None,
            debug_override_protocol_type: None,
            skin,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum PaddingConfig {
    None,
    Centered(u16),
}

impl PaddingConfig {
    pub fn calculate_width(&self, screen_width: u16) -> u16 {
        match self {
            PaddingConfig::None => screen_width,
            PaddingConfig::Centered(width) => screen_width.min(*width),
        }
    }

    pub fn calculate_height(&self, screen_height: u16) -> u16 {
        match self {
            PaddingConfig::None => screen_height,
            PaddingConfig::Centered(_) => screen_height,
        }
    }
}

impl Default for PaddingConfig {
    fn default() -> Self {
        PaddingConfig::Centered(100)
    }
}

const CONFIG_APP_NAME: &str = "mdfried";
const CONFIG_CONFIG_NAME: &str = "config";

pub fn get_configuration_file_path() -> Option<PathBuf> {
    confy::get_configuration_file_path(CONFIG_APP_NAME, CONFIG_CONFIG_NAME).ok()
}

// Save (overwrite) the config file.
fn store(new_config: &Config) -> Result<(), ConfyError> {
    log::warn!("store config file");
    confy::store(CONFIG_APP_NAME, CONFIG_CONFIG_NAME, new_config)
}

// Save (overwrite) only the font_family into the config file.
pub fn store_font_family(config: &mut Config, font_family: String) -> Result<(), ConfyError> {
    log::warn!("store config file with new font_family");
    config.font_family = Some(font_family);
    store(config)
}

pub fn load_or_ask() -> Result<Config, Error> {
    use crate::setup::configpicker::{ConfigResolution::*, interactive_resolve_config};
    let file_existed = get_configuration_file_path()
        .map(|p| p.exists())
        .unwrap_or_default();
    match confy::load::<Config>(CONFIG_APP_NAME, CONFIG_CONFIG_NAME) {
        Ok(config) => {
            if !file_existed {
                crate::setup::notification::interactive_notification(
                    "Default config file has been written...",
                )?;
            }
            Ok(config)
        }
        Err(error) => match interactive_resolve_config(error.into())? {
            Overwrite => {
                let config = Config::default();
                store(&config)?;
                crate::setup::notification::interactive_notification(
                    "Config file has been overwritten...",
                )?;
                Ok(config)
            }
            Ignore => Ok(Config::default()),
            Abort => {
                println!(
                    "Aborted: edit and resolve configuration file errors or delete the file manually.",
                );
                Err(Error::UserAbort("aborted"))
            }
        },
    }
}
