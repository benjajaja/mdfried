mod blocks;
mod links;

use ratatui::text::Line;
use ratskin::RatSkin;

use crate::{
    DocumentId, Event, WidgetSource,
    markdown::blocks::{Block, split_headers_and_images},
    widget_sources::{BigText, WidgetSourceData},
};

pub fn parse(
    text: &str,
    skin: &RatSkin,
    document_id: DocumentId,
    width: u16,
    has_text_size_protocol: bool,
) -> impl Iterator<Item = Event> {
    let mut id = 0;

    let blocks = split_headers_and_images(text);

    let mut needs_space = false;

    blocks.into_iter().flat_map(move |block| {
        let mut events = Vec::new();
        if needs_space {
            // Send a newline after things like Markdowns and Images, but not after the last block.
            events = vec![send_parsed(
                document_id,
                &mut id,
                WidgetSourceData::Line(Line::default(), Vec::new()),
                1,
            )];
        }

        match block {
            Block::Header(tier, text) => {
                needs_space = false;
                if has_text_size_protocol {
                    let (n, d) = BigText::size_ratio(tier);
                    let scaled_with = width / 2 * u16::from(d) / u16::from(n);

                    // Leverage ratskin/termimad's line-wrapping feature.
                    // TODO: this is probably inefficient, find something else that simply
                    // word-wraps.
                    let madtext = RatSkin::parse_text(&text);
                    for line in skin.parse(madtext, scaled_with) {
                        let text = line.to_string();
                        events.push(send_parsed(
                            document_id,
                            &mut id,
                            WidgetSourceData::Header(text, tier),
                            2,
                        ));
                    }
                } else {
                    let event = Event::ParseHeader(document_id, id, tier, text);
                    events.push(send_event(&mut id, event));
                }
            }
            Block::Image(alt, url) => {
                needs_space = true;
                let event = Event::ParseImage(document_id, id, url, alt, String::new());
                events.push(send_event(&mut id, event));
            }
            Block::Markdown(text) => {
                needs_space = true;
                let madtext = RatSkin::parse_text(&text);

                for line in skin.parse(madtext, width) {
                    let (line, links) = links::capture_line(line, &text, width);

                    events.push(send_parsed(
                        document_id,
                        &mut id,
                        WidgetSourceData::Line(line, links),
                        1,
                    ));
                }
            }
        }
        events
    })
}

fn send_parsed(
    document_id: DocumentId,
    id: &mut usize,
    data: WidgetSourceData,
    height: u16,
) -> Event {
    send_event(
        id,
        Event::Parsed(
            document_id,
            WidgetSource {
                id: *id,
                height,
                data,
            },
        ),
    )
}

fn send_event(id: &mut usize, ev: Event) -> Event {
    *id += 1;
    ev
}

#[cfg(test)]
mod tests {
    use crate::{
        markdown::{
            links::{COLOR_DECOR, COLOR_LINK, COLOR_TEXT},
            parse,
        },
        *,
    };
    use pretty_assertions::assert_eq;
    use ratskin::RatSkin;

    #[test]
    fn parse_one_basic_line() {
        let events: Vec<Event> = parse(
            "*ah* ha ha",
            &RatSkin::default(),
            DocumentId::default(),
            80,
            true,
        )
        .collect();
        let expected = vec![Event::Parsed(
            DocumentId::default(),
            WidgetSource {
                id: 0,
                height: 1,
                data: WidgetSourceData::Line(
                    Line::from(vec![Span::from("ah").italic(), Span::from(" ha ha")]),
                    Vec::new(),
                ),
            },
        )];
        assert_eq!(events, expected);
    }

    #[test]
    fn parse_link() {
        let events: Vec<Event> = parse(
            "[text](http://link.com)",
            &RatSkin::default(),
            DocumentId::default(),
            80,
            true,
        )
        .collect();
        let expected = vec![Event::Parsed(
            DocumentId::default(),
            WidgetSource {
                id: 0,
                height: 1,
                data: WidgetSourceData::Line(
                    Line::from(vec![
                        Span::from("[").fg(COLOR_DECOR),
                        Span::from("text").fg(COLOR_TEXT),
                        Span::from("]").fg(COLOR_DECOR),
                        Span::from("(").fg(COLOR_DECOR),
                        Span::from("http://link.com").fg(COLOR_LINK).underlined(),
                        Span::from(")").fg(COLOR_DECOR),
                    ]),
                    vec![LineExtra::Link("http://link.com".to_owned(), 7, 22)],
                ),
            },
        )];
        assert_eq!(events, expected);
    }

    #[test]
    fn parse_long_link() {
        let events: Vec<Event> = parse(
            "[text](http://link.com/veeeeeeeeeeeeeeeeery/long/tail)",
            &RatSkin::default(),
            DocumentId::default(),
            30,
            true,
        )
        .collect();
        let expected = vec![
            Event::Parsed(
                DocumentId::default(),
                WidgetSource {
                    id: 0,
                    height: 1,
                    data: WidgetSourceData::Line(
                        Line::from(vec![
                            Span::from("[").fg(COLOR_DECOR),
                            Span::from("text").fg(COLOR_TEXT),
                            Span::from("]").fg(COLOR_DECOR),
                            Span::from("(").fg(COLOR_DECOR),
                            Span::from("http://link.com/veeeeee")
                                .fg(COLOR_LINK)
                                .underlined(),
                        ]),
                        vec![LineExtra::Link(
                            "http://link.com/veeeeeeeeeeeeeeeeery/long/tail".to_owned(),
                            7,
                            30,
                        )],
                    ),
                },
            ),
            Event::Parsed(
                DocumentId::default(),
                WidgetSource {
                    id: 1,
                    height: 1,
                    data: WidgetSourceData::Line(
                        Line::from(vec![Span::from("eeeeeeeeeeery/long/tail)")]),
                        Vec::new(),
                    ),
                },
            ),
        ];
        assert_eq!(events, expected);
    }

    #[test]
    fn parse_long_linebroken_link() {
        let events: Vec<Event> = parse(
            "[a b](http://link.com/veeeeeeeeeeeeeeeeery/long/tail)",
            &RatSkin::default(),
            DocumentId::default(),
            30,
            true,
        )
        .collect();

        let str_lines: Vec<String> = events
            .iter()
            .map(|ev| {
                if let Event::Parsed(_, source) = ev {
                    return source.to_string();
                }
                "<unrelated event>".into()
            })
            .collect();
        assert_eq!(
            vec![
                "[a ",
                "b](http://link.com/veeeeeeeeee",
                "eeeeeeery/long/tail)"
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
                        Line::from(vec![Span::from("[a"), Span::from(" ")]),
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
                            Span::from("b]("),
                            Span::from("http://link.com/veeeeeeeeee")
                                .fg(COLOR_LINK)
                                .underlined(),
                        ]),
                        vec![LineExtra::Link(
                            "http://link.com/veeeeeeeeeeeeeeeeery/long/tail".to_owned(),
                            3,
                            30,
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
                        Line::from(vec![Span::from("eeeeeeery/long/tail)")]),
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
    fn parse_multiple_links_same_line() {
        let events: Vec<Event> = parse(
            "http://a.com http://b.com",
            &RatSkin::default(),
            DocumentId::default(),
            80,
            true,
        )
        .collect();

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
        let events: Vec<Event> = parse(
            "# 1234567890",
            &RatSkin::default(),
            DocumentId::default(),
            10,
            true,
        )
        .collect();
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
        let events: Vec<Event> = parse(
            "#### 1234567890",
            &RatSkin::default(),
            DocumentId::default(),
            10,
            true,
        )
        .collect();
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
}
