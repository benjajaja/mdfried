use unicode_width::UnicodeWidthStr as _;

use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Layout, Position, Rect, Size},
    style::{Color, Stylize as _},
    text::{Line, Span},
    widgets::{Block, BorderType, Paragraph, Widget},
};

use mdfrier::{Mapper as _, SourceContent};
use ratatui_image::{
    Image,
    sliced::{SignedPosition, SlicedImage},
};

use crate::{
    big_text::BigText,
    cursor::{Cursor, CursorPointer},
    document::{LineExtra, SectionContent},
    model::{InputQueue, Model},
    sources::BuiltIn,
};

pub const WELCOME_LOGO_SIZE: (u16, u16) = (32, 8);

pub fn view(model: &Model, buf: &mut Buffer) -> Position {
    let inner_area = {
        let frame_area = *buf.area();
        let padding = model.block_padding(frame_area);
        let block = Block::new().padding(padding);
        let inner = block.inner(frame_area);
        block.render(frame_area, buf);
        inner
    };

    // Get the selected link URL if in Links mode (for highlighting all spans of wrapped URLs)
    let selected_url = match &model.cursor {
        Cursor::Links(pointer) => model.selected_link_url(pointer),
        _ => None,
    };

    let mut y: i32 = 0 - (model.scroll as i32);
    for section in model.sections() {
        if y + (section.height as i32) < 0 {
            y += section.height as i32;
            continue;
        }
        match &section.content {
            SectionContent::Lines(lines) | SectionContent::Code(_, lines) => {
                section_lines(
                    lines,
                    buf,
                    &mut y,
                    inner_area,
                    model,
                    &selected_url,
                    section.id,
                );
            }
            SectionContent::Image(_markdown_link, sliced_proto, size, _max_size) => {
                // TODO: just fix up inner_area at once
                let mut inner_area = inner_area;
                inner_area.height -= 1;
                SlicedImage::new(sliced_proto, SignedPosition { x: 0, y: y as i16 })
                    .render(inner_area, buf);
                y += size.height as i32;
            }
            SectionContent::ImagePlaceholder(_, lines) => {
                for (line, _extras) in lines.iter() {
                    if y < 0 {
                        y += 1;
                        continue; // skip this line.
                    }
                    let p = Paragraph::new(line.clone());
                    render_lines(p, 1, y as u16, inner_area, buf);
                    y += 1;
                }
            }
            SectionContent::Header(text, tier, proto) => {
                // Only render headers if fully in view
                if y >= 0 && (y as u16) < inner_area.bottom() - 2 {
                    if let Some(proto) = proto {
                        let img = Image::new(proto);
                        render_lines(img, section.height, y as u16, inner_area, buf);
                    } else {
                        let big_text = BigText::new(text, *tier, model.theme().header_color);
                        render_lines(big_text, 2, y as u16, inner_area, buf);
                    }
                }
                y += section.height as i32;
            }
            SectionContent::HeaderPlaceholder(_, _, lines) => {
                for (line, _) in lines.iter() {
                    if y < 0 {
                        y += 1;
                        continue; // skip this line.
                    }
                    let line = if let Some(header_color) = model.theme().header_color {
                        line.clone().fg(header_color)
                    } else {
                        line.clone()
                    };
                    let p = Paragraph::new(line);
                    render_lines(p, 1, y as u16, inner_area, buf);
                    y += 1;
                }
                y += 1;
            }
        }
        if y >= inner_area.height as i32 - 1 {
            // Do not render into last line, nor beyond area.
            break;
        }
    }

    if let Some(BuiltIn::Welcome) = model.builtin_override_view() {
        render_welcome(model, inner_area, buf);
    }

    let status_line_y = inner_area.height - 1;
    let mut cursor_position = Position::from((0, buf.area.height - 1));
    if let Some(err) = &model.last_error {
        let line = Line::from(err.to_string());
        let width = line.width() as u16;
        let searchbar = Paragraph::new(line).fg(Color::Red);
        searchbar.render(Rect::new(0, status_line_y, width, 1), buf);
        cursor_position.x = width;
    } else {
        match &model.input_queue {
            InputQueue::None => match &model.cursor {
                Cursor::None => {}
                Cursor::Links(_) => {
                    let (fg, bg) = (Color::Indexed(15), Color::Indexed(32));
                    let line = if model.theme().hide_urls()
                        && let Some(selected_url) = selected_url
                    {
                        let url_display = selected_url.as_ref().to_owned();
                        Line::from(vec![
                            Span::from(model.theme().link_url_open()).fg(bg),
                            Span::from(url_display).fg(fg).bg(bg),
                            Span::from(model.theme().link_url_close()).fg(bg),
                        ])
                    } else {
                        Line::from(Span::from("Links").fg(Color::Indexed(32)))
                    };
                    let width = line.width() as u16;
                    let searchbar = Paragraph::new(line);
                    searchbar.render(Rect::new(0, status_line_y, width, 1), buf);
                }
                Cursor::Search(needle, _) => {
                    let mut line = Line::default();
                    line.spans.push(Span::from("/").fg(Color::Indexed(148)));
                    let needle = Span::from(needle.as_str()).fg(Color::Indexed(148));
                    line.spans.push(needle);
                    let width = line.width() as u16;
                    let searchbar = Paragraph::new(line);
                    searchbar.render(Rect::new(0, status_line_y, width, 1), buf);
                }
            },
            InputQueue::Search(needle) => {
                let mut line = Line::default();
                line.spans.push(Span::from("/").fg(Color::Indexed(148)));
                let needle = Span::from(needle.as_str());
                line.spans.push(needle);
                let width = line.width() as u16;
                let searchbar = Paragraph::new(line);
                searchbar.render(Rect::new(0, status_line_y, width, 1), buf);
                cursor_position.x = width;
            }
            InputQueue::MovementCount(movement_count) => {
                let movement_count = movement_count.get();
                let mut line = Line::default();
                let mut span = Span::from(movement_count.to_string()).fg(Color::Indexed(250));
                if movement_count == u16::MAX {
                    span = span.fg(Color::Indexed(167));
                }
                line.spans.push(span);
                let width = line.width() as u16;
                let searchbar = Paragraph::new(line);
                searchbar.render(Rect::new(0, status_line_y, width, 1), buf);
                cursor_position.x = width;
            }
            InputQueue::CursorPositioningCommands => {
                let line = Line::from(Span::from("z").fg(Color::Indexed(32)));
                let width = line.width() as u16;
                let searchbar = Paragraph::new(line);
                searchbar.render(Rect::new(0, status_line_y, width, 1), buf);
                cursor_position.x = width;
            }
            InputQueue::Command(command) => {
                let mut line = Line::default();
                line.spans.push(Span::from(":").fg(Color::Indexed(148)));
                let needle = Span::from(command.as_str());
                line.spans.push(needle);
                let width = line.width() as u16;
                let searchbar = Paragraph::new(line);
                searchbar.render(Rect::new(0, status_line_y, width, 1), buf);
                cursor_position.x = width;
            }
        };
    }
    cursor_position
}

fn render_welcome(model: &Model, inner_area: Rect, buf: &mut Buffer) {
    let body = [
        "Welcome to the ULTIMATE terminal markdown viewer.",
        "Type :help for help and documentation.",
        "Press Q to quit.",
    ];

    let logo_size: Size = WELCOME_LOGO_SIZE.into();
    let logo_rows = logo_size.height;
    let logo_cols = logo_size.width;

    let w = logo_cols.max(50);
    let h = logo_rows + body.len() as u16;
    let x = inner_area.x + inner_area.width.saturating_sub(w) / 2;
    let y = inner_area.y + inner_area.height.saturating_sub(h) / 2;

    let logo_x = x + w.saturating_sub(logo_cols) / 2;
    let logo_area = Rect {
        x: logo_x,
        y,
        width: logo_cols,
        height: logo_rows,
    };
    if let Some(proto) = &model.root_image_proto {
        Image::new(proto).render(logo_area, buf);
    } else {
        let block = Block::bordered().border_type(BorderType::Rounded);

        let inner = block.inner(logo_area);
        block.render(logo_area, buf);

        let [_, text_area, _] = Layout::vertical([
            Constraint::Fill(1),
            Constraint::Length(2),
            Constraint::Fill(1),
        ])
        .areas(inner);

        Paragraph::new(vec![
            Line::from("MdFried"),
            Line::from("Markdown, deep fried!"),
        ])
        .alignment(Alignment::Center)
        .render(text_area, buf);
    }

    let text_area = Rect {
        x,
        y: y + logo_rows,
        width: w,
        height: body.len() as u16,
    };
    Paragraph::new(body.iter().map(|t| Line::from(*t)).collect::<Vec<_>>())
        .alignment(Alignment::Center)
        .render(text_area, buf);
}

fn section_lines(
    lines: &[(Line<'static>, Vec<LineExtra>)],
    buf: &mut Buffer,
    y: &mut i32,
    inner_area: Rect,
    model: &Model,
    selected_url: &Option<SourceContent>,
    section_id: usize,
) {
    let mut flat_index = 0;
    for (line_idx, (line, extras)) in lines.iter().enumerate() {
        const LINE_HEIGHT: u16 = 1;

        if *y < 0 {
            *y += LINE_HEIGHT as i32;
            continue; // skip this line.
        }

        // Positive Y
        let line_y = *y as u16;
        if line_y >= inner_area.height - 1 {
            break;
        }

        let p = Paragraph::new(line.clone());
        render_lines(p, LINE_HEIGHT, line_y, inner_area, buf);

        // Highlight all links that share the same URL as the selected link
        if let Cursor::Links(CursorPointer { .. }) = &model.cursor {
            if let Some(selected) = selected_url {
                for extra in extras.iter() {
                    if let LineExtra::Link {
                        source: url,
                        start,
                        end,
                        lines: lines_count,
                        ..
                    } = extra
                    {
                        if url.as_ptr() == selected.as_ptr() {
                            for (link_overlay, area) in link_overlays(
                                line,
                                *start,
                                *end,
                                lines_count,
                                line_idx,
                                lines,
                                inner_area,
                                line_y,
                            ) {
                                link_overlay.render(area, buf);
                            }
                            // TODO: Find out if positioning the cursor on the link
                            // would help with screen readers, or anything else in
                            // general.
                        }
                    }
                }
            }
        } else if let Cursor::Search(_, pointer) = &model.cursor {
            for (i, extra) in extras.iter().enumerate() {
                if let LineExtra::SearchMatch(start, end, text) = extra {
                    let x = inner_area.x + (*start as u16);
                    let width = *end as u16 - *start as u16;
                    let area = Rect::new(x, line_y, width, 1);
                    let mut search_highlight_overlay = Paragraph::new(text.clone());
                    search_highlight_overlay = if let Some(CursorPointer { id, index }) = pointer
                        && section_id == *id
                        && flat_index + i == *index
                    {
                        search_highlight_overlay
                            .fg(Color::Black)
                            .bg(Color::Indexed(197))
                    } else {
                        search_highlight_overlay
                            .fg(Color::Black)
                            .bg(Color::Indexed(148))
                    };
                    search_highlight_overlay.render(area, buf);
                }
            }
        }
        flat_index += extras.len();
        *y += LINE_HEIGHT as i32;
    }
}

fn render_lines<W: Widget>(widget: W, source_height: u16, y: u16, area: Rect, buf: &mut Buffer) {
    let mut widget_area = area;
    widget_area.y += y;
    widget_area.height = widget_area.height.min(source_height);
    widget.render(widget_area, buf);
}

#[expect(clippy::too_many_arguments)]
fn link_overlays<'a>(
    line: &Line<'a>,
    start: u16,
    end: u16,
    lines_count: &Option<usize>,
    line_idx: usize,
    lines: &[(Line<'a>, Vec<LineExtra>)],
    inner_area: Rect,
    line_y: u16,
) -> Vec<(Paragraph<'a>, Rect)> {
    let mut overlays = Vec::new();

    let max_line_end = inner_area.width;

    let start = if let Some(previous_lines_count) = lines_count
        && *previous_lines_count > 0
    {
        for previous_lines_idx in (0..*previous_lines_count).rev() {
            let (start, end) = if previous_lines_idx == previous_lines_count - 1 {
                (start, max_line_end)
            } else {
                (0, max_line_end)
            };

            let previous_line_y = if previous_lines_idx as u16 >= line_y {
                break;
            } else {
                line_y - (previous_lines_idx as u16 + 1)
            };

            let Some(previous_line) = line_idx
                .checked_sub(previous_lines_idx + 1)
                .and_then(|i| lines.get(i))
            else {
                log::error!("LineExtra::Link with multiline out of bounds");
                break;
            };
            let display_text = extract_line_content(&previous_line.0, start, end);
            let (link_overlay, width) = link_overlay_widget(start, end, display_text);
            let x = inner_area.x + start;
            let area = Rect::new(x, previous_line_y, width, 1);
            overlays.push((link_overlay, area));
        }
        0
    } else {
        start
    };

    if !(start == 0 && end == 0) {
        // Links may end at 0-0 if the closing bracket "]" is at the beginning of a line.
        // Just skip the overlay, although that line is the "anchor" for other purposes.
        let display_text = extract_line_content(line, start, end);
        let (link_overlay, width) = link_overlay_widget(start, end, display_text);
        let x = inner_area.x + start;
        let area = Rect::new(x, line_y, width, 1);
        overlays.push((link_overlay, area));
    }

    overlays
}

fn link_overlay_widget<'a>(start: u16, end: u16, display_text: Line<'a>) -> (Paragraph<'a>, u16) {
    let width = end - start;
    let link_overlay = Paragraph::new(display_text)
        .fg(Color::Indexed(15))
        .bg(Color::Indexed(32));
    (link_overlay, width)
}

/// Extract text content from a Line.
/// The start and end positions must be exactly at the boundaries of the spans.
fn extract_line_content<'a>(line: &Line<'a>, start: u16, end: u16) -> Line<'a> {
    debug_assert!(
        end > start,
        "extract_line_content expects start > end: {start}-{end}"
    );
    let mut pos: u16 = 0;
    let mut content = Line::default();
    for span in &line.spans {
        if pos >= start {
            let mut link_span = span.clone();
            link_span.style.fg = None;
            link_span.style.bg = None;
            content.push_span(link_span);
        }

        let span_width = span.content.width() as u16;
        pos += span_width;

        if pos >= end {
            break;
        }
    }
    content
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn simple_link_overlays() {
        let lines = vec![
            (
                Line::from(vec![Span::from("prefix "), Span::from("link desc")]),
                vec![],
            ),
            (
                Line::from(vec![Span::from("link cont"), Span::from(" suffix")]),
                vec![],
            ),
        ];
        let start = 7;
        let end = 9;
        let lines_count = Some(1);
        let line_idx = 1;
        let line = &lines[line_idx].0;
        let line_y = 1;
        let inner_area = Rect::new(10, 0, 100, 50);
        let overlays = link_overlays(
            line,
            start,
            end,
            &lines_count,
            line_idx,
            &lines,
            inner_area,
            line_y,
        );
        assert_eq!(
            overlays,
            vec![
                (
                    Paragraph::new(Line::from(Span::from("link desc")))
                        .fg(Color::Indexed(15))
                        .bg(Color::Indexed(32)),
                    Rect::new(17, 0, 93, 1)
                ),
                (
                    Paragraph::new(Line::from(Span::from("link cont")))
                        .fg(Color::Indexed(15))
                        .bg(Color::Indexed(32)),
                    Rect::new(10, 1, 9, 1)
                )
            ]
        );
    }
}
