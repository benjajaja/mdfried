#![cfg_attr(docsrs, feature(doc_auto_cfg))]

//! mdfrier - Deep fry markdown for [mdfried](https://crates.io/crates/mdfried).
//!
//! This crate parses markdown with tree-sitter-md into wrapped lines for a fixed width styled
//! output.
//!
//! This isn't as straightforward as wrapping the source and then highlighting syntax, because the
//! wrapping relies on markdown context. The process is:
//!
//! 1. Parse into raw lines with nodes
//! 2. Map the node's markdown symbols (optionally, because we want to strip e.g. `*` when
//!    highlighting with color later)
//! 3. Wrap the lines of nodes to a maximum width
//! 4. ???
//!
//! At step 4, the users of this library will typically convert the wrapped lines of nodes with
//! their style information to whatever the target is: ANSI escape sequences, or whatever some
//! their library expects.
//!
//! There is a `ratatui` feature that enables the [`ratatui`] module, which does exactly this, for
//! [ratatui](https://ratatui.rs).
//!
//! The [`Mapper`] trait controls decorator symbols (e.g., blockquote bar, link brackets).
//! The optional `ratatui` feature provides the [`ratatui::Theme`] trait that combines [`Mapper`]
//! with [`ratatui::style::Style`](https://docs.rs/ratatui/latest/ratatui/style/struct.Style.html) conversion.
//!
//! # Examples
//!
//! [`StyledMapper`] is the default goal of this crate. It heavily maps markdown symbols, and
//! strips many, with the intention of adding syles (color, bold, italics...) later, after wrapping.
//! That is, it does not "stylize" the markdown, but is intented *for* stylizing later.
//!
//! The styles should be applied when iterating over the [`Line`]'s [`Span`]s.
//! ```
//! use mdfrier::{MdFrier, Line, Span, Mapper, DefaultMapper, StyledMapper};
//!
//! let mut frier = MdFrier::new().unwrap();
//!
//! // StyledMapper removes decorators (for use with colors/bold/italic styling)
//! let text: String = frier.parse(80, "*emphasis* and **strong**", &StyledMapper).unwrap()
//!     .flat_map(|l: Line| l.spans.into_iter().map(|s: Span|
//!         // We should really add colors from `s.modifiers` here!
//!         s.content
//!     ))
//!     .collect();
//! assert_eq!(text, "emphasis and strong");
//! ```
//!
//! A custom mapper should implement the [`Mapper`] trait. For example, here we replace some
//! markdown delimiters with fancy symbols.
//! ```
//! use mdfrier::{MdFrier, Mapper};
//!
//! struct FancyMapper;
//! impl Mapper for FancyMapper {
//!     fn emphasis_open(&self) -> &str { "♥" }
//!     fn emphasis_close(&self) -> &str { "♥" }
//!     fn strong_open(&self) -> &str { "✦" }
//!     fn strong_close(&self) -> &str { "✦" }
//!     fn blockquote_bar(&self) -> &str { "➤ " }
//! }
//!
//! let mut frier = MdFrier::new().unwrap();
//!
//! let lines = frier.parse(80, "Hello *world*!\n\n> Quote\n\n**Bold**", &FancyMapper).unwrap();
//! let mut output = String::new();
//! for line in lines {
//!     for span in line.spans {
//!         output.push_str(&span.content);
//!     }
//!     output.push('\n');
//! }
//! assert_eq!(output, "Hello ♥world♥!\n\n➤ Quote\n\n✦Bold✦\n");
//! ```
//!
//! A [`DefaultMapper`] exists, which could be used only style, preserving the markdown content.
//! Note that it would be much more efficient to use the
//! [`tree-sitter-md`](https://crates.io/crates/tree-sitter-md) crate directly instead,
//! since it operates with byte-ranges of the original text. Think editor syntax highlighting.
//! ```
//! use mdfrier::{MdFrier, DefaultMapper};
//!
//! let mut frier = MdFrier::new().unwrap();
//!
//! let text: String = frier.parse(80, "*emphasis* and **strong**", &DefaultMapper).unwrap()
//!     .flat_map(|l| l.spans.into_iter().map(|s| s.content))
//!     .collect();
//! assert_eq!(text, "*emphasis* and **strong**");
//!
//! ```

mod lines;
pub mod mapper;
mod markdown;
mod wrap;

#[cfg(feature = "ratatui")]
pub mod ratatui;

use tree_sitter::Parser;

pub use lines::{BulletStyle, LineIterator};
pub use mapper::{DefaultMapper, Mapper, StyledMapper};
pub use markdown::{Modifier, SourceContent, Span};

use crate::markdown::MdIterator;

// ============================================================================
// Public output types
// ============================================================================

/// A single output line from the markdown parser.
///
/// This is the final, flattened representation with all decorators applied
/// and nesting converted to prefix spans.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    /// The text spans making up this line, including any prefix spans
    /// (blockquote bars, list markers) that were added from nesting.
    pub spans: Vec<Span>,
    /// The kind of content this line represents.
    pub kind: LineKind,
}

/// The kind of content a line represents.
#[derive(Debug, Clone, PartialEq)]
pub enum LineKind {
    /// Regular text paragraph.
    Paragraph,
    /// Header line with tier (1-6).
    Header(u8),
    /// Code block line with language.
    CodeBlock { language: String },
    /// Horizontal rule (content is in spans).
    HorizontalRule,
    /// Table data row.
    TableRow { is_header: bool },
    /// Table border/separator.
    TableBorder,
    /// Image reference.
    Image { url: String, description: String },
    /// Blank line.
    Blank,
}

/// Failed to parse markdown.
#[derive(Debug)]
pub struct MarkdownParseError;

impl std::fmt::Display for MarkdownParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Failed to parse markdown")
    }
}

impl std::error::Error for MarkdownParseError {}

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
    pub fn new() -> Result<Self, MarkdownParseError> {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_md::LANGUAGE.into())
            .ok()
            .ok_or(MarkdownParseError)?;

        let mut inline_parser = Parser::new();
        inline_parser
            .set_language(&tree_sitter_md::INLINE_LANGUAGE.into())
            .ok()
            .ok_or(MarkdownParseError)?;

        Ok(Self {
            parser,
            inline_parser,
        })
    }

    /// Parse markdown text and return an iterator of `Line` items.
    ///
    /// The mapper controls how decorators are rendered (link brackets,
    /// blockquote bars, list markers, etc.). Use `DefaultMapper` for
    /// plain ASCII output, or implement your own `Mapper` for custom symbols.
    ///
    /// # Arguments
    ///
    /// * `width` - The terminal width for line wrapping
    /// * `text` - The markdown text to parse
    /// * `mapper` - The mapper to use for content transformation
    pub fn parse<'a, M: Mapper>(
        &'a mut self,
        width: u16,
        text: &'a str,
        mapper: &'a M,
    ) -> Result<LineIterator<'a, M>, MarkdownParseError> {
        let tree = self.parser.parse(text, None).ok_or(MarkdownParseError)?;
        let iter = MdIterator::new(tree, &mut self.inline_parser, text);
        Ok(LineIterator::new(iter, width, mapper))
    }
}

#[cfg(test)]
#[expect(clippy::unwrap_used)]
mod tests {
    use crate::markdown::SourceContent;

    use super::*;
    use pretty_assertions::assert_eq;

    /// Convert MdLines to a string representation for testing.
    /// With the new flat API, all prefix spans are included in spans.
    fn lines_to_string(lines: &[Line]) -> String {
        lines
            .iter()
            .map(|line| {
                if matches!(line.kind, LineKind::Blank) {
                    String::new()
                } else {
                    line.spans.iter().map(|s| s.content.as_str()).collect()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn parse_simple_text() {
        let mut frier = MdFrier::new().unwrap();
        let lines: Vec<_> = frier
            .parse(80, "Hello world!", &DefaultMapper)
            .unwrap()
            .collect();
        assert_eq!(lines.len(), 1);

        let line = &lines[0];
        assert_eq!(line.spans.len(), 1);
        assert_eq!(line.spans[0].content, "Hello world!");
    }

    #[test]
    fn parse_styled_text() {
        let mut frier = MdFrier::new().unwrap();
        // DefaultMapper preserves decorators around emphasis
        let lines: Vec<_> = frier
            .parse(80, "Hello *world*!", &DefaultMapper)
            .unwrap()
            .collect();
        assert_eq!(lines.len(), 1);

        let line = &lines[0];
        // Spans: "Hello " + "*" (open) + "world" (emphasis) + "*" (close) + "!"
        assert_eq!(line.spans.len(), 5);
        assert_eq!(line.spans[0].content, "Hello ");
        assert_eq!(line.spans[1].content, "*");
        assert!(line.spans[1].modifiers.contains(Modifier::EmphasisWrapper));
        assert_eq!(line.spans[2].content, "world");
        assert!(line.spans[2].modifiers.contains(Modifier::Emphasis));
        assert_eq!(line.spans[3].content, "*");
        assert!(line.spans[3].modifiers.contains(Modifier::EmphasisWrapper));
        assert_eq!(line.spans[4].content, "!");
    }

    #[test]
    fn parse_header() {
        let mut frier = MdFrier::new().unwrap();
        let lines: Vec<_> = frier
            .parse(80, "# Hello\n", &DefaultMapper)
            .unwrap()
            .collect();
        assert_eq!(lines.len(), 1);

        let line = &lines[0];
        assert!(matches!(line.kind, LineKind::Header(1)));
    }

    #[test]
    fn parse_code_block() {
        let mut frier = MdFrier::new().unwrap();
        let lines: Vec<_> = frier
            .parse(80, "```rust\nlet x = 1;\n```\n", &DefaultMapper)
            .unwrap()
            .collect();
        assert_eq!(lines.len(), 1);

        let line = &lines[0];
        assert!(matches!(line.kind, LineKind::CodeBlock { .. }));
        // First span is the code content
        assert!(line.spans[0].content.starts_with("let x = 1;"));
    }

    #[test]
    fn parse_blockquote() {
        let mut frier = MdFrier::new().unwrap();
        let lines: Vec<_> = frier
            .parse(80, "> Hello world", &DefaultMapper)
            .unwrap()
            .collect();
        assert_eq!(lines.len(), 1);

        let line = &lines[0];
        // With flat API, first span should be the blockquote bar
        assert!(line.spans[0].modifiers.contains(Modifier::BlockquoteBar));
        assert_eq!(line.spans[0].content, "> ");
    }

    #[test]
    fn parse_list() {
        let mut frier = MdFrier::new().unwrap();
        let lines: Vec<_> = frier
            .parse(80, "- Item 1\n- Item 2", &DefaultMapper)
            .unwrap()
            .collect();
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn paragraph_breaks() {
        let mut frier = MdFrier::new().unwrap();
        let lines: Vec<_> = frier
            .parse(10, "longline1\nlongline2", &DefaultMapper)
            .unwrap()
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].spans[0].content, "longline1");
        assert_eq!(lines[1].spans[0].content, "longline2");
    }

    #[test]
    fn soft_break_with_styling() {
        let mut frier = MdFrier::new().unwrap();
        // DefaultMapper preserves decorators
        let lines: Vec<_> = frier
            .parse(80, "This \n*is* a test.", &DefaultMapper)
            .unwrap()
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].spans[0].content, "This");
        // Second line: "*" (open) + "is" (emphasis) + "*" (close) + " a test."
        assert_eq!(lines[1].spans[0].content, "*");
        assert!(
            lines[1].spans[0]
                .modifiers
                .contains(Modifier::EmphasisWrapper)
        );
        assert_eq!(lines[1].spans[1].content, "is");
        assert!(lines[1].spans[1].modifiers.contains(Modifier::Emphasis));
        assert_eq!(lines[1].spans[2].content, "*");
        assert!(
            lines[1].spans[2]
                .modifiers
                .contains(Modifier::EmphasisWrapper)
        );
    }

    #[test]
    fn code_block_spacing() {
        let input = "Paragraph before.
```rust
let x = 1;
```
Paragraph after.";

        let mut frier = MdFrier::new().unwrap();
        let lines: Vec<_> = frier.parse(80, input, &DefaultMapper).unwrap().collect();
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
        let lines: Vec<_> = frier.parse(80, input, &DefaultMapper).unwrap().collect();
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
        let lines: Vec<_> = frier.parse(80, input, &DefaultMapper).unwrap().collect();
        let output = lines_to_string(&lines);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn bare_url_line_broken() {
        let mut frier = MdFrier::new().unwrap();
        let spans: Vec<_> = frier
            .parse(15, "See https://example.com/path ok?", &DefaultMapper)
            .unwrap()
            .flat_map(|l| l.spans)
            .collect();
        let url_source = SourceContent::from("https://example.com/path");
        assert_eq!(
            spans,
            vec![
                Span::new("See ".into(), Modifier::empty()),
                Span::new("(".into(), Modifier::LinkURLWrapper),
                Span::source_link(
                    "https://".into(),
                    Modifier::LinkURL | Modifier::BareLink,
                    url_source.clone()
                ),
                Span::source_link(
                    "example.com/".into(),
                    Modifier::LinkURL | Modifier::BareLink,
                    url_source.clone()
                ),
                Span::source_link(
                    "path".into(),
                    Modifier::LinkURL | Modifier::BareLink,
                    url_source.clone()
                ),
                Span::new(")".into(), Modifier::LinkURLWrapper),
                Span::new(" ok?".into(), Modifier::empty()),
            ]
        );
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
        let lines: Vec<_> = frier.parse(80, input, &DefaultMapper).unwrap().collect();
        let output = lines_to_string(&lines);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn code_block_wrapping() {
        // Test that code blocks wrap at width boundary
        let input = "```\nabcdefghij\n```\n";

        let mut frier = MdFrier::new().unwrap();
        // Width of 5 should wrap "abcdefghij" into two lines
        let lines: Vec<_> = frier.parse(5, input, &DefaultMapper).unwrap().collect();
        assert_eq!(lines.len(), 2);
        // First line should be 5 chars
        assert_eq!(lines[0].spans[0].content, "abcde");
        // Second line should be remaining 5 chars
        assert_eq!(lines[1].spans[0].content, "fghij");
    }

    #[test]
    fn code_block_no_wrap_when_fits() {
        // Test that code blocks don't wrap when they fit
        let input = "```\nabcde\n```\n";

        let mut frier = MdFrier::new().unwrap();
        let lines: Vec<_> = frier.parse(5, input, &DefaultMapper).unwrap().collect();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].spans[0].content, "abcde");
    }

    #[test]
    fn hide_urls() {
        let mut frier = MdFrier::new().unwrap();
        struct HideUrlsMapper;
        impl Mapper for HideUrlsMapper {
            fn hide_urls(&self) -> bool {
                true
            }
        }
        let mapper = HideUrlsMapper {};
        let lines: Vec<_> = frier
            .parse(80, "[desc](https://url)", &mapper)
            .unwrap()
            .collect();
        assert_eq!(lines.len(), 1);

        let url_source = SourceContent::from("https://url");
        assert_eq!(
            lines[0].spans,
            vec![
                Span::new(
                    "[".into(),
                    Modifier::Link | Modifier::LinkDescriptionWrapper
                ),
                Span::source_link(
                    "desc".into(),
                    Modifier::Link | Modifier::LinkDescription,
                    url_source.clone()
                ),
                Span::new(
                    "]".into(),
                    Modifier::Link | Modifier::LinkDescriptionWrapper
                ),
            ]
        );
    }
}
