use std::{fs, path::PathBuf, str::FromStr as _};

use confy::ConfyError;
use mdfrier::{Mapper, TableSymbol};
use ratatui::style::Color;
use ratatui_image::picker::ProtocolType;
use serde::{Deserialize, Serialize};

use crate::error::Error;

// The configuration struct used throughout the program.
//
// Has implicit `Default` in `From<UserConfig>`.
#[derive(Debug, Clone)]
pub struct Config {
    pub padding: Padding,
    pub max_image_height: u16,
    pub watch_debounce_milliseconds: u64,
    pub enable_mouse_capture: bool,
    pub osc8_links: bool,
    pub debug_override_protocol_type: Option<ProtocolType>,
    pub url_transform_command: Option<String>,
    pub theme: Theme,
    pub mermaid: MermaidConfig,
}

impl From<UserConfig> for Config {
    fn from(uc: UserConfig) -> Self {
        Config {
            padding: uc.padding.unwrap_or_default(),
            max_image_height: uc.max_image_height.unwrap_or(30),
            watch_debounce_milliseconds: uc.watch_debounce_milliseconds.unwrap_or(100),
            enable_mouse_capture: uc.enable_mouse_capture.unwrap_or(false),
            osc8_links: uc.osc8_links.unwrap_or(true),
            debug_override_protocol_type: uc.debug_override_protocol_type,
            url_transform_command: uc.url_transform_command,
            mermaid: uc.mermaid.unwrap_or_default(),
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
    pub stdio_query_timeout_ms: Option<i32>,
    pub ignore_text_sizing_protocol: Option<bool>,
    pub padding: Option<Padding>,
    pub max_image_height: Option<u16>,
    pub watch_debounce_milliseconds: Option<u64>,
    pub enable_mouse_capture: Option<bool>,
    pub osc8_links: Option<bool>,
    pub debug_override_protocol_type: Option<ProtocolType>,
    pub url_transform_command: Option<String>,
    pub theme: Option<Theme>,
    pub mermaid: Option<MermaidConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum MermaidConfig {
    Disabled,
    #[cfg(feature = "mermaid")]
    Builtin,
    Command(String),
}

#[expect(clippy::derivable_impls)]
impl Default for MermaidConfig {
    fn default() -> Self {
        #[cfg(feature = "mermaid")]
        {
            MermaidConfig::Builtin
        }
        #[cfg(not(feature = "mermaid"))]
        {
            MermaidConfig::Disabled
        }
    }
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
    pub preserve_list_ordinals: Option<bool>,
    pub hide_codeblock_language: Option<bool>,
    pub table_row_separators: Option<bool>,
    /// Exactly 11 table border symbols, one character each. In order: the corners (top-left,
    /// top-right, bottom-left, bottom-right), horizontal, vertical, the junctions (top, bottom,
    /// left, right), and cross.
    #[serde(default, deserialize_with = "deserialize_table_symbols")]
    pub table_symbols: Option<String>,
    pub table_proportional_columns: Option<bool>,
    /// Name of an arborium built-in theme for code blocks, e.g. "GitHub Light".
    pub code_theme: Option<String>,
}

/// The number of symbols in `table_symbols`: one per `TableSymbol`.
const TABLE_SYMBOLS_LEN: usize = 11;

fn deserialize_table_symbols<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let symbols = Option::<String>::deserialize(deserializer)?;
    if let Some(symbols) = &symbols {
        let len = symbols.chars().count();
        if len != TABLE_SYMBOLS_LEN {
            return Err(serde::de::Error::custom(format!(
                "table_symbols needs {TABLE_SYMBOLS_LEN} characters, found {len}"
            )));
        }
    }
    Ok(symbols)
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
    fn table_symbol(&self, symbol: TableSymbol) -> &str {
        self.table_symbols
            .as_deref()
            .and_then(|symbols| {
                let (i, c) = symbols.char_indices().nth(symbol as usize)?;
                symbols.get(i..i + c.len_utf8())
            })
            .unwrap_or_else(|| STYLED_MAPPER.table_symbol(symbol))
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
    fn preserve_list_ordinals(&self) -> bool {
        self.preserve_list_ordinals.unwrap_or(false)
    }
    fn hide_codeblock_language(&self) -> bool {
        self.hide_codeblock_language.unwrap_or(false)
    }
    fn table_row_separators(&self) -> bool {
        self.table_row_separators.unwrap_or(false)
    }
    fn table_proportional_columns(&self) -> bool {
        self.table_proportional_columns.unwrap_or(false)
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
            preserve_list_ordinals: Some(false),
            hide_codeblock_language: Some(false),
            table_row_separators: Some(false),
            table_symbols: Some("╭╮╰╯─│┬┴├┤┼".to_owned()),
            table_proportional_columns: Some(false),
            code_theme: Some(arborium::theme::builtin::tokyo_night().name),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum Padding {
    AlignLeft {
        #[serde(skip_serializing_if = "Option::is_none")]
        width: Option<u16>,
    },
    Centered {
        width: u16,
    },
}
impl Default for Padding {
    fn default() -> Self {
        Padding::Centered { width: 100 }
    }
}

impl Padding {
    pub fn calculate_width(&self, screen_width: u16) -> u16 {
        match self {
            Padding::AlignLeft { width: None } => screen_width,
            Padding::AlignLeft { width: Some(width) } => screen_width.min(*width),
            Padding::Centered { width } => screen_width.min(*width),
        }
    }

    pub fn calculate_height(&self, screen_height: u16) -> u16 {
        screen_height
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
        font_family: Some("your-font-name".to_owned()),
        stdio_query_timeout_ms: Some(2000),
        ignore_text_sizing_protocol: Some(false),
        padding: Some(config.padding),
        max_image_height: Some(config.max_image_height),
        watch_debounce_milliseconds: Some(config.watch_debounce_milliseconds),
        enable_mouse_capture: Some(config.enable_mouse_capture),
        osc8_links: Some(config.osc8_links),
        debug_override_protocol_type: None,
        url_transform_command: Some("readable | html2text".to_owned()),
        theme: Some(Theme::defaults_for_print()),
        mermaid: Some(MermaidConfig::Command("mmdc -i - -o - -e png".to_owned())),
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

#[cfg(test)]
mod tests {
    use mdfrier::{Mapper as _, TableSymbol};

    use super::Theme;

    fn parse(symbols: &str) -> Result<Theme, confy::ConfyError> {
        let path = std::env::temp_dir().join(format!(
            "mdfried_table_symbols_{}_{}",
            std::process::id(),
            symbols.chars().count()
        ));
        std::fs::write(&path, format!("table_symbols = \"{symbols}\"\n"))
            .map_err(confy::ConfyError::GeneralLoadError)?;
        let theme = confy::load_path(&path);
        let _: std::io::Result<()> = std::fs::remove_file(&path);
        theme
    }

    #[test]
    fn table_symbols_default_is_rounded() {
        let theme = Theme::default();
        assert_eq!(theme.table_symbol(TableSymbol::TopLeft), "╭");
        assert_eq!(theme.table_symbol(TableSymbol::Cross), "┼");
    }

    #[test]
    fn table_symbols_override() {
        let theme = parse("┌┐└┘─│┬┴├┤┼").expect("valid table_symbols");
        assert_eq!(theme.table_symbol(TableSymbol::TopLeft), "┌");
        assert_eq!(theme.table_symbol(TableSymbol::Horizontal), "─");
        assert_eq!(theme.table_symbol(TableSymbol::Cross), "┼");
    }

    #[test]
    fn table_symbols_must_be_exactly_11_characters() {
        assert!(parse("╭╮╰╯").is_err(), "too few");
        assert!(parse("╭╮╰╯─│┬┴├┤┼+").is_err(), "too many");
        assert!(parse("┌┐└┘─ │ ┬ ┴ ├ ┤ ┼").is_err(), "spaced");
    }
}
