use std::path::PathBuf;

use confy::ConfyError;
use ratatui::crossterm::style::Color;
use ratatui_image::picker::ProtocolType;
use serde::{Deserialize, Serialize};

use crate::error::Error;

// The configuration struct used throughout the program.
//
// Has implicit `Default` in `From<UserConfig>`.
#[derive(Debug)]
pub struct Config {
    pub padding: PaddingConfig,
    pub max_image_height: u16,
    pub watch_debounce_milliseconds: u64,
    pub enable_mouse_capture: bool,
    pub debug_override_protocol_type: Option<ProtocolType>,
    pub theme: Theme,
}

impl From<UserConfig> for Config {
    fn from(uc: UserConfig) -> Self {
        Config {
            padding: uc.padding.unwrap_or_default(),
            max_image_height: uc.max_image_height.unwrap_or(30),
            watch_debounce_milliseconds: uc.watch_debounce_milliseconds.unwrap_or(100),
            enable_mouse_capture: uc.enable_mouse_capture.unwrap_or(false),
            debug_override_protocol_type: uc.debug_override_protocol_type,
            theme: uc.theme.unwrap_or_default(),
        }
    }
}

// The configuration struct of the config file, everything must be optional.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct UserConfig {
    pub font_family: Option<String>,
    pub padding: Option<PaddingConfig>,
    pub max_image_height: Option<u16>,
    pub watch_debounce_milliseconds: Option<u64>,
    pub enable_mouse_capture: Option<bool>,
    pub debug_override_protocol_type: Option<ProtocolType>,
    pub theme: Option<Theme>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Theme {
    pub skin: ratskin::MadSkin,
}
impl Default for Theme {
    fn default() -> Self {
        let mut skin = ratskin::MadSkin::default();

        skin.bold.set_fg(Color::AnsiValue(220));

        skin.inline_code.set_fg(Color::AnsiValue(203));
        skin.inline_code.set_bg(Color::AnsiValue(236));

        skin.code_block.set_fg(Color::AnsiValue(203));

        skin.quote_mark.set_fg(Color::AnsiValue(63));
        skin.bullet.set_fg(Color::AnsiValue(63));
        Theme { skin }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum PaddingConfig {
    None,
    Centered(u16),
}
impl Default for PaddingConfig {
    fn default() -> Self {
        PaddingConfig::Centered(100)
    }
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
            PaddingConfig::None | PaddingConfig::Centered(_) => screen_height,
        }
    }
}

const CONFIG_APP_NAME: &str = "mdfried";
const CONFIG_CONFIG_NAME: &str = "config";

pub fn get_configuration_file_path() -> Option<PathBuf> {
    confy::get_configuration_file_path(CONFIG_APP_NAME, CONFIG_CONFIG_NAME).ok()
}

// Save (overwrite) the config file.
fn store(new_config: &UserConfig) -> Result<(), ConfyError> {
    log::warn!("store config file");
    confy::store(CONFIG_APP_NAME, CONFIG_CONFIG_NAME, new_config)
}

// Save (overwrite) only the font_family into the config file.
pub fn store_font_family(config: &mut UserConfig, font_family: String) -> Result<(), ConfyError> {
    log::warn!("store config file with new font_family");
    config.font_family = Some(font_family);
    store(config)
}

pub fn load_or_ask() -> Result<UserConfig, Error> {
    use crate::setup::configpicker::{
        ConfigResolution::{Abort, Ignore, Overwrite},
        interactive_resolve_config,
    };
    let file_exists = get_configuration_file_path().is_some_and(|p| p.exists());
    if !file_exists {
        return Ok(UserConfig::default());
    }
    confy::load::<UserConfig>(CONFIG_APP_NAME, CONFIG_CONFIG_NAME).or_else(|error| {
        match interactive_resolve_config(&error.into())? {
            Overwrite => {
                let config = UserConfig::default();
                store(&config)?;
                crate::setup::notification::interactive_notification(
                    "Config file has been overwritten...",
                )?;
                Ok(config)
            }
            Ignore => Ok(UserConfig::default()),
            Abort => {
                println!(
                    "Aborted: edit and resolve configuration file errors or delete the file manually.",
                );
                Err(Error::UserAbort("aborted"))
            }
        }
    })
}
