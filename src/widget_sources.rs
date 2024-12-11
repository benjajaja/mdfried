use std::{fmt::Debug, io::Cursor, path::PathBuf, sync::Arc};

use image::{
    imageops, DynamicImage, GenericImage, ImageFormat, ImageReader, Pixel, Rgba, RgbaImage,
};
use ratatui::{layout::Rect, text::Text};

use ratatui_image::{picker::Picker, protocol::Protocol, Resize};
use reqwest::{
    header::{HeaderMap, HeaderValue, ACCEPT, CONTENT_TYPE},
    Client,
};
use rusttype::{point, Font, Scale};

use crate::Error;

#[derive(Debug)]
pub struct WidgetSource<'a> {
    pub index: usize,
    pub height: u16,
    pub source: WidgetSourceData<'a>,
}

pub enum WidgetSourceData<'a> {
    Image(Protocol),
    Text(Text<'a>),
}

impl Debug for WidgetSourceData<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Image(_) => f.debug_tuple("Image").finish(),
            Self::Text(arg0) => f.debug_tuple("Text").field(arg0).finish(),
        }
    }
}

impl<'a> WidgetSource<'a> {
    pub fn image_unknown(index: usize, url: String, title: String) -> WidgetSource<'a> {
        WidgetSource {
            index,
            height: 1,
            source: WidgetSourceData::Text(format!("![{title}]({url})").into()),
        }
    }
}

pub fn header_source<'a>(
    picker: &mut Picker,
    font: Arc<Font<'_>>,
    bg: Option<[u8; 4]>,
    width: u16,
    index: usize,
    text: String,
    tier: u8,
    deep_fry_meme: bool,
) -> Result<WidgetSource<'a>, Error> {
    static TRANSPARENT_BACKGROUND: [u8; 4] = [0, 0, 0, 0];
    let bg = bg.unwrap_or(TRANSPARENT_BACKGROUND);

    let cell_height = 2;
    let (font_width, font_height) = picker.font_size();
    let img_width = (width * font_width) as u32;
    let img_height = (cell_height * font_height) as u32;
    let img: RgbaImage = RgbaImage::from_pixel(img_width, img_height, Rgba(bg));
    let mut dyn_img = image::DynamicImage::ImageRgba8(img);

    let tier_scale = ((12 - tier) as f32) / 12.0f32;
    let scale = Scale::uniform((font_height * cell_height) as f32 * tier_scale);
    let v_metrics = font.v_metrics(scale);
    let glyphs: Vec<_> = font
        .layout(&text, scale, point(0.0, 0.0 + v_metrics.ascent))
        .collect();

    let max_x = img_width as u64;
    let max_y = img_height as u64;
    for glyph in glyphs {
        if let Some(bounding_box) = glyph.pixel_bounding_box() {
            let mut outside = false;
            let bb_x = bounding_box.min.x as u64;
            let bb_y = bounding_box.min.y as u64;
            glyph.draw(
                |x, y, v| match (bb_x.checked_add(x as u64), bb_y.checked_add(y as u64)) {
                    (Some(p_x), Some(p_y)) if p_x < max_x && p_y < max_y => {
                        let u8v = (255.0 * v) as u8;
                        let mut pixel = Rgba(bg);
                        pixel.blend(&Rgba([u8v, u8v, u8v, u8v]));
                        dyn_img.put_pixel(p_x as u32, p_y as u32, pixel);
                    }
                    _ => outside = true,
                },
            );
            if outside {
                break;
            }
        }
    }

    if deep_fry_meme {
        dyn_img = deep_fry(dyn_img);
    }

    let proto = picker.new_protocol(
        dyn_img,
        Rect::new(0, 0, width, cell_height),
        Resize::Fit(None),
    )?;

    Ok(WidgetSource {
        index,
        height: cell_height,
        source: WidgetSourceData::Image(proto),
    })
}

pub async fn image_source<'a>(
    picker: &mut Picker,
    width: u16,
    basepath: &Option<PathBuf>,
    client: &mut Client,
    index: usize,
    link: &str,
    deep_fry_meme: bool,
) -> Result<WidgetSource<'a>, Error> {
    let mut dyn_img = if link.starts_with("https://") || link.starts_with("http://") {
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT, HeaderValue::from_static("image/png,image/jpg")); // or "image/jpeg"
        let response = client.get(link).headers(headers).send().await?;
        if !response.status().is_success() {
            return Err(Error::UnknownImage(index, link.to_string()));
        }
        let ct = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|h| h.to_str().ok());
        let format = match ct {
            Some("image/jpeg") => Ok(ImageFormat::Jpeg),
            Some("image/png") => Ok(ImageFormat::Png),
            Some("image/webp") => Ok(ImageFormat::WebP),
            Some("image/gif") => Ok(ImageFormat::Gif),
            _ => Err(Error::UnknownImage(index, link.to_string())),
        }?;

        let bytes = response.bytes().await?;
        ImageReader::with_format(Cursor::new(bytes), format).decode()?
    } else {
        let link: String = match basepath {
            Some(basepath) if link.starts_with("./") => basepath
                .join(link)
                .to_str()
                .map(String::from)
                .unwrap_or(link.to_string()),
            _ => link.to_string(),
        };
        ImageReader::open(link)?.decode()?
    };
    if deep_fry_meme {
        dyn_img = deep_fry(dyn_img);
    }

    let height: u16 = 10;

    let proto = picker.new_protocol(dyn_img, Rect::new(0, 0, width, height), Resize::Fit(None))?;

    Ok(WidgetSource {
        index,
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
