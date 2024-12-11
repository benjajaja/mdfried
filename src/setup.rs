use font_loader::system_fonts;

use ratatui_image::picker::{Picker, ProtocolType};
use rusttype::Font;

use crate::{config::Config, error::Error, fontpicker::set_up_font, CONFIG};

pub fn setup_imagery<'a>(
    font_family: Option<String>,
    force_font_setup: bool,
) -> Result<(Picker, Font<'a>, Option<[u8; 4]>), Error> {
    print!("Detecting supported graphics protocols...");
    let mut picker = Picker::from_query_stdio().map_err(|err| Error::Msg(format!("{err}")))?;
    println!(" {:?}.", picker.protocol_type());

    let bg = match picker.protocol_type() {
        ProtocolType::Sixel => Some([20, 0, 40, 255]),
        _ => {
            picker.set_background_color([0, 0, 0, 0]);
            None
        }
    };

    let mut fp_builder = system_fonts::FontPropertyBuilder::new();

    let all_fonts = system_fonts::query_all();

    let config_font_family = font_family.and_then(|font_family| {
        // Ensure this font exists
        if all_fonts.contains(&font_family) {
            return Some(font_family);
        }
        None
    });

    let font_family = if let Some(mut font_family) = config_font_family {
        if force_font_setup {
            println!("Entering forced font setup");
            match set_up_font(&mut picker, bg) {
                Ok(setup_font_family) => {
                    let new_config = Config {
                        font_family: Some(font_family.clone()),
                        ..Default::default()
                    };
                    confy::store(CONFIG.0, CONFIG.1, new_config)?;
                    font_family = setup_font_family;
                }
                Err(err) => return Err(err),
            }
        }
        font_family
    } else {
        println!("Entering one-time font setup");
        match set_up_font(&mut picker, bg) {
            Ok(font_family) => {
                let new_config = Config {
                    font_family: Some(font_family.clone()),
                    ..Default::default()
                };
                confy::store(CONFIG.0, CONFIG.1, new_config)?;
                font_family
            }
            Err(err) => return Err(err),
        }
    };

    fp_builder = fp_builder.family(&font_family);

    let property = fp_builder.build();

    let (font_data, _) =
        system_fonts::get(&property).ok_or("Could not get system fonts property")?;

    let font = Font::try_from_vec(font_data).ok_or(Error::NoFont)?;
    Ok((picker, font, bg))
}
