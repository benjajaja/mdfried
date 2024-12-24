use ratatui::text::{Line, Span};
use textwrap::{
    core::Fragment,
    wrap_algorithms::{wrap_optimal_fit, Penalties},
};

#[derive(Debug)]
struct WordSpan<'a> {
    span: Span<'a>,
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

// This probably has bugs and doesn't handle multi-width characters properly. Generated with AI.
pub fn wrap_spans(spans: Vec<Span>, max_width: usize) -> Vec<Line> {
    let mut word_spans: Vec<WordSpan> = vec![];
    for span in &spans {
        let words = span.content.split_whitespace();
        for word in words {
            word_spans.push(WordSpan {
                span: Span::from(word).style(span.style),
            });
        }
    }

    let widths = word_spans
        .iter()
        .map(|_| max_width as f64)
        .collect::<Vec<f64>>();

    let result = wrap_optimal_fit(&word_spans, &widths, &Penalties::default());
    let wrapped_lines = result.unwrap();
    let mut lines = vec![];
    for words in wrapped_lines {
        let mut spans_out: Vec<Span> = vec![];
        let word_count = words.len();
        for (i, word) in words.iter().enumerate() {
            let span = Span::from(word.span.content.to_string()).style(word.span.style);
            spans_out.push(span);
            if i < word_count - 1 {
                spans_out.push(Span::from(" "));
            }
        }
        let line = Line::from(spans_out);
        debug_assert!(line.width() <= max_width);
        lines.push(line);
    }
    lines
}

#[cfg(test)]
mod tests {
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
        let lines = wrap_spans(vec![Span::from("hello this is dog")], 10);
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
        let lines = wrap_spans(vec![Span::from(line)], 24);
        assert_eq!(letra, lines_as_string(lines));
    }
}
