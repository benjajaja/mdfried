use std::io::Write;

use flexi_logger::{DeferredNow, FlexiLoggerError, Logger};
use log::Record;
use ratatui::{Frame, crossterm::style::Color, layout::Rect, widgets::Block};
use ratskin::RatSkin;

pub fn ui_logger() -> Result<flexi_logger::LoggerHandle, FlexiLoggerError> {
    Logger::try_with_env_or_str("info")?
        .log_to_buffer(10000, Some(markdown_format))
        .start()
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

pub(crate) fn render_snapshot(snapshot: &flexi_logger::Snapshot, frame: &mut Frame) -> Rect {
    let debug_block = Block::bordered().title("logs");

    let frame_area = frame.area();
    let mut half_area_left = frame_area;
    half_area_left.width /= 2;

    let mut half_area_right = half_area_left;
    half_area_right.x = frame_area.width / 2;

    let inner_area = debug_block.inner(half_area_right);
    frame.render_widget(debug_block, half_area_right);

    // We just leverage ratskin here for the text wrapping.
    let madtext = RatSkin::parse_text(&snapshot.text);
    let mut skin = RatSkin::default();
    skin.skin.bold.set_fg(Color::AnsiValue(220));
    skin.skin.italic.set_fg(Color::AnsiValue(96));
    skin.skin.inline_code.set_fg(Color::AnsiValue(203));
    skin.skin.inline_code.set_bg(Color::AnsiValue(236));

    let lines = skin.parse(madtext, inner_area.width);
    for (i, mut line) in lines.into_iter().rev().enumerate() {
        if i as u16 >= inner_area.height {
            break;
        }
        if let Some(span) = line.spans.get_mut(0) {
            if let Some(color) = match span.content.to_string().as_str() {
                "DEBUG" => Some(ratatui::prelude::Color::LightBlue),
                "INFO" => Some(ratatui::prelude::Color::Green),
                "WARN" => Some(ratatui::prelude::Color::Yellow),
                "ERROR" => Some(ratatui::prelude::Color::Red),
                _ => None,
            } {
                span.style = span.style.fg(color);
            }
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
