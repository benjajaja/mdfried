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
    Cmd, Event,
    config::Theme,
    document::{Section, SectionContent, header_images, header_sections, image_section},
    error::Error,
    setup::FontRenderer,
    worker::markdown::md_line_to_events,
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
                    Cmd::Parse(document_id, width, text) => {
                        log::info!("Parse {document_id}");

                        event_tx.send(Event::NewDocument(document_id))?;

                        let mut section_id: Option<usize> = None;
                        for section in parser.parse_sections(width, &text, &theme) {
                            for event in md_line_to_events(
                                document_id,
                                &mut section_id,
                                width,
                                has_text_size_protocol,
                                &theme,
                                section,
                            ) {
                                event_tx.send(event)?;
                            }
                        }

                        event_tx.send(Event::ParseDone(document_id, section_id))?;
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
                    Cmd::UrlImage(document_id, section_id, width, url, text) => {
                        let task_tx = event_tx.clone();
                        let basepath = basepath.clone();
                        let client = client.clone();
                        let picker = thread_picker.clone();
                        // TODO: handle spawned task result errors, right now it's just discarded.
                        tokio::spawn(async move {
                            match image_section(
                                &picker,
                                config_max_image_height,
                                width,
                                &basepath,
                                client,
                                section_id,
                                &url,
                                deep_fry,
                            )
                            .await
                            {
                                Ok(section) => {
                                    task_tx.send(Event::Update(document_id, vec![section]))?
                                }
                                Err(Error::UnknownImage(id, link)) => {
                                    log::error!("image_section UnknownImage");
                                    task_tx.send(Event::Update(
                                        document_id,
                                        vec![Section::image_unknown(id, link, text)],
                                    ))?
                                }
                                Err(err) => {
                                    log::error!("image_section error: {err}");
                                    task_tx.send(Event::Update(
                                        document_id,
                                        vec![Section::image_unknown(section_id, url, text)],
                                    ))?
                                }
                            }
                            Ok::<(), Error>(())
                        });
                    }
                }
            }
            Ok::<(), Error>(())
        })?;
        Ok::<(), Error>(())
    })
}
