use textwrap::{
    Options,
    WordSeparator::UnicodeBreakProperties,
    WordSplitter,
    core::{Fragment, Word, break_words},
    wrap,
    wrap_algorithms::{Penalties, wrap_optimal_fit},
};
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

/// Wrap into a WrappedLine
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
            let is_source_newline = mdspans
                .first()
                .is_some_and(|s| s.modifiers.contains(Modifier::NewLine));

            let is_first = line_idx == 0 || is_source_newline;

            // Extract images from spans
            let images: Vec<ImageRef> = mdspans
                .iter()
                .filter_map(|s| {
                    if s.modifiers.contains(Modifier::LinkURL)
                        && s.modifiers.contains(Modifier::Image)
                        && let Some(source_content) = &s.source_content
                    {
                        Some(ImageRef {
                            url: source_content.as_ref().to_owned(),
                            description: String::new(),
                        })
                    } else {
                        None
                    }
                })
                .collect();

            // Filter out image URL spans from content
            let spans: Vec<Span> = mdspans
                .into_iter()
                .filter(|s| !s.modifiers.contains(Modifier::Image))
                .collect();

            WrappedLine {
                is_first,
                spans,
                images,
            }
        })
        .collect()
}

/// Wrapper around [`Word`] with a flag that indicates that the word has been broken.
/// The following [`TaggedWord`] would be the next part, and so on.
#[derive(Debug)]
struct TaggedWord<'a> {
    word: Word<'a>,
    broken: bool,
    is_last: bool,
}
impl<'a> TaggedWord<'a> {
    fn broken(word: Word<'a>, is_last: bool) -> Self {
        Self {
            word,
            broken: true,
            is_last,
        }
    }
    fn width_with_whitespace(&self) -> u16 {
        self.word.width() as u16 + self.word.whitespace.width() as u16
    }
}

impl<'a> From<Word<'a>> for TaggedWord<'a> {
    fn from(word: Word<'a>) -> Self {
        TaggedWord {
            word,
            broken: false,
            is_last: false,
        }
    }
}

impl Fragment for TaggedWord<'_> {
    fn width(&self) -> f64 {
        self.word.width()
    }

    fn whitespace_width(&self) -> f64 {
        self.word.whitespace_width()
    }

    fn penalty_width(&self) -> f64 {
        self.word.penalty_width()
    }
}

impl<'a> From<TaggedWord<'a>> for Word<'a> {
    fn from(value: TaggedWord<'a>) -> Self {
        value.word
    }
}

/// Wrap a Vec<Span>, for example for wrapping table cell content.
pub(crate) fn wrap_md_spans_lines(width: u16, spans: Vec<Span>) -> Vec<Vec<Span>> {
    let mut lines: Vec<Vec<Span>> = Vec::new();
    let mut line: Vec<Span> = Vec::new();
    let mut after_newline = false;

    for span in spans {
        if span.modifiers.contains(Modifier::NewLine) {
            if let Some(last) = line.last_mut() {
                last.content.truncate(last.content.trim_end().len());
            }
            lines.push(std::mem::take(&mut line));
            after_newline = true;
        }

        // Strip leading whitespace from content after a hard line break
        let mut span = span;
        if after_newline && !span.content.is_empty() {
            trim_start_inplace(&mut span.content);
            after_newline = false;
        }

        let span_width = span.content.width() as u16;
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
                let unicode_words = UnicodeBreakProperties.find_words(&span.content);

                let split_words: Vec<TaggedWord> = unicode_words
                    .flat_map(|mut word| {
                        let split_points = WordSplitter::NoHyphenation.split_points(&word);
                        if split_points.len() == 0 {
                            return vec![word];
                        }

                        let mut split_words: Vec<Word> = Vec::new();
                        for mid in split_points {
                            let (a, b) = word.word.split_at(mid);
                            let mut clone = word.clone();
                            word.word = b;
                            clone.word = a;
                            split_words.push(clone);
                        }
                        split_words.push(word);
                        return split_words;
                    })
                    .flat_map(|word| {
                        if word.word.width() <= width as usize {
                            return vec![word.into()];
                        }
                        let parts: Vec<Word> = word.break_apart(width as usize).collect();
                        parts
                            .iter()
                            .enumerate()
                            .map(|(i, word)| TaggedWord::broken(*word, i == parts.len() - 1))
                            .collect()
                    })
                    .collect();

                // let broken_words = break_words(&split_words, width as usize);
                // let word_lines =
                // wrap_optimal_fit(&split_words, &[width as f64], &Penalties::default())
                // .expect("overflow error");
                //
                // for word_line in word_lines {
                // let line: Vec<Span> = word_line
                // .into_iter()
                // .enumerate()
                // .map(|(i, word)| {
                // let mut content = String::from(word.word.word);
                // if i < word_line.len() - 1 {
                // content.push_str(word.word.whitespace);
                // }
                // let modifiers = span.modifiers.union(word.modifier());
                // Span {
                // content,
                // modifiers,
                // source_content: span.source_content.clone(),
                // }
                // })
                // .collect();
                // lines.push(line);
                // }
                // continue;

                // let options = Options::new(width as usize)
                // .break_words(true)
                // .word_splitter(WordSplitter::NoHyphenation);
                // let parts: Vec<_> = wrap(&span.content, options).into_iter().collect();
                let parts = split_words;
                let num_parts = parts.len();
                let ends_with_space = span.content.ends_with(' ');
                let mut copied_newline = false;
                for (i, tagged) in parts.into_iter().enumerate() {
                    eprintln!("part #{i} \"{:?}\"", tagged);
                    let is_last = i == num_parts - 1;
                    let is_first = i == 0;
                    // TODO: review and decide if we can use tagged.is_last instead.
                    // let mut part_content: String = if !is_last && ends_with_space
                    //&& !tagged.word.whitespace.is_empty()
                    // !tagged.word.whitespace.is_empty()
                    // && !tagged.is_last
                    // && !ends_with_space
                    // {
                    // let mut s = String::with_capacity(tagged.word.len() + 1);
                    // s.push_str(&tagged.word);
                    // s.push_str(&tagged.word.whitespace);
                    // s
                    // } else {
                    // tagged.word.word.to_owned()
                    // };
                    // if is_first && starting_new_line && !span.modifiers.contains(Modifier::NewLine)
                    // {
                    // trim_start_inplace(&mut part_content);
                    // }
                    // let part_width = part_content.width() as u16;
                    let carriage_return = line_width + tagged.width_with_whitespace() > width;
                    let part_content = if carriage_return {
                        tagged.word.word.to_owned()
                    } else {
                        let mut s = String::with_capacity(tagged.word.len() + 1);
                        s.push_str(&tagged.word);
                        s.push_str(&tagged.word.whitespace);
                        s
                    };

                    if carriage_return {
                        lines.push(std::mem::take(&mut line));
                        line_width = 0;
                    }
                    let mut extra = span.modifiers;
                    if !copied_newline {
                        copied_newline = true;
                    } else {
                        extra.remove(Modifier::NewLine);
                    }
                    if tagged.broken {
                        extra = extra.union(if tagged.is_last {
                            Modifier::WrappedEnd
                        } else {
                            Modifier::Wrapped
                        });
                    }
                    // Preserve source_content when splitting spans (for wrapped URLs)
                    line.push(Span {
                        content: part_content,
                        modifiers: extra,
                        source_content: span.source_content.clone(),
                    });
                    line_width += tagged.width_with_whitespace();
                }
            } else {
                let mut mdspan = span;
                if starting_new_line && !mdspan.modifiers.contains(Modifier::NewLine) {
                    trim_start_inplace(&mut mdspan.content);
                }
                line.push(mdspan);
            }
        } else {
            line.push(span);
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
                vec![Span::from("on").modifiers(Modifier::Wrapped)],
                vec![Span::from("e").modifiers(Modifier::WrappedEnd)],
                vec![Span::from("tw").modifiers(Modifier::Wrapped)],
                vec![Span::from("o").modifiers(Modifier::WrappedEnd)]
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
            Span::new("https://example.com".into(), Modifier::LinkURL),
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
