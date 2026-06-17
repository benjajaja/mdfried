//! Section aggregation from Lines.
//!
//! This module provides `SectionIterator` which groups parsed lines into sections
//! for display. Lines are aggregated based on their type:
//! - Header lines become their own section
//! - Image lines become their own section
//! - All other lines are aggregated into text sections

use std::iter::Peekable;

use mdfrier::link_tracker::TrackedUrl;
use mdfrier::ratatui::{Theme as _, render_line};
use mdfrier::{Line, LineKind, MarkdownLink, Modifier, SourceContent};
use ratatui::text::Span;

use crate::config::Theme;
use crate::document::{LineExtra, LinkReference, Section, SectionContent, SectionID};

/// Events produced during section iteration that need post-processing.
pub enum SectionEvent {
    Image(SectionID, MarkdownLink, bool),
    Header(SectionID, String, u8),
    ReferenceDefinition { id: String, url: String },
    Code(SectionID, String, Vec<ratatui::prelude::Line<'static>>),
}

impl std::fmt::Display for SectionEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SectionEvent::Image(_, link, _has_trailing_blank) => write!(f, "Image({link})"),
            SectionEvent::Header(_, text, level) => write!(f, "H{level}({text})"),
            SectionEvent::ReferenceDefinition { id, url } => write!(f, "Ref({id} => {url})"),
            SectionEvent::Code(_, lang, _) => write!(f, "Code({lang})"),
        }
    }
}

/// Iterator that groups lines into sections and renders them.
pub struct SectionIterator<'a, I: Iterator<Item = Line>> {
    inner: Peekable<I>,
    theme: &'a Theme,
    section_id: usize,
}

impl<'a, I: Iterator<Item = Line>> SectionIterator<'a, I> {
    /// Create a new section iterator from a line iterator.
    pub fn new(inner: I, theme: &'a Theme) -> Self {
        SectionIterator {
            inner: inner.peekable(),
            theme,
            section_id: 0,
        }
    }

    /// Get the last section ID that was assigned (for ParseDone).
    pub fn last_section_id(&self) -> Option<usize> {
        if self.section_id == 0 {
            None
        } else {
            Some(self.section_id - 1)
        }
    }

    pub fn next_section_id(&mut self) -> SectionID {
        let id = self.section_id;
        self.section_id += 1;
        id
    }

    /// Render a line to ratatui Line without links, for headers, images, or other non-text
    /// content.
    fn render_simple_line(&self, line: Line) -> ratatui::text::Line<'static> {
        let (line, _) = render_line(line, self.theme);
        line
    }

    /// Process header lines into sections.
    fn process_header(&mut self, first: Line, tier: u8) -> Section {
        let text: String = first.spans.iter().map(|s| s.content.as_str()).collect();
        let id = self.next_section_id();
        if self.theme.has_text_size_protocol.unwrap_or_default() {
            return Section {
                id,
                height: 2,
                content: SectionContent::Header(text.clone(), tier, None),
            };
        }
        let mut lines = vec![self.render_simple_line(first)];
        if let Some(first) = lines.get_mut(0) {
            first.spans.insert(0, Span::from(" "));
            first.spans.insert(0, Span::from("#".repeat(tier as usize)));
        }
        Section {
            id,
            height: 2,
            content: SectionContent::HeaderPlaceholder(
                text.clone(),
                tier,
                lines.into_iter().map(|line| (line, Vec::new())).collect(),
            ),
        }
    }

    /// Process image lines into a section.
    fn process_image(&mut self, first: Line, link: MarkdownLink) -> Section {
        let id = self.next_section_id();
        let lines = vec![self.render_simple_line(first)];

        Section {
            id,
            height: lines.len() as u16,
            content: SectionContent::ImagePlaceholder(
                link,
                lines.into_iter().map(|line| (line, Vec::new())).collect(),
            ),
        }
    }

    /// Process text lines (paragraphs, code blocks, tables, etc.) into a section.
    fn process_text(&mut self, first: Line) -> Option<Section> {
        let mut lines = vec![first];

        // Aggregate consecutive "text" lines
        while let Some(peeked) = self.inner.peek() {
            match &peeked.kind {
                // Stop aggregating at headers or images
                LineKind::Header(_) | LineKind::Image { .. } | LineKind::CodeBlock { .. } => break,
                // Continue aggregating all other lines (including blanks)
                _ => {
                    let line = self.inner.next().expect("peeked value should exist");
                    lines.push(line);
                }
            }
        }

        // Skip if section ended up empty
        if lines.is_empty() {
            return None;
        }

        let rendered_lines: Vec<_> = lines
            .into_iter()
            .map(|line| {
                let mut link_reference_definition =
                    if line.kind == LineKind::LinkReferenceDefinitions {
                        let reference = line.spans.iter().find_map(|span| {
                            span.modifiers
                                .contains(Modifier::LinkDescription)
                                .then(|| span.content.clone())
                        });
                        if reference.is_none() {
                            log::error!("LineKind::LinkReferenceDefinitions but no LinkDescription span for the reference-id");
                            log::debug!("line: {line:?}");
                        }
                        reference
                    } else {
                        None
                    };

                let (ratatui_line, urls) = render_line(line, self.theme);

                let extras: Vec<LineExtra> = urls
                    .into_iter()
                    .filter_map(|tracked_url| {
                        if let TrackedUrl::Link {
                            start,
                            lines,
                            end,
                            url,
                            is_reference,
                        } = tracked_url
                        {
                            Some(LineExtra::Link {
                                source: SourceContent::from(url.as_str()),
                                start,
                                end,
                                lines: if lines == 0 { None } else { Some(lines) },
                                // Build the reference, both on the links that point the reference
                                // definition, and the reference definitions.
                                // The worker emits a special event on definitions for the document
                                // to update all reference links, after all `Parse` events.
                                reference: if is_reference {
                                    LinkReference::Reference { id: url }
                                } else if let Some(id) = link_reference_definition.take() {
                                    // We can take it because there should only be one
                                    // ReferenceDefinition per line.
                                    LinkReference::ReferenceDefinition { id, url }
                                } else {
                                    LinkReference::None
                                },
                            })
                        } else {
                            None
                        }
                    })
                    .collect();

                (ratatui_line, extras)
            })
            .collect();

        let id = self.next_section_id();
        Some(Section {
            id,
            height: rendered_lines.len() as u16,
            content: SectionContent::Lines(rendered_lines),
        })
    }

    fn process_codeblock(&mut self, first: Line, language: String) -> Section {
        let to_line = |line: Line| {
            line.spans
                .into_iter()
                .map(|span| Span::from(span.content).style(self.theme.code_style()))
                .collect()
        };
        let mut lines = vec![to_line(first)];

        // Aggregate consecutive code lines
        while let Some(peeked) = self.inner.peek() {
            match &peeked.kind {
                LineKind::CodeBlock {
                    language: next_language,
                } if *next_language == language => {
                    let line = self.inner.next().expect("peeked value should exist");
                    lines.push(to_line(line));
                }
                _ => {
                    break;
                }
            }
        }

        let height = lines.len() as u16;
        let id = self.next_section_id();
        let lines = lines.into_iter().map(|line| (line, Vec::new())).collect();
        Section {
            id,
            height,
            content: SectionContent::Code(language, lines),
        }
    }
}

impl<I: Iterator<Item = Line>> Iterator for SectionIterator<'_, I> {
    type Item = Section;

    fn next(&mut self) -> Option<Self::Item> {
        // Return buffered section if available
        loop {
            let first = self.inner.next()?;

            match first.kind {
                // Headers are always their own section
                LineKind::Header(tier) => return Some(self.process_header(first, tier)),

                // Images are always their own section
                #[expect(clippy::ref_patterns)]
                LineKind::Image(ref link) => {
                    let link = link.clone();
                    return Some(self.process_image(first, link));
                }

                #[expect(clippy::ref_patterns)]
                LineKind::CodeBlock { ref language } => {
                    let language = language.clone();
                    return Some(self.process_codeblock(first, language));
                }

                // All other line types get aggregated into text sections
                _ => {
                    if let Some(section) = self.process_text(first) {
                        return Some(section);
                    }
                    // Section was empty after trimming, continue to next
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{LineExtra, SectionContent};
    use mdfrier::{MdFrier, SourceContent};

    #[ctor::ctor]
    fn init_logger() {
        crate::debug::init_test_logger();
    }

    #[expect(clippy::unwrap_used)]
    fn parse_sections(text: &str) -> Vec<Section> {
        let mut frier = MdFrier::new().unwrap();
        let theme = Theme::default();
        let lines = frier.parse(80, text, &theme).unwrap();
        SectionIterator::new(lines, &theme).collect()
    }

    #[test]
    fn header_is_own_section() {
        let sections = parse_sections("# Hello\n\nWorld");
        assert_eq!(sections.len(), 2);
        assert!(matches!(
            sections[0].content,
            SectionContent::HeaderPlaceholder(_, 1, _)
        ));
        assert!(matches!(sections[1].content, SectionContent::Lines(_)));
    }

    #[test]
    fn consecutive_text_aggregated() {
        let sections = parse_sections("Line 1\nLine 2\nLine 3");
        assert_eq!(sections.len(), 1);
        assert!(matches!(sections[0].content, SectionContent::Lines(_)));
    }

    #[test]
    fn image_is_own_section() {
        let sections = parse_sections("Before\n\n![alt](http://example.com/img.png)\n\nAfter");
        assert_eq!(sections.len(), 3);
        assert!(matches!(sections[0].content, SectionContent::Lines(_)));
        assert!(matches!(
            sections[1].content,
            SectionContent::ImagePlaceholder(_, _)
        ));
        assert!(matches!(sections[2].content, SectionContent::Lines(_)));
    }

    #[test]
    fn multiple_headers() {
        let sections = parse_sections("# One\n\n## Two\n\n### Three");
        assert_eq!(sections.len(), 3);
        assert!(matches!(
            sections[0].content,
            SectionContent::HeaderPlaceholder(_, 1, _)
        ));
        assert!(matches!(
            sections[1].content,
            SectionContent::HeaderPlaceholder(_, 2, _)
        ));
        assert!(matches!(
            sections[2].content,
            SectionContent::HeaderPlaceholder(_, 3, _)
        ));
    }

    #[test]
    #[expect(clippy::unwrap_used)]
    fn header_wrapping_tier_1() {
        let mut frier = MdFrier::new().unwrap();
        let theme = Theme {
            has_text_size_protocol: Some(true),
            ..Default::default()
        };
        let lines = frier.parse(10, "# 1234567890", &theme).unwrap();
        let sections: Vec<Section> = SectionIterator::new(lines, &theme).collect();

        assert_eq!(sections.len(), 2);

        let SectionContent::Header(text, tier, _) = &sections[0].content else {
            panic!("expected Header");
        };
        assert_eq!(1, *tier);
        assert_eq!("12345", text);

        let SectionContent::Header(text, tier, _) = &sections[1].content else {
            panic!("expected Header");
        };
        assert_eq!(1, *tier);
        assert_eq!("67890", text);
    }

    #[test]
    fn image_after_blank() {
        let sections = parse_sections("Before\n\n![alt](http://example.com/img.png)");
        assert_eq!(sections.len(), 2);
        assert!(matches!(sections[0].content, SectionContent::Lines(_)));
        assert!(matches!(
            sections[1].content,
            SectionContent::ImagePlaceholder(_, _)
        ));
        let SectionContent::Lines(lines) = &sections[0].content else {
            panic!("expected SectionContent::Lines");
        };
        assert_eq!(lines.len(), 2, "two lines");
    }

    #[test]
    fn md_link_parses_as_section_with_one_link() {
        let sections = parse_sections("[example](https://example.org/)\n");
        assert_eq!(sections.len(), 1);
        assert!(matches!(sections[0].content, SectionContent::Lines(_)));
        let SectionContent::Lines(lines) = &sections[0].content else {
            panic!("expected SectionContent::Lines");
        };
        assert_eq!(lines.len(), 1, "one line");
        assert!(matches!(lines[0].1.as_slice(), [LineExtra::Link { .. }]),);
    }

    #[test]
    fn md_link_with_code_block_parses_as_section_with_one_link() {
        let sections = parse_sections("[example `code`](https://example.org/)\n");
        assert_eq!(sections.len(), 1);
        assert!(matches!(sections[0].content, SectionContent::Lines(_)));
        let SectionContent::Lines(lines) = &sections[0].content else {
            panic!("expected SectionContent::Lines");
        };
        assert_eq!(lines.len(), 1);
        assert!(matches!(lines[0].1.as_slice(), [LineExtra::Link { .. }]),);
    }

    #[test]
    fn link_with_multiple_spans_has_correct_url() {
        let url = "https://example.com/target";
        let markdown = format!("unrelated [text with `code`]({})", url);

        let sections = parse_sections(&markdown);
        assert_eq!(sections.len(), 1);

        let SectionContent::Lines(lines) = &sections[0].content else {
            panic!("expected SectionContent::Lines");
        };
        assert_eq!(lines.len(), 1, "one line");

        let link_extras: Vec<_> = lines[0]
            .1
            .iter()
            .filter_map(|extra| {
                if let LineExtra::Link { source: url, .. } = extra {
                    Some(url)
                } else {
                    None
                }
            })
            .collect();

        log::debug!("TEST LOG");
        assert_eq!(link_extras.len(), 1);
        assert_eq!(link_extras[0].as_ref(), url,);
    }

    #[test]
    fn nested_image_link() {
        let markdown = "[![test image](http://example.com/image.png)](http://example.com/link)";

        let sections = parse_sections(markdown);

        let SectionContent::Lines(lines) = &sections[0].content else {
            panic!("expected SectionContent::Lines");
        };

        let link_extras: Vec<_> = lines[0]
            .1
            .iter()
            .filter_map(|extra| {
                if let LineExtra::Link { source: url, .. } = extra {
                    Some(url)
                } else {
                    None
                }
            })
            .collect();

        assert_eq!(
            link_extras,
            vec![&SourceContent::from("http://example.com/link")]
        );
    }

    #[test]
    fn multiple_links() {
        let markdown = r#"Here goes [link one](http://example.com/link1), here goes [link two](http://example.com/link2).  
Definitely on another line (soft-break) goes [link three](http://example.com/link3).  
That's all."#;

        let sections = parse_sections(markdown);

        assert_eq!(1, sections.len());
        let SectionContent::Lines(lines) = &sections[0].content else {
            panic!("expected SectionContent::Lines");
        };

        assert_eq!(
            lines[0].0.to_string(),
            String::from("Here goes link one, here goes link two."),
        );
        assert_eq!(
            lines[0].1,
            vec![
                LineExtra::Link {
                    source: "http://example.com/link1".into(),
                    start: 10,
                    end: 18,
                    lines: None,
                    reference: LinkReference::None,
                },
                LineExtra::Link {
                    source: "http://example.com/link2".into(),
                    start: 30,
                    end: 38,
                    lines: None,
                    reference: LinkReference::None,
                },
            ]
        );
        assert_eq!(
            lines[1].1,
            vec![LineExtra::Link {
                source: "http://example.com/link3".into(),
                start: 45,
                end: 55,
                lines: None,
                reference: LinkReference::None,
            },]
        );
    }
}
