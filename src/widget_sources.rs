use std::{
    fmt::Debug,
    io::Cursor,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use font_kit::{
    canvas::{Canvas, Format, RasterizationOptions},
    font::Font,
    hinting::HintingOptions,
};
use image::{
    imageops, DynamicImage, GenericImage, ImageFormat, ImageReader, Pixel, Rgba, RgbaImage,
};
use pathfinder_geometry::transform2d::Transform2F;
use ratatui::{layout::Rect, text::Line};

use ratatui_image::{picker::Picker, protocol::Protocol, Resize};
use reqwest::{
    header::{HeaderMap, HeaderValue, ACCEPT, CONTENT_TYPE},
    Client,
};
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
    // renderer: &Renderer,
    bg: Option<[u8; 4]>,
    (font_width, font_height): (u16, u16),
    font: Font,
    picker: &Picker,
    width: u16,
    id: SourceID,
    text: String,
    tier: u8,
    deep_fry_meme: bool,
) -> Result<Vec<WidgetSource<'a>>, Error> {
    eprintln!("HEADER ---------------------------------");
    eprintln!("{text}");
    eprintln!("       ---------------------------------");
    static TRANSPARENT_BACKGROUND: [u8; 4] = [0, 0, 0, 0];
    let bg = bg.unwrap_or(TRANSPARENT_BACKGROUND);

    const HEADER_ROW_COUNT: u16 = 2;
    // let (font_width, font_height) = font_size;

    let img_width = (width * font_width) as u32;
    let img_height = (HEADER_ROW_COUNT * font_height) as u32;

    let tier_scale = ((12 - tier) as f32) / 12.0f32;

    let scale = (font_height * HEADER_ROW_COUNT) as f32 * tier_scale;

    // let scale = Scale::uniform((font_height * HEADER_ROW_COUNT) as f32 * tier_scale);
    //
    // let v_metrics = renderer.font.v_metrics(scale);

    let words = text.split_whitespace();
    eprintln!("{} words", words.clone().count());

    let mut lines = vec![];
    let mut current_line = String::new();
    for word in words {
        let mut maybe_current_line = current_line.clone();
        if !maybe_current_line.is_empty() {
            maybe_current_line.push(' ');
        }
        maybe_current_line.push_str(word);

        let width: u32 = maybe_current_line
            .chars()
            .map(|ch| {
                let glyph = font.glyph_for_char(ch).ok_or(Error::NoFont).unwrap();
                font.raster_bounds(
                    glyph,
                    scale,
                    Transform2F::default(),
                    HintingOptions::None,
                    RasterizationOptions::GrayscaleAa,
                )
                .unwrap()
                .width() as u32
            })
            .sum();
        eprintln!("width iteration: {width}");

        if width <= img_width {
            current_line = maybe_current_line.clone();
        } else {
            // glyphs_line = renderer
            // .font
            // .layout(&current_line, scale, point(0.0, 0.0 + v_metrics.ascent))
            // .collect();
            lines.push(current_line.clone());
            current_line = word.to_string();
        }
    }

    if !current_line.is_empty() {
        lines.push(current_line);
    }

    let mut sources = vec![];

    let max_x = img_width;
    let max_y = img_height;
    for line in lines {
        let img: RgbaImage = RgbaImage::from_pixel(img_width, img_height, Rgba(bg));
        let mut dyn_img = image::DynamicImage::ImageRgba8(img);

        eprintln!("line: {line:?}");
        let mut offset_x = 0;
        for ch in line.chars() {
            let glyph = font.glyph_for_char(ch).ok_or(Error::NoFont).unwrap();
            let raster_rect = font
                .raster_bounds(
                    glyph,
                    scale,
                    Transform2F::default(),
                    HintingOptions::None,
                    RasterizationOptions::GrayscaleAa,
                )
                .unwrap();
            let mut canvas = Canvas::new(raster_rect.size(), Format::A8);
            font.rasterize_glyph(
                &mut canvas,
                glyph,
                scale,
                // Transform2F::default(),
                Transform2F::from_translation(-raster_rect.origin().to_f32()),
                HintingOptions::None,
                RasterizationOptions::GrayscaleAa,
            )
            .unwrap();
            eprintln!("char: {ch:?}");
            eprintln!(
                "rect: {raster_rect:?} {}x{}",
                raster_rect.width(),
                raster_rect.height()
            );
            let offset_y = (img_height as i32) - raster_rect.height();
            for y in 0..raster_rect.height() {
                let (row_start, row_end) =
                    (y as usize * canvas.stride, (y + 1) as usize * canvas.stride);
                let row = &canvas.pixels[row_start..row_end];
                for x in 0..raster_rect.width() {
                    match canvas.format {
                        Format::A8 => {
                            let u8v = row[x as usize];
                            let mut pixel = Rgba(bg);
                            pixel.blend(&Rgba([u8v, u8v, u8v, u8v]));
                            dyn_img.put_pixel(
                                (offset_x as u32) + x as u32,
                                (offset_y as u32) + y as u32,
                                pixel,
                            );
                        }
                        _ => unimplemented!(),
                    }
                }
            }
            offset_x += raster_rect.width();
        }
        // for glyph in word_glyphs {
        // if let Some(bounding_box) = glyph.pixel_bounding_box() {
        // let mut outside = false;
        // let bb_x = bounding_box.min.x as u32;
        // let bb_y = bounding_box.min.y as u32;
        // glyph.draw(|x, y, v| match (bb_x.checked_add(x), bb_y.checked_add(y)) {
        // (Some(p_x), Some(p_y)) if (p_x) < max_x && p_y < max_y => {
        // let u8v = (255.0 * v) as u8;
        // let mut pixel = Rgba(bg);
        // pixel.blend(&Rgba([u8v, u8v, u8v, u8v]));
        // dyn_img.put_pixel(p_x, p_y, pixel);
        // }
        // _ => outside = true,
        // });
        // if outside {
        // break;
        // }
        // }
        // }

        if deep_fry_meme {
            dyn_img = deep_fry(dyn_img, &font);
        }

        let proto = picker.new_protocol(
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

#[allow(clippy::too_many_arguments)]
pub async fn image_source<'a>(
    picker: &Picker,
    width: u16,
    basepath: &Option<PathBuf>,
    client: Arc<RwLock<Client>>,
    id: SourceID,
    url: &str,
    deep_fry_meme: bool,
    font: &Font,
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
        dyn_img = deep_fry(dyn_img, font);
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

fn deep_fry(mut dyn_img: DynamicImage, _font: &Font) -> DynamicImage {
    let width = dyn_img.width();
    let height = dyn_img.height();
    dyn_img = dyn_img.adjust_contrast(50.0);
    dyn_img = dyn_img.huerotate(45);

    let down_width = (width as f32 * 0.9) as u32;
    let down_height = (height as f32 * 0.8) as u32;
    dyn_img = dyn_img.resize(down_width, down_height, imageops::FilterType::Gaussian);
    dyn_img = dyn_img.resize(width, height, imageops::FilterType::Nearest);

    let mut deep_fried = dyn_img.to_rgba8();
    let mut seed: i32 = 42;

    for pixel in deep_fried.pixels_mut() {
        // Boost color intensities and add artifacts
        let mut r = pixel[0] as f32;
        let mut g = pixel[1] as f32;
        let mut b = pixel[2] as f32;

        // Exaggerate color values
        r = (r * 1.5).min(255.0);
        g = (g * 1.5).min(255.0);
        b = (b * 1.5).min(255.0);

        // Add "random" noise for "deep fried" effect
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let noise = (seed % 30) as f32;

        r = (r + noise).min(255.0);
        g = (g + noise).min(255.0);
        b = (b + noise).min(255.0);

        *pixel = Rgba([r as u8, g as u8, b as u8, pixel[3]]);
    }

    DynamicImage::ImageRgba8(deep_fried)
}
