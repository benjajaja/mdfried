//! Track links and track back URLs.

use std::mem::swap;

use crate::{Modifier, Span};
use unicode_width::UnicodeWidthStr as _;

#[derive(Debug, PartialEq)]
pub enum TrackedUrl {
    Link {
        start: u16,
        lines: usize,
        end: u16,
        url: String,
        is_reference: bool,
    },
    Image {
        desc: String,
        url: String,
    },
}
impl TrackedUrl {
    pub fn link<S: Into<String>>(url: S, start: u16, end: u16, lines: usize) -> Self {
        Self::Link {
            start,
            lines,
            end,
            url: url.into(),
            is_reference: false,
        }
    }
    pub fn link_reference<S: Into<String>>(url: S, start: u16, end: u16, lines: usize) -> Self {
        Self::Link {
            start,
            lines,
            end,
            url: url.into(),
            is_reference: true,
        }
    }
    pub fn image<S: Into<String>>(desc: S, url: S) -> Self {
        Self::Image {
            desc: desc.into(),
            url: url.into(),
        }
    }
}

#[derive(Default, Debug)]
/// Iterator over [`mdfrier::Span`]s and extract links as [`LineExtra::Link`] with `source`, `start`, `end` fields,
/// where "start" and "end" are the respective positions in the [`mdfrier::Line`].
pub struct LinkTracker {
    offset: u16,
    urls: Vec<TrackedUrl>,
    state: LinkState,
    nested_state: LinkState,
    hide_urls: bool,
}

#[derive(Default, Debug)]
enum LinkState {
    #[default]
    None,
    LinkDescOpen,
    LinkDesc(u16, usize),
    LinkDescClose(u16, usize, u16),
    LinkUrlOpen(u16, usize, u16),
    LinkUrl(u16, usize, u16, String),
    /// A bare URL being accumulated. Fields: start col, lines above anchor, end col, url so far.
    /// `end` is stored explicitly so it survives the offset reset in `carriage_return`.
    BareLink(u16, usize, u16, String),
    ImageDesc(String),
    ImageUrl(String, String),
}

impl LinkTracker {
    pub fn hide_urls(mut self, hide_urls: bool) -> LinkTracker {
        self.hide_urls = hide_urls;
        self
    }

    /// `next` is the modifiers of the first span on the following line, if any.
    /// Used to decide whether a pending bare link continues or ends here.
    /// Returns the last offset (ending) if mid-link.
    pub fn carriage_return(&mut self, next: Option<Modifier>) -> Option<u16> {
        let continues = next.is_some_and(|m| m.contains(Modifier::BareLink | Modifier::LinkURL));
        if let LinkState::BareLink(..) = &self.state {
            if !continues {
                if let LinkState::BareLink(start, lines, end, url) = std::mem::take(&mut self.state)
                {
                    self.urls.push(TrackedUrl::link(url, start, end, lines));
                }
            }
        }
        let end_offset = self.offset;
        self.offset = 0;
        if let LinkState::LinkDesc(_, lines) = &mut self.state {
            *lines += 1;
        }
        if let LinkState::BareLink(_, lines, ..) = &mut self.state {
            *lines += 1;
        }

        match self.state {
            LinkState::BareLink(..) if continues => Some(end_offset),
            LinkState::LinkDesc(..) => Some(end_offset),
            _ => None,
        }
    }
    pub fn track(&mut self, node: &Span) {
        use LinkState::*;

        let Span { modifiers, content } = &node;
        let span_width = content.width() as u16;

        self.state = match std::mem::take(&mut self.state) {
            None if modifiers.contains(Modifier::Link | Modifier::LinkDescriptionWrapper) => {
                LinkDescOpen
            }
            LinkDescOpen if modifiers.contains(Modifier::Link | Modifier::LinkDescription) => {
                LinkDesc(self.offset, 0)
            }
            keep @ LinkDesc(..)
                if modifiers.contains(Modifier::Link | Modifier::LinkDescription) =>
            {
                keep
            }
            LinkDesc(start, lines)
                if modifiers.contains(Modifier::Link | Modifier::LinkDescriptionWrapper) =>
            {
                LinkDescClose(start, lines, self.offset)
            }
            LinkDescClose(start, lines, end)
                if modifiers.contains(Modifier::Link | Modifier::LinkURLWrapper) =>
            {
                LinkUrlOpen(start, lines, end)
            }
            LinkDescClose(start, lines, end)
                if modifiers.contains(Modifier::Link | Modifier::LinkURL) =>
            {
                // full_reference_link
                self.urls.push(TrackedUrl::link_reference(
                    content.clone(),
                    start,
                    end,
                    lines,
                ));
                None
            }
            LinkUrlOpen(start, lines, end)
                if modifiers.contains(Modifier::Link | Modifier::LinkURL) =>
            {
                LinkUrl(start, lines, end, content.clone())
            }
            LinkUrl(start, lines, end, mut url)
                if modifiers.contains(Modifier::Link | Modifier::LinkURL) =>
            {
                url.push_str(content);
                LinkUrl(start, lines, end, url)
            }
            LinkUrl(start, lines, end, url)
                if modifiers.contains(Modifier::Link | Modifier::LinkURLWrapper) =>
            {
                self.urls.push(TrackedUrl::link(url, start, end, lines));
                None
            }
            None if modifiers.contains(Modifier::BareLink | Modifier::LinkURL) => {
                BareLink(self.offset, 0, self.offset + span_width, content.clone())
            }
            BareLink(start, lines, _, mut url)
                if modifiers.contains(Modifier::BareLink | Modifier::LinkURL) =>
            {
                url.push_str(content);
                BareLink(start, lines, self.offset + span_width, url)
            }
            BareLink(start, lines, end, url) => {
                self.urls.push(TrackedUrl::link(url, start, end, lines));
                self.track_images(None, modifiers, content)
            }
            state => self.track_images(state, modifiers, content),
        };

        // Track images nested in links at the same time.
        if matches!(self.state, LinkDesc(..)) {
            let state = std::mem::take(&mut self.nested_state);
            self.nested_state = self.track_images(state, modifiers, content);
        }

        if self.hide_urls {
            if !node.modifiers.is_link_url() {
                self.offset += span_width;
            }
        } else {
            self.offset += span_width;
        }
    }

    fn track_images(&mut self, state: LinkState, modifiers: &Modifier, content: &str) -> LinkState {
        // Nested images carry over LinkDescription, so we can't only rely on that, we must also
        // check if content is a wrapper marker. Tree-sitter doesn't produce nodes for those for
        // some reason.
        use LinkState::*;
        match state {
            None if modifiers.contains(Modifier::Image | Modifier::LinkDescription) => {
                ImageDesc(if content != "![" {
                    String::from(content)
                } else {
                    String::new()
                })
            }
            ImageDesc(mut desc)
                if modifiers.contains(Modifier::Image | Modifier::LinkDescription) =>
            {
                if content != "](" {
                    desc.push_str(content);
                    ImageDesc(desc)
                } else {
                    ImageUrl(desc, String::new())
                }
            }
            ImageDesc(desc) if modifiers.contains(Modifier::Image | Modifier::LinkURL) => {
                ImageUrl(desc, content.to_owned())
            }
            ImageUrl(desc, mut url) if modifiers.contains(Modifier::Image | Modifier::LinkURL) => {
                url.push_str(content);
                ImageUrl(desc, url)
            }
            ImageUrl(desc, url) if !modifiers.contains(Modifier::Image | Modifier::LinkURL) => {
                self.urls.push(TrackedUrl::image(desc, url));
                None
            }
            state => state,
        }
    }

    pub fn take_urls(&mut self) -> Vec<TrackedUrl> {
        let mut extras = Vec::new();
        swap(&mut self.urls, &mut extras);
        extras
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn test_link(description: &str, url: &str) -> Vec<Span> {
        vec![
            Span::new(
                "[".to_owned(),
                Modifier::Link | Modifier::LinkDescriptionWrapper,
            ),
            Span::new(
                description.to_owned(),
                Modifier::Link | Modifier::LinkDescription,
            ),
            Span::new(
                "]".to_owned(),
                Modifier::Link | Modifier::LinkDescriptionWrapper,
            ),
            Span::new("(".to_owned(), Modifier::Link | Modifier::LinkURLWrapper),
            Span::new(url.to_owned(), Modifier::Link | Modifier::LinkURL),
            Span::new(")".to_owned(), Modifier::Link | Modifier::LinkURLWrapper),
        ]
    }

    #[test]
    fn track_link() {
        let mut tracker = LinkTracker::default();
        for span in test_link("desc", "url") {
            tracker.track(&span);
        }
        let extras = tracker.take_urls();
        assert_eq!(extras[0], TrackedUrl::link("url".to_owned(), 1, 5, 0));
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
                    Modifier::Link | Modifier::LinkDescription | Modifier::Image,
                ),
                Span::new(
                    "image".to_owned(),
                    Modifier::Link | Modifier::LinkDescription | Modifier::Image,
                ),
                Span::new(
                    "](".to_owned(),
                    Modifier::Link | Modifier::LinkDescription | Modifier::Image,
                ),
                Span::new(
                    "image_url".to_owned(),
                    Modifier::LinkDescription | Modifier::Image | Modifier::LinkURL,
                ),
                Span::new(
                    ")".to_owned(),
                    Modifier::Link | Modifier::LinkDescription | Modifier::Image,
                ),
            ],
        );
        for span in spans {
            tracker.track(&span);
        }
        let extras = tracker.take_urls();
        assert_eq!(
            extras,
            vec![
                TrackedUrl::image("image".to_owned(), "image_url".to_owned()),
                TrackedUrl::link("url".to_owned(), 1, 20, 0),
            ]
        )
    }

    #[test]
    fn track_wrapped_link() {
        let mut tracker = LinkTracker::default();

        tracker.track(&Span::new(
            "[".to_owned(),
            Modifier::Link | Modifier::LinkDescriptionWrapper,
        ));
        tracker.track(&Span::new(
            "desc".to_owned(),
            Modifier::Link | Modifier::LinkDescription,
        ));
        tracker.carriage_return(None);

        tracker.track(&Span::new(
            "cont".to_owned(),
            Modifier::Link | Modifier::LinkDescription,
        ));
        tracker.track(&Span::new(
            "]".to_owned(),
            Modifier::Link | Modifier::LinkDescriptionWrapper,
        ));
        tracker.track(&Span::new(
            "(".to_owned(),
            Modifier::Link | Modifier::LinkURLWrapper,
        ));
        tracker.carriage_return(None);

        tracker.track(&Span::new(
            "url".to_owned(),
            Modifier::Link | Modifier::LinkURL,
        ));
        tracker.track(&Span::new(
            ")".to_owned(),
            Modifier::Link | Modifier::LinkURLWrapper,
        ));

        let extras = tracker.take_urls();
        assert_eq!(extras[0], TrackedUrl::link("url".to_owned(), 1, 4, 1),);
    }

    #[test]
    fn track_multiple_wraps_link() {
        let mut tracker = LinkTracker::default();

        tracker.track(&Span::new("nothing ".to_owned(), Modifier::default()));

        tracker.track(&Span::new(
            "[".to_owned(),
            Modifier::Link | Modifier::LinkDescriptionWrapper,
        ));
        tracker.track(&Span::new(
            "desc1".to_owned(),
            Modifier::Link | Modifier::LinkDescription,
        ));
        tracker.carriage_return(None);

        tracker.track(&Span::new(
            "desc2".to_owned(),
            Modifier::Link | Modifier::LinkDescription,
        ));
        tracker.carriage_return(None);

        tracker.track(&Span::new(
            "desc3".to_owned(),
            Modifier::Link | Modifier::LinkDescription,
        ));

        tracker.track(&Span::new(
            "]".to_owned(),
            Modifier::Link | Modifier::LinkDescriptionWrapper,
        ));
        tracker.track(&Span::new(
            "(".to_owned(),
            Modifier::Link | Modifier::LinkURLWrapper,
        ));
        tracker.carriage_return(None);

        tracker.track(&Span::new(
            "url-1/".to_owned(),
            Modifier::Link | Modifier::LinkURL,
        ));
        tracker.carriage_return(None);

        tracker.track(&Span::new(
            "url-2".to_owned(),
            Modifier::Link | Modifier::LinkURL,
        ));
        tracker.carriage_return(None);

        tracker.track(&Span::new(
            ")".to_owned(),
            Modifier::Link | Modifier::LinkURLWrapper,
        ));

        let extras = tracker.take_urls();
        // ```
        // nothing [desc1
        // desc2
        // desc3]
        // ```
        // So we want: start at "[" 9, end at "]" 5, two lines "up", and correct URL
        // reconstruction.
        assert_eq!(
            extras[0],
            TrackedUrl::link("url-1/url-2".to_owned(), 9, 5, 2),
        );
    }

    #[test]
    fn track_bare_link() {
        let mut tracker = LinkTracker::default();

        tracker.track(&Span::new(
            "http://bare".to_owned(),
            Modifier::BareLink | Modifier::LinkURL,
        ));
        tracker.carriage_return(None);
        let extras = tracker.take_urls();
        assert_eq!(
            extras[0],
            TrackedUrl::link("http://bare".to_owned(), 0, 11, 0),
        );
    }

    #[test]
    fn track_bare_link_eol() {
        // BareLink at end of line, plain text on next line: emitted for the correct line.
        let mut tracker = LinkTracker::default();

        tracker.track(&Span::new("see ".to_owned(), Modifier::default()));
        tracker.track(&Span::new(
            "http://bare".to_owned(),
            Modifier::BareLink | Modifier::LinkURL,
        ));
        tracker.carriage_return(Some(Modifier::default()));
        let extras = tracker.take_urls();
        assert_eq!(
            extras[0],
            TrackedUrl::link("http://bare".to_owned(), 4, 15, 0),
        );
    }

    #[test]
    fn track_bare_link_wrapped() {
        // BareLink URL split across two lines by wrapping.
        let mut tracker = LinkTracker::default();

        tracker.track(&Span::new(
            "https://ex.com/".to_owned(),
            Modifier::BareLink | Modifier::LinkURL,
        ));
        tracker.carriage_return(Some(Modifier::BareLink | Modifier::LinkURL));

        tracker.track(&Span::new(
            "rest".to_owned(),
            Modifier::BareLink | Modifier::LinkURL,
        ));
        tracker.carriage_return(None);

        let extras = tracker.take_urls();
        assert_eq!(
            extras[0],
            TrackedUrl::link("https://ex.com/rest".to_owned(), 0, 4, 1),
        );
    }
}
