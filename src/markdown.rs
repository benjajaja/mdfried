use std::sync::mpsc::Sender;

use ratatui::text::Line;
use ratskin::RatSkin;
use regex::Regex;

use crate::{
    widget_sources::{SourceID, WidgetSourceData},
    Error, Event, WidgetSource, WidthEvent,
};

// Crude "pre-parsing" of markdown by lines.
// Headers are always on a line of their own.
// Images are only processed if it appears on a line by itself, to avoid having to deal with text
// wrapping around some area.
#[derive(Debug, PartialEq)]
enum Block {
    Header(u8, String),
    Image(String, String),
    Markdown(String),
}

fn split_headers_and_images(text: &str) -> Vec<Block> {
    // Regex to match lines starting with 1-6 `#` characters
    let header_re = Regex::new(r"^(#+)\s*(.*)").unwrap();
    // Regex to match standalone image lines: ![alt](url)
    let image_re = Regex::new(r"^!\[(.*?)\]\((.*?)\)$").unwrap();

    let mut blocks = Vec::new();
    let mut current_block = String::new();

    for line in text.lines() {
        if let Some(captures) = header_re.captures(line) {
            // If there's an ongoing block, push it as a plain text block
            if !current_block.is_empty() {
                blocks.push(Block::Markdown(current_block.clone()));
                current_block.clear();
            }
            // Push the header as (level, text)
            let level = captures[1].len().min(6) as u8;
            let text = captures[2].to_string();
            blocks.push(Block::Header(level, text));
        } else if let Some(captures) = image_re.captures(line) {
            // If there's an ongoing block, push it as a plain text block
            if !current_block.is_empty() {
                blocks.push(Block::Markdown(current_block.clone()));
                current_block.clear();
            }
            // Push the image as (alt_text, url)
            let alt_text = captures[1].to_string();
            let url = captures[2].to_string();
            blocks.push(Block::Image(alt_text, url));
        } else {
            // Accumulate lines that are neither headers nor images
            if !current_block.is_empty() {
                current_block.push('\n');
            }
            current_block.push_str(line);
        }
    }

    // Push the final block if there's remaining content
    if !current_block.is_empty() {
        blocks.push(Block::Markdown(current_block));
    }

    blocks
}

pub fn parse(
    text: &str,
    skin: &RatSkin,
    width: u16,
    tx: &Sender<WidthEvent>,
    has_text_size_protocol: bool,
) -> Result<(), Error> {
    let mut sender = SendTracker {
        width,
        tx,
        index: 0,
    };

    let text_blocks = split_headers_and_images(text);

    let mut needs_space = false;
    for block in text_blocks {
        if needs_space {
            // Send a newline after Markdowns and Images, but not after the last block.
            sender.send_line(WidgetSourceData::Line(Line::default()), 1)?;
        }

        match block {
            Block::Header(tier, text) => {
                needs_space = false;
                if has_text_size_protocol {
                    // Leverage ratskin/termimad's line-wrapping feature.
                    let madtext = RatSkin::parse_text(&text);
                    for line in skin.parse(madtext, width / 2) {
                        let text = line.to_string();
                        sender.send_line(WidgetSourceData::SizedLine(text, tier), 2)?;
                    }
                } else {
                    sender.send_event(Event::ParseHeader(sender.index, tier, text))?;
                }
            }
            Block::Image(alt, url) => {
                needs_space = true;
                sender.send_event(Event::ParseImage(sender.index, url, alt, "".to_string()))?;
            }
            Block::Markdown(text) => {
                needs_space = true;
                let madtext = RatSkin::parse_text(&text);
                for line in skin.parse(madtext, width) {
                    sender.send_line(WidgetSourceData::Line(line), 1)?;
                }
            }
        }
    }

    Ok(())
}

// Just so that we don't miss an `index += 1`.
struct SendTracker<'a, 'b> {
    width: u16,
    index: SourceID,
    tx: &'a Sender<WidthEvent<'b>>,
}

impl<'b> SendTracker<'_, 'b> {
    fn send_line(&mut self, source: WidgetSourceData<'b>, height: u16) -> Result<(), Error> {
        self.send_event(Event::Parsed(WidgetSource {
            id: self.index,
            height,
            source,
        }))
    }
    fn send_event(&mut self, ev: Event<'b>) -> Result<(), Error> {
        self.tx.send((self.width, ev))?;
        self.index += 1;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::*;

    #[test]
    fn test_split_headers_and_images() {
        let blocks = markdown::split_headers_and_images(
            r#"
# header

paragraph

paragraph

# header

paragraph
paragraph

# header

paragraph

# header
"#,
        );
        assert_eq!(
            blocks,
            vec![
                markdown::Block::Header(1, "header".to_string()),
                markdown::Block::Markdown("paragraph\n\nparagraph\n".to_string()),
                markdown::Block::Header(1, "header".to_string()),
                markdown::Block::Markdown("paragraph\nparagraph\n".to_string()),
                markdown::Block::Header(1, "header".to_string()),
                markdown::Block::Markdown("paragraph\n".to_string()),
                markdown::Block::Header(1, "header".to_string()),
            ]
        );
    }

    #[test]
    fn test_split_headers_and_images_without_space() {
        let blocks = markdown::split_headers_and_images(
            r#"
# header
paragraph
# header
# header
paragraph
# header
"#,
        );
        assert_eq!(6, blocks.len());
        assert_eq!(
            blocks,
            vec![
                markdown::Block::Header(1, "header".to_string()),
                markdown::Block::Markdown("paragraph".to_string()),
                markdown::Block::Header(1, "header".to_string()),
                markdown::Block::Header(1, "header".to_string()),
                markdown::Block::Markdown("paragraph".to_string()),
                markdown::Block::Header(1, "header".to_string()),
            ]
        );
    }
}
