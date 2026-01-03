use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

use crate::{
    Event, LineExtra,
    model::DocumentId,
    widget_sources::{WidgetSource, WidgetSourceData},
    worker::pipeline::{
        markdown::{MdModifier, MdNode},
        post_incr_source_id,
        wrap::WrappedLine,
    },
};

pub const LINK_DESC_OPEN: &str = "▐";
pub const LINK_DESC_CLOSE: &str = "▌";
pub const LINK_URL_OPEN: &str = "◖";
pub const LINK_URL_CLOSE: &str = "◗";
pub const BLOCKQUOTE_BAR: &str = "▌";

pub const COLOR_LINK_BG: Color = Color::Indexed(237);
pub const COLOR_LINK_FG: Color = Color::Indexed(4);
pub const COLOR_PREFIX: Color = Color::Indexed(189);
pub const BLOCKQUOTE_COLORS: [Color; 6] = [
    Color::Indexed(202),
    Color::Indexed(203),
    Color::Indexed(204),
    Color::Indexed(205),
    Color::Indexed(206),
    Color::Indexed(207),
];

pub fn span_from_mdspan(mdspan: MdNode) -> Span<'static> {
    let mut style = Style::default();
    if mdspan.extra.contains(MdModifier::Emphasis) {
        style = style.add_modifier(Modifier::ITALIC).fg(Color::Indexed(220));
    }
    if mdspan.extra.contains(MdModifier::StrongEmphasis) {
        style = style.add_modifier(Modifier::BOLD).fg(Color::Indexed(220));
    }
    if mdspan.extra.contains(MdModifier::Code) {
        style = style.fg(Color::Indexed(203)).bg(Color::Indexed(236));
    }

    if mdspan.extra.contains(MdModifier::LinkURLWrapper) {
        let bracket = if mdspan.content == "(" {
            LINK_URL_OPEN
        } else {
            LINK_URL_CLOSE
        };
        return Span::styled(bracket, style.fg(COLOR_LINK_BG));
    }
    if mdspan.extra.contains(MdModifier::LinkURL) {
        style = style.fg(COLOR_LINK_FG).bg(COLOR_LINK_BG).underlined();
    }

    if mdspan.extra.contains(MdModifier::LinkDescriptionWrapper) {
        let bracket = if mdspan.content == "[" {
            LINK_DESC_OPEN
        } else {
            LINK_DESC_CLOSE
        };
        return Span::styled(bracket, style.fg(COLOR_LINK_BG));
    }
    if mdspan.extra.contains(MdModifier::LinkDescription) {
        style = style.fg(COLOR_LINK_FG).bg(COLOR_LINK_BG);
    }

    Span::styled(mdspan.content, style)
}

pub fn blank_line_event(document_id: DocumentId, source_id: &mut Option<usize>) -> Event {
    Event::Parsed(
        document_id,
        WidgetSource {
            id: post_incr_source_id(source_id),
            height: 1,
            data: WidgetSourceData::Line(Line::default(), Vec::new()),
        },
    )
}

/// Create a blank line with blockquote prefix bars.
pub fn blockquote_blank_line_event(
    document_id: DocumentId,
    source_id: &mut Option<usize>,
    blockquote_depth: usize,
) -> Event {
    // Build blockquote prefix with styled bars
    let mut spans = Vec::new();
    for depth in 0..blockquote_depth {
        let color = BLOCKQUOTE_COLORS[depth.min(5)];
        spans.push(Span::styled(
            BLOCKQUOTE_BAR.to_owned(),
            Style::default().fg(color),
        ));
        spans.push(Span::from(" "));
    }
    Event::Parsed(
        document_id,
        WidgetSource {
            id: post_incr_source_id(source_id),
            height: 1,
            data: WidgetSourceData::Line(Line::from(spans), Vec::new()),
        },
    )
}

/// Style a prefix string, handling blockquote markers and checklist items.
fn style_prefix(prefix: &str, blockquote_depth: usize) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut remaining = prefix;
    let mut current_depth = 0;

    // Process blockquote markers ("> " patterns or "  " for soft-wrapped continuation lines)
    while current_depth < blockquote_depth {
        if let Some(rest) = remaining.strip_prefix("> ") {
            let color = BLOCKQUOTE_COLORS[current_depth.min(5)];
            spans.push(Span::styled(
                BLOCKQUOTE_BAR.to_owned(),
                Style::default().fg(color),
            ));
            spans.push(Span::from(" "));
            remaining = rest;
            current_depth += 1;
        } else if let Some(rest) = remaining.strip_prefix(">") {
            let color = BLOCKQUOTE_COLORS[current_depth.min(5)];
            spans.push(Span::styled(
                BLOCKQUOTE_BAR.to_owned(),
                Style::default().fg(color),
            ));
            remaining = rest;
            current_depth += 1;
        } else if let Some(rest) = remaining.strip_prefix("  ") {
            // Soft-wrapped continuation line (spaces instead of "> ")
            let color = BLOCKQUOTE_COLORS[current_depth.min(5)];
            spans.push(Span::styled(
                BLOCKQUOTE_BAR.to_owned(),
                Style::default().fg(color),
            ));
            spans.push(Span::from(" "));
            remaining = rest;
            current_depth += 1;
        } else {
            break;
        }
    }

    // Style the rest of the prefix (list markers, checkboxes, etc.)
    if !remaining.is_empty() {
        // Replace [x] or [X] with [✓] for checklist items
        let styled_remaining = remaining.replace("[x]", "[✓]").replace("[X]", "[✓]");
        spans.push(Span::styled(
            styled_remaining,
            Style::default().fg(COLOR_PREFIX),
        ));
    }

    spans
}

pub fn wrapped_lines_to_events(
    document_id: DocumentId,
    source_id: &mut Option<usize>,
    wrapped_lines: Vec<WrappedLine>,
    blockquote_depth: usize,
) -> Vec<Event> {
    let mut events = Vec::new();

    for wrapped_line in wrapped_lines {
        let WrappedLine {
            prefix,
            spans,
            images,
        } = wrapped_line;

        // Skip if no meaningful content (just prefix or whitespace)
        let has_content = spans.iter().any(|s| !s.content.trim().is_empty());
        if !has_content && images.is_empty() {
            continue;
        }

        // Convert MdNode spans to ratatui Spans, collecting link URLs
        let mut line_spans = Vec::new();
        let mut links = Vec::new();

        // Add styled prefix spans
        if !prefix.is_empty() {
            line_spans.extend(style_prefix(&prefix, blockquote_depth));
        }

        let mut char_offset = line_spans.iter().map(|s| s.content.len()).sum::<usize>();

        for mdspan in spans {
            // Track link URLs for LineExtra
            if mdspan.extra.contains(MdModifier::LinkURL)
                && !mdspan.extra.contains(MdModifier::LinkURLWrapper)
            {
                let start = char_offset as u16;
                let end = (char_offset + mdspan.content.len()) as u16;
                links.push(LineExtra::Link(mdspan.content.clone(), start, end));
            }
            char_offset += mdspan.content.len();
            line_spans.push(span_from_mdspan(mdspan));
        }

        // Create line event
        if !line_spans.is_empty() {
            events.push(Event::Parsed(
                document_id,
                WidgetSource {
                    id: post_incr_source_id(source_id),
                    height: 1,
                    data: WidgetSourceData::Line(Line::from(line_spans), links),
                },
            ));
        }

        // Create image events
        for img in images {
            events.push(Event::ParsedImage(
                document_id,
                post_incr_source_id(source_id),
                img,
            ));
        }
    }

    events
}
