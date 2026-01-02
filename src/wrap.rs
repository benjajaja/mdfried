use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
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
            let mut had_images = Vec::new();
            for mdspan in mdspans {
                if mdspan.extra.contains(MdModifier::LinkURL) {
                    if mdspan.extra.contains(MdModifier::Image) {
                        had_images.push(MarkdownImage {
                            destination: mdspan.content.clone(),
                            description: String::from("TODO:img_desc"),
                        });
                    } else {
                        let offset = line.width() as u16;
                        extras.push(LineExtra::Link(
                            mdspan.content.clone(),
                            offset,
                            offset + mdspan.content.width() as u16,
                        ));
                    }
                }

                if !mdspan.extra.contains(MdModifier::Image) {
                    let span = span_from_mdspan(mdspan);
                    line.spans.push(span);
                }
            }

            if !line.spans.is_empty() {
                events.push(Event::Parsed(
                    document_id,
                    WidgetSource {
                        id: post_incr_source_id(source_id),
                        height: 1,
                        data: WidgetSourceData::Line(line, extras),
                    },
                ));
            }
            for image in had_images {
                events.push(Event::ParsedImage(
                    document_id,
                    post_incr_source_id(source_id),
                    image,
                ));
            }
            events
        })
        .collect()
}

pub const LINK_DESC_OPEN: &str = "▐";
pub const LINK_DESC_CLOSE: &str = "▌";
pub const LINK_URL_OPEN: &str = "◖";
pub const LINK_URL_CLOSE: &str = "◗";

pub const COLOR_LINK_BG: Color = Color::Indexed(237);
pub const COLOR_LINK_FG: Color = Color::Indexed(4);

fn span_from_mdspan(mdspan: MdSpan) -> Span<'static> {
    let mut style = Style::default();
    if mdspan.extra.contains(MdModifier::Emphasis) {
        style = style.add_modifier(Modifier::ITALIC).fg(Color::Indexed(220));
    }
    if mdspan.extra.contains(MdModifier::StrongEmphasis) {
        style = style.add_modifier(Modifier::BOLD).fg(Color::Indexed(220));
    }
    if mdspan.extra.contains(MdModifier::Code) {
        style = style.fg(Color::Indexed(203)).bg(Color::Indexed(236));
    }

    if mdspan.extra.contains(MdModifier::LinkURLWrapper) {
        let bracket = if mdspan.content == "(" {
            LINK_URL_OPEN
        } else {
            LINK_URL_CLOSE
        };
        return Span::styled(bracket, style.fg(COLOR_LINK_BG));
    }
    if mdspan.extra.contains(MdModifier::LinkURL) {
        style = style.fg(COLOR_LINK_FG).bg(COLOR_LINK_BG).underlined();
    }

    if mdspan.extra.contains(MdModifier::LinkDescriptionWrapper) {
        let bracket = if mdspan.content == "[" {
            LINK_DESC_OPEN
        } else {
            LINK_DESC_CLOSE
        };
        return Span::styled(bracket, style.fg(COLOR_LINK_BG));
    }
    if mdspan.extra.contains(MdModifier::LinkDescription) {
        style = style.fg(COLOR_LINK_FG).bg(COLOR_LINK_BG);
    }

    Span::styled(mdspan.content, style)
}

pub fn wrap_md_spans_lines(width: u16, mdspans: Vec<MdSpan>) -> Vec<Vec<MdSpan>> {
    let mut lines: Vec<Vec<MdSpan>> = Vec::new();

    let mut line: Vec<MdSpan> = Vec::new();

    for mdspan in mdspans {
        if mdspan.extra.contains(MdModifier::NewLine) {
            if let Some(last) = line.last_mut() {
                last.content.truncate(last.content.trim_end().len());
            }
            lines.push(std::mem::take(&mut line));
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
                    let mut extra = mdspan.extra;
                    if !copied_newline {
                        copied_newline = true;
                    } else {
                        extra.remove(MdModifier::NewLine); // We don't want to carry over the newlines.
                    }
                    line.push(MdSpan::new(part.to_string(), extra));
                    line_width += part_width;
                }
            } else {
                line.push(mdspan);
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
            MdSpan::new("two".into(), MdModifier::NewLine),
        ];
        let lines = wrap_md_spans_lines(10, mdspans);
        assert_eq!(
            lines,
            vec![
                vec![MdSpan::from("one")],
                vec![MdSpan::new("two".into(), MdModifier::NewLine),]
            ],
        );
    }

    #[test]
    fn newline_wordbreak() {
        let mdspans = vec![
            MdSpan::from("one "),
            MdSpan::new("twoooo".into(), MdModifier::NewLine),
        ];
        let lines = wrap_md_spans_lines(4, mdspans);
        assert_eq!(
            lines,
            vec![
                vec![MdSpan::from("one")],
                vec![MdSpan::new("twoo".into(), MdModifier::NewLine)],
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
                vec![MdSpan::new("twoo".into(), MdModifier::NewLine)],
                vec![MdSpan::from("oo")],
            ],
        );
    }
}
