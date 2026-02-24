mod big_text;
mod config;
mod cursor;
mod debug;
mod document;
mod error;
mod keybindings;
mod model;
mod setup;
mod watch;
mod worker;

#[cfg(not(windows))]
use std::os::fd::IntoRawFd as _;

use std::{
    fmt::Display,
    fs::{self, File},
    io::{self, Read as _},
    path::{Path, PathBuf},
    sync::mpsc::{self},
};

use clap::{ArgMatches, arg, command, value_parser};
use flexi_logger::LoggerHandle;
use ratatui::{
    DefaultTerminal, Frame, Terminal,
    crossterm::{
        event::{DisableMouseCapture, EnableMouseCapture},
        tty::IsTty as _,
    },
    layout::Rect,
    prelude::CrosstermBackend,
    style::{Color, Stylize as _},
    text::{Line, Span, Text},
    widgets::{Block, Paragraph, Widget},
};

use mdfrier::Mapper as _;
use ratatui_image::{Image, picker::ProtocolType, protocol::Protocol};
use setup::{SetupResult, setup_graphics};

use crate::{
    big_text::BigText,
    config::Config,
    cursor::{Cursor, CursorPointer},
    document::{LineExtra, Section, SectionContent, SectionID},
    error::Error,
    keybindings::PollResult,
    model::{DocumentId, InputQueue, Model},
    watch::watch,
    worker::{ImageCache, worker_thread},
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

    let (picker, renderer, has_text_size_protocol) = {
        let setup_result = setup_graphics(
            &mut user_config,
            force_setup,
            no_cap_checks,
            debug_override_protocol_type,
        );
        match setup_result {
            Ok(result) => match result {
                SetupResult::Aborted => return Err(Error::UserAbort("cancelled setup")),
                SetupResult::TextSizing(picker) => (picker, None, true),
                SetupResult::AsciiArt(picker) => (picker, None, false),
                SetupResult::Complete(picker, renderer) => (picker, Some(renderer), false),
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
    let cmd_thread = worker_thread(
        basepath,
        picker,
        renderer,
        config.theme.clone(),
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
    let model = Model::new(path.cloned(), cmd_tx, event_rx, terminal.size()?, config);
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

enum Cmd {
    Parse(DocumentId, u16, String, Option<ImageCache>),
}

impl std::fmt::Debug for Cmd {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Display::fmt(self, f)
    }
}

impl Display for Cmd {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Cmd::Parse(reload_id, width, _, cache) => {
                write!(
                    f,
                    "Cmd::Parse({reload_id:?}, {width}, <text>, cache={})",
                    cache
                        .as_ref()
                        .map(|c| c.images.len() + c.headers.len())
                        .unwrap_or(0)
                )
            }
        }
    }
}

pub enum Event {
    NewDocument(DocumentId),
    ParseDone(DocumentId, Option<SectionID>), // Only signals "parsing done", not "images ready"!
    Parsed(DocumentId, Section),
    ImageLoaded(DocumentId, SectionID, String, Protocol),
    HeaderLoaded(DocumentId, SectionID, Vec<(String, u8, Protocol)>),
    FileChanged,
}

impl Display for Event {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Event::NewDocument(document_id) => write!(f, "Event::NewDocument({document_id})"),
            Event::ParseDone(document_id, last_section_id) => {
                write!(f, "Event::ParseDone({document_id}, {last_section_id:?})")
            }

            Event::Parsed(document_id, section) => {
                write!(
                    f,
                    "Event::Parsed({document_id}, id:{}, content: {})",
                    section.id, section.content
                )
            }

            Event::ImageLoaded(document_id, section_id, url, _) => {
                write!(f, "Event::ImageLoaded({document_id}, {section_id}, {url})")
            }

            Event::HeaderLoaded(document_id, section_id, rows) => {
                write!(
                    f,
                    "Event::HeaderLoaded({document_id}, {section_id}, {})",
                    rows.first()
                        .map(|(text, _, _)| text.clone())
                        .unwrap_or_default()
                )
            }

            Event::FileChanged => write!(f, "Event::FileChanged"),
        }
    }
}

impl std::fmt::Debug for Event {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Reuse Display impl
        Display::fmt(self, f)
    }
}

#[derive(Debug, PartialEq)]
pub struct MarkdownImage {
    pub destination: String,
    pub description: String,
}
impl Display for MarkdownImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ParseImageArgs{{ {}, {} }}",
            self.destination, self.description
        )
    }
}

fn run(
    terminal: &mut DefaultTerminal,
    mut model: Model,
    ui_logger: &LoggerHandle,
) -> Result<(), Error> {
    terminal.draw(|frame| view(&model, frame))?;
    loop {
        let (had_events, _, had_reload) = model.process_events()?;

        let (had_input, skip_render) = match keybindings::poll(had_events, &mut model)? {
            PollResult::Quit => return Ok(()),
            PollResult::None => (false, false),
            PollResult::HadInput => (true, false),
            PollResult::SkipRender => (true, true),
        };

        if (had_events || had_input) && !skip_render && !had_reload {
            if let Some(ref mut snapshot) = model.log_snapshot {
                ui_logger.update_snapshot(snapshot)?;
            }
            terminal.draw(|frame| view(&model, frame))?;
        }
    }
}

/// Extract text content from a Line at a given character position.
/// Each LineExtra::Link corresponds to exactly one span, so we find the span starting at `start`.
fn extract_line_content(line: &Line, start: u16) -> String {
    let mut pos: u16 = 0;
    for span in &line.spans {
        if pos == start {
            return span.content.to_string();
        }
        let span_width = unicode_width::UnicodeWidthStr::width(span.content.as_ref()) as u16;
        pos += span_width;
        if pos > start {
            // We passed the start position without finding an exact match
            break;
        }
    }
    String::new()
}

fn view(model: &Model, frame: &mut Frame) {
    let frame_area = frame.area();
    let mut block = Block::new();
    let padding = model.block_padding(frame_area);
    block = block.padding(padding);

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

    // Get the selected link URL if in Links mode (for highlighting all spans of wrapped URLs)
    let selected_url = match &model.cursor {
        Cursor::Links(pointer) => model.selected_link_url(pointer),
        _ => None,
    };

    let mut y: i16 = 0 - (model.scroll as i16);
    for section in model.sections() {
        if y >= 0 {
            let y: u16 = y as u16;
            match &section.content {
                SectionContent::Lines(lines) => {
                    let mut flat_index = 0;
                    let mut line_y = y;
                    for (line, extras) in lines.iter() {
                        if line_y >= inner_area.height {
                            break;
                        }

                        // Check if this line has a loaded image
                        let image_extra = extras.iter().find_map(|e| {
                            if let LineExtra::Image(_, proto) = e {
                                Some(proto)
                            } else {
                                None
                            }
                        });

                        let line_height = if let Some(proto) = image_extra {
                            let img = Image::new(proto);
                            let h = proto.area().height;
                            render_widget(img, h, line_y, inner_area, frame);
                            h
                        } else {
                            let p = Paragraph::new(line.clone());
                            render_widget(p, 1, line_y, inner_area, frame);
                            1
                        };

                        // Highlight all links that share the same URL as the selected link
                        if let Cursor::Links(CursorPointer { id, index }) = &model.cursor {
                            if let Some(selected) = &selected_url {
                                for (i, extra) in extras.iter().enumerate() {
                                    if let LineExtra::Link(url, start, end) = extra {
                                        if url.as_ptr() == selected.as_ptr() {
                                            let x = frame_area.x + padding.left + *start;
                                            let width = end - start;
                                            let area = Rect::new(x, line_y, width, 1);
                                            // Highlight with original content - source_content is only for grouping/opening
                                            let display_text = extract_line_content(line, *start);
                                            let link_overlay_widget = Paragraph::new(display_text)
                                                .fg(Color::Indexed(15))
                                                .bg(Color::Indexed(32));
                                            frame.render_widget(link_overlay_widget, area);

                                            // Position cursor on the actual selected link
                                            if *id == section.id && *index == flat_index + i {
                                                cursor_positioned = Some((x, line_y));
                                            }
                                        }
                                    }
                                }
                            }
                        } else if let Cursor::Search(_, pointer) = &model.cursor {
                            for (i, extra) in extras.iter().enumerate() {
                                if let LineExtra::SearchMatch(start, end, text) = extra {
                                    let x = frame_area.x + padding.left + (*start as u16);
                                    let width = *end as u16 - *start as u16;
                                    let area = Rect::new(x, line_y, width, 1);
                                    let mut link_overlay_widget = Paragraph::new(text.clone());
                                    link_overlay_widget = if let Some(CursorPointer { id, index }) =
                                        pointer
                                        && section.id == *id
                                        && flat_index + i == *index
                                    {
                                        link_overlay_widget.fg(Color::Black).bg(Color::Indexed(197))
                                    } else {
                                        link_overlay_widget.fg(Color::Black).bg(Color::Indexed(148))
                                    };
                                    frame.render_widget(link_overlay_widget, area);
                                    cursor_positioned = Some((x, line_y));
                                }
                            }
                        }
                        flat_index += extras.len();
                        line_y += line_height;
                    }
                }
                SectionContent::Image(_, proto) => {
                    let img = Image::new(proto);
                    render_widget(img, section.height, y, inner_area, frame);
                }
                SectionContent::BrokenImage(url, text) => {
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
                SectionContent::Header(text, tier, proto) => {
                    if let Some(proto) = proto {
                        let img = Image::new(proto);
                        render_widget(img, section.height, y, inner_area, frame);
                    } else {
                        let big_text = BigText::new(text, *tier);
                        render_widget(big_text, 2, y, inner_area, frame);
                    }
                }
            }
        }
        y += section.height as i16;
        if y >= inner_area.height as i16 - 1 {
            // Do not render into last line, nor beyond area.
            break;
        }
    }

    match &model.input_queue {
        InputQueue::None => match &model.cursor {
            Cursor::None => frame.set_cursor_position((0, frame_area.height - 1)),
            Cursor::Links(_) => {
                let (fg, bg) = (Color::Indexed(15), Color::Indexed(32));
                let line = if model.theme().hide_urls()
                    && let Some(selected_url) = selected_url
                {
                    let url_display = selected_url.as_ref().to_owned();
                    Line::from(vec![
                        Span::from(model.theme().link_url_open()).fg(bg),
                        Span::from(url_display).fg(fg).bg(bg),
                        Span::from(model.theme().link_url_close()).fg(bg),
                    ])
                } else {
                    Line::from(Span::from("Links").fg(Color::Indexed(32)))
                };
                let width = line.width() as u16;
                let searchbar = Paragraph::new(line);
                frame.render_widget(searchbar, Rect::new(0, frame_area.height - 1, width, 1));
                if cursor_positioned.is_none() {
                    frame.set_cursor_position((0, frame_area.height - 1));
                }
            }
            Cursor::Search(needle, _) => {
                let mut line = Line::default();
                line.spans.push(Span::from("/").fg(Color::Indexed(148)));
                let needle = Span::from(needle.clone()).fg(Color::Indexed(148));
                line.spans.push(needle);
                let width = line.width() as u16;
                let searchbar = Paragraph::new(line);
                frame.render_widget(searchbar, Rect::new(0, frame_area.height - 1, width, 1));
                frame.set_cursor_position((0, frame_area.height - 1));
            }
        },
        InputQueue::Search(needle) => {
            let mut line = Line::default();
            line.spans.push(Span::from("/").fg(Color::Indexed(148)));
            let needle = Span::from(needle.clone());
            line.spans.push(needle);
            let width = line.width() as u16;
            let searchbar = Paragraph::new(line);
            frame.render_widget(searchbar, Rect::new(0, frame_area.height - 1, width, 1));
            frame.set_cursor_position((width, frame_area.height - 1));
        }
        InputQueue::MovementCount(movement_count) => {
            let movement_count = movement_count.get();
            let mut line = Line::default();
            let mut span = Span::from(movement_count.to_string()).fg(Color::Indexed(250));
            if movement_count == u16::MAX {
                span = span.fg(Color::Indexed(167));
            }
            line.spans.push(span);
            let width = line.width() as u16;
            let searchbar = Paragraph::new(line);
            frame.render_widget(searchbar, Rect::new(0, frame_area.height - 1, width, 1));
            frame.set_cursor_position((width, frame_area.height - 1));
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
            config.theme.clone(),
            true,
            false,
            cmd_rx,
            event_tx,
            config.max_image_height,
        );

        let screen_size = (80, 20).into();

        let model = Model::new(None, cmd_tx, event_rx, screen_size, config);
        (model, worker, screen_size)
    }

    // Drop model so that cmd_rx gets closed and worker exits, then exit/join worker.
    fn teardown(model: Model, worker: JoinHandle<Result<(), Error>>) {
        drop(model);
        worker.join().unwrap().unwrap();
    }

    // Poll until parsed and no pending images.
    fn poll_parsed(model: &mut Model) {
        let mut fuse = 1000_000;
        loop {
            let (_, parse_done, _) = model.process_events().unwrap();
            if parse_done {
                break;
            }
            fuse -= 1;
            if fuse == 0 {
                panic!("fuse exhausted");
            }
        }
        log::debug!("poll_parsed completed");
    }

    // Poll until parsed and no pending images.
    fn poll_done(model: &mut Model) {
        while model.has_pending_images() {
            model.process_events().unwrap();
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
        poll_parsed(&mut model);
        terminal.draw(|frame| view(&model, frame)).unwrap();
        assert_snapshot!("first parse image previews", terminal.backend());
        // Must load an image.
        poll_done(&mut model);
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
        poll_parsed(&mut model);
        poll_done(&mut model);

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
        poll_parsed(&mut model);
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
        poll_parsed(&mut model);
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
        poll_parsed(&mut model);
        poll_done(&mut model);

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
        poll_parsed(&mut model);
        terminal.draw(|frame| view(&model, frame)).unwrap();
        assert_snapshot!("reload add image preview", terminal.backend());
        // Must load an image.
        poll_done(&mut model);
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
        poll_parsed(&mut model);
        poll_done(&mut model);

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
        poll_parsed(&mut model);
        terminal.draw(|frame| view(&model, frame)).unwrap();
        assert_snapshot!("duplicate image preview", terminal.backend());
        // Must load an image.
        poll_done(&mut model);
        terminal.draw(|frame| view(&model, frame)).unwrap();
        assert_snapshot!("duplicate image done", terminal.backend());
        teardown(model, worker);
    }
}
