//! Track links and track back URLs.

use std::mem::swap;

use crate::{Modifier, Span};
use unicode_width::UnicodeWidthStr as _;

#[derive(Default, Debug)]
/// Iterator over [`mdfrier::Span`]s and extract links as [`LineExtra::Link`]`(url, start, end)`,
/// where "start" and "end" are the respective positions in the [`mdfrier::Line`].
pub struct LinkTracker {
    offset: u16,
    urls: Vec<TrackedUrl>,
    state: [LinkState; 2],
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
    ImageDesc(String),
    ImageUrl(String, String),
}

#[derive(Debug, PartialEq)]
pub enum TrackedUrl {
    Link {
        start: u16,
        lines: usize,
        end: u16,
        url: String,
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
        }
    }
    pub fn image<S: Into<String>>(desc: S, url: S) -> Self {
        Self::Image {
            desc: desc.into(),
            url: url.into(),
        }
    }
}

impl LinkTracker {
    pub fn hide_urls(mut self, hide_urls: bool) -> LinkTracker {
        self.hide_urls = hide_urls;
        self
    }

    pub fn carriage_return(&mut self) {
        self.offset = 0;
        // Only increase line count if we haven't reached "end" yet.
        if let LinkState::LinkDesc(_start, lines) = &mut self.state[0] {
            *lines += 1;
        }
    }
    pub fn track(&mut self, node: &Span) {
        use LinkState::*;

        let Span { modifiers, content } = &node;
        let span_width = content.width() as u16;

        self.state[0] = match std::mem::take(&mut self.state[0]) {
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
            // Bare links
            None if modifiers.contains(Modifier::Link | Modifier::BareLink | Modifier::LinkURL) => {
                // This assumes that bare links cannot be line broken.
                self.urls.push(TrackedUrl::link(
                    content.clone(),
                    self.offset,
                    self.offset + span_width,
                    0,
                ));
                None
            }
            // Images
            None if modifiers.contains(Modifier::Image | Modifier::LinkDescription) => {
                ImageDesc(String::from(content))
            }
            ImageDesc(mut desc)
                if modifiers.contains(Modifier::Image | Modifier::LinkDescription) =>
            {
                desc.push_str(content);
                ImageDesc(desc)
            }
            // ImageDesc(desc)
            // if modifiers.contains(Modifier::Image | Modifier::LinkDescriptionWrapper) =>
            // {
            // panic!("LinkDescriptionWrapper");
            // }
            // ImageDesc(desc) if modifiers.contains(Modifier::Image | Modifier::LinkURLWrapper) => {
            // panic!("LinkURLWrapper");
            // }
            ImageDesc(desc) if modifiers.contains(Modifier::Image | Modifier::LinkURL) => {
                ImageUrl(desc, content.clone())
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
        };

        // Nested images
        // TODO: shouldn't this be the same logic as above? Can we also not copypasta it?
        if matches!(self.state[0], LinkDesc(..)) {
            self.state[1] = match std::mem::take(&mut self.state[1]) {
                // Images
                None if modifiers.contains(Modifier::Image) => ImageDesc(String::new()),
                ImageDesc(mut desc)
                    if modifiers.contains(Modifier::Image | Modifier::LinkDescription)
                        && !modifiers.contains(Modifier::LinkURL) =>
                {
                    if content != "](" {
                        desc.push_str(content);
                    }
                    ImageDesc(desc)
                }
                ImageDesc(desc) if modifiers.contains(Modifier::Image | Modifier::LinkURL) => {
                    ImageUrl(desc, content.clone())
                }
                ImageUrl(desc, mut url)
                    if modifiers.contains(Modifier::Image | Modifier::LinkURL) =>
                {
                    url.push_str(content);
                    ImageUrl(desc, url)
                }
                ImageUrl(desc, url) if !modifiers.contains(Modifier::Image | Modifier::LinkURL) => {
                    self.urls.push(TrackedUrl::image(desc, url));
                    None
                }
                state => state,
            };
        }

        if self.hide_urls {
            if !node.modifiers.is_link_url() {
                self.offset += span_width;
            }
        } else {
            self.offset += span_width;
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
            extras[0],
            TrackedUrl::image("image".to_owned(), "image_url".to_owned())
        );
        assert_eq!(extras[1], TrackedUrl::link("url".to_owned(), 1, 20, 0));
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
        tracker.carriage_return();

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
        tracker.carriage_return();

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
        tracker.carriage_return();

        tracker.track(&Span::new(
            "desc2".to_owned(),
            Modifier::Link | Modifier::LinkDescription,
        ));
        tracker.carriage_return();

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
        tracker.carriage_return();

        tracker.track(&Span::new(
            "url-1/".to_owned(),
            Modifier::Link | Modifier::LinkURL,
        ));
        tracker.carriage_return();

        tracker.track(&Span::new(
            "url-2".to_owned(),
            Modifier::Link | Modifier::LinkURL,
        ));
        tracker.carriage_return();

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
            Modifier::Link | Modifier::LinkURL | Modifier::BareLink,
        ));
        let extras = tracker.take_urls();
        // ```
        // http://bare
        // ```
        assert_eq!(
            extras[0],
            TrackedUrl::link("http://bare".to_owned(), 0, 11, 0),
        );
    }
}
