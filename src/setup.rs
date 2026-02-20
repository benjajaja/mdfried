pub mod configpicker;
mod fontpicker;
pub mod notification;

use cosmic_text::{FontSystem, SwashCache};
use ratatui_image::{
    FontSize,
    picker::{Capability, Picker, ProtocolType, cap_parser::QueryStdioOptions},
};

use crate::{
    config::{self, UserConfig},
    error::Error,
};
use fontpicker::interactive_font_picker;

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
    TextSizing(Picker),
    AsciiArt(Picker),
    Complete(Picker, Box<FontRenderer>),
}

pub fn setup_graphics(
    config: &mut UserConfig,
    force_font_setup: bool,
    no_cap_checks: bool,
    debug_override_protocol_type: Option<ProtocolType>,
) -> Result<SetupResult, Error> {
    let mut picker = if no_cap_checks {
        Picker::halfblocks()
    } else {
        print!("Detecting supported graphics protocols...");
        let mut options = QueryStdioOptions::default();
        options.text_sizing_protocol = true;
        let picker = Picker::from_query_stdio_with_options(options)?;
        println!(" {:?}.", picker.protocol_type());
        picker
    };

    let has_text_size_protocol = picker
        .capabilities()
        .contains(&Capability::TextSizingProtocol);
    if has_text_size_protocol {
        return Ok(SetupResult::TextSizing(picker));
    }

    if picker.protocol_type() == ProtocolType::Halfblocks {
        return Ok(SetupResult::AsciiArt(picker));
    }

    let mut font_system = FontSystem::new();
    let db = font_system.db_mut();
    db.load_system_fonts();

    let all_font_families: Vec<String> = db
        .faces()
        .map(|faceinfo| faceinfo.families[0].0.clone())
        .collect();

    let config_font_family = if force_font_setup {
        println!("Forced font setup");
        None
    } else {
        config.font_family.as_ref().and_then(|font_family| {
            // Ensure this font exists
            if all_font_families.contains(font_family) {
                return Some(font_family);
            }
            println!("Configured font not found: {font_family}");
            None
        })
    };

    let font_name = match config_font_family {
        Some(font_family) => font_family.clone(),
        None => match interactive_font_picker(&mut picker) {
            Ok(Some(setup_font_family)) => {
                config::store_font_family(config, setup_font_family.clone())?;
                notification::interactive_notification("Font has been written to config file.")?;
                setup_font_family
            }
            Ok(None) => return Ok(SetupResult::Aborted),
            Err(err) => return Err(err),
        },
    };

    let font_size = picker.font_size();

    if let Some(debug_override_protocol_type) = debug_override_protocol_type {
        log::warn!("debug_override_protocol_type set to {debug_override_protocol_type:?}");
        picker.set_protocol_type(debug_override_protocol_type);
    }

    Ok(SetupResult::Complete(
        picker,
        Box::new(FontRenderer::new(
            font_system,
            SwashCache::new(),
            font_name,
            font_size,
        )),
    ))
}
