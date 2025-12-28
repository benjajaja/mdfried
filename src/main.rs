mod config;
mod cursor;
mod debug;
mod error;
mod markdown;
mod model;
mod setup;
mod watch;
mod widget_sources;
mod worker;

#[cfg(not(windows))]
use std::os::fd::IntoRawFd as _;

use std::{
    fmt::Display,
    fs::{self, File},
    io::{self, Read as _},
    path::{Path, PathBuf},
    sync::mpsc::{self},
    time::Duration,
};

use clap::{ArgMatches, arg, command, value_parser};
use flexi_logger::LoggerHandle;
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
use setup::{SetupResult, setup_graphics};

use crate::{
    config::Config,
    cursor::{Cursor, CursorPointer, SearchState},
    error::Error,
    model::{DocumentId, Model},
    watch::watch,
    widget_sources::{BigText, LineExtra, SourceID, WidgetSource, WidgetSourceData},
    worker::worker_thread,
};

const OK_END: &str = " ok.";

fn main() -> io::Result<()> {
    let mut cmd = command!() // requires `cargo` feature
        .arg(arg!(-d --"deep-fry" "Extra deep fried images").value_parser(value_parser!(bool)))
        .arg(arg!(-w --"watch" "Watch markdown file").value_parser(value_parser!(bool)))
        .arg(arg!(-s --"setup" "Force font setup").value_parser(value_parser!(bool)))
        .arg(
            arg!(--"print-config" "Write out full config file example to stdout")
                .value_parser(value_parser!(bool)),
        )
        .arg(
            arg!(--"no-cap-checks" "Don't query the terminal stdin for capabilities")
                .value_parser(value_parser!(bool)),
        )
        .arg(arg!(--"debug-override-protocol-type" <PROTOCOL> "Force graphics protocol to a specific type"))
        .arg(
            arg!(--"log" "log to mdfried_<timestamp>.log file in working directory")
                .value_parser(value_parser!(bool)),
        )
        .arg(
            arg!([path] "The markdown file path, or '-', or omit, for stdin")
                .value_parser(value_parser!(PathBuf)),
        );
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

    if *matches.get_one("print-config").unwrap_or(&false) {
        config::print_default()?;
        return Ok(());
    }

    let ui_logger = debug::ui_logger(*matches.get_one("log").unwrap_or(&false))?;

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

    let mut user_config = config::load_or_ask()?;
    let config = Config::from(user_config.clone());

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
    let no_cap_checks = *matches.get_one("no-cap-checks").unwrap_or(&false);
    let debug_override_protocol_type = config.debug_override_protocol_type.or(matches
        .get_one::<String>("debug-override-protocol-type")
        .map(|s| match s.as_str() {
            "Sixel" => ProtocolType::Sixel,
            "Iterm2" => ProtocolType::Iterm2,
            "Kitty" => ProtocolType::Kitty,
            _ => ProtocolType::Halfblocks,
        }));

    let (picker, bg, renderer, has_text_size_protocol) = {
        let setup_result = setup_graphics(
            &mut user_config,
            force_setup,
            no_cap_checks,
            debug_override_protocol_type,
        );
        match setup_result {
            Ok(result) => match result {
                SetupResult::Aborted => return Err(Error::UserAbort("cancelled setup")),
                SetupResult::TextSizing(picker, bg) => (picker, bg, None, true),
                SetupResult::Complete(picker, bg, renderer) => (picker, bg, Some(renderer), false),
            },
            Err(err) => return Err(err),
        }
    };

    let deep_fry = *matches.get_one("deep-fry").unwrap_or(&false);

    let watchmode_path = if *matches.get_one("watch").unwrap_or(&false) {
        path.cloned()
    } else {
        None
    };

    let (cmd_tx, cmd_rx) = mpsc::channel::<Cmd>();
    let (event_tx, event_rx) = mpsc::channel::<Event>();
    let watch_event_tx = event_tx.clone();

    let config_max_image_height = config.max_image_height;
    let skin = config.theme.skin.clone();
    let cmd_thread = worker_thread(
        basepath,
        picker,
        renderer,
        skin,
        bg,
        has_text_size_protocol,
        deep_fry,
        cmd_rx,
        event_tx,
        config_max_image_height,
    );

    ratatui::crossterm::terminal::enable_raw_mode()?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;
    let enable_mouse_capture = config.enable_mouse_capture;
    if enable_mouse_capture {
        ratatui::crossterm::execute!(io::stderr(), EnableMouseCapture)?;
    }
    let watch_debounce_milliseconds = config.watch_debounce_milliseconds;
    terminal.clear()?;

    let terminal_size = terminal.size()?;
    let model = Model::new(
        bg,
        path.cloned(),
        cmd_tx,
        event_rx,
        terminal.size()?,
        config,
    );
    model.open(terminal_size, text)?;

    let debouncer = if let Some(path) = watchmode_path {
        log::info!("watching file");
        Some(watch(&path, watch_event_tx, watch_debounce_milliseconds)?)
    } else {
        drop(watch_event_tx);
        None
    };

    run(&mut terminal, model, &ui_logger)?;
    drop(debouncer);

    // Cursor might be in wird places, prompt or whatever should always show at the bottom now.
    terminal.set_cursor_position((0, terminal_size.height - 1))?;

    if enable_mouse_capture {
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
    Parse(DocumentId, u16, String),
    UrlImage(DocumentId, usize, u16, String, String, String),
    Header(DocumentId, usize, u16, u8, String),
}

impl Display for Cmd {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Cmd::Parse(reload_id, width, _) => {
                write!(f, "Cmd::Parse({reload_id:?}, {width}, <text>)")
            }
            Cmd::UrlImage(document_id, source_id, width, url, _, _) => write!(
                f,
                "Cmd::UrlImage({document_id}, {source_id}, {width}, {url}, _, _)"
            ),
            Cmd::Header(document_id, source_id, width, tier, text) => write!(
                f,
                "Cmd::Header({document_id}, {source_id}, {width}, {tier}, {text})"
            ),
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum Event {
    NewDocument(DocumentId),
    ParseDone(DocumentId, Option<SourceID>), // Only signals "parsing done", not "images ready"!
    Parsed(DocumentId, WidgetSource),
    ParseImage(DocumentId, SourceID, String, String, String),
    ParseHeader(DocumentId, SourceID, u8, String),
    Update(DocumentId, Vec<WidgetSource>),
    FileChanged,
}

impl Display for Event {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Event::NewDocument(document_id) => write!(f, "Event::NewDocument({document_id})"),
            Event::ParseDone(document_id, last_source_id) => {
                write!(f, "Event::ParseDone({document_id}, {last_source_id:?})")
            }

            Event::Parsed(document_id, source) => {
                write!(
                    f,
                    "Event::Parsed({document_id}, id:{}, data: {})",
                    source.id, source.data
                )
            }

            Event::Update(document_id, updates) => {
                write!(f, "Event::Update({document_id}, <{updates:?}>)",)
            }

            Event::ParseImage(document_id, id, url, _, _) => {
                write!(f, "Event::ParseImage({document_id}, {id}, {url}, _, _)")
            }

            Event::ParseHeader(document_id, id, tier, text) => {
                write!(f, "Event::ParseHeader({document_id}, {id}, {tier}, {text})")
            }

            Event::FileChanged => write!(f, "Event::FileChanged"),
        }
    }
}

// Just a width key, to discard events for stale screen widths.
// type WidthEvent<'a> = (u16, Event<'a>);

#[expect(clippy::too_many_lines)]
fn run(
    terminal: &mut DefaultTerminal,
    mut model: Model,
    ui_logger: &LoggerHandle,
) -> Result<(), Error> {
    terminal.draw(|frame| view(&model, frame))?;
    let mut screen_size = terminal.size()?;

    loop {
        let page_scroll_count = model.inner_height(screen_size.height) as i16 - 2;

        let (had_events, _) = model.process_events(screen_size.width)?;

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
                                            let url = model.sources().find_map(|source| {
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
    for source in model.sources() {
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
                WidgetSourceData::Image(_, proto) => {
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
        if y >= inner_area.height as i16 - 1 {
            // Do not render into last line, nor beyond area.
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

#[cfg(test)]
#[expect(clippy::unwrap_used)]
mod tests {
    use std::{sync::mpsc, thread::JoinHandle};

    use insta::assert_snapshot;
    use ratatui::{Terminal, backend::TestBackend, layout::Size};
    use ratatui_image::picker::{Picker, ProtocolType};

    use crate::{
        Cmd, Event,
        config::{Config, UserConfig},
        error::Error,
        model::Model,
        view,
        worker::worker_thread,
    };

    fn setup(config: Config) -> (Model, JoinHandle<Result<(), Error>>, Size) {
        #[expect(clippy::let_underscore_untyped)]
        let _ = flexi_logger::Logger::try_with_env()
            .unwrap()
            .start()
            .inspect_err(|err| eprint!("test logger setup failed: {err}"));

        let (cmd_tx, cmd_rx) = mpsc::channel::<Cmd>();
        let (event_tx, event_rx) = mpsc::channel::<Event>();

        let picker = Picker::halfblocks();
        assert_eq!(picker.protocol_type(), ProtocolType::Halfblocks);
        let worker = worker_thread(
            None,
            picker,
            None,
            config.theme.skin.clone(),
            None,
            true,
            false,
            cmd_rx,
            event_tx,
            config.max_image_height,
        );

        let screen_size = (80, 20).into();

        let model = Model::new(None, None, cmd_tx, event_rx, screen_size, config);
        (model, worker, screen_size)
    }

    // Drop model so that cmd_rx gets closed and worker exits, then exit/join worker.
    fn teardown(model: Model, worker: JoinHandle<Result<(), Error>>) {
        drop(model);
        worker.join().unwrap().unwrap();
    }

    // Poll until parsed and no pending images.
    fn poll_parsed(model: &mut Model, screen_size: &Size) {
        loop {
            let (_, parse_done) = model.process_events(screen_size.width).unwrap();
            if parse_done {
                break;
            }
        }
        log::debug!("poll_parsed completed");
    }

    // Poll until parsed and no pending images.
    fn poll_done(model: &mut Model, screen_size: &Size) {
        while model.pending_image_count > 0 {
            model.process_events(screen_size.width).unwrap();
        }
        log::debug!("poll_done completed");
    }

    #[test]
    fn parse() {
        let config = UserConfig {
            max_image_height: Some(10),
            ..Default::default()
        }
        .into();
        let (mut model, worker, screen_size) = setup(config);
        let mut terminal =
            Terminal::new(TestBackend::new(screen_size.width, screen_size.height)).unwrap();

        model
            .open(
                screen_size,
                String::from(
                    r#"# Hello
This is a *test* markdown document.
Another line of same paragraph.
![image](./assets/NixOS.png)

# Last bit
Goodbye."#,
                ),
            )
            .unwrap();
        poll_parsed(&mut model, &screen_size);
        terminal.draw(|frame| view(&model, frame)).unwrap();
        assert_snapshot!("first parse image previews", terminal.backend());
        // Must load an image.
        poll_done(&mut model, &screen_size);
        terminal.draw(|frame| view(&model, frame)).unwrap();
        assert_snapshot!("first parse done", terminal.backend());

        teardown(model, worker);
    }

    #[test]
    fn reload_move_image() {
        let config = UserConfig {
            max_image_height: Some(10),
            ..Default::default()
        }
        .into();
        let (mut model, worker, screen_size) = setup(config);
        let mut terminal =
            Terminal::new(TestBackend::new(screen_size.width, screen_size.height)).unwrap();

        model
            .open(
                screen_size,
                String::from(
                    r#"# Hello
This is a test markdown document.
![image](./assets/NixOS.png)
Goodbye."#,
                ),
            )
            .unwrap();
        poll_parsed(&mut model, &screen_size);
        poll_done(&mut model, &screen_size);

        model
            .reparse(
                screen_size,
                String::from(
                    r#"# Hello
![image](./assets/NixOS.png)
This is a test markdown document.
Goodbye."#,
                ),
            )
            .unwrap();
        poll_parsed(&mut model, &screen_size);
        terminal.draw(|frame| view(&model, frame)).unwrap();
        assert_snapshot!("reload move image up", terminal.backend());

        model
            .reparse(
                screen_size,
                String::from(
                    r#"# Hello
This is a test markdown document.
![image](./assets/NixOS.png)
Goodbye."#,
                ),
            )
            .unwrap();
        poll_parsed(&mut model, &screen_size);
        terminal.draw(|frame| view(&model, frame)).unwrap();
        assert_snapshot!("reload move image down", terminal.backend());

        teardown(model, worker);
    }

    #[test]
    fn reload_add_image() {
        let config = UserConfig {
            max_image_height: Some(10),
            ..Default::default()
        }
        .into();
        let (mut model, worker, screen_size) = setup(config);
        let mut terminal =
            Terminal::new(TestBackend::new(screen_size.width, screen_size.height)).unwrap();

        model
            .open(
                screen_size,
                String::from(
                    r#"# Hello
This is a test markdown document.
![image](./assets/NixOS.png)
Goodbye."#,
                ),
            )
            .unwrap();
        poll_parsed(&mut model, &screen_size);
        poll_done(&mut model, &screen_size);

        model
            .reparse(
                screen_size,
                String::from(
                    r#"# Hello
This is a test markdown document.
![image](./assets/NixOS.png)
![image](./assets/you_fried.png)
Goodbye."#,
                ),
            )
            .unwrap();
        poll_parsed(&mut model, &screen_size);
        terminal.draw(|frame| view(&model, frame)).unwrap();
        assert_snapshot!("reload add image preview", terminal.backend());
        // Must load an image.
        poll_done(&mut model, &screen_size);
        terminal.draw(|frame| view(&model, frame)).unwrap();
        assert_snapshot!("reload add image done", terminal.backend());
        teardown(model, worker);
    }

    #[test]
    fn duplicate_image() {
        let config = UserConfig {
            max_image_height: Some(8),
            ..Default::default()
        }
        .into();
        let (mut model, worker, screen_size) = setup(config);
        let mut terminal =
            Terminal::new(TestBackend::new(screen_size.width, screen_size.height)).unwrap();

        model
            .open(
                screen_size,
                String::from(
                    r#"# Hello
![image](./assets/NixOS.png)
Goodbye."#,
                ),
            )
            .unwrap();
        poll_parsed(&mut model, &screen_size);
        poll_done(&mut model, &screen_size);

        model
            .reparse(
                screen_size,
                String::from(
                    r#"# Hello
![image](./assets/NixOS.png)
Goodbye.
![image](./assets/NixOS.png)"#,
                ),
            )
            .unwrap();
        poll_parsed(&mut model, &screen_size);
        terminal.draw(|frame| view(&model, frame)).unwrap();
        assert_snapshot!("duplicate image preview", terminal.backend());
        // Must load an image.
        poll_done(&mut model, &screen_size);
        terminal.draw(|frame| view(&model, frame)).unwrap();
        assert_snapshot!("duplicate image done", terminal.backend());
        teardown(model, worker);
    }
}
