//! Worker
//!
//! Ideally, any intensive work *must* happen in the worker, to avoid locking up the main/UI thread
//! as much as possible.
//!
//! For now this only happens for markdown parsing, and image loading, resizing, and encoding.
//!
//! For example, text search could benefit from running in the worker, but it's not clear how the
//! text should then actually be shared.
pub mod highlighter;
pub mod mermaid;
pub mod sections;

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc,
        mpsc::{Receiver, Sender},
    },
    thread::{self, JoinHandle},
};

use cosmic_text::fontdb::Database;
use mdfrier::MdFrier;
use ratatui::layout::Size;
use ratatui_image::{Resize, picker::Picker, sliced::SlicedProtocol};
use reqwest::Client;
use tokio::{runtime::Builder, sync::RwLock, task::JoinSet};

use crate::{
    Cmd, Event, Protocol, VERSION,
    config::{Config, MermaidConfig},
    document::{
        LineExtra, LinkReference, Section, SectionContent, header_images, header_sections,
        image_section,
    },
    error::Error,
    model::DocumentId,
    setup::FontRenderer,
    sources::{MultiFileEntry, SharedDocumentSource, open_source},
    worker::{
        highlighter::Highlighter,
        sections::{SectionEvent, SectionIterator},
    },
};

#[expect(clippy::too_many_arguments)]
pub fn worker_thread(
    document_source: SharedDocumentSource,
    picker: Picker,
    renderer: Option<Box<FontRenderer>>,
    config: Config,
    deep_fry: bool,
    cmd_rx: Receiver<Cmd>,
    event_tx: Sender<Event>,
    config_max_image_height: u16,
) -> JoinHandle<Result<(), Error>> {
    thread::spawn(move || {
        let runtime = Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()?;
        let result = runtime.block_on(async {
            let builder = Client::builder().user_agent(format!(
                "mdfried/{}",
                VERSION.get().unwrap_or(&"unknown".to_owned())
            ));

            // Attempt to mitigate a test failure on darwin.
            // `system-configuration` is traced to reqwest via:
            //     cargo tree -i system-configuration --target all
            // The tests shouldn't be doing any requests, so the client building here would be the
            // most likely source of that panic.
            // ```
            // thread '<unnamed>' (103071) panicked at /nix/build/nix-5646-2352996470/mdfried-0.20.1-vendor/source-registry-0/system-configuration-0.5.1/src/dynamic_store.rs:154:1:
            // Attempted to create a NULL object.
            // ```
            #[cfg(test)]
            let builder = builder.no_proxy();

            let client = Arc::new(RwLock::new(builder.build()?));

            #[cfg(feature = "svg")]
            let fontdb = renderer
                .as_ref()
                .map(|fr| {
                    let db = fr.font_system.db();
                    Arc::new(db.clone())
                })
                .or_else(|| {
                    #[cfg(test)]
                    {
                        // Making the font db fails some tests, maybe takes too long.
                        None
                    }
                    #[expect(clippy::cfg_not_test)]
                    #[cfg(not(test))]
                    {
                        if !config.theme.has_text_size_protocol.unwrap_or_default() {
                            log::warn!(
                                "loading system fonts for SVG despite not using text-size-protocol"
                            );
                        }
                        let mut fontdb = Database::new();
                        fontdb.load_system_fonts(); // loads all system fonts
                        Some(Arc::new(fontdb))
                    }
                });
            #[cfg(not(feature = "svg"))]
            let fontdb = None;

            let highlighter = Arc::new(Highlighter::new(&config.theme));

            // Specifically not a tokio Mutex, because we use it in spawn_blocking.
            let thread_renderer =
                renderer.map(|renderer| Arc::new(std::sync::Mutex::new(renderer)));
            let thread_picker = Arc::new(picker);
            let mut parser = MdFrier::new()?;

            for cmd in cmd_rx {
                log::debug!("Cmd: {cmd}");
                let result = async {
                    match cmd {
                        Cmd::Parse(document_id, width, text, image_cache) => {
                            event_tx.send(Event::NewDocument(document_id))?;

                            let (next_section_id, post_parse_events) = parse_one_file(
                                &mut parser,
                                &event_tx,
                                document_id,
                                width,
                                &text,
                                None,
                                thread_renderer.is_some(),
                                &config,
                                0,
                            )?;
                            let last_section_id = next_section_id.checked_sub(1);

                            let mut image_cache = image_cache.unwrap_or_default();
                            let uncached_post_parse_events = drain_image_cache(
                                &event_tx,
                                document_id,
                                width,
                                &mut image_cache,
                                config_max_image_height,
                                post_parse_events,
                            )?;

                            event_tx.send(Event::ParseDone(document_id, last_section_id, text))?;

                            if !uncached_post_parse_events.is_empty() {
                                process_post_parse_events(
                                    event_tx.clone(),
                                    document_source.clone(),
                                    client.clone(),
                                    thread_picker.clone(),
                                    thread_renderer.clone(),
                                    fontdb.clone(),
                                    highlighter.clone(),
                                    width,
                                    &config,
                                    deep_fry,
                                    document_id,
                                    uncached_post_parse_events,
                                )
                                .await?;
                            }
                        }
                        Cmd::ParseMulti(document_id, width, entries, image_cache) => {
                            event_tx.send(Event::NewDocument(document_id))?;
                            let mut all_post_parse_events: Vec<SectionEvent> = Vec::new();
                            let mut last_section_id: Option<usize> = None;
                            let mut image_cache = image_cache.unwrap_or_default();
                            let has_renderer = thread_renderer.is_some();
                            // Section ids are unique per parsed document. Each file
                            // (and the FileSeparator that precedes it) needs an id,
                            // so the worker keeps a counter that survives across the
                            // per-file parse loops.
                            let mut next_section_id: usize = 0;

                            for MultiFileEntry {
                                path,
                                basepath,
                                text,
                            } in entries
                            {
                                let display_name = path.display().to_string();
                                let separator_section = Section {
                                    id: next_section_id,
                                    height: 1,
                                    content: SectionContent::FileSeparator {
                                        filename: display_name,
                                    },
                                };
                                next_section_id += 1;
                                event_tx.send(Event::Parsed(document_id, separator_section))?;

                                let (file_next_id, mut file_events) = parse_one_file(
                                    &mut parser,
                                    &event_tx,
                                    document_id,
                                    width,
                                    &text,
                                    basepath.as_deref(),
                                    has_renderer,
                                    &config,
                                    next_section_id,
                                )?;
                                next_section_id = file_next_id;
                                last_section_id = file_next_id.checked_sub(1).or(last_section_id);
                                all_post_parse_events.append(&mut file_events);
                            }

                            let uncached_post_parse_events = drain_image_cache(
                                &event_tx,
                                document_id,
                                width,
                                &mut image_cache,
                                config_max_image_height,
                                all_post_parse_events,
                            )?;

                            // `Event::ParseDone`'s text payload is only
                            // consumed by `DocumentSource::Stdin::return_text`,
                            // which is not used in multi-file mode, so we
                            // pass an empty string to avoid duplicating
                            // every input file in memory.
                            event_tx.send(Event::ParseDone(
                                document_id,
                                last_section_id,
                                String::new(),
                            ))?;

                            if !uncached_post_parse_events.is_empty() {
                                process_post_parse_events(
                                    event_tx.clone(),
                                    document_source.clone(),
                                    client.clone(),
                                    thread_picker.clone(),
                                    thread_renderer.clone(),
                                    fontdb.clone(),
                                    highlighter.clone(),
                                    width,
                                    &config,
                                    deep_fry,
                                    document_id,
                                    uncached_post_parse_events,
                                )
                                .await?;
                            }
                        }
                        Cmd::OpenUrl(url) => {
                            let event_tx = event_tx.clone();
                            tokio::task::spawn_blocking(move || -> Result<(), Error> {
                                if let Ok((text, _)) = open_source(&url, None) {
                                    event_tx.send(Event::NewSourceContent(text))?;
                                }
                                Ok(())
                            })
                            .await??;
                        }
                        #[cfg(not(feature = "pdf"))]
                        Cmd::LoadPdf(_path, _available) => {
                            return Err(Error::Usage(Some(
                                "PDF support has not been enabled at build",
                            )));
                        }
                        #[cfg(feature = "pdf")]
                        Cmd::LoadPdf(path, available) => {
                            let event_tx = event_tx.clone();
                            let picker = thread_picker.clone();
                            tokio::task::spawn_blocking(move || -> Result<(), Error> {
                                use mupdf::{Colorspace, Document as MuDocument, Matrix};

                                let path_str = path.to_str().ok_or_else(|| {
                                    Error::Generic("invalid utf-8 in path".to_owned())
                                })?;
                                let doc = MuDocument::open(path_str)?;
                                let page_count = doc.page_count()?;
                                let font_size = picker.font_size();
                                let pixel_width = available.width as f32 * font_size.width as f32;

                                for idx in 0..page_count {
                                    let page = doc.load_page(idx)?;
                                    let bounds = page.bounds()?;
                                    let page_width = bounds.x1 - bounds.x0;
                                    let scale = if page_width > 0.0 {
                                        pixel_width / page_width
                                    } else {
                                        1.0
                                    };
                                    let matrix = Matrix::new_scale(scale, scale);
                                    let pixmap = page.to_pixmap(
                                        &matrix,
                                        &Colorspace::device_rgb(),
                                        0.0,
                                        false,
                                    )?;
                                    let width = pixmap.width();
                                    let height = pixmap.height();
                                    let samples = pixmap.samples().to_vec();
                                    let dyn_img = image::DynamicImage::ImageRgb8(
                                        image::RgbImage::from_raw(width, height, samples)
                                            .ok_or_else(|| {
                                                Error::ImageLoad(
                                                    path_str.to_owned(),
                                                    "could not create RBGA image from pixmap"
                                                        .to_owned(),
                                                )
                                            })?,
                                    );
                                    let resize =
                                        Resize::Fit(Some(ratatui_image::FilterType::Lanczos3));
                                    let page_available = Size::new(available.width, u16::MAX);
                                    let size = resize.size_for(
                                        &dyn_img,
                                        picker.font_size(),
                                        page_available,
                                    );
                                    let proto = SlicedProtocol::new(&picker, dyn_img, Some(size))?;
                                    event_tx.send(Event::PdfPageLoaded(idx as usize, proto))?;
                                }
                                Ok(())
                            })
                            .await??;
                        }
                        Cmd::LoadImage(image) => {
                            let event_tx = event_tx.clone();
                            let picker = thread_picker.clone();
                            tokio::task::spawn_blocking(move || -> Result<(), Error> {
                                let resize = Resize::Fit(Some(ratatui_image::FilterType::Lanczos3));
                                let (dyn_img, size) = if let Some((path, available)) = image {
                                    let dyn_img = image::ImageReader::open(path)?.decode()?;
                                    let size =
                                        resize.size_for(&dyn_img, picker.font_size(), available);
                                    (dyn_img, size)
                                } else {
                                    let bytes = include_bytes!("../assets/logo.png");
                                    (
                                        image::ImageReader::with_format(
                                            std::io::Cursor::new(bytes),
                                            image::ImageFormat::Png,
                                        )
                                        .decode()?,
                                        crate::view::WELCOME_LOGO_SIZE.into(),
                                    )
                                };
                                let proto = picker.new_protocol(dyn_img, size, resize)?;
                                event_tx.send(Event::RootImageLoaded(proto))?;
                                Ok(())
                            })
                            .await??;
                        }
                    }

                    Ok::<(), Error>(())
                }
                .await;

                match result {
                    Err(Error::ThreadClosed) => break, // Goodbye.
                    Err(err) => event_tx.send(Event::WorkerError(err))?,
                    Ok(()) => {} // Keep processing.
                }
            }

            Ok(())
        });

        if let Err(Error::ThreadClosed) = result {
            log::info!("ThreadClosedError: Abandoning blocking worker threads");
            runtime.shutdown_background();
        }
        result
    })
}

/// Parse a single markdown blob and emit `Event::Parsed` for each section.
///
/// Returns the next section id that would be assigned if parsing continued
/// (so the caller can pass it to the next file in multi-file mode) and the
/// list of `SectionEvent`s that still need post-processing (image loading,
/// header rendering, code highlighting, reference definition application).
///
/// `current_basepath` is the basepath of the file currently being parsed; it
/// is attached to every `SectionEvent::Image` produced for this file so the
/// worker can resolve relative image links against the correct file.
#[expect(clippy::too_many_arguments)]
fn parse_one_file(
    parser: &mut MdFrier,
    event_tx: &Sender<Event>,
    document_id: DocumentId,
    width: u16,
    text: &str,
    current_basepath: Option<&std::path::Path>,
    has_renderer: bool,
    config: &Config,
    starting_section_id: usize,
) -> Result<(usize, Vec<SectionEvent>), Error> {
    let lines = parser.parse(width, text, &config.theme)?;
    let mut section_iter =
        SectionIterator::with_starting_id(lines, &config.theme, starting_section_id);
    let mut post_parse_events = Vec::new();
    for section in &mut section_iter {
        match &section.content {
            SectionContent::Lines(lines) => {
                for (_, extras) in lines {
                    for extra in extras {
                        if let LineExtra::Link { reference, .. } = extra
                            && let LinkReference::ReferenceDefinition { id, url } = reference
                        {
                            post_parse_events.push(SectionEvent::ReferenceDefinition {
                                id: id.clone(),
                                url: url.clone(),
                            });
                        }
                    }
                }
                event_tx.send(Event::Parsed(document_id, section))?;
            }
            SectionContent::Code(language, lines) => {
                post_parse_events.push(SectionEvent::Code(
                    section.id,
                    language.clone(),
                    lines.iter().map(|(line, _)| line.clone()).collect(),
                ));
                event_tx.send(Event::Parsed(document_id, section))?;
            }
            SectionContent::Image(_, _, _, _) => {
                unreachable!("SectionIterator produced Image");
            }
            SectionContent::ImagePlaceholder(link, lines) => {
                let section_id = section.id;
                let link = link.clone();
                let has_trailing_blank = lines
                    .last()
                    .map(|(line, _)| line.spans.is_empty())
                    .unwrap_or_default();
                event_tx.send(Event::Parsed(document_id, section))?;
                post_parse_events.push(SectionEvent::Image(
                    section_id,
                    link,
                    has_trailing_blank,
                    current_basepath.map(PathBuf::from),
                ));
            }
            SectionContent::Header(_, _, _) => {
                if !config.theme.has_text_size_protocol.unwrap_or_default() {
                    unreachable!("SectionIterator produced Header without text-size-protocol");
                }
                event_tx.send(Event::Parsed(document_id, section))?;
            }
            SectionContent::HeaderPlaceholder(text, tier, _) => {
                if config.theme.has_text_size_protocol.unwrap_or_default() {
                    unreachable!(
                        "SectionIterator produced HeaderPlaceholder with text-size-protocol"
                    );
                }
                let section_id = section.id;
                let text = text.clone();
                let tier = *tier;
                event_tx.send(Event::Parsed(document_id, section))?;
                if has_renderer {
                    post_parse_events.push(SectionEvent::Header(section_id, text, tier));
                }
            }
            SectionContent::FileSeparator { .. } => {
                unreachable!("SectionIterator produced FileSeparator");
            }
        }
    }
    let next_section_id = section_iter.next_id();
    Ok((next_section_id, post_parse_events))
}

/// Apply the image cache to the post-parse events.
///
/// Events whose `Image` or `Header` payload has a matching cache entry are
/// converted into `Event::ImageLoaded` / `Event::HeaderLoaded` immediately and
/// not returned. `ReferenceDefinition` events are always applied inline.
/// Everything else is returned so the caller can hand it to
/// `process_post_parse_events` for async work.
fn drain_image_cache(
    event_tx: &Sender<Event>,
    document_id: DocumentId,
    width: u16,
    image_cache: &mut ImageCache,
    config_max_image_height: u16,
    post_parse_events: Vec<SectionEvent>,
) -> Result<Vec<SectionEvent>, Error> {
    let mut uncached_post_parse_events = Vec::new();
    for event in post_parse_events {
        match &event {
            SectionEvent::Image(section_id, link, has_trailing_blank, _section_basepath) => {
                if let Some((proto, size, max_size)) = image_cache.images.remove(&link.url) {
                    if width == max_size.width && config_max_image_height >= max_size.height {
                        event_tx.send(Event::ImageLoaded(
                            document_id,
                            *section_id,
                            link.clone(),
                            (proto, size, max_size),
                            *has_trailing_blank,
                        ))?;
                        log::debug!(
                            "image cache hit: {max_size:?} vs {width}x{config_max_image_height}, {size:?}, {}",
                            link.url
                        );
                    } else {
                        log::debug!(
                            "image cache hit but different max width ({width}x{config_max_image_height} vs {max_size}): {size:?}, {}",
                            link.url
                        );
                        uncached_post_parse_events.push(event);
                    }
                } else {
                    log::debug!("image cache miss: {}", link.url);
                    uncached_post_parse_events.push(event);
                }
            }
            SectionEvent::Header(section_id, text, tier) => {
                let key = (text.clone(), *tier);
                if let Some(protos) = image_cache.headers(width).and_then(|hc| hc.remove(&key)) {
                    log::debug!("header cache hit: {key:?}");
                    event_tx.send(Event::HeaderLoaded(
                        document_id,
                        *section_id,
                        protos
                            .into_iter()
                            .map(|proto| (text.clone(), *tier, proto))
                            .collect(),
                    ))?;
                } else {
                    log::debug!("header cache miss: {key:?}");
                    uncached_post_parse_events.push(event);
                }
            }
            SectionEvent::ReferenceDefinition { id, url } => {
                event_tx.send(Event::ReferenceDefinition {
                    id: format!("[{id}]"),
                    url: url.clone(),
                })?;
            }
            _ => uncached_post_parse_events.push(event),
        }
    }
    Ok(uncached_post_parse_events)
}

#[expect(clippy::too_many_arguments)]
async fn process_post_parse_events(
    task_tx: Sender<Event>,
    document_source: SharedDocumentSource,
    client: Arc<RwLock<Client>>,
    picker: Arc<Picker>,
    font_renderer: Option<Arc<std::sync::Mutex<Box<FontRenderer>>>>,
    fontdb: Option<Arc<Database>>,
    highlighter: Arc<Highlighter>,
    width: u16,
    config: &Config,
    deep_fry: bool,
    document_id: DocumentId,
    post_parse_events: Vec<SectionEvent>,
) -> Result<(), Error> {
    // TODO: handle spawned task result errors, right now it's just logged and discarded.
    let config_max_image_height = config.max_image_height;

    let mut set: JoinSet<Result<(), Error>> = JoinSet::new();
    for event in post_parse_events {
        let task_tx = task_tx.clone();
        let picker = picker.clone();
        let client = client.clone();
        let font_renderer = font_renderer.clone();
        let fontdb = fontdb.clone();
        let highlighter = highlighter.clone();
        let document_source = document_source.clone();
        let mermaid_config = config.mermaid.clone();

        set.spawn(async move {
            match event {
                SectionEvent::Image(section_id, link, has_trailing_blank, section_basepath) => {
                    let url = link.url.clone(); // For potential errors
                    // Load fresh image
                    match image_section(
                        &picker,
                        config_max_image_height,
                        width,
                        document_source.clone(),
                        section_basepath.as_deref(),
                        client.clone(),
                        section_id,
                        link,
                        deep_fry,
                        fontdb.clone(),
                    )
                    .await
                    {
                        Ok(section) => {
                            let SectionContent::Image(link, protos, size, max_size) =
                                section.content
                            else {
                                unreachable!("image_section should return SectionContent::Image");
                            };
                            task_tx.send(Event::ImageLoaded(
                                document_id,
                                section_id,
                                link,
                                (protos, size, max_size),
                                has_trailing_blank,
                            ))?
                        }
                        Err(Error::ImageLoad(url, err)) => {
                            task_tx.send(Event::ImageFailed(document_id, section_id, url, err))?
                        }
                        Err(err) => {
                            let err = match err {
                                Error::Io(err) => err.to_string(),
                                _ => format!("{err}"),
                            };
                            task_tx.send(Event::ImageFailed(document_id, section_id, url, err))?
                        }
                    }
                }
                SectionEvent::Header(section_id, text, tier) => {
                    log::debug!("SectionEvent::Header: {text}");
                    let Some(font_renderer) = &font_renderer else {
                        panic!("should not have produced SectionEvent::Header without renderer");
                    };
                    let font_renderer = font_renderer.clone();
                    let images = tokio::task::spawn_blocking(move || {
                        let mut r = font_renderer.lock()?;
                        header_images(&mut r, width, text, tier, deep_fry)
                    })
                    .await??;
                    let picker = picker.clone();
                    let images = tokio::task::spawn_blocking(move || {
                        header_sections(&picker, width, images, deep_fry)
                    })
                    .await??;
                    task_tx.send(Event::HeaderLoaded(document_id, section_id, images))?;
                }
                SectionEvent::ReferenceDefinition { .. } => {}
                SectionEvent::Code(section_id, language, lines) => {
                    if language == "mermaid" {
                        let result = match mermaid_config {
                            MermaidConfig::Disabled => Ok::<_, Error>(None),
                            #[cfg(feature = "mermaid")]
                            MermaidConfig::Builtin => {
                                if let Some(fontdb) = fontdb {
                                    Ok(Some(
                                        mermaid::internal::render(
                                            &lines,
                                            width,
                                            config_max_image_height,
                                            fontdb,
                                            picker,
                                        )
                                        .await?,
                                    ))
                                } else {
                                    log::error!("mermaid: no fontdb available");
                                    Err(Error::Mermaid("your message here".into()))
                                }
                            }
                            MermaidConfig::Command(cmd) => Ok(Some(
                                mermaid::render_with_cmd(
                                    &cmd,
                                    &lines,
                                    width,
                                    config_max_image_height,
                                    picker,
                                )
                                .await?,
                            )),
                        };
                        match result {
                            Ok(Some((sliced, size, max_size, link))) => {
                                task_tx.send(Event::ImageLoaded(
                                    document_id,
                                    section_id,
                                    link,
                                    (sliced, size, max_size),
                                    true, // comes from a codeblock
                                ))?;
                                return Ok(());
                            }
                            Ok(None) => {} // Fall through to regular syntax highlighter.
                            Err(err) => log::error!("{err}"),
                        }
                    }
                    let mut hl = highlighter.fork();
                    let text = tokio::task::spawn_blocking(move || hl.highlight(&language, lines))
                        .await??;
                    task_tx.send(Event::CodeLoaded(document_id, section_id, text))?;
                }
            }
            Ok(())
        });
    }

    while let Some(result) = set.join_next().await {
        match result {
            Ok(Ok(())) => {}
            Err(e) => log::error!("process_post_parse_events join error: {e}"),
            Ok(Err(e)) => {
                if let Error::ThreadClosed = e {
                    log::debug!("process_post_parse_events JoinSet::join_next early exit: {e}");
                    return Err(e);
                }
                log::error!("process_post_parse_events task error: {e}")
            }
        }
    }

    Ok(())
}

#[derive(Default)]
pub struct ImageCache {
    pub images: HashMap<String, (SlicedProtocol, Size, Size)>,
    headers_width: u16,
    headers: HashMap<(String, u8), Vec<Protocol>>,
}
impl ImageCache {
    pub fn is_empty(&self) -> bool {
        self.images.is_empty() && self.headers.is_empty()
    }
    pub fn headers(&mut self, width: u16) -> Option<&mut HashMap<(String, u8), Vec<Protocol>>> {
        if self.headers_width == width {
            Some(&mut self.headers)
        } else {
            self.headers = Default::default();
            None
        }
    }
}
impl std::fmt::Debug for ImageCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ImageCache(image_count:{},header_count:{})",
            self.images.len(),
            self.headers.len(),
        )
    }
}
