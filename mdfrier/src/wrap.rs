use textwrap::{Options, wrap};
use unicode_width::UnicodeWidthStr;

use crate::markdown::{Modifier, Span};

/// Trim leading whitespace in place.
#[inline]
fn trim_start_inplace(s: &mut String) {
    let trimmed_len = s.trim_start().len();
    if trimmed_len < s.len() {
        let start = s.len() - trimmed_len;
        s.drain(..start);
    }
}

/// Image reference extracted from markdown.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ImageRef {
    pub url: String,
    pub description: String,
}

/// A wrapped line of markdown content.
pub(crate) struct WrappedLine {
    /// Whether this is a first line (not a soft-wrapped continuation).
    pub is_first: bool,
    /// The content spans.
    pub spans: Vec<Span>,
    /// Any images found on this line.
    pub images: Vec<ImageRef>,
}

pub(crate) fn wrap_md_spans(
    width: u16,
    mdspans: Vec<Span>,
    prefix_width: usize,
) -> Vec<WrappedLine> {
    let available_width = width.saturating_sub(prefix_width as u16).max(1);

    wrap_md_spans_lines(available_width, mdspans)
        .into_iter()
        .filter(|line| !line.is_empty())
        .enumerate()
        .map(|(line_idx, mdspans)| {
            // Extract images from spans
            let mut images: Vec<ImageRef> = Vec::new();
            for (i, s) in mdspans.iter().enumerate() {
                if s.modifiers.contains(Modifier::LinkURL)
                    && s.modifiers.contains(Modifier::Image)
                    && let Some(source_content) = &s.get_source_content()
                {
                    // Track back to get description if any.
                    // TODO: something's wrong about this!
                    let mut description = None;
                    for j in 0..3 {
                        if i > j
                            && let Some(desc_span) = mdspans.get(i - j)
                            && desc_span.modifiers.contains(Modifier::LinkDescription)
                            && desc_span.modifiers.contains(Modifier::Image)
                        {
                            description = Some(desc_span.content.clone());
                        }
                    }
                    #[cfg(feature = "ratatui")]
                    if description.is_none() {
                        log::warn!("image description node not found (really absent?)");
                    }
                    images.push(ImageRef {
                        url: source_content.as_ref().to_owned(),
                        description: description.unwrap_or_default(),
                    });
                }
            }

            WrappedLine {
                is_first: line_idx == 0,
                spans: mdspans,
                images,
            }
        })
        .collect()
}

pub(crate) fn wrap_md_spans_lines(width: u16, mdspans: Vec<Span>) -> Vec<Vec<Span>> {
    let mut lines: Vec<Vec<Span>> = Vec::new();
    let mut line: Vec<Span> = Vec::new();
    let mut after_newline = false;

    for mdspan in mdspans {
        if mdspan.modifiers.contains(Modifier::NewLine) {
            if let Some(last) = line.last_mut() {
                last.content.truncate(last.content.trim_end().len());
            }
            lines.push(std::mem::take(&mut line));
            after_newline = true;
        }

        // Strip leading whitespace from content after a hard line break
        let mut mdspan = mdspan;
        if after_newline && !mdspan.content.is_empty() {
            trim_start_inplace(&mut mdspan.content);
            after_newline = false;
        }

        let span_width = mdspan.content.width() as u16;
        let mut line_width = line.iter().map(UnicodeWidthStr::width).sum::<usize>() as u16;
        let would_overflow = line_width + span_width > width;
        if would_overflow {
            let starting_new_line = !line.is_empty();
            if starting_new_line {
                // Keep opening "(" with the URL, not on previous line
                let move_paren = line.last().is_some_and(|last| {
                    last.modifiers.contains(Modifier::LinkURLWrapper) && last.content == "("
                });
                let moved_paren = if move_paren { line.pop() } else { None };

                lines.push(std::mem::take(&mut line));
                line_width = 0;

                if let Some(paren) = moved_paren {
                    line.push(paren);
                    line_width = 1;
                }
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
                    let mut part_content: String = if is_last && ends_with_space {
                        let mut s = String::with_capacity(part.len() + 1);
                        s.push_str(&part);
                        s.push(' ');
                        s
                    } else {
                        part.into_owned()
                    };
                    if is_first
                        && starting_new_line
                        && !mdspan.modifiers.contains(Modifier::NewLine)
                    {
                        trim_start_inplace(&mut part_content);
                    }
                    let part_width = part_content.width() as u16;
                    if line_width + part_width > width {
                        lines.push(std::mem::take(&mut line));
                        line_width = 0;
                    }
                    let mut modifiers = mdspan.modifiers;
                    if !copied_newline {
                        copied_newline = true;
                    } else {
                        modifiers.remove(Modifier::NewLine);
                    }
                    // Preserve source_content when splitting spans (for wrapped URLs)
                    if modifiers.contains(Modifier::LinkURL) {
                        line.push(Span::link(
                            part_content,
                            modifiers,
                            mdspan.get_source_content(),
                        ));
                    } else {
                        line.push(Span::new(part_content, modifiers));
                    }
                    line_width += part_width;
                }
            } else {
                let mut mdspan = mdspan;
                if starting_new_line && !mdspan.modifiers.contains(Modifier::NewLine) {
                    trim_start_inplace(&mut mdspan.content);
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
    use crate::markdown::{Modifier, Span};

    #[test]
    fn simple_wrap() {
        let mdspans = vec![Span::from("one two")];
        let lines = wrap_md_spans_lines(4, mdspans);
        assert_eq!(
            lines,
            vec![vec![Span::from("one")], vec![Span::from("two")]]
        );
    }

    #[test]
    fn no_wrap() {
        let mdspans = vec![Span::from("one two")];
        let lines = wrap_md_spans_lines(10, mdspans);
        assert_eq!(lines, vec![vec![Span::from("one two")]]);
    }

    #[test]
    fn word_break() {
        let mdspans = vec![Span::from("one two")];
        let lines = wrap_md_spans_lines(2, mdspans);
        assert_eq!(
            lines,
            vec![
                vec![Span::from("on")],
                vec![Span::from("e")],
                vec![Span::from("tw")],
                vec![Span::from("o")]
            ]
        );
    }

    #[test]
    fn newline() {
        let mdspans = vec![
            Span::from("one "),
            Span::new("two".into(), Modifier::NewLine),
        ];
        let lines = wrap_md_spans_lines(10, mdspans);
        assert_eq!(
            lines,
            vec![
                vec![Span::from("one")],
                vec![Span::new("two".into(), Modifier::NewLine),]
            ],
        );
    }

    #[test]
    fn link_wrapping() {
        let mdspans = vec![
            Span::new("[".into(), Modifier::LinkDescriptionWrapper),
            Span::new("link".into(), Modifier::LinkDescription),
            Span::new("]".into(), Modifier::LinkDescriptionWrapper),
            Span::new("(".into(), Modifier::LinkURLWrapper),
            Span::link("https://example.com".into(), Modifier::LinkURL, None),
            Span::new(")".into(), Modifier::LinkURLWrapper),
        ];
        let lines = wrap_md_spans_lines(25, mdspans);
        assert_eq!(
            lines
                .iter()
                .map(|spans| spans
                    .iter()
                    .map(Span::to_string)
                    .collect::<Vec<String>>()
                    .join(""))
                .collect::<Vec<String>>(),
            vec!["[link]", "(https://example.com)",],
        );
    }
}
