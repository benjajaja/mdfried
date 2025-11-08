use ratatui::{Frame, layout::Rect, widgets::Block};
use ratskin::RatSkin;

pub(crate) fn render_snapshot(snapshot: &flexi_logger::Snapshot, frame: &mut Frame) -> Rect {
    let debug_block = Block::bordered().title("logs");

    let frame_area = frame.area();
    let mut half_area_left = frame_area;
    half_area_left.width /= 2;

    let mut half_area_right = half_area_left;
    half_area_right.x = frame_area.width / 2;

    let inner_area = debug_block.inner(half_area_right);
    frame.render_widget(debug_block, half_area_right);

    // We just leverage ratskin here for the text wrapping, so we convert logs into markdown first.
    let lines = snapshot.text.lines().map(|line| format!("`{line}`"));
    let text = lines.collect::<Vec<String>>().join("\n");

    let madtext = RatSkin::parse_text(&text);
    let skin = RatSkin::default();
    let lines = skin.parse(madtext, inner_area.width);
    for (i, line) in lines.iter().rev().enumerate() {
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
