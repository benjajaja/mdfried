use std::sync::Arc;

use font_kit::{
    family_name::FamilyName, font::Font, handle::Handle, properties::Properties,
    source::SystemSource,
};
use ratatui_image::{
    picker::{Picker, ProtocolType},
    FontSize,
};
use tokio::sync::Mutex;

use crate::{config::Config, error::Error, fontpicker::interactive_font_picker, CONFIG};

type BgColor = Option<[u8; 4]>;

pub struct Renderer {
    pub picker: Picker,
    pub font_size: FontSize,
    pub font: Arc<Mutex<Font>>,
    pub bg: Option<[u8; 4]>,
}

impl Renderer {
    pub fn new(picker: Picker, font: Font, bg: Option<[u8; 4]>) -> Self {
        let font_size = picker.font_size();
        Renderer {
            picker,
            font_size,
            font: Arc::new(Mutex::new(font)),
            bg,
        }
    }
}

pub fn setup_graphics<'a>() -> Result<(Picker, BgColor), Error> {
    print!("Detecting supported graphics protocols...");
    let mut picker = Picker::from_query_stdio()?;
    println!(" {:?}.", picker.protocol_type());

    let bg = match picker.protocol_type() {
        ProtocolType::Sixel => Some([20, 0, 40, 255]),
        _ => {
            picker.set_background_color([0, 0, 0, 0]);
            None
        }
    };

    Ok((picker, bg))
}

pub fn setup_font<'a>(
    font_name: Option<String>,
    force_font_setup: bool,
    picker: &mut Picker,
    bg: BgColor,
) -> Result<Option<Handle>, Error> {
    let source = SystemSource::new();

    let config_font = font_name.and_then(|font_family| {
        // Ensure this font exists
        if let Ok(found) = source.select_best_match(
            &[FamilyName::Title(font_family.clone())],
            &Properties::new(),
        ) {
            return Some(found);
        }
        println!("Configured font not found: {font_family}");
        None
    });

    let font = match config_font {
        Some(handle) if !force_font_setup => handle, // .load(),
        _ => {
            println!("Entering font setup");
            match interactive_font_picker(&source, picker, bg) {
                Ok(Some((setup_handle, pattern))) => {
                    let new_config = Config {
                        font_family: Some(pattern),
                        ..Default::default()
                    };
                    confy::store(CONFIG.0, CONFIG.1, new_config)?;
                    setup_handle //.load()
                }
                Ok(None) => return Ok(None),
                Err(err) => return Err(err),
            }
        }
    };

    Ok(Some(font))
}
