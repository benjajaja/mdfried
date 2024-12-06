use std::{
    cell::RefCell,
    fs::File,
    io::{self, Read},
    path::{Path, PathBuf},
};

use clap::{arg, command, value_parser};
use config::Config;
use confy::ConfyError;
use font_loader::system_fonts;
use image::{GenericImage, ImageError, Pixel, Rgb, RgbImage, Rgba};
use ratatui::{
    crossterm::event::{self, KeyCode, KeyEventKind},
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Paragraph, Widget},
    DefaultTerminal, Frame,
};

use comrak::{
    arena_tree::{Node, NodeEdge},
    nodes::{Ast, NodeValue},
    ExtensionOptions,
};
use comrak::{parse_document, Arena, Options};
use ratatui_image::{picker::Picker, protocol::Protocol, Image, Resize};
use rusttype::{point, Font, Scale};
mod config;

fn main() -> io::Result<()> {
    let matches = command!() // requires `cargo` feature
        .arg(arg!(<path> "The input markdown file path").value_parser(value_parser!(PathBuf)))
        .get_matches();

    let path = matches.get_one::<PathBuf>("path").expect("required input");

    let config: Config = confy::load("mdcooked", None).map_err(map_to_io_error)?;

    let text = read_file_to_str(path.to_str().unwrap())?;

    let basepath = path.parent();

    let arena = Box::new(Arena::new());
    let model =
        Model::new(&arena, &text, &config, basepath).map_err::<io::Error, _>(Error::into)?;

    let mut terminal = ratatui::init();
    terminal.clear()?;

    let app_result = run(terminal, model);
    ratatui::restore();
    app_result.map_err(Error::into)
}

fn map_to_io_error<I>(err: I) -> io::Error
where
    I: Into<Error>,
{
    err.into().into()
}

struct Model<'a> {
    bg: [u8; 3],
    scroll: i64,
    root: Box<&'a Node<'a, RefCell<Ast>>>,
    picker: Picker,
    font: Font<'a>,
    basepath: Option<&'a Path>,
}

impl<'a> Model<'a> {
    fn new(
        arena: &'a Arena<Node<'a, RefCell<Ast>>>,
        text: &str,
        config: &Config,
        basepath: Option<&'a Path>,
    ) -> Result<Self, Error> {
        let mut ext_options = ExtensionOptions::default();
        ext_options.strikethrough = true;
        let root = Box::new(parse_document(
            &arena,
            text,
            &Options {
                extension: ext_options,
                ..Default::default()
            },
        ));

        let mut picker = Picker::from_query_stdio()
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, format!("{err}")))?;

        let mut fp_builder = system_fonts::FontPropertyBuilder::new();
        if let Some(ref font_family) = config.font_family {
            fp_builder = fp_builder.family(font_family);
        }
        let property = fp_builder.build();

        let (font_data, _) =
            system_fonts::get(&property).ok_or("Could not get system fonts property")?;

        let font = Font::try_from_vec(font_data).ok_or(Error::NoFont)?;

        let bg = [0, 0, 50];
        picker.set_background_color(Some(image::Rgb(bg)));

        Ok(Model {
            // text,
            bg,
            scroll: 0,
            root,
            picker,
            font,
            basepath,
        })
    }
}

fn run(mut terminal: DefaultTerminal, mut model: Model) -> Result<(), Error> {
    loop {
        terminal.draw(|frame| view(&mut model, frame))?;

        if let event::Event::Key(key) = event::read()? {
            if key.kind == KeyEventKind::Press {
                match key.code {
                    KeyCode::Char('q') => {
                        return Ok(());
                    }
                    KeyCode::Char('j') => {
                        model.scroll += 1;
                    }
                    KeyCode::Char('k') => {
                        model.scroll -= 1;
                    }
                    _ => {}
                }
            }
        }
    }
}

fn view(model: &mut Model, frame: &mut Frame) {
    let area = frame.area();
    let block = Block::default().style(Style::default().bg(Color::Rgb(
        model.bg[0],
        model.bg[1],
        model.bg[2],
    )));
    frame.render_widget(block, area);

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
                        let widget = Header::new(
                            &mut model.picker,
                            &mut model.font,
                            model.bg,
                            area.width,
                            spans,
                            tier.level,
                        )
                        .unwrap();
                        let height = widget.height;
                        y = render_lines(widget, height, y, area, frame);
                        lines = vec![];
                        spans = vec![];
                    }
                    NodeValue::Image(ref link) => {
                        match LinkImage::new(
                            &mut model.picker,
                            area.width,
                            model.basepath,
                            link.url.as_str(),
                        ) {
                            Ok(widget) => {
                                let height = widget.height;
                                y = render_lines(widget, height, y, area, frame);
                                lines = vec![];
                                spans = vec![];
                            }
                            Err(err) => {
                                let text = Text::from(format!("[Image error: {err:?}]"));
                                let height = text.height() as u16;
                                let p = Paragraph::new(text);
                                y = render_lines(p, height, y, area, frame);
                            }
                        }
                    }
                    NodeValue::Paragraph => {
                        lines.push(Line::from(spans));
                        lines.push(Line::default());
                        let text = Text::from(lines);
                        lines = vec![];
                        spans = vec![];
                        let height = text.height() as u16;
                        let p = Paragraph::new(text);
                        y = render_lines(p, height, y, area, frame);
                    }
                    NodeValue::LineBreak | NodeValue::SoftBreak => {
                        lines.push(Line::from(spans));
                        let text = Text::from(lines);
                        lines = vec![];
                        spans = vec![];
                        let height = text.height() as u16;
                        let p = Paragraph::new(text);
                        y = render_lines(p, height, y, area, frame);
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
}

struct Header {
    proto: Protocol,
    height: u16,
}

impl<'a> Header {
    fn new(
        picker: &mut Picker,
        font: &mut Font<'a>,
        bg: [u8; 3],
        width: u16,
        spans: Vec<Span>,
        tier: u8,
    ) -> Result<Header, Error> {
        let cell_height = 2;
        let (font_width, font_height) = picker.font_size();
        let img_width = (width * font_width) as u32;
        let img_height = (cell_height * font_height) as u32;
        let img: RgbImage = RgbImage::from_pixel(img_width, img_height, Rgb(bg));
        let mut dyn_img = image::DynamicImage::ImageRgb8(img);

        //let mut spans = spans.clone();
        //spans.push(Span::raw(format!("#{tier}")));
        let s: String = spans.iter().map(|s| s.to_string()).collect();
        let tier_scale = ((12 - tier) as f32) / 12.0f32;
        let scale = Scale::uniform((font_height * cell_height) as f32 * tier_scale);
        let v_metrics = font.v_metrics(scale);
        let glyphs: Vec<_> = font
            .layout(&s, scale, point(0.0, 0.0 + v_metrics.ascent))
            .collect();

        let max_x = img_width as i32;
        let max_y = img_height as i32;
        for glyph in glyphs {
            if let Some(bounding_box) = glyph.pixel_bounding_box() {
                let mut outside = false;
                let bb_x = bounding_box.min.x;
                let bb_y = bounding_box.min.y;
                glyph.draw(|x, y, v| {
                    let p_x = bb_x + (x as i32);
                    let p_y = bb_y + (y as i32);
                    if p_x >= max_x {
                        outside = true;
                    } else if p_y >= max_y {
                        outside = true;
                    } else {
                        let u8v = (255.0 * v) as u8;
                        let mut pixel = Rgba([bg[0], bg[1], bg[2], 255]);
                        pixel.blend(&Rgba([u8v, u8v, u8v, u8v]));
                        dyn_img.put_pixel(p_x as u32, p_y as u32, pixel);
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
        Ok(Header {
            proto,
            height: cell_height,
        })
    }
}

impl Widget for Header {
    fn render(self, area: Rect, buf: &mut ratatui::prelude::Buffer)
    where
        Self: Sized,
    {
        let image = Image::new(&self.proto);
        image.render(area, buf);
    }
}

struct LinkImage {
    proto: Protocol,
    height: u16,
}

impl LinkImage {
    fn new(
        picker: &mut Picker,
        width: u16,
        basepath: Option<&Path>,
        link: &str,
    ) -> Result<LinkImage, Error> {
        let link: String = if basepath.is_some() && link.starts_with("./") {
            let joined = basepath.unwrap().join(link);
            joined.to_str().unwrap_or(link).to_owned()
        } else {
            link.to_string()
        };
        let dyn_img = image::ImageReader::open(link)?.decode()?;
        let height: u16 = 10;

        let proto = picker
            .new_protocol(dyn_img, Rect::new(0, 0, width, height), Resize::Fit(None))
            .unwrap();
        Ok(LinkImage { proto, height })
    }
}

impl Widget for LinkImage {
    fn render(self, area: Rect, buf: &mut ratatui::prelude::Buffer)
    where
        Self: Sized,
    {
        let image = Image::new(&self.proto);
        image.render(area, buf);
    }
}

#[derive(Debug)]
#[allow(dead_code)]
enum Error {
    Cli(clap::error::Error),
    Config(ConfyError),
    Io(io::Error),
    Image(image::ImageError),
    Msg(String),
    NoFont,
}

impl Into<io::Error> for Error {
    fn into(self) -> io::Error {
        match self {
            Error::Io(io_err) => io_err,
            err => io::Error::new(io::ErrorKind::Other, format!("{err:?}")),
        }
    }
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

impl From<ConfyError> for Error {
    fn from(value: ConfyError) -> Self {
        Self::Config(value)
    }
}

impl From<clap::error::Error> for Error {
    fn from(value: clap::error::Error) -> Self {
        Self::Cli(value)
    }
}

fn render_lines<W: Widget>(widget: W, height: u16, y: u16, area: Rect, f: &mut Frame) -> u16 {
    if y < area.height && area.height - y > height {
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
