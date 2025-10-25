#[cfg(not(windows))]
use std::os::fd::IntoRawFd;

use std::{
    cmp::min,
    fs::File,
    io::{self, Read, stdout},
    path::{Path, PathBuf},
    sync::{
        Arc,
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::Duration,
};

use clap::{ArgMatches, arg, command, value_parser};
use config::Config;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture, KeyModifiers, MouseEventKind},
    tty::IsTty,
};
use error::Error;
use markdown::parse;
use ratatui::{
    DefaultTerminal, Frame, Terminal,
    crossterm::event::{self, KeyCode, KeyEventKind},
    layout::Rect,
    prelude::CrosstermBackend,
    style::{Style, Stylize},
    text::{Line, Span, Text},
    widgets::{Block, Paragraph, Widget},
};

use ratatui_image::{Image, picker::ProtocolType};
use ratskin::RatSkin;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use setup::{BgColor, SetupResult, setup_graphics};
use tokio::{
    runtime::Builder,
    sync::{Mutex, RwLock},
};
use widget_sources::{
    BigText, SourceID, WidgetSource, WidgetSourceData, header_images, header_sources, image_source,
};

use crate::widget_sources::LineExtra;

mod config;
mod error;
mod fontpicker;
mod markdown;
mod setup;
mod widget_sources;

const OK_END: &str = " ok.";

const CONFIG_APP_NAME: &str = "mdfried";
const CONFIG_CONFIG_NAME: &str = "config";

fn main() -> io::Result<()> {
    let mut cmd = command!() // requires `cargo` feature
        .arg(
            arg!([path] "The markdown file path, or '-' for stdin")
                .value_parser(value_parser!(PathBuf)),
        )
        .arg(arg!(-s --setup "Force font setup").value_parser(value_parser!(bool)))
        .arg(arg!(-d --deep "Extra deep fried images").value_parser(value_parser!(bool)));
    let matches = cmd.get_matches_mut();

    match start(&matches) {
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

fn start(matches: &ArgMatches) -> Result<(), Error> {
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

    let config: Config = confy::load(CONFIG_APP_NAME, CONFIG_CONFIG_NAME)?;

    #[cfg(not(windows))]
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
    let setup_result = setup_graphics(config.font_family, force_setup);
    let (picker, bg, renderer, has_text_size_protocol) = match setup_result {
        Ok(result) => match result {
            SetupResult::Aborted => return Err(Error::UserAbort("cancelled setup")),
            SetupResult::TextSizing(picker, bg) => (picker, bg, None, true),
            SetupResult::Complete(picker, bg, renderer) => (picker, bg, Some(renderer), false),
        },
        Err(err) => return Err(err),
    };

    let deep_fry = *matches.get_one("deep").unwrap_or(&false);

    let (cmd_tx, cmd_rx) = mpsc::channel::<ImgCmd>();
    let (event_tx, event_rx) = mpsc::channel::<(u16, Event)>();

    let cmd_thread = thread::spawn(move || {
        let runtime = Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()?;
        runtime.block_on(async {
            let basepath = basepath.clone();
            let client = Arc::new(RwLock::new(Client::new()));
            let protocol_type = picker.protocol_type(); // Won't change
            let thread_renderer = renderer.map(|renderer| Arc::new(Mutex::new(renderer)));
            let thread_picker = Arc::new(picker);
            let skin = RatSkin { skin: config.skin };
            for cmd in cmd_rx {
                match cmd {
                    ImgCmd::Parse(width, text) => {
                        parse(&text, &skin, width, &event_tx, has_text_size_protocol)?;
                    }
                    ImgCmd::Header(id, width, tier, text) => {
                        debug_assert!(
                            thread_renderer.is_some(),
                            "should not have sent ImgCmd::Header without renderer"
                        );
                        if let Some(ref thread_renderer) = thread_renderer {
                            let task_tx = event_tx.clone();
                            if protocol_type != ProtocolType::Halfblocks {
                                let renderer = thread_renderer.clone();
                                let picker = thread_picker.clone();
                                tokio::spawn(async move {
                                    // Grab lock...
                                    let mut r = renderer.lock().await;
                                    let images =
                                        header_images(bg, &mut r, width, text, tier, deep_fry)?;
                                    // ...release right after text rendering...
                                    drop(r);
                                    // ...process images to terminal image protocol.
                                    let headers =
                                        header_sources(&picker, width, id, images, deep_fry)?;
                                    task_tx.send((width, Event::Update(headers)))?;
                                    Ok::<(), Error>(())
                                });
                            }
                        }
                    }
                    ImgCmd::UrlImage(id, width, url, text, _title) => {
                        let task_tx = event_tx.clone();
                        let basepath = basepath.clone();
                        let client = client.clone();
                        let picker = thread_picker.clone();
                        // TODO: handle spawned task result errors, right now it's just discarded.
                        tokio::spawn(async move {
                            match image_source(
                                &picker, width, &basepath, client, id, &url, deep_fry,
                            )
                            .await
                            {
                                Ok(source) => task_tx.send((width, Event::Update(vec![source])))?,
                                Err(Error::UnknownImage(id, link)) => task_tx.send((
                                    width,
                                    Event::Update(vec![WidgetSource::image_unknown(
                                        id, link, text,
                                    )]),
                                ))?,
                                Err(_) => task_tx.send((
                                    width,
                                    Event::Update(vec![WidgetSource::image_unknown(id, url, text)]),
                                ))?,
                            }
                            Ok::<(), Error>(())
                        });
                    }
                    ImgCmd::XdgOpen(url) => {
                        std::process::Command::new("xdg-open")
                            .arg(&url)
                            .spawn()
                            .ok();
                    }
                };
            }
            Ok::<(), Error>(())
        })?;
        Ok::<(), Error>(())
    });

    let model = Model::new(bg, path.cloned(), cmd_tx, event_rx)?;

    crossterm::terminal::enable_raw_mode()?;
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;
    if config.enable_mouse_capture {
        crossterm::execute!(std::io::stderr(), EnableMouseCapture)?;
    }
    terminal.clear()?;

    let inner_width = model.inner_width(terminal.size()?.width);
    model
        .cmd_tx
        .send(ImgCmd::Parse(inner_width, text))
        .map_err(Error::from)?;

    run(terminal, model)?;

    if config.enable_mouse_capture {
        crossterm::execute!(std::io::stderr(), DisableMouseCapture)?;
    }
    crossterm::terminal::disable_raw_mode()?;

    if let Err(e) = cmd_thread.join() {
        eprintln!("Thread error: {e:?}");
    }
    Ok(())
}

#[derive(Debug)]
enum ImgCmd {
    Parse(u16, String),
    UrlImage(usize, u16, String, String, String),
    Header(usize, u16, u8, String),
    XdgOpen(String),
}

#[derive(Debug)]
enum Event<'a> {
    Parsed(WidgetSource<'a>),
    Update(Vec<WidgetSource<'a>>),
    #[allow(dead_code)]
    ParseImage(SourceID, String, String, String),
    ParseHeader(SourceID, u8, String),
}

// Just a width key, to discard events for stale screen widths.
pub(crate) type WidthEvent<'a> = (u16, Event<'a>);

struct Model<'a, 'b> {
    original_file_path: Option<PathBuf>,
    bg: Option<BgColor>,
    scroll: u16,
    sources: Vec<WidgetSource<'a>>,
    padding: Padding,
    cmd_tx: Sender<ImgCmd>,
    event_rx: Receiver<WidthEvent<'b>>,
    mode: Mode,
}

#[derive(Default, Debug, Serialize, Deserialize)]
enum Padding {
    None,
    Border,
    #[default]
    Empty,
}

#[derive(PartialEq)]
enum Mode {
    Normal,
    Link(LinkModeState),
}

#[derive(Default, PartialEq)]
struct LinkModeState {
    links: Vec<(String, i16, u16, u16)>,
    cursor: usize,
}

impl Mode {
    fn link(&mut self) {
        *self = Mode::Link(LinkModeState::default());
    }
    fn normal(&mut self) {
        *self = Mode::Normal
    }
}

impl<'a, 'b: 'a> Model<'a, 'b> {
    fn new(
        bg: Option<BgColor>,
        original_file_path: Option<PathBuf>,
        cmd_tx: Sender<ImgCmd>,
        event_rx: Receiver<WidthEvent<'b>>,
    ) -> Result<Model<'a, 'b>, Error> {
        let model = Model {
            original_file_path,
            bg,
            scroll: 0,
            sources: vec![],
            padding: Padding::Empty,
            cmd_tx,
            event_rx,
            mode: Mode::Normal,
        };

        // model_reload(&mut model, screen_width)?;

        Ok(model)
    }

    fn inner_width(&self, screen_width: u16) -> u16 {
        match self.padding {
            Padding::None => screen_width,
            Padding::Empty | Padding::Border => screen_width - 2,
        }
    }

    fn inner_height(&self, screen_height: u16) -> u16 {
        match self.padding {
            Padding::None | Padding::Empty => screen_height,
            Padding::Border => screen_height - 2,
        }
    }

    fn total_lines(&self) -> u16 {
        self.sources.iter().map(|s| s.height).sum()
    }

    fn process_events(&mut self, screen_width: u16) -> Result<bool, Error> {
        let inner_width = self.inner_width(screen_width);
        let mut had_events = false;
        while let Ok((id, ev)) = self.event_rx.try_recv() {
            if id == inner_width {
                had_events = true;
                match ev {
                    Event::Parsed(source) => {
                        self.sources.push(source);
                    }
                    Event::Update(updates) => {
                        if let Some(id) = updates.first().map(|s| s.id) {
                            let mut first_position = None;
                            let mut i = 0;
                            self.sources.retain(|w| {
                                if w.id == id {
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
                                self.sources.splice(from..to, updates);
                            }
                            debug_assert!(
                                first_position.is_some(),
                                "Update #{:?} not found anymore",
                                id,
                            );
                        }
                    }
                    Event::ParseImage(id, url, text, title) => {
                        self.cmd_tx.send(ImgCmd::UrlImage(
                            id,
                            inner_width,
                            url.clone(),
                            text,
                            title,
                        ))?;
                        self.sources.push(WidgetSource {
                            id,
                            height: 1,
                            source: WidgetSourceData::Line(Line::from(format!(
                                "![Loading...]({url})"
                            ))),
                        });
                    }
                    Event::ParseHeader(id, tier, text) => {
                        self.cmd_tx
                            .send(ImgCmd::Header(id, inner_width, tier, text.clone()))?;

                        let line = Line::from(vec![
                            Span::from("#".repeat(tier as usize) + " ").light_blue(),
                            Span::from(text),
                        ]);
                        self.sources.push(WidgetSource {
                            id,
                            height: 2,
                            source: WidgetSourceData::Line(line),
                        });
                    }
                }
            }
        }
        Ok(had_events)
    }

    fn scroll_by(&mut self, lines: i16) {
        self.scroll = self.scroll.saturating_add_signed(lines);
    }
}

fn model_reload<'a>(model: &mut Model<'a, 'a>, width: u16) -> Result<(), Error> {
    if let Some(original_file_path) = &model.original_file_path {
        let text = read_file_to_str(
            original_file_path
                .to_str()
                .ok_or(Error::Path(original_file_path.to_path_buf()))?,
        )?;

        model.sources = vec![];
        model.scroll = 0;

        let inner_width = model.inner_width(width);
        model.cmd_tx.send(ImgCmd::Parse(inner_width, text))?;
    }
    Ok(())
}

fn run<'a>(mut terminal: DefaultTerminal, mut model: Model<'a, 'a>) -> Result<(), Error> {
    terminal.draw(|frame| view(&mut model, frame))?;

    loop {
        let screen_size = terminal.size()?;
        let page_scroll_count = model.inner_height(screen_size.height) as i16 - 2;
        let screen_width = screen_size.width;

        let had_events = model.process_events(screen_width)?;

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
                            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                return Ok(());
                            }
                            KeyCode::Char('r') => {
                                model_reload(&mut model, screen_width)?;
                            }
                            KeyCode::Char('j') | KeyCode::Down => match model.mode {
                                Mode::Normal => model.scroll_by(1),
                                Mode::Link(ref mut state) => {
                                    if state.cursor < state.links.len().saturating_sub(1) {
                                        state.cursor += 1;
                                    }
                                }
                            },
                            KeyCode::Char('k') | KeyCode::Up => match model.mode {
                                Mode::Normal => model.scroll_by(-1),
                                Mode::Link(ref mut state) => {
                                    if state.cursor > 0 {
                                        state.cursor -= 1;
                                    }
                                }
                            },
                            KeyCode::Char('d') => {
                                model.scroll_by((page_scroll_count + 1) / 2);
                            }
                            KeyCode::Char('u') => {
                                model.scroll_by(-(page_scroll_count + 1) / 2);
                            }
                            KeyCode::PageDown | KeyCode::Char(' ') => {
                                model.scroll_by(page_scroll_count);
                            }
                            KeyCode::Char('b') | KeyCode::PageUp => {
                                model.scroll_by(-page_scroll_count);
                            }
                            KeyCode::Char('g') => {
                                model.scroll = 0;
                            }
                            KeyCode::Char('G') => {
                                model.scroll = model.total_lines();
                            }
                            KeyCode::Char('f') => {
                                model.mode.link();
                            }
                            KeyCode::Esc if matches!(model.mode, Mode::Link(_)) => {
                                model.mode.normal();
                            }
                            KeyCode::Enter => match model.mode {
                                Mode::Normal => {}
                                Mode::Link(ref state) => {
                                    if let Some(url) =
                                        state.links.get(state.cursor).map(|t| t.0.clone())
                                    {
                                        model.cmd_tx.send(ImgCmd::XdgOpen(url))?;
                                    }
                                }
                            },
                            _ => {}
                        }
                    }
                }
                event::Event::Resize(new_width, _) => {
                    if screen_width != new_width {
                        model_reload(&mut model, new_width)?;
                    }
                }
                event::Event::Mouse(mouse) => match mouse.kind {
                    MouseEventKind::ScrollUp => {
                        model.scroll_by(-2);
                    }
                    MouseEventKind::ScrollDown => {
                        model.scroll_by(2);
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
        block = block.style(Style::default().bg(bg.into()));
    }
    let inner_area = block.inner(frame_area);
    frame.render_widget(block, frame_area);

    let mut rendered_links = Vec::new();

    model.scroll = min(
        model.scroll,
        model.total_lines().saturating_sub(inner_area.height) + 1,
    );
    let mut y: i16 = 0 - (model.scroll as i16);
    for source in &mut model.sources {
        if y >= 0 {
            match &mut source.source {
                WidgetSourceData::Line(line) | WidgetSourceData::LineExtra(line, _) => {
                    let p = Paragraph::new(line.clone());

                    if matches!(model.mode, Mode::Link(_))
                        && let WidgetSourceData::LineExtra(_, extra) = &source.source
                    {
                        for link in extra {
                            match link {
                                LineExtra::Link(url, start, end) => {
                                    rendered_links.push((url.clone(), y, *start, *end))
                                }
                            }
                        }
                    }

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
                WidgetSourceData::SizedLine(text, tier) => {
                    let big_text = BigText::new(text, *tier);
                    render_widget(big_text, 2, y as u16, inner_area, frame);
                }
            }
        }
        y += source.height as i16;
        if y >= inner_area.height as i16 {
            break;
        }
    }

    if let Mode::Link(LinkModeState {
        ref mut links,
        cursor,
    }) = model.mode
    {
        *links = rendered_links;
        if let Some((url, y, start, end)) = links.get(cursor) {
            let x = frame_area.x + *start + 1;
            let width = end - start;
            let area = Rect::new(x, *y as u16, width, 1);
            let link_overlay_widget = Paragraph::new(url.clone()).black().on_yellow();
            frame.render_widget(link_overlay_widget, area);
        }
    }

    let mode_str = match model.mode {
        Mode::Normal => "N",
        Mode::Link(_) => "L",
    };
    let mode_widget =
        Paragraph::new(" ".repeat(frame_area.width as usize - mode_str.len()) + mode_str);
    frame.render_widget(
        mode_widget,
        Rect::new(0, frame_area.height - 1, frame_area.width, 1),
    );
    frame.set_cursor_position((0, frame_area.height - 1));
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
