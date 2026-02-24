//! Worker
//!
//! Ideally, any intensive work *must* happen in the worker, to avoid locking up the main/UI thread
//! as much as possible.
//!
//! For now this only happens for markdown parsing, and image loading, resizing, and encoding.
//!
//! For example, text search could benefit from running in the worker, but it's not clear how the
//! text should then actually be shared.
pub mod markdown;

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc,
        mpsc::{Receiver, Sender},
    },
    thread::{self, JoinHandle},
};

use mdfrier::MdFrier;
use ratatui_image::picker::Picker;
use reqwest::Client;
use tokio::{runtime::Builder, sync::RwLock};

use crate::{
    Cmd, Event, MarkdownImage, Protocol,
    config::Theme,
    document::{SectionContent, header_images, header_sections, image_section},
    error::Error,
    model::DocumentId,
    setup::FontRenderer,
    worker::markdown::{SectionEvent, section_to_events},
};

#[expect(clippy::too_many_arguments)]
pub fn worker_thread(
    basepath: Option<PathBuf>,
    picker: Picker,
    renderer: Option<Box<FontRenderer>>,
    theme: Theme,
    has_text_size_protocol: bool,
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
        runtime.block_on(async {
            let basepath = basepath.clone();
            let client = Arc::new(RwLock::new(Client::new()));
            // Specifically not a tokio Mutex, because we use it in spawn_blocking.
            let thread_renderer =
                renderer.map(|renderer| Arc::new(std::sync::Mutex::new(renderer)));
            let thread_picker = Arc::new(picker);
            let mut parser = MdFrier::new()?;

            for cmd in cmd_rx {
                log::debug!("Cmd: {cmd}");
                match cmd {
                    Cmd::Parse(document_id, width, text, image_cache) => {
                        log::info!("Parse {document_id}");

                        let mut post_parse_events = Vec::new();

                        event_tx.send(Event::NewDocument(document_id))?;

                        let mut section_id: Option<usize> = None;
                        let sections = parser.parse_sections(width, &text, &theme)?;
                        for section in sections {
                            let (sections, section_events) = section_to_events(
                                &mut section_id,
                                width,
                                has_text_size_protocol,
                                &theme,
                                section,
                            );
                            for section in sections {
                                event_tx.send(Event::Parsed(document_id, section))?;
                            }
                            post_parse_events.extend(section_events);
                        }

                        // Send cached images synchronously before ParseDone
                        let mut image_cache = image_cache.unwrap_or_default();
                        let mut uncached_image_events = Vec::new();
                        for event in post_parse_events {
                            match &event {
                                SectionEvent::Image(
                                    section_id,
                                    MarkdownImage { destination, .. },
                                ) => {
                                    if let Some(proto) = image_cache.images.remove(destination) {
                                        log::debug!("image cache hit: {destination}");
                                        event_tx.send(Event::ImageLoaded(
                                            document_id,
                                            *section_id,
                                            destination.clone(),
                                            proto,
                                        ))?;
                                    } else {
                                        uncached_image_events.push(event);
                                    }
                                }
                                SectionEvent::Header(section_id, text, tier) => {
                                    let key = (text.clone(), *tier);
                                    if let Some(protos) = image_cache.headers.remove(&key) {
                                        log::debug!("header cache hit: {text}");
                                        event_tx.send(Event::HeaderLoaded(
                                            document_id,
                                            *section_id,
                                            protos
                                                .into_iter()
                                                .map(|proto| (text.clone(), *tier, proto))
                                                .collect(),
                                        ))?;
                                    } else {
                                        uncached_image_events.push(event);
                                    }
                                }
                            }
                        }

                        event_tx.send(Event::ParseDone(document_id, section_id))?;

                        if !uncached_image_events.is_empty() {
                            process_post_parse_events(
                                event_tx.clone(),
                                basepath.clone(),
                                client.clone(),
                                thread_picker.clone(),
                                thread_renderer.clone(),
                                width,
                                config_max_image_height,
                                deep_fry,
                                document_id,
                                uncached_image_events,
                            );
                        }
                    }
                }
            }
            Ok::<(), Error>(())
        })?;
        Ok::<(), Error>(())
    })
}

#[expect(clippy::too_many_arguments)]
fn process_post_parse_events(
    task_tx: Sender<Event>,
    basepath: Option<PathBuf>,
    client: Arc<RwLock<Client>>,
    picker: Arc<Picker>,
    font_renderer: Option<Arc<std::sync::Mutex<Box<FontRenderer>>>>,
    width: u16,
    config_max_image_height: u16,
    deep_fry: bool,
    document_id: DocumentId,
    post_parse_events: Vec<SectionEvent>,
) {
    // TODO: handle spawned task result errors, right now it's just discarded.
    tokio::spawn(async move {
        for event in post_parse_events {
            match event {
                SectionEvent::Image(section_id, MarkdownImage { destination, .. }) => {
                    // Load fresh image
                    match image_section(
                        &picker,
                        config_max_image_height,
                        width,
                        &basepath,
                        client.clone(),
                        section_id,
                        &destination,
                        deep_fry,
                    )
                    .await
                    {
                        Ok(section) => {
                            let SectionContent::Image(url, proto) = section.content else {
                                unreachable!("image_section should return SectionContent::Image");
                            };
                            task_tx.send(Event::ImageLoaded(document_id, section_id, url, proto))?
                        }
                        Err(Error::UnknownImage(_id, link)) => {
                            log::error!("image_section UnknownImage: {link}");
                            // Leave the image line as-is (shows ![alt](url))
                        }
                        Err(err) => {
                            log::error!("image_section error: {err}");
                            // Leave the image line as-is (shows ![alt](url))
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
            }
        }
        Ok::<(), Error>(())
    });
}

#[derive(Default)]
pub struct ImageCache {
    pub images: HashMap<String, Protocol>,
    pub headers: HashMap<(String, u8), Vec<Protocol>>,
}
impl ImageCache {
    pub fn is_empty(&self) -> bool {
        self.images.is_empty() && self.headers.is_empty()
    }
}
