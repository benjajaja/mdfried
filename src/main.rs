use std::{
    cell::RefCell,
    fs::File,
    io::{self, Read},
};

use font_loader::system_fonts;
use image::{GenericImage, ImageError, Rgb, RgbImage, Rgba, RgbaImage};
use ratatui::{
    crossterm::event::{self, KeyCode, KeyEventKind},
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Paragraph, Widget},
    DefaultTerminal, Frame,
};

use comrak::{
    arena_tree::{Node, NodeEdge},
    nodes::{Ast, NodeHeading, NodeValue},
};
use comrak::{parse_document, Arena, Options};
use ratatui_image::{picker::Picker, protocol::Protocol, Image, Resize};
use rusttype::{point, Font, Scale};

fn main() -> io::Result<()> {
    let mut terminal = ratatui::init();
    terminal.clear()?;
    let app_result = run(terminal);
    ratatui::restore();
    match app_result {
        Err(Error::Io(io_err)) => Err(io_err),
        Err(err) => Err(io::Error::new(io::ErrorKind::Other, format!("{err:?}"))),
        Ok(ok) => Ok(ok),
    }
}

struct Model<'a> {
    text: String,
    root: Box<&'a Node<'a, RefCell<Ast>>>,
    picker: Picker,
    font: Font<'a>,
}

fn run(mut terminal: DefaultTerminal) -> Result<(), Error> {
    let arena = Arena::new();

    //let md = read_file_to_str("/home/gipsy/o/ratatu-image/README.md")?;
    let text = read_file_to_str("./test.md")?;

    let root = parse_document(&arena, &text, &Options::default());

    let mut picker = Picker::from_termios()
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, format!("{err}")))?;
    picker.guess_protocol();

    let property = system_fonts::FontPropertyBuilder::new()
        .monospace()
        //.family(name)
        .family("ProFontWindows Nerd Font Mono")
        .build();
    let (font_data, _) =
        system_fonts::get(&property).ok_or("Could not get system fonts property")?;

    let font = Font::try_from_vec(font_data).ok_or(Error::NoFont)?;

    let mut model = Model {
        text,
        root: Box::new(root),
        picker,
        font,
    };

    loop {
        terminal.draw(|frame| view(&mut model, frame))?;

        if let event::Event::Key(key) = event::read()? {
            if key.kind == KeyEventKind::Press && key.code == KeyCode::Char('q') {
                return Ok(());
            }
        }
    }
}

fn view(model: &mut Model, f: &mut Frame) {
    let [left, right] =
        Layout::horizontal([Constraint::Fill(2), Constraint::Fill(1)]).areas(f.area());

    let mut debug = vec![];
    let mut lines = vec![];
    //let mut line: Option<Line> = None;
    let mut spans = vec![];
    let mut style = Style::new();
    let mut y = 0;
    //let mut span = None;

    for edge in model.root.traverse() {
        match edge {
            NodeEdge::Start(node) => match node.data.borrow().value {
                ref node_value => {
                    if let CookedModifier::Raw(modifier) = modifier(&node_value) {
                        style = style.add_modifier(modifier);
                    }
                }
            },
            NodeEdge::End(node) => {
                debug.push(Line::from(format!("End {:?}", node.data.borrow().value)));
                match node.data.borrow().value {
                    NodeValue::Text(ref literal) => {
                        let span = Span::from(literal.clone()).style(style);
                        spans.push(span);
                    }
                    NodeValue::Heading(ref tier) => {
                        let widget =
                            Header::new(&mut model.picker, &mut model.font, left.width, spans)
                                .unwrap();
                        let height = widget.proto.rect().height;
                        y = render_lines(widget, height, y, left, f);
                        lines = vec![];
                        spans = vec![];
                    }
                    NodeValue::Image(ref link) => {
                        let widget =
                            LinkImage::new(&mut model.picker, left.width, link.url.as_str());
                        let height = widget.proto.rect().height;
                        y = render_lines(widget, height, y, left, f);
                        lines = vec![];
                        spans = vec![];
                    }
                    NodeValue::Paragraph => {
                        lines.push(Line::from(spans));
                        lines.push(Line::default());
                        let text = Text::from(lines);
                        lines = vec![];
                        spans = vec![];
                        let height = text.height() as u16;
                        let p = Paragraph::new(text);
                        y = render_lines(p, height, y, left, f);
                    }
                    NodeValue::LineBreak | NodeValue::SoftBreak => {
                        lines.push(Line::from(spans));
                        let text = Text::from(lines);
                        lines = vec![];
                        spans = vec![];
                        let height = text.height() as u16;
                        let p = Paragraph::new(text);
                        y = render_lines(p, height, y, left, f);
                    }
                    _ => {
                        if let CookedModifier::Raw(modifier) = modifier(&node.data.borrow().value) {
                            style = style.remove_modifier(modifier);
                        }
                    }
                }
            }
        }
    }

    f.render_widget(Paragraph::new(debug).block(Block::bordered()), right);
}

struct Header {
    proto: Box<dyn Protocol>,
}

impl<'a> Header {
    fn new(
        picker: &mut Picker,
        font: &mut Font<'a>,
        width: u16,
        spans: Vec<Span>,
    ) -> Result<Header, Error> {
        let cell_height = 2;
        let img_width = (width * picker.font_size.0) as u32;
        let img_height = (cell_height * picker.font_size.1) as u32;
        let img: RgbImage = RgbImage::from_pixel(img_width, img_height, Rgb([0, 0, 0]));
        let mut dyn_img = image::DynamicImage::ImageRgb8(img);

        let s: String = spans.iter().map(|s| s.to_string()).collect();
        let scale = Scale::uniform(64f32);
        let v_metrics = font.v_metrics(scale);
        let glyphs: Vec<_> = font
            .layout(&s, scale, point(0.0, 0.0 + v_metrics.ascent))
            .collect();

        let max_x = img_width as u32;
        let max_y = img_height as u32;
        for glyph in glyphs {
            if let Some(bounding_box) = glyph.pixel_bounding_box() {
                let mut outside = false;
                let bb_x = bounding_box.min.x as u32;
                let bb_y = bounding_box.min.y as u32;
                glyph.draw(|x, y, v| {
                    let p_x = bb_x + x as u32;
                    let p_y = bb_y + y as u32;
                    if p_x > max_x {
                        outside = true;
                    } else if p_y > max_y {
                        outside = true;
                    } else {
                        let u8v = (255.0 * v) as u8;
                        let pixel = Rgba([u8v, u8v, u8v, 255]);
                        dyn_img.put_pixel(p_x, p_y, pixel);
                    }
                });
                if outside {
                    break;
                }
            }
        }

        let proto = picker
            .new_protocol(
                dyn_img,
                Rect::new(0, 0, width, cell_height),
                Resize::Fit(None),
            )
            .unwrap();
        Ok(Header { proto })
    }
}

impl Widget for Header {
    fn render(self, area: Rect, buf: &mut ratatui::prelude::Buffer)
    where
        Self: Sized,
    {
        let image = Image::new(self.proto.as_ref());
        image.render(area, buf);
    }
}

struct LinkImage {
    proto: Box<dyn Protocol>,
}

impl LinkImage {
    fn new(picker: &mut Picker, width: u16, link: &str) -> LinkImage {
        let dyn_img = image::io::Reader::open(link).unwrap().decode().unwrap();

        let proto = picker
            .new_protocol(dyn_img, Rect::new(0, 0, width, width), Resize::Fit(None))
            .unwrap();
        LinkImage { proto }
    }
}

impl Widget for LinkImage {
    fn render(self, area: Rect, buf: &mut ratatui::prelude::Buffer)
    where
        Self: Sized,
    {
        let image = Image::new(self.proto.as_ref());
        image.render(area, buf);
    }
}

#[derive(Debug)]
enum Error {
    Io(io::Error),
    Image(image::ImageError),
    Msg(String),
    NoFont,
}

impl From<&str> for Error {
    fn from(value: &str) -> Self {
        Self::Msg(value.to_string())
    }
}

impl From<ImageError> for Error {
    fn from(value: image::ImageError) -> Self {
        Self::Image(value)
    }
}

impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

fn render_lines<W: Widget>(widget: W, height: u16, y: u16, area: Rect, f: &mut Frame) -> u16 {
    //let text = Text::from(lines);
    //let next_y = y + height;
    if y < area.height && area.height - y > height {
        //let p = Paragraph::new(text);
        let mut area = area.clone();
        area.y += y;
        area.height = height;
        f.render_widget(widget, area);
    }
    y + height
}

fn read_file_to_str(path: &str) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;
    Ok(contents)
}

enum CookedModifier {
    None,
    Raw(Modifier),
}

fn modifier(node_value: &NodeValue) -> CookedModifier {
    match node_value {
        NodeValue::Strong => CookedModifier::Raw(Modifier::BOLD),
        NodeValue::Emph => CookedModifier::Raw(Modifier::ITALIC),
        NodeValue::Strikethrough => CookedModifier::Raw(Modifier::CROSSED_OUT),
        _ => CookedModifier::None,
    }
}
