//! Worker
//!
//! TODO: wrap up a good explanation!
//!
//! # Worker pipeline
//!
//! # Worker process `Cmd`s
//!
//! ## Markdown parse
//! The markdown module produces a list of `MdEvent`s.
//!
//! ## Model `process_events`
//! From event, either insert line-widget, or send `Cmd` to worker to process an image.
//!
//! ## View
//! Renders line-widgets.
//!
//!     Parse
//!      ↓
//!     Event → Image
//!      ↓
//!     WidgetSource
//!      ↓
//!     View
//!
use std::{
    path::PathBuf,
    sync::{
        Arc,
        mpsc::{Receiver, Sender},
    },
    thread::{self, JoinHandle},
};

use ratatui::text::Line;
use ratatui_image::picker::{Picker, ProtocolType};
use ratskin::MadSkin;
use reqwest::Client;
use textwrap::{Options, wrap};
use tokio::{runtime::Builder, sync::RwLock};

use crate::{
    Cmd, Event,
    error::Error,
    markdown::{MdDocument, MdParser, MdSection},
    model::DocumentId,
    setup::{BgColor, FontRenderer},
    widget_sources::{
        BigText, WidgetSource, WidgetSourceData, header_images, header_sources, image_source,
    },
    wrap::wrap_md_spans,
};

#[expect(clippy::too_many_arguments)]
pub fn worker_thread(
    basepath: Option<PathBuf>,
    picker: Picker,
    renderer: Option<Box<FontRenderer>>,
    _skin: MadSkin,
    bg: Option<BgColor>,
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
            let mut parser = MdParser::new()?;

            for cmd in cmd_rx {
                log::debug!("Cmd: {cmd}");
                match cmd {
                    Cmd::Parse(document_id, width, text) => {
                        log::info!("Parse {document_id}");

                        event_tx.send(Event::NewDocument(document_id))?;
                        let doc = MdDocument::new(text, &mut parser)?;
                        let mut source_id = None;
                        let mut needs_space = false;
                        for event in doc.iter().flat_map(|section| {
                            let mut prefixed = if needs_space {
                                vec![Event::Parsed(
                                    document_id,
                                    WidgetSource {
                                        id: post_incr_source_id(&mut source_id),
                                        height: 1,
                                        data: WidgetSourceData::Line(Line::default(), Vec::new()),
                                    },
                                )]
                            } else {
                                Vec::new()
                            };

                            needs_space = match section {
                                // Counterintuitive, but this looks closer to the source, and (subjectively) more readable.
                                MdSection::Header(_, _) => false,
                                // Always add space before the next section (if any).
                                MdSection::Markdown(_) => true,
                            };

                            let events = section_into_events(
                                document_id,
                                &mut source_id,
                                width,
                                has_text_size_protocol,
                                section,
                            );
                            prefixed.extend(events);
                            prefixed
                        }) {
                            event_tx.send(event)?;
                        }
                        event_tx.send(Event::ParseDone(document_id, source_id))?;
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
                    Cmd::UrlImage(document_id, source_id, width, url, text) => {
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
                                    log::error!("image_source UnknownImage");
                                    task_tx.send(Event::Update(
                                        document_id,
                                        vec![WidgetSource::image_unknown(id, link, text)],
                                    ))?
                                }
                                Err(err) => {
                                    log::error!("image_source error: {err}");
                                    task_tx.send(Event::Update(
                                        document_id,
                                        vec![WidgetSource::image_unknown(source_id, url, text)],
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

pub fn post_incr_source_id(source_id: &mut Option<usize>) -> usize {
    if source_id.is_none() {
        *source_id = Some(0);
        0
    } else {
        *source_id = source_id.map(|id| id + 1);
        source_id.unwrap_or_default()
    }
}

fn section_into_events(
    document_id: DocumentId,
    source_id: &mut Option<usize>,
    width: u16,
    has_text_size_protocol: bool,
    section: MdSection,
) -> Vec<Event> {
    match section {
        MdSection::Header(text, tier) => {
            if has_text_size_protocol {
                let (n, d) = BigText::size_ratio(tier);
                let scaled_with = width as usize / 2 * usize::from(d) / usize::from(n);
                let options = Options::new(scaled_with)
                    .break_words(true) // break long words/URLs if they exceed width
                    .word_splitter(textwrap::word_splitters::WordSplitter::NoHyphenation); // no hyphens when breaking
                wrap(&text, options)
                    .iter()
                    .map(|part| {
                        Event::Parsed(
                            document_id,
                            WidgetSource {
                                id: post_incr_source_id(source_id),
                                height: 2,
                                data: WidgetSourceData::Header(part.to_string(), tier),
                            },
                        )
                    })
                    .collect()
            } else {
                vec![Event::ParseHeader(
                    document_id,
                    post_incr_source_id(source_id),
                    tier,
                    text,
                )]
            }
        }
        MdSection::Markdown(mdspans) => wrap_md_spans(document_id, source_id, width, mdspans),
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        markdown::{MdDocument, MdParser},
        worker::section_into_events,
        wrap::{
            COLOR_LINK_BG, COLOR_LINK_FG, LINK_DESC_CLOSE, LINK_DESC_OPEN, LINK_URL_CLOSE,
            LINK_URL_OPEN,
        },
        *,
    };
    use pretty_assertions::assert_eq;

    #[expect(clippy::unwrap_used)]
    fn parse(text: String, width: u16, has_text_size_protocol: bool) -> Vec<Event> {
        let mut parser = MdParser::new().unwrap();
        let doc = MdDocument::new(text, &mut parser).unwrap();
        let mut source_id = None;
        let document_id = DocumentId::default();
        doc.iter()
            .flat_map(|section| {
                section_into_events(
                    document_id,
                    &mut source_id,
                    width,
                    has_text_size_protocol,
                    section,
                )
            })
            .collect()
    }

    #[test]
    fn parse_one_basic_line() {
        let events: Vec<Event> = parse("oh *ah* ha ha".into(), 80, true);
        let expected = vec![Event::Parsed(
            DocumentId::default(),
            WidgetSource {
                id: 0,
                height: 1,
                data: WidgetSourceData::Line(
                    Line::from(vec![
                        Span::from("oh "),
                        Span::from("ah").fg(Color::Indexed(220)).italic(),
                        Span::from(" ha ha"),
                    ]),
                    Vec::new(),
                ),
            },
        )];
        assert_eq!(events, expected);
    }

    #[test]
    fn parse_link() {
        let events = parse("[text](http://link.com)".to_owned(), 80, true);
        let expected = vec![Event::Parsed(
            DocumentId::default(),
            WidgetSource {
                id: 0,
                height: 1,
                data: WidgetSourceData::Line(
                    Line::from(vec![
                        Span::from(LINK_DESC_OPEN).fg(COLOR_LINK_BG),
                        Span::from("text").fg(COLOR_LINK_FG).bg(COLOR_LINK_BG),
                        Span::from(LINK_DESC_CLOSE).fg(COLOR_LINK_BG),
                        Span::from(LINK_URL_OPEN).fg(COLOR_LINK_BG),
                        Span::from("http://link.com")
                            .fg(COLOR_LINK_FG)
                            .bg(COLOR_LINK_BG)
                            .underlined(),
                        Span::from(LINK_URL_CLOSE).fg(COLOR_LINK_BG),
                    ]),
                    vec![LineExtra::Link("http://link.com".to_owned(), 7, 22)],
                ),
            },
        )];
        assert_eq!(events, expected);
    }

    #[test]
    #[ignore] // TODO: rework the whole link range stuff - can we maybe just work with MdSpans?
    fn parse_long_linebroken_link() {
        let events: Vec<Event> = parse(
            "[a b](http://link.com/veeeeeeeeeeeeeeeeery/long/tail)".to_owned(),
            30,
            true,
        );

        let str_lines: Vec<String> = events
            .iter()
            .map(|ev| {
                let Event::Parsed(_, source) = ev else {
                    panic!("unrelated event");
                };
                source.to_string()
            })
            .collect();
        assert_eq!(
            vec![
                "[a b](",
                "http://link.com/",
                "veeeeeeeeeeeeeeeeery/long/tail",
                ")",
            ],
            str_lines,
            "breaks into 3 lines",
        );

        let urls: Vec<String> = events
            .iter()
            .flat_map(|ev| {
                if let Event::Parsed(
                    _,
                    WidgetSource {
                        data: WidgetSourceData::Line(_, links),
                        ..
                    },
                ) = ev
                {
                    let urls: Vec<String> = links
                        .iter()
                        .flat_map(|extra| {
                            if let LineExtra::Link(url, _, _) = extra {
                                vec![url.to_owned()]
                            } else {
                                Vec::new()
                            }
                        })
                        .collect();
                    return urls;
                }
                vec![]
            })
            .collect();
        assert_eq!(
            vec!["http://link.com/veeeeeeeeeeeeeeeeery/long/tail"],
            urls,
            "finds the full URL"
        );

        let expected = vec![
            Event::Parsed(
                DocumentId::default(),
                WidgetSource {
                    id: 0,
                    height: 1,
                    data: WidgetSourceData::Line(
                        Line::from(vec![
                            Span::from("[").fg(Color::Indexed(237)),
                            Span::from("a b").fg(Color::Indexed(4)),
                            Span::from("]").fg(Color::Indexed(237)),
                            Span::from("(").fg(Color::Indexed(237)),
                        ]),
                        Vec::new(),
                    ),
                },
            ),
            Event::Parsed(
                DocumentId::default(),
                WidgetSource {
                    id: 1,
                    height: 1,
                    data: WidgetSourceData::Line(
                        Line::from(vec![
                            Span::from("http://link.com/")
                                .fg(Color::Indexed(32))
                                .underlined(),
                        ]),
                        vec![LineExtra::Link(
                            "http://link.com/veeeeeeeeeeeeeeeeery/long/tail".to_owned(),
                            0,
                            15,
                        )],
                    ),
                },
            ),
            Event::Parsed(
                DocumentId::default(),
                WidgetSource {
                    id: 2,
                    height: 1,
                    data: WidgetSourceData::Line(
                        Line::from(vec![
                            Span::from("veeeeeeeeeeeeeeeeery/long/tail")
                                .fg(Color::Indexed(32))
                                .underlined(),
                        ]),
                        Vec::new(),
                    ),
                },
            ),
            Event::Parsed(
                DocumentId::default(),
                WidgetSource {
                    id: 3,
                    height: 1,
                    data: WidgetSourceData::Line(
                        Line::from(vec![Span::from(")").fg(Color::Indexed(237))]),
                        Vec::new(),
                    ),
                },
            ),
        ];
        assert_eq!(
            events, expected,
            "stylizes the part of the URL that starts on one line"
        );
    }

    #[test]
    #[ignore] // https://github.com/tree-sitter-grammars/tree-sitter-markdown/issues/171
    fn parse_bare_link() {
        let events: Vec<Event> = parse("http://ratatui.rs".to_owned(), 80, true);

        let Event::Parsed(
            _,
            WidgetSource {
                data: WidgetSourceData::Line(_, links),
                ..
            },
        ) = &events[0]
        else {
            panic!("expected one widget event");
        };
        assert_eq!(
            *links,
            vec![LineExtra::Link("http://ratatui.rs".to_owned(), 0, 20)]
        );
    }

    #[test]
    fn parse_multiple_links_same_line() {
        let events: Vec<Event> = parse("[a](http://a.com) [b](http://b.com)".to_owned(), 80, true);

        let urls: Vec<String> = events
            .iter()
            .flat_map(|ev| {
                if let Event::Parsed(
                    _,
                    WidgetSource {
                        data: WidgetSourceData::Line(_, links),
                        ..
                    },
                ) = ev
                {
                    let urls: Vec<String> = links
                        .iter()
                        .flat_map(|extra| {
                            if let LineExtra::Link(url, _, _) = extra {
                                vec![url.to_owned()]
                            } else {
                                Vec::new()
                            }
                        })
                        .collect();
                    return urls;
                }
                vec![]
            })
            .collect();
        assert_eq!(vec!["http://a.com", "http://b.com"], urls, "finds all URLs");
    }

    #[test]
    fn parse_header_wrapping_tier_1() {
        let events: Vec<Event> = parse("# 1234567890".to_owned(), 10, true);
        assert_eq!(2, events.len());

        let Event::Parsed(
            _,
            WidgetSource {
                data: WidgetSourceData::Header(text, tier),
                ..
            },
        ) = &events[0]
        else {
            panic!("expected Header");
        };
        assert_eq!(1, *tier);
        assert_eq!("12345", text);

        let Event::Parsed(
            _,
            WidgetSource {
                data: WidgetSourceData::Header(text, tier),
                ..
            },
        ) = &events[1]
        else {
            panic!("expected Header");
        };
        assert_eq!(1, *tier);
        assert_eq!("67890", text);
    }

    #[test]
    fn parse_header_wrapping_tier_4() {
        let events: Vec<Event> = parse("#### 1234567890".to_owned(), 10, true);
        assert_eq!(2, events.len());

        let Event::Parsed(
            _,
            WidgetSource {
                data: WidgetSourceData::Header(text, tier),
                ..
            },
        ) = &events[0]
        else {
            panic!("expected Header");
        };
        assert_eq!(4, *tier);
        assert_eq!("1234567", text);

        let Event::Parsed(
            _,
            WidgetSource {
                data: WidgetSourceData::Header(text, tier),
                ..
            },
        ) = &events[1]
        else {
            panic!("expected Header");
        };
        assert_eq!(4, *tier);
        assert_eq!("890", text);
    }

    #[test]
    fn long_line_break() {
        let events: Vec<Event> = parse("longline1\nlongline2".into(), 10, true);
        let expected = vec![
            Event::Parsed(
                DocumentId::default(),
                WidgetSource {
                    id: 0,
                    height: 1,
                    data: WidgetSourceData::Line(
                        Line::from(vec![Span::from("longline1")]),
                        Vec::new(),
                    ),
                },
            ),
            Event::Parsed(
                DocumentId::default(),
                WidgetSource {
                    id: 1,
                    height: 1,
                    data: WidgetSourceData::Line(
                        Line::from(vec![Span::from("longline2")]),
                        Vec::new(),
                    ),
                },
            ),
        ];
        assert_eq!(events, expected);
    }

    #[test]
    fn line_break() {
        let events: Vec<Event> = parse("line1\nline2".into(), 10, true);
        let expected = vec![
            Event::Parsed(
                DocumentId::default(),
                WidgetSource {
                    id: 0,
                    height: 1,
                    data: WidgetSourceData::Line(Line::from(vec![Span::from("line1")]), Vec::new()),
                },
            ),
            Event::Parsed(
                DocumentId::default(),
                WidgetSource {
                    id: 1,
                    height: 1,
                    data: WidgetSourceData::Line(Line::from(vec![Span::from("line2")]), Vec::new()),
                },
            ),
        ];
        assert_eq!(events, expected);
    }

    #[test]
    fn parse_sections_spacing() {
        let events: Vec<Event> = parse(
            "This is a test markdown document.\nAnother line of same paragraph.\n![image](url)"
                .into(),
            80,
            true,
        );
        fn line_event(id: usize, line: &str) -> Event {
            Event::Parsed(
                DocumentId::default(),
                WidgetSource {
                    id,
                    height: 1,
                    data: WidgetSourceData::Line(Line::from(line.to_owned()), Vec::new()),
                },
            )
        }
        let expected = vec![
            line_event(0, "This is a test markdown document."),
            line_event(1, "Another line of same paragraph."),
            Event::ParsedImage(
                DocumentId::default(),
                2,
                MarkdownImage {
                    destination: "url".to_owned(),
                    description: "TODO:img_desc".to_owned(), // TODO: fix this
                },
            ),
        ];
        assert_eq!(events, expected);
    }

    #[test]
    fn parse_newlines_at_styled() {
        let events: Vec<Event> = parse("This \n*is* a test.".into(), 80, true);
        let expected = vec![
            Event::Parsed(
                DocumentId::default(),
                WidgetSource {
                    id: 0,
                    height: 1,
                    data: WidgetSourceData::Line(Line::from(vec![Span::from("This")]), Vec::new()),
                },
            ),
            Event::Parsed(
                DocumentId::default(),
                WidgetSource {
                    id: 1,
                    height: 1,
                    data: WidgetSourceData::Line(
                        Line::from(vec![
                            Span::from("is").fg(Color::Indexed(220)).italic(),
                            Span::from(" a test."),
                        ]),
                        Vec::new(),
                    ),
                },
            ),
        ];
        assert_eq!(events, expected);
    }
}
