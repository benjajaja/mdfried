use textwrap::{Options, wrap};
use unicode_width::UnicodeWidthStr;

use crate::{
    MarkdownImage,
    worker::pipeline::markdown::{MdModifier, MdNode},
};

/// A wrapped line of markdown content.
pub struct WrappedLine {
    /// Prefix to prepend (source prefix or continuation spaces).
    pub prefix: String,
    /// The content spans.
    pub spans: Vec<MdNode>,
    /// Any images found on this line.
    pub images: Vec<MarkdownImage>,
}

pub fn wrap_md_spans(width: u16, mdspans: Vec<MdNode>, source_prefix: &str) -> Vec<WrappedLine> {
    let prefix_width = source_prefix.width() as u16;
    let continuation_prefix = " ".repeat(source_prefix.width());
    let available_width = width.saturating_sub(prefix_width).max(1);

    wrap_md_spans_lines(available_width, mdspans)
        .into_iter()
        .filter(|line| !line.is_empty())
        .enumerate()
        .map(|(line_idx, mdspans)| {
            // Check if this line came from a hard line break in source (NewLine marker)
            let is_source_newline = mdspans
                .first()
                .is_some_and(|s| s.extra.contains(MdModifier::NewLine));

            let prefix = if line_idx == 0 || is_source_newline {
                // First line or hard line break from source - use source prefix
                source_prefix.to_owned()
            } else {
                // Soft-wrapped continuation line - use spaces
                continuation_prefix.clone()
            };

            // Extract images from spans
            let images: Vec<MarkdownImage> = mdspans
                .iter()
                .filter(|s| {
                    s.extra.contains(MdModifier::LinkURL) && s.extra.contains(MdModifier::Image)
                })
                .map(|s| MarkdownImage {
                    destination: s.content.clone(),
                    description: String::from("TODO:img_desc"),
                })
                .collect();

            // Filter out image URL spans from content
            let spans: Vec<MdNode> = mdspans
                .into_iter()
                .filter(|s| !s.extra.contains(MdModifier::Image))
                .collect();

            WrappedLine {
                prefix,
                spans,
                images,
            }
        })
        .collect()
}

fn wrap_md_spans_lines(width: u16, mdspans: Vec<MdNode>) -> Vec<Vec<MdNode>> {
    let mut lines: Vec<Vec<MdNode>> = Vec::new();
    let mut line: Vec<MdNode> = Vec::new();

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
            let starting_new_line = !line.is_empty();
            if starting_new_line {
                lines.push(std::mem::take(&mut line));
                line_width = 0;
            }
            if span_width > width {
                let options = Options::new(width as usize)
                    .break_words(true)
                    .word_splitter(textwrap::word_splitters::WordSplitter::NoHyphenation);
                let parts: Vec<_> = wrap(&mdspan.content, options).into_iter().collect();
                let num_parts = parts.len();
                let ends_with_space = mdspan.content.ends_with(' ');
                let mut copied_newline = false;
                for (i, part) in parts.into_iter().enumerate() {
                    let is_last = i == num_parts - 1;
                    let is_first = i == 0;
                    // textwrap strips trailing spaces - preserve them on the last part
                    // if the original content ended with space
                    let mut part_content = if is_last && ends_with_space {
                        format!("{} ", part)
                    } else {
                        part.to_string()
                    };
                    // Trim leading whitespace on soft-wrapped continuation lines
                    if is_first && starting_new_line && !mdspan.extra.contains(MdModifier::NewLine)
                    {
                        part_content = part_content.trim_start().to_owned();
                    }
                    let part_width = part_content.width() as u16;
                    if line_width + part_width > width {
                        lines.push(std::mem::take(&mut line));
                        line_width = 0;
                    }
                    let mut extra = mdspan.extra;
                    if !copied_newline {
                        copied_newline = true;
                    } else {
                        extra.remove(MdModifier::NewLine);
                    }
                    line.push(MdNode::new(part_content, extra));
                    line_width += part_width;
                }
            } else {
                // Trim leading whitespace on soft-wrapped continuation lines
                let mut mdspan = mdspan;
                if starting_new_line && !mdspan.extra.contains(MdModifier::NewLine) {
                    mdspan.content = mdspan.content.trim_start().to_owned();
                }
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

    use super::wrap_md_spans_lines;
    use crate::worker::pipeline::markdown::{MdModifier, MdNode};

    #[test]
    fn simple_wrap() {
        let mdspans = vec![MdNode::from("one two")];
        let lines = wrap_md_spans_lines(4, mdspans);
        assert_eq!(
            lines,
            vec![vec![MdNode::from("one")], vec![MdNode::from("two")]]
        );
    }

    #[test]
    fn no_wrap() {
        let mdspans = vec![MdNode::from("one two")];
        let lines = wrap_md_spans_lines(10, mdspans);
        assert_eq!(lines, vec![vec![MdNode::from("one two")]]);
    }

    #[test]
    fn word_break() {
        let mdspans = vec![MdNode::from("one two")];
        let lines = wrap_md_spans_lines(2, mdspans);
        assert_eq!(
            lines,
            vec![
                vec![MdNode::from("on")],
                vec![MdNode::from("e")],
                vec![MdNode::from("tw")],
                vec![MdNode::from("o")]
            ]
        );
    }

    #[test]
    fn trailing_word_break() {
        let mdspans = vec![MdNode::from("one twoo")];
        let lines = wrap_md_spans_lines(4, mdspans);
        assert_eq!(
            lines,
            vec![vec![MdNode::from("one")], vec![MdNode::from("twoo")],]
        );
    }

    #[test]
    fn multiline_break() {
        let mdspans = vec![MdNode::from("onetwo")];
        let lines = wrap_md_spans_lines(2, mdspans);
        assert_eq!(
            lines,
            vec![
                vec![MdNode::from("on")],
                vec![MdNode::from("et")],
                vec![MdNode::from("wo")],
            ]
        );
    }

    #[test]
    fn newline() {
        let mdspans = vec![
            MdNode::from("one "),
            MdNode::new("two".into(), MdModifier::NewLine),
        ];
        let lines = wrap_md_spans_lines(10, mdspans);
        assert_eq!(
            lines,
            vec![
                vec![MdNode::from("one")],
                vec![MdNode::new("two".into(), MdModifier::NewLine),]
            ],
        );
    }

    #[test]
    fn newline_wordbreak() {
        let mdspans = vec![
            MdNode::from("one "),
            MdNode::new("twoooo".into(), MdModifier::NewLine),
        ];
        let lines = wrap_md_spans_lines(4, mdspans);
        assert_eq!(
            lines,
            vec![
                vec![MdNode::from("one")],
                vec![MdNode::new("twoo".into(), MdModifier::NewLine)],
                vec![MdNode::from("oo")],
            ],
        );
    }

    #[test]
    fn trailing_space_before_styled() {
        // When a span ending with space is broken by textwrap, the trailing space
        // should be preserved so styled spans following it have proper spacing
        let mdspans = vec![
            MdNode::from("you can "),
            MdNode::new("put".into(), MdModifier::Emphasis),
        ];
        // Width 10: "you can " (8) fits, then "put" (3) would overflow
        // so "put" goes on next line. But we want the trailing space preserved
        // in case they end up on the same line at different widths
        let lines = wrap_md_spans_lines(10, mdspans);
        // Both should fit on same line with trailing space preserved
        let content: String = lines
            .iter()
            .flat_map(|l| l.iter().map(|s| s.content.as_str()))
            .collect();
        assert_eq!(content, "you can put"); // space between "can" and "put"
    }

    #[test]
    fn trailing_space_before_styled_wrapped() {
        // When a long span ending with space is broken, the last part
        // should preserve the trailing space
        let mdspans = vec![
            MdNode::from("hello world "),
            MdNode::new("styled".into(), MdModifier::Emphasis),
        ];
        let lines = wrap_md_spans_lines(8, mdspans);
        // "hello world " broken into ["hello", "world "]
        // Check that "world" has trailing space for styled text
        let last_unstyled = lines
            .iter()
            .flat_map(|l| l.iter())
            .rfind(|s| !s.extra.contains(MdModifier::Emphasis))
            .expect("should have unstyled span");
        assert!(
            last_unstyled.content.ends_with(' '),
            "Last unstyled span should preserve trailing space: {:?}",
            last_unstyled.content
        );
    }

    #[test]
    #[ignore]
    fn link() {
        let mut mdspans = vec![MdNode::from("one ")];
        mdspans.extend(MdNode::link("here", "http://googoo"));
        mdspans.push(MdNode::from("two"));
        let lines = wrap_md_spans_lines(15, mdspans);
        assert_eq!(
            lines,
            vec![
                vec![MdNode::from("one")],
                MdNode::link("here", "http://googoo"),
                vec![MdNode::from("two")],
            ],
        );
    }

    #[test]
    #[ignore]
    fn link_break() {
        let mut mdspans = vec![MdNode::from("one ")];
        mdspans.extend(MdNode::link("here", "http://googoo"));
        mdspans.push(MdNode::from("two"));
        let lines = wrap_md_spans_lines(10, mdspans);
        assert_eq!(
            lines,
            vec![
                vec![MdNode::from("one")],
                vec![MdNode::new("twoo".into(), MdModifier::NewLine)],
                vec![MdNode::from("oo")],
            ],
        );
    }
}
