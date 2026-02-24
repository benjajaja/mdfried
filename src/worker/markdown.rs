//! Pipeline for parsing markdown into Events.
//!
//! Uses the mdfrier crate for parsing and styling, then converts to application Events.

use mdfrier::ratatui::{Tag, render_line};
use textwrap::{Options, wrap};

use crate::{
    MarkdownImage,
    big_text::BigText,
    config::Theme,
    document::{LineExtra, Section, SectionContent},
    worker::sections::{Section as MdSection, SectionKind},
};

pub enum SectionEvent {
    Image(usize, MarkdownImage),
    Header(usize, String, u8),
}

/// Convert an MdSection to application Events.
pub fn section_to_events(
    section_id: &mut Option<usize>,
    width: u16,
    has_text_size_protocol: bool,
    theme: &Theme,
    section: MdSection,
) -> (Vec<Section>, Vec<SectionEvent>) {
    match section.kind {
        SectionKind::Header(tier) => {
            let text: String = section
                .lines
                .first()
                .map(|line| line.spans.iter().map(|s| s.content.as_str()).collect())
                .unwrap_or_default();

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
                        content: SectionContent::Header(part.to_string(), tier, None),
                    })
                    .collect();
                (sections, Vec::new())
            } else {
                let id = post_incr_section_id(section_id);
                (
                    vec![Section {
                        id,
                        height: 2,
                        content: SectionContent::Header(text.clone(), tier, None),
                    }],
                    vec![SectionEvent::Header(id, text, tier)],
                )
            }
        }

        SectionKind::Image { url, description } => {
            let id = post_incr_section_id(section_id);
            let lines: Vec<_> = section
                .lines
                .into_iter()
                .map(|line| {
                    let (line, tags) = render_line(line, theme);
                    let links = extract_links(&line, tags);
                    (line, links)
                })
                .collect();
            (
                vec![Section {
                    id,
                    height: lines.len() as u16,
                    content: SectionContent::Lines(lines),
                }],
                vec![SectionEvent::Image(
                    id,
                    MarkdownImage {
                        destination: url,
                        description,
                    },
                )],
            )
        }

        SectionKind::Text => {
            let lines: Vec<_> = section
                .lines
                .into_iter()
                .map(|line| {
                    let (line, tags) = render_line(line, theme);
                    let links = extract_links(&line, tags);
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
                Vec::new(),
            )
        }
    }
}

/// Extract link info from tags, using span index to calculate character offsets.
fn extract_links(line: &ratatui::text::Line<'_>, tags: Vec<Tag>) -> Vec<LineExtra> {
    tags.into_iter()
        .filter_map(|tag| {
            if let Tag::Link(span_idx, url) = tag {
                // Sum widths of spans before this one to get character offset
                let offset: u16 = line.spans[..span_idx]
                    .iter()
                    .map(|s| unicode_width::UnicodeWidthStr::width(s.content.as_ref()) as u16)
                    .sum();
                let span_width =
                    unicode_width::UnicodeWidthStr::width(line.spans[span_idx].content.as_ref())
                        as u16;
                Some(LineExtra::Link(url, offset, offset + span_width))
            } else {
                None
            }
        })
        .collect()
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
        worker::{markdown::section_to_events, sections::SectionIterator},
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
        let lines = parser.parse(width, text, theme);
        for section in SectionIterator::new(lines) {
            let (sections, _section_events) = section_to_events(
                &mut section_id,
                width,
                has_text_size_protocol,
                theme,
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
                content: SectionContent::Header(text, tier, _),
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
                content: SectionContent::Header(text, tier, _),
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
