use cosmic_text::{FontSystem, SwashCache};
use image::Rgba;
use ratatui_image::{
    FontSize,
    picker::{Capability, Picker, ProtocolType, cap_parser::QueryStdioOptions},
};

use crate::{CONFIG, config::Config, error::Error, fontpicker::interactive_font_picker};

#[derive(Default, Clone, Copy)]
pub struct BgColor([u8; 4]);

impl From<BgColor> for Rgba<u8> {
    fn from(value: BgColor) -> Self {
        Rgba(value.0)
    }
}

impl From<BgColor> for ratatui::style::Color {
    fn from(value: BgColor) -> Self {
        ratatui::style::Color::Rgb(value.0[0], value.0[1], value.0[2])
    }
}

pub struct FontRenderer {
    pub font_size: FontSize, // Terminal font-size, not rendered font-size.
    pub font_name: String,
    pub font_system: FontSystem,
    pub swash_cache: SwashCache,
}

impl FontRenderer {
    pub fn new(
        font_system: FontSystem,
        swash_cache: SwashCache,
        font_name: String,
        font_size: FontSize,
    ) -> Self {
        FontRenderer {
            font_size,
            font_name,
            font_system,
            swash_cache,
        }
    }
}

pub enum SetupResult {
    Aborted,
    TextSizing(Picker, Option<BgColor>),
    Complete(Picker, Option<BgColor>, Box<FontRenderer>),
}

pub fn setup_graphics(
    font_family: Option<String>,
    force_font_setup: bool,
) -> Result<SetupResult, Error> {
    print!("Detecting supported graphics protocols...");
    let mut picker = Picker::from_query_stdio_with_options(QueryStdioOptions {
        text_sizing_protocol: true,
    })?;
    println!(" {:?}.", picker.protocol_type());

    let bg = match picker.protocol_type() {
        ProtocolType::Sixel => Some(BgColor([20, 0, 40, 255])),
        _ => {
            picker.set_background_color([0, 0, 0, 0]);
            None
        }
    };

    let has_text_size_protocol = picker
        .capabilities()
        .contains(&Capability::TextSizingProtocol);
    if has_text_size_protocol {
        return Ok(SetupResult::TextSizing(picker, bg));
    }

    let mut font_system = FontSystem::new();
    let db = font_system.db_mut();
    db.load_system_fonts();

    let all_font_families: Vec<String> = db
        .faces()
        .map(|faceinfo| faceinfo.families[0].0.clone())
        .collect();

    let config_font_family = font_family.and_then(|font_family| {
        // Ensure this font exists
        if all_font_families.contains(&font_family) {
            return Some(font_family);
        }
        println!("Configured font not found: {font_family}");
        None
    });

    let font_name = if let Some(mut font_family) = config_font_family {
        if force_font_setup {
            println!("Entering forced font setup");
            match interactive_font_picker(&mut picker, bg) {
                Ok(Some(setup_font_family)) => {
                    let new_config = Config {
                        font_family: Some(setup_font_family.clone()),
                        ..Default::default()
                    };
                    confy::store(CONFIG.0, CONFIG.1, new_config)?;
                    font_family = setup_font_family;
                }
                Ok(None) => return Ok(SetupResult::Aborted),
                Err(err) => return Err(err),
            }
        }
        font_family
    } else {
        println!("Entering one-time font setup");
        match interactive_font_picker(&mut picker, bg) {
            Ok(Some(font_family)) => {
                let new_config = Config {
                    font_family: Some(font_family.clone()),
                    ..Default::default()
                };
                confy::store(CONFIG.0, CONFIG.1, new_config)?;
                font_family
            }
            Ok(None) => return Ok(SetupResult::Aborted),
            Err(err) => return Err(err),
        }
    };

    let font_size = picker.font_size();
    Ok(SetupResult::Complete(
        picker,
        bg,
        Box::new(FontRenderer::new(
            font_system,
            SwashCache::new(),
            font_name,
            font_size,
        )),
    ))
}
