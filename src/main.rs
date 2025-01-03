#[cfg(not(windows))]
use std::os::fd::IntoRawFd;

use std::{
    fs::File,
    io::{self, stdout, Read},
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
    prelude::CrosstermBackend,
    style::{Color, Style, Stylize},
    text::{Line, Span, Text},
    widgets::{Block, Paragraph, Widget},
    DefaultTerminal, Frame, Terminal,
};

use comrak::ExtensionOptions;
use ratatui_image::{picker::ProtocolType, Image};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use setup::setup_graphics;
use tokio::sync::RwLock;
use widget_sources::{header_source, image_source, SourceID, WidgetSource, WidgetSourceData};

mod config;
mod error;
mod fontpicker;
mod markdown;
mod setup;
mod widget_sources;
mod wordwrap;

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
    let renderer = match setup_graphics(config.font_family, force_setup).await {
        Ok(Some(renderer)) => renderer,
        Ok(None) => return Err(Error::UserAbort("cancelled setup")),
        Err(err) => return Err(err),
    };
    let _deep_fry = *matches.get_one("deep").unwrap_or(&false);

    let bg = renderer.bg;

    let (cmd_tx, cmd_rx) = mpsc::channel::<ImgCmd>();
    let (event_tx, event_rx) = mpsc::channel::<(u16, Event)>();

    let event_image_tx = event_tx.clone();
    let parse_handle = tokio::spawn(async move {
        let basepath = basepath.clone();
        let client = Arc::new(RwLock::new(Client::new()));
        let renderer = Arc::new(renderer);
        for cmd in cmd_rx {
            match cmd {
                ImgCmd::Header(id, width, tier, text) => {
                    if renderer.picker.protocol_type() != ProtocolType::Halfblocks {
                        let task_tx = event_image_tx.clone();
                        let r = renderer.clone();
                        tokio::spawn(async move {
                            let header = header_source(&r, width, id, text, tier, false).await?;
                            task_tx.send((width, Event::Update(header)))?;
                            Ok::<(), Error>(())
                        });
                    }
                }
                ImgCmd::UrlImage(id, width, url, text, _title) => {
                    let task_tx = event_image_tx.clone();
                    let r = renderer.clone();
                    let basepath = basepath.clone();
                    let client = client.clone();
                    tokio::spawn(async move {
                        let picker = r.picker;
                        match image_source(&picker, width, &basepath, client, id, &url, false).await
                        {
                            Ok(source) => task_tx.send((width, Event::Update(vec![source])))?,
                            Err(Error::UnknownImage(id, link)) => task_tx.send((
                                width,
                                Event::Update(vec![WidgetSource::image_unknown(id, link, text)]),
                            ))?,
                            Err(_) => task_tx.send((
                                width,
                                Event::Update(vec![WidgetSource::image_unknown(id, url, text)]),
                            ))?,
                        }
                        Ok::<(), Error>(())
                    });
                }
            };
        }
        Ok(())
    });

    let (parse_tx, parse_rx) = mpsc::channel::<ParseCmd>();
    let parse_handle2 = tokio::spawn(async move {
        for ParseCmd { width, text } in parse_rx {
            parse(&text, width, &event_tx)?;
        }
        Ok(())
    });

    let model = Model::new(bg, path.cloned(), cmd_tx, parse_tx, event_rx)?;

    crossterm::terminal::enable_raw_mode()?;
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;
    crossterm::execute!(std::io::stderr(), EnableMouseCapture)?;
    terminal.clear()?;

    let inner_width = model.inner_width(terminal.size()?.width);
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
    crossterm::terminal::disable_raw_mode()?;
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
    ParseImage(SourceID, String, String, String),
    ParseHeader(SourceID, u8, Vec<Span<'a>>),
}

// Just a width key, to discard events for stale screen widths.
pub(crate) type WidthEvent<'a> = (u16, Event<'a>);

struct Model<'a, 'b> {
    original_file_path: Option<PathBuf>,
    bg: Option<[u8; 4]>,
    scroll: u16,
    sources: Vec<WidgetSource<'a>>,
    padding: Padding,
    cmd_tx: Sender<ImgCmd>,
    parse_tx: Sender<ParseCmd>,
    event_rx: Receiver<WidthEvent<'b>>,
}

#[derive(Default, Debug, Serialize, Deserialize)]
enum Padding {
    None,
    Border,
    #[default]
    Empty,
}

impl<'a, 'b: 'a> Model<'a, 'b> {
    fn new(
        bg: Option<[u8; 4]>,
        original_file_path: Option<PathBuf>,
        cmd_tx: Sender<ImgCmd>,
        parse_tx: Sender<ParseCmd>,
        event_rx: Receiver<WidthEvent<'b>>,
    ) -> Result<Model<'a, 'b>, Error> {
        let model = Model {
            original_file_path,
            bg,
            scroll: 0,
            sources: vec![],
            padding: Padding::Empty,
            cmd_tx,
            parse_tx,
            event_rx,
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

    fn process_events(&mut self, screen_width: u16) -> Result<bool, Error> {
        let inner_width = match self.padding {
            Padding::None => screen_width,
            Padding::Empty | Padding::Border => screen_width - 2,
        };

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
                    Event::ParseHeader(id, tier, mut spans) => {
                        spans.insert(0, Span::from("#".repeat(tier as usize) + " ").light_blue());
                        let line = Line::from(spans);
                        let inner_width = match self.padding {
                            Padding::None => screen_width,
                            Padding::Empty | Padding::Border => screen_width - 2,
                        };
                        self.cmd_tx.send(ImgCmd::Header(
                            id,
                            inner_width,
                            tier,
                            line.to_string(),
                        ))?;
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

        let inner_width = model.inner_width(width);
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

    terminal.draw(|frame| view(&mut model, frame))?;

    loop {
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

#[cfg(test)]
mod tests {
    use insta::assert_snapshot;
    use ratatui::{backend::TestBackend, Terminal};

    use crate::*;

    #[test]
    fn test_md_snapshot() -> Result<(), Error> {
        const TERM_WIDTH: u16 = 120;
        let (cmd_tx, _cmd_rx) = mpsc::channel::<ImgCmd>();
        let (event_tx, event_rx) = mpsc::channel::<(u16, Event)>();
        let (parse_tx, _parse_rx) = mpsc::channel::<ParseCmd>();
        let mut model = Model::new(None, None, cmd_tx, parse_tx, event_rx).unwrap();

        parse(
            include_str!("../assets/test.md"),
            model.inner_width(TERM_WIDTH),
            &event_tx,
        )?;
        model.process_events(TERM_WIDTH).unwrap();

        let mut terminal = Terminal::new(TestBackend::new(TERM_WIDTH, 64)).unwrap();
        terminal.draw(|frame| view(&mut model, frame)).unwrap();

        assert_snapshot!(terminal.backend());
        Ok(())
    }

    #[test]
    fn test_list_snapshot() -> Result<(), Error> {
        const TERM_WIDTH: u16 = 80;
        let (cmd_tx, _cmd_rx) = mpsc::channel::<ImgCmd>();
        let (event_tx, event_rx) = mpsc::channel::<(u16, Event)>();
        let (parse_tx, _parse_rx) = mpsc::channel::<ParseCmd>();
        let mut model = Model::new(None, None, cmd_tx, parse_tx, event_rx).unwrap();

        parse(
            r#"
1. First ordered list item
2. Another item
   - Unordered sub-list.
3. Actual numbers don't matter, just that it's a number
   1. Ordered sub-list
4. And another item.

   You can have properly indented paragraphs within list items. Notice the blank line above, and the leading spaces (at least one, but we'll use three here to also align the raw Markdown).

   To have a line break without a paragraph, you will need to use two trailing spaces.  
   Note that this line is separate, but within the same paragraph.  
   (This is contrary to the typical GFM line break behaviour, where trailing spaces are not required.)

- Unordered list can use asterisks

* Or minuses

- Or pluses

1. Make my changes
   1. Fix bug
   2. Improve formatting
      - Make the headings bigger
2. Push my commits to GitHub
3. Open a pull request
   - Describe my changes
   - Mention all the members of my team
     - Ask for feedback

- Create a list by starting a line with `+`, `-`, or `*`
- Sub-lists are made by indenting 2 spaces:
  - Marker character change forces new list start:
    - Ac tristique libero volutpat at
    * Facilisis in pretium nisl aliquet
    - Nulla volutpat aliquam velit
- Very easy!
"#,
            model.inner_width(TERM_WIDTH),
            &event_tx,
        )?;
        model.process_events(TERM_WIDTH).unwrap();

        let mut terminal = Terminal::new(TestBackend::new(TERM_WIDTH, 40)).unwrap();
        terminal.draw(|frame| view(&mut model, frame)).unwrap();

        assert_snapshot!(terminal.backend());
        Ok(())
    }
}
