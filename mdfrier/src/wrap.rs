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

pub fn wrap_md_spans(
    width: u16,
    mdspans: Vec<Span>,
    prefix_width: usize,
    hide_urls: bool,
) -> Vec<WrappedLine> {
    let available_width = width.saturating_sub(prefix_width as u16).max(1);

    wrap_md_spans_lines(available_width, mdspans, hide_urls)
        .into_iter()
        .filter(|line| !line.is_empty())
        .enumerate()
        .map(|(line_idx, mdspans)| {
            // Extract images from spans
            let mut images: Vec<ImageRef> = Vec::new();
            for (i, s) in mdspans.iter().enumerate() {
                if s.modifiers.contains(Modifier::LinkURL) && s.modifiers.contains(Modifier::Image)
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
                        url: s.content.clone(),
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

pub fn wrap_md_spans_lines(width: u16, mdspans: Vec<Span>, hide_urls: bool) -> Vec<Vec<Span>> {
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

        let span_width = if hide_urls && mdspan.modifiers.is_link_url() {
            // If hide_urls, the LinkURL is kept for building LinkExtra::Link after wrapping, but
            // will be filtered out later. Therefore, ignore for width counts.
            0
        } else {
            mdspan.content.width() as u16
        };
        let mut line_width = line
            .iter()
            .filter(|span| !hide_urls || !span.modifiers.is_link_url())
            .map(UnicodeWidthStr::width)
            .sum::<usize>() as u16;
        let would_overflow = line_width + span_width > width;
        if would_overflow {
            // Noe: this *was* something weird about moving links that would exceed `width`
            // together with their surrounding parens.
            let starting_new_line = !line.is_empty();

            // Split once with "remaining width" (`width - line_width`), to append the first part
            // onto the current line (if any, otherwise would just make a new line).
            let options = Options::new((width - line_width) as usize)
                .break_words(true)
                .word_splitter(textwrap::word_splitters::WordSplitter::NoHyphenation);
            let parts: Vec<_> = wrap(&mdspan.content, options).into_iter().collect();
            let Some(first_part) = parts.first() else {
                continue;
            };
            let first_content = first_part.as_ref();
            line.push(Span::new(first_content.to_owned(), mdspan.modifiers));
            lines.push(std::mem::take(&mut line));
            line_width = 0;

            // Now split again on the remaining content of the span, with the full `width`.
            let rest = {
                let orig = mdspan.content.as_str();
                let first_end =
                    first_part.as_ptr() as usize + first_part.len() - orig.as_ptr() as usize;
                debug_assert!(
                    orig.is_char_boundary(first_end),
                    "pointer arithmetic ndexing into string must be at UTF-8 boundaries"
                );
                #[expect(clippy::string_slice)]
                orig[first_end..].trim_start()
            };
            let options = Options::new(width as usize)
                .break_words(true)
                .word_splitter(textwrap::word_splitters::WordSplitter::NoHyphenation);
            let parts: Vec<_> = wrap(rest, options).into_iter().collect();

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
                if is_first && starting_new_line && !mdspan.modifiers.contains(Modifier::NewLine) {
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
                line.push(Span::new(part_content, modifiers));
                line_width += part_width;
            }
        } else {
            line.push(mdspan);
        }
    }

    if !line.is_empty() {
        lines.push(line);
    }

    // Nothing should ever exceed `width`.
    #[cfg(debug_assertions)]
    {
        for line in &lines {
            if line
                .iter()
                .any(|span| span.modifiers.contains(Modifier::LinkURL) && span.content.width() > 0)
            {
                // Ignore links, which can go over `width`.
                continue;
            }
            let widths: Vec<usize> = line.iter().map(|span| span.content.width()).collect();
            if (widths.into_iter().sum::<usize>() as u16) > width {
                #[cfg(feature = "ratatui")]
                log::error!(
                    "wrapped line longer than {width}: {:?}",
                    line.iter()
                        .map(|span| span.content.clone())
                        .collect::<Vec<String>>()
                        .join("")
                );
            }
        }
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
        let lines = wrap_md_spans_lines(4, mdspans, false);
        assert_eq!(
            lines,
            vec![vec![Span::from("one")], vec![Span::from("two")]]
        );
    }

    #[test]
    fn no_wrap() {
        let mdspans = vec![Span::from("one two")];
        let lines = wrap_md_spans_lines(10, mdspans, false);
        assert_eq!(lines, vec![vec![Span::from("one two")]]);
    }

    #[test]
    fn word_break() {
        let mdspans = vec![Span::from("one two")];
        let lines = wrap_md_spans_lines(2, mdspans, false);
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
        let mdspans = vec![Span::from("one "), Span::with("two", Modifier::NewLine)];
        let lines = wrap_md_spans_lines(10, mdspans, false);
        assert_eq!(
            lines,
            vec![
                vec![Span::from("one")],
                vec![Span::with("two", Modifier::NewLine),]
            ],
        );
    }

    #[test]
    #[ignore]
    // We are not doing the special "don't break URLs but move the surrounding parens into the URL
    // line" anymore.
    fn link_wrapping() {
        let mdspans = vec![
            Span::with("[", Modifier::LinkDescriptionWrapper),
            Span::with("link", Modifier::LinkDescription),
            Span::with("]", Modifier::LinkDescriptionWrapper),
            Span::with("(", Modifier::LinkURLWrapper),
            Span::with("https://example.com", Modifier::LinkURL),
            Span::with(")", Modifier::LinkURLWrapper),
        ];
        let lines = wrap_md_spans_lines(25, mdspans, false);
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
