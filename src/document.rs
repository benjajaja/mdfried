use std::{
    any::Any as _,
    fmt::{Debug, Display},
    num::NonZero,
    ops::{Deref, DerefMut},
    path::PathBuf,
    sync::Arc,
};

use itertools::Either;

use cosmic_text::{Attrs, Buffer, Family, Metrics, Shaping};
use image::{
    DynamicImage, GenericImage as _, ImageFormat, ImageReader, Rgba, RgbaImage,
    imageops::FilterType,
};
use mdfrier::{MarkdownLink, SourceContent};
use ratatui::{layout::Rect, text::Line};

use ratatui_image::{
    FontSize, Resize,
    picker::{Picker, ProtocolType},
    protocol::Protocol,
};
use regex::{Match, Regex};
use reqwest::{
    Client,
    header::{ACCEPT, CONTENT_TYPE, HeaderMap, HeaderValue},
};
use tokio::sync::RwLock;
use unicode_width::UnicodeWidthStr as _;

use crate::{Error, cursor::CursorPointer, setup::FontRenderer, worker::ImageCache};

#[derive(Default)]
pub struct Document {
    sections: Vec<Section>,
}

impl Document {
    pub fn push(&mut self, section: Section) {
        debug_assert!(
            !self.sections.iter().any(|s| s.id == section.id),
            "Document::push expects unique ids"
        );
        self.sections.push(section);
    }

    // Update widgets with a list by id
    pub fn update(&mut self, updates: Vec<Section>) {
        let Some(first_id) = updates.first().map(|s| s.id) else {
            log::error!("ineffective Document::update with empty list");
            return;
        };
        debug_assert!(
            updates[1..].iter().all(|s| s.id == first_id),
            "Document::update must be called with same id for in the one updates list"
        );

        let mut range = None;

        for (i, section) in self.sections.iter().enumerate() {
            if section.id == first_id {
                range = match range {
                    None => Some((i, i + 1)),
                    Some((start, _)) => Some((start, i + 1)),
                };
            } else if range.is_some() {
                break; // Found the end of consecutive ID sections
            }
        }

        if let Some((start, end)) = range {
            self.sections.splice(start..end, updates);
        } else if let Some(last) = self.sections.last()
            && last.id < first_id
        {
            log::debug!("Update section #{first_id} not found but id is higher than last section");
            for section in updates {
                self.sections.push(section);
            }
        } else {
            log::error!("Update section #{first_id} not found anymore: {updates:?}");
        }
    }

    /// Update a specific image line within a section with loaded image data.
    pub fn update_image(
        &mut self,
        section_id: SectionID,
        link: MarkdownLink,
        protos: ProtocolWrapper,
    ) {
        let Some(section) = self.sections.iter_mut().find(|s| s.id == section_id) else {
            log::error!("update_image: section #{section_id} not found");
            return;
        };

        *section = Section {
            id: section_id,
            height: protos.height(),
            content: SectionContent::Image(link, protos),
        };
    }

    pub fn update_header(&mut self, section_id: SectionID, rows: Vec<(String, u8, Protocol)>) {
        if rows.is_empty() {
            log::error!("update_header: empty rows for section #{section_id}");
            return;
        }

        let new_sections: Vec<Section> = rows
            .into_iter()
            .map(|(text, tier, proto)| {
                log::debug!("update_header: {text}");
                let height = proto.area().height;
                Section {
                    id: section_id,
                    height,
                    content: SectionContent::Header(text, tier, Some(proto)),
                }
            })
            .collect();

        self.update(new_sections);
    }

    /// Extract all image protocols from the document for caching before reparse.
    /// Returns Vec<(url, protocol)>. Protocols are moved out, lines revert to height 1.
    pub fn take_image_protocols(&mut self) -> ImageCache {
        let mut cache = ImageCache::default();
        for section in &mut self.sections {
            match &mut section.content {
                SectionContent::Image(url, _) => {
                    let url = url.clone();
                    let SectionContent::Image(link, proto) = std::mem::replace(
                        &mut section.content,
                        SectionContent::ImagePlaceholder(url, vec![]),
                    ) else {
                        unreachable!();
                    };
                    cache.images.insert(link.url.clone(), proto);
                    section.height = 1;
                }
                SectionContent::Header(text, tier, proto) => {
                    if proto.is_some() {
                        let text = text.clone();
                        let tier = *tier;
                        let SectionContent::Header(text, tier, Some(proto)) = std::mem::replace(
                            &mut section.content,
                            SectionContent::HeaderPlaceholder(text, tier, vec![]),
                        ) else {
                            unreachable!();
                        };
                        let key = (text, tier);
                        if let Some(existing) = cache.headers.get_mut(&key) {
                            existing.push(proto);
                        } else {
                            cache.headers.insert(key, vec![proto]);
                        }
                    }
                }
                _ => {}
            }
        }
        cache
    }

    pub fn trim(&mut self, last_section_id: Option<usize>) {
        let Some(last_section_id) = last_section_id else {
            log::warn!("Document::trim without last_section_id, nothing parsed");
            return;
        };
        if let Some(last) = self.sections.last()
            && last.id == last_section_id
        {
            return;
        }
        if let Some(idx) = self
            .sections
            .iter()
            .position(|section| section.id == last_section_id)
        {
            log::debug!("trim: {idx} + 1");
            self.sections.truncate(idx + 1);
        }
    }

    pub fn get_y(&self, CursorPointer { id, index }: &CursorPointer) -> Option<i16> {
        let mut y = 0;
        for section in &self.sections {
            match &section.content {
                SectionContent::Lines(lines) => {
                    if section.id != *id {
                        y += section.height as i16;
                        continue;
                    }

                    // Flatten extras, mirrors what we do when building cursor in
                    // find_nth_next_cursor and friends. Not pretty.
                    let mut i = 0;
                    for (line_y, (_line, extras)) in lines.iter().enumerate() {
                        for _extra in extras {
                            if i == *index {
                                return Some(y + (line_y as i16));
                            }
                            i += 1;
                        }
                    }
                    // Probably some test, didn't have LineExtras.
                    log::warn!("get_y did not match index {index} in LineExtras: {y}");
                    return Some(y);
                }
                _ => {
                    if section.id == *id {
                        return Some(y);
                    }
                    y += section.height as i16;
                }
            }
        }
        log::warn!("get_y did not find {id},{index}");
        None
    }

    // Find first should be slightly more efficient.
    pub fn find_first_cursor<'b, Iter: Iterator<Item = &'b Section>>(
        iter: Iter,
        target: FindTarget,
        scroll: u16,
    ) -> Option<CursorPointer> {
        let locate = move |section: &Section| -> Option<CursorPointer> {
            if let SectionContent::Lines(lines) = &section.content {
                let mut flat_index = 0;
                for (_, extras) in lines {
                    if let Some(i) = extras.iter().position(|extra| target.matches(extra)) {
                        return Some(CursorPointer {
                            id: section.id,
                            index: flat_index + i,
                        });
                    }
                    flat_index += extras.len();
                }
            }
            None
        };

        let mut first = None;
        let mut offset_acc = 0;
        for section in iter {
            offset_acc += section.height;
            if offset_acc < scroll + 1 {
                if first.is_none() {
                    first = locate(section);
                }
                continue;
            }
            match locate(section) {
                None => {}
                x => return x,
            }
        }
        first
    }

    // Find nth (nonzero) next cursor.
    pub fn find_nth_next_cursor<'b, Iter>(
        iter: Iter,
        current: &CursorPointer,
        mode: FindMode,
        target: FindTarget,
        steps: NonZero<u16>,
    ) -> Option<CursorPointer>
    where
        Iter: DoubleEndedIterator<Item = &'b Section> + Clone,
    {
        let mut iter = Document::flatten_sections(iter, &mode, &target);
        let mut iter2 = iter.clone();
        let Some(curr_pos) = iter2.position(|x| x == *current) else {
            // TODO: This probably won't happen after #52 and #53 are fixed
            return iter.next();
        };
        let total = curr_pos + 1 + iter2.count();
        let index = (curr_pos + steps.get() as usize) % total;
        if index == curr_pos {
            return Some(current.clone());
        }
        iter.nth(index)
    }

    fn flatten_sections<'a, Iter>(
        iter: Iter,
        mode: &FindMode,
        target: &FindTarget,
    ) -> Either<
        impl Iterator<Item = CursorPointer> + Clone,
        impl Iterator<Item = CursorPointer> + Clone,
    >
    where
        Iter: DoubleEndedIterator<Item = &'a Section> + Clone,
    {
        match mode {
            FindMode::Next => Either::Left(iter.flat_map(move |section| {
                Document::line_extras_to_cursor_pointers(section, mode, target)
            })),
            FindMode::Prev => Either::Right(iter.rev().flat_map(move |section| {
                Document::line_extras_to_cursor_pointers(section, mode, target)
            })),
        }
    }

    fn line_extras_to_cursor_pointers(
        section: &Section,
        mode: &FindMode,
        target: &FindTarget,
    ) -> Either<
        Either<
            impl Iterator<Item = CursorPointer> + Clone,
            impl Iterator<Item = CursorPointer> + Clone,
        >,
        impl Iterator<Item = CursorPointer> + Clone,
    > {
        match mode {
            FindMode::Next => {
                if let SectionContent::Lines(lines) = &section.content {
                    let id = section.id;
                    let mut flat_index = 0;
                    let flattened: Vec<_> = lines
                        .iter()
                        .flat_map(|(_, extras)| {
                            let start = flat_index;
                            flat_index += extras.len();
                            extras
                                .iter()
                                .enumerate()
                                .map(move |(i, extra)| (start + i, extra))
                        })
                        .filter(|(_, extra)| target.matches(extra))
                        .map(move |(index, _)| CursorPointer { id, index })
                        .collect();
                    Either::Left(Either::Left(flattened.into_iter()))
                } else {
                    Either::Right(std::iter::empty())
                }
            }
            FindMode::Prev => {
                if let SectionContent::Lines(lines) = &section.content {
                    let id = section.id;
                    let mut flat_index = 0;
                    let mut flattened: Vec<_> = lines
                        .iter()
                        .flat_map(|(_, extras)| {
                            let start = flat_index;
                            flat_index += extras.len();
                            extras
                                .iter()
                                .enumerate()
                                .map(move |(i, extra)| (start + i, extra))
                        })
                        .filter(|(_, extra)| target.matches(extra))
                        .map(move |(index, _)| CursorPointer { id, index })
                        .collect();
                    flattened.reverse();
                    Either::Left(Either::Right(flattened.into_iter()))
                } else {
                    Either::Right(std::iter::empty())
                }
            }
        }
    }

    #[cfg(test)]
    pub fn find_extra_by_cursor(&self, pointer: &CursorPointer) -> Option<&LineExtra> {
        for section in self.iter() {
            if section.id != pointer.id {
                continue;
            }
            let SectionContent::Lines(lines) = &section.content else {
                continue;
            };
            let mut remaining = pointer.index;
            for (_, extras) in lines {
                if remaining < extras.len() {
                    return Some(&extras[remaining]);
                }
                remaining -= extras.len();
            }
        }
        None
    }

    #[cfg(test)]
    pub fn has_pending_images(&self) -> bool {
        self.sections
            .iter()
            .any(|section| matches!(&section.content, SectionContent::ImagePlaceholder(..)))
    }
}

impl Deref for Document {
    type Target = Vec<Section>;
    fn deref(&self) -> &Vec<Section> {
        &self.sections
    }
}

impl DerefMut for Document {
    fn deref_mut(&mut self) -> &mut Vec<Section> {
        &mut self.sections
    }
}

#[derive(Debug, Clone, Copy)]
pub enum FindMode {
    Prev,
    Next,
}

#[derive(Debug, Clone, Copy)]
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

pub type SectionID = usize;

#[derive(Debug)]
pub struct Section {
    pub id: SectionID,
    pub height: u16,
    pub content: SectionContent,
}

pub enum SectionContent {
    Image(MarkdownLink, ProtocolWrapper),
    ImagePlaceholder(MarkdownLink, Vec<(Line<'static>, Vec<LineExtra>)>),
    Header(String, u8, Option<Protocol>),
    HeaderPlaceholder(String, u8, Vec<(Line<'static>, Vec<LineExtra>)>),
    Lines(Vec<(Line<'static>, Vec<LineExtra>)>),
}

impl SectionContent {
    pub fn add_search(&mut self, re: Option<&Regex>) {
        if let SectionContent::Lines(lines) = self {
            for (line, extras) in lines {
                let line_string = line.to_string();
                extras.retain(|extra| !matches!(extra, LineExtra::SearchMatch(_, _, _)));
                if let Some(re) = re {
                    extras.extend(
                        re.find_iter(&line_string)
                            .map(SectionContent::regex_to_searchmatch(&line_string)),
                    );
                }
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

#[cfg(test)]
impl PartialEq for SectionContent {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Image(..), _) | (_, Self::Image(..)) => {
                panic!("PartialEq not supported for SectionContent::Image")
            }
            (Self::Lines(l), Self::Lines(r)) => l == r,
            (Self::Header(l0, l1, l2), Self::Header(r0, r1, r2)) => {
                l0 == r0 && l1 == r1 && l2.is_some() == r2.is_some()
            }
            _ => false,
        }
    }
}

impl Debug for SectionContent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Image(url, _) => f.debug_tuple(format!("Image({url:?})").as_str()).finish(),
            Self::ImagePlaceholder(url, _) => f
                .debug_tuple(format!("ImagePlaceholder({url:?})").as_str())
                .finish(),
            Self::Lines(lines) => {
                let mut tuple = f.debug_tuple("Line");
                for (line, extra) in lines {
                    let mut tuple = tuple.field(line);
                    if !extra.is_empty() {
                        tuple = tuple.field(extra);
                    }
                }
                tuple.finish()
            }
            Self::Header(text, tier, _) => f.debug_tuple("Header").field(text).field(tier).finish(),
            Self::HeaderPlaceholder(_, _, lines) => {
                let mut tuple = f.debug_tuple("HeaderPlaceholder");
                for (line, extra) in lines {
                    let mut tuple = tuple.field(line);
                    if !extra.is_empty() {
                        tuple = tuple.field(extra);
                    }
                }
                tuple.finish()
            }
        }
    }
}

impl Display for SectionContent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Image(url, protocol) => {
                write!(f, "Image({url:?}, {:?})", protocol.type_id())
            }
            Self::ImagePlaceholder(url, _) => {
                write!(f, "ImagePlaceholder({url:?})")
            }
            Self::Lines(lines) => write!(f, "Line({lines:?})"),
            Self::Header(text, tier, _) => write!(f, "Header({text}, {tier})"),
            Self::HeaderPlaceholder(_, _, lines) => write!(f, "HeaderPlaceholder({lines:?})"),
        }
    }
}

pub enum ProtocolWrapper {
    Sliced(Vec<Protocol>),
    Kitty(Protocol),
}
impl ProtocolWrapper {
    fn height(&self) -> u16 {
        match self {
            ProtocolWrapper::Sliced(protos) => protos.len() as u16,
            ProtocolWrapper::Kitty(proto) => proto.area().height,
        }
    }
}

impl Section {
    pub fn add_search(&mut self, re: Option<&Regex>) {
        self.content.add_search(re);
    }
}

#[cfg(test)]
impl Display for Section {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.content {
            SectionContent::Image(_, _) => write!(f, "<image>"),
            SectionContent::ImagePlaceholder(_, _) => write!(f, "<image-placeholder>"),
            SectionContent::Lines(lines) => {
                for (i, (line, _)) in lines.iter().enumerate() {
                    if i > 0 {
                        writeln!(f)?;
                    }
                    write!(f, "{}", line)?;
                }
                Ok(())
            }
            SectionContent::Header(text, tier, _) => {
                write!(f, "{} {}", "#".repeat(*tier as usize), text)
            }
            SectionContent::HeaderPlaceholder(_, _, lines) => {
                for (i, (line, _)) in lines.iter().enumerate() {
                    if i > 0 {
                        writeln!(f)?;
                    }
                    write!(f, "{}", line)?;
                }
                Ok(())
            }
        }
    }
}

pub enum LineExtra {
    Link(SourceContent, u16, u16),
    SearchMatch(usize, usize, String),
}

impl Debug for LineExtra {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LineExtra::Link(url, start, end) => {
                write!(f, "Link({:?}, {}, {})", url, start, end)
            }
            LineExtra::SearchMatch(start, end, text) => {
                write!(f, "SearchMatch({}, {}, {:?})", start, end, text)
            }
        }
    }
}

#[cfg(test)]
impl PartialEq for LineExtra {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (LineExtra::Link(l0, l1, l2), LineExtra::Link(r0, r1, r2)) => {
                l0 == r0 && l1 == r1 && l2 == r2
            }
            (LineExtra::SearchMatch(l0, l1, l2), LineExtra::SearchMatch(r0, r1, r2)) => {
                l0 == r0 && l1 == r1 && l2 == r2
            }
            _ => false,
        }
    }
}

/// Layout/shape and render `text` into a list of [`DynamicImage`] with a given terminal width.
pub fn header_images(
    font_renderer: &mut FontRenderer,
    width: u16,
    text: String,
    tier: u8,
    deep_fry_meme: bool,
) -> Result<Vec<(String, u8, DynamicImage)>, Error> {
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
        const RGBA_BG: [u8; 4] = [0, 0, 0, 0];
        let img: RgbaImage =
            RgbaImage::from_pixel(img_width, img_height, Rgba::<u8>::from(RGBA_BG));
        let dyn_img = DynamicImage::ImageRgba8(img);
        dyn_imgs.push((layout_run.text.into(), tier, dyn_img));
    }

    let fg = font_renderer.font_color;

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

            let dyn_img = &mut dyn_imgs[index].2;

            // Adjust picked image's Y coord offset.
            let y_offset: u32 = index as u32 * img_height;
            dyn_img.put_pixel(x as u32, y as u32 - y_offset, color.as_rgba().into());
        },
    );

    Ok(dyn_imgs)
}

const HEADER_ROW_COUNT: u16 = 2;

/// Render a list of images to [`Section`]s.
pub fn header_sections(
    picker: &Picker,
    width: u16,
    dyn_imgs: Vec<(String, u8, DynamicImage)>,
    deep_fry_meme: bool,
) -> Result<Vec<(String, u8, Protocol)>, Error> {
    let mut protos = vec![];
    for (text, tier, mut dyn_img) in dyn_imgs {
        if deep_fry_meme {
            dyn_img = deep_fry(dyn_img);
        }
        let proto = picker.new_protocol(
            dyn_img,
            Rect::new(0, 0, width, HEADER_ROW_COUNT),
            Resize::Fit(None),
        )?;
        protos.push((text, tier, proto));
    }
    Ok(protos)
}

#[expect(clippy::too_many_arguments)]
pub async fn image_section(
    picker: &Arc<Picker>,
    max_height: u16,
    width: u16,
    basepath: &Option<PathBuf>,
    client: Arc<RwLock<Client>>,
    id: SectionID,
    link: MarkdownLink,
    deep_fry_meme: bool,
) -> Result<Section, Error> {
    enum ImageSource {
        Bytes(Vec<u8>, ImageFormat),
        Path(String),
    }
    let url = &link.url;
    let image_source = if url.starts_with("https://") || url.starts_with("http://") {
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT, HeaderValue::from_static("image/png,image/jpg")); // or "image/jpeg"
        let client = client.read().await;
        let response = client.get(url).headers(headers).send().await?;
        drop(client);
        if !response.status().is_success() {
            return Err(Error::ImageLoad(
                url.to_owned(),
                format!("status {}", response.status()),
            ));
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
            Some(ct) => Err(Error::ImageLoad(
                url.to_owned(),
                format!("unhandled content-type {ct}"),
            )),
            None => Err(Error::ImageLoad(
                url.to_owned(),
                "no content-type".to_owned(),
            )),
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
    let section = tokio::task::spawn_blocking(move || {
        let mut dyn_img = match image_source {
            ImageSource::Bytes(bytes, format) => {
                ImageReader::with_format(std::io::Cursor::new(bytes), format).decode()?
            }
            ImageSource::Path(path) => ImageReader::open(path)?.decode()?,
        };

        if deep_fry_meme {
            dyn_img = deep_fry(dyn_img);
        }

        let protocol_type = picker.protocol_type();
        let max_width: u16 = (max_height * 3 / 2).min(width);
        match protocol_type {
            ProtocolType::Kitty => {
                let proto = picker.new_protocol(
                    dyn_img,
                    Rect::new(0, 0, max_width, max_height),
                    Resize::Fit(None),
                )?;

                Ok::<Section, Error>(Section {
                    id,
                    height: proto.area().height,
                    content: SectionContent::Image(link, ProtocolWrapper::Kitty(proto)),
                })
            }
            _ => {
                let dyn_img = resize(&picker, dyn_img, max_width, max_height);

                let (slices, image_size) = slice(dyn_img, &picker.font_size());
                let row_count = slices.len() as u16;
                let mut row_size = image_size;
                row_size.height /= row_count;
                let rows = slices
                    .into_iter()
                    .map(|row| {
                        picker
                            .new_protocol(row, row_size, Resize::Fit(None))
                            .map_err(Error::from)
                    })
                    .collect::<Result<Vec<Protocol>, Error>>()?;

                Ok::<Section, Error>(Section {
                    id,
                    height: rows.len() as u16,
                    content: SectionContent::Image(link, ProtocolWrapper::Sliced(rows)),
                })
            }
        }
    })
    .await??;
    Ok(section)
}

fn resize(picker: &Picker, dyn_img: DynamicImage, max_width: u16, max_height: u16) -> DynamicImage {
    let font_size = picker.font_size();
    let source = ratatui_image::protocol::ImageSource::new(dyn_img, font_size, Rgba([0, 0, 0, 0]));

    let max_area = Rect::new(0, 0, max_width, max_height);
    let needs_resize =
        Resize::Fit(None).needs_resize(&source, font_size, source.desired, max_area, false);
    if let Some(area) = needs_resize {
        let width = (area.width * font_size.0) as u32;
        let height = (area.height * font_size.1) as u32;
        source.image.resize(width, height, FilterType::Nearest)
    } else {
        source.image
    }
}

fn slice(image: DynamicImage, font_size: &FontSize) -> (Vec<DynamicImage>, Rect) {
    let height = image.height();
    let width = image.width();

    let row_count = (height as f64 / font_size.1 as f64).ceil() as u16;
    let mut rows = Vec::new();

    let font_height = font_size.1 as u32;
    for i in 0..row_count {
        let y = i as u32 * font_height;
        let row_height = font_height.min(height - y);
        let row = image.crop_imm(0, y, width, row_height);
        rows.push(row);
    }

    let col_count = (width as f64 / font_size.0 as f64).ceil() as u16;
    (rows, Rect::new(0, 0, col_count, row_count))
}

fn deep_fry(mut dyn_img: DynamicImage) -> DynamicImage {
    let width = dyn_img.width();
    let height = dyn_img.height();
    dyn_img = dyn_img.adjust_contrast(50.0);
    dyn_img = dyn_img.huerotate(45);

    let down_width = (width as f32 * 0.9) as u32;
    let down_height = (height as f32 * 0.8) as u32;
    dyn_img = dyn_img.resize(down_width, down_height, FilterType::Gaussian);
    dyn_img = dyn_img.resize(width, height, FilterType::Nearest);

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

#[cfg(test)]
mod tests {

    use ratatui::{
        style::Stylize as _,
        text::{Line, Span},
    };
    use regex::Regex;

    use crate::{
        cursor::CursorPointer,
        document::{Document, LineExtra, SectionContent},
        *,
    };

    #[test]
    fn widgestsources_update() {
        let mut ws = Document::default();
        ws.push(Section {
            id: 0,
            height: 2,
            content: SectionContent::Lines(vec![(Line::from("line #0"), Vec::new())]),
        });
        ws.push(Section {
            id: 1,
            height: 2,
            content: SectionContent::Lines(vec![(
                Line::from("headerline1 headerline2"),
                Vec::new(),
            )]),
        });
        ws.push(Section {
            id: 2,
            height: 2,
            content: SectionContent::Lines(vec![(Line::from("line #2"), Vec::new())]),
        });

        ws.update(vec![
            Section {
                id: 1,
                height: 2,
                content: SectionContent::Header(String::from("headerline1"), 1, None),
            },
            Section {
                id: 1,
                height: 2,
                content: SectionContent::Header(String::from("headerline2"), 1, None),
            },
        ]);
        assert_eq!(ws.sections.len(), 4);
        assert_eq!(0, ws.sections[0].id,);
        assert_eq!(1, ws.sections[1].id,);
        assert_eq!(
            SectionContent::Header(String::from("headerline1"), 1, None),
            ws.sections[1].content
        );
        assert_eq!(1, ws.sections[2].id,);
        assert_eq!(
            SectionContent::Header(String::from("headerline2"), 1, None),
            ws.sections[2].content
        );
        assert_eq!(2, ws.sections[3].id,);

        ws.update(vec![
            Section {
                id: 1,
                height: 2,
                content: SectionContent::Header(String::from("headerline3"), 1, None),
            },
            Section {
                id: 1,
                height: 2,
                content: SectionContent::Header(String::from("headerline4"), 1, None),
            },
        ]);
        assert_eq!(ws.sections.len(), 4);
        assert_eq!(0, ws.sections[0].id,);
        assert_eq!(1, ws.sections[1].id,);
        assert_eq!(
            SectionContent::Header(String::from("headerline3"), 1, None),
            ws.sections[1].content
        );
        assert_eq!(1, ws.sections[2].id,);
        assert_eq!(
            SectionContent::Header(String::from("headerline4"), 1, None),
            ws.sections[2].content
        );
        assert_eq!(2, ws.sections[3].id,);
    }

    #[test]
    #[expect(clippy::unwrap_used)]
    fn get_y() {
        #[expect(clippy::let_underscore_untyped)]
        let _ = flexi_logger::Logger::try_with_env()
            .unwrap()
            .start()
            .inspect_err(|err| eprint!("test logger setup failed: {err}"));

        let mut doc = Document::default();
        doc.push(Section {
            id: 1,
            height: 2,
            content: SectionContent::Header(String::from("one"), 1, None),
        });
        doc.push(Section {
            id: 2,
            height: 1,
            content: SectionContent::Lines(vec![(Line::from("line"), Vec::new())]),
        });
        doc.push(Section {
            id: 3,
            height: 1,
            content: SectionContent::Lines(vec![(Line::from("line"), Vec::new())]),
        });
        doc.push(Section {
            id: 4,
            height: 2,
            content: SectionContent::Header(String::from("one"), 1, None),
        });
        doc.push(Section {
            id: 5,
            height: 1,
            content: SectionContent::Lines(vec![(Line::from("line"), Vec::new())]),
        });
        assert_eq!(doc.get_y(&CursorPointer { id: 1, index: 0 }).unwrap(), 0);
        assert_eq!(doc.get_y(&CursorPointer { id: 2, index: 0 }).unwrap(), 2);
        assert_eq!(doc.get_y(&CursorPointer { id: 3, index: 0 }).unwrap(), 3);
        assert_eq!(doc.get_y(&CursorPointer { id: 4, index: 0 }).unwrap(), 4);
        assert_eq!(doc.get_y(&CursorPointer { id: 5, index: 0 }).unwrap(), 6);
    }

    #[test]
    fn add_search_offset() {
        let line = Line::from(vec![Span::from("▐").magenta(), Span::from(" hi")]);
        let mut wsd = SectionContent::Lines(vec![(line, Vec::new())]);
        wsd.add_search(Regex::new("hi").ok().as_ref());
        let SectionContent::Lines(lines) = wsd else {
            panic!("Line");
        };
        assert_eq!(
            lines[0].1[0],
            LineExtra::SearchMatch(2, 4, String::from("hi"))
        );
    }
}
