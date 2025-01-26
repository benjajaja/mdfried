use std::{collections::BTreeMap, fs::File, io::Read};

use ratatui_image::{
    picker::{Picker, ProtocolType},
    FontSize,
};
use rust_fontconfig::{FcFontCache, FcFontPath};
use rusttype::Font;

use crate::{config::Config, error::Error, fontpicker::interactive_font_picker, CONFIG};

pub struct Renderer<'a> {
    pub picker: Picker,
    pub font_size: FontSize,
    pub font: Font<'a>,
    pub bg: Option<[u8; 4]>,
}

impl<'a> Renderer<'a> {
    pub fn new(picker: Picker, font: Font<'a>, bg: Option<[u8; 4]>) -> Self {
        let font_size = picker.font_size();
        Renderer {
            picker,
            font_size,
            font,
            bg,
        }
    }
}

pub fn setup_graphics<'a>(
    font_family: Option<String>,
    force_font_setup: bool,
) -> Result<Option<Renderer<'a>>, Error> {
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

    let cache = FcFontCache::build();

    let all_font_families: BTreeMap<String, &FcFontPath> = cache
        .list()
        .iter()
        .filter_map(|(pattern, path)| pattern.family.clone().map(|family| (family, path)))
        .collect();

    let config_font_family = font_family.and_then(|font_family| {
        // Ensure this font exists
        if all_font_families.contains_key(&font_family) {
            return Some(font_family);
        }
        println!("Configured font not found: {font_family}");
        None
    });

    let font_family = if let Some(mut font_family) = config_font_family {
        if force_font_setup {
            println!("Entering forced font setup");
            match interactive_font_picker(&cache, &mut picker, bg) {
                Ok(Some(setup_font_family)) => {
                    let new_config = Config {
                        font_family: Some(setup_font_family.clone()),
                        ..Default::default()
                    };
                    confy::store(CONFIG.0, CONFIG.1, new_config)?;
                    font_family = setup_font_family;
                }
                Ok(None) => return Ok(None),
                Err(err) => return Err(err),
            }
        }
        font_family
    } else {
        println!("Entering one-time font setup");
        match interactive_font_picker(&cache, &mut picker, bg) {
            Ok(Some(font_family)) => {
                let new_config = Config {
                    font_family: Some(font_family.clone()),
                    ..Default::default()
                };
                confy::store(CONFIG.0, CONFIG.1, new_config)?;
                font_family
            }
            Ok(None) => return Ok(None),
            Err(err) => return Err(err),
        }
    };

    let result = all_font_families.get(&font_family).ok_or(Error::NoFont)?;

    let font = load_font(&result.path)?;

    Ok(Some(Renderer::new(picker, font, bg)))
}

pub fn load_font<'a>(path: &str) -> Result<Font<'a>, Error> {
    let mut file = File::open(path)?;
    let mut contents = vec![];
    file.read_to_end(&mut contents)?;
    Font::try_from_vec(contents).ok_or(Error::NoFont)
}
