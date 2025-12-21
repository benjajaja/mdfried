use std::{
    path::PathBuf,
    sync::{
        Arc,
        mpsc::{Receiver, Sender},
    },
    thread::{self, JoinHandle},
};

use ratatui_image::picker::{Picker, ProtocolType};
use ratskin::{MadSkin, RatSkin};
use reqwest::Client;
use tokio::{runtime::Builder, sync::RwLock};

use crate::{
    Cmd, Event,
    error::Error,
    markdown::parse,
    setup::{BgColor, FontRenderer},
    widget_sources::{WidgetSource, header_images, header_sources, image_source},
};

#[expect(clippy::too_many_arguments)]
pub fn worker_thread(
    basepath: Option<PathBuf>,
    picker: Picker,
    renderer: Option<Box<FontRenderer>>,
    skin: MadSkin,
    bg: Option<BgColor>,
    has_text_size_protocol: bool,
    deep_fry: bool,
    cmd_rx: Receiver<Cmd>,
    event_tx: Sender<Event<'static>>,
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
            let skin = RatSkin { skin };

            for cmd in cmd_rx {
                log::debug!("Cmd: {cmd}");
                match cmd {
                    Cmd::Parse(document_id, width, text) => {
                        log::info!("Parse {document_id}");
                        event_tx.send(Event::NewDocument(document_id))?;
                        let mut last_parsed_source_id = None;
                        for event in parse(&text, &skin, document_id, width, has_text_size_protocol)
                        {
                            match &event {
                                Event::Parsed(_, source) => {
                                    last_parsed_source_id = Some(source.id);
                                }
                                Event::ParseImage(_, source_id, _, _, _) => {
                                    last_parsed_source_id = Some(*source_id);
                                }
                                Event::ParseHeader(_, source_id, _, _) => {
                                    last_parsed_source_id = Some(*source_id);
                                }
                                _ => {}
                            }
                            event_tx.send(event)?;
                        }
                        log::debug!("Cmd::Parse finished");
                        event_tx.send(Event::ParseDone(document_id, last_parsed_source_id))?;
                    }
                    Cmd::Header(document_id, source_id, width, tier, text) => {
                        debug_assert!(
                            thread_renderer.is_some(),
                            "should not have sent ImgCmd::Header without renderer"
                        );
                        if let Some(thread_renderer) = &thread_renderer {
                            let task_tx = event_tx.clone();
                            if protocol_type != ProtocolType::Halfblocks {
                                let renderer = thread_renderer.clone();
                                let picker = thread_picker.clone();
                                tokio::spawn(async move {
                                    let images = tokio::task::spawn_blocking(move || {
                                        let mut r = renderer.lock()?;
                                        header_images(bg, &mut r, width, text, tier, deep_fry)
                                    })
                                    .await??;

                                    let headers = tokio::task::spawn_blocking(move || {
                                        header_sources(&picker, width, source_id, images, deep_fry)
                                    })
                                    .await??;
                                    task_tx.send(Event::Update(document_id, headers))?;
                                    Ok::<(), Error>(())
                                });
                            }
                        }
                    }
                    Cmd::UrlImage(document_id, source_id, width, url, text, _title) => {
                        let task_tx = event_tx.clone();
                        let basepath = basepath.clone();
                        let client = client.clone();
                        let picker = thread_picker.clone();
                        // TODO: handle spawned task result errors, right now it's just discarded.
                        tokio::spawn(async move {
                            match image_source(
                                &picker,
                                config_max_image_height,
                                width,
                                &basepath,
                                client,
                                source_id,
                                &url,
                                deep_fry,
                            )
                            .await
                            {
                                Ok(source) => {
                                    task_tx.send(Event::Update(document_id, vec![source]))?
                                }
                                Err(Error::UnknownImage(id, link)) => {
                                    task_tx.send(Event::Update(
                                        document_id,
                                        vec![WidgetSource::image_unknown(id, link, text)],
                                    ))?
                                }
                                Err(_) => task_tx.send(Event::Update(
                                    document_id,
                                    vec![WidgetSource::image_unknown(source_id, url, text)],
                                ))?,
                            }
                            Ok::<(), Error>(())
                        });
                    }
                    Cmd::XdgOpen(url) => {
                        std::process::Command::new("xdg-open").arg(&url).spawn()?;
                    }
                    Cmd::FileChanged => {
                        log::info!("cmd FileChanged");
                        event_tx.send(Event::FileChanged)?;
                    }
                }
            }
            Ok::<(), Error>(())
        })?;
        Ok::<(), Error>(())
    })
}
