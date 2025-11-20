use std::{
    cmp::min,
    path::PathBuf,
    sync::mpsc::{Receiver, SendError, Sender},
};

use ratatui::{
    layout::{Rect, Size},
    style::Stylize,
    text::{Line, Span},
    widgets::Padding,
};

use crate::{
    Cmd,
    config::{Config, PaddingConfig},
    error::Error,
};
use crate::{Event, widget_sources::WidgetSources};
use crate::{WidthEvent, setup::BgColor};
use crate::{
    read_file_to_str,
    widget_sources::{WidgetSource, WidgetSourceData},
};

pub struct Model<'a, 'b> {
    pub bg: Option<BgColor>,
    pub sources: WidgetSources<'a>,
    pub scroll: u16,
    pub log_snapshot: Option<flexi_logger::Snapshot>,
    original_file_path: Option<PathBuf>,
    terminal_height: u16,
    config: Config,
    cmd_tx: Sender<Cmd>,
    event_rx: Receiver<WidthEvent<'b>>,
}

impl<'a, 'b: 'a> Model<'a, 'b> {
    pub fn new(
        bg: Option<BgColor>,
        original_file_path: Option<PathBuf>,
        cmd_tx: Sender<Cmd>,
        event_rx: Receiver<WidthEvent<'b>>,
        terminal_height: u16,
        config: Config,
    ) -> Result<Model<'a, 'b>, Error> {
        let model = Model {
            original_file_path,
            bg,
            terminal_height,
            config,
            scroll: 0,
            sources: WidgetSources::default(),
            cmd_tx,
            event_rx,
            log_snapshot: None,
        };

        // model_reload(&mut model, screen_width)?;

        Ok(model)
    }

    pub fn reload(&mut self, screen_size: Size) -> Result<(), Error> {
        log::debug!("reload");
        if let Some(original_file_path) = &self.original_file_path {
            let text = read_file_to_str(
                original_file_path
                    .to_str()
                    .ok_or(Error::Path(original_file_path.to_path_buf()))?,
            )?;

            self.sources = WidgetSources::default();
            self.scroll = 0;
            self.terminal_height = screen_size.height;
            self.parse(screen_size, text)?;
        }
        Ok(())
    }

    pub fn parse(&self, screen_size: Size, text: String) -> Result<(), SendError<Cmd>> {
        let inner_width = self.inner_width(screen_size.width);
        self.cmd_tx.send(Cmd::Parse(inner_width, text))
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

    pub fn process_events(&mut self, screen_width: u16) -> Result<bool, Error> {
        let inner_width = self.inner_width(screen_width);
        let mut had_events = false;
        while let Ok((id, ev)) = self.event_rx.try_recv() {
            if id == inner_width {
                had_events = true;
                log::debug!("Event: {ev:?}");
                match ev {
                    Event::Parsed(source) => self.sources.push(source),
                    Event::Update(updates) => self.sources.update(updates),
                    Event::ParseImage(id, url, text, title) => {
                        self.sources.push(WidgetSource {
                            id,
                            height: 1,
                            data: WidgetSourceData::Line(Line::from(format!(
                                "![Loading...]({url})"
                            ))),
                        });
                        self.cmd_tx
                            .send(Cmd::UrlImage(id, inner_width, url, text, title))?;
                    }
                    Event::ParseHeader(id, tier, text) => {
                        let line = Line::from(vec![
                            Span::from("#".repeat(tier as usize) + " ").light_blue(),
                            Span::from(text.clone()),
                        ]);
                        self.sources.push(WidgetSource {
                            id,
                            height: 2,
                            data: WidgetSourceData::Line(line),
                        });
                        self.cmd_tx.send(Cmd::Header(id, inner_width, tier, text))?;
                    }
                    Event::MarkHadEvents => {}
                }
            } else if id == 0 && matches!(ev, Event::MarkHadEvents) {
                had_events = true;
            }
        }
        Ok(had_events)
    }

    pub fn scroll_by(&mut self, lines: i16) {
        self.scroll = min(
            self.scroll.saturating_add_signed(lines),
            self.total_lines()
                .saturating_sub(self.inner_height(self.terminal_height))
                + 1,
        );
        // For now we just clear the link cursor, maybe we could keep it if still visible.
        self.sources.clear_cursor();
    }

    pub fn visible_lines(&self) -> (i16, i16) {
        (0 - (self.scroll as i16), self.terminal_height as i16) // TODO padding?
    }

    pub fn open_link(&self, url: String) -> Result<(), SendError<Cmd>> {
        self.cmd_tx.send(Cmd::XdgOpen(url))
    }
}
