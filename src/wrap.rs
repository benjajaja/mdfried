use ratatui::text::{Line, Span};
use textwrap::{Options, wrap};
use unicode_width::UnicodeWidthStr as _;

use crate::{
    Event, MarkdownImage,
    markdown::{MdModifier, MdSpan},
    model::DocumentId,
    widget_sources::{LineExtra, WidgetSource, WidgetSourceData},
    worker::post_incr_source_id,
};

pub fn wrap_md_spans(
    document_id: DocumentId,
    source_id: &mut Option<usize>,
    width: u16,
    mdspans: Vec<MdSpan>,
) -> Vec<Event> {
    let mut line_events: Vec<Event> = Vec::new();

    let mut line_width = 0;
    let mut spans = Vec::new();
    let mut extras = Vec::new();
    let mut link_offset = 0; // TODO this sucks
    let mut had_image = None;

    for mdspan in mdspans {
        let span_width = mdspan.content.width();
        let would_overflow = line_width + span_width as u16 > width;

        if mdspan.extra.contains(MdModifier::NewLine) || would_overflow {
            // println!(
            // "is_overflow {would_overflow} / starts_with_newline {starts_with_newline}"
            // );
            // push spans before this one into a line
            line_width = 0;
            // println!("push line: {spans:?}");
            carriage_return(
                &mut line_events,
                document_id,
                source_id,
                &mut spans,
                &mut extras,
                &mut had_image,
                width,
            );
            link_offset = 0;
        }

        if mdspan.extra.contains(MdModifier::LinkURL) {
            if mdspan.extra.contains(MdModifier::Image) {
                had_image = Some(mdspan.content.clone());
            } else {
                let url = mdspan.content.clone();
                let url_width = url.width();
                extras.push(LineExtra::Link(
                    url,
                    link_offset,
                    link_offset + (url_width as u16),
                ));
            }
        }
        link_offset += span_width as u16;
        line_width += span_width as u16;
        // println!("next: {mdspan:?}");
        let span: Span<'static> = Span::styled(mdspan.content, mdspan.style);
        spans.push(span);
    }

    if !spans.is_empty() {
        // println!("last");
        carriage_return(
            &mut line_events,
            document_id,
            source_id,
            &mut spans,
            &mut extras,
            &mut had_image,
            width,
        );
    }
    debug_assert!(spans.is_empty(), "used up all spans");

    line_events
}

// Do you remember that sound?
fn carriage_return(
    line_events: &mut Vec<Event>,
    document_id: DocumentId,
    source_id: &mut Option<usize>,
    spans: &mut Vec<Span<'static>>,
    extras: &mut Vec<LineExtra>,
    had_image: &mut Option<String>,
    max_width: u16,
) {
    let line = if spans.len() == 1 && spans[0].width() > max_width as usize {
        // println!("break it down");
        let spans = std::mem::take(spans);
        let span = &spans[0];
        let options = Options::new(max_width as usize)
            .break_words(true) // break long words/URLs if they exceed width
            .word_splitter(textwrap::word_splitters::WordSplitter::NoHyphenation); // no hyphens when breaking
        let parts = wrap(&span.content, options);

        let part_spans: Vec<Span<'static>> = parts
            .iter()
            .map(|part| {
                let mut part_span = Span::from(part.to_string());
                part_span.style = span.style;
                // println!("part : {}", part);
                // println!("part width: {}", part.width());
                part_span
            })
            .collect();
        // println!("parts: {part_spans:?}");

        let last_index = part_spans.len().saturating_sub(1);
        let mut last_line = Line::default();
        for (i, part_span) in part_spans.into_iter().enumerate() {
            if i != last_index {
                let line = Line::from(part_span);
                line_events.push(Event::Parsed(
                    document_id,
                    WidgetSource {
                        id: post_incr_source_id(source_id),
                        height: 1,
                        data: WidgetSourceData::Line(line, std::mem::take(extras)),
                    },
                ));
            } else {
                last_line = Line::from(part_span);
            }
        }
        last_line
    } else {
        Line::from(std::mem::take(spans))
    };

    if let Some(url) = had_image.take() {
        // Once this works, stop the "parse only lines with an image on their own" thing, drop
        // MdSection::Image entirely.
        log::debug!("had_image");
        line_events.push(Event::ParsedImage(
            document_id,
            post_incr_source_id(source_id),
            MarkdownImage {
                destination: url,
                description: String::from("TODO: image_description"),
            },
        ));
    } else {
        line_events.push(Event::Parsed(
            document_id,
            WidgetSource {
                id: post_incr_source_id(source_id),
                height: 1,
                data: WidgetSourceData::Line(line, std::mem::take(extras)),
            },
        ));
    }
}
