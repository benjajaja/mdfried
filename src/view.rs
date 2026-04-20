use unicode_width::UnicodeWidthStr as _;

use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Stylize as _},
    text::{Line, Span},
    widgets::{Block, Paragraph, Widget},
};

use mdfrier::Mapper as _;
use ratatui_image::Image;

use crate::{
    big_text::BigText,
    cursor::{Cursor, CursorPointer},
    document::{LineExtra, SectionContent},
    model::{InputQueue, Model},
};

pub fn view(model: &Model, frame: &mut Frame) {
    let frame_area = frame.area();
    let mut block = Block::new();
    let padding = model.block_padding(frame_area);
    block = block.padding(padding);

    let inner_area = if model.log_snapshot.is_some() {
        let mut half_area_left = frame_area;
        half_area_left.width /= 2;
        let mut fixed_padding = padding;
        fixed_padding.right = 0;
        block = block.padding(fixed_padding);
        block.inner(half_area_left)
    } else {
        block.inner(frame_area)
    };

    frame.render_widget(block, frame_area);

    let mut cursor_positioned = None;

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
            SectionContent::Lines(lines) => {
                let mut flat_index = 0;
                for (line, extras) in lines.iter() {
                    const LINE_HEIGHT: u16 = 1;

                    if y < 0 {
                        y += LINE_HEIGHT as i32;
                        continue; // skip this line.
                    }

                    // Positive Y
                    let line_y = y as u16;
                    if line_y >= inner_area.height - 1 {
                        break;
                    }

                    let p = Paragraph::new(line.clone());
                    render_lines(p, LINE_HEIGHT, line_y, inner_area, frame);

                    // Highlight all links that share the same URL as the selected link
                    if let Cursor::Links(CursorPointer { id, index }) = &model.cursor {
                        if let Some(selected) = &selected_url {
                            for (i, extra) in extras.iter().enumerate() {
                                if let LineExtra::Link(url, start, end) = extra {
                                    if url.as_ptr() == selected.as_ptr() {
                                        let x = frame_area.x + padding.left + *start;
                                        let width = end - start;
                                        let area = Rect::new(x, line_y, width, 1);
                                        // Highlight with original content - source_content is only for grouping/opening
                                        let display_text = extract_line_content(line, *start);
                                        let link_overlay_widget = Paragraph::new(display_text)
                                            .fg(Color::Indexed(15))
                                            .bg(Color::Indexed(32));
                                        frame.render_widget(link_overlay_widget, area);

                                        // Position cursor on the actual selected link
                                        if *id == section.id && *index == flat_index + i {
                                            cursor_positioned = Some((x, line_y));
                                        }
                                    }
                                }
                            }
                        }
                    } else if let Cursor::Search(_, pointer) = &model.cursor {
                        for (i, extra) in extras.iter().enumerate() {
                            if let LineExtra::SearchMatch(start, end, text) = extra {
                                let x = frame_area.x + padding.left + (*start as u16);
                                let width = *end as u16 - *start as u16;
                                let area = Rect::new(x, line_y, width, 1);
                                let mut link_overlay_widget = Paragraph::new(text.clone());
                                link_overlay_widget = if let Some(CursorPointer { id, index }) =
                                    pointer
                                    && section.id == *id
                                    && flat_index + i == *index
                                {
                                    link_overlay_widget.fg(Color::Black).bg(Color::Indexed(197))
                                } else {
                                    link_overlay_widget.fg(Color::Black).bg(Color::Indexed(148))
                                };
                                frame.render_widget(link_overlay_widget, area);
                                cursor_positioned = Some((x, line_y));
                            }
                        }
                    }
                    flat_index += extras.len();
                    y += LINE_HEIGHT as i32;
                }
            }
            SectionContent::Image(_, proto) => {
                // TODO: kitty can actually render partially, but this should probably be improved
                // on ratatui-image, so that we can also render partially at the top of the frame.
                let can_render = (y as u16) < inner_area.bottom() - proto.area().height;
                if y >= 0 && can_render {
                    let img = Image::new(proto);
                    render_lines(img, section.height, y as u16, inner_area, frame);
                }
                y += proto.area().height as i32;
            }
            SectionContent::ImagePlaceholder(_, lines) => {
                for (line, _extras) in lines.iter() {
                    if y < 0 {
                        y += 1;
                        continue; // skip this line.
                    }
                    let p = Paragraph::new(line.clone());
                    render_lines(p, 1, y as u16, inner_area, frame);
                    y += 1;
                }
            }
            SectionContent::Header(text, tier, proto) => {
                // Only render headers if fully in view
                if y >= 0 && (y as u16) < inner_area.bottom() - 2 {
                    if let Some(proto) = proto {
                        let img = Image::new(proto);
                        render_lines(img, section.height, y as u16, inner_area, frame);
                    } else {
                        let big_text = BigText::new(text, *tier, model.theme().header_color);
                        render_lines(big_text, 2, y as u16, inner_area, frame);
                    }
                }
                y += 2;
            }
            SectionContent::HeaderPlaceholder(_, _, lines) => {
                for (line, _) in lines.iter() {
                    if y < 0 {
                        y += 1;
                        continue; // skip this line.
                    }
                    let p = Paragraph::new(line.clone());
                    render_lines(p, 1, y as u16, inner_area, frame);
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

    match &model.input_queue {
        InputQueue::None => match &model.cursor {
            Cursor::None => frame.set_cursor_position((0, frame_area.height - 1)),
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
                frame.render_widget(searchbar, Rect::new(0, frame_area.height - 1, width, 1));
                if cursor_positioned.is_none() {
                    frame.set_cursor_position((0, frame_area.height - 1));
                }
            }
            Cursor::Search(needle, _) => {
                let mut line = Line::default();
                line.spans.push(Span::from("/").fg(Color::Indexed(148)));
                let needle = Span::from(needle.clone()).fg(Color::Indexed(148));
                line.spans.push(needle);
                let width = line.width() as u16;
                let searchbar = Paragraph::new(line);
                frame.render_widget(searchbar, Rect::new(0, frame_area.height - 1, width, 1));
                frame.set_cursor_position((0, frame_area.height - 1));
            }
        },
        InputQueue::Search(needle) => {
            let mut line = Line::default();
            line.spans.push(Span::from("/").fg(Color::Indexed(148)));
            let needle = Span::from(needle.clone());
            line.spans.push(needle);
            let width = line.width() as u16;
            let searchbar = Paragraph::new(line);
            frame.render_widget(searchbar, Rect::new(0, frame_area.height - 1, width, 1));
            frame.set_cursor_position((width, frame_area.height - 1));
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
            frame.render_widget(searchbar, Rect::new(0, frame_area.height - 1, width, 1));
            frame.set_cursor_position((width, frame_area.height - 1));
        }
    }
}

fn render_lines<W: Widget>(widget: W, source_height: u16, y: u16, area: Rect, f: &mut Frame) {
    let mut widget_area = area;
    widget_area.y += y;
    widget_area.height = widget_area.height.min(source_height);
    f.render_widget(widget, widget_area);
}

/// Extract text content from a Line at a given character position.
/// Each LineExtra::Link corresponds to exactly one span, so we find the span starting at `start`.
fn extract_line_content(line: &Line, start: u16) -> String {
    let mut pos: u16 = 0;
    for span in &line.spans {
        if pos == start {
            return span.content.to_string();
        }
        let span_width = span.content.width() as u16;
        pos += span_width;
        if pos > start {
            // We passed the start position without finding an exact match
            break;
        }
    }
    String::new()
}
