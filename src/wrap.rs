use ratatui::text::{Line, Span};
use textwrap::{Options, wrap};
use unicode_width::UnicodeWidthStr;

use crate::{
    Event, MarkdownImage,
    markdown::{MdModifier, MdSpan},
    model::DocumentId,
    widget_sources::{LineExtra, WidgetSource, WidgetSourceData},
    worker::post_incr_source_id,
};

pub fn wrap_md_spans(
    document_id: DocumentId,
    source_id: &mut Option<usize>,
    width: u16,
    mdspans: Vec<MdSpan>,
) -> Vec<Event> {
    wrap_md_spans_lines(width, mdspans)
        .into_iter()
        .flat_map(|mdspans| {
            let mut events = Vec::new();
            let mut line = Line::default();
            let mut extras = Vec::new();
            for mdspan in mdspans {
                if mdspan.extra.contains(MdModifier::LinkURL) {
                    if mdspan.extra.contains(MdModifier::Image) {
                        events.push(Event::ParsedImage(
                            document_id,
                            post_incr_source_id(source_id),
                            MarkdownImage {
                                destination: mdspan.content.clone(),
                                description: String::from("TODO:img_desc"),
                            },
                        ));
                    } else {
                        extras.push(LineExtra::Link(mdspan.content.clone(), 0, 1));
                    }
                }

                let span = Span::styled(mdspan.content, mdspan.style);
                line.spans.push(span);
            }
            events.push(Event::Parsed(
                document_id,
                WidgetSource {
                    id: post_incr_source_id(source_id),
                    height: 1,
                    data: WidgetSourceData::Line(line, extras),
                },
            ));
            events
        })
        .collect()
}

pub fn wrap_md_spans_lines(width: u16, mdspans: Vec<MdSpan>) -> Vec<Vec<MdSpan>> {
    let mut lines: Vec<Vec<MdSpan>> = Vec::new();

    let mut line: Vec<MdSpan> = Vec::new();

    for mdspan in mdspans {
        if mdspan.extra.contains(MdModifier::NewLine) {
            if let Some(last) = line.last_mut() {
                last.content.truncate(last.content.trim_end().len());
                lines.push(std::mem::take(&mut line));
            }
        }

        let span_width = mdspan.content.width() as u16;
        let mut line_width = line.iter().map(UnicodeWidthStr::width).sum::<usize>() as u16;
        let would_overflow = line_width + span_width > width;
        if would_overflow {
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
                line_width = 0;
            }
            if span_width > width {
                let options = Options::new(width as usize)
                    .break_words(true) // break long words/URLs if they exceed width
                    .word_splitter(textwrap::word_splitters::WordSplitter::NoHyphenation); // no hyphens when breaking
                let parts = wrap(&mdspan.content, options);
                let mut copied_newline = false;
                for part in parts {
                    let part_width = part.width() as u16;
                    if line_width + part_width > width {
                        lines.push(std::mem::take(&mut line));
                        line_width = 0;
                    }
                    let mut extra = mdspan.extra.clone();
                    if !copied_newline {
                        copied_newline = true;
                    } else {
                        extra.remove(MdModifier::NewLine); // We don't want to carry over the newlines.
                    }
                    line.push(MdSpan::new(part.to_string(), mdspan.style.clone(), extra));
                    line_width += part_width;
                }
            }
        } else {
            line.push(mdspan);
        }
    }

    if !line.is_empty() {
        lines.push(line);
    }

    lines
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use ratatui::style::Style;

    use crate::{
        markdown::{MdModifier, MdSpan},
        wrap::wrap_md_spans_lines,
    };

    #[test]
    fn simple_wrap() {
        let mdspans = vec![MdSpan::from("one two")];
        let lines = wrap_md_spans_lines(4, mdspans);
        assert_eq!(
            lines,
            vec![vec![MdSpan::from("one")], vec![MdSpan::from("two")]]
        );
    }

    #[test]
    fn no_wrap() {
        let mdspans = vec![MdSpan::from("one two")];
        let lines = wrap_md_spans_lines(10, mdspans);
        assert_eq!(lines, vec![vec![MdSpan::from("one two")]]);
    }

    #[test]
    fn word_break() {
        let mdspans = vec![MdSpan::from("one two")];
        let lines = wrap_md_spans_lines(2, mdspans);
        assert_eq!(
            lines,
            vec![
                vec![MdSpan::from("on")],
                vec![MdSpan::from("e")],
                vec![MdSpan::from("tw")],
                vec![MdSpan::from("o")]
            ]
        );
    }

    #[test]
    fn trailing_word_break() {
        let mdspans = vec![MdSpan::from("one twoo")];
        let lines = wrap_md_spans_lines(4, mdspans);
        assert_eq!(
            lines,
            vec![vec![MdSpan::from("one")], vec![MdSpan::from("twoo")],]
        );
    }

    #[test]
    fn multiline_break() {
        let mdspans = vec![MdSpan::from("onetwo")];
        let lines = wrap_md_spans_lines(2, mdspans);
        assert_eq!(
            lines,
            vec![
                vec![MdSpan::from("on")],
                vec![MdSpan::from("et")],
                vec![MdSpan::from("wo")],
            ]
        );
    }

    #[test]
    fn newline() {
        let mdspans = vec![
            MdSpan::from("one "),
            MdSpan::new("two".into(), Style::default(), MdModifier::NewLine),
        ];
        let lines = wrap_md_spans_lines(10, mdspans);
        assert_eq!(
            lines,
            vec![
                vec![MdSpan::from("one")],
                vec![MdSpan::new(
                    "two".into(),
                    Style::default(),
                    MdModifier::NewLine
                ),]
            ],
        );
    }

    #[test]
    fn newline_wordbreak() {
        let mdspans = vec![
            MdSpan::from("one "),
            MdSpan::new("twoooo".into(), Style::default(), MdModifier::NewLine),
        ];
        let lines = wrap_md_spans_lines(4, mdspans);
        assert_eq!(
            lines,
            vec![
                vec![MdSpan::from("one")],
                vec![MdSpan::new(
                    "twoo".into(),
                    Style::default(),
                    MdModifier::NewLine
                )],
                vec![MdSpan::from("oo")],
            ],
        );
    }

    #[test]
    #[ignore]
    fn link() {
        let mut mdspans = vec![MdSpan::from("one ")];
        mdspans.extend(MdSpan::link("here", "http://googoo"));
        mdspans.push(MdSpan::from("two"));
        let lines = wrap_md_spans_lines(15, mdspans);
        assert_eq!(
            lines,
            vec![
                vec![MdSpan::from("one")],
                MdSpan::link("here", "http://googoo"),
                vec![MdSpan::from("two")],
            ],
        );
    }

    #[test]
    #[ignore]
    fn link_break() {
        let mut mdspans = vec![MdSpan::from("one ")];
        mdspans.extend(MdSpan::link("here", "http://googoo"));
        mdspans.push(MdSpan::from("two"));
        let lines = wrap_md_spans_lines(10, mdspans);
        assert_eq!(
            lines,
            vec![
                vec![MdSpan::from("one")],
                vec![MdSpan::new(
                    "twoo".into(),
                    Style::default(),
                    MdModifier::NewLine
                )],
                vec![MdSpan::from("oo")],
            ],
        );
    }
}
