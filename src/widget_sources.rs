use std::{
    any::Any as _,
    fmt::{Debug, Display, Write as _},
    ops::{Deref, DerefMut},
    path::PathBuf,
    sync::Arc,
};

use itertools::Either;

use cosmic_text::{Attrs, Buffer, Color, Family, Metrics, Shaping};
use image::{
    DynamicImage, GenericImage as _, ImageFormat, ImageReader, Pixel as _, Rgba, RgbaImage,
    imageops,
};
use ratatui::{layout::Rect, text::Line, widgets::Widget};

use ratatui_image::{Resize, picker::Picker, protocol::Protocol};
use regex::{Match, Regex};
use reqwest::{
    Client,
    header::{ACCEPT, CONTENT_TYPE, HeaderMap, HeaderValue},
};
use tokio::sync::RwLock;
use unicode_width::UnicodeWidthStr as _;

use crate::{
    Error,
    cursor::CursorPointer,
    setup::{BgColor, FontRenderer},
};

#[derive(Default)]
pub struct WidgetSources<'a> {
    sources: Vec<WidgetSource<'a>>,
    updated_images: Vec<(u16, String, Protocol)>,
}

impl<'a> WidgetSources<'a> {
    pub fn push(&mut self, source: WidgetSource<'a>) {
        debug_assert!(
            !self.sources.iter().any(|s| s.id == source.id),
            "WidgetSources::push expects unique ids"
        );
        self.sources.push(source);
    }

    // Update widgets with a list by id
    pub fn update(&mut self, updates: Vec<WidgetSource<'a>>) {
        let Some(first_id) = updates.first().map(|s| s.id) else {
            log::error!("ineffective WidgetSources::update with empty list");
            return;
        };
        debug_assert!(
            updates[1..].iter().all(|s| s.id == first_id),
            "WidgetSources::update must be called with same id for in the one updates list"
        );

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

        if let Some((start, end)) = range {
            let splice = self.sources.splice(start..end, updates);
            for splice in splice {
                if let WidgetSourceData::Image(url, proto) = splice.data {
                    self.updated_images.push((splice.height, url, proto));
                }
            }
        } else if let Some(last) = self.sources.last()
            && last.id < first_id
        {
            log::debug!("Update source #{first_id} not found but id is higher than last source");
            for source in updates {
                self.sources.push(source);
            }
        } else {
            log::error!("Update source #{first_id} not found anymore: {updates:?}");
        }
    }

    pub fn replace(&mut self, id: SourceID, url: &str) -> Option<WidgetSource<'a>> {
        for source in &mut self.sources {
            if source.id < id {
                continue;
            }
            if let WidgetSourceData::Image(existing_url, _) = &source.data
                && *existing_url == url
            {
                let removed_image = std::mem::replace(
                    source,
                    WidgetSource {
                        id: source.id,
                        height: 1,
                        data: WidgetSourceData::Line(
                            Line::from(format!("![Replacing...]({url})")),
                            Vec::new(),
                        ),
                    },
                );
                log::debug!("search & replaced #{}: {}", source.id, source.data);
                return Some(removed_image);
            }
        }
        self.updated_images
            .iter()
            .position(|(_, stored_url, _)| stored_url == url)
            .map(|i| {
                let (height, url, proto) = self.updated_images.remove(i);
                WidgetSource {
                    id: 0, // will be overwritten by caller
                    height,
                    data: WidgetSourceData::Image(url, proto),
                }
            })
    }

    pub fn trim_last_source(&mut self, last_source_id: Option<usize>) {
        self.updated_images.clear();
        let Some(last_source_id) = last_source_id else {
            log::warn!("WidgetSources::trim without last_source_id, nothing parsed");
            return;
        };
        if let Some(last) = self.sources.last()
            && last.id == last_source_id
        {
            log::debug!("no trim needed");
            return;
        }
        if let Some(idx) = self
            .sources
            .iter()
            .position(|source| source.id == last_source_id)
        {
            log::debug!("trim: {idx} + 1");
            self.sources.truncate(idx + 1);
        }
    }

    pub fn get_y(&self, id: usize) -> i16 {
        let mut y = 0;
        for source in self.sources.iter() {
            if source.id == id {
                break;
            }
            y += source.height as i16;
        }
        y
    }

    pub fn find_first_cursor<'b, Iter: Iterator<Item = &'b WidgetSource<'b>>>(
        iter: Iter,
        target: FindTarget,
    ) -> Option<CursorPointer> {
        for source in iter {
            if let WidgetSourceData::Line(_, extras) = &source.data {
                for (i, extra) in extras.iter().enumerate() {
                    if target.matches(extra) {
                        return Some(CursorPointer {
                            id: source.id,
                            index: i,
                        });
                    }
                }
            }
        }
        None
    }

    pub fn find_next_cursor<'b, Iter: DoubleEndedIterator<Item = &'b WidgetSource<'b>>>(
        iter: Iter,
        current: &CursorPointer,
        mode: FindMode,
        target: FindTarget,
    ) -> Option<CursorPointer> {
        let iter = WidgetSources::flatten_sources(iter, &mode, &target);

        let mut found = false;
        let mut first = None;
        for pointer in iter {
            if pointer == *current {
                found = true;
            } else if found {
                return Some(pointer);
            } else if first.is_none() {
                first = Some(pointer.clone())
            }
        }
        first
    }

    fn flatten_sources<'b>(
        iter: impl DoubleEndedIterator<Item = &'b WidgetSource<'b>>,
        mode: &FindMode,
        target: &FindTarget,
    ) -> Either<impl Iterator<Item = CursorPointer>, impl Iterator<Item = CursorPointer>> {
        match mode {
            FindMode::Next => Either::Left(iter.flat_map(move |source| {
                WidgetSources::line_extras_to_cursor_pointers(source, mode, target)
            })),
            FindMode::Prev => Either::Right(iter.rev().flat_map(move |source| {
                WidgetSources::line_extras_to_cursor_pointers(source, mode, target)
            })),
        }
    }

    fn line_extras_to_cursor_pointers(
        source: &WidgetSource<'a>,
        mode: &FindMode,
        target: &FindTarget,
    ) -> Either<
        Either<impl Iterator<Item = CursorPointer>, impl Iterator<Item = CursorPointer>>,
        impl Iterator<Item = CursorPointer>,
    > {
        match mode {
            FindMode::Next => {
                if let WidgetSourceData::Line(_, extras) = &source.data {
                    let id = source.id;
                    Either::Left(Either::Left(
                        extras
                            .iter()
                            .enumerate()
                            .filter(|(_, extra)| target.matches(extra))
                            .map(move |(index, _)| CursorPointer { id, index }),
                    ))
                } else {
                    Either::Right(std::iter::empty())
                }
            }
            FindMode::Prev => {
                if let WidgetSourceData::Line(_, extras) = &source.data {
                    let id = source.id;
                    Either::Left(Either::Right(
                        extras
                            .iter()
                            .enumerate()
                            .rev()
                            .filter(|(_, extra)| target.matches(extra))
                            .map(move |(index, _)| CursorPointer { id, index }),
                    ))
                } else {
                    Either::Right(std::iter::empty())
                }
            }
        }
    }

    #[cfg(test)]
    pub fn find_extra_by_cursor(&self, pointer: &CursorPointer) -> Option<&LineExtra> {
        for source in self.iter() {
            if source.id != pointer.id {
                continue;
            }
            let WidgetSourceData::Line(_, extras) = &source.data else {
                continue;
            };
            if let Some(extra) = extras.get(pointer.index) {
                return Some(extra);
            }
        }
        None
    }
}

impl<'a> Deref for WidgetSources<'a> {
    type Target = Vec<WidgetSource<'a>>;
    fn deref(&self) -> &Vec<WidgetSource<'a>> {
        &self.sources
    }
}

impl<'a> DerefMut for WidgetSources<'a> {
    // type Target = Vec<WidgetSource<'a>>;
    fn deref_mut(&mut self) -> &mut Vec<WidgetSource<'a>> {
        &mut self.sources
    }
}

#[derive(Debug)]
pub enum FindMode {
    Prev,
    Next,
}

#[derive(Debug)]
pub enum FindTarget {
    Link,
    Search,
}
impl FindTarget {
    fn matches(&self, extra: &LineExtra) -> bool {
        match self {
            FindTarget::Link => matches!(extra, LineExtra::Link(_, _, _)),
            FindTarget::Search => matches!(extra, LineExtra::SearchMatch(_, _, _)),
        }
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
    Image(String, Protocol),
    BrokenImage(String, String),
    Line(Line<'a>, Vec<LineExtra>),
    Header(String, u8),
}

impl WidgetSourceData<'_> {
    pub fn add_search(&mut self, re: &Option<Regex>) {
        if let WidgetSourceData::Line(line, extras) = self {
            let line_string = line.to_string();
            extras.retain(|extra| !matches!(extra, LineExtra::SearchMatch(_, _, _)));
            if let Some(re) = re {
                extras.extend(
                    re.find_iter(&line_string)
                        .map(WidgetSourceData::regex_to_searchmatch(&line_string)),
                );
            }
        }
        // TODO: search in headers
    }

    #[expect(clippy::string_slice)] // Regex byte ranges are guaranteed to fall between characters.
    fn regex_to_searchmatch(line_string: &str) -> impl Fn(Match<'_>) -> LineExtra {
        |m: Match| {
            // Convert from byte positions to character positions, with unicode_width.
            let start = line_string[..m.start()].width();
            let end = line_string[..m.end()].width();
            LineExtra::SearchMatch(start, end, m.as_str().to_owned())
        }
    }
}

impl PartialEq for WidgetSourceData<'_> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Image(l0, l1), Self::Image(r0, r1)) => l0 == r0 && l1.type_id() == r1.type_id(),
            (Self::BrokenImage(l0, l1), Self::BrokenImage(r0, r1)) => l0 == r0 && l1 == r1,
            (Self::Line(l0, l1), Self::Line(r0, r1)) => l0 == r0 && l1 == r1,
            (Self::Header(l0, l1), Self::Header(r0, r1)) => l0 == r0 && l1 == r1,
            _ => false,
        }
    }
}

impl Debug for WidgetSourceData<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Image(url, _) => f.debug_tuple(format!("Image({url})").as_str()).finish(),
            Self::BrokenImage(url, _) => f
                .debug_tuple(format!("BrokenImage({url})").as_str())
                .finish(),
            Self::Line(line, extra) => {
                let mut tuple = f.debug_tuple("Line");
                let mut tuple = tuple.field(line);
                if !extra.is_empty() {
                    tuple = tuple.field(extra);
                }
                tuple.finish()
            }
            Self::Header(text, tier) => f.debug_tuple("Header").field(text).field(tier).finish(),
        }
    }
}

impl Display for WidgetSourceData<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Image(url, protocol) => write!(f, "Image({url}, {:?})", protocol.type_id()),
            Self::BrokenImage(url, _) => write!(f, "BrokenImage({url})"),
            Self::Line(line, extra) => write!(f, "Line({}, {})", line, extra.len()),
            Self::Header(text, tier) => write!(f, "Header({text}, {tier})"),
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

    pub fn add_search(&mut self, re: &Option<Regex>) {
        self.data.add_search(re);
    }
}

#[cfg(test)]
impl Display for WidgetSource<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.data {
            WidgetSourceData::Image(_, _) => write!(f, "<image>"),
            WidgetSourceData::BrokenImage(_, _) => write!(f, "<broken-image>"),
            WidgetSourceData::Line(line, _) => Display::fmt(&line, f),
            WidgetSourceData::Header(text, tier) => {
                write!(f, "{} {}", "#".repeat(*tier as usize), text)
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum LineExtra {
    Link(String, u16, u16),
    SearchMatch(usize, usize, String),
}

/// Layout/shape and render `text` into a list of [`DynamicImage`] with a given terminal width.
pub fn header_images(
    bg: Option<BgColor>,
    font_renderer: &mut FontRenderer,
    width: u16,
    text: String,
    tier: u8,
    deep_fry_meme: bool,
) -> Result<Vec<(String, DynamicImage)>, Error> {
    let bg = bg.unwrap_or_default(); // Default is transparent (black, but that's irrelevant).

    const HEADER_ROW_COUNT: u16 = 2;
    let (font_width, font_height) = font_renderer.font_size;

    let tier_scale = f32::from(12 - tier) / 12.0_f32;

    let line_height = f32::from(font_height * HEADER_ROW_COUNT);
    let font_size = line_height * tier_scale;
    let metrics = Metrics::new(font_size, line_height);

    let mut buffer = Buffer::new(&mut font_renderer.font_system, metrics);

    let mut attrs = Attrs::new();
    attrs = attrs.family(Family::Name(&font_renderer.font_name));

    let max_width = width * font_width;
    buffer.set_size(
        &mut font_renderer.font_system,
        Some(f32::from(max_width)),
        None,
    );
    buffer.set_text(
        &mut font_renderer.font_system,
        &(if deep_fry_meme {
            text.replace('a', "🤣")
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
    let img_height = u32::from(font_height * 2);
    let img_width = u32::from(width * font_width);
    for layout_run in buffer.layout_runs() {
        let img: RgbaImage = RgbaImage::from_pixel(img_width, img_height, bg.into());
        let dyn_img = DynamicImage::ImageRgba8(img);
        dyn_imgs.push((layout_run.text.into(), dyn_img));
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
                || x >= i32::from(max_width)
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

            let dyn_img = &mut dyn_imgs[index].1;

            // Adjust picked image's Y coord offset.
            let y_offset: u32 = index as u32 * img_height;
            dyn_img.put_pixel(x as u32, y as u32 - y_offset, pixel);
        },
    );

    Ok(dyn_imgs)
}

const HEADER_ROW_COUNT: u16 = 2;

/// Render a list of images to [`WidgetSource`]s.
pub fn header_sources<'a>(
    picker: &Picker,
    width: u16,
    id: SourceID,
    dyn_imgs: Vec<(String, DynamicImage)>,
    deep_fry_meme: bool,
) -> Result<Vec<WidgetSource<'a>>, Error> {
    let mut sources = vec![];
    for (text, mut dyn_img) in dyn_imgs {
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
            data: WidgetSourceData::Image(text, proto),
        });
    }

    Ok(sources)
}

#[expect(clippy::too_many_arguments)]
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
            return Err(Error::UnknownImage(id, url.to_owned()));
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
            _ => Err(Error::UnknownImage(id, url.to_owned())),
        }?;

        ImageSource::Bytes(response.bytes().await?.to_vec(), format)
    } else {
        let path: String = match basepath {
            Some(basepath) if url.starts_with("./") => basepath
                .join(url)
                .to_str()
                .map(String::from)
                .unwrap_or(url.to_owned()),
            _ => url.to_owned(),
        };
        ImageSource::Path(path)
    };

    // Now do all the blocking stuff
    let picker = picker.clone();
    let url = String::from(url);
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
            data: WidgetSourceData::Image(url, proto),
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

    #[expect(clippy::cast_possible_truncation)]
    for pixel in deep_fried.pixels_mut() {
        // Boost color intensities and add artifacts
        let mut r = f32::from(pixel[0]);
        let mut g = f32::from(pixel[1]);
        let mut b = f32::from(pixel[2]);

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
        write!(symbol, "\x1b[{}X\x1B[?7l", area.width).expect("write to string");
        write!(symbol, "\x1b[1B").expect("write to string");
        write!(symbol, "\x1b[{}X\x1B[?7l", area.width).expect("write to string");
        write!(symbol, "\x1b[1A").expect("write to string");

        let (n, d) = match self.tier {
            1 => (1, 1),
            2 => (3, 4),
            3 => (7, 12),
            4 => (1, 2),
            5 => (5, 12),
            _ => (1, 3),
        };
        // Start the Text Size Protocol sequence.
        write!(symbol, "\x1b]66;s=2:n={n}:d={d};").expect("write to string");
        symbol.push_str(truncate_str(self.text, (area.width / 2) as usize));
        write!(symbol, "\x1b\x5c").expect("write to string"); // Could also use BEL, but this seems safer.

        // Skip entire text area except first cell
        let mut skip_first = false;

        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                if skip_first {
                    buf.cell_mut((x, y)).map(|cell| cell.set_skip(true));
                } else {
                    skip_first = true;
                    buf.cell_mut((x, y)).map(|cell| cell.set_symbol(&symbol));
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

    #[expect(clippy::string_slice)] // using char_indices here.
    &s[..end]
}

#[cfg(test)]
mod tests {

    use regex::Regex;

    use crate::{widget_sources::WidgetSources, *};

    #[test]
    fn widgestsources_update() {
        let mut ws = WidgetSources::default();
        ws.push(WidgetSource {
            id: 0,
            height: 2,
            data: WidgetSourceData::Line(Line::from("line #0"), Vec::new()),
        });
        ws.push(WidgetSource {
            id: 1,
            height: 2,
            data: WidgetSourceData::Line(Line::from("headerline1 headerline2"), Vec::new()),
        });
        ws.push(WidgetSource {
            id: 2,
            height: 2,
            data: WidgetSourceData::Line(Line::from("line #2"), Vec::new()),
        });

        ws.update(vec![
            WidgetSource {
                id: 1,
                height: 2,
                data: WidgetSourceData::Header(String::from("headerline1"), 1),
            },
            WidgetSource {
                id: 1,
                height: 2,
                data: WidgetSourceData::Header(String::from("headerline2"), 1),
            },
        ]);
        assert_eq!(ws.sources.len(), 4);
        assert_eq!(0, ws.sources[0].id,);
        assert_eq!(1, ws.sources[1].id,);
        assert_eq!(
            WidgetSourceData::Header(String::from("headerline1"), 1),
            ws.sources[1].data
        );
        assert_eq!(1, ws.sources[2].id,);
        assert_eq!(
            WidgetSourceData::Header(String::from("headerline2"), 1),
            ws.sources[2].data
        );
        assert_eq!(2, ws.sources[3].id,);

        ws.update(vec![
            WidgetSource {
                id: 1,
                height: 2,
                data: WidgetSourceData::Header(String::from("headerline3"), 1),
            },
            WidgetSource {
                id: 1,
                height: 2,
                data: WidgetSourceData::Header(String::from("headerline4"), 1),
            },
        ]);
        assert_eq!(ws.sources.len(), 4);
        assert_eq!(0, ws.sources[0].id,);
        assert_eq!(1, ws.sources[1].id,);
        assert_eq!(
            WidgetSourceData::Header(String::from("headerline3"), 1),
            ws.sources[1].data
        );
        assert_eq!(1, ws.sources[2].id,);
        assert_eq!(
            WidgetSourceData::Header(String::from("headerline4"), 1),
            ws.sources[2].data
        );
        assert_eq!(2, ws.sources[3].id,);
    }

    #[test]
    fn get_y() {
        let mut ws = WidgetSources::default();
        ws.push(WidgetSource {
            id: 1,
            height: 2,
            data: WidgetSourceData::Header(String::from("one"), 1),
        });
        ws.push(WidgetSource {
            id: 2,
            height: 1,
            data: WidgetSourceData::Line(Line::from("line"), Vec::new()),
        });
        ws.push(WidgetSource {
            id: 3,
            height: 1,
            data: WidgetSourceData::Line(Line::from("line"), Vec::new()),
        });
        ws.push(WidgetSource {
            id: 4,
            height: 2,
            data: WidgetSourceData::Header(String::from("one"), 1),
        });
        ws.push(WidgetSource {
            id: 5,
            height: 1,
            data: WidgetSourceData::Line(Line::from("line"), Vec::new()),
        });
        assert_eq!(ws.get_y(1), 0);
        assert_eq!(ws.get_y(2), 2);
        assert_eq!(ws.get_y(3), 3);
        assert_eq!(ws.get_y(4), 4);
        assert_eq!(ws.get_y(5), 6);
    }

    #[test]
    fn add_search_offset() {
        let line = Line::from(vec![Span::from("▐").magenta(), Span::from(" hi")]);
        let mut wsd = WidgetSourceData::Line(line, Vec::new());
        wsd.add_search(&Regex::new("hi").ok());
        let WidgetSourceData::Line(_, extra) = wsd else {
            panic!("Line");
        };
        assert_eq!(extra[0], LineExtra::SearchMatch(2, 4, String::from("hi")));
    }
}
