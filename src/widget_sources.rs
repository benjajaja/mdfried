use std::{fmt::Debug, io::Cursor, path::PathBuf, sync::Arc};

use image::{
    imageops, DynamicImage, GenericImage, ImageFormat, ImageReader, Pixel, Rgba, RgbaImage,
};
use ratatui::{layout::Rect, text::Line};

use ratatui_image::{picker::Picker, protocol::Protocol, Resize};
use reqwest::{
    header::{HeaderMap, HeaderValue, ACCEPT, CONTENT_TYPE},
    Client,
};
use rusttype::{point, PositionedGlyph, Scale};
use tokio::sync::RwLock;

use crate::{setup::Renderer, Error};

pub type SourceID = usize;

#[derive(Debug)]
pub struct WidgetSource<'a> {
    pub id: SourceID,
    pub height: u16,
    pub source: WidgetSourceData<'a>,
}

pub enum WidgetSourceData<'a> {
    Image(Protocol),
    BrokenImage(String, String),
    Line(Line<'a>),
}

impl Debug for WidgetSourceData<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Image(_) => f.debug_tuple("Image").finish(),
            Self::BrokenImage(_, _) => f.debug_tuple("BrokenImage").finish(),
            Self::Line(arg0) => f.debug_tuple("Text").field(arg0).finish(),
        }
    }
}

impl<'a> WidgetSource<'a> {
    pub fn image_unknown(id: SourceID, url: String, text: String) -> WidgetSource<'a> {
        WidgetSource {
            id,
            height: 1,
            source: WidgetSourceData::BrokenImage(url, text),
        }
    }
}

pub fn header_source<'a>(
    renderer: &Renderer<'a>,
    width: u16,
    id: SourceID,
    text: String,
    tier: u8,
    deep_fry_meme: bool,
) -> Result<Vec<WidgetSource<'a>>, Error> {
    static TRANSPARENT_BACKGROUND: [u8; 4] = [0, 0, 0, 0];
    let bg = renderer.bg.unwrap_or(TRANSPARENT_BACKGROUND);

    const HEADER_ROW_COUNT: u16 = 2;
    let (font_width, font_height) = renderer.font_size;

    let img_width = (width * font_width) as u32;
    let img_height = (HEADER_ROW_COUNT * font_height) as u32;

    let tier_scale = ((12 - tier) as f32) / 12.0f32;
    let scale = Scale::uniform((font_height * HEADER_ROW_COUNT) as f32 * tier_scale);

    let v_metrics = renderer.font.v_metrics(scale);

    let words = text.split_whitespace();

    let mut lines = vec![];
    let mut current_line = String::new();
    let mut glyphs_line: Vec<PositionedGlyph> = vec![];
    for word in words {
        let mut maybe_current_line = current_line.clone();
        if !maybe_current_line.is_empty() {
            maybe_current_line.push(' ');
        }
        maybe_current_line.push_str(word);

        glyphs_line = renderer
            .font
            .layout(
                &maybe_current_line,
                scale,
                point(0.0, 0.0 + v_metrics.ascent),
            )
            .collect();

        let width = glyphs_line
            .last()
            .and_then(|g| g.pixel_bounding_box())
            .map(|bb| bb.max.x)
            .unwrap_or(0) as u32;

        if width <= img_width {
            current_line = maybe_current_line;
        } else {
            glyphs_line = renderer
                .font
                .layout(&current_line, scale, point(0.0, 0.0 + v_metrics.ascent))
                .collect();
            lines.push(glyphs_line);
            glyphs_line = vec![];
            current_line = word.to_string();
        }
    }

    if !current_line.is_empty() {
        lines.push(glyphs_line);
    }

    let mut sources = vec![];

    let max_x = img_width;
    let max_y = img_height;
    for word_glyphs in lines {
        let img: RgbaImage = RgbaImage::from_pixel(img_width, img_height, Rgba(bg));
        let mut dyn_img = image::DynamicImage::ImageRgba8(img);

        for glyph in word_glyphs {
            if let Some(bounding_box) = glyph.pixel_bounding_box() {
                let mut outside = false;
                let bb_x = bounding_box.min.x as u32;
                let bb_y = bounding_box.min.y as u32;
                glyph.draw(|x, y, v| match (bb_x.checked_add(x), bb_y.checked_add(y)) {
                    (Some(p_x), Some(p_y)) if (p_x) < max_x && p_y < max_y => {
                        let u8v = (255.0 * v) as u8;
                        let mut pixel = Rgba(bg);
                        pixel.blend(&Rgba([u8v, u8v, u8v, u8v]));
                        dyn_img.put_pixel(p_x, p_y, pixel);
                    }
                    _ => outside = true,
                });
                if outside {
                    break;
                }
            }
        }

        if deep_fry_meme {
            dyn_img = deep_fry(dyn_img);
        }

        let proto = renderer.picker.new_protocol(
            dyn_img,
            Rect::new(0, 0, width, HEADER_ROW_COUNT),
            Resize::Fit(None),
        )?;
        sources.push(WidgetSource {
            id,
            height: HEADER_ROW_COUNT,
            source: WidgetSourceData::Image(proto),
        });
    }

    Ok(sources)
}

pub async fn image_source<'a>(
    picker: &Picker,
    width: u16,
    basepath: &Option<PathBuf>,
    client: Arc<RwLock<Client>>,
    id: SourceID,
    url: &str,
    deep_fry_meme: bool,
) -> Result<WidgetSource<'a>, Error> {
    let mut dyn_img = if url.starts_with("https://") || url.starts_with("http://") {
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT, HeaderValue::from_static("image/png,image/jpg")); // or "image/jpeg"
        let client = client.read().await;
        let response = client.get(url).headers(headers).send().await?;
        drop(client);
        if !response.status().is_success() {
            return Err(Error::UnknownImage(id, url.to_string()));
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
            _ => Err(Error::UnknownImage(id, url.to_string())),
        }?;

        let bytes = response.bytes().await?;
        ImageReader::with_format(Cursor::new(bytes), format).decode()?
    } else {
        let path: String = match basepath {
            Some(basepath) if url.starts_with("./") => basepath
                .join(url)
                .to_str()
                .map(String::from)
                .unwrap_or(url.to_string()),
            _ => url.to_string(),
        };
        ImageReader::open(path)?.decode()?
    };
    if deep_fry_meme {
        dyn_img = deep_fry(dyn_img);
    }

    let max_height: u16 = 20;
    let max_width: u16 = (max_height * 3 / 2).min(width);

    let proto = picker.new_protocol(
        dyn_img,
        Rect::new(0, 0, max_width, max_height),
        Resize::Fit(None),
    )?;

    let height = proto.area().height;
    Ok(WidgetSource {
        id,
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
