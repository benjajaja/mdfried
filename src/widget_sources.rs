use std::{
    fmt::{Debug, Write},
    io::Cursor,
    path::PathBuf,
    sync::Arc,
};

use cosmic_text::{Attrs, Buffer, Color, Family, Metrics, Shaping};
use image::{
    imageops, DynamicImage, GenericImage, ImageFormat, ImageReader, Pixel, Rgba, RgbaImage,
};
use ratatui::{layout::Rect, text::Line, widgets::Widget};

use ratatui_image::{picker::Picker, protocol::Protocol, Resize};
use reqwest::{
    header::{HeaderMap, HeaderValue, ACCEPT, CONTENT_TYPE},
    Client,
};
use tokio::sync::RwLock;

use crate::{
    setup::{BgColor, FontRenderer},
    Error,
};

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
    SizedLine(String, u8),
}

impl Debug for WidgetSourceData<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Image(_) => f.debug_tuple("Image").finish(),
            Self::BrokenImage(_, _) => f.debug_tuple("BrokenImage").finish(),
            Self::Line(arg0) => f.debug_tuple("Line").field(arg0).finish(),
            Self::SizedLine(text, tier) => {
                f.debug_tuple("SizedLine").field(text).field(tier).finish()
            }
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

/// Layout/shape and render `text` into a list of [DynamicImage] with a given terminal width.
pub fn header_images(
    bg: Option<BgColor>,
    font_renderer: &mut FontRenderer,
    width: u16,
    text: String,
    tier: u8,
    deep_fry_meme: bool,
) -> Result<Vec<DynamicImage>, Error> {
    let bg = bg.unwrap_or_default(); // Default is transparent (black, but that's irrelevant).

    const HEADER_ROW_COUNT: u16 = 2;
    let (font_width, font_height) = font_renderer.font_size;

    let tier_scale = ((12 - tier) as f32) / 12.0f32;

    let line_height = (font_height * HEADER_ROW_COUNT) as f32;
    let font_size = line_height * tier_scale;
    let metrics = Metrics::new(font_size, line_height);

    let mut buffer = Buffer::new(&mut font_renderer.font_system, metrics);

    let mut attrs = Attrs::new();
    attrs = attrs.family(Family::Name(&font_renderer.font_name));

    let max_width = width * font_width;
    buffer.set_size(&mut font_renderer.font_system, Some(max_width as f32), None);
    buffer.set_text(
        &mut font_renderer.font_system,
        &(if deep_fry_meme {
            text.replace("a", "🤣")
        } else {
            text
        }),
        &attrs,
        Shaping::Advanced,
    );
    buffer.shape_until_scroll(&mut font_renderer.font_system, false);

    // Make one image per shaped line.
    let run_count = buffer.layout_runs().collect::<Vec<_>>().len();
    let mut dyn_imgs = Vec::with_capacity(run_count);
    let img_height = (font_height * 2) as u32;
    let img_width = (width * font_width) as u32;
    for _ in buffer.layout_runs() {
        let img: RgbaImage = RgbaImage::from_pixel(img_width, img_height, bg.into());
        let dyn_img = image::DynamicImage::ImageRgba8(img);
        dyn_imgs.push(dyn_img);
    }

    let fg = Color::rgba(255, 255, 255, 255);

    // Render shaped text, picking the image off the Vec by the Y coord.
    buffer.draw(
        &mut font_renderer.font_system,
        &mut font_renderer.swash_cache,
        fg,
        |x, y, w, h, color| {
            let a = color.a();
            if a == 0
                || x < 0
                || x >= max_width as i32
                || y < 0
                // || y >= ... // Just pick relevant dyn_img
                || w != 1
                || h != 1
            {
                // Ignore alphas of 0, or invalid x, y coordinates, or unimplemented sizes
                return;
            }

            // Pick image-index by Y coord.
            let index = (y / img_height as i32) as usize;

            if index >= dyn_imgs.len() {
                return;
            }

            // Blend pixel with background (likely transparent).
            let mut pixel: Rgba<u8> = bg.into();
            pixel.blend(&Rgba(color.as_rgba()));

            let dyn_img = &mut dyn_imgs[index];

            // Adjust picked image's Y coord offset.
            let y_offset: u32 = index as u32 * img_height;
            dyn_img.put_pixel(x as u32, y as u32 - y_offset, pixel);
        },
    );

    Ok(dyn_imgs)
}

const HEADER_ROW_COUNT: u16 = 2;

/// Render a list of images to [WidgetSource]s.
pub fn header_sources<'a>(
    picker: &Picker,
    width: u16,
    id: SourceID,
    dyn_imgs: Vec<DynamicImage>,
    deep_fry_meme: bool,
) -> Result<Vec<WidgetSource<'a>>, Error> {
    let mut sources = vec![];
    for mut dyn_img in dyn_imgs {
        if deep_fry_meme {
            dyn_img = deep_fry(dyn_img);
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

pub struct BigText<'a> {
    text: &'a str,
    tier: u8,
}

impl<'a> BigText<'a> {
    pub fn new(text: &'a str, tier: u8) -> Self {
        BigText { text, tier }
    }
}

impl Widget for BigText<'_> {
    fn render(self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let mut symbol = String::new();

        // Erase character dance.
        // We must erase anything inside area, which is 2 lines high and `area.width` wide.
        // This must be done before we write the text.
        // Also disable DECAWM, unsure if really necessary.
        write!(symbol, "\x1b[{}X\x1B[?7l", area.width).unwrap();
        write!(symbol, "\x1b[1B").unwrap();
        write!(symbol, "\x1b[{}X\x1B[?7l", area.width).unwrap();
        write!(symbol, "\x1b[1A").unwrap();

        let (n, d) = match self.tier {
            1 => (1, 1),
            2 => (3, 4),
            3 => (7, 12),
            4 => (1, 2),
            5 => (5, 12),
            _ => (1, 3),
        };
        // Start the Text Size Protocol sequence.
        write!(symbol, "\x1b]66;s=2:n={n}:d={d};").unwrap();
        symbol.push_str(truncate_str(self.text, (area.width / 2) as usize));
        write!(symbol, "\x1b\x5c").unwrap(); // Could also use BEL, but this seems safer.

        // Skip entire text area except first cell
        let mut skip_first = false;

        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                if !skip_first {
                    skip_first = true;
                    buf.cell_mut((x, y)).map(|cell| cell.set_symbol(&symbol));
                } else {
                    buf.cell_mut((x, y)).map(|cell| cell.set_skip(true));
                }
            }
        }
    }
}

fn truncate_str(s: &str, max_chars: usize) -> &str {
    if s.chars().count() <= max_chars {
        return s;
    }

    let mut end = 0;
    for (i, _) in s.char_indices().take(max_chars) {
        end = i;
    }

    &s[..end]
}
