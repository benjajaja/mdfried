mod links;

use ratatui::text::Line;
use ratskin::RatSkin;
use regex::Regex;

use crate::{DocumentId, Event, WidgetSource, widget_sources::WidgetSourceData};

// Crude "pre-parsing" of markdown by lines.
// Headers are always on a line of their own.
// Images are only processed if it appears on a line by itself, to avoid having to deal with text
// wrapping around some area.
#[derive(Debug, PartialEq)]
enum Block {
    Header(u8, String),
    Image(String, String),
    Markdown(String),
}

fn split_headers_and_images(text: &str) -> Vec<Block> {
    // Regex to match lines starting with 1-6 `#` characters
    let header_re = Regex::new(r"^(#+)\s*(.*)").expect("regex");
    // Regex to match standalone image lines: ![alt](url)
    let image_re = Regex::new(r"^!\[(.*?)\]\((.*?)\)$").expect("regex");
    // Regex to match beginning or end of code fence
    let codefence_re = Regex::new(r"^ {0,3}(`{3,}|~{3,})").expect("regex");

    let mut blocks = Vec::new();
    let mut current_block = String::new();
    let mut current_codefence: Option<String> = None;

    for line in text.lines() {
        if let Some(codefence_str) = &current_codefence {
            if !current_block.is_empty() {
                current_block.push('\n');
            }
            current_block.push_str(line);
            if let Some(captures) = codefence_re.captures(line) {
                // End of codefence must match start, with at least as many characters
                if captures[1].starts_with(codefence_str) {
                    current_codefence = None;
                }
            }
        } else if let Some(captures) = header_re.captures(line) {
            // If there's an ongoing block, push it as a plain text block
            if !current_block.is_empty() {
                blocks.push(Block::Markdown(current_block.clone()));
                current_block.clear();
            }
            // Push the header as (level, text)
            let level = captures[1].len().min(6) as u8;
            let text = captures[2].to_string();
            blocks.push(Block::Header(level, text));
        } else if let Some(captures) = image_re.captures(line) {
            // If there's an ongoing block, push it as a plain text block
            if !current_block.is_empty() {
                blocks.push(Block::Markdown(current_block.clone()));
                current_block.clear();
            }
            // Push the image as (alt_text, url)
            let alt_text = captures[1].to_string();
            let url = captures[2].to_string();
            blocks.push(Block::Image(alt_text, url));
        } else if let Some(captures) = codefence_re.captures(line) {
            if !current_block.is_empty() {
                current_block.push('\n');
            }
            current_block.push_str(line);
            current_codefence = Some(captures[1].to_string());
        } else {
            // Accumulate lines that are neither headers nor images
            if !current_block.is_empty() {
                current_block.push('\n');
            }
            current_block.push_str(line);
        }
    }

    // Push the final block if there's remaining content
    if !current_block.is_empty() {
        blocks.push(Block::Markdown(current_block));
    }

    blocks
}

pub fn parse<'a>(
    text: &str,
    skin: &RatSkin,
    document_id: DocumentId,
    width: u16,
    has_text_size_protocol: bool,
) -> impl Iterator<Item = Event<'a>> {
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
                    // Leverage ratskin/termimad's line-wrapping feature.
                    let madtext = RatSkin::parse_text(&text);
                    for line in skin.parse(madtext, width / 2) {
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

fn send_parsed<'a>(
    document_id: DocumentId,
    id: &mut usize,
    data: WidgetSourceData<'a>,
    height: u16,
) -> Event<'a> {
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

fn send_event<'a>(id: &mut usize, ev: Event<'a>) -> Event<'a> {
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
    fn split_headers_and_images() {
        let blocks = markdown::split_headers_and_images(
            r#"
# header

paragraph

paragraph

# header

paragraph
paragraph

# header

paragraph

# header
"#,
        );
        assert_eq!(
            blocks,
            vec![
                markdown::Block::Header(1, "header".to_owned()),
                markdown::Block::Markdown("paragraph\n\nparagraph\n".to_owned()),
                markdown::Block::Header(1, "header".to_owned()),
                markdown::Block::Markdown("paragraph\nparagraph\n".to_owned()),
                markdown::Block::Header(1, "header".to_owned()),
                markdown::Block::Markdown("paragraph\n".to_owned()),
                markdown::Block::Header(1, "header".to_owned()),
            ]
        );
    }

    #[test]
    fn split_headers_and_images_without_space() {
        let blocks = markdown::split_headers_and_images(
            r#"
# header
paragraph
# header
# header
paragraph
# header
"#,
        );
        assert_eq!(6, blocks.len());
        assert_eq!(
            blocks,
            vec![
                markdown::Block::Header(1, "header".to_owned()),
                markdown::Block::Markdown("paragraph".to_owned()),
                markdown::Block::Header(1, "header".to_owned()),
                markdown::Block::Header(1, "header".to_owned()),
                markdown::Block::Markdown("paragraph".to_owned()),
                markdown::Block::Header(1, "header".to_owned()),
            ]
        );
    }

    #[test]
    fn codefence() {
        let blocks = markdown::split_headers_and_images(
            r#"
# header

paragraph

```c
#ifdef FOO
bar();
#endif
```

paragraph

  ~~~~
  x("
  ~~~
  ");
  #define Y
  z();
  ~~~~

# header

paragraph
"#,
        );
        assert_eq!(
            blocks,
            vec![
                markdown::Block::Header(1, "header".to_owned()),
                markdown::Block::Markdown(
                    r#"paragraph

```c
#ifdef FOO
bar();
#endif
```

paragraph

  ~~~~
  x("
  ~~~
  ");
  #define Y
  z();
  ~~~~
"#
                    .to_owned()
                ),
                markdown::Block::Header(1, "header".to_owned()),
                markdown::Block::Markdown("paragraph".to_owned()),
            ]
        );
    }

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
}
