use ratatui::{
    style::{Color, Stylize},
    text::{Line, Span},
};
use regex::Regex;

use crate::widget_sources::LineExtra;

pub fn capture_line<'a>(line: Line<'a>, text: &str, width: u16) -> (Line<'a>, Vec<LineExtra>) {
    let mut links = Vec::new();

    let mut new_spans = Vec::new();
    let mut offset = 0;
    for span in line.spans {
        capture_links(span, &mut offset, text, width, &mut new_spans, &mut links);
    }
    (Line::from(new_spans), links)
}

pub fn capture_links<'a>(
    span: Span<'a>,
    offset: &mut u16,
    text: &str,
    width: u16,
    new_spans: &mut Vec<Span<'a>>,
    links: &mut Vec<LineExtra>,
) {
    let md_link_regex = Regex::new(r"\[([^\]]+)\]\(([^)]+)\)?").unwrap();
    let url_regex = Regex::new(r"https?://[^\s)]+").unwrap();

    let mut spans: Vec<Span> = Vec::new();
    let mut last_end = 0;

    let span_content = &span.content;
    let parent_style = span.style;

    let mut found_link = false;
    for cap in md_link_regex.captures_iter(span_content) {
        let Some(full_match) = cap.get(0) else {
            continue;
        };
        found_link = true;

        let match_start = full_match.start();
        let match_end = full_match.end();

        // Add any non-link text before this match
        if match_start > last_end {
            spans.push(
                Span::from(span_content[last_end..match_start].to_string()).style(parent_style),
            );
        }

        if let (Some(link_text), Some(url)) = (cap.get(1), cap.get(2)) {
            let mut url_str = url.as_str();
            let decor_style = parent_style.fg(Color::DarkGray);
            // TODO: we should check if it got cut off before!
            spans.push(Span::from("[").style(decor_style));
            spans.push(
                Span::from(link_text.as_str().to_string())
                    .style(parent_style)
                    .fg(Color::LightBlue),
            );
            spans.push(Span::from("]").style(decor_style));
            spans.push(Span::from("(").style(decor_style));
            spans.push(
                Span::from(url_str.to_string())
                    .style(parent_style)
                    .fg(Color::Blue)
                    .underlined(),
            );
            if full_match.as_str().ends_with(')') {
                spans.push(Span::from(")").style(decor_style));
            }

            last_end = match_end;

            // Now try to find the full url in the original text again, might have
            // been split up and we don't want to open cut-off URLs.
            // This is code block is pretty ugly, but it works for now.
            if match_end as u16 == width
                && let Some(pos) = text.find(url_str)
            {
                let line_end = text[pos..]
                    .find('\n')
                    .map(|n| pos + n)
                    .unwrap_or(text.len());
                let line_slice = &text[pos..line_end];
                if let Some(full_match) = url_regex.find(line_slice) {
                    url_str = full_match.as_str();
                }
            }

            links.push(LineExtra::Link(
                url_str.to_string(),
                *offset + url.start() as u16,
                *offset + url.end() as u16,
            ));
        }
    }
    if found_link {
        if last_end < span_content.len() {
            // There is some leftover spans.
            spans.push(Span::from(span_content[last_end..].to_string()).style(parent_style));
        }
        for span in &spans {
            *offset += span.width() as u16;
        }
        new_spans.append(&mut spans);
    } else {
        capture_urls(span, offset, text, width, new_spans, links);
    }
}

pub fn capture_urls<'a>(
    span: Span<'a>,
    offset: &mut u16,
    text: &str,
    width: u16,
    new_spans: &mut Vec<Span<'a>>,
    links: &mut Vec<LineExtra>,
) {
    let url_regex =
        Regex::new(r"https?://[A-Za-z0-9._~:/?#\[\]@!$&'()*+,;=%\-]+[A-Za-z0-9/?#=\-]").unwrap();

    let mut spans: Vec<Span> = Vec::new();
    let mut last_end = 0;

    let span_content = &span.content;
    let parent_style = span.style;

    let mut found_link = false;
    for cap in url_regex.find_iter(span_content) {
        found_link = true;

        let match_start = cap.start();
        let match_end = cap.end();

        // Add any non-link text before this match
        if match_start > last_end {
            spans.push(
                Span::from(span_content[last_end..match_start].to_string()).style(parent_style),
            );
        }

        let mut url_str = cap.as_str();
        spans.push(
            Span::from(url_str.to_string())
                .style(parent_style)
                .fg(Color::Blue)
                .underlined(),
        );

        last_end = match_end;

        // Now try to find the full url in the original text again, might have
        // been split up and we don't want to open cut-off URLs.
        // This is code block is pretty ugly, but it works for now.
        if match_end as u16 == width
            && let Some(pos) = text.find(url_str)
        {
            let line_end = text[pos..]
                .find('\n')
                .map(|n| pos + n)
                .unwrap_or(text.len());
            let line_slice = &text[pos..line_end];
            if let Some(full_match) = url_regex.find(line_slice) {
                url_str = full_match.as_str();
            }
        }

        links.push(LineExtra::Link(
            url_str.to_string(),
            *offset + cap.start() as u16,
            *offset + cap.end() as u16,
        ));
    }
    if found_link {
        if last_end < span_content.len() {
            // There is some leftover spans.
            spans.push(Span::from(span_content[last_end..].to_string()).style(parent_style));
        }
        for span in &spans {
            *offset += span.width() as u16;
        }
        new_spans.append(&mut spans);
    } else {
        // Nothing found, push original span.
        *offset += span.width() as u16;
        new_spans.push(span);
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use ratatui::{
        style::{Color, Stylize},
        text::Span,
    };

    use crate::{markdown::links::capture_links, widget_sources::LineExtra};

    #[test]
    fn basic() {
        let text = "*deep* [fried](http://url)";
        let preceding_span = Span::from("deep").bold();
        let span = Span::from(" [fried](http://url)");
        let width = 80;
        let mut new_spans = Vec::new();
        let mut links = Vec::new();
        let mut offset = preceding_span.width() as u16;
        capture_links(span, &mut offset, text, width, &mut new_spans, &mut links);

        let decor_style = Color::DarkGray;
        assert_eq!(
            vec![
                Span::from(" "),
                Span::from("[").style(decor_style),
                Span::from("fried").fg(Color::LightBlue),
                Span::from("]").style(decor_style),
                Span::from("(").style(decor_style),
                Span::from("http://url").fg(Color::Blue).underlined(),
                Span::from(")").style(decor_style),
            ],
            new_spans
        );

        assert_eq!(
            vec![LineExtra::Link("http://url".to_string(), 13, 23)],
            links,
        );
    }

    #[test]
    fn offsets() {
        let text = "[a](http://a) [b](http://b)";
        let span = Span::from("[a](http://a) [b](http://b)");
        let width = 80;
        let mut new_spans = Vec::new();
        let mut links = Vec::new();
        capture_links(span, &mut 0, text, width, &mut new_spans, &mut links);

        let decor_style = Color::DarkGray;
        assert_eq!(
            vec![
                Span::from("[").style(decor_style),
                Span::from("a").fg(Color::LightBlue),
                Span::from("]").style(decor_style),
                Span::from("(").style(decor_style),
                Span::from("http://a").fg(Color::Blue).underlined(),
                Span::from(")").style(decor_style),
                Span::from(" "),
                Span::from("[").style(decor_style),
                Span::from("b").fg(Color::LightBlue),
                Span::from("]").style(decor_style),
                Span::from("(").style(decor_style),
                Span::from("http://b").fg(Color::Blue).underlined(),
                Span::from(")").style(decor_style),
            ],
            new_spans
        );

        assert_eq!(
            vec![
                LineExtra::Link("http://a".to_string(), 4, 12),
                LineExtra::Link("http://b".to_string(), 18, 26),
            ],
            links,
        );
    }
}
