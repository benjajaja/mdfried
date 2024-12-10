use std::{
    fs::File,
    io::{self, stdout, Read},
    os::fd::IntoRawFd,
    path::{Path, PathBuf},
    process,
    sync::{
        mpsc::{self, SendError},
        Arc, RwLock,
    },
    thread,
};

use clap::{arg, command, value_parser};
use config::Config;
use confy::ConfyError;
use crossterm::{
    event::KeyModifiers,
    execute,
    terminal::{disable_raw_mode, LeaveAlternateScreen},
    tty::IsTty,
};
use font_loader::system_fonts;
use image::ImageError;
use ratatui::{
    crossterm::event::{self, KeyCode, KeyEventKind},
    layout::Rect,
    style::{Color, Style},
    widgets::{Block, Paragraph, Widget},
    DefaultTerminal, Frame,
};

use comrak::Arena;
use comrak::ExtensionOptions;
use ratatui_image::{
    picker::{Picker, ProtocolType},
    Image,
};
use rusttype::Font;
use serde::{Deserialize, Serialize};
use widget_sources::{WidgetSource, WidgetSourceData};

use crate::{fontpicker::set_up_font, markdown::Parser};
mod config;
mod fontpicker;
mod markdown;
mod widget_sources;

const OK_END: &str = " ok.";

const CONFIG: (&str, Option<&str>) = ("mdfried", Some("config"));

fn main() -> io::Result<()> {
    std::env::set_var("FONTCONFIG_LOG_LEVEL", "silent");

    let matches = command!() // requires `cargo` feature
        .arg(arg!([path] "The input markdown file path").value_parser(value_parser!(PathBuf)))
        .arg(arg!(-s --setup "Force font setup").value_parser(value_parser!(bool)))
        .arg(arg!(-d --deep "Extra deep fried images").value_parser(value_parser!(bool)))
        .get_matches();

    let path = matches.get_one::<PathBuf>("path");

    let (text, basepath) = match path {
        Some(path) => (
            read_file_to_str(path.to_str().unwrap())?,
            path.parent().map(Path::to_path_buf),
        ),
        None if !io::stdin().is_tty() => {
            let mut text = String::new();
            print!("Reading stdin...");
            let _ = io::stdin().read_to_string(&mut text)?;
            println!("{OK_END}");
            (text, None)
        }
        None => {
            return Err(Error::Msg(
                "no input file path provided, and no stdin pipe detected".into(),
            )
            .into())
        }
    };

    if text.is_empty() {
        return Err(Error::Msg("input is empty".into()).into());
    }

    let config: Config = confy::load(CONFIG.0, CONFIG.1).map_err(map_to_io_error)?;

    if !io::stdin().is_tty() {
        print!("Setting stdin to /dev/tty...");
        // Close the current stdin so that ratatui-image can read stuff from tty stdin.
        unsafe {
            // Attempt to open /dev/tty which will give us a new stdin
            let tty = File::open("/dev/tty")?;

            // Get the file descriptor for /dev/tty
            let tty_fd = tty.into_raw_fd();

            // Duplicate the tty file descriptor to stdin (file descriptor 0)
            libc::dup2(tty_fd, libc::STDIN_FILENO);

            // Close the original tty file descriptor
            libc::close(tty_fd);
        }
        println!("{OK_END}");
    }

    match Model::new(
        &text,
        path.cloned(),
        config,
        basepath,
        *matches.get_one("deep").unwrap_or(&false),
        *matches.get_one("setup").unwrap_or(&false),
    ) {
        Err(err) => match err {
            Error::Msg(ref msg) => {
                println!("Startup error: {msg}");
                process::exit(1);
            }
            err => Err(err.into()),
        },
        Ok(model) => {
            let mut terminal = ratatui::init();
            terminal.clear()?;

            let app_result = run(terminal, model);
            ratatui::restore();
            app_result.map_err(Error::into)
        }
    }
}

fn map_to_io_error<I>(err: I) -> io::Error
where
    I: Into<Error>,
{
    err.into().into()
}

struct Model<'a> {
    original_file_path: Option<PathBuf>,
    bg: Option<[u8; 4]>,
    scroll: u16,
    picker: Arc<RwLock<Picker>>,
    font: Font<'a>,
    basepath: Option<PathBuf>,
    sources: Vec<WidgetSource<'a>>,
    padding: Padding,
    deep_fry: bool,
}

#[derive(Default, Debug, Serialize, Deserialize)]
enum Padding {
    None,
    Border,
    #[default]
    Empty,
}

impl<'a> Model<'a> {
    fn new(
        text: &str,
        original_file_path: Option<PathBuf>,
        config: Config,
        basepath: Option<PathBuf>,
        deep_fry: bool,
        force_font_setup: bool,
    ) -> Result<Self, Error> {
        print!("Detecting supported graphics protocols...");
        let mut picker = Picker::from_query_stdio().map_err(|err| Error::Msg(format!("{err}")))?;
        println!("{OK_END}");

        let bg = match picker.protocol_type() {
            ProtocolType::Sixel => Some([0, 0, 0, 255]),
            _ => {
                picker.set_background_color([0, 0, 0, 0]);
                None
            }
        };

        let mut fp_builder = system_fonts::FontPropertyBuilder::new();

        let all_fonts = system_fonts::query_all();

        let config_font_family = config.font_family.and_then(|font_family| {
            // Ensure this font exists
            if all_fonts.contains(&font_family) {
                return Some(font_family);
            }
            None
        });

        let font_family = if let Some(mut font_family) = config_font_family {
            if force_font_setup {
                println!("Entering forced font setup");
                match set_up_font(&mut picker, bg) {
                    Ok(setup_font_family) => {
                        let new_config = Config {
                            font_family: Some(font_family.clone()),
                            ..Default::default()
                        };
                        confy::store(CONFIG.0, CONFIG.1, new_config)?;
                        font_family = setup_font_family;
                    }
                    Err(err) => return Err(err),
                }
            }
            font_family
        } else {
            println!("Entering one-time font setup");
            match set_up_font(&mut picker, bg) {
                Ok(font_family) => {
                    let new_config = Config {
                        font_family: Some(font_family.clone()),
                        ..Default::default()
                    };
                    confy::store(CONFIG.0, CONFIG.1, new_config)?;
                    font_family
                }
                Err(err) => return Err(err),
            }
        };

        fp_builder = fp_builder.family(&font_family);

        let property = fp_builder.build();

        let (font_data, _) =
            system_fonts::get(&property).ok_or("Could not get system fonts property")?;

        let font = Font::try_from_vec(font_data).ok_or(Error::NoFont)?;

        let mut loading_terminal = ratatui::init_with_options(ratatui::TerminalOptions {
            viewport: ratatui::Viewport::Inline(1),
        });
        let screen_width = loading_terminal.size()?.width;
        loading_terminal.clear()?;

        loading_terminal.draw(|frame| {
            frame.render_widget(Paragraph::new("Parsing..."), frame.area());
        })?;

        let picker = Arc::new(RwLock::new(picker));

        let (tx, rx) = mpsc::channel();

        let text_thread = text.to_string();
        let thread_picker = picker.clone();
        let thread_font = font.clone();
        let thread_basepath = basepath.clone();

        let thread_width = match Padding::Empty {
            Padding::Empty | Padding::Border => screen_width - 2,
            _ => screen_width,
        };
        thread::spawn(move || {
            let arena = Box::new(Arena::new());
            let parser = Parser::new(
                &arena,
                thread_picker,
                thread_font,
                bg.clone(),
                thread_basepath,
            );
            parser.parse(&text_thread, thread_width, &tx.clone());
        });

        let mut model = Model {
            original_file_path,
            bg,
            scroll: 0,
            picker,
            font,
            basepath,
            sources: vec![],
            padding: Padding::Empty,
            deep_fry,
        };

        let mut sources = vec![];
        let mut i = 0;
        for received in rx {
            sources.push(received);
            loading_terminal.draw(|frame| {
                frame.render_widget(Paragraph::new(format!("Processing ({i})...")), frame.area());
            })?;
            i += 1;
        }

        model.sources = sources;

        disable_raw_mode()?;
        execute!(stdout(), LeaveAlternateScreen)?;
        println!("{OK_END}");

        Ok(model)
    }
}

fn model_reload(model: &mut Model) -> Result<(), Error> {
    if let Some(original_file_path) = &model.original_file_path {
        model.sources = vec![];

        let text = read_file_to_str(
            original_file_path
                .to_str()
                .ok_or(Error::Msg("could not convert original_file_path".into()))?,
        )?;

        let mut ext_options = ExtensionOptions::default();
        ext_options.strikethrough = true;
    }
    Ok(())
}

fn run(mut terminal: DefaultTerminal, mut model: Model) -> Result<(), Error> {
    let screen_size = terminal.size()?;
    let page_scroll_count = screen_size.height / 2;

    loop {
        if model.sources.is_empty() {
            let (tx, rx) = mpsc::channel();
            let mut sources = vec![];
            for received in rx {
                sources.push(received);
            }
            model.sources = sources;
        }

        terminal.draw(|frame| view(&mut model, frame))?;

        match event::read()? {
            event::Event::Key(key) => {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('q') => {
                            return Ok(());
                        }
                        KeyCode::Char('r') => {
                            model_reload(&mut model)?;
                        }
                        KeyCode::Char('j') => {
                            model.scroll += 1;
                        }
                        KeyCode::Char('k') => {
                            if model.scroll > 0 {
                                model.scroll -= 1;
                            }
                        }
                        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            model.scroll += page_scroll_count;
                        }
                        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            if page_scroll_count < model.scroll {
                                model.scroll -= page_scroll_count;
                            } else {
                                model.scroll = 0;
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
    let frame_area = frame.area();
    let mut block = Block::new();
    match model.padding {
        Padding::Border => {
            block = block.borders(ratatui::widgets::Borders::all());
        }
        Padding::Empty => {
            block = block.padding(ratatui::widgets::Padding::horizontal(1));
        }
        _ => {}
    }

    if let Some(bg) = model.bg {
        block = block.style(Style::default().bg(Color::Rgb(bg[0], bg[1], bg[2])));
    }
    let inner_area = block.inner(frame_area);
    frame.render_widget(block, frame_area);

    let mut y: i16 = 0 - (model.scroll as i16);
    for source in &mut model.sources {
        if y >= 0 {
            match &mut source.source {
                WidgetSourceData::Text(text) => {
                    let p = Paragraph::new(text.clone());
                    render_widget(p, source.height, y as u16, inner_area, frame);
                }
                WidgetSourceData::Image(proto) => {
                    let img = Image::new(proto);
                    render_widget(img, source.height, y as u16, inner_area, frame);
                }
            }
        }
        y += source.height as i16;
        if y >= inner_area.height as i16 {
            break;
        }
    }
}

fn render_widget<W: Widget>(widget: W, source_height: u16, y: u16, area: Rect, f: &mut Frame) {
    if source_height < area.height - y {
        let mut widget_area = area;
        widget_area.y += y;
        widget_area.height = widget_area.height.min(source_height);
        f.render_widget(widget, widget_area);
    }
}

#[derive(Debug)]
#[allow(dead_code)]
enum Error {
    Cli(clap::error::Error),
    Config(ConfyError),
    Io(io::Error),
    Image(image::ImageError),
    Download(reqwest::Error),
    Msg(String),
    NoFont,
    Thread,
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

impl From<reqwest::Error> for Error {
    fn from(value: reqwest::Error) -> Self {
        Self::Download(value)
    }
}

impl From<SendError<WidgetSource<'_>>> for Error {
    fn from(_: SendError<WidgetSource<'_>>) -> Self {
        Self::Thread
    }
}

fn read_file_to_str(path: &str) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;
    Ok(contents)
}
