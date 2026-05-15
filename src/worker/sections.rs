//! Section aggregation from Lines.
//!
//! This module provides `SectionIterator` which groups parsed lines into sections
//! for display. Lines are aggregated based on their type:
//! - Header lines become their own section
//! - Image lines become their own section
//! - All other lines are aggregated into text sections

use std::cell::RefCell;
use std::iter::Peekable;
use std::rc::Rc;

use mdfrier::ratatui::render_line;
use mdfrier::{Line, LineKind, Mapper, MarkdownLink};
use ratatui::text::Span;

use super::link_tracker::LinkTracker;
use crate::config::Theme;
use crate::document::{Section, SectionContent, SectionID};

/// Events produced during section iteration that need post-processing.
pub enum SectionEvent {
    Image(SectionID, MarkdownLink),
    Header(SectionID, String, u8),
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
        render_line(line, self.theme, None::<fn(&mdfrier::Span)>)
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
        let mut lines = vec![self.render_simple_line(first)];

        // Include trailing blank line if present (to maintain spacing)
        if let Some(peeked) = self.inner.peek() {
            if matches!(peeked.kind, LineKind::Blank) {
                let blank = self.inner.next().expect("peeked");
                lines.push(self.render_simple_line(blank));
            }
        }

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

        // Aggregate consecutive non-header, non-image lines
        while let Some(peeked) = self.inner.peek() {
            match &peeked.kind {
                // Stop aggregating at headers or images
                LineKind::Header(_) | LineKind::Image { .. } => break,
                // Continue aggregating all other lines (including blanks)
                _ => {
                    let line = self.inner.next().expect("peeked value should exist");
                    lines.push(line);
                }
            }
        }

        // Check if a header follows (need to preserve one blank line for spacing)
        let (followed_by_header, followed_by_image) = self
            .inner
            .peek()
            .map(|l| {
                (
                    matches!(l.kind, LineKind::Header(_)),
                    matches!(l.kind, LineKind::Image { .. }),
                )
            })
            .unwrap_or_default();

        // Trim trailing blank lines, unless image
        if !followed_by_image {
            while lines
                .last()
                .is_some_and(|l| matches!(l.kind, LineKind::Blank))
            {
                lines.pop();
            }
        }

        // Skip if section ended up empty after trimming
        if lines.is_empty() {
            return None;
        }

        // Re-add one blank line if needed for spacing before header
        if followed_by_header {
            lines.push(Line {
                kind: LineKind::Blank,
                spans: Vec::new(),
            });
        }

        let hide_urls = Mapper::hide_urls(self.theme);
        let link_tracker = Rc::new(RefCell::new(LinkTracker::default().hide_urls(hide_urls)));

        let rendered_lines: Vec<_> = lines
            .into_iter()
            .map(|line| {
                let link_tracker_inner = Rc::clone(&link_tracker);
                link_tracker_inner.borrow_mut().carriage_return();
                let lines = render_line(
                    line,
                    self.theme,
                    Some(move |node: &mdfrier::Span| {
                        link_tracker_inner.borrow_mut().track(node);
                    }),
                );
                (lines, link_tracker.borrow_mut().extras())
            })
            .collect();

        let id = self.next_section_id();
        Some(Section {
            id,
            height: rendered_lines.len() as u16,
            content: SectionContent::Lines(rendered_lines),
        })
    }
}

impl<I: Iterator<Item = Line>> Iterator for SectionIterator<'_, I> {
    type Item = Section;

    fn next(&mut self) -> Option<Self::Item> {
        // Return buffered section if available
        loop {
            let first = self.inner.next()?;

            // Clone kind to avoid partial move issues
            let kind = first.kind.clone();
            match kind {
                // Headers are always their own section
                LineKind::Header(tier) => return Some(self.process_header(first, tier)),

                // Images are always their own section
                LineKind::Image(link) => {
                    return Some(self.process_image(first, link));
                }

                // Skip blank lines at the start of a section
                LineKind::Blank => {
                    continue;
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
        #[expect(clippy::let_underscore_untyped, clippy::unwrap_used)]
        let _ = flexi_logger::Logger::try_with_env()
            .unwrap()
            .start()
            .inspect_err(|err| eprint!("test logger setup failed: {err}"));
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
        assert!(matches!(lines[0].1.as_slice(), [LineExtra::Link(..)]),);
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
        assert!(matches!(lines[0].1.as_slice(), [LineExtra::Link(..)]),);
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
                if let LineExtra::Link(url, ..) = extra {
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
                if let LineExtra::Link(url, ..) = extra {
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
                LineExtra::Link("http://example.com/link1".into(), 10, 18, None),
                LineExtra::Link("http://example.com/link2".into(), 30, 38, None),
            ]
        );
        assert_eq!(
            lines[1].1,
            vec![LineExtra::Link(
                "http://example.com/link3".into(),
                45,
                55,
                None
            ),]
        );
    }
}
