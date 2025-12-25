use std::{
    cmp::min,
    fmt::Display,
    fs,
    num::{NonZero, NonZeroU16},
    path::PathBuf,
    sync::mpsc::{Receiver, Sender},
};

use ratatui::{
    layout::{Rect, Size},
    style::Stylize as _,
    text::{Line, Span},
    widgets::Padding,
};
use regex::RegexBuilder;

use crate::{
    Cmd,
    config::{Config, PaddingConfig},
    error::Error,
    widget_sources::{FindMode, FindTarget},
};
use crate::{Event, widget_sources::WidgetSources};
use crate::{MarkdownImage, setup::BgColor};
use crate::{
    cursor::Cursor,
    widget_sources::{WidgetSource, WidgetSourceData},
};

pub struct Model {
    pub bg: Option<BgColor>,
    sources: WidgetSources,
    pub scroll: u16,
    pub movement_count: Option<NonZeroU16>,
    pub cursor: Cursor,
    pub log_snapshot: Option<flexi_logger::Snapshot>,
    original_file_path: Option<PathBuf>,
    screen_size: Size,
    config: Config,
    cmd_tx: Sender<Cmd>,
    event_rx: Receiver<Event>,
    document_id: DocumentId,
    #[cfg(test)]
    pub pending_image_count: usize,
}

impl Model {
    pub fn new(
        bg: Option<BgColor>,
        original_file_path: Option<PathBuf>,
        cmd_tx: Sender<Cmd>,
        event_rx: Receiver<Event>,
        screen_size: Size,
        config: Config,
    ) -> Model {
        Model {
            original_file_path,
            bg,
            screen_size,
            config,
            scroll: 0,
            movement_count: None,
            cursor: Cursor::default(),
            sources: WidgetSources::default(),
            cmd_tx,
            event_rx,
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
        text: String,
    ) -> Result<(), Error> {
        let inner_width = self.inner_width(screen_size.width);
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
        self.sources.iter().map(|s| s.height).sum()
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
                Event::ParseDone(document_id, last_source_id) => {
                    if !self.document_id.is_same_document(&document_id) {
                        log::debug!("stale event, ignoring");
                        continue;
                    }
                    self.sources.trim_last_source(last_source_id);
                    self.reload_search();
                    had_done = true;
                }
                Event::Parsed(document_id, source) => {
                    if !self.document_id.is_same_document(&document_id) {
                        log::debug!("stale event, ignoring");
                        continue;
                    }

                    debug_assert!(
                        !matches!(source.data, WidgetSourceData::Image(_, _),),
                        "unexped Event::Parsed with Image: {:?}",
                        source.data
                    );

                    if self.document_id.is_first_load() {
                        self.sources.push(source);
                    } else {
                        self.sources.update(vec![source]);
                    }
                }
                Event::Update(document_id, updates) => {
                    if !self.document_id.is_same_document(&document_id) {
                        log::debug!("stale event, ignoring");
                        continue;
                    }
                    #[cfg(test)]
                    for source in &updates {
                        if let WidgetSourceData::Image(_, _) = source.data {
                            log::debug!("Update #{}: {:?}", source.id, source.data);
                            self.pending_image_count -= 1;
                        }
                    }
                    self.sources.update(updates);
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

                    if let Some(mut existing_image) = self.sources.replace(id, &link_destination) {
                        log::debug!("replacing from existing image ({link_destination})");
                        existing_image.id = id;
                        self.sources.update(vec![existing_image]);
                    } else {
                        if self.document_id.is_first_load() {
                            log::debug!(
                                "existing image not found, push placeholder and process image ({link_destination})"
                            );
                            self.sources.push(WidgetSource {
                                id,
                                height: 1,
                                data: WidgetSourceData::Line(
                                    Line::from(format!("![Loading...]({link_destination})")),
                                    Vec::new(),
                                ),
                            });
                        } else {
                            log::debug!(
                                "existing image not found, update placeholder and process image ({link_destination})"
                            );
                            self.sources.update(vec![WidgetSource {
                                id,
                                height: 1,
                                data: WidgetSourceData::Line(
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
                        self.sources.push(WidgetSource {
                            id,
                            height: 2,
                            data: WidgetSourceData::Line(line, Vec::new()),
                        });
                    } else {
                        self.sources.update(vec![WidgetSource {
                            id,
                            height: 2,
                            data: WidgetSourceData::Line(line, Vec::new()),
                        }]);
                    }
                    #[cfg(test)]
                    {
                        log::debug!("ParseHeader");
                        self.pending_image_count += 1;
                    }
                    self.cmd_tx
                        .send(Cmd::Header(document_id, id, inner_width, tier, text))?;
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
            Cursor::Search(state, _) => {
                // TODO: See above
                self.add_searches(Some(&state.needle));
                self.cursor = Cursor::Search(state, None);
            }
        }
    }

    pub fn scroll_by(&mut self, lines: i32) {
        let count = self.movement_count.take().map_or(1, NonZero::get) as i32;
        let lines = lines.saturating_mul(count);
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

    pub fn cursor_next(&mut self) {
        self.cursor_find(FindMode::Next)
    }

    pub fn cursor_prev(&mut self) {
        self.cursor_find(FindMode::Prev)
    }

    fn cursor_find(&mut self, mode: FindMode) {
        match &mut self.cursor {
            Cursor::None => {
                if let Some(pointer) = WidgetSources::find_first_cursor(
                    self.sources.iter(),
                    FindTarget::Link,
                    self.scroll,
                ) {
                    self.cursor = Cursor::Links(pointer);
                } else {
                    self.movement_count = None;
                }
            }
            Cursor::Links(current) => {
                if let Some(pointer) = WidgetSources::find_nth_next_cursor(
                    self.sources.iter(),
                    current,
                    mode,
                    FindTarget::Link,
                    self.movement_count.take().map_or(1, NonZero::get),
                ) {
                    self.cursor = Cursor::Links(pointer);
                }
            }
            Cursor::Search(_, pointer) => match pointer {
                None => {
                    *pointer = WidgetSources::find_first_cursor(
                        self.sources.iter(),
                        FindTarget::Search,
                        self.scroll,
                    );
                    if pointer.is_none() {
                        self.movement_count = None;
                    }
                }
                Some(current) => {
                    *pointer = WidgetSources::find_nth_next_cursor(
                        self.sources.iter(),
                        current,
                        mode,
                        FindTarget::Search,
                        self.movement_count.take().map_or(1, NonZero::get),
                    );
                }
            },
        }
        if let Some(count) = self.movement_count.take().map(NonZero::get)
            && count > 1
        {
            self.movement_count = NonZero::new(count - 1);
            return self.cursor_find(mode);
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
        for source in self.sources.iter_mut() {
            source.add_search(re.as_ref());
        }
    }

    fn jump_to_pointer(&mut self) {
        if let Some(pointer) = self.cursor.pointer() {
            let id = pointer.id;
            let pointer_y = self.sources.get_y(id);
            let (from, to) = self.visible_lines();
            if pointer_y > to {
                self.scroll_by((pointer_y - to) as i32);
            } else if pointer_y < from {
                self.scroll_by((pointer_y - from) as i32);
            }
        }
    }

    pub fn sources(&self) -> impl Iterator<Item = &WidgetSource> {
        self.sources.iter()
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

    use std::{num::NonZero, sync::mpsc};

    use ratatui::text::Line;

    use crate::{
        Cmd, DocumentId, Event,
        config::UserConfig,
        cursor::{Cursor, CursorPointer, SearchState},
        model::Model,
        widget_sources::{LineExtra, WidgetSource, WidgetSourceData, WidgetSources},
    };

    fn test_model() -> Model {
        let (cmd_tx, _) = mpsc::channel::<Cmd>();
        let (_, event_rx) = mpsc::channel::<Event>();
        Model {
            original_file_path: None,
            bg: None,
            screen_size: (80, 20).into(),
            config: UserConfig::default().into(),
            scroll: 0,
            movement_count: None,
            cursor: Cursor::default(),
            sources: WidgetSources::default(),
            cmd_tx,
            event_rx,
            log_snapshot: None,
            document_id: DocumentId::default(),
            pending_image_count: 0,
        }
    }

    #[track_caller]
    fn assert_cursor_link(model: &Model, expected_url: &str) {
        let LineExtra::Link(url, ..) = model
            .sources
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
                    .and_then(|p| model.sources.find_extra_by_cursor(p))
            );
        };
        assert_eq!(url, expected_url);
    }

    #[test]
    fn finds_link_per_line() {
        let mut model = test_model();
        model.sources.push(WidgetSource {
            id: 1,
            height: 1,
            data: WidgetSourceData::Line(
                Line::from("http://a.com http://b.com"),
                vec![
                    LineExtra::Link("http://a.com".into(), 0, 11),
                    LineExtra::Link("http://b.com".into(), 12, 21),
                ],
            ),
        });
        model.sources.push(WidgetSource {
            id: 2,
            height: 1,
            data: WidgetSourceData::Line(
                Line::from("http://c.com"),
                vec![LineExtra::Link("http://c.com".into(), 0, 11)],
            ),
        });

        model.cursor_next();
        assert_cursor_link(&model, "http://a.com");

        model.cursor_next();
        assert_cursor_link(&model, "http://b.com");

        model.cursor_next();
        assert_cursor_link(&model, "http://c.com");
    }

    #[test]
    fn finds_link_with_scroll() {
        let mut model = test_model();
        for i in 1..5 {
            let link = format!("http://{}.com", i);
            model.sources.push(WidgetSource {
                id: i,
                height: 1,
                data: WidgetSourceData::Line(
                    Line::from(link.clone()),
                    vec![LineExtra::Link(link, 0, 11)],
                ),
            });
        }

        model.scroll = 2;
        model.cursor_next();
        assert_cursor_link(&model, "http://3.com");
    }

    #[test]
    fn finds_link_with_scroll_wrapping() {
        let mut model = test_model();
        model.sources.push(WidgetSource {
            id: 1,
            height: 1,
            data: WidgetSourceData::Line(
                Line::from("http://a.com"),
                vec![LineExtra::Link("http://a.com".into(), 0, 11)],
            ),
        });
        for i in 2..5 {
            model.sources.push(WidgetSource {
                id: i,
                height: 1,
                data: WidgetSourceData::Line(Line::from("text"), vec![]),
            });
        }

        model.scroll = 2;
        model.cursor_next();
        assert_cursor_link(&model, "http://a.com");
    }

    #[test]
    fn finds_multiple_links_per_line_next() {
        let mut model = test_model();
        model.sources.push(WidgetSource {
            id: 1,
            height: 1,
            data: WidgetSourceData::Line(
                Line::from("http://a.com http://b.com"),
                vec![
                    LineExtra::Link("http://a.com".into(), 0, 11),
                    LineExtra::Link("http://b.com".into(), 12, 21),
                ],
            ),
        });
        model.sources.push(WidgetSource {
            id: 2,
            height: 1,
            data: WidgetSourceData::Line(
                Line::from("http://c.com"),
                vec![LineExtra::Link("http://c.com".into(), 0, 11)],
            ),
        });

        model.cursor_next();
        assert_cursor_link(&model, "http://a.com");

        model.cursor_next();
        assert_cursor_link(&model, "http://b.com");

        model.cursor_next();
        assert_cursor_link(&model, "http://c.com");
    }

    #[test]
    fn finds_multiple_links_per_line_prev() {
        let mut model = test_model();
        model.sources.push(WidgetSource {
            id: 1,
            height: 1,
            data: WidgetSourceData::Line(
                Line::from("http://a.com http://b.com"),
                vec![
                    LineExtra::Link("http://a.com".into(), 0, 11),
                    LineExtra::Link("http://b.com".into(), 12, 21),
                ],
            ),
        });
        model.sources.push(WidgetSource {
            id: 2,
            height: 1,
            data: WidgetSourceData::Line(
                Line::from("http://c.com"),
                vec![LineExtra::Link("http://c.com".into(), 0, 11)],
            ),
        });

        model.cursor_prev();
        assert_cursor_link(&model, "http://a.com");

        model.cursor_prev();
        assert_cursor_link(&model, "http://c.com");

        model.cursor_prev();
        assert_cursor_link(&model, "http://b.com");
    }

    #[test]
    fn jump_to_pointer() {
        let mut model = test_model();
        for i in 0..31 {
            model.sources.push(WidgetSource {
                id: i,
                height: 1,
                data: WidgetSourceData::Line(Line::from(format!("line {}", i + 1)), Vec::new()),
            });
        }

        // Just outside of view (terminal height is 20, we don't render on last line)
        model.cursor = Cursor::Search(
            SearchState::default(),
            Some(CursorPointer { id: 19, index: 0 }),
        );
        model.jump_to_pointer();
        assert_eq!(model.scroll, 1);

        model.scroll = 0;
        // Towards the end
        model.cursor = Cursor::Search(
            SearchState::default(),
            Some(CursorPointer { id: 30, index: 0 }),
        );
        model.jump_to_pointer();
        assert_eq!(model.scroll, 12);
    }

    #[test]
    fn jump_back_to_pointer() {
        let mut model = test_model();
        for i in 0..31 {
            model.sources.push(WidgetSource {
                id: i,
                height: 1,
                data: WidgetSourceData::Line(Line::from(format!("line {}", i + 1)), Vec::new()),
            });
        }

        model.scroll = 12;
        model.cursor = Cursor::Search(
            SearchState::default(),
            Some(CursorPointer { id: 0, index: 0 }),
        );
        model.jump_to_pointer();
        assert_eq!(model.scroll, 0);
    }

    #[test]
    fn scrolls_into_view() {
        let mut model = test_model();
        for i in 0..30 {
            model.sources.push(WidgetSource {
                id: i,
                height: 1,
                data: WidgetSourceData::Line(Line::from(format!("line {}", i + 1)), Vec::new()),
            });
        }
        model.sources.push(WidgetSource {
            id: 30,
            height: 1,
            data: WidgetSourceData::Line(
                Line::from("http://a.com"),
                vec![LineExtra::Link("http://a.com".into(), 0, 11)],
            ),
        });

        model.cursor_next();
        assert_cursor_link(&model, "http://a.com");

        assert_eq!(model.scroll, 12);
        assert_eq!(model.visible_lines(), (12, 30));

        let mut last_rendered = None;
        let mut y: i16 = 0 - (model.scroll as i16);
        for source in model.sources.iter() {
            y += source.height as i16;
            if y >= model.inner_height(model.screen_size.height) as i16 - 1 {
                last_rendered = Some(source);
                break;
            }
        }
        let last_rendered = last_rendered.unwrap();
        let WidgetSourceData::Line(_, extra) = &last_rendered.data else {
            panic!("expected Line");
        };
        let LineExtra::Link(url, _, _) = &extra[0] else {
            panic!("expected Link");
        };
        assert_eq!("http://a.com", url);
    }

    #[test]
    fn finds_links_with_count() {
        let mut model = test_model();
        for i in 1..10 {
            let link = format!("http://{}.com", i);
            model.sources.push(WidgetSource {
                id: i,
                height: 1,
                data: WidgetSourceData::Line(
                    Line::from(link.clone()),
                    vec![LineExtra::Link(link, 0, 11)],
                ),
            });
        }

        model.movement_count = NonZero::new(3);
        model.cursor_next();
        assert_cursor_link(&model, "http://3.com");

        model.movement_count = NonZero::new(2);
        model.cursor_prev();
        assert_cursor_link(&model, "http://1.com");

        model.movement_count = NonZero::new(1);
        model.cursor_prev();
        assert_cursor_link(&model, "http://9.com");

        model.movement_count = NonZero::new(4);
        model.cursor_next();
        assert_cursor_link(&model, "http://4.com");
    }
}
