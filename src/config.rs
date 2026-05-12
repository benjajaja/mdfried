use std::{fs, path::PathBuf, str::FromStr as _};

use confy::ConfyError;
use mdfrier::Mapper;
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
    pub url_transform_command: Option<String>,
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
            url_transform_command: uc.url_transform_command,
            theme: uc.theme.unwrap_or_else(|| Theme {
                hide_urls: Some(true),
                ..Default::default()
            }),
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
    pub url_transform_command: Option<String>,
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

    // Other options
    pub header_color: Option<Color>,
    pub hide_urls: Option<bool>,
    pub has_text_size_protocol: Option<bool>,
}

// Delegate to StyledMapper for defaults
const STYLED_MAPPER: mdfrier::StyledMapper = mdfrier::StyledMapper;

// Mapper implementation provides content/decorator symbols
impl Mapper for Theme {
    fn blockquote_bar(&self) -> &str {
        self.blockquote_bar
            .as_deref()
            .unwrap_or(STYLED_MAPPER.blockquote_bar())
    }
    fn link_desc_open(&self) -> &str {
        self.link_desc_open
            .as_deref()
            .unwrap_or(if self.hide_urls() {
                ""
            } else {
                STYLED_MAPPER.link_desc_open()
            })
    }
    fn link_desc_close(&self) -> &str {
        self.link_desc_close
            .as_deref()
            .unwrap_or(if self.hide_urls() {
                ""
            } else {
                STYLED_MAPPER.link_desc_close()
            })
    }
    fn link_url_open(&self) -> &str {
        self.link_url_open
            .as_deref()
            .unwrap_or(STYLED_MAPPER.link_url_open())
    }
    fn link_url_close(&self) -> &str {
        self.link_url_close
            .as_deref()
            .unwrap_or(STYLED_MAPPER.link_url_close())
    }
    fn horizontal_rule_char(&self) -> &str {
        self.horizontal_rule_char
            .as_deref()
            .unwrap_or(STYLED_MAPPER.horizontal_rule_char())
    }
    fn task_checked(&self) -> &str {
        self.task_checked_mark
            .as_deref()
            .unwrap_or(STYLED_MAPPER.task_checked())
    }
    // Table borders - delegate to StyledMapper
    fn table_vertical(&self) -> &str {
        STYLED_MAPPER.table_vertical()
    }
    fn table_horizontal(&self) -> &str {
        STYLED_MAPPER.table_horizontal()
    }
    fn table_top_left(&self) -> &str {
        STYLED_MAPPER.table_top_left()
    }
    fn table_top_right(&self) -> &str {
        STYLED_MAPPER.table_top_right()
    }
    fn table_bottom_left(&self) -> &str {
        STYLED_MAPPER.table_bottom_left()
    }
    fn table_bottom_right(&self) -> &str {
        STYLED_MAPPER.table_bottom_right()
    }
    fn table_top_junction(&self) -> &str {
        STYLED_MAPPER.table_top_junction()
    }
    fn table_bottom_junction(&self) -> &str {
        STYLED_MAPPER.table_bottom_junction()
    }
    fn table_left_junction(&self) -> &str {
        STYLED_MAPPER.table_left_junction()
    }
    fn table_right_junction(&self) -> &str {
        STYLED_MAPPER.table_right_junction()
    }
    fn table_cross(&self) -> &str {
        STYLED_MAPPER.table_cross()
    }
    // Text decorators - delegate to StyledMapper (removes them)
    fn emphasis_open(&self) -> &str {
        STYLED_MAPPER.emphasis_open()
    }
    fn emphasis_close(&self) -> &str {
        STYLED_MAPPER.emphasis_close()
    }
    fn strong_open(&self) -> &str {
        STYLED_MAPPER.strong_open()
    }
    fn strong_close(&self) -> &str {
        STYLED_MAPPER.strong_close()
    }
    fn code_open(&self) -> &str {
        STYLED_MAPPER.code_open()
    }
    fn code_close(&self) -> &str {
        STYLED_MAPPER.code_close()
    }
    fn strikethrough_open(&self) -> &str {
        STYLED_MAPPER.strikethrough_open()
    }
    fn strikethrough_close(&self) -> &str {
        STYLED_MAPPER.strikethrough_close()
    }
    fn hide_urls(&self) -> bool {
        self.hide_urls.unwrap_or(true)
    }
    fn has_text_size_protocol(&self) -> bool {
        self.has_text_size_protocol.unwrap_or_default()
    }
}

// Delegate to DefaultTheme for defaults
const DEFAULT_THEME: mdfrier::ratatui::DefaultTheme = mdfrier::ratatui::DefaultTheme;

// Theme implementation provides colors/styles (extends Mapper)
impl mdfrier::ratatui::Theme for Theme {
    fn blockquote_color(&self, depth: usize) -> Color {
        match &self.blockquote_colors {
            Some(colors) if !colors.is_empty() => colors[depth % colors.len()],
            _ => DEFAULT_THEME.blockquote_color(depth),
        }
    }

    fn link_bg(&self) -> Color {
        self.link_bg.unwrap_or(DEFAULT_THEME.link_bg())
    }

    fn link_fg(&self) -> Color {
        self.link_fg.unwrap_or(DEFAULT_THEME.link_fg())
    }

    fn prefix_color(&self) -> Color {
        self.prefix_color.unwrap_or(DEFAULT_THEME.prefix_color())
    }

    fn emphasis_color(&self) -> Color {
        self.emphasis_color
            .unwrap_or(DEFAULT_THEME.emphasis_color())
    }

    fn code_bg(&self) -> Color {
        self.code_bg.unwrap_or(DEFAULT_THEME.code_bg())
    }

    fn code_fg(&self) -> Color {
        self.code_fg.unwrap_or(DEFAULT_THEME.code_fg())
    }

    fn hr_color(&self) -> Color {
        self.hr_color.unwrap_or(DEFAULT_THEME.hr_color())
    }

    fn table_border_color(&self) -> Color {
        self.table_border_color
            .unwrap_or(DEFAULT_THEME.table_border_color())
    }

    fn table_header_color(&self) -> Color {
        self.table_header_color
            .unwrap_or(DEFAULT_THEME.table_header_color())
    }
}

impl Theme {
    fn defaults_for_print() -> Theme {
        use mdfrier::ratatui::Theme as _;
        let theme = Theme::default();
        // Copypaste from mdfrier ratatui.rs
        const DEFAULT_BLOCKQUOTE_COLORS: [Color; 6] = [
            Color::Indexed(202),
            Color::Indexed(203),
            Color::Indexed(204),
            Color::Indexed(205),
            Color::Indexed(206),
            Color::Indexed(207),
        ];
        Theme {
            blockquote_bar: Some(Theme::blockquote_bar(&theme).to_owned()),
            link_desc_open: Some(Theme::link_desc_open(&theme).to_owned()),
            link_desc_close: Some(Theme::link_desc_close(&theme).to_owned()),
            link_url_open: Some(Theme::link_url_open(&theme).to_owned()),
            link_url_close: Some(Theme::link_url_close(&theme).to_owned()),
            horizontal_rule_char: Some(Theme::horizontal_rule_char(&theme).to_owned()),
            task_checked_mark: Some(Theme::task_checked(&theme).to_owned()),
            blockquote_colors: Some(DEFAULT_BLOCKQUOTE_COLORS.to_vec()),
            link_bg: Some(Theme::link_bg(&theme)),
            link_fg: Some(Theme::link_fg(&theme)),
            prefix_color: Some(Theme::prefix_color(&theme)),
            emphasis_color: Some(Theme::emphasis_color(&theme)),
            code_bg: Some(Theme::code_bg(&theme)),
            code_fg: Some(Theme::code_fg(&theme)),
            hr_color: Some(Theme::hr_color(&theme)),
            table_border_color: Some(Theme::table_border_color(&theme)),
            table_header_color: Some(Theme::table_header_color(&theme)),
            hide_urls: Some(Theme::hide_urls(&theme)),
            header_color: Some(Color::from_str("#FFFFFF").unwrap_or_default()),
            has_text_size_protocol: None,
        }
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
        font_family: Some("your-font-name".to_owned()),
        max_image_height: Some(config.max_image_height),
        watch_debounce_milliseconds: Some(config.watch_debounce_milliseconds),
        enable_mouse_capture: Some(config.enable_mouse_capture),
        debug_override_protocol_type: None,
        url_transform_command: Some("readable | html2text".to_owned()),
        theme: Some(Theme::defaults_for_print()),
    };

    let default_config_path = get_configuration_file_path()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| String::from("(not found)"));
    eprintln!("Config file default path: {default_config_path}",);

    // We could use the toml crate to avoid doing the temp-file roundtrip, but doing it this way
    // means it's guaranteed to be good for `confy::load`.
    let tmp_path = std::env::temp_dir().join(format!("mdfried_tmp_config_{}", std::process::id()));
    confy::store_path(&tmp_path, user_config)?;
    let text = fs::read_to_string(&tmp_path)?;
    println!("{text}");
    fs::remove_file(tmp_path)?;
    Ok(())
}
