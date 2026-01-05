use textwrap::{Options, wrap};
use unicode_width::UnicodeWidthStr;

use crate::markdown::{MdModifier, MdNode};

/// Image reference extracted from markdown.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageRef {
    pub url: String,
    pub description: String,
}

/// A wrapped line of markdown content.
pub struct WrappedLine {
    /// Whether this is a first line (not a soft-wrapped continuation).
    pub is_first: bool,
    /// The content spans.
    pub spans: Vec<MdNode>,
    /// Any images found on this line.
    pub images: Vec<ImageRef>,
}

pub fn wrap_md_spans(width: u16, mdspans: Vec<MdNode>, prefix_width: usize) -> Vec<WrappedLine> {
    let available_width = width.saturating_sub(prefix_width as u16).max(1);

    wrap_md_spans_lines(available_width, mdspans)
        .into_iter()
        .filter(|line| !line.is_empty())
        .enumerate()
        .map(|(line_idx, mdspans)| {
            let is_source_newline = mdspans
                .first()
                .is_some_and(|s| s.extra.contains(MdModifier::NewLine));

            let is_first = line_idx == 0 || is_source_newline;

            // Extract images from spans
            let images: Vec<ImageRef> = mdspans
                .iter()
                .filter(|s| {
                    s.extra.contains(MdModifier::LinkURL) && s.extra.contains(MdModifier::Image)
                })
                .map(|s| ImageRef {
                    url: s.content.clone(),
                    description: String::from("TODO:img_desc"),
                })
                .collect();

            // Filter out image URL spans from content
            let spans: Vec<MdNode> = mdspans
                .into_iter()
                .filter(|s| !s.extra.contains(MdModifier::Image))
                .collect();

            WrappedLine {
                is_first,
                spans,
                images,
            }
        })
        .collect()
}

fn wrap_md_spans_lines(width: u16, mdspans: Vec<MdNode>) -> Vec<Vec<MdNode>> {
    let mut lines: Vec<Vec<MdNode>> = Vec::new();
    let mut line: Vec<MdNode> = Vec::new();
    let mut after_newline = false;

    for mdspan in mdspans {
        if mdspan.extra.contains(MdModifier::NewLine) {
            if let Some(last) = line.last_mut() {
                last.content.truncate(last.content.trim_end().len());
            }
            lines.push(std::mem::take(&mut line));
            after_newline = true;
        }

        // Strip leading whitespace from content after a hard line break
        let mut mdspan = mdspan;
        if after_newline && !mdspan.content.is_empty() {
            mdspan.content = mdspan.content.trim_start().to_owned();
            after_newline = false;
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
                    let mut part_content = if is_last && ends_with_space {
                        format!("{} ", part)
                    } else {
                        part.to_string()
                    };
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
    use crate::markdown::{MdModifier, MdNode};

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
}
