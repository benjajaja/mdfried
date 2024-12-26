use ratatui::text::{Line, Span};
use textwrap::{
    core::Fragment,
    wrap_algorithms::{wrap_optimal_fit, Penalties},
};

use crate::error::Error;

#[derive(Debug)]
struct WordSpan<'a> {
    span: Span<'a>,
    leading_whitespace: bool,
    is_first: bool,
    trailing_whitespace: bool,
}

impl Fragment for WordSpan<'_> {
    fn width(&self) -> f64 {
        self.span.width() as f64
    }

    fn whitespace_width(&self) -> f64 {
        1.0
    }

    fn penalty_width(&self) -> f64 {
        0.0
    }
}

#[macro_export]
macro_rules! tprintln {
    ($($arg:tt)*) => {
        #[cfg(test)]
        {
            println!($($arg)*);
        }
    };
}

pub fn wrap_spans(spans: Vec<Span>, max_width: usize) -> Result<Vec<Line>, Error> {
    tprintln!("wrap_spans({spans:?})");
    // Split the spans into more spans, one for every word.
    let mut word_spans: Vec<WordSpan> = vec![];
    for span in &spans {
        let leading_whitespace = span.content.starts_with(" ");
        let trailing_whitespace = span.content.ends_with(" ");

        let mut words = span.content.split_whitespace().peekable();

        let mut is_first = true;
        while let Some(word) = words.next() {
            let is_last = words.peek().is_none();
            word_spans.push(WordSpan {
                span: Span::from(word).style(span.style),
                leading_whitespace: is_first && leading_whitespace,
                is_first,
                trailing_whitespace: is_last && trailing_whitespace,
            });
            is_first = false;
        }
    }

    let widths = [max_width as f64];

    let wrapped_lines = wrap_optimal_fit(&word_spans, &widths, &Penalties::default())?;
    let mut lines = vec![];
    for words in wrapped_lines {
        let mut spans_out: Vec<Span> = vec![];
        let mut iter = words.iter().peekable();

        let mut fused_span = Span::default();
        while let Some(word) = iter.next() {
            let mut content = word.span.content.to_string();

            if word.leading_whitespace {
                content = format!(" {}", content);
            }
            if word.trailing_whitespace {
                content = format!("{} ", content);
            }

            if fused_span.content == "" {
                fused_span = Span::from(content).style(word.span.style);
            } else if fused_span.style == word.span.style {
                if word.is_first {
                    spans_out.push(fused_span);
                    if !word.leading_whitespace {
                        spans_out.push(Span::from(" "));
                    }
                    fused_span = Span::from(content).style(word.span.style);
                } else {
                    fused_span.content = format!("{} {}", fused_span.content, content).into();
                }
            } else {
                spans_out.push(fused_span);
                fused_span = Span::from(content).style(word.span.style);
            }
        }
        spans_out.push(fused_span);

        let line = Line::from(spans_out);
        debug_assert!(line.width() <= max_width);
        lines.push(line);
    }
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use ratatui::style::Stylize;

    use super::*; // Import items from parent module

    fn lines_as_string(lines: Vec<Line>) -> String {
        lines
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<String>>()
            .join("\n")
    }

    #[test]
    fn test_wrap_spans() {
        let lines = wrap_spans(vec![Span::from("hello this is dog")], 10).unwrap();
        assert_eq!(
            r#"hello this
is dog"#,
            lines_as_string(lines)
        );

        let letra = "El limón con la canela\n\
                     el limón con la canela\n\
                     lo rosita con el jazmín\n\
                     así tu cuerpo me huele\n\
                     cuando yo me arrimo a ti";
        let line = letra.split('\n').collect::<Vec<&str>>().join(" ");
        let lines = wrap_spans(vec![Span::from(line)], 24).unwrap();
        assert_eq!(letra, lines_as_string(lines));
    }

    fn s(content: &str) -> Span {
        Span::from(content)
    }

    #[test]
    fn test_style_with_space() {
        let lines = wrap_spans(vec![s("hello world").bold()], 100).unwrap();
        assert_eq!(vec![Line::from(vec![s("hello world").bold()]),], lines);

        let lines = wrap_spans(vec![s("hello").bold(), s(" "), s("world").bold()], 100).unwrap();
        assert_eq!(
            vec![Line::from(vec![
                s("hello").bold(),
                s(" "),
                s("world").bold(),
            ]),],
            lines
        );
    }

    #[test]
    fn test_styled_spans() {
        let lines = wrap_spans(vec![s("hello "), s("this").bold(), s(" is dog")], 20).unwrap();
        assert_eq!(
            vec![Line::from(vec![
                s("hello "),
                s("this").bold(),
                s(" is dog"),
            ]),],
            lines
        );
    }

    #[test]
    fn test_leading_trailing_space_in_styled_span() {
        let lines = wrap_spans(vec![s("hello "), s("this ").bold(), s("is dog")], 20).unwrap();
        assert_eq!(
            vec![Line::from(vec![
                s("hello "),
                s("this ").bold(),
                s("is dog"),
            ]),],
            lines
        );
        let lines = wrap_spans(vec![s("hello"), s(" this").bold(), s(" is dog")], 20).unwrap();
        assert_eq!(
            vec![Line::from(vec![
                s("hello"),
                s(" this").bold(),
                s(" is dog"),
            ]),],
            lines
        );
    }
}
