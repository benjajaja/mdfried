//! Section aggregation from Lines.
//!
//! This module provides `SectionIterator` which groups parsed lines into sections
//! for display. Lines are aggregated based on their type:
//! - Header lines become their own section
//! - Image lines become their own section
//! - All other lines are aggregated into text sections

use std::iter::Peekable;

use mdfrier::{Line, LineKind};

/// A section of parsed markdown content.
#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    /// Lines in this section.
    pub lines: Vec<Line>,
    /// The kind of section.
    pub kind: SectionKind,
}

/// The type of section content.
#[derive(Debug, Clone, PartialEq)]
pub enum SectionKind {
    /// Plain text content.
    Text,
    /// A header with its tier (1-6).
    Header(u8),
    /// An image with URL and description.
    Image { url: String, description: String },
}

/// Iterator that groups lines into sections.
pub struct SectionIterator<I: Iterator<Item = Line>> {
    inner: Peekable<I>,
}

impl<I: Iterator<Item = Line>> SectionIterator<I> {
    /// Create a new section iterator from a line iterator.
    pub fn new(inner: I) -> Self {
        SectionIterator {
            inner: inner.peekable(),
        }
    }
}

impl<I: Iterator<Item = Line>> Iterator for SectionIterator<I> {
    type Item = Section;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let first = self.inner.next()?;

            // Clone kind to avoid partial move issues
            let kind = first.kind.clone();
            match kind {
                // Headers are always their own section
                LineKind::Header(tier) => {
                    return Some(Section {
                        lines: vec![first],
                        kind: SectionKind::Header(tier),
                    });
                }

                // Images are always their own section
                LineKind::Image { url, description } => {
                    let mut lines = vec![first];
                    // Include trailing blank line if present (to maintain spacing)
                    if let Some(peeked) = self.inner.peek() {
                        if matches!(peeked.kind, LineKind::Blank) {
                            lines.push(self.inner.next().expect("peeked"));
                        }
                    }
                    return Some(Section {
                        lines,
                        kind: SectionKind::Image { url, description },
                    });
                }

                // Skip blank lines at the start of a section
                LineKind::Blank => {
                    continue;
                }

                // All other line types get aggregated into text sections
                _ => {
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

                    // Trim trailing blank lines
                    while lines.last().is_some_and(|l| matches!(l.kind, LineKind::Blank)) {
                        lines.pop();
                    }

                    // Skip if section ended up empty after trimming
                    if lines.is_empty() {
                        continue;
                    }

                    return Some(Section {
                        lines,
                        kind: SectionKind::Text,
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mdfrier::{MdFrier, Mapper};

    fn parse_sections(text: &str) -> Vec<Section> {
        let mut frier = MdFrier::new().unwrap();
        struct TestMapper;
        impl Mapper for TestMapper {}
        let lines = frier.parse(80, text, &TestMapper);
        SectionIterator::new(lines).collect()
    }

    #[test]
    fn header_is_own_section() {
        let sections = parse_sections("# Hello\n\nWorld");
        assert_eq!(sections.len(), 2);
        assert!(matches!(sections[0].kind, SectionKind::Header(1)));
        assert!(matches!(sections[1].kind, SectionKind::Text));
    }

    #[test]
    fn consecutive_text_aggregated() {
        let sections = parse_sections("Line 1\nLine 2\nLine 3");
        assert_eq!(sections.len(), 1);
        assert!(matches!(sections[0].kind, SectionKind::Text));
    }

    #[test]
    fn image_is_own_section() {
        let sections = parse_sections("Before\n\n![alt](http://example.com/img.png)\n\nAfter");
        assert_eq!(sections.len(), 3);
        assert!(matches!(sections[0].kind, SectionKind::Text));
        assert!(matches!(sections[1].kind, SectionKind::Image { .. }));
        assert!(matches!(sections[2].kind, SectionKind::Text));
    }

    #[test]
    fn multiple_headers() {
        let sections = parse_sections("# One\n\n## Two\n\n### Three");
        assert_eq!(sections.len(), 3);
        assert!(matches!(sections[0].kind, SectionKind::Header(1)));
        assert!(matches!(sections[1].kind, SectionKind::Header(2)));
        assert!(matches!(sections[2].kind, SectionKind::Header(3)));
    }
}
