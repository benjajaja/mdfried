use std::{io::Cursor, path::PathBuf};

use image::{
    imageops, DynamicImage, GenericImage, ImageFormat, ImageReader, Pixel, Rgba, RgbaImage,
};
use ratatui::{
    layout::Rect,
    text::{Span, Text},
};

use ratatui_image::{picker::Picker, protocol::Protocol, Resize};
use reqwest::{
    blocking::Client,
    header::{HeaderMap, HeaderValue, ACCEPT, CONTENT_TYPE},
};
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

pub fn header_source<'a, 'b>(
    picker: &mut Picker,
    font: &mut Font<'a>,
    bg: Option<[u8; 4]>,
    width: u16,
    spans: Vec<Span>,
    tier: u8,
    deep_fry_meme: bool,
) -> Result<WidgetSource<'b>, Error> {
    static TRANSPARENT_BACKGROUND: [u8; 4] = [0, 0, 0, 0];
    let bg = bg.unwrap_or(TRANSPARENT_BACKGROUND);

    let cell_height = 2;
    let (font_width, font_height) = picker.font_size();
    let img_width = (width * font_width) as u32;
    let img_height = (cell_height * font_height) as u32;
    let img: RgbaImage = RgbaImage::from_pixel(img_width, img_height, Rgba(bg));
    let mut dyn_img = image::DynamicImage::ImageRgba8(img);

    let s: String = spans.iter().map(|s| s.to_string()).collect();
    let tier_scale = ((12 - tier) as f32) / 12.0f32;
    let scale = Scale::uniform((font_height * cell_height) as f32 * tier_scale);
    let v_metrics = font.v_metrics(scale);
    let glyphs: Vec<_> = font
        .layout(&s, scale, point(0.0, 0.0 + v_metrics.ascent))
        .collect();

    let max_x = img_width as u64;
    let max_y = img_height as u64;
    for glyph in glyphs {
        if let Some(bounding_box) = glyph.pixel_bounding_box() {
            let mut outside = false;
            let bb_x = bounding_box.min.x as u64;
            let bb_y = bounding_box.min.y as u64;
            glyph.draw(|x, y, v| {
                let p_x = bb_x.checked_add(x as u64);
                let p_y = bb_y.checked_add(y as u64);
                if p_x.is_none() || p_x.unwrap() >= max_x || p_y.is_none() || p_y.unwrap() >= max_y
                {
                    outside = true;
                } else {
                    let u8v = (255.0 * v) as u8;
                    let mut pixel = Rgba(bg);
                    pixel.blend(&Rgba([u8v, u8v, u8v, u8v]));
                    dyn_img.put_pixel(p_x.unwrap() as u32, p_y.unwrap() as u32, pixel);
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
    basepath: Option<PathBuf>,
    client: &mut Client,
    link: &str,
    deep_fry_meme: bool,
) -> Result<WidgetSource<'a>, Error> {
    let mut dyn_img = if link.starts_with("https://") || link.starts_with("http://") {
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT, HeaderValue::from_static("image/png,image/jpg")); // or "image/jpeg"
        let response = client.get(link).headers(headers).send()?;
        if !response.status().is_success() {
            return Err(Error::Msg("bad HTTP status".to_string()));
        }
        let ct = response
            .headers()
            .get(CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        let bytes = response.bytes()?;
        let format = match ct.as_str() {
            "image/jpeg" => Ok(ImageFormat::Jpeg),
            "image/png" => Ok(ImageFormat::Png),
            "image/webp" => Ok(ImageFormat::WebP),
            _ => Err(Error::Msg(format!("unhandled Content-Type: {ct}"))),
        }?;

        ImageReader::with_format(Cursor::new(bytes), format).decode()?
    } else {
        let link: String = if basepath.is_some() && link.starts_with("./") {
            let joined = basepath.unwrap().join(link);
            joined.to_str().unwrap_or(link).to_owned()
        } else {
            link.to_string()
        };
        ImageReader::open(link)?.decode()?
    };
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

    let down_width = (width as f32 * 0.9) as u32;
    let down_height = (height as f32 * 0.8) as u32;
    dyn_img = dyn_img.resize(down_width, down_height, imageops::FilterType::Gaussian);
    dyn_img = dyn_img.resize(width, height, imageops::FilterType::Nearest);

    dyn_img
}
