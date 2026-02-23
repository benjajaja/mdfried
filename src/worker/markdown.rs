//! Pipeline for parsing markdown into Events.
//!
//! Uses the mdfrier crate for parsing and styling, then converts to application Events.

use mdfrier::{
    LineKind,
    ratatui::{Tag, render_line},
};
use textwrap::{Options, wrap};

use crate::{
    MarkdownImage,
    big_text::BigText,
    config::Theme,
    document::{LineExtra, Section, SectionContent},
};

pub enum SectionEvent {
    Header(usize, u8, String),
    Image(usize, MarkdownImage),
}

/// Convert an MdLine to application Events.
pub fn section_to_events(
    section_id: &mut Option<usize>,
    width: u16,
    has_text_size_protocol: bool,
    theme: &Theme,
    section: mdfrier::sections::Section,
) -> (Vec<Section>, Vec<SectionEvent>) {
    match section.kind {
        mdfrier::sections::SectionKind::Header => {
            let Some(line) = section.lines.first() else {
                panic!("Header section must have one line: {:?}", section.lines);
            };
            let LineKind::Header(tier) = line.kind else {
                panic!("SectionKind::Header first line not Header kind");
            };
            let text: String = line.spans.iter().map(|s| s.content.as_str()).collect();

            if has_text_size_protocol {
                // Wrap header text for big text rendering
                let (n, d) = BigText::size_ratio(tier);
                let scaled_width = width as usize / 2 * usize::from(d) / usize::from(n);
                let options = Options::new(scaled_width)
                    .break_words(true)
                    .word_splitter(textwrap::word_splitters::WordSplitter::NoHyphenation);
                let sections = wrap(&text, options)
                    .iter()
                    .map(|part| Section {
                        id: post_incr_section_id(section_id),
                        height: 2,
                        content: SectionContent::Header(part.to_string(), tier),
                    })
                    .collect();
                (sections, Vec::new())
            } else {
                let id = post_incr_section_id(section_id);
                (
                    vec![Section {
                        id,
                        height: 2,
                        content: SectionContent::Header(text.clone(), tier),
                    }],
                    vec![SectionEvent::Header(id, tier, text)],
                )
            }
        }
        // mdfrier::sections::SectionKind::Image => {
        // eprintln!("Image section");
        // let Some(line) = section.lines.first() else {
        // panic!("Image section must have one line");
        // };
        // let LineKind::Image { url, description } = &line.kind else {
        // panic!("Header section must have one header line");
        // };
        // vec![Event::ParsedImage(
        // document_id,
        // post_incr_section_id(section_id),
        // MarkdownImage {
        // destination: url.clone(),
        // description: description.clone(),
        // },
        // )]
        // }
        // All other lines: render via mdfrier and convert to Event
        _ => {
            let mut images = Vec::new();
            let lines: Vec<_> = section
                .lines
                .into_iter()
                .map(|line| {
                    if let LineKind::Image { url, description } = &line.kind {
                        images.push(MarkdownImage {
                            destination: url.clone(),
                            description: description.clone(),
                        });
                    }
                    let (line, tags) = render_line(line, theme);

                    // Extract link info from tags, using span index to calculate character offsets
                    let links: Vec<LineExtra> = tags
                        .into_iter()
                        .filter_map(|tag| {
                            if let Tag::Link(span_idx, url) = tag {
                                // Sum widths of spans before this one to get character offset
                                let offset: u16 = line.spans[..span_idx]
                                    .iter()
                                    .map(|s| {
                                        unicode_width::UnicodeWidthStr::width(s.content.as_ref())
                                            as u16
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
                    (line, links)
                })
                .collect();

            let id = post_incr_section_id(section_id);
            (
                vec![Section {
                    id,
                    height: lines.len() as u16,
                    content: SectionContent::Lines(lines),
                }],
                images
                    .into_iter()
                    .map(|img| SectionEvent::Image(id, img))
                    .collect(),
            )
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
        worker::markdown::section_to_events,
    };
    use mdfrier::MdFrier;

    /// Main pipeline function that parses markdown text into Events.
    fn parse_to_events(
        parser: &mut MdFrier,
        document_id: DocumentId,
        width: u16,
        has_text_size_protocol: bool,
        theme: &Theme,
        text: &str,
    ) -> (Vec<Event>, Option<usize>) {
        let mut events = Vec::new();
        let mut section_id: Option<usize> = None;
        for section in parser.parse_sections(width, &text, theme) {
            let (sections, section_events) = section_to_events(
                &mut section_id,
                width,
                has_text_size_protocol,
                &theme,
                section,
            );
            for section in sections {
                events.push(Event::Parsed(document_id, section));
            }
        }

        (events, section_id)
    }

    #[expect(clippy::unwrap_used)]
    fn parse(text: String, width: u16, has_text_size_protocol: bool) -> Vec<Event> {
        let mut parser = MdFrier::new().unwrap();
        let (events, _) = parse_to_events(
            &mut parser,
            DocumentId::default(),
            width,
            has_text_size_protocol,
            &Theme::default(),
            &text,
        );
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
