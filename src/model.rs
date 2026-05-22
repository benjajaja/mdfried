use std::{
    cmp::min,
    fmt::Display,
    fs,
    num::NonZero,
    path::Path,
    sync::mpsc::{Receiver, Sender},
};

use mdfrier::SourceContent;
use ratatui::{
    layout::{Rect, Size},
    style::{Color, Stylize as _},
    text::{Line, Span},
    widgets::Padding,
};
use regex::RegexBuilder;
use url::Url;

use crate::{
    Cmd,
    config::{Config, PaddingConfig, Theme},
    cursor::{Cursor, CursorPointer},
    document::{Document, FindMode, FindTarget, LineExtra, Section, SectionContent},
    error::Error,
    sources::{DocumentHistoryEntry, DocumentSource, extend_url, github_usercontent_url},
    worker::ImageCache,
};
use crate::{Event, sources::SharedDocumentSource};

pub struct Model {
    pub scroll: u16,
    pub cursor: Cursor,
    pub input_queue: InputQueue,
    pub screen_size: Size,
    document: Document,
    document_id: DocumentId,
    document_source: SharedDocumentSource,
    document_history: Vec<DocumentHistoryEntry>,
    config: Config,
    cmd_tx: Sender<Cmd>,
    event_rx: Receiver<Event>,
}

// The temporary keypress input queue for operations like search or movement-count prefix.
// Stored in model, but the model methods should usually not be responsible for changing it.
#[derive(PartialEq)]
pub enum InputQueue {
    None,
    MovementCount(NonZero<u16>),
    Search(String),
    CursorPositioningCommands,
    Command(String),
}
impl InputQueue {
    // Convenience for model "cursor_find" method. Consumes the input, resets self to
    // `InputQueue::None`.
    pub fn take_count_or_unit_u16(&mut self) -> u16 {
        self.take_count()
            .unwrap_or(NonZero::new(1).expect("NonZero::new(1)"))
            .get()
    }
    // Convenience for model "scroll" methods. Consumes the input, resets self to
    // `InputQueue::None`.
    pub fn take_count_or_unit_i32(&mut self) -> i32 {
        self.take_count()
            .unwrap_or(NonZero::new(1).expect("NonZero::new(1)"))
            .get()
            .into()
    }
    // Consumes the input, resets self to `InputQueue::None`.
    fn take_count(&mut self) -> Option<NonZero<u16>> {
        if let InputQueue::MovementCount(count) = self {
            let icount = *count;
            *self = InputQueue::None;
            Some(icount)
        } else {
            None
        }
    }
}

impl Model {
    pub fn new(
        document_source: SharedDocumentSource,
        cmd_tx: Sender<Cmd>,
        event_rx: Receiver<Event>,
        screen_size: Size,
        config: Config,
    ) -> Model {
        Model {
            screen_size,
            config,
            scroll: 0,
            input_queue: InputQueue::None,
            cursor: Cursor::default(),
            document: Document::default(),
            document_id: DocumentId::default(),
            document_source,
            document_history: Vec::new(),
            cmd_tx,
            event_rx,
        }
    }

    #[cfg(test)]
    pub fn has_pending_images(&self) -> bool {
        self.document.has_pending_images()
    }

    pub fn reload(&mut self, screen_size: Size) -> Result<(), Error> {
        let old_width = self.config.padding.calculate_width(self.screen_size.width);
        self.screen_size = screen_size;
        log::debug!("reload on {:?}", self.document_source.read()?);
        let text = match self.document_source.read()? {
            DocumentSource::File { path, .. } => fs::read_to_string(path)?,
            DocumentSource::Stdin { mut text } => text.take().ok_or(Error::Thread(
                "reload on stdin while processing text".to_owned(),
            ))?,
            _source => todo!("reload for other sources: {_source:?}"),
        };
        self.reparse(text, old_width)
    }

    pub fn open(&self, text: String) -> Result<(), Error> {
        self.parse(self.document_id.open(), text, None)
    }

    fn open_new_source(&mut self, source: DocumentSource, text: String) -> Result<(), Error> {
        self.document_history.push(DocumentHistoryEntry {
            source: self.document_source.read()?,
            document: std::mem::take(&mut self.document), // resets self.document to default
            scroll: self.scroll,
        });
        self.document_source.write(source)?;
        self.cursor = Cursor::None;
        self.scroll = 0;
        self.input_queue = InputQueue::None;
        self.open(text)
    }

    pub fn history_pop(&mut self) -> Result<(), Error> {
        let DocumentHistoryEntry {
            source,
            document,
            scroll,
        } = self
            .document_history
            .pop()
            .ok_or(Error::Generic("No history to go back".to_owned()))?;
        self.document_source.write(source)?;
        self.document = document;
        self.cursor = Cursor::None;
        self.scroll = scroll;
        self.input_queue = InputQueue::None;

        Ok(())
    }

    pub fn reparse(&mut self, text: String, old_width: u16) -> Result<(), Error> {
        log::info!("reparse with {:?}", self.screen_size);
        let image_cache = self.document.take_image_protocols(old_width);
        let cache = if image_cache.is_empty() {
            None
        } else {
            Some(image_cache)
        };
        self.document = Document::default();
        self.parse(self.document_id.reload(), text, cache)
    }

    fn parse(
        &self,
        next_document_id: DocumentId,
        mut text: String,
        image_cache: Option<ImageCache>,
    ) -> Result<(), Error> {
        let inner_width = self.config.padding.calculate_width(self.screen_size.width);
        if !text.ends_with('\n') {
            // mdfrier needs this, either because of its own limitation or something with
            // tree-sitter-md. Doesn't really matter as long as we're reading a file.
            text.push('\n');
        }
        self.cmd_tx
            .send(Cmd::Parse(next_document_id, inner_width, text, image_cache))?;
        Ok(())
    }

    pub fn inner_height(&self) -> u16 {
        self.config
            .padding
            .calculate_height(self.screen_size.height)
            .saturating_sub(1) // Account for the status line at the bottom.
    }

    pub fn block_padding(&self, area: Rect) -> Padding {
        match self.config.padding {
            PaddingConfig::None => Padding::default(),
            PaddingConfig::Centered(width) => Padding::horizontal(
                area.width
                    .checked_sub(width)
                    .map(|padding| padding / 2)
                    .unwrap_or_default(),
            ),
        }
    }

    pub fn total_lines(&self) -> u16 {
        self.document.iter().map(|s| s.height).sum()
    }

    pub fn process_events(&mut self) -> Result<(bool, bool, bool), Error> {
        let mut had_events = false;
        let mut had_done = false;
        let mut had_reload = false;
        while let Ok(event) = self.event_rx.try_recv() {
            had_events = true;

            if !matches!(event, Event::Parsed(_, _)) {
                log::debug!("{event}");
            }

            match event {
                Event::NewDocument(document_id) => {
                    log::info!("NewDocument {document_id}");
                    self.document_id = document_id;
                }
                Event::ParseDone(document_id, last_section_id, text) => {
                    if !self.document_id.is_same_document(&document_id) {
                        log::debug!("stale event, ignoring");
                        continue;
                    }
                    self.document.trim(last_section_id);
                    self.reload_search();
                    if let Some(updated) = self.document_source.read()?.return_text(text) {
                        self.document_source.write(updated)?;
                    }
                    had_done = true;
                }
                Event::Parsed(document_id, section) => {
                    if !self.document_id.is_same_document(&document_id) {
                        log::debug!("stale event, ignoring");
                        continue;
                    }

                    debug_assert!(
                        !matches!(section.content, SectionContent::Image(_, _, _, _),),
                        "unexpected Event::Parsed with Image: {:?}",
                        section.content
                    );

                    self.document.push(section);
                }
                Event::ImageLoaded(document_id, section_id, link, protos) => {
                    if !self.document_id.is_same_document(&document_id) {
                        log::debug!("stale event, ignoring");
                        continue;
                    }
                    self.document.update_image(section_id, link, protos);
                }
                Event::ImageFailed(document_id, section_id, _url, error) => {
                    if !self.document_id.is_same_document(&document_id) {
                        log::debug!("stale event, ignoring");
                        continue;
                    }
                    self.document.update(vec![Section {
                        id: section_id,
                        height: 2,
                        content: SectionContent::Lines(vec![
                            (
                                Line::from(vec![
                                    Span::from("["),
                                    Span::from(error),
                                    Span::from("]"),
                                ])
                                .fg(Color::DarkGray),
                                vec![],
                            ),
                            (Line::from(""), vec![]),
                        ]),
                    }]);
                }
                Event::HeaderLoaded(document_id, section_id, rows) => {
                    if !self.document_id.is_same_document(&document_id) {
                        log::debug!("stale event, ignoring");
                        continue;
                    }
                    self.document.update_header(section_id, rows);
                }
                Event::ReferenceDefinition { id, url } => {
                    self.document.update_link_references(id, &url);
                }
                Event::FileChanged => {
                    log::info!("reload: FileChanged");
                    self.reload(self.screen_size)?;
                    had_reload = true;
                }
                Event::Scroll(delta) => {
                    self.scroll = self.scroll.saturating_add_signed(delta);
                }
                Event::NewSourceContent(text) => {
                    self.open_new_source(self.document_source.read()?, text)?;
                }
            }
        }
        Ok((had_events, had_done, had_reload))
    }

    fn reload_search(&mut self) {
        let old_cursor = std::mem::take(&mut self.cursor);
        match old_cursor {
            Cursor::None => {}
            Cursor::Links(_) => {
                // TODO: Search state is lost on reparse
                // and can't be reliably restored after resize
                // due to possibly different line breaks.
                // Might be fixed after search over line breaks is implemented.
            }
            Cursor::Search(needle, _) => {
                // TODO: See above
                self.add_searches(Some(&needle));
                self.cursor = Cursor::Search(needle, None);
            }
        }
    }

    /// Returns false the scroll did not change.
    pub fn scroll_by(&mut self, lines: i32) -> bool {
        let new_scroll = (self.scroll as u32)
            .saturating_add_signed(lines)
            .min(u16::MAX as u32) as u16;

        if new_scroll == self.scroll {
            return false;
        }

        self.scroll = min(
            new_scroll,
            self.total_lines().saturating_sub(self.inner_height()),
        );
        true
    }

    pub fn visible_lines(&self) -> (i16, i16) {
        let start_y = self.scroll as i16;
        let end_y = start_y + (self.inner_height() as i16).saturating_sub(1);
        (start_y, end_y)
    }

    pub fn open_link(&mut self, link_url: String) -> Result<(), Error> {
        if let Some(header_reference) = link_url.strip_prefix("#") {
            let pointer = {
                let mut target = None;
                for Section {
                    id,
                    content,
                    height,
                } in self.document.iter()
                {
                    if let SectionContent::Header(text, _, _) = content {
                        // Is this `#kebab-case` the only scheme? Probably not.
                        if text.to_lowercase().replace(' ', "-") == header_reference {
                            let Some(y) = self.document.get_y(&CursorPointer { id: *id, index: 0 })
                            else {
                                return Err(Error::Generic(format!(
                                    "Header position not found: {}",
                                    link_url
                                )));
                            };
                            target = Some((y, 0));
                        }
                    }
                    if let Some((_, target_height)) = &mut target {
                        *target_height += height;
                    }
                }
                target
            };
            let Some((y, remaining_document_height)) = pointer else {
                return Err(Error::Generic(format!(
                    "Header link not found: {}",
                    link_url
                )));
            };

            self.cursor = Cursor::None;
            self.scroll = y as u16;
            if remaining_document_height < self.inner_height() {
                self.scroll -= self.inner_height() - remaining_document_height;
            }
            return Ok(());
        }

        match self.document_source.read()? {
            source @ DocumentSource::File { .. }
            | source @ DocumentSource::Stdin { .. }
            | source @ DocumentSource::BuiltInHelp => {
                let url_as_path = Path::new(&link_url);
                if url_as_path.extension() == Some(std::ffi::OsStr::new("md"))
                    && fs::exists(url_as_path).unwrap_or_default()
                    && let Ok(text) = fs::read_to_string(url_as_path)
                {
                    self.open_new_source(source, text)?;
                }

                if let Err(err) = open::that(&link_url) {
                    log::error!("{err}");
                }
            }
            DocumentSource::Github { repo, branch } => {
                if Url::parse(&link_url).is_ok() {
                    if let Err(err) = open::that(&link_url) {
                        log::error!("{err}");
                    }
                } else {
                    let url = github_usercontent_url(&repo, &branch, &link_url)?;
                    self.cmd_tx.send(Cmd::OpenUrl(url))?;
                }
            }
            DocumentSource::HyperText { url } => {
                if Url::parse(&link_url).is_ok() {
                    if let Err(err) = open::that(&link_url) {
                        log::error!("{err}");
                    }
                } else {
                    let url = extend_url(url, &link_url)?;
                    self.cmd_tx.send(Cmd::OpenUrl(url))?;
                }
            }
        }
        Ok(())
    }

    /// Returns the URL of the currently selected link, if any.
    pub fn selected_link_url(&self, pointer: &CursorPointer) -> Option<SourceContent> {
        self.url_at_pointer(pointer)
    }

    /// Returns the URL at a given cursor pointer, if it points to a link.
    fn url_at_pointer(&self, pointer: &CursorPointer) -> Option<SourceContent> {
        self.document.iter().find_map(|section| {
            if section.id == pointer.id {
                let SectionContent::Lines(lines) = &section.content else {
                    return None;
                };
                let mut remaining = pointer.index;
                for (_, extras) in lines {
                    if remaining < extras.len() {
                        let LineExtra::Link { source: url, .. } = &extras[remaining] else {
                            return None;
                        };
                        return Some(url.clone());
                    }
                    remaining -= extras.len();
                }
                None
            } else {
                None
            }
        })
    }

    pub fn cursor_next(&mut self, count: u16) {
        self.cursor_find(
            NonZero::new(count).expect("cursor_next expects NonZero raw u16"),
            FindMode::Next,
        )
    }

    pub fn cursor_prev(&mut self, count: u16) {
        self.cursor_find(
            NonZero::new(count).expect("cursor_prev expects NonZero raw u16"),
            FindMode::Prev,
        )
    }

    fn cursor_find(&mut self, count: NonZero<u16>, mode: FindMode) {
        let mut recurse = true; // TODO: make Search + pointer-None work the same way.

        // For Links cursor, get current URL and pointer before the match to avoid borrow issues
        let (current_url, start_pointer) = if let Cursor::Links(current) = &self.cursor {
            (self.url_at_pointer(current).clone(), Some(current.clone()))
        } else {
            (None, None)
        };

        match &mut self.cursor {
            Cursor::None => {
                if let Some(pointer) =
                    Document::find_first_cursor(self.document.iter(), FindTarget::Link, self.scroll)
                {
                    self.cursor = Cursor::Links(pointer);
                }
            }
            Cursor::Links(current) => {
                if let Some(mut pointer) = Document::find_nth_next_cursor(
                    self.document.iter(),
                    current,
                    mode,
                    FindTarget::Link,
                    count,
                ) {
                    // Skip link parts with the same URL (for wrapped URLs)
                    if let (Some(current_url), Some(start)) = (&current_url, &start_pointer) {
                        while self.url_at_pointer(&pointer).is_some_and(|source_content| {
                            source_content.as_ptr() == current_url.as_ptr()
                        }) && &pointer != start
                        {
                            if let Some(next) = Document::find_nth_next_cursor(
                                self.document.iter(),
                                &pointer,
                                mode,
                                FindTarget::Link,
                                NonZero::new(1).expect("NonZero 1 for find_nth_next_cursor"),
                            ) {
                                if &next == start {
                                    // Wrapped around to start, no different URL found
                                    break;
                                }
                                pointer = next;
                            } else {
                                break;
                            }
                        }
                    }
                    self.cursor = Cursor::Links(pointer);
                    recurse = false;
                }
            }
            Cursor::Search(_, pointer) => match pointer {
                None => {
                    *pointer = Document::find_first_cursor(
                        self.document.iter(),
                        FindTarget::Search,
                        self.scroll,
                    );
                    if pointer.is_none() {
                        recurse = false;
                    }
                }
                Some(current) => {
                    *pointer = Document::find_nth_next_cursor(
                        self.document.iter(),
                        current,
                        mode,
                        FindTarget::Search,
                        count,
                    );
                }
            },
        }
        if recurse && count.get() > 1 {
            let count = NonZero::new(count.get() - 1).expect("NonZero was > 1");
            return self.cursor_find(count, mode);
        }
        self.jump_to_pointer();
    }

    pub fn add_searches(&mut self, needle: Option<&str>) {
        let re = needle.and_then(|needle| {
            RegexBuilder::new(&regex::escape(needle))
                .case_insensitive(true)
                .build()
                .inspect_err(|err| log::error!("{err}"))
                .ok()
        });
        for section in self.document.iter_mut() {
            section.add_search(re.as_ref());
        }
    }

    fn jump_to_pointer(&mut self) {
        if let Some(pointer) = self.cursor.pointer() {
            if let Some(pointer_y) = self.document.get_y(pointer) {
                let (from, to) = self.visible_lines();
                if pointer_y > to {
                    self.scroll_by((pointer_y - to) as i32);
                } else if pointer_y < from {
                    self.scroll_by((pointer_y - from) as i32);
                }
            } else {
                log::error!("jump_to_pointer did not find Y for {pointer:?}");
            }
        } else {
            log::error!("jump_to_pointer without cursor / pointer");
        }
    }

    pub fn sections(&self) -> impl Iterator<Item = &Section> {
        self.document.iter()
    }

    pub fn theme(&self) -> &Theme {
        &self.config.theme
    }

    pub fn position_cursor(&mut self, positioning: CursorPositioning) {
        if let Some(pointer_y) = self.cursor.pointer().and_then(|p| self.document.get_y(p)) {
            let (from, to) = self.visible_lines();
            let by = match positioning {
                CursorPositioning::Top => pointer_y as i32 - from as i32,
                CursorPositioning::Center => pointer_y as i32 - (from + to) as i32 / 2,
                CursorPositioning::Bottom => pointer_y as i32 - to as i32,
            };
            self.scroll_by(by);
        } else {
            log::error!("jump_to_pointer without cursor / pointer");
        }
    }

    /// User has typed `:some_command<Enter>`.
    pub fn user_command_str(&mut self, command: String) -> Result<(), Error> {
        match command.as_str() {
            "help" => {
                const HELP_MD: &str = include_str!("../assets/help.md");
                self.open_new_source(DocumentSource::BuiltInHelp, String::from(HELP_MD))
            }
            "back" => self.history_pop(),
            _ => Err(Error::Generic("unknown command: {command}".to_owned())),
        }
    }

    pub fn is_help_screen(&self) -> Result<bool, Error> {
        Ok(self.document_source.read()? == DocumentSource::BuiltInHelp)
    }
}

#[derive(Default, Debug, PartialEq, Clone, Copy)]
pub struct DocumentId {
    id: usize, // Reserved for when we can open another file
    reload_id: usize,
}

impl DocumentId {
    fn is_same_document(&self, other: &DocumentId) -> bool {
        self.id == other.id
    }

    fn open(&self) -> DocumentId {
        DocumentId {
            id: self.id + 1,
            reload_id: 0,
        }
    }

    fn reload(&self) -> DocumentId {
        DocumentId {
            id: self.id,
            reload_id: self.reload_id + 1,
        }
    }
}

impl Display for DocumentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "D{}.{}", self.id, self.reload_id,)
    }
}

pub enum CursorPositioning {
    Top,
    Center,
    Bottom,
}

impl From<char> for CursorPositioning {
    fn from(value: char) -> Self {
        match value {
            't' => CursorPositioning::Top,
            'z' => CursorPositioning::Center,
            'b' => CursorPositioning::Bottom,
            _ => unreachable!("CursorPositioning from char must be 't', 'z', or 'b'"),
        }
    }
}

#[cfg(test)]
#[expect(clippy::unwrap_used)]
mod tests {

    use std::sync::mpsc;

    use mdfrier::SourceContent;
    use ratatui::text::Line;

    use crate::{
        Cmd, DocumentId, Event,
        config::UserConfig,
        cursor::{Cursor, CursorPointer},
        document::{Document, LineExtra, LinkReference, Section, SectionContent},
        model::{InputQueue, Model},
        sources::SharedDocumentSource,
    };

    /// Test model, 80x20 screen size.
    fn test_model() -> Model {
        let (cmd_tx, _) = mpsc::channel::<Cmd>();
        let (_, event_rx) = mpsc::channel::<Event>();
        Model {
            screen_size: (80, 20).into(),
            config: UserConfig::default().into(),
            scroll: 0,
            input_queue: InputQueue::None,
            cursor: Cursor::default(),
            document: Document::default(),
            cmd_tx,
            event_rx,
            document_id: DocumentId::default(),
            document_source: SharedDocumentSource::test(),
            document_history: Vec::new(),
        }
    }

    #[track_caller]
    fn assert_cursor_link(model: &Model, expected_url: &SourceContent) {
        let LineExtra::Link { source: url, .. } = model
            .document
            .find_extra_by_cursor(
                model
                    .cursor
                    .pointer()
                    .expect("model.cursor.pointer() should be Some"),
            )
            .expect("find_extra_by_cursor(...).unwrap()")
        else {
            panic!(
                "assert_link expected LineExtra::Link, is: {:?}",
                model
                    .cursor
                    .pointer()
                    .and_then(|p| model.document.find_extra_by_cursor(p))
            );
        };
        assert_eq!(url.as_ptr(), expected_url.as_ptr());
    }

    #[test]
    fn finds_link_per_line() {
        let mut model = test_model();
        let link_a = SourceContent::from("http://a.com");
        let link_b = SourceContent::from("http://b.com");
        let link_c = SourceContent::from("http://c.com");
        model.document.push(Section {
            id: 1,
            height: 1,
            content: SectionContent::Lines(vec![(
                Line::from("http://a.com http://b.com"),
                vec![
                    LineExtra::Link {
                        source: link_a.clone(),
                        start: 0,
                        end: 11,
                        lines: None,
                        reference: LinkReference::None,
                    },
                    LineExtra::Link {
                        source: link_b.clone(),
                        start: 12,
                        end: 21,
                        lines: None,
                        reference: LinkReference::None,
                    },
                ],
            )]),
        });
        model.document.push(Section {
            id: 2,
            height: 1,
            content: SectionContent::Lines(vec![(
                Line::from("http://c.com"),
                vec![LineExtra::Link {
                    source: link_c.clone(),
                    start: 0,
                    end: 11,
                    lines: None,
                    reference: LinkReference::None,
                }],
            )]),
        });

        model.cursor_next(1);
        assert_cursor_link(&model, &link_a);

        model.cursor_next(1);
        assert_cursor_link(&model, &link_b);

        model.cursor_next(1);
        assert_cursor_link(&model, &link_c);
    }

    #[test]
    fn finds_link_with_scroll() {
        let mut model = test_model();
        let mut links = Vec::new();
        for i in 1..5 {
            let url = format!("http://{}.com", i);
            let link = SourceContent::from(url.as_str());
            links.push(link.clone());
            model.document.push(Section {
                id: i,
                height: 1,
                content: SectionContent::Lines(vec![(
                    Line::from(url.clone()),
                    vec![LineExtra::Link {
                        source: link,
                        start: 0,
                        end: 11,
                        lines: None,
                        reference: LinkReference::None,
                    }],
                )]),
            });
        }

        model.scroll = 2;
        model.cursor_next(1);
        assert_cursor_link(&model, &links[2]);
    }

    #[test]
    fn finds_link_with_scroll_wrapping() {
        let mut model = test_model();
        let link = SourceContent::from("http://a.com");
        model.document.push(Section {
            id: 1,
            height: 1,
            content: SectionContent::Lines(vec![(
                Line::from("http://a.com"),
                vec![LineExtra::Link {
                    source: link.clone(),
                    start: 0,
                    end: 11,
                    lines: None,
                    reference: LinkReference::None,
                }],
            )]),
        });
        for i in 2..5 {
            model.document.push(Section {
                id: i,
                height: 1,
                content: SectionContent::Lines(vec![(Line::from("text"), vec![])]),
            });
        }

        model.scroll = 2;
        model.cursor_next(1);
        assert_cursor_link(&model, &link);
    }

    #[test]
    fn finds_multiple_links_per_line_next() {
        let mut model = test_model();
        let link_a = SourceContent::from("http://a.com");
        let link_b = SourceContent::from("http://b.com");
        let link_c = SourceContent::from("http://c.com");
        model.document.push(Section {
            id: 1,
            height: 1,
            content: SectionContent::Lines(vec![(
                Line::from("http://a.com http://b.com"),
                vec![
                    LineExtra::Link {
                        source: link_a.clone(),
                        start: 0,
                        end: 11,
                        lines: None,
                        reference: LinkReference::None,
                    },
                    LineExtra::Link {
                        source: link_b.clone(),
                        start: 12,
                        end: 21,
                        lines: None,
                        reference: LinkReference::None,
                    },
                ],
            )]),
        });
        model.document.push(Section {
            id: 2,
            height: 1,
            content: SectionContent::Lines(vec![(
                Line::from("http://c.com"),
                vec![LineExtra::Link {
                    source: link_c.clone(),
                    start: 0,
                    end: 11,
                    lines: None,
                    reference: LinkReference::None,
                }],
            )]),
        });

        model.cursor_next(1);
        assert_cursor_link(&model, &link_a);

        model.cursor_next(1);
        assert_cursor_link(&model, &link_b);

        model.cursor_next(1);
        assert_cursor_link(&model, &link_c);
    }

    #[test]
    fn finds_multiple_links_per_line_prev() {
        let mut model = test_model();
        let link_a = SourceContent::from("http://a.com");
        let link_b = SourceContent::from("http://b.com");
        let link_c = SourceContent::from("http://c.com");
        model.document.push(Section {
            id: 1,
            height: 1,
            content: SectionContent::Lines(vec![(
                Line::from("http://a.com http://b.com"),
                vec![
                    LineExtra::Link {
                        source: link_a.clone(),
                        start: 0,
                        end: 11,
                        lines: None,
                        reference: LinkReference::None,
                    },
                    LineExtra::Link {
                        source: link_b.clone(),
                        start: 12,
                        end: 21,
                        lines: None,
                        reference: LinkReference::None,
                    },
                ],
            )]),
        });
        model.document.push(Section {
            id: 2,
            height: 1,
            content: SectionContent::Lines(vec![(
                Line::from("http://c.com"),
                vec![LineExtra::Link {
                    source: link_c.clone(),
                    start: 0,
                    end: 11,
                    lines: None,
                    reference: LinkReference::None,
                }],
            )]),
        });

        model.cursor_prev(1);
        assert_cursor_link(&model, &link_a);

        model.cursor_prev(1);
        assert_cursor_link(&model, &link_c);

        model.cursor_prev(1);
        assert_cursor_link(&model, &link_b);
    }

    #[test]
    fn jump_to_pointer() {
        let mut model = test_model();
        for i in 0..31 {
            model.document.push(Section {
                id: i,
                height: 1,
                content: SectionContent::Lines(vec![(
                    Line::from(format!("line {}", i + 1)),
                    Vec::new(),
                )]),
            });
        }

        // Just outside of view (test terminal height is 20, we don't render on last line)
        model.cursor = Cursor::Search(String::new(), Some(CursorPointer { id: 19, index: 0 }));
        model.jump_to_pointer();
        assert_eq!(model.scroll, 1);

        model.scroll = 0;
        // Towards the end
        model.cursor = Cursor::Search(String::new(), Some(CursorPointer { id: 30, index: 0 }));
        model.jump_to_pointer();
        assert_eq!(model.scroll, 12);
    }

    #[test]
    fn jump_back_to_pointer() {
        let mut model = test_model();
        for i in 0..31 {
            model.document.push(Section {
                id: i,
                height: 1,
                content: SectionContent::Lines(vec![(
                    Line::from(format!("line {}", i + 1)),
                    Vec::new(),
                )]),
            });
        }

        model.scroll = 12;
        model.cursor = Cursor::Search(String::new(), Some(CursorPointer { id: 0, index: 0 }));
        model.jump_to_pointer();
        assert_eq!(model.scroll, 0);
    }

    #[test]
    fn scrolls_into_view() {
        let mut model = test_model();
        for i in 0..30 {
            model.document.push(Section {
                id: i,
                height: 1,
                content: SectionContent::Lines(vec![(
                    Line::from(format!("line {}", i + 1)),
                    Vec::new(),
                )]),
            });
        }
        let link = SourceContent::from("http://a.com");
        model.document.push(Section {
            id: 30,
            height: 1,
            content: SectionContent::Lines(vec![(
                Line::from("http://a.com"),
                vec![LineExtra::Link {
                    source: link.clone(),
                    start: 0,
                    end: 11,
                    lines: None,
                    reference: LinkReference::None,
                }],
            )]),
        });

        model.cursor_next(1);
        assert_cursor_link(&model, &link);

        assert_eq!(model.scroll, 12);
        assert_eq!(model.visible_lines(), (12, 30));

        let mut last_rendered = None;
        let mut y: i16 = 0 - (model.scroll as i16);
        for source in model.document.iter() {
            y += source.height as i16;
            if y >= model.inner_height() as i16 {
                last_rendered = Some(source);
                break;
            }
        }
        let last_rendered = last_rendered.unwrap();
        let SectionContent::Lines(lines) = &last_rendered.content else {
            panic!("expected Line");
        };
        let LineExtra::Link { source: url, .. } = &lines[0].1[0] else {
            panic!("expected Link");
        };
        assert_eq!("http://a.com", url.as_ref());
    }

    #[test]
    fn finds_links_with_count() {
        let mut model = test_model();
        let mut links = Vec::new();
        for i in 1..10 {
            let url = format!("http://{}.com", i);
            let link = SourceContent::from(url.as_str());
            model.document.push(Section {
                id: i,
                height: 1,
                content: SectionContent::Lines(vec![(
                    Line::from(url),
                    vec![LineExtra::Link {
                        source: link.clone(),
                        start: 0,
                        end: 11,
                        lines: None,
                        reference: LinkReference::None,
                    }],
                )]),
            });
            links.push(link);
        }

        model.cursor_next(3);
        assert_cursor_link(&model, &links[2]);

        model.cursor_prev(2);
        assert_cursor_link(&model, &links[0]);

        model.cursor_prev(1);
        assert_cursor_link(&model, &links[8]);

        model.cursor_next(4);
        assert_cursor_link(&model, &links[3]);
    }
}
