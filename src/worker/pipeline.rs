pub mod line;
pub mod markdown;
pub mod table;
pub mod wrap;

use textwrap::{Options, wrap};

use crate::{
    Event,
    error::Error,
    model::DocumentId,
    widget_sources::{BigText, WidgetSource, WidgetSourceData},
    worker::pipeline::{
        line::{blank_line_event, blockquote_blank_line_event, wrapped_lines_to_events},
        markdown::{MdContainer, MdContent, MdDocument, MdParser, MdSection},
        table::table_to_events,
        wrap::wrap_md_spans,
    },
};

pub fn pipeline(
    parser: &mut MdParser,
    document_id: DocumentId,
    width: u16,
    has_text_size_protocol: bool,
    text: String,
) -> Result<(Vec<Event>, Option<usize>), Error> {
    let doc = MdDocument::new(text, parser)?;
    let mut source_id = None;

    let mut needs_space = false;
    let mut prev_nesting: Vec<MdContainer> = Vec::new();
    let mut prev_was_blank_line = false;
    let mut prev_in_list = false;

    let events = doc
        .iter()
        .flat_map(|section| {
            let in_list = section
                .nesting
                .iter()
                .any(|c| matches!(c, MdContainer::ListItem(_)));

            let is_blank_line = section.content.is_blank();

            // Nesting change: one non-empty nesting is a strict prefix of the other
            // e.g., [Blockquote] → [Blockquote, Blockquote] is nesting deeper
            let is_nesting = |a: &[MdContainer], b: &[MdContainer]| {
                !a.is_empty() && a.len() < b.len() && b.starts_with(a)
            };
            let nesting_change = is_nesting(&prev_nesting, &section.nesting)
                || is_nesting(&section.nesting, &prev_nesting);

            // Only suppress blank lines between items within the same list,
            // not when transitioning from something else into a list
            let both_in_list = in_list && prev_in_list;

            // Emit blank between sections unless:
            // - both in same list (list items handle their own spacing)
            // - blank line involved (it IS the spacing)
            // - nesting changed (going deeper/shallower in same container)
            let should_emit_blank = needs_space
                && !both_in_list
                && !is_blank_line
                && !prev_was_blank_line
                && !nesting_change;

            let mut maybe_blank_line: Vec<Event> = if should_emit_blank {
                vec![blank_line_event(document_id, &mut source_id)]
            } else {
                Vec::new()
            };

            // Only headers don't need space because we either render them BIG or with a blank line
            // included.
            needs_space = !matches!(section.content, MdContent::Header { .. });
            // Track state for next iteration
            prev_nesting = section.nesting.clone();
            prev_was_blank_line = is_blank_line;
            prev_in_list = in_list;

            let events = section_into_events(
                document_id,
                &mut source_id,
                width,
                has_text_size_protocol,
                section,
            );
            maybe_blank_line.extend(events);
            maybe_blank_line
        })
        .collect();

    Ok((events, source_id))
}

fn section_into_events(
    document_id: DocumentId,
    source_id: &mut Option<usize>,
    width: u16,
    has_text_size_protocol: bool,
    section: MdSection,
) -> Vec<Event> {
    let MdSection { content, nesting } = section;

    // Reconstruct source_prefix from nesting
    let source_prefix = build_source_prefix(&nesting);

    // Count blockquote depth from nesting
    let blockquote_depth = nesting
        .iter()
        .filter(|c| matches!(c, MdContainer::Blockquote(_)))
        .count();

    match content {
        MdContent::Header { tier, text } => {
            // TODO: Apply context prefix to headers inside blockquotes
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
        MdContent::Paragraph(mdspans) => {
            // Empty spans = blank line
            if mdspans.is_empty() {
                if blockquote_depth > 0 {
                    vec![blockquote_blank_line_event(
                        document_id,
                        source_id,
                        blockquote_depth,
                    )]
                } else {
                    vec![blank_line_event(document_id, source_id)]
                }
            } else {
                let wrapped_lines = wrap_md_spans(width, mdspans, &source_prefix);
                wrapped_lines_to_events(document_id, source_id, wrapped_lines, blockquote_depth)
            }
        }
        MdContent::CodeBlock { code, .. } => {
            code_block_to_events(document_id, source_id, width, &code, blockquote_depth)
        }
        MdContent::HorizontalRule => {
            horizontal_rule_to_events(document_id, source_id, width, blockquote_depth)
        }
        MdContent::Table {
            header,
            rows,
            alignments,
        } => table_to_events(
            document_id,
            source_id,
            width,
            header,
            rows,
            alignments,
            blockquote_depth,
        ),
    }
}

/// Build the source prefix string from nesting path.
fn build_source_prefix(nesting: &[MdContainer]) -> String {
    let mut prefix = String::new();

    // Find the index of the last ListItem (innermost) - only that one shows its marker
    let last_list_item_idx = nesting
        .iter()
        .rposition(|c| matches!(c, MdContainer::ListItem(_)));

    for (i, c) in nesting.iter().enumerate() {
        match c {
            MdContainer::Blockquote(_) => prefix.push_str("> "),
            MdContainer::ListItem(marker) => {
                // Only the innermost list item shows its marker.
                // Outer list items: their indent is already captured in inner marker's indent.
                if Some(i) == last_list_item_idx {
                    prefix.push_str(&" ".repeat(marker.indent));
                    prefix.push_str(&marker.original);
                    prefix.push(' ');
                    // Add task list marker if present
                    if let Some(checked) = marker.task {
                        if checked {
                            prefix.push_str("[x] ");
                        } else {
                            prefix.push_str("[ ] ");
                        }
                    }
                }
            }
            // List doesn't contribute to prefix
            MdContainer::List(_) => {}
        }
    }
    prefix
}

fn horizontal_rule_to_events(
    document_id: DocumentId,
    source_id: &mut Option<usize>,
    width: u16,
    blockquote_depth: usize,
) -> Vec<Event> {
    use ratatui::{
        style::{Color, Style},
        text::{Line, Span},
    };

    let rule_style = Style::default().fg(Color::Indexed(240));

    // Calculate available width for rule (subtract blockquote prefix)
    let prefix_width = blockquote_depth * 2; // "▌ " per level
    let available_width = (width as usize).saturating_sub(prefix_width);

    let mut spans = Vec::new();

    // Add blockquote prefix if inside blockquote
    if blockquote_depth > 0 {
        for depth in 0..blockquote_depth {
            let color = line::BLOCKQUOTE_COLORS[depth.min(5)];
            spans.push(Span::styled(
                line::BLOCKQUOTE_BAR.to_owned(),
                Style::default().fg(color),
            ));
            spans.push(Span::from(" "));
        }
    }

    // Create the horizontal rule line
    let rule_line = "─".repeat(available_width);
    spans.push(Span::styled(rule_line, rule_style));

    vec![Event::Parsed(
        document_id,
        WidgetSource {
            id: post_incr_source_id(source_id),
            height: 1,
            data: WidgetSourceData::Line(Line::from(spans), Vec::new()),
        },
    )]
}

fn code_block_to_events(
    document_id: DocumentId,
    source_id: &mut Option<usize>,
    width: u16,
    code: &str,
    blockquote_depth: usize,
) -> Vec<Event> {
    use ratatui::{
        style::{Color, Style},
        text::{Line, Span},
    };
    use unicode_width::UnicodeWidthStr as _;

    let code_style = Style::default()
        .fg(Color::Indexed(203))
        .bg(Color::Indexed(236));

    // Calculate available width for code (subtract blockquote prefix)
    let prefix_width = blockquote_depth * 2; // "▌ " per level
    let available_width = (width as usize).saturating_sub(prefix_width);

    code.lines()
        .map(|line| {
            let mut spans = Vec::new();

            // Add blockquote prefix if inside blockquote
            if blockquote_depth > 0 {
                for depth in 0..blockquote_depth {
                    let color = line::BLOCKQUOTE_COLORS[depth.min(5)];
                    spans.push(Span::styled(
                        line::BLOCKQUOTE_BAR.to_owned(),
                        Style::default().fg(color),
                    ));
                    spans.push(Span::from(" "));
                }
            }

            // Pad line to fill available width with background color
            let line_width = line.width();
            let padding = available_width.saturating_sub(line_width);
            let padded_line = format!("{}{}", line, " ".repeat(padding));

            spans.push(Span::styled(padded_line, code_style));

            Event::Parsed(
                document_id,
                WidgetSource {
                    id: post_incr_source_id(source_id),
                    height: 1,
                    data: WidgetSourceData::Line(Line::from(spans), Vec::new()),
                },
            )
        })
        .collect()
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

#[cfg(test)]
mod tests {
    use ratatui::{
        style::Color,
        text::{Line, Span},
    };

    use crate::{
        worker::pipeline::{
            line::{
                COLOR_LINK_BG, COLOR_LINK_FG, LINK_DESC_CLOSE, LINK_DESC_OPEN, LINK_URL_CLOSE,
                LINK_URL_OPEN,
            },
            markdown::MdParser,
            pipeline,
        },
        *,
    };
    use pretty_assertions::assert_eq;

    #[expect(clippy::unwrap_used)]
    fn parse(text: String, width: u16, has_text_size_protocol: bool) -> Vec<Event> {
        let mut parser = MdParser::new().unwrap();
        let (events, _) = pipeline(
            &mut parser,
            DocumentId::default(),
            width,
            has_text_size_protocol,
            text,
        )
        .unwrap();
        events
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

    #[test]
    fn list_rendering() {
        // Test that list prefixes, nesting, checkboxes, and inline code are preserved
        let input = r#"1. First ordered list item
2. Another item
   - Unordered sub-list.
3. Actual numbers don't matter, just that it's a number
   1. Ordered sub-list
4. And another item."#;

        let events = parse(input.to_owned(), 500, true);
        let lines: Vec<String> = events
            .iter()
            .filter_map(|ev| {
                if let Event::Parsed(_, source) = ev {
                    Some(source.to_string())
                } else {
                    None
                }
            })
            .collect();
        let output = lines.join("\n");
        assert_eq!(output, input);
    }

    #[test]
    fn list_checkboxes() {
        let input = r#"- [x] Checked item
- [ ] Unchecked item"#;

        // Expected output uses ✓ instead of x for checked items
        let expected = r#"- [✓] Checked item
- [ ] Unchecked item"#;

        let events = parse(input.to_owned(), 500, true);
        let lines: Vec<String> = events
            .iter()
            .filter_map(|ev| {
                if let Event::Parsed(_, source) = ev {
                    Some(source.to_string())
                } else {
                    None
                }
            })
            .collect();
        let output = lines.join("\n");
        assert_eq!(output, expected);
    }

    #[test]
    fn list_with_inline_code() {
        let input = "- Create a list by starting a line with `+`, `-`, or `*`";

        let events = parse(input.to_owned(), 500, true);
        let lines: Vec<String> = events
            .iter()
            .filter_map(|ev| {
                if let Event::Parsed(_, source) = ev {
                    Some(source.to_string())
                } else {
                    None
                }
            })
            .collect();
        let output = lines.join("\n");
        assert_eq!(output, input);
    }

    #[test]
    fn blockquote_rendering() {
        let input = r#"> This is a blockquote.
> Continuation of blockquote.
> > Nested blockquote
> > Continuation of nested blockquote."#;

        // Expected output uses ▌ instead of > for blockquote markers
        let expected = r#"▌ This is a blockquote.
▌ Continuation of blockquote.
▌ ▌ Nested blockquote
▌ ▌ Continuation of nested blockquote."#;

        let events = parse(input.to_owned(), 500, true);
        let lines: Vec<String> = events
            .iter()
            .filter_map(|ev| {
                if let Event::Parsed(_, source) = ev {
                    Some(source.to_string())
                } else {
                    None
                }
            })
            .collect();
        let output = lines.join("\n");
        assert_eq!(output, expected);
    }

    #[test]
    fn blockquote_with_blank_lines() {
        let input = r#"> First paragraph
>
> Second paragraph"#;

        // Expected output includes blank line with blockquote bar
        let expected = "▌ First paragraph\n▌ \n▌ Second paragraph";

        let events = parse(input.to_owned(), 500, true);
        let lines: Vec<String> = events
            .iter()
            .filter_map(|ev| {
                if let Event::Parsed(_, source) = ev {
                    Some(source.to_string())
                } else {
                    None
                }
            })
            .collect();
        let output = lines.join("\n");
        assert_eq!(output, expected);
    }

    #[test]
    fn code_block_spacing() {
        // Code blocks should have blank lines before and after them
        // even when the source markdown doesn't have blank lines
        let input = "Paragraph before.
```rust
let x = 1;
```
Paragraph after.";

        let events = parse(input.to_owned(), 80, true);
        let lines: Vec<String> = events
            .iter()
            .filter_map(|ev| {
                if let Event::Parsed(_, source) = ev {
                    Some(source.to_string())
                } else {
                    None
                }
            })
            .collect();
        let output = lines.join("\n");
        insta::assert_snapshot!(output);
    }

    #[test]
    fn code_block_before_list_spacing() {
        // Code block followed by list should have blank line between
        let input = "```rust
let x = 1;
```
- list item";

        let events = parse(input.to_owned(), 80, true);
        let lines: Vec<String> = events
            .iter()
            .filter_map(|ev| {
                if let Event::Parsed(_, source) = ev {
                    Some(source.to_string())
                } else {
                    None
                }
            })
            .collect();
        let output = lines.join("\n");
        insta::assert_snapshot!(output);
    }

    #[test]
    fn separate_blockquotes_have_blank_lines() {
        // Multiple separate blockquotes (separated by blank line or other content)
        // should have blank lines between them
        let input = r#"> Blockquotes are very handy in email to emulate reply text.
> This line is part of the same quote.

Quote break.

> This is a very long line that will still be quoted properly when it wraps.

> Blockquotes can also be nested...
>
> > ...by using additional greater-than signs right next to each other...
> >
> > > ...or with spaces between arrows."#;

        let events = parse(input.to_owned(), 500, true);
        let lines: Vec<String> = events
            .iter()
            .filter_map(|ev| {
                if let Event::Parsed(_, source) = ev {
                    Some(source.to_string())
                } else {
                    None
                }
            })
            .collect();
        let output = lines.join("\n");
        insta::assert_snapshot!(output);
    }

    #[test]
    fn table_rendering() {
        let input = r#"| Header 1 | Header 2 |
|----------|----------|
| Cell *1* | Cell 2   |
| Cell 3   | Cell 4   |"#;

        let events = parse(input.to_owned(), 80, true);
        let lines: Vec<String> = events
            .iter()
            .filter_map(|ev| {
                if let Event::Parsed(_, source) = ev {
                    Some(source.to_string())
                } else {
                    None
                }
            })
            .collect();
        let output = lines.join("\n");
        insta::assert_snapshot!(output);
    }

    #[test]
    fn table_with_alignment() {
        let input = r#"| Left | Center | Right |
|:-----|:------:|------:|
| L    |   C    |     R |"#;

        let events = parse(input.to_owned(), 80, true);
        let lines: Vec<String> = events
            .iter()
            .filter_map(|ev| {
                if let Event::Parsed(_, source) = ev {
                    Some(source.to_string())
                } else {
                    None
                }
            })
            .collect();
        let output = lines.join("\n");
        insta::assert_snapshot!(output);
    }
}
