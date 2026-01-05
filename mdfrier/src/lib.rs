//! MdFrier - A markdown parser that produces styled terminal lines
//!
//! This crate parses markdown text and produces an iterator of `MdLine` items
//! that can be rendered to a terminal. The optional `ratatui` feature provides
//! conversion to styled ratatui `Line` widgets.
//!
//! # Example
//!
//! ```
//! use mdfrier::MdFrier;
//!
//! let mut frier = MdFrier::new().unwrap();
//! let lines: Vec<_> = frier.parse(80, "Hello *world*!".to_owned()).collect();
//! ```

mod lines;
mod markdown;
mod wrap;

#[cfg(feature = "ratatui")]
pub mod ratatui;

use std::collections::VecDeque;

use tree_sitter::Parser;

use markdown::{MdContainer, MdContent, MdDocument, MdSection};

pub use lines::{
    BorderPosition, BulletStyle, Container, LineKind, LineMeta, ListMarker, MdLine, TableColumnInfo,
};
pub use markdown::{MdModifier, MdNode, TableAlignment};

/// Error type for mdfrier operations.
#[derive(Debug)]
pub enum Error {
    /// Failed to parse markdown.
    MarkdownParse,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::MarkdownParse => write!(f, "Failed to parse markdown"),
        }
    }
}

impl std::error::Error for Error {}

/// The main markdown parser struct.
///
/// Wraps tree-sitter parsers and provides a simple interface for parsing
/// markdown text into lines.
pub struct MdFrier {
    parser: Parser,
    inline_parser: Parser,
}

impl MdFrier {
    /// Create a new MdFrier instance.
    pub fn new() -> Result<Self, Error> {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_md::LANGUAGE.into())
            .ok()
            .ok_or(Error::MarkdownParse)?;

        let mut inline_parser = Parser::new();
        inline_parser
            .set_language(&tree_sitter_md::INLINE_LANGUAGE.into())
            .ok()
            .ok_or(Error::MarkdownParse)?;

        Ok(Self {
            parser,
            inline_parser,
        })
    }

    /// Parse markdown text and return an iterator of `MdLine` items.
    ///
    /// # Arguments
    ///
    /// * `width` - The terminal width for line wrapping
    /// * `text` - The markdown text to parse
    pub fn parse(&mut self, width: u16, text: String) -> impl Iterator<Item = MdLine> {
        let doc = match MdDocument::new(text, &mut self.parser, &mut self.inline_parser) {
            Ok(doc) => doc,
            Err(_) => return MdLineIterator::empty(),
        };
        MdLineIterator::new(doc, width)
    }
}

/// Iterator over MdLine items produced from parsing markdown.
struct MdLineIterator {
    sections: Vec<MdSection>,
    section_idx: usize,
    width: u16,
    pending_lines: VecDeque<MdLine>,
    needs_blank: bool,
    prev_nesting: Vec<MdContainer>,
    prev_was_blank: bool,
    prev_in_list: bool,
}

impl MdLineIterator {
    fn new(mut doc: MdDocument, width: u16) -> Self {
        let sections: Vec<_> = doc.sections().collect();
        Self {
            sections,
            section_idx: 0,
            width,
            pending_lines: VecDeque::new(),
            needs_blank: false,
            prev_nesting: Vec::new(),
            prev_was_blank: false,
            prev_in_list: false,
        }
    }

    fn empty() -> Self {
        Self {
            sections: Vec::new(),
            section_idx: 0,
            width: 80,
            pending_lines: VecDeque::new(),
            needs_blank: false,
            prev_nesting: Vec::new(),
            prev_was_blank: false,
            prev_in_list: false,
        }
    }

    fn process_next_section(&mut self) -> bool {
        if self.section_idx >= self.sections.len() {
            return false;
        }

        let section = &self.sections[self.section_idx];
        self.section_idx += 1;

        let in_list = section
            .nesting
            .iter()
            .any(|c| matches!(c, MdContainer::ListItem(_)));

        let is_blank_line = section.content.is_blank();

        // Nesting change detection - compare container types, not exact values
        let container_type_matches = |a: &MdContainer, b: &MdContainer| -> bool {
            matches!(
                (a, b),
                (MdContainer::List(_), MdContainer::List(_))
                    | (MdContainer::ListItem(_), MdContainer::ListItem(_))
                    | (MdContainer::Blockquote(_), MdContainer::Blockquote(_))
            )
        };
        let is_type_prefix = |shorter: &[MdContainer], longer: &[MdContainer]| -> bool {
            !shorter.is_empty()
                && shorter.len() < longer.len()
                && shorter
                    .iter()
                    .zip(longer.iter())
                    .all(|(a, b)| container_type_matches(a, b))
        };
        let nesting_change = is_type_prefix(&self.prev_nesting, &section.nesting)
            || is_type_prefix(&section.nesting, &self.prev_nesting);

        // Count list nesting depth (number of List containers)
        let list_depth = |nesting: &[MdContainer]| -> usize {
            nesting
                .iter()
                .filter(|c| matches!(c, MdContainer::List(_)))
                .count()
        };
        let curr_list_depth = list_depth(&section.nesting);
        let prev_list_depth = list_depth(&self.prev_nesting);

        // Check if both sections are at the same top-level list (depth 1) with same List container
        let same_top_level_list =
            if in_list && self.prev_in_list && curr_list_depth == 1 && prev_list_depth == 1 {
                // Compare first List container only for top-level items
                let curr_list = section
                    .nesting
                    .iter()
                    .find(|c| matches!(c, MdContainer::List(_)));
                let prev_list = self
                    .prev_nesting
                    .iter()
                    .find(|c| matches!(c, MdContainer::List(_)));
                curr_list == prev_list
            } else {
                false
            };

        // For nested lists (depth > 1), treat all items at same depth as same context
        // to avoid blanks between items with different markers
        let same_nested_context =
            in_list && self.prev_in_list && curr_list_depth > 1 && prev_list_depth > 1;

        let same_list_context = same_top_level_list || same_nested_context;

        // Check if we're exiting to a new top-level list (not part of previous ancestry)
        let exiting_to_new_top_level =
            nesting_change && curr_list_depth == 1 && prev_list_depth > 1 && {
                // Check if the current top-level List was in the previous nesting
                let curr_list = section
                    .nesting
                    .iter()
                    .find(|c| matches!(c, MdContainer::List(_)));
                let was_in_prev = curr_list.is_none_or(|cl| self.prev_nesting.contains(cl));
                !was_in_prev
            };

        // Allow blank lines before continuation paragraphs or between different top-level lists,
        // but not during nesting changes (unless exiting to a new top-level list)
        let should_emit_blank = self.needs_blank
            && (!same_list_context || section.is_list_continuation)
            && !is_blank_line
            && !self.prev_was_blank
            && (!nesting_change || exiting_to_new_top_level);

        if should_emit_blank {
            self.pending_lines.push_back(MdLine::blank());
        }

        // Only headers don't need space after
        self.needs_blank = !matches!(section.content, MdContent::Header { .. });
        // Clone nesting for comparison in next iteration
        self.prev_nesting.clone_from(&section.nesting);
        self.prev_was_blank = is_blank_line;
        self.prev_in_list = in_list;

        let lines = lines::section_to_lines(self.width, section);
        self.pending_lines.extend(lines);

        true
    }
}

impl Iterator for MdLineIterator {
    type Item = MdLine;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(line) = self.pending_lines.pop_front() {
                return Some(line);
            }

            if !self.process_next_section() {
                return None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    /// Convert MdLines to a string representation for testing.
    fn lines_to_string(lines: &[MdLine]) -> String {
        lines
            .iter()
            .map(|line| {
                if matches!(line.meta.kind, LineKind::Blank) {
                    String::new()
                } else {
                    let prefix = nesting_to_prefix(&line.meta.nesting);
                    let content: String = line.spans.iter().map(|s| s.content.as_str()).collect();
                    format!("{prefix}{content}")
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Build prefix string from nesting for test output.
    fn nesting_to_prefix(nesting: &[lines::Container]) -> String {
        use lines::{Container, ListMarker};
        let mut prefix = String::new();
        let last_list_idx = nesting
            .iter()
            .rposition(|c| matches!(c, Container::ListItem { .. }));
        for (i, c) in nesting.iter().enumerate() {
            match c {
                Container::Blockquote => prefix.push_str("> "),
                Container::ListItem {
                    marker,
                    continuation,
                } => {
                    // Only render marker for innermost non-continuation list item
                    if Some(i) == last_list_idx && !continuation {
                        match marker {
                            ListMarker::Unordered(b) => {
                                prefix.push(b.char());
                                prefix.push(' ');
                            }
                            ListMarker::Ordered(n) => prefix.push_str(&format!("{}. ", n)),
                            ListMarker::TaskUnchecked(b) => {
                                prefix.push(b.char());
                                prefix.push_str(" [ ] ");
                            }
                            ListMarker::TaskChecked(b) => {
                                prefix.push(b.char());
                                prefix.push_str(" [x] ");
                            }
                        }
                    } else {
                        // Outer list items or continuations render as indentation
                        prefix.push_str(&" ".repeat(marker.width()));
                    }
                }
            }
        }
        prefix
    }

    #[test]
    fn parse_simple_text() {
        let mut frier = MdFrier::new().unwrap();
        let lines: Vec<_> = frier.parse(80, "Hello world!".to_owned()).collect();
        assert_eq!(lines.len(), 1);

        let line = &lines[0];
        assert_eq!(line.spans.len(), 1);
        assert_eq!(line.spans[0].content, "Hello world!");
    }

    #[test]
    fn parse_styled_text() {
        let mut frier = MdFrier::new().unwrap();
        let lines: Vec<_> = frier.parse(80, "Hello *world*!".to_owned()).collect();
        assert_eq!(lines.len(), 1);

        let line = &lines[0];
        assert_eq!(line.spans.len(), 3);
        assert_eq!(line.spans[0].content, "Hello ");
        assert_eq!(line.spans[1].content, "world");
        assert!(line.spans[1].extra.contains(MdModifier::Emphasis));
        assert_eq!(line.spans[2].content, "!");
    }

    #[test]
    fn parse_header() {
        let mut frier = MdFrier::new().unwrap();
        let lines: Vec<_> = frier.parse(80, "# Hello".to_owned()).collect();
        assert_eq!(lines.len(), 1);

        let line = &lines[0];
        assert!(matches!(line.meta.kind, LineKind::Header(1)));
    }

    #[test]
    fn parse_code_block() {
        let mut frier = MdFrier::new().unwrap();
        let lines: Vec<_> = frier
            .parse(80, "```rust\nlet x = 1;\n```".to_owned())
            .collect();
        assert_eq!(lines.len(), 1);

        let line = &lines[0];
        assert!(matches!(line.meta.kind, LineKind::CodeBlock { .. }));
        assert_eq!(line.spans[0].content, "let x = 1;");
    }

    #[test]
    fn parse_blockquote() {
        let mut frier = MdFrier::new().unwrap();
        let lines: Vec<_> = frier.parse(80, "> Hello world".to_owned()).collect();
        assert_eq!(lines.len(), 1);

        let line = &lines[0];
        assert_eq!(line.meta.blockquote_depth(), 1);
    }

    #[test]
    fn parse_list() {
        let mut frier = MdFrier::new().unwrap();
        let lines: Vec<_> = frier.parse(80, "- Item 1\n- Item 2".to_owned()).collect();
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn paragraph_breaks() {
        let mut frier = MdFrier::new().unwrap();
        let lines: Vec<_> = frier.parse(10, "longline1\nlongline2".to_owned()).collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].spans[0].content, "longline1");
        assert_eq!(lines[1].spans[0].content, "longline2");
    }

    #[test]
    fn soft_break_with_styling() {
        let mut frier = MdFrier::new().unwrap();
        let lines: Vec<_> = frier.parse(80, "This \n*is* a test.".to_owned()).collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].spans[0].content, "This");
        assert!(lines[1].spans[0].extra.contains(MdModifier::Emphasis));
        assert_eq!(lines[1].spans[0].content, "is");
    }

    #[test]
    fn code_block_spacing() {
        let input = "Paragraph before.
```rust
let x = 1;
```
Paragraph after.";

        let mut frier = MdFrier::new().unwrap();
        let lines: Vec<_> = frier.parse(80, input.to_owned()).collect();
        let output = lines_to_string(&lines);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn code_block_before_list_spacing() {
        let input = "```rust
let x = 1;
```
- list item";

        let mut frier = MdFrier::new().unwrap();
        let lines: Vec<_> = frier.parse(80, input.to_owned()).collect();
        let output = lines_to_string(&lines);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn separate_blockquotes_have_blank_lines() {
        let input = r#"> Blockquotes are very handy in email to emulate reply text.
> This line is part of the same quote.

Quote break.

> This is a very long line that will still be quoted properly when it wraps. Oh boy let's keep writing to make sure this is long enough to actually wrap for everyone. Oh, you can *put* **Markdown** into a blockquote.

> Blockquotes can also be nested...
>
> > ...by using additional greater-than signs right next to each other...
> >
> > > ...or with spaces between arrows."#;

        let mut frier = MdFrier::new().unwrap();
        let lines: Vec<_> = frier.parse(80, input.to_owned()).collect();
        let output = lines_to_string(&lines);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn list_preserve_formatting() {
        let input = r#"1. First ordered list item
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
  - Task lists
    - [x] Finish my changes
    - [ ] Push my commits to GitHub
    - [ ] Open a pull request
    - [x] @mentions, #refs, [links](), **formatting**, and <del>tags</del> supported
    - [x] list syntax required (any unordered or ordered list supported)
    - [ ] this is a complete item
    - [ ] this is an incomplete item
- Very easy!
"#;

        let mut frier = MdFrier::new().unwrap();
        let lines: Vec<_> = frier.parse(80, input.to_owned()).collect();
        let output = lines_to_string(&lines);
        insta::assert_snapshot!(output);
    }
}
