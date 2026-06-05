use textwrap::{Options, wrap};
use unicode_width::UnicodeWidthStr;

use crate::{
    Line, LineKind, Mapper,
    link_tracker::{LinkTracker, TrackedUrl},
    markdown::{Modifier, Span},
};

/// Trim leading whitespace in place.
#[inline]
fn trim_start_inplace(s: &mut String) {
    let trimmed_len = s.trim_start().len();
    if trimmed_len < s.len() {
        let start = s.len() - trimmed_len;
        s.drain(..start);
    }
}

pub fn wrap_md_spans<M: Mapper>(
    width: u16,
    mdspans: Vec<Span>,
    prefix_width: usize,
    mapper: &M,
) -> Vec<Line> {
    let available_width = width.saturating_sub(prefix_width as u16).max(1);

    let mut tracker = LinkTracker::default().hide_urls(mapper.hide_urls());

    wrap_md_spans_lines(available_width, mdspans, mapper, prefix_width != 0)
        .into_iter()
        .filter(|line| !line.is_empty())
        .map(|mut spans| {
            for span in &spans {
                tracker.track(span);
            }

            if let Some(offset) = tracker.is_mid_link()
                && offset < available_width
            {
                // The link has been wrapped, fill it towards the end of the line.
                // This is important for OSC8 links, which must match the underlying ratatui buffer
                // exactly to avoid artifacts.
                spans.push(Span::new(
                    " ".repeat((available_width - offset) as usize),
                    Modifier::Link | Modifier::LinkDescription,
                ));
            }
            tracker.carriage_return();

            Line {
                spans,
                urls: tracker
                    .take_urls()
                    // Shift start-end by prefix_width.
                    .into_iter()
                    .map(|mut tracked_url| {
                        if let TrackedUrl::Link { start, end, .. } = &mut tracked_url {
                            *start += prefix_width as u16;
                            *end += prefix_width as u16;
                        }
                        tracked_url
                    })
                    .collect(),
                kind: LineKind::Blank, // will be set by wrapped_to_lines()
            }
        })
        .collect()
}

// Also used by table cell wrapping.
pub fn wrap_md_spans_lines<M: Mapper>(
    width: u16,
    mdspans: Vec<Span>,
    mapper: &M,
    is_indented: bool,
) -> Vec<Vec<Span>> {
    let hide_urls = mapper.hide_urls();
    let hard_softbreaks = mapper.hard_softbreaks();

    let mut lines: Vec<Vec<Span>> = Vec::new();
    let mut line: Vec<Span> = Vec::new();
    let mut after_newline = false;

    for mdspan in mdspans {
        if is_indented || hard_softbreaks {
            if mdspan.modifiers.contains(Modifier::NewLine) {
                if let Some(last) = line.last_mut() {
                    last.content.truncate(last.content.trim_end().len());
                }
                lines.push(std::mem::take(&mut line));
                after_newline = true;
            }
        } else {
            if mdspan.modifiers.contains(Modifier::HardLineBreak) {
                if let Some(last) = line.last_mut() {
                    last.content.truncate(last.content.trim_end().len());
                }
                lines.push(std::mem::take(&mut line));
                after_newline = true;
            }
            if mdspan.modifiers.contains(Modifier::NewLine) && !line.is_empty() {
                line.push(Span::new(String::from(" "), Modifier::default()));
            }
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
        let mut would_overflow = line_width + span_width > width;
        if would_overflow && mdspan.modifiers.contains(Modifier::LinkURL) {
            let move_paren = line.last().is_some_and(|last| {
                last.modifiers.contains(Modifier::LinkURLWrapper) && last.content == "("
            });
            if move_paren && let Some(paren) = line.pop() {
                lines.push(std::mem::take(&mut line));
                line.push(paren);
                line_width = 1;
            } else {
                // The last span was not "(", could be a fused "](" from an image.
                // Ignore but do move this LinkURL into its own line to try to avoid breaking.
                lines.push(std::mem::take(&mut line));
                line_width = 0;
            }
            would_overflow = line_width + span_width > width;
        }
        if would_overflow {
            // Note: this *was* something weird about moving links that would exceed `width`
            // together with their surrounding parens.
            let starting_new_line = !line.is_empty();

            // Split once with "remaining width" (`width - line_width`), to append the first part
            // onto the current line (if any, otherwise would just make a new line).
            let mut remaining_width = width.saturating_sub(line_width);
            if remaining_width == 0 {
                lines.push(std::mem::take(&mut line));
                remaining_width = width;
            }

            // If the first word of the content doesn't fit in the remaining space but does fit on
            // a full line, push the current line and start fresh. This prevents short words from
            // being split mid-word by break_words (e.g. "Is" → "I" + "s this...").
            let mut trim_first_part = false;
            if starting_new_line {
                let first_word_width = mdspan
                    .content
                    .split_whitespace()
                    .next()
                    .map(|w| UnicodeWidthStr::width(w) as u16)
                    .unwrap_or(0);
                if first_word_width > remaining_width && first_word_width <= width {
                    lines.push(std::mem::take(&mut line));
                    remaining_width = width;
                    // first_part will start a fresh line; trim any leading whitespace.
                    trim_first_part = true;
                }
            }

            let options = Options::new((remaining_width) as usize)
                .break_words(true)
                .word_splitter(textwrap::word_splitters::WordSplitter::NoHyphenation);
            let parts: Vec<_> = wrap(&mdspan.content, options).into_iter().collect();
            let Some(first_part) = parts.first() else {
                continue;
            };
            let first_content = first_part.as_ref();
            let mut first_content_owned = first_content.to_owned();
            if trim_first_part {
                trim_start_inplace(&mut first_content_owned);
            }
            let first_span = Span::new(first_content_owned, mdspan.modifiers);
            line.push(first_span);
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
    use crate::{
        DefaultMapper,
        markdown::{Modifier, Span},
    };

    #[test]
    fn simple_wrap() {
        let mdspans = vec![Span::from("one two")];
        let lines = wrap_md_spans_lines(4, mdspans, &DefaultMapper {}, false);
        assert_eq!(
            lines,
            vec![vec![Span::from("one")], vec![Span::from("two")]]
        );
    }

    #[test]
    fn no_wrap() {
        let mdspans = vec![Span::from("one two")];
        let lines = wrap_md_spans_lines(10, mdspans, &DefaultMapper {}, false);
        assert_eq!(lines, vec![vec![Span::from("one two")]]);
    }

    #[test]
    fn word_break() {
        let mdspans = vec![Span::from("one two")];
        let lines = wrap_md_spans_lines(2, mdspans, &DefaultMapper {}, false);
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
        let mdspans = vec![Span::from("one"), Span::with("two", Modifier::NewLine)];
        let lines = wrap_md_spans_lines(10, mdspans, &DefaultMapper {}, false);
        assert_eq!(
            lines,
            vec![vec![
                Span::from("one"),
                Span::from(" "),
                Span::with("two", Modifier::NewLine),
            ]],
        );
    }

    #[test]
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
        let lines = wrap_md_spans_lines(25, mdspans, &DefaultMapper {}, false);
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
