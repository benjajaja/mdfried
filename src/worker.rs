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
    path::PathBuf,
    sync::{
        Arc,
        mpsc::{Receiver, Sender},
    },
    thread::{self, JoinHandle},
};

use mdfrier::MdFrier;
use ratatui_image::picker::{Picker, ProtocolType};
use reqwest::Client;
use tokio::{runtime::Builder, sync::RwLock};

use crate::{
    Cmd, Event, MarkdownImage, Protocol,
    config::Theme,
    document::{SectionContent, header_images, header_sections, image_section},
    error::Error,
    model::DocumentId,
    setup::FontRenderer,
    worker::markdown::section_to_events,
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
            let protocol_type = picker.protocol_type(); // Won't change
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
                        for section in parser.parse_sections(width, &text, &theme) {
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
                        let mut cache: std::collections::HashMap<String, Protocol> = image_cache
                            .unwrap_or_default()
                            .into_iter()
                            .collect();
                        let mut uncached_events = Vec::new();
                        for event in post_parse_events {
                            let markdown::SectionEvent::Image(
                                section_id,
                                MarkdownImage { destination, .. },
                            ) = &event;
                            if let Some(proto) = cache.remove(destination) {
                                log::debug!("reusing cached image: {destination}");
                                event_tx.send(Event::ImageLoaded(
                                    document_id,
                                    *section_id,
                                    destination.clone(),
                                    proto,
                                ))?;
                            } else {
                                uncached_events.push(event);
                            }
                        }

                        event_tx.send(Event::ParseDone(document_id, section_id))?;

                        if !uncached_events.is_empty() {
                            process_post_parse_events(
                                event_tx.clone(),
                                basepath.clone(),
                                client.clone(),
                                thread_picker.clone(),
                                width,
                                config_max_image_height,
                                deep_fry,
                                document_id,
                                uncached_events,
                            );
                        }
                    }
                    Cmd::Header(document_id, section_id, width, tier, text) => {
                        debug_assert!(
                            thread_renderer.is_some(),
                            "should not have sent Cmd::Header without renderer"
                        );
                        if let Some(thread_renderer) = &thread_renderer {
                            let task_tx = event_tx.clone();
                            if protocol_type != ProtocolType::Halfblocks {
                                let renderer = thread_renderer.clone();
                                let picker = thread_picker.clone();
                                tokio::spawn(async move {
                                    let images = tokio::task::spawn_blocking(move || {
                                        let mut r = renderer.lock()?;
                                        header_images(&mut r, width, text, tier, deep_fry)
                                    })
                                    .await??;

                                    let headers = tokio::task::spawn_blocking(move || {
                                        header_sections(
                                            &picker, width, section_id, images, deep_fry,
                                        )
                                    })
                                    .await??;
                                    task_tx.send(Event::Update(document_id, headers))?;
                                    Ok::<(), Error>(())
                                });
                            }
                        }
                    }
                }
            }
            Ok::<(), Error>(())
        })?;
        Ok::<(), Error>(())
    })
}

fn process_post_parse_events(
    task_tx: Sender<Event>,
    basepath: Option<PathBuf>,
    client: Arc<RwLock<Client>>,
    picker: Arc<Picker>,
    width: u16,
    config_max_image_height: u16,
    deep_fry: bool,
    document_id: DocumentId,
    post_parse_events: Vec<markdown::SectionEvent>,
) {
    // TODO: handle spawned task result errors, right now it's just discarded.
    tokio::spawn(async move {
        for event in post_parse_events {
            match event {
                markdown::SectionEvent::Image(
                    section_id,
                    MarkdownImage {
                        destination,
                        description: _,
                    },
                ) => {
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
                            task_tx.send(Event::ImageLoaded(
                                document_id,
                                section_id,
                                url,
                                proto,
                            ))?
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
            }
        }
        Ok::<(), Error>(())
    });
}
