#[cfg(test)]
use std::fmt::Display;
use std::{
    any::Any,
    fmt::{Debug, Write},
    ops::Deref,
    path::PathBuf,
    sync::Arc,
};

use cosmic_text::{Attrs, Buffer, Color, Family, Metrics, Shaping};
use image::{
    DynamicImage, GenericImage, ImageFormat, ImageReader, Pixel, Rgba, RgbaImage, imageops,
};
use ratatui::{layout::Rect, text::Line, widgets::Widget};

use ratatui_image::{Resize, picker::Picker, protocol::Protocol};
use reqwest::{
    Client,
    header::{ACCEPT, CONTENT_TYPE, HeaderMap, HeaderValue},
};
use tokio::sync::RwLock;

use crate::{
    Error,
    setup::{BgColor, FontRenderer},
};

#[derive(Default)]
pub struct WidgetSources<'a> {
    sources: Vec<WidgetSource<'a>>,
    cursor: Option<Cursor>,
}

#[derive(Debug)]
pub struct Cursor {
    // The WidgetSource (line(s))
    pub id: SourceID,
    // The matched part index (e.g. LineExtra::Link)
    // This should change when we add support for searching any text.
    pub index: usize,
}

impl<'a> WidgetSources<'a> {
    pub fn push(&mut self, source: WidgetSource<'a>) {
        self.sources.push(source);
    }

    // Update widgets with a list by id
    pub fn update(&mut self, updates: Vec<WidgetSource<'a>>) {
        let Some(first_id) = updates.first().map(|s| s.id) else {
            return;
        };
        debug_assert!(updates[1..].iter().all(|s| s.id == first_id));

        let mut range = None;

        for (i, source) in self.sources.iter().enumerate() {
            if source.id == first_id {
                range = match range {
                    None => Some((i, i + 1)),
                    Some((start, _)) => Some((start, i + 1)),
                };
            } else if range.is_some() {
                break; // Found the end of consecutive ID sources
            }
        }

        debug_assert!(range.is_some(), "Update #{first_id} not found anymore");

        if let Some((start, end)) = range {
            self.sources.splice(start..end, updates);
        }
    }

    pub fn is_cursor(&self, source: &WidgetSource) -> Option<&Cursor> {
        if let Some(Cursor { id, .. }) = self.cursor
            && id == source.id
        {
            self.cursor.as_ref()
        } else {
            None
        }
    }

    pub fn get_extra_by_cursor(&self) -> Option<&LineExtra> {
        if let Some((_, _, extra)) =
            WidgetSources::cursor_find(self.sources.iter(), &self.cursor, 0)
        {
            Some(extra)
        } else {
            None
        }
    }

    pub fn clear_cursor(&mut self) {
        self.cursor = None;
    }

    pub fn cursor_next(&mut self, visible_lines: (i16, i16)) {
        if let Some((source, index, _)) = WidgetSources::cursor_find(
            self.visible(visible_lines),
            &self.cursor,
            if self.cursor.is_some() { 1 } else { 0 },
        ) {
            self.cursor = Some(Cursor {
                id: source.id,
                index,
            });
        }
    }

    pub fn cursor_prev(&mut self, visible_lines: (i16, i16)) {
        if let Some((source, index, _)) = WidgetSources::cursor_find(
            self.visible(visible_lines)
                .collect::<Vec<&WidgetSource>>()
                .into_iter()
                .rev(),
            &self.cursor,
            if self.cursor.is_some() { -1 } else { 0 },
        ) {
            self.cursor = Some(Cursor {
                id: source.id,
                index,
            });
        }
    }

    fn cursor_find<'b>(
        iter: impl Iterator<Item = &'b WidgetSource<'b>>,
        cursor: &Option<Cursor>,
        next: i8,
    ) -> Option<(&'b WidgetSource<'b>, usize, &'b LineExtra)> {
        let mut found = false;
        for source in iter {
            if let WidgetSourceData::LineExtra(_, ref extras) = source.data {
                // We're reversing the sources outside, but then reversing the extras here.
                // This should be unified, flat_map (and reverse) sounds good, but we will have to
                // do text searches in just the source (not in extras).
                let mut extras: Vec<(usize, &LineExtra)> = extras.iter().enumerate().collect();
                if next == -1 {
                    extras.reverse();
                }

                for (i, extra) in extras {
                    match cursor {
                        None => {
                            return Some((source, i, extra));
                        }
                        Some(Cursor { id, index, .. }) => {
                            if next == 0 {
                                if source.id == *id && i == *index {
                                    return Some((source, i, extra));
                                }
                            } else if !found && source.id == *id && i == *index {
                                found = true;
                            } else if found {
                                return Some((source, i, extra));
                            }
                        }
                    }
                }
            }
        }
        None
    }

    fn visible(&self, (start_y, end_y): (i16, i16)) -> impl Iterator<Item = &'_ WidgetSource<'_>> {
        // Quick & dirty without allocations, we only need to reverse when user presses "up" and
        // there we can just allocate inline.
        let mut y = start_y;
        self.sources.iter().filter(move |source| {
            let include = y >= 0;
            y += source.height as i16;
            if y >= end_y {
                return false;
            }
            include
        })
    }
}

impl<'a> Deref for WidgetSources<'a> {
    type Target = Vec<WidgetSource<'a>>;
    fn deref(&self) -> &Vec<WidgetSource<'a>> {
        &self.sources
    }
}

pub type SourceID = usize;

#[derive(Debug, PartialEq)]
pub struct WidgetSource<'a> {
    pub id: SourceID,
    pub height: u16,
    pub data: WidgetSourceData<'a>,
}

pub enum WidgetSourceData<'a> {
    Image(Protocol),
    BrokenImage(String, String),
    Line(Line<'a>),
    LineExtra(Line<'a>, Vec<LineExtra>),
    SizedLine(String, u8),
}

impl<'a> PartialEq for WidgetSourceData<'a> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Image(l0), Self::Image(r0)) => l0.type_id() == r0.type_id(),
            (Self::BrokenImage(l0, l1), Self::BrokenImage(r0, r1)) => l0 == r0 && l1 == r1,
            (Self::Line(l0), Self::Line(r0)) => l0 == r0,
            (Self::LineExtra(l0, l1), Self::LineExtra(r0, r1)) => l0 == r0 && l1 == r1,
            (Self::SizedLine(l0, l1), Self::SizedLine(r0, r1)) => l0 == r0 && l1 == r1,
            _ => false,
        }
    }
}

impl Debug for WidgetSourceData<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Image(_) => f.debug_tuple("Image").finish(),
            Self::BrokenImage(_, _) => f.debug_tuple("BrokenImage").finish(),
            Self::Line(arg0) => f.debug_tuple("Line").field(arg0).finish(),
            Self::LineExtra(arg0, arg1) => {
                f.debug_tuple("LineExtra").field(arg0).field(arg1).finish()
            }
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
            data: WidgetSourceData::BrokenImage(url, text),
        }
    }
}

#[cfg(test)]
impl Display for WidgetSource<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.data {
            WidgetSourceData::Image(_) => write!(f, "<image>"),
            WidgetSourceData::BrokenImage(_, _) => write!(f, "<broken-image>"),
            WidgetSourceData::Line(line) => std::fmt::Display::fmt(&line, f),
            WidgetSourceData::LineExtra(line, _) => std::fmt::Display::fmt(&line, f),
            WidgetSourceData::SizedLine(text, tier) => {
                write!(f, "{} {}", "#".repeat(*tier as usize), text)
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum LineExtra {
    Link(String, u16, u16),
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
            data: WidgetSourceData::Image(proto),
        });
    }

    Ok(sources)
}

#[allow(clippy::too_many_arguments)]
pub async fn image_source<'a>(
    picker: &Arc<Picker>,
    max_height: u16,
    width: u16,
    basepath: &Option<PathBuf>,
    client: Arc<RwLock<Client>>,
    id: SourceID,
    url: &str,
    deep_fry_meme: bool,
) -> Result<WidgetSource<'a>, Error> {
    enum ImageSource {
        Bytes(Vec<u8>, ImageFormat),
        Path(String),
    }
    let image_source = if url.starts_with("https://") || url.starts_with("http://") {
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

        ImageSource::Bytes(response.bytes().await?.to_vec(), format)
    } else {
        let path: String = match basepath {
            Some(basepath) if url.starts_with("./") => basepath
                .join(url)
                .to_str()
                .map(String::from)
                .unwrap_or(url.to_string()),
            _ => url.to_string(),
        };
        ImageSource::Path(path)
    };

    // Now do all the blocking stuff
    let picker = picker.clone();
    let source = tokio::task::spawn_blocking(move || {
        let mut dyn_img = match image_source {
            ImageSource::Bytes(bytes, format) => {
                ImageReader::with_format(std::io::Cursor::new(bytes), format).decode()?
            }
            ImageSource::Path(path) => ImageReader::open(path)?.decode()?,
        };

        if deep_fry_meme {
            dyn_img = deep_fry(dyn_img);
        }

        let max_width: u16 = (max_height * 3 / 2).min(width);

        let proto = picker.new_protocol(
            dyn_img,
            Rect::new(0, 0, max_width, max_height),
            Resize::Fit(None),
        )?;

        let height = proto.area().height;
        Ok::<WidgetSource<'_>, Error>(WidgetSource {
            id,
            height,
            data: WidgetSourceData::Image(proto),
        })
    })
    .await??;
    Ok(source)
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

#[cfg(test)]
mod tests {
    use crate::{widget_sources::WidgetSources, *};

    #[test]
    fn test_widgestsources_update() {
        let mut ws = WidgetSources::default();
        ws.push(WidgetSource {
            id: 1,
            height: 2,
            data: WidgetSourceData::SizedLine(String::from("one"), 1),
        });
        ws.push(WidgetSource {
            id: 2,
            height: 2,
            data: WidgetSourceData::SizedLine(String::from("two"), 1),
        });
        ws.push(WidgetSource {
            id: 3,
            height: 2,
            data: WidgetSourceData::SizedLine(String::from("three"), 1),
        });

        ws.update(vec![WidgetSource {
            id: 2,
            height: 2,
            data: WidgetSourceData::SizedLine(String::from("two updated"), 1),
        }]);
        assert_eq!(ws.sources.len(), 3);
    }

    #[test]
    fn test_finds_multiple_links_per_line_next() {
        let mut ws = WidgetSources::default();
        ws.push(WidgetSource {
            id: 1,
            height: 1,
            data: WidgetSourceData::LineExtra(
                Line::from("http://a.com http://b.com"),
                vec![
                    LineExtra::Link("http://a.com".into(), 0, 11),
                    LineExtra::Link("http://b.com".into(), 12, 21),
                ],
            ),
        });
        ws.push(WidgetSource {
            id: 2,
            height: 1,
            data: WidgetSourceData::LineExtra(
                Line::from("http://c.com"),
                vec![LineExtra::Link("http://c.com".into(), 0, 11)],
            ),
        });

        ws.cursor_next((0, 30));
        let LineExtra::Link(url, ..) = ws.get_extra_by_cursor().unwrap();
        assert_eq!("http://a.com", url);

        ws.cursor_next((0, 30));
        let LineExtra::Link(url, ..) = ws.get_extra_by_cursor().unwrap();
        assert_eq!("http://b.com", url);

        ws.cursor_next((0, 30));
        let LineExtra::Link(url, ..) = ws.get_extra_by_cursor().unwrap();
        assert_eq!("http://c.com", url);
    }

    #[test]
    fn test_finds_multiple_links_per_line_prev() {
        let mut ws = WidgetSources::default();
        ws.push(WidgetSource {
            id: 1,
            height: 1,
            data: WidgetSourceData::LineExtra(
                Line::from("http://a.com http://b.com"),
                vec![
                    LineExtra::Link("http://a.com".into(), 0, 11),
                    LineExtra::Link("http://b.com".into(), 12, 21),
                ],
            ),
        });
        ws.push(WidgetSource {
            id: 2,
            height: 1,
            data: WidgetSourceData::LineExtra(
                Line::from("http://c.com"),
                vec![LineExtra::Link("http://c.com".into(), 0, 11)],
            ),
        });

        ws.cursor_prev((0, 30));
        let LineExtra::Link(url, ..) = ws.get_extra_by_cursor().unwrap();
        assert_eq!("http://c.com", url);

        ws.cursor_prev((0, 30));
        let LineExtra::Link(url, ..) = ws.get_extra_by_cursor().unwrap();
        assert_eq!("http://b.com", url);

        ws.cursor_prev((0, 30));
        let LineExtra::Link(url, ..) = ws.get_extra_by_cursor().unwrap();
        assert_eq!("http://a.com", url);
    }
}
