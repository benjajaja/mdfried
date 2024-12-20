use std::{
    fs::File,
    io::{self, Read},
    os::fd::IntoRawFd,
    path::{Path, PathBuf},
    sync::{
        mpsc::{self, Receiver, Sender},
        Arc,
    },
    time::Duration,
};

use clap::{arg, command, value_parser, ArgMatches};
use config::Config;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture, KeyModifiers, MouseEventKind},
    tty::IsTty,
};
use error::Error;
use markdown::parse;
use ratatui::{
    crossterm::event::{self, KeyCode, KeyEventKind},
    layout::Rect,
    style::{Color, Style, Stylize},
    text::{Line, Span, Text},
    widgets::{Block, Paragraph, Widget},
    DefaultTerminal, Frame,
};

use comrak::ExtensionOptions;
use ratatui_image::Image;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use setup::setup_imagery;
use widget_sources::{header_source, image_source, WidgetSource, WidgetSourceData};

mod config;
mod error;
mod fontpicker;
mod markdown;
mod setup;
mod widget_sources;

const OK_END: &str = " ok.";

const CONFIG: (&str, Option<&str>) = ("mdfried", Some("config"));

#[tokio::main]
async fn main() -> io::Result<()> {
    let mut cmd = command!() // requires `cargo` feature
        .arg(
            arg!([path] "The markdown file path, or '-' for stdin")
                .value_parser(value_parser!(PathBuf)),
        )
        .arg(arg!(-s --setup "Force font setup").value_parser(value_parser!(bool)))
        .arg(arg!(-d --deep "Extra deep fried images").value_parser(value_parser!(bool)));
    let matches = cmd.get_matches_mut();

    match start(&matches).await {
        Err(Error::Usage(msg)) => {
            if let Some(msg) = msg {
                println!("Usage error: {msg}");
                println!();
            }
            cmd.write_help(&mut std::io::stdout())?;
        }
        Err(Error::UserAbort(msg)) => {
            println!("Abort: {msg}");
        }
        Err(err) => eprintln!("{err}"),
        _ => {}
    };
    Ok(())
}

async fn start(matches: &ArgMatches) -> Result<(), Error> {
    let path = matches.get_one::<PathBuf>("path");

    let (text, basepath) = match path {
        Some(path) if path.as_os_str() == "-" => {
            let mut text = String::new();
            print!("Reading stdin...");
            io::stdin().read_to_string(&mut text)?;
            println!("{OK_END}");
            (text, None)
        }
        Some(path) => (
            read_file_to_str(path.to_str().ok_or(Error::Path(path.to_owned()))?)?,
            path.parent().map(Path::to_path_buf),
        ),
        None => {
            if io::stdin().is_tty() {
                return Err(Error::Usage(Some(
                    "no path nor '-', and stdin is a tty (not a pipe)",
                )));
            }
            let mut text = String::new();
            print!("Reading stdin...");
            io::stdin().read_to_string(&mut text)?;
            println!("{OK_END}");
            (text, None)
        }
    };

    if text.is_empty() {
        return Err(Error::Usage(Some("no input or emtpy")));
    }

    let config: Config = confy::load(CONFIG.0, CONFIG.1)?;

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

    let force_setup = *matches.get_one("setup").unwrap_or(&false);
    let (mut picker, font, bg) = match setup_imagery(config.font_family, force_setup) {
        Ok(Some((picker, font, bg))) => (picker, font, bg),
        Ok(None) => return Err(Error::UserAbort("cancelled setup")),
        Err(err) => return Err(err),
    };

    let (cmd_tx, cmd_rx) = mpsc::channel::<ImgCmd>();
    let (event_tx, event_rx) = mpsc::channel::<(u16, Event)>();

    let _deep_fry = *matches.get_one("deep").unwrap_or(&false);

    let event_image_tx = event_tx.clone();
    let parse_handle = tokio::spawn(async move {
        let basepath = basepath.clone();
        let mut client = Client::new();
        let arc_font = Arc::new(font);
        for cmd in cmd_rx {
            match cmd {
                ImgCmd::Header(index, width, tier, text) => {
                    let task_tx = event_image_tx.clone();
                    let task_font = arc_font.clone();
                    task_tx.send((
                        width,
                        Event::Update(header_source(
                            &mut picker,
                            task_font,
                            bg,
                            width,
                            index,
                            text,
                            tier,
                            false,
                        )?),
                    ))?;
                }
                ImgCmd::UrlImage(index, width, url, text, _title) => {
                    match image_source(
                        &mut picker,
                        width,
                        &basepath,
                        &mut client,
                        index,
                        &url,
                        false,
                    )
                    .await
                    {
                        Ok(source) => event_image_tx.send((width, Event::Update(vec![source])))?,
                        Err(Error::UnknownImage(index, link)) => event_image_tx.send((
                            width,
                            Event::Update(vec![WidgetSource::image_unknown(index, link, text)]),
                        ))?,
                        Err(_) => event_image_tx.send((
                            width,
                            Event::Update(vec![WidgetSource::image_unknown(index, url, text)]),
                        ))?,
                    }
                }
            };
        }
        Ok(())
    });

    let (parse_tx, parse_rx) = mpsc::channel::<ParseCmd>();
    let parse_handle2 = tokio::spawn(async move {
        for ParseCmd { width, text } in parse_rx {
            parse(&text, width, &event_tx).await?;
        }
        Ok(())
    });

    let model = Model::new(bg, path.cloned(), cmd_tx, parse_tx, event_rx)?;

    let mut terminal = ratatui::init();
    crossterm::execute!(std::io::stderr(), EnableMouseCapture)?;
    terminal.clear()?;

    let inner_width = model.width(terminal.size()?.width);
    model
        .parse_tx
        .send(ParseCmd {
            width: inner_width,
            text,
        })
        .map_err(Error::from)?;

    let ui_handle = tokio::spawn(async move { run(terminal, model) });
    let result = tokio::select! {
        parse_res = parse_handle => parse_res?,
        parse2_res = parse_handle2 => parse2_res?,
        ui_res = ui_handle => ui_res?,
    };
    crossterm::execute!(std::io::stderr(), DisableMouseCapture)?;
    ratatui::restore();
    result.map_err(Error::from)
}

#[derive(Debug)]
enum ImgCmd {
    UrlImage(usize, u16, String, String, String),
    Header(usize, u16, u8, String),
}

struct ParseCmd {
    width: u16,
    text: String,
}

#[derive(Debug)]
enum Event<'a> {
    Parsed(WidgetSource<'a>),
    Update(Vec<WidgetSource<'a>>),
    ParseImage(usize, String, String, String),
    ParseHeader(usize, u8, Vec<Span<'a>>),
}

// Just a width key, to discard events for stale screen widths.
pub(crate) type WidthEvent<'a> = (u16, Event<'a>);

struct Model<'a, 'b> {
    original_file_path: Option<PathBuf>,
    bg: Option<[u8; 4]>,
    scroll: u16,
    sources: Vec<WidgetSource<'a>>,
    padding: Padding,
    tx: Sender<ImgCmd>,
    parse_tx: Sender<ParseCmd>,
    rx: Receiver<WidthEvent<'b>>,
}

#[derive(Default, Debug, Serialize, Deserialize)]
enum Padding {
    None,
    Border,
    #[default]
    Empty,
}

impl Model<'_, '_> {
    fn new<'a, 'b: 'a>(
        bg: Option<[u8; 4]>,
        original_file_path: Option<PathBuf>,
        tx: Sender<ImgCmd>,
        parse_tx: Sender<ParseCmd>,
        rx: Receiver<WidthEvent<'b>>,
    ) -> Result<Model<'a, 'b>, Error> {
        let model = Model {
            original_file_path,
            bg,
            scroll: 0,
            sources: vec![],
            padding: Padding::Empty,
            tx,
            parse_tx,
            rx,
        };

        // model_reload(&mut model, screen_width)?;

        Ok(model)
    }

    fn width(&self, screen_width: u16) -> u16 {
        match self.padding {
            Padding::None => screen_width,
            Padding::Empty | Padding::Border => screen_width - 2,
        }
    }
}

fn model_reload<'a>(model: &mut Model<'a, 'a>, width: u16) -> Result<(), Error> {
    if let Some(original_file_path) = &model.original_file_path {
        let text = read_file_to_str(
            original_file_path
                .to_str()
                .ok_or(Error::Path(original_file_path.to_path_buf()))?,
        )?;

        let mut ext_options = ExtensionOptions::default();
        ext_options.strikethrough = true;

        model.sources = vec![];
        model.scroll = 0;

        let inner_width = model.width(width);
        model.parse_tx.send(ParseCmd {
            width: inner_width,
            text,
        })?;
    }
    Ok(())
}

fn run<'a>(mut terminal: DefaultTerminal, mut model: Model<'a, 'a>) -> Result<(), Error> {
    let screen_size = terminal.size()?;
    let page_scroll_count = screen_size.height / 2;
    let mut screen_width = screen_size.width;
    let mut inner_width = match model.padding {
        Padding::None => screen_width,
        Padding::Empty | Padding::Border => screen_width - 2,
    };

    terminal.draw(|frame| view(&mut model, frame))?;

    loop {
        let mut had_events = false;
        if let Ok((id, ev)) = model.rx.try_recv() {
            if id == inner_width {
                had_events = true;
                match ev {
                    Event::Parsed(source) => {
                        model.sources.push(source);
                    }
                    Event::Update(updates) => {
                        if let Some(index) = updates.first().map(|s| s.index) {
                            let mut first_position = None;
                            let mut i = 0;
                            model.sources.retain(|w| {
                                if w.index == index {
                                    first_position = match first_position {
                                        None => Some((i, i)),
                                        Some((f, _)) => Some((f, i)),
                                    };
                                    return false;
                                }
                                i += 1;
                                true
                            });

                            if let Some((from, to)) = first_position {
                                model.sources.splice(from..to, updates);
                            }
                            debug_assert!(
                                first_position.is_some(),
                                "Update #{:?} not found anymore",
                                index,
                            );
                        }
                    }
                    Event::ParseImage(index, url, text, title) => {
                        model.tx.send(ImgCmd::UrlImage(
                            index,
                            inner_width,
                            url.clone(),
                            text,
                            title,
                        ))?;
                        model.sources.push(WidgetSource {
                            index,
                            height: 1,
                            source: WidgetSourceData::Line(Line::from(format!(
                                "![Loading...]({url})"
                            ))),
                        });
                    }
                    Event::ParseHeader(index, tier, spans) => {
                        let line = Line::from(spans);
                        let inner_width = match model.padding {
                            Padding::None => screen_width,
                            Padding::Empty | Padding::Border => screen_width - 2,
                        };
                        model.tx.send(ImgCmd::Header(
                            index,
                            inner_width,
                            tier,
                            line.to_string(),
                        ))?;
                        model.sources.push(WidgetSource {
                            index,
                            height: 2,
                            source: WidgetSourceData::Line(line),
                        });
                    }
                }
            }
        }

        let mut had_input = false;
        if event::poll(if had_events {
            Duration::ZERO
        } else {
            Duration::from_millis(100)
        })? {
            had_input = true;
            match event::read()? {
                event::Event::Key(key) => {
                    if key.kind == KeyEventKind::Press {
                        match key.code {
                            KeyCode::Char('q') => {
                                return Ok(());
                            }
                            KeyCode::Char('r') => {
                                model_reload(&mut model, screen_width)?;
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
                event::Event::Resize(width, _) => {
                    screen_width = width;
                    inner_width = match model.padding {
                        Padding::None => screen_width,
                        Padding::Empty | Padding::Border => screen_width - 2,
                    };
                    model_reload(&mut model, screen_width)?;
                }
                event::Event::Mouse(mouse) => match mouse.kind {
                    MouseEventKind::ScrollUp => {
                        if model.scroll > 0 {
                            if let Some(yea) = model.scroll.checked_sub(2) {
                                model.scroll = yea;
                            }
                        }
                    }
                    MouseEventKind::ScrollDown => {
                        model.scroll += 2;
                    }
                    _ => {}
                },
                _ => {}
            }
        }

        if had_events || had_input {
            terminal.draw(|frame| view(&mut model, frame))?;
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
                WidgetSourceData::Line(text) => {
                    let p = Paragraph::new(text.clone());
                    render_widget(p, source.height, y as u16, inner_area, frame);
                }
                WidgetSourceData::CodeBlock(text) => {
                    let p = Paragraph::new(text.clone()).on_dark_gray();
                    render_widget(p, source.height, y as u16, inner_area, frame);
                }
                WidgetSourceData::Image(proto) => {
                    let img = Image::new(proto);
                    render_widget(img, source.height, y as u16, inner_area, frame);
                }
                WidgetSourceData::BrokenImage(url, text) => {
                    let spans = vec![
                        Span::from(format!("![{text}](")).red(),
                        Span::from(url.clone()).blue(),
                        Span::from(")").red(),
                    ];
                    let text = Text::from(Line::from(spans));
                    let height = text.height();
                    let p = Paragraph::new(text);
                    render_widget(p, height as u16, y as u16, inner_area, frame);
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

fn read_file_to_str(path: &str) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;
    Ok(contents)
}
