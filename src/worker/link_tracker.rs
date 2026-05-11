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
    extras: Vec<LineExtra>,
    link_builder: LinkExtraLinkBuilder,
}

#[derive(Debug, Default, PartialEq)]
enum LinkExtraLinkBuilder {
    #[default]
    None,
    Start(u16),
    StartEnd(u16, u16),
    StartEndUrl(u16, u16, String),
}

impl LinkTracker {
    pub fn carriage_return(&mut self) {
        self.offset = 0;
    }
    pub fn track(&mut self, node: &mdfrier::Span) {
        let span_width = node.content.width() as u16;
        if self.link_builder == LinkExtraLinkBuilder::None
            && node
                .modifiers
                .is_link_modifier(MdModifier::LinkDescriptionWrapper)
        {
            // Enter link description at next span.
            self.link_builder = LinkExtraLinkBuilder::Start(self.offset + span_width);
        } else if let LinkExtraLinkBuilder::Start(start) = self.link_builder
            && node
                .modifiers
                .is_link_modifier(MdModifier::LinkDescriptionWrapper)
        {
            // Exit link description before this span.
            self.link_builder = LinkExtraLinkBuilder::StartEnd(start, self.offset);
        } else if let LinkExtraLinkBuilder::StartEnd(start, end) = self.link_builder
            && node.modifiers.is_link_modifier(MdModifier::LinkURLWrapper)
        {
            // Enter link URL next span.
            self.link_builder = LinkExtraLinkBuilder::StartEndUrl(start, end, String::new());
        } else if let LinkExtraLinkBuilder::StartEndUrl(_, _, url) = &mut self.link_builder
            && node.modifiers.is_link_url()
        {
            // Push all LinkURL spans into URL.
            url.push_str(&node.content);
        } else if node.modifiers.is_link_modifier(MdModifier::LinkURLWrapper)
            && matches!(
                self.link_builder,
                LinkExtraLinkBuilder::StartEndUrl(_, _, _)
            )
        {
            let LinkExtraLinkBuilder::StartEndUrl(start, end, url) =
                std::mem::take(&mut self.link_builder)
            else {
                unreachable!("invariant by matches macro");
            };
            // Exit URL, can build the LinkExtra::Link now.
            self.extras
                .push(LineExtra::Link(SourceContent::from(&*url), start, end));
        }
        self.offset += span_width;
    }
    pub fn extras(&mut self) -> Vec<LineExtra> {
        let mut extras = Vec::new();
        swap(&mut self.extras, &mut extras);
        extras
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mdfrier::Span;
    use pretty_assertions::assert_eq;

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
        assert_eq!(extras[0], LineExtra::Link(SourceContent::from("url"), 1, 5));
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
            LineExtra::Link(SourceContent::from("url"), 1, 20)
        );
    }
}
