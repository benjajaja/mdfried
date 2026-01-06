//! Pipeline for parsing markdown into Events.
//!
//! Uses the mdfrier crate for parsing and styling, then converts to application Events.

use mdfrier::{
    Line as MdLine, LineKind, MdFrier,
    ratatui::{Tag, render_line},
};
use textwrap::{Options, wrap};

use crate::{
    Event, MarkdownImage,
    big_text::BigText,
    config::Theme,
    document::{LineExtra, Section, SectionContent},
    error::Error,
    model::DocumentId,
};

/// Main pipeline function that parses markdown text into Events.
pub fn parse_to_events(
    parser: &mut MdFrier,
    document_id: DocumentId,
    width: u16,
    has_text_size_protocol: bool,
    theme: &Theme,
    text: String,
) -> Result<(Vec<Event>, Option<usize>), Error> {
    let mut section_id: Option<usize> = None;
    let mut events = Vec::new();

    for md_line in parser.parse(width, text, theme) {
        let line_events = md_line_to_events(
            document_id,
            &mut section_id,
            width,
            has_text_size_protocol,
            theme,
            md_line,
        );
        events.extend(line_events);
    }

    Ok((events, section_id))
}

/// Convert an MdLine to application Events.
fn md_line_to_events(
    document_id: DocumentId,
    section_id: &mut Option<usize>,
    width: u16,
    has_text_size_protocol: bool,
    theme: &Theme,
    md_line: MdLine,
) -> Vec<Event> {
    // Handle special cases that need application-specific treatment
    match &md_line.kind {
        LineKind::Header(tier) => {
            let tier = *tier;
            let text: String = md_line.spans.iter().map(|s| s.content.as_str()).collect();

            if has_text_size_protocol {
                // Wrap header text for big text rendering
                let (n, d) = BigText::size_ratio(tier);
                let scaled_width = width as usize / 2 * usize::from(d) / usize::from(n);
                let options = Options::new(scaled_width)
                    .break_words(true)
                    .word_splitter(textwrap::word_splitters::WordSplitter::NoHyphenation);
                wrap(&text, options)
                    .iter()
                    .map(|part| {
                        Event::Parsed(
                            document_id,
                            Section {
                                id: post_incr_section_id(section_id),
                                height: 2,
                                content: SectionContent::Header(part.to_string(), tier),
                            },
                        )
                    })
                    .collect()
            } else {
                vec![Event::ParseHeader(
                    document_id,
                    post_incr_section_id(section_id),
                    tier,
                    text,
                )]
            }
        }
        LineKind::Image { url, description } => {
            vec![Event::ParsedImage(
                document_id,
                post_incr_section_id(section_id),
                MarkdownImage {
                    destination: url.clone(),
                    description: description.clone(),
                },
            )]
        }
        // All other lines: render via mdfrier and convert to Event
        _ => {
            let (line, tags) = render_line(md_line, theme);

            // Extract link info from tags, using span index to calculate character offsets
            let links: Vec<LineExtra> = tags
                .into_iter()
                .filter_map(|tag| {
                    if let Tag::Link(span_idx, url) = tag {
                        // Sum widths of spans before this one to get character offset
                        let offset: u16 = line.spans[..span_idx]
                            .iter()
                            .map(|s| {
                                unicode_width::UnicodeWidthStr::width(s.content.as_ref()) as u16
                            })
                            .sum();
                        let span_width = unicode_width::UnicodeWidthStr::width(
                            line.spans[span_idx].content.as_ref(),
                        ) as u16;
                        Some(LineExtra::Link(url, offset, offset + span_width))
                    } else {
                        None
                    }
                })
                .collect();

            vec![Event::Parsed(
                document_id,
                Section {
                    id: post_incr_section_id(section_id),
                    height: 1,
                    content: SectionContent::Line(line, links),
                },
            )]
        }
    }
}

/// Post-increment the source ID.
pub fn post_incr_section_id(section_id: &mut Option<usize>) -> usize {
    if section_id.is_none() {
        *section_id = Some(0);
        0
    } else {
        *section_id = section_id.map(|id| id + 1);
        section_id.unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        DocumentId, Event,
        config::Theme,
        document::{Section, SectionContent},
        worker::markdown::parse_to_events,
    };
    use mdfrier::MdFrier;

    #[expect(clippy::unwrap_used)]
    fn parse(text: String, width: u16, has_text_size_protocol: bool) -> Vec<Event> {
        let mut parser = MdFrier::new().unwrap();
        let (events, _) = parse_to_events(
            &mut parser,
            DocumentId::default(),
            width,
            has_text_size_protocol,
            &Theme::default(),
            text,
        )
        .unwrap();
        events
    }

    #[test]
    fn parse_header_wrapping_tier_1() {
        let events: Vec<Event> = parse("# 1234567890".to_owned(), 10, true);
        assert_eq!(2, events.len());

        let Event::Parsed(
            _,
            Section {
                content: SectionContent::Header(text, tier),
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
            Section {
                content: SectionContent::Header(text, tier),
                ..
            },
        ) = &events[1]
        else {
            panic!("expected Header");
        };
        assert_eq!(1, *tier);
        assert_eq!("67890", text);
    }
}
