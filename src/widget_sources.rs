use std::path::Path;

use image::{imageops, DynamicImage, GenericImage, Pixel, Rgb, RgbImage, Rgba};
use ratatui::{
    layout::Rect,
    text::{Span, Text},
};

use ratatui_image::{picker::Picker, protocol::Protocol, Resize};
use rusttype::{point, Font, Scale};

use crate::Error;

pub struct WidgetSource<'a> {
    pub height: u16,
    pub source: WidgetSourceData<'a>,
}

pub enum WidgetSourceData<'a> {
    Image(Protocol),
    Text(Text<'a>),
}

pub fn header_source<'a>(
    picker: &mut Picker,
    font: &mut Font<'a>,
    bg: [u8; 3],
    width: u16,
    spans: Vec<Span>,
    tier: u8,
    deep_fry_meme: bool,
) -> Result<WidgetSource<'a>, Error> {
    let cell_height = 2;
    let (font_width, font_height) = picker.font_size();
    let img_width = (width * font_width) as u32;
    let img_height = (cell_height * font_height) as u32;
    let img: RgbImage = RgbImage::from_pixel(img_width, img_height, Rgb(bg));
    let mut dyn_img = image::DynamicImage::ImageRgb8(img);

    let s: String = spans.iter().map(|s| s.to_string()).collect();
    let tier_scale = ((12 - tier) as f32) / 12.0f32;
    let scale = Scale::uniform((font_height * cell_height) as f32 * tier_scale);
    let v_metrics = font.v_metrics(scale);
    let glyphs: Vec<_> = font
        .layout(&s, scale, point(0.0, 0.0 + v_metrics.ascent))
        .collect();

    let max_x = img_width as i32;
    let max_y = img_height as i32;
    for glyph in glyphs {
        if let Some(bounding_box) = glyph.pixel_bounding_box() {
            let mut outside = false;
            let bb_x = bounding_box.min.x;
            let bb_y = bounding_box.min.y;
            glyph.draw(|x, y, v| {
                let p_x = bb_x + (x as i32);
                let p_y = bb_y + (y as i32);
                if p_x >= max_x || p_y >= max_y {
                    outside = true;
                } else {
                    let u8v = (255.0 * v) as u8;
                    let mut pixel = Rgba([bg[0], bg[1], bg[2], 255]);
                    pixel.blend(&Rgba([u8v, u8v, u8v, u8v]));
                    dyn_img.put_pixel(p_x as u32, p_y as u32, pixel);
                }
            });
            if outside {
                break;
            }
        }
    }

    if deep_fry_meme {
        dyn_img = deep_fry(dyn_img);
    }

    let proto = picker
        .new_protocol(
            dyn_img,
            Rect::new(0, 0, width, cell_height),
            Resize::Fit(None),
        )
        .unwrap();

    Ok(WidgetSource {
        height: cell_height,
        source: WidgetSourceData::Image(proto),
    })
}

pub fn image_source<'a>(
    picker: &mut Picker,
    width: u16,
    basepath: Option<&Path>,
    link: &str,
    deep_fry_meme: bool,
) -> Result<WidgetSource<'a>, Error> {
    let link: String = if basepath.is_some() && link.starts_with("./") {
        let joined = basepath.unwrap().join(link);
        joined.to_str().unwrap_or(link).to_owned()
    } else {
        link.to_string()
    };
    let mut dyn_img = image::ImageReader::open(link)?.decode()?;
    if deep_fry_meme {
        dyn_img = deep_fry(dyn_img);
    }

    let height: u16 = 10;

    let proto = picker
        .new_protocol(dyn_img, Rect::new(0, 0, width, height), Resize::Fit(None))
        .unwrap();
    Ok(WidgetSource {
        height,
        source: WidgetSourceData::Image(proto),
    })
}

fn deep_fry(mut dyn_img: DynamicImage) -> DynamicImage {
    let width = dyn_img.width();
    let height = dyn_img.height();
    dyn_img = dyn_img.adjust_contrast(100.0);
    dyn_img = dyn_img.huerotate(45);

    // for x in 0..img_width {
    // for y in 0..img_height {
    // if let Some(pixel) = dyn_img.get_pixel_mut(x, y).0.iter_mut().next() {
    // *pixel = (*pixel).saturating_add(rand::random::<u8>() % 50);
    // }
    // }
    // }

    dyn_img = dyn_img.resize(width / 4, height / 4, imageops::FilterType::Nearest);
    dyn_img = dyn_img.resize(width, height, imageops::FilterType::Nearest);
    dyn_img
}
