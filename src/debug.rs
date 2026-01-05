use std::io::Write;

use flexi_logger::{DeferredNow, FileSpec, FlexiLoggerError, Logger};
use log::Record;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::Block,
};
use textwrap::{Options, wrap};

pub fn ui_logger(log_to_file: bool) -> Result<flexi_logger::LoggerHandle, FlexiLoggerError> {
    if log_to_file {
        Logger::try_with_env_or_str("info")?
            .log_to_file(FileSpec::default())
            .start()
    } else {
        Logger::try_with_env_or_str("info")?
            .log_to_buffer(10000, Some(markdown_format))
            .start()
    }
}

fn markdown_format(
    w: &mut dyn Write,
    _now: &mut DeferredNow,
    record: &Record,
) -> Result<(), std::io::Error> {
    write!(
        w,
        "**{}** *{}* `{}`",
        record.level(),
        record.module_path().unwrap_or("<unknown>"),
        record.args()
    )
}

pub fn render_snapshot(snapshot: &flexi_logger::Snapshot, frame: &mut Frame) -> Rect {
    let debug_block = Block::bordered().title("logs");

    let frame_area = frame.area();
    let mut half_area_left = frame_area;
    half_area_left.width /= 2;

    let mut half_area_right = half_area_left;
    half_area_right.x = frame_area.width / 2;

    let inner_area = debug_block.inner(half_area_right);
    frame.render_widget(debug_block, half_area_right);

    let options = Options::new(inner_area.width as usize).break_words(true);

    let mut output_lines: Vec<Line> = Vec::new();
    for log_line in snapshot.text.lines() {
        let line = parse_log_line(log_line);
        let plain_text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        let wrapped = wrap(&plain_text, &options);

        if wrapped.len() == 1 {
            output_lines.push(line);
        } else {
            // For wrapped lines, just use plain text with level color on first line
            let level_color = line.spans.first().map(|s| s.style);
            for (i, part) in wrapped.iter().enumerate() {
                let style = if i == 0 {
                    level_color.unwrap_or_default()
                } else {
                    Style::default()
                };
                output_lines.push(Line::from(Span::styled(part.to_string(), style)));
            }
        }
    }

    for (i, line) in output_lines.into_iter().rev().enumerate() {
        if i as u16 >= inner_area.height {
            break;
        }
        let rect = Rect::new(
            inner_area.x,
            inner_area.height - i as u16,
            inner_area.width,
            1,
        );
        frame.render_widget(line, rect);
    }
    half_area_left
}

/// Parse a log line in format: **LEVEL** *module* `message`
#[expect(clippy::string_slice)] // Searching for ASCII delimiters guarantees valid UTF-8 boundaries.
fn parse_log_line(line: &str) -> Line<'static> {
    let mut spans = Vec::new();
    let mut remaining = line;

    // Parse **LEVEL**
    if let Some(rest) = remaining.strip_prefix("**") {
        if let Some(end) = rest.find("**") {
            let level = &rest[..end];
            let color = match level {
                "DEBUG" => Color::LightBlue,
                "INFO" => Color::Green,
                "WARN" => Color::Yellow,
                "ERROR" => Color::Red,
                _ => Color::Yellow,
            };
            spans.push(Span::styled(level.to_owned(), Style::default().fg(color)));
            remaining = &rest[end + 2..];
        }
    }

    // Parse *module*
    if let Some(rest) = remaining.strip_prefix(" *") {
        if let Some(end) = rest.find('*') {
            let module = &rest[..end];
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                module.to_owned(),
                Style::default().fg(Color::Indexed(96)),
            ));
            remaining = &rest[end + 1..];
        }
    }

    // Parse `message`
    if let Some(rest) = remaining.strip_prefix(" `") {
        if let Some(end) = rest.find('`') {
            let message = &rest[..end];
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                message.to_owned(),
                Style::default()
                    .fg(Color::Indexed(203))
                    .bg(Color::Indexed(236)),
            ));
            remaining = &rest[end + 1..];
        }
    }

    // Any remaining text
    if !remaining.is_empty() {
        spans.push(Span::raw(remaining.to_owned()));
    }

    // Fallback if parsing failed
    if spans.is_empty() {
        spans.push(Span::raw(line.to_owned()));
    }

    Line::from(spans)
}
