use textwrap::{Options, wrap};
use unicode_width::UnicodeWidthStr as _;

use super::{MdLineContainer, nesting_to_prefix_spans};
use crate::markdown::{Modifier, Span};
use crate::{Line, LineKind, Mapper};

pub(super) fn code_block_to_lines<M: Mapper>(
    width: u16,
    language: String,
    code: String,
    nesting: Vec<MdLineContainer>,
    mapper: &M,
) -> Vec<Line> {
    let code_lines: Vec<&str> = code.lines().collect();
    let num_lines = code_lines.len();
    if num_lines == 0 {
        return vec![];
    }

    let prefix_spans = nesting_to_prefix_spans(&nesting, mapper);
    let prefix_width: usize = prefix_spans.iter().map(|s| s.content.width()).sum();
    let available_width = (width as usize).saturating_sub(prefix_width).max(1);

    let mut result = Vec::new();

    if !mapper.hide_codeblock_language() && !language.is_empty() {
        let parts = if language.width() > available_width {
            let options = Options::new(available_width)
                .break_words(true)
                .word_splitter(textwrap::word_splitters::WordSplitter::NoHyphenation);
            wrap(&language, options).into_iter().collect()
        } else {
            vec![language.clone().into()]
        };
        for part in parts {
            let mut spans = prefix_spans.clone();
            spans.push(Span::new(part.into_owned(), Modifier::Code));
            result.push(Line {
                spans,
                kind: LineKind::CodeBlock {
                    language: language.clone(),
                },
                urls: Vec::new(),
            });
        }
    }

    for line in code_lines {
        let line_width = line.width();

        if line_width > available_width {
            let options = Options::new(available_width)
                .break_words(true)
                .word_splitter(textwrap::word_splitters::WordSplitter::NoHyphenation);
            let parts: Vec<_> = wrap(line, options).into_iter().collect();

            for part in parts {
                let content_width = part.width();
                let padding = available_width.saturating_sub(content_width);

                let mut spans = prefix_spans.clone();
                spans.push(Span::new(part.into_owned(), Modifier::Code));
                if padding > 0 {
                    spans.push(Span::new(" ".repeat(padding), Modifier::Code));
                }
                result.push(Line {
                    spans,
                    kind: LineKind::CodeBlock {
                        language: language.clone(),
                    },
                    urls: Vec::new(),
                });
            }
        } else {
            let padding = available_width.saturating_sub(line_width);

            let mut spans = prefix_spans.clone();
            spans.push(Span::new(line.to_owned(), Modifier::Code));
            if padding > 0 {
                spans.push(Span::new(" ".repeat(padding), Modifier::Code));
            }
            result.push(Line {
                spans,
                kind: LineKind::CodeBlock {
                    language: language.clone(),
                },
                urls: Vec::new(),
            });
        }
    }

    result
}

#[cfg(test)]
#[expect(clippy::unwrap_used)]
mod tests {
    use crate::{DefaultMapper, MdFrier};

    #[test]
    fn code_block_wrapping() {
        // Width of 5 should wrap "abcdefghij" into two lines.
        let input = "```\nabcdefghij\n```\n";
        let mut frier = MdFrier::new().unwrap();
        let lines: Vec<_> = frier.parse(5, input, &DefaultMapper).unwrap().collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].spans[0].content, "abcde");
        assert_eq!(lines[1].spans[0].content, "fghij");
    }

    #[test]
    fn code_block_no_wrap_when_fits() {
        let input = "```\nabcde\n```\n";
        let mut frier = MdFrier::new().unwrap();
        let lines: Vec<_> = frier.parse(5, input, &DefaultMapper).unwrap().collect();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].spans[0].content, "abcde");
    }
}
