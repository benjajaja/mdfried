use std::{fs, path::PathBuf};

use confy::ConfyError;
use ratatui::style::Color;
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Theme {
    // Symbols
    pub blockquote_bar: Option<String>,
    pub link_desc_open: Option<String>,
    pub link_desc_close: Option<String>,
    pub link_url_open: Option<String>,
    pub link_url_close: Option<String>,
    pub horizontal_rule_char: Option<String>,
    pub task_checked_mark: Option<String>,

    // Colors
    pub blockquote_colors: Option<Vec<Color>>,
    pub link_bg: Option<Color>,
    pub link_fg: Option<Color>,
    pub prefix_color: Option<Color>,
    pub emphasis_color: Option<Color>,
    pub code_bg: Option<Color>,
    pub code_fg: Option<Color>,
    pub hr_color: Option<Color>,
    pub table_border_color: Option<Color>,
    pub table_header_color: Option<Color>,
}

// Delegate to StyledMapper for defaults
const STYLED: mdfrier::StyledMapper = mdfrier::StyledMapper;

// Mapper implementation provides content/decorator symbols
impl mdfrier::Mapper for Theme {
    fn blockquote_bar(&self) -> &str {
        self.blockquote_bar
            .as_deref()
            .unwrap_or(STYLED.blockquote_bar())
    }
    fn link_desc_open(&self) -> &str {
        self.link_desc_open
            .as_deref()
            .unwrap_or(STYLED.link_desc_open())
    }
    fn link_desc_close(&self) -> &str {
        self.link_desc_close
            .as_deref()
            .unwrap_or(STYLED.link_desc_close())
    }
    fn link_url_open(&self) -> &str {
        self.link_url_open
            .as_deref()
            .unwrap_or(STYLED.link_url_open())
    }
    fn link_url_close(&self) -> &str {
        self.link_url_close
            .as_deref()
            .unwrap_or(STYLED.link_url_close())
    }
    fn horizontal_rule_char(&self) -> &str {
        self.horizontal_rule_char
            .as_deref()
            .unwrap_or(STYLED.horizontal_rule_char())
    }
    fn task_checked(&self) -> &str {
        self.task_checked_mark
            .as_deref()
            .unwrap_or(STYLED.task_checked())
    }
    // Table borders - delegate to StyledMapper
    fn table_vertical(&self) -> &str {
        STYLED.table_vertical()
    }
    fn table_horizontal(&self) -> &str {
        STYLED.table_horizontal()
    }
    fn table_top_left(&self) -> &str {
        STYLED.table_top_left()
    }
    fn table_top_right(&self) -> &str {
        STYLED.table_top_right()
    }
    fn table_bottom_left(&self) -> &str {
        STYLED.table_bottom_left()
    }
    fn table_bottom_right(&self) -> &str {
        STYLED.table_bottom_right()
    }
    fn table_top_junction(&self) -> &str {
        STYLED.table_top_junction()
    }
    fn table_bottom_junction(&self) -> &str {
        STYLED.table_bottom_junction()
    }
    fn table_left_junction(&self) -> &str {
        STYLED.table_left_junction()
    }
    fn table_right_junction(&self) -> &str {
        STYLED.table_right_junction()
    }
    fn table_cross(&self) -> &str {
        STYLED.table_cross()
    }
    // Text decorators - delegate to StyledMapper (removes them)
    fn emphasis_open(&self) -> &str {
        STYLED.emphasis_open()
    }
    fn emphasis_close(&self) -> &str {
        STYLED.emphasis_close()
    }
    fn strong_open(&self) -> &str {
        STYLED.strong_open()
    }
    fn strong_close(&self) -> &str {
        STYLED.strong_close()
    }
    fn code_open(&self) -> &str {
        STYLED.code_open()
    }
    fn code_close(&self) -> &str {
        STYLED.code_close()
    }
    fn strikethrough_open(&self) -> &str {
        STYLED.strikethrough_open()
    }
    fn strikethrough_close(&self) -> &str {
        STYLED.strikethrough_close()
    }
}

// Theme implementation provides colors/styles (extends Mapper)
impl mdfrier::ratatui::Theme for Theme {
    fn blockquote_color(&self, depth: usize) -> Color {
        const DEFAULT_COLORS: [Color; 6] = [
            Color::Indexed(202),
            Color::Indexed(203),
            Color::Indexed(204),
            Color::Indexed(205),
            Color::Indexed(206),
            Color::Indexed(207),
        ];
        match &self.blockquote_colors {
            Some(colors) if !colors.is_empty() => colors[depth % colors.len()],
            _ => DEFAULT_COLORS[depth % DEFAULT_COLORS.len()],
        }
    }

    fn link_bg(&self) -> Color {
        self.link_bg.unwrap_or(Color::Indexed(237))
    }

    fn link_fg(&self) -> Color {
        self.link_fg.unwrap_or(Color::Indexed(4))
    }

    fn prefix_color(&self) -> Color {
        self.prefix_color.unwrap_or(Color::Indexed(189))
    }

    fn emphasis_color(&self) -> Color {
        self.emphasis_color.unwrap_or(Color::Indexed(220))
    }

    fn code_bg(&self) -> Color {
        self.code_bg.unwrap_or(Color::Indexed(236))
    }

    fn code_fg(&self) -> Color {
        self.code_fg.unwrap_or(Color::Indexed(203))
    }

    fn hr_color(&self) -> Color {
        self.hr_color.unwrap_or(Color::Indexed(240))
    }

    fn table_border_color(&self) -> Color {
        self.table_border_color.unwrap_or(Color::Indexed(240))
    }

    fn table_header_color(&self) -> Color {
        self.table_header_color.unwrap_or(Color::Indexed(255))
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

// Write a default config file to stdout.
pub fn print_default() -> Result<(), Error> {
    let config = Config::from(UserConfig::default());
    let user_config = UserConfig {
        padding: Some(config.padding),
        font_family: None,
        max_image_height: Some(config.max_image_height),
        watch_debounce_milliseconds: Some(config.watch_debounce_milliseconds),
        enable_mouse_capture: Some(config.enable_mouse_capture),
        debug_override_protocol_type: config.debug_override_protocol_type,
        theme: Some(config.theme),
    };

    // We could use the toml crate to avoid doing the temp-file roundtrip, but doing it this way
    // means it's guaranteed to be good for `confy::load`.
    let tmp_path = std::env::temp_dir().join(format!("mdfried_tmp_config_{}", std::process::id()));
    confy::store_path(&tmp_path, user_config)?;
    let text = fs::read_to_string(&tmp_path)?;
    println!("{text}");
    fs::remove_file(tmp_path)?;

    let default_config_path = get_configuration_file_path()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| String::from("(not found)"));
    eprintln!("Config file default path: {default_config_path}",);
    Ok(())
}
