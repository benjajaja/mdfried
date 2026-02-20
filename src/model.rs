use std::{
    cmp::min,
    fmt::Display,
    fs,
    num::NonZero,
    path::PathBuf,
    sync::mpsc::{Receiver, Sender},
};

use mdfrier::SourceContent;
use ratatui::{
    layout::{Rect, Size},
    style::Stylize as _,
    text::{Line, Span},
    widgets::Padding,
};
use regex::RegexBuilder;

use crate::{
    Cmd,
    config::{Config, PaddingConfig, Theme},
    cursor::{Cursor, CursorPointer},
    document::{Document, FindMode, FindTarget, LineExtra, Section, SectionContent},
    error::Error,
};
use crate::{Event, MarkdownImage};

pub struct Model {
    pub scroll: u16,
    pub cursor: Cursor,
    pub input_queue: InputQueue,
    pub log_snapshot: Option<flexi_logger::Snapshot>,
    pub screen_size: Size,
    document: Document,
    document_id: DocumentId,
    original_file_path: Option<PathBuf>,
    config: Config,
    cmd_tx: Sender<Cmd>,
    event_rx: Receiver<Event>,
    can_render_headers: bool,
    #[cfg(test)]
    pub pending_image_count: usize,
}

// The temporary keypress input queue for operations like search or movement-count prefix.
// Stored in model, but the model methods should usually not be responsible for changing it.
#[derive(PartialEq)]
pub enum InputQueue {
    None,
    MovementCount(NonZero<u16>),
    Search(String),
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
        original_file_path: Option<PathBuf>,
        cmd_tx: Sender<Cmd>,
        event_rx: Receiver<Event>,
        screen_size: Size,
        config: Config,
        can_render_headers: bool,
    ) -> Model {
        Model {
            original_file_path,
            screen_size,
            config,
            scroll: 0,
            input_queue: InputQueue::None,
            cursor: Cursor::default(),
            document: Document::default(),
            cmd_tx,
            event_rx,
            can_render_headers,
            log_snapshot: None,
            document_id: DocumentId::default(),
            #[cfg(test)]
            pending_image_count: 0,
        }
    }

    pub fn reload(&mut self, screen_size: Size) -> Result<(), Error> {
        self.screen_size = screen_size;
        if let Some(original_file_path) = &self.original_file_path {
            let text = fs::read_to_string(original_file_path)?;
            self.reparse(screen_size, text)?;
        }
        Ok(())
    }

    pub fn open(&self, screen_size: Size, text: String) -> Result<(), Error> {
        self.parse(self.document_id.open(), screen_size, text)
    }

    pub fn reparse(&self, screen_size: Size, text: String) -> Result<(), Error> {
        log::info!("reparse");
        self.parse(self.document_id.reload(), screen_size, text)
    }

    fn parse(
        &self,
        next_document_id: DocumentId,
        screen_size: Size,
        mut text: String,
    ) -> Result<(), Error> {
        let inner_width = self.inner_width(screen_size.width);
        if !text.ends_with('\n') {
            // mdfrier needs this, either because of its own limitation or something with
            // tree-sitter-md. Doesn't really matter as long as we're reading a file.
            text.push('\n');
        }
        self.cmd_tx
            .send(Cmd::Parse(next_document_id, inner_width, text))?;
        Ok(())
    }

    pub fn inner_width(&self, screen_width: u16) -> u16 {
        self.config.padding.calculate_width(screen_width)
    }

    pub fn inner_height(&self, screen_height: u16) -> u16 {
        self.config.padding.calculate_height(screen_height)
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

    pub fn process_events(&mut self, screen_width: u16) -> Result<(bool, bool), Error> {
        let inner_width = self.inner_width(screen_width);
        let mut had_events = false;
        let mut had_done = false;
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
                Event::ParseDone(document_id, last_section_id) => {
                    if !self.document_id.is_same_document(&document_id) {
                        log::debug!("stale event, ignoring");
                        continue;
                    }
                    self.document.trim(last_section_id);
                    self.reload_search();
                    had_done = true;
                }
                Event::Parsed(document_id, section) => {
                    if !self.document_id.is_same_document(&document_id) {
                        log::debug!("stale event, ignoring");
                        continue;
                    }

                    debug_assert!(
                        !matches!(section.content, SectionContent::Image(_, _),),
                        "unexpected Event::Parsed with Image: {:?}",
                        section.content
                    );

                    if self.document_id.is_first_load() {
                        self.document.push(section);
                    } else {
                        self.document.update(vec![section]);
                    }
                }
                Event::Update(document_id, updates) => {
                    if !self.document_id.is_same_document(&document_id) {
                        log::debug!("stale event, ignoring");
                        continue;
                    }
                    #[cfg(test)]
                    for section in &updates {
                        if let SectionContent::Image(_, _) = section.content {
                            log::debug!("Update #{}: {:?}", section.id, section.content);
                            self.pending_image_count -= 1;
                        }
                    }
                    self.document.update(updates);
                }
                Event::ParsedImage(
                    document_id,
                    id,
                    MarkdownImage {
                        destination: link_destination,
                        description: image_description,
                    },
                ) => {
                    if !self.document_id.is_same_document(&document_id) {
                        log::debug!("stale event, ignoring");
                        continue;
                    }

                    if let Some(mut existing_image) = self.document.replace(id, &link_destination) {
                        log::debug!("replacing from existing image ({link_destination})");
                        existing_image.id = id;
                        self.document.update(vec![existing_image]);
                    } else {
                        if self.document_id.is_first_load() {
                            log::debug!(
                                "existing image not found, push placeholder and process image ({link_destination})"
                            );
                            self.document.push(Section {
                                id,
                                height: 1,
                                content: SectionContent::Line(
                                    Line::from(format!("![Loading...]({link_destination})")),
                                    Vec::new(),
                                ),
                            });
                        } else {
                            log::debug!(
                                "existing image not found, update placeholder and process image ({link_destination})"
                            );
                            self.document.update(vec![Section {
                                id,
                                height: 1,
                                content: SectionContent::Line(
                                    Line::from(format!("![Loading...]({link_destination})")),
                                    Vec::new(),
                                ),
                            }]);
                        }
                        #[cfg(test)]
                        {
                            log::debug!("UrlImage");
                            self.pending_image_count += 1;
                        }
                        self.cmd_tx.send(Cmd::UrlImage(
                            document_id,
                            id,
                            inner_width,
                            link_destination,
                            image_description,
                        ))?;
                    }
                }
                Event::ParseHeader(document_id, id, tier, text) => {
                    if !self.document_id.is_same_document(&document_id) {
                        log::debug!("stale event, ignoring");
                        continue;
                    }
                    let line = Line::from(vec![
                        #[expect(clippy::string_add)]
                        Span::from("#".repeat(tier as usize) + " ").light_blue(),
                        Span::from(text.clone()),
                    ]);
                    if self.document_id.is_first_load() {
                        self.document.push(Section {
                            id,
                            height: 2,
                            content: SectionContent::Line(line, Vec::new()),
                        });
                    } else {
                        self.document.update(vec![Section {
                            id,
                            height: 2,
                            content: SectionContent::Line(line, Vec::new()),
                        }]);
                    }
                    #[cfg(test)]
                    {
                        log::debug!("ParseHeader");
                        self.pending_image_count += 1;
                    }
                    if self.can_render_headers {
                        self.cmd_tx
                            .send(Cmd::Header(document_id, id, inner_width, tier, text))?;
                    }
                }
                Event::FileChanged => {
                    log::info!("reload: FileChanged");
                    self.reload(self.screen_size)?;
                }
            }
        }
        Ok((had_events, had_done))
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

    pub fn scroll_by(&mut self, lines: i32) {
        let new_scroll = (self.scroll as u32)
            .saturating_add_signed(lines)
            .min(u16::MAX as u32) as u16;

        self.scroll = min(
            new_scroll,
            self.total_lines()
                .saturating_sub(self.inner_height(self.screen_size.height))
                + 1,
        );
    }

    pub fn visible_lines(&self) -> (i16, i16) {
        let start_y = self.scroll as i16;
        // We don't render the last line, so sub one extra:
        let end_y = start_y + self.inner_height(self.screen_size.height) as i16 - 2;
        (start_y, end_y)
    }

    pub fn open_link(&self, url: String) -> Result<(), Error> {
        std::process::Command::new("xdg-open").arg(&url).spawn()?;
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
                let SectionContent::Line(_, extras) = &section.content else {
                    return None;
                };
                let LineExtra::Link(url, _, _) = extras.get(pointer.index)? else {
                    return None;
                };
                Some(url.clone())
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
            let id = pointer.id;
            let pointer_y = self.document.get_y(id);
            let (from, to) = self.visible_lines();
            if pointer_y > to {
                self.scroll_by((pointer_y - to) as i32);
            } else if pointer_y < from {
                self.scroll_by((pointer_y - from) as i32);
            }
        }
    }

    pub fn sections(&self) -> impl Iterator<Item = &Section> {
        self.document.iter()
    }

    pub fn theme(&self) -> &Theme {
        &self.config.theme
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

    fn is_first_load(&self) -> bool {
        self.reload_id == 0
    }
}

impl Display for DocumentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "D{}.{}", self.id, self.reload_id,)
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
        document::{Document, LineExtra, Section, SectionContent},
        model::{InputQueue, Model},
    };

    fn test_model() -> Model {
        let (cmd_tx, _) = mpsc::channel::<Cmd>();
        let (_, event_rx) = mpsc::channel::<Event>();
        Model {
            original_file_path: None,
            screen_size: (80, 20).into(),
            config: UserConfig::default().into(),
            scroll: 0,
            input_queue: InputQueue::None,
            cursor: Cursor::default(),
            document: Document::default(),
            cmd_tx,
            event_rx,
            log_snapshot: None,
            document_id: DocumentId::default(),
            pending_image_count: 0,
            can_render_headers: true,
        }
    }

    #[track_caller]
    fn assert_cursor_link(model: &Model, expected_url: &SourceContent) {
        let LineExtra::Link(url, ..) = model
            .document
            .find_extra_by_cursor(
                model
                    .cursor
                    .pointer()
                    .expect("model.cursor.pointer() should be Some(CursorPointer{ .. })"),
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
            content: SectionContent::Line(
                Line::from("http://a.com http://b.com"),
                vec![
                    LineExtra::Link(link_a.clone(), 0, 11),
                    LineExtra::Link(link_b.clone(), 12, 21),
                ],
            ),
        });
        model.document.push(Section {
            id: 2,
            height: 1,
            content: SectionContent::Line(
                Line::from("http://c.com"),
                vec![LineExtra::Link(link_c.clone(), 0, 11)],
            ),
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
                content: SectionContent::Line(
                    Line::from(url.clone()),
                    vec![LineExtra::Link(link, 0, 11)],
                ),
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
            content: SectionContent::Line(
                Line::from("http://a.com"),
                vec![LineExtra::Link(link.clone(), 0, 11)],
            ),
        });
        for i in 2..5 {
            model.document.push(Section {
                id: i,
                height: 1,
                content: SectionContent::Line(Line::from("text"), vec![]),
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
            content: SectionContent::Line(
                Line::from("http://a.com http://b.com"),
                vec![
                    LineExtra::Link(link_a.clone(), 0, 11),
                    LineExtra::Link(link_b.clone(), 12, 21),
                ],
            ),
        });
        model.document.push(Section {
            id: 2,
            height: 1,
            content: SectionContent::Line(
                Line::from("http://c.com"),
                vec![LineExtra::Link(link_c.clone(), 0, 11)],
            ),
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
            content: SectionContent::Line(
                Line::from("http://a.com http://b.com"),
                vec![
                    LineExtra::Link(link_a.clone(), 0, 11),
                    LineExtra::Link(link_b.clone(), 12, 21),
                ],
            ),
        });
        model.document.push(Section {
            id: 2,
            height: 1,
            content: SectionContent::Line(
                Line::from("http://c.com"),
                vec![LineExtra::Link(link_c.clone(), 0, 11)],
            ),
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
                content: SectionContent::Line(Line::from(format!("line {}", i + 1)), Vec::new()),
            });
        }

        // Just outside of view (terminal height is 20, we don't render on last line)
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
                content: SectionContent::Line(Line::from(format!("line {}", i + 1)), Vec::new()),
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
                content: SectionContent::Line(Line::from(format!("line {}", i + 1)), Vec::new()),
            });
        }
        let link = SourceContent::from("http://a.com");
        model.document.push(Section {
            id: 30,
            height: 1,
            content: SectionContent::Line(
                Line::from("http://a.com"),
                vec![LineExtra::Link(link.clone(), 0, 11)],
            ),
        });

        model.cursor_next(1);
        assert_cursor_link(&model, &link);

        assert_eq!(model.scroll, 12);
        assert_eq!(model.visible_lines(), (12, 30));

        let mut last_rendered = None;
        let mut y: i16 = 0 - (model.scroll as i16);
        for source in model.document.iter() {
            y += source.height as i16;
            if y >= model.inner_height(model.screen_size.height) as i16 - 1 {
                last_rendered = Some(source);
                break;
            }
        }
        let last_rendered = last_rendered.unwrap();
        let SectionContent::Line(_, extra) = &last_rendered.content else {
            panic!("expected Line");
        };
        let LineExtra::Link(url, _, _) = &extra[0] else {
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
                content: SectionContent::Line(
                    Line::from(url),
                    vec![LineExtra::Link(link.clone(), 0, 11)],
                ),
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
