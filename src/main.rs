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
use image::ImageError;
use markdown::traverse;
use ratatui::{
    crossterm::event::{self, KeyCode, KeyEventKind},
    layout::Rect,
    style::{Color, Style},
    widgets::{Block, Paragraph, Widget},
    DefaultTerminal, Frame,
};

use comrak::{arena_tree::Node, nodes::Ast, ExtensionOptions};
use comrak::{parse_document, Arena, Options};
use ratatui_image::{
    picker::{Picker, ProtocolType},
    Image,
};
use rusttype::Font;
use widget_sources::{WidgetSource, WidgetSourceData};
mod config;
mod markdown;
mod widget_sources;

fn main() -> io::Result<()> {
    let matches = command!() // requires `cargo` feature
        .arg(arg!(<path> "The input markdown file path").value_parser(value_parser!(PathBuf)))
        .arg(arg!(-d --deep "Extra deep fried images").value_parser(value_parser!(bool)))
        .get_matches();

    let path = matches.get_one::<PathBuf>("path").expect("required input");
    let text = read_file_to_str(path.to_str().unwrap())?;
    let basepath = path.parent();

    let config: Config = confy::load("mdcooked", None).map_err(map_to_io_error)?;

    let arena = Box::new(Arena::new());
    let model = Model::new(
        &arena,
        &text,
        &config,
        basepath,
        *matches.get_one("deep").unwrap_or(&false),
    )
    .map_err::<io::Error, _>(Error::into)?;

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
    bg: Option<[u8; 4]>,
    scroll: i16,
    root: &'a Node<'a, RefCell<Ast>>,
    picker: Picker,
    font: Font<'a>,
    basepath: Option<&'a Path>,
    sources: Vec<WidgetSource<'a>>,
    deep_fry: bool,
}

impl<'a> Model<'a> {
    fn new(
        arena: &'a Arena<Node<'a, RefCell<Ast>>>,
        text: &str,
        config: &Config,
        basepath: Option<&'a Path>,
        deep_fry: bool,
    ) -> Result<Self, Error> {
        let mut ext_options = ExtensionOptions::default();
        ext_options.strikethrough = true;
        let root = Box::new(parse_document(
            arena,
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

        let bg = match picker.protocol_type() {
            ProtocolType::Sixel => Some([0, 0, 0, 255]),
            _ => None,
        };
        picker.set_background_color(bg.map(image::Rgba));

        Ok(Model {
            bg,
            scroll: 0,
            root: &root,
            picker,
            font,
            basepath,
            sources: vec![],
            deep_fry,
        })
    }
}

fn run(mut terminal: DefaultTerminal, mut model: Model) -> Result<(), Error> {
    loop {
        terminal.draw(|frame| view(&mut model, frame))?;

        match event::read()? {
            event::Event::Key(key) => {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('q') => {
                            return Ok(());
                        }
                        KeyCode::Char('j') => {
                            model.scroll -= 1;
                        }
                        KeyCode::Char('k') => {
                            if model.scroll < 0 {
                                model.scroll += 1;
                            }
                        }
                        _ => {}
                    }
                }
            }
            event::Event::Resize(_, _) => {
                // TODO: do it now based on screen size?
                // traverse(model, area.width);
                model.sources = vec![];
            }
            _ => {}
        }
    }
}

fn view(model: &mut Model, frame: &mut Frame) {
    let area = frame.area();
    let mut block = Block::bordered();
    if let Some(bg) = model.bg {
        block = block.style(Style::default().bg(Color::Rgb(bg[0], bg[1], bg[2])));
    }
    let inner_area = block.inner(area);
    frame.render_widget(block, area);

    if model.sources.is_empty() {
        model.sources = traverse(model, inner_area.width);
    }

    let mut y = model.scroll;
    // eprintln!("view {y}");
    for source in &mut model.sources {
        match &mut source.source {
            WidgetSourceData::Text(text) => {
                let p = Paragraph::new(text.clone());
                y = render_lines(p, source.height, y, inner_area, frame);
            }
            WidgetSourceData::Image(proto) => {
                let img = Image::new(proto);
                y = render_lines(img, source.height, y, inner_area, frame);
            }
        }
    }
}

fn render_lines<W: Widget>(widget: W, height: u16, scroll: i16, area: Rect, f: &mut Frame) -> i16 {
    if scroll >= 0 {
        let y = scroll as u16;
        if y <= area.height && area.height - y >= height {
            let mut area = area;
            area.y += y;
            area.height = height;
            f.render_widget(widget, area);
        }
    }
    scroll + (height as i16)
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

impl From<Error> for io::Error {
    fn from(value: Error) -> Self {
        match value {
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

fn read_file_to_str(path: &str) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;
    Ok(contents)
}
