mod big_text;
mod config;
mod cursor;
mod debug;
mod document;
mod error;
mod keybindings;
mod model;
mod setup;
mod view;
mod watch;
mod worker;

#[cfg(not(windows))]
use std::os::fd::IntoRawFd as _;

use std::{
    fmt::Display,
    fs::read_to_string,
    io::{self, Read as _, Write as _},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        OnceLock,
        mpsc::{self},
    },
};

use clap::{ArgMatches, arg, command, value_parser};
use ratatui::{
    DefaultTerminal, Terminal,
    crossterm::{
        event::{DisableMouseCapture, EnableMouseCapture},
        tty::IsTty as _,
    },
    layout::Size,
    prelude::CrosstermBackend,
};

use mdfrier::MarkdownLink;
use ratatui_image::{picker::ProtocolType, protocol::Protocol, sliced::SlicedProtocol};
use reqwest::header::CONTENT_TYPE;
use setup::{SetupResult, setup_graphics};

use crate::{
    config::Config,
    document::{Section, SectionID},
    error::Error,
    keybindings::PollResult,
    model::{DocumentId, Model},
    view::view,
    watch::watch,
    worker::{ImageCache, worker_thread},
};

const OK_END: &str = " ok.";

static VERSION: OnceLock<String> = OnceLock::new();

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
            arg!(--"log-to-stderr" "Log to stderr.\nMust be used with stderr redirection, otherwise garbled text will appear.\nFor example, in another terminal, get and copy the filename of stdin with `tty`,\nlet's say it's `/dev/pts/7`. Then run:\nmdfried FILE.md --log-to-stderr 2>/dev/pts/7\nThe logs will appear nicely colored in the other terminal.")
                .value_parser(value_parser!(bool)),
        )
        .arg(
            arg!([source] "The markdown source.\nCan be a file path, a URL, a github repo in \"github:[owner]/[repo]\" format, or '-' or omit, for stdin")
        );
    let matches = cmd.get_matches_mut();

    if let Some(version) = cmd.get_version() {
        #[expect(unused_must_use)]
        VERSION.set(version.to_owned());
    }

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
        if let Err(err) = crossterm::terminal::disable_raw_mode() {
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

    debug::init_logger(*matches.get_one("log-to-stderr").unwrap_or(&false))?;

    let source: Option<String> = matches.get_one::<String>("source").cloned();

    let mut user_config = config::load_or_ask()?;
    let config = Config::from(user_config.clone());

    let (text, file_path, basepath) = match source {
        Some(source) if source == "-" => {
            let mut text = String::new();
            print!("Reading stdin...");
            io::stdin().read_to_string(&mut text)?;
            println!("{OK_END}");
            (text, None, None)
        }
        None => {
            if io::stdin().is_tty() {
                return Err(Error::Usage(Some(
                    "no source nor '-', and stdin is a tty (not a pipe)",
                )));
            }
            let mut text = String::new();
            print!("Reading stdin...");
            io::stdin().read_to_string(&mut text)?;
            println!("{OK_END}");
            (text, None, None)
        }
        Some(source) => open_source(&source, config.url_transform_command.clone())?,
    };

    if text.is_empty() {
        return Err(Error::Usage(Some("no input or empty")));
    }

    #[cfg(not(windows))]
    if !io::stdin().is_tty() {
        print!("Setting stdin to /dev/tty...");
        // Close the current stdin so that ratatui-image can read stuff from tty stdin.
        // SAFETY:
        // Calls some libc, not sure if this could be done otherwise.
        unsafe {
            // Attempt to open /dev/tty which will give us a new stdin
            let tty = std::fs::File::open("/dev/tty")?;

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
        file_path.clone()
    } else {
        None
    };

    let (cmd_tx, cmd_rx) = mpsc::channel::<Cmd>();
    let (event_tx, event_rx) = mpsc::channel::<Event>();
    let watch_event_tx = event_tx.clone();

    let config_max_image_height = config.max_image_height;
    let mut worker_theme = config.theme.clone();
    worker_theme.has_text_size_protocol = Some(has_text_size_protocol);
    let cmd_thread = worker_thread(
        basepath,
        picker,
        renderer,
        worker_theme,
        deep_fry,
        cmd_rx,
        event_tx,
        config_max_image_height,
    );

    crossterm::terminal::enable_raw_mode()?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;
    let enable_mouse_capture = config.enable_mouse_capture;
    if enable_mouse_capture {
        crossterm::execute!(io::stderr(), EnableMouseCapture)?;
    }
    let watch_debounce_milliseconds = config.watch_debounce_milliseconds;
    terminal.clear()?;

    let terminal_size = terminal.size()?;
    let model = Model::new(file_path, cmd_tx, event_rx, terminal.size()?, config);
    model.open(terminal_size, text)?;

    let debouncer = if let Some(path) = watchmode_path {
        log::info!("watching file");
        Some(watch(&path, watch_event_tx, watch_debounce_milliseconds)?)
    } else {
        drop(watch_event_tx);
        None
    };

    run(&mut terminal, model)?;
    drop(debouncer);

    // Cursor might be in wird places, prompt or whatever should always show at the bottom now.
    terminal.set_cursor_position((0, terminal_size.height - 1))?;

    if enable_mouse_capture {
        crossterm::execute!(io::stderr(), DisableMouseCapture)?;
    }
    crossterm::terminal::disable_raw_mode()?;

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
    ImageLoaded(DocumentId, SectionID, MarkdownLink, (SlicedProtocol, Size)),
    ImageFailed(DocumentId, SectionID, String, String),
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

            Event::ImageFailed(document_id, section_id, url, error) => {
                write!(
                    f,
                    "Event::ImageFailed({document_id}, {section_id}, {url}, {error})"
                )
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

fn run(terminal: &mut DefaultTerminal, mut model: Model) -> Result<(), Error> {
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
            terminal.draw(|frame| {
                view(&model, frame);
                if let Some(snapshot) = &mut model.log_snapshot {
                    debug::update_snapshot(snapshot);
                    debug::render_snapshot(snapshot, frame);
                }
            })?;
        }
    }
}

fn open_source(
    source: &str,
    url_transform_command: Option<String>,
) -> Result<(String, Option<PathBuf>, Option<PathBuf>), Error> {
    use ghrepo::GHRepo;
    use url::Url;

    log::debug!("source: {source:?}");
    use core::str::FromStr as _;
    if let Some(handle) = source.strip_prefix("github:")
        && let Ok(repo) = GHRepo::from_str(handle)
    {
        // We only want to try github if the user explicitly prefixed with "github:...".
        // Otherwise a path like "dir/file.md" would be a valid GHRepo and cause a useless request.
        let owner = repo.owner();
        let name = repo.name();
        let client = reqwest::blocking::Client::builder()
            .user_agent(format!(
                "mdfried/{}",
                VERSION.get().unwrap_or(&"unknown".to_owned())
            ))
            .build()?;
        for branch in ["master", "main"] {
            let url = format!(
                "https://raw.githubusercontent.com/{owner}/{name}/refs/heads/{branch}/README.md"
            );
            log::info!("trying github URL: {url}");
            print!("Fetching URL {url}...");
            let response = client.get(&url).send()?;
            if response.status().is_success() {
                println!("{OK_END}");
                return Ok((response.text()?, None, None));
            } else {
                println!("error.");
            }
        }
        return Err(Error::Io(io::Error::other(format!(
            "failed to request https://raw.githubusercontent.com/{owner}/{name}/refs/heads/[master|main]/README.md"
        ))));
    } else if let Ok(url) = Url::parse(source) {
        log::info!("requesting URL: {url}");
        print!("Fetching URL {url}...");
        let client = reqwest::blocking::Client::builder()
            .user_agent(format!(
                "mdfried/{}",
                VERSION.get().unwrap_or(&"unknown".to_owned())
            ))
            .build()?;
        let response = client.get(url.as_ref()).send()?;
        if response.status().is_success() {
            println!("{OK_END}");
            log::debug!(
                "have url_transform_command? {}, content_type: {:?}",
                url_transform_command.is_some(),
                response.headers().get(CONTENT_TYPE)
            );
            if let Some(url_transform_command) = url_transform_command
                && let Some(content_type) = response.headers().get(CONTENT_TYPE)
                && content_type
                    .to_str()
                    .map_err(|err| {
                        Error::Io(io::Error::other(format!("content-type error: {err}")))
                    })?
                    .starts_with("text/html")
            {
                let mut child = Command::new("sh")
                    .arg("-c")
                    .arg(url_transform_command)
                    .env("URL", url.as_ref())
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .spawn()?;

                let Some(stdin) = child.stdin.as_mut() else {
                    return Err(Error::Io(io::Error::other(
                        "url_transform_command pipe error",
                    )));
                };
                stdin.write_all(response.text()?.as_bytes())?;

                let output = child.wait_with_output()?;

                return Ok((
                    String::from_utf8(output.stdout)
                        .map_err(|_err| Error::Io(io::Error::other("response not utf-8")))?,
                    None,
                    None,
                ));
            }
            return Ok((response.text()?, None, None));
        } else {
            println!("error.");
            return Err(Error::Io(io::Error::other(format!(
                "failed to request {url}"
            ))));
        }
    }

    let path = PathBuf::from(source);
    let basepath = path.parent().map(Path::to_path_buf);
    Ok((read_to_string(&path)?, Some(path), basepath))
}

#[cfg(test)]
#[expect(clippy::unwrap_used)]
mod tests {
    use std::{sync::mpsc, thread::JoinHandle};

    #[cfg(not(target_os = "macos"))]
    use insta::assert_snapshot;
    use ratatui::{Terminal, backend::TestBackend, layout::Size};
    use ratatui_image::picker::{Picker, ProtocolType};

    use crate::{
        Cmd, Event,
        config::{Config, UserConfig},
        error::Error,
        model::Model,
        view::view,
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
        let mut worker_theme = config.theme.clone();
        worker_theme.has_text_size_protocol = Some(true);
        let worker = worker_thread(
            None,
            picker,
            None,
            worker_theme,
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
    #[track_caller]
    fn poll_parsed(model: &mut Model) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        loop {
            let (_, parse_done, _) = model.process_events().unwrap();
            if parse_done {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for process_events to be done"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        log::debug!("poll_parsed completed");
    }

    // Poll until parsed and no pending images.
    fn poll_images_done(model: &mut Model) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        while model.has_pending_images() {
            model.process_events().unwrap();
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for has_pending_images to be done"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
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

# Another header
Some text

# Last bit
Goodbye."#,
                ),
            )
            .unwrap();
        poll_parsed(&mut model);
        terminal.draw(|frame| view(&model, frame)).unwrap();
        #[cfg(not(target_os = "macos"))]
        assert_snapshot!("first parse image previews", terminal.backend());
        // Must load an image.
        poll_images_done(&mut model);
        terminal.draw(|frame| view(&model, frame)).unwrap();
        #[cfg(not(target_os = "macos"))]
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
        poll_images_done(&mut model);

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
        log::debug!("poll_parsed before failing done");
        terminal.draw(|frame| view(&model, frame)).unwrap();
        #[cfg(not(target_os = "macos"))]
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
        #[cfg(not(target_os = "macos"))]
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
        poll_images_done(&mut model);

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
        #[cfg(not(target_os = "macos"))]
        assert_snapshot!("reload add image preview", terminal.backend());
        // Must load an image.
        poll_images_done(&mut model);
        terminal.draw(|frame| view(&model, frame)).unwrap();
        #[cfg(not(target_os = "macos"))]
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
        poll_images_done(&mut model);

        model
            .reparse(
                screen_size,
                String::from(
                    r#"# Hello
![image A](./assets/NixOS.png)
Goodbye.
![image B](./assets/NixOS.png)"#,
                ),
            )
            .unwrap();
        poll_parsed(&mut model);
        terminal.draw(|frame| view(&model, frame)).unwrap();
        #[cfg(not(target_os = "macos"))]
        assert_snapshot!("duplicate image preview", terminal.backend());
        // Must load an image.
        poll_images_done(&mut model);
        terminal.draw(|frame| view(&model, frame)).unwrap();
        #[cfg(not(target_os = "macos"))]
        assert_snapshot!("duplicate image done", terminal.backend());
        teardown(model, worker);
    }
}
