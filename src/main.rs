mod config;
mod cursor;
mod debug;
mod error;
mod markdown;
mod model;
mod setup;
mod widget_sources;

#[cfg(not(windows))]
use std::os::fd::IntoRawFd as _;

use std::{
    fs::{self, File},
    io::{self, Read as _},
    path::{Path, PathBuf},
    sync::{
        Arc,
        mpsc::{self},
    },
    thread,
    time::Duration,
};

use clap::{ArgMatches, arg, command, value_parser};
use flexi_logger::LoggerHandle;
use log::warn;
use ratatui::{
    DefaultTerminal, Frame, Terminal,
    crossterm::{
        event::{
            self, DisableMouseCapture, EnableMouseCapture, KeyCode, KeyEventKind, KeyModifiers,
            MouseEventKind,
        },
        tty::IsTty as _,
    },
    layout::{Rect, Size},
    prelude::CrosstermBackend,
    style::{Color, Style, Stylize as _},
    text::{Line, Span, Text},
    widgets::{Block, Paragraph, Widget},
};

use ratatui_image::{Image, picker::ProtocolType};
use ratskin::RatSkin;
use reqwest::Client;
use setup::{SetupResult, setup_graphics};
use tokio::{runtime::Builder, sync::RwLock};

use crate::{
    cursor::{Cursor, CursorPointer, SearchState},
    error::Error,
    markdown::parse,
    model::Model,
    widget_sources::{
        BigText, LineExtra, SourceID, WidgetSource, WidgetSourceData, header_images,
        header_sources, image_source,
    },
};

const OK_END: &str = " ok.";

fn main() -> io::Result<()> {
    let mut cmd = command!() // requires `cargo` feature
        .arg(
            arg!([path] "The markdown file path, or '-', or omit, for stdin")
                .value_parser(value_parser!(PathBuf)),
        )
        .arg(arg!(-s --setup "Force font setup").value_parser(value_parser!(bool)))
        .arg(arg!(--debug_override_protocol_type <PROTOCOL> "Force graphics protocol type"))
        .arg(arg!(-d --deep "Extra deep fried images").value_parser(value_parser!(bool)));
    let matches = cmd.get_matches_mut();

    match main_with_args(&matches) {
        Err(Error::Usage(msg)) => {
            if let Some(msg) = msg {
                println!("Usage error: {msg}");
                println!();
            }
            cmd.write_help(&mut io::stdout())?;
        }
        Err(Error::UserAbort(msg)) => {
            println!("Abort: {msg}");
        }
        Err(err) => eprintln!("{err}"),
        _ => {}
    }
    Ok(())
}

#[expect(clippy::too_many_lines)]
fn main_with_args(matches: &ArgMatches) -> Result<(), Error> {
    let (panic_hook, eyre_hook) = color_eyre::config::HookBuilder::default()
        .panic_section(format!(
            "This is a bug. Consider reporting it at {}",
            env!("CARGO_PKG_REPOSITORY")
        ))
        .display_location_section(true)
        .display_env_section(true)
        .into_hooks();
    eyre_hook.install()?;
    std::panic::set_hook(Box::new(move |panic_info| {
        if let Err(err) = ratatui::crossterm::terminal::disable_raw_mode() {
            eprintln!("Unable to disable raw mode: {:?}", err);
        }
        let msg = format!("{}", panic_hook.panic_report(panic_info));
        log::error!("Panic: {}", msg);
        eprint!("{msg}");
        #[expect(clippy::exit)]
        std::process::exit(libc::EXIT_FAILURE);
    }));

    let ui_logger = debug::ui_logger()?;

    let path = matches.get_one::<PathBuf>("path");

    let (text, basepath) = match path {
        Some(path) if path.as_os_str() == "-" => {
            let mut text = String::new();
            print!("Reading stdin...");
            io::stdin().read_to_string(&mut text)?;
            println!("{OK_END}");
            (text, None)
        }
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
        Some(path) => (
            fs::read_to_string(path)?,
            path.parent().map(Path::to_path_buf),
        ),
    };

    if text.is_empty() {
        return Err(Error::Usage(Some("no input or empty")));
    }

    let mut config = config::load_or_ask()?;

    #[cfg(not(windows))]
    if !io::stdin().is_tty() {
        print!("Setting stdin to /dev/tty...");
        // Close the current stdin so that ratatui-image can read stuff from tty stdin.
        // SAFETY:
        // Calls some libc, not sure if this could be done otherwise.
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
    let setup_result = setup_graphics(&mut config, force_setup);
    let (mut picker, bg, renderer, has_text_size_protocol) = match setup_result {
        Ok(result) => match result {
            SetupResult::Aborted => return Err(Error::UserAbort("cancelled setup")),
            SetupResult::TextSizing(picker, bg) => (picker, bg, None, true),
            SetupResult::Complete(picker, bg, renderer) => (picker, bg, Some(renderer), false),
        },
        Err(err) => return Err(err),
    };

    if let Some(debug_override_protocol_type) = config.debug_override_protocol_type.or(matches
        .get_one::<String>("debug_override_protocol_type")
        .map(|s| match s.as_str() {
            "Sixel" => ProtocolType::Sixel,
            "Iterm2" => ProtocolType::Iterm2,
            "Kitty" => ProtocolType::Kitty,
            _ => ProtocolType::Halfblocks,
        }))
    {
        warn!("debug_override_protocol_type set to {debug_override_protocol_type:?}");
        picker.set_protocol_type(debug_override_protocol_type);
    }

    let deep_fry = *matches.get_one("deep").unwrap_or(&false);

    let (cmd_tx, cmd_rx) = mpsc::channel::<Cmd>();
    let (event_tx, event_rx) = mpsc::channel::<(u16, Event)>();

    let config_max_image_height = config.max_image_height;
    let skin = config.skin.clone();
    let cmd_thread = thread::spawn(move || {
        let runtime = Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()?;
        runtime.block_on(async {
            let basepath = basepath.clone();
            let client = Arc::new(RwLock::new(Client::new()));
            let protocol_type = picker.protocol_type(); // Won't change
            // Specifically not a tokio Mutex, because we use it in spawn_blocking.
            let thread_renderer =
                renderer.map(|renderer| Arc::new(std::sync::Mutex::new(renderer)));
            let thread_picker = Arc::new(picker);
            let skin = RatSkin { skin };
            log::debug!("cmd thread running");
            for cmd in cmd_rx {
                log::debug!("Cmd: {cmd:?}");
                match cmd {
                    Cmd::Parse(width, text) => {
                        for event in parse(&text, &skin, width, has_text_size_protocol) {
                            event_tx.send((width, event))?;
                        }
                    }
                    Cmd::Header(id, width, tier, text) => {
                        debug_assert!(
                            thread_renderer.is_some(),
                            "should not have sent ImgCmd::Header without renderer"
                        );
                        if let Some(thread_renderer) = &thread_renderer {
                            let task_tx = event_tx.clone();
                            if protocol_type != ProtocolType::Halfblocks {
                                let renderer = thread_renderer.clone();
                                let picker = thread_picker.clone();
                                tokio::spawn(async move {
                                    let images = tokio::task::spawn_blocking(move || {
                                        let mut r = renderer.lock()?;
                                        header_images(bg, &mut r, width, text, tier, deep_fry)
                                    })
                                    .await??;

                                    let headers = tokio::task::spawn_blocking(move || {
                                        header_sources(&picker, width, id, images, deep_fry)
                                    })
                                    .await??;
                                    task_tx.send((width, Event::Update(headers)))?;
                                    Ok::<(), Error>(())
                                });
                            }
                        }
                    }
                    Cmd::UrlImage(id, width, url, text, _title) => {
                        let task_tx = event_tx.clone();
                        let basepath = basepath.clone();
                        let client = client.clone();
                        let picker = thread_picker.clone();
                        // TODO: handle spawned task result errors, right now it's just discarded.
                        tokio::spawn(async move {
                            match image_source(
                                &picker,
                                config_max_image_height,
                                width,
                                &basepath,
                                client,
                                id,
                                &url,
                                deep_fry,
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
                    Cmd::XdgOpen(url) => {
                        std::process::Command::new("xdg-open").arg(&url).spawn()?;
                    }
                }
                event_tx.send((0, Event::MarkHadEvents))?;
            }
            Ok::<(), Error>(())
        })?;
        Ok::<(), Error>(())
    });

    ratatui::crossterm::terminal::enable_raw_mode()?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;
    let enable_mouse_capture = config.enable_mouse_capture;
    if enable_mouse_capture.unwrap_or_default() {
        ratatui::crossterm::execute!(io::stderr(), EnableMouseCapture)?;
    }
    terminal.clear()?;

    let terminal_size = terminal.size()?;
    let model = Model::new(
        bg,
        path.cloned(),
        cmd_tx,
        event_rx,
        terminal_size.height,
        config,
    );
    model.parse(terminal_size, text).map_err(Error::from)?;

    run(&mut terminal, model, &ui_logger)?;

    // Cursor might be in wird places, prompt or whatever should always show at the bottom now.
    terminal.set_cursor_position((0, terminal_size.height - 1))?;

    if enable_mouse_capture.unwrap_or_default() {
        ratatui::crossterm::execute!(io::stderr(), DisableMouseCapture)?;
    }
    ratatui::crossterm::terminal::disable_raw_mode()?;

    if let Err(e) = cmd_thread.join() {
        eprintln!("Thread error: {e:?}");
    }
    Ok(())
}

#[derive(Debug)]
enum Cmd {
    Parse(u16, String),
    UrlImage(usize, u16, String, String, String),
    Header(usize, u16, u8, String),
    // TODO: why not run this at call-site?
    XdgOpen(String),
}

#[derive(Debug, PartialEq)]
enum Event<'a> {
    Parsed(WidgetSource<'a>),
    ParseImage(SourceID, String, String, String),
    ParseHeader(SourceID, u8, String),
    Update(Vec<WidgetSource<'a>>),
    MarkHadEvents,
}

// Just a width key, to discard events for stale screen widths.
type WidthEvent<'a> = (u16, Event<'a>);

#[expect(clippy::too_many_lines)]
fn run<'a>(
    terminal: &mut DefaultTerminal,
    mut model: Model<'a, 'a>,
    ui_logger: &LoggerHandle,
) -> Result<(), Error> {
    terminal.draw(|frame| view(&model, frame))?;
    let mut screen_size = terminal.size()?;

    loop {
        let page_scroll_count = model.inner_height(screen_size.height) as i16 - 2;

        let had_events = model.process_events(screen_size.width)?;

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
                        match model.cursor {
                            Cursor::Search(ref mut mode, _) if !mode.accepted => match key.code {
                                KeyCode::Char('/') if mode.accepted => {
                                    *mode = SearchState::default();
                                    model.add_searches(None);
                                }
                                KeyCode::Char(c) => {
                                    mode.needle.push(c);
                                    let needle = mode.needle.clone();
                                    model.add_searches(Some(needle));
                                }
                                KeyCode::Backspace => {
                                    mode.needle.pop();
                                    let needle = mode.needle.clone();
                                    model.add_searches(Some(needle));
                                }
                                KeyCode::Esc => {
                                    model.cursor = Cursor::None;
                                }
                                KeyCode::Enter => {
                                    mode.accepted = true;
                                    model.cursor_next();
                                }
                                _ => {}
                            },
                            _ => {
                                match key.code {
                                    KeyCode::Char('q') => {
                                        return Ok(());
                                    }
                                    KeyCode::Char('c')
                                        if key.modifiers.contains(KeyModifiers::CONTROL) =>
                                    {
                                        return Ok(());
                                    }
                                    KeyCode::Char('r') => {
                                        model.reload(screen_size)?;
                                    }
                                    KeyCode::Char('j') | KeyCode::Down => {
                                        model.scroll_by(1);
                                    }
                                    KeyCode::Char('k') | KeyCode::Up => {
                                        model.scroll_by(-1);
                                    }
                                    KeyCode::Char('d') => {
                                        model.scroll_by((page_scroll_count + 1) / 2);
                                    }
                                    KeyCode::Char('u') => {
                                        model.scroll_by(-(page_scroll_count + 1) / 2);
                                    }
                                    KeyCode::Char('f' | ' ') | KeyCode::PageDown => {
                                        model.scroll_by(page_scroll_count);
                                    }
                                    KeyCode::Char('b') | KeyCode::PageUp => {
                                        model.scroll_by(-page_scroll_count);
                                    }
                                    KeyCode::Char('g') => {
                                        model.scroll = 0;
                                    }
                                    KeyCode::Char('G') => {
                                        model.scroll = model.total_lines().saturating_sub(
                                            page_scroll_count as u16 + 1, // Why +1?
                                        );
                                    }
                                    KeyCode::Char('/') => {
                                        model.cursor = Cursor::Search(SearchState::default(), None);
                                    }
                                    KeyCode::Char('n') => {
                                        model.cursor_next();
                                    }
                                    KeyCode::Char('N') => {
                                        model.cursor_prev();
                                    }
                                    KeyCode::F(11) => {
                                        model.log_snapshot = match model.log_snapshot {
                                            None => Some(flexi_logger::Snapshot::new()),
                                            Some(_) => None,
                                        };
                                    }
                                    KeyCode::Enter => {
                                        if let Cursor::Links(CursorPointer { id, index }) =
                                            model.cursor
                                        {
                                            let url = model.sources.iter().find_map(|source| {
                                                if source.id == id {
                                                    let WidgetSourceData::Line(_, extras) =
                                                        &source.data
                                                    else {
                                                        return None;
                                                    };

                                                    match extras.get(index) {
                                                        Some(LineExtra::Link(url, _, _)) => {
                                                            Some(url.clone())
                                                        }
                                                        _ => None,
                                                    }
                                                } else {
                                                    None
                                                }
                                            });
                                            if let Some(url) = url {
                                                log::debug!("open link_cursor {url}");
                                                model.open_link(url)?;
                                            }
                                        }
                                    }
                                    KeyCode::Esc => {
                                        if let Cursor::Search(SearchState { accepted, .. }, _) =
                                            model.cursor
                                            && accepted
                                        {
                                            model.cursor = Cursor::None;
                                        } else if let Cursor::Links(_) = model.cursor {
                                            model.cursor = Cursor::None;
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
                event::Event::Resize(new_width, new_height) => {
                    log::debug!("Resize {new_width},{new_height}");
                    if screen_size.width != new_width || screen_size.height != new_height {
                        screen_size = Size::new(new_width, new_height);
                        model.reload(screen_size)?;
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
            if let Some(ref mut snapshot) = model.log_snapshot {
                ui_logger.update_snapshot(snapshot)?;
            }
            terminal.draw(|frame| view(&model, frame))?;
        }
    }
}

fn view(model: &Model, frame: &mut Frame) {
    let frame_area = frame.area();
    let mut block = Block::new();
    let padding = model.block_padding(frame_area);
    block = block.padding(padding);

    if let Some(bg) = model.bg {
        block = block.style(Style::default().bg(bg.into()));
    }

    let inner_area = if let Some(snapshot) = &model.log_snapshot {
        let area = debug::render_snapshot(snapshot, frame);
        let mut fixed_padding = padding;
        fixed_padding.right = 0;
        block = block.padding(fixed_padding);
        block.inner(area)
    } else {
        block.inner(frame_area)
    };

    frame.render_widget(block, frame_area);

    let mut cursor_positioned = None;

    let mut y: i16 = 0 - (model.scroll as i16);
    for source in model.sources.iter() {
        if y >= 0 {
            let y: u16 = y as u16;
            match &source.data {
                WidgetSourceData::Line(line, extras) => {
                    let p = Paragraph::new(line.clone());

                    render_widget(p, source.height, y, inner_area, frame);

                    match &model.cursor {
                        Cursor::Links(CursorPointer { id, index })
                            if *id == source.id && !extras.is_empty() =>
                        {
                            // Render links now on top, again, this shouldn't be a performance concern.

                            if let Some(LineExtra::Link(url, start, end)) = extras.get(*index) {
                                let x = frame_area.x + padding.left + *start;
                                let width = end - start;
                                let area = Rect::new(x, y, width, 1);
                                let link_overlay_widget = Paragraph::new(url.clone())
                                    .fg(Color::Indexed(15))
                                    .bg(Color::Indexed(32));
                                frame.render_widget(link_overlay_widget, area);
                                cursor_positioned = Some((x, y));
                            }
                        }
                        Cursor::Search(SearchState { .. }, pointer) => {
                            for (i, extra) in extras.iter().enumerate() {
                                if let LineExtra::SearchMatch(start, end, text) = extra {
                                    let x = frame_area.x + padding.left + (*start as u16);
                                    let width = *end as u16 - *start as u16;
                                    let area = Rect::new(x, y, width, 1);
                                    let mut link_overlay_widget = Paragraph::new(text.clone());
                                    link_overlay_widget = if let Some(CursorPointer { id, index }) =
                                        pointer
                                        && source.id == *id
                                        && i == *index
                                    {
                                        link_overlay_widget.fg(Color::Black).bg(Color::Indexed(197))
                                    } else {
                                        link_overlay_widget.fg(Color::Black).bg(Color::Indexed(148))
                                    };
                                    frame.render_widget(link_overlay_widget, area);
                                    cursor_positioned = Some((x, y));
                                }
                            }
                        }
                        _ => {}
                    }
                }
                WidgetSourceData::Image(proto) => {
                    let img = Image::new(proto);
                    render_widget(img, source.height, y, inner_area, frame);
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
                    render_widget(p, height as u16, y, inner_area, frame);
                }
                WidgetSourceData::Header(text, tier) => {
                    let big_text = BigText::new(text, *tier);
                    render_widget(big_text, 2, y, inner_area, frame);
                }
            }
        }
        y += source.height as i16;
        if y >= inner_area.height as i16 {
            break;
        }
    }

    match &model.cursor {
        Cursor::None => {
            frame.set_cursor_position((0, frame_area.height - 1));
        }
        Cursor::Links(_) => {
            let mut line = Line::default();
            line.spans.push(Span::from("Links").fg(Color::Indexed(32)));
            let width = line.width() as u16;
            let searchbar = Paragraph::new(line);
            frame.render_widget(searchbar, Rect::new(0, frame_area.height - 1, width, 1));
            if cursor_positioned.is_none() {
                frame.set_cursor_position((0, frame_area.height - 1));
            }
        }
        Cursor::Search(mode, _) => {
            let mut line = Line::default();
            line.spans.push(Span::from("/").fg(Color::Indexed(148)));
            let mut needle = Span::from(mode.needle.clone());
            if mode.accepted {
                needle = needle.fg(Color::Indexed(148));
            }
            line.spans.push(needle);
            let width = line.width() as u16;
            let searchbar = Paragraph::new(line);
            frame.render_widget(searchbar, Rect::new(0, frame_area.height - 1, width, 1));
            if !mode.accepted {
                frame.set_cursor_position((width, frame_area.height - 1));
            } else if cursor_positioned.is_none() {
                frame.set_cursor_position((0, frame_area.height - 1));
            }
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
