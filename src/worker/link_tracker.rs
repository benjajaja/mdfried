//! Track links and track back URLs.

use std::mem::swap;

use mdfrier::{Modifier as MdModifier, SourceContent};
use unicode_width::UnicodeWidthStr as _;

use crate::document::LineExtra;

#[derive(Default)]
/// Iterator over [`mdfrier::Span`]s and extract links as [`LineExtra::Link`]`(url, start, end)`,
/// where "start" and "end" are the respective positions in the [`mdfrier::Line`].
pub struct LinkTracker {
    offset: u16,
    // line_count: usize,
    extras: Vec<LineExtra>,
    link_builder: LinkExtraLinkBuilder,
    hide_urls: bool,
}

#[derive(Debug, Default, PartialEq)]
enum LinkExtraLinkBuilder {
    #[default]
    None,
    Start {
        start: u16,
        lines: usize,
    },
    StartEnd {
        start: u16,
        end: u16,
        lines: usize,
    },
    StartEndUrl {
        start: u16,
        end: u16,
        lines: usize,
        url: String,
    },
}

impl LinkTracker {
    pub fn hide_urls(mut self, hide_urls: bool) -> LinkTracker {
        self.hide_urls = hide_urls;
        self
    }

    pub fn carriage_return(&mut self) {
        self.offset = 0;
        // Only increase line count if we haven't reached "end" yet.
        if let LinkExtraLinkBuilder::Start { lines, .. } = &mut self.link_builder {
            *lines += 1;
        }
    }
    pub fn track(&mut self, node: &mdfrier::Span) {
        let span_width = node.content.width() as u16;
        if self.link_builder == LinkExtraLinkBuilder::None
            && node
                .modifiers
                .is_link_modifier(MdModifier::LinkDescriptionWrapper)
        {
            // Enter link description at next span.
            self.link_builder = LinkExtraLinkBuilder::Start {
                start: self.offset + span_width,
                lines: 0,
            };
        } else if let LinkExtraLinkBuilder::Start { start, lines } = self.link_builder
            && node
                .modifiers
                .is_link_modifier(MdModifier::LinkDescriptionWrapper)
        {
            // Exit link description before this span.
            self.link_builder = LinkExtraLinkBuilder::StartEnd {
                start,
                end: self.offset,
                lines,
            };
        } else if let LinkExtraLinkBuilder::StartEnd { start, end, lines } = self.link_builder
            && node.modifiers.is_link_modifier(MdModifier::LinkURLWrapper)
        {
            // Enter link URL next span.
            self.link_builder = LinkExtraLinkBuilder::StartEndUrl {
                start,
                end,
                lines,
                url: String::new(),
            };
        } else if let LinkExtraLinkBuilder::StartEndUrl { url, .. } = &mut self.link_builder
            && node.modifiers.is_link_url()
        {
            // Push all LinkURL spans into URL.
            url.push_str(&node.content);
        } else if node.modifiers.is_link_modifier(MdModifier::LinkURLWrapper)
            && matches!(self.link_builder, LinkExtraLinkBuilder::StartEndUrl { .. })
        {
            let LinkExtraLinkBuilder::StartEndUrl {
                start,
                end,
                lines,
                url,
            } = std::mem::take(&mut self.link_builder)
            else {
                // We need to "take if matches", so we can't put the destructure in the `if` above.
                unreachable!("invariant by matches macro");
            };
            // Exit URL, can build the LinkExtra::Link now.
            self.exit(start, end, lines, url.as_str());
        }
        if self.hide_urls {
            if !node.modifiers.is_link_url() {
                self.offset += span_width;
            }
        } else {
            self.offset += span_width;
        }
    }
    pub fn extras(&mut self) -> Vec<LineExtra> {
        let mut extras = Vec::new();
        swap(&mut self.extras, &mut extras);
        extras
    }
    fn exit(&mut self, start: u16, end: u16, lines: usize, url: &str) {
        let lines = if lines == 0 { None } else { Some(lines) };
        self.extras
            .push(LineExtra::Link(SourceContent::from(url), start, end, lines));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mdfrier::Span;
    use pretty_assertions::assert_eq;

    #[ctor::ctor]
    fn init_logger() {
        crate::debug::init_test_logger();
    }

    fn test_link(description: &str, url: &str) -> Vec<Span> {
        vec![
            Span::new(
                "[".to_owned(),
                MdModifier::Link | MdModifier::LinkDescriptionWrapper,
            ),
            Span::new(
                description.to_owned(),
                MdModifier::Link | MdModifier::LinkDescription,
            ),
            Span::new(
                "]".to_owned(),
                MdModifier::Link | MdModifier::LinkDescriptionWrapper,
            ),
            Span::new(
                "(".to_owned(),
                MdModifier::Link | MdModifier::LinkURLWrapper,
            ),
            Span::new(url.to_owned(), MdModifier::Link | MdModifier::LinkURL),
            Span::new(
                ")".to_owned(),
                MdModifier::Link | MdModifier::LinkURLWrapper,
            ),
        ]
    }

    #[test]
    fn track_link() {
        let mut tracker = LinkTracker::default();
        for span in test_link("desc", "url") {
            tracker.track(&span);
        }
        let extras = tracker.extras();
        assert_eq!(
            extras[0],
            LineExtra::Link(SourceContent::from("url"), 1, 5, None)
        );
    }

    #[test]
    fn track_nested_image() {
        let mut tracker = LinkTracker::default();
        let mut spans = test_link("desc", "url");
        spans.splice(
            1..2,
            [
                Span::new(
                    "![".to_owned(),
                    MdModifier::Image | MdModifier::Link | MdModifier::LinkDescriptionWrapper,
                ),
                Span::new(
                    "image".to_owned(),
                    MdModifier::Image | MdModifier::Link | MdModifier::LinkDescription,
                ),
                Span::new(
                    "]".to_owned(),
                    MdModifier::Image | MdModifier::Link | MdModifier::LinkDescriptionWrapper,
                ),
                Span::new(
                    "(".to_owned(),
                    MdModifier::Image | MdModifier::Link | MdModifier::LinkURLWrapper,
                ),
                Span::new(
                    "image_url".to_owned(),
                    MdModifier::Link | MdModifier::LinkURL,
                ),
                Span::new(
                    ")".to_owned(),
                    MdModifier::Image | MdModifier::Link | MdModifier::LinkURLWrapper,
                ),
            ],
        );
        for span in spans {
            tracker.track(&span);
        }
        let extras = tracker.extras();
        assert_eq!(
            extras[0],
            LineExtra::Link(SourceContent::from("url"), 1, 20, None)
        );
    }

    #[test]
    fn track_wrapped_link() {
        let mut tracker = LinkTracker::default();

        tracker.track(&Span::new(
            "[".to_owned(),
            MdModifier::Link | MdModifier::LinkDescriptionWrapper,
        ));
        tracker.track(&Span::new(
            "desc".to_owned(),
            MdModifier::Link | MdModifier::LinkDescription,
        ));
        tracker.carriage_return();

        tracker.track(&Span::new(
            "cont".to_owned(),
            MdModifier::Link | MdModifier::LinkDescription,
        ));
        tracker.track(&Span::new(
            "]".to_owned(),
            MdModifier::Link | MdModifier::LinkDescriptionWrapper,
        ));
        tracker.track(&Span::new(
            "(".to_owned(),
            MdModifier::Link | MdModifier::LinkURLWrapper,
        ));
        tracker.carriage_return();

        tracker.track(&Span::new(
            "url".to_owned(),
            MdModifier::Link | MdModifier::LinkURL,
        ));
        tracker.track(&Span::new(
            ")".to_owned(),
            MdModifier::Link | MdModifier::LinkURLWrapper,
        ));

        let extras = tracker.extras();
        assert_eq!(
            extras[0],
            LineExtra::Link(SourceContent::from("url"), 1, 4, Some(1)),
        );
    }

    #[test]
    fn track_multiple_wraps_link() {
        let mut tracker = LinkTracker::default();

        tracker.track(&Span::new("nothing ".to_owned(), MdModifier::default()));

        tracker.track(&Span::new(
            "[".to_owned(),
            MdModifier::Link | MdModifier::LinkDescriptionWrapper,
        ));
        tracker.track(&Span::new(
            "desc1".to_owned(),
            MdModifier::Link | MdModifier::LinkDescription,
        ));
        tracker.carriage_return();

        tracker.track(&Span::new(
            "desc2".to_owned(),
            MdModifier::Link | MdModifier::LinkDescription,
        ));
        tracker.carriage_return();

        tracker.track(&Span::new(
            "desc3".to_owned(),
            MdModifier::Link | MdModifier::LinkDescription,
        ));

        tracker.track(&Span::new(
            "]".to_owned(),
            MdModifier::Link | MdModifier::LinkDescriptionWrapper,
        ));
        tracker.track(&Span::new(
            "(".to_owned(),
            MdModifier::Link | MdModifier::LinkURLWrapper,
        ));
        tracker.carriage_return();

        tracker.track(&Span::new(
            "url-1/".to_owned(),
            MdModifier::Link | MdModifier::LinkURL,
        ));
        tracker.carriage_return();

        tracker.track(&Span::new(
            "url-2".to_owned(),
            MdModifier::Link | MdModifier::LinkURL,
        ));
        tracker.carriage_return();

        tracker.track(&Span::new(
            ")".to_owned(),
            MdModifier::Link | MdModifier::LinkURLWrapper,
        ));

        let extras = tracker.extras();
        // ```
        // nothing [desc1
        // desc2
        // desc3]
        // ```
        // So we want: start at "[" 9, end at "]" 5, two lines "up", and correct URL
        // reconstruction.
        assert_eq!(
            extras[0],
            LineExtra::Link(SourceContent::from("url-1/url-2"), 9, 5, Some(2)),
        );
    }
}
