# mdfrier

mdfrier - Deep fry markdown for [mdfried](https://crates.io/crates/mdfried).

This crate parses markdown with tree-sitter-md into wrapped lines for a fixed width styled
output.

This isn't as straightforward as wrapping the source and then highlighting syntax, because the
wrapping relies on markdown context. The process is:

1. Parse into raw lines with nodes
2. Map the node's markdown symbols (optionally, because we want to strip e.g. `*` when
   highlighting with color later)
3. Wrap the lines of nodes to a maximum width
4. ???

At step 4, the users of this library will typically convert the wrapped lines of nodes with
their style information to whatever the target is: ANSI escape sequences, or whatever some
their library expects.

There is a `ratatui` feature that enables the [`ratatui`] module, which does exactly this, for
[ratatui](https://ratatui.rs).

The [`Mapper`] trait controls decorator symbols (e.g., blockquote bar, link brackets).
The optional `ratatui` feature provides the [`ratatui::Theme`] trait that combines [`Mapper`]
with [`ratatui::style::Style`](https://docs.rs/ratatui/latest/ratatui/style/struct.Style.html) conversion.

## Examples

[`StyledMapper`] is the default goal of this crate. It heavily maps markdown symbols, and
strips many, with the intention of adding syles (color, bold, italics...) later, after wrapping.
That is, it does not "stylize" the markdown, but is intented *for* stylizing later.

The styles should be applied when iterating over the [`Line`]'s [`Span`]s.
```rust
use mdfrier::{MdFrier, Line, Span, Mapper, DefaultMapper, StyledMapper};

let mut frier = MdFrier::new().unwrap();

// StyledMapper removes decorators (for use with colors/bold/italic styling)
let lines = frier.parse(80, "*emphasis* and **strong**".to_owned(), &StyledMapper);
let text: String = lines.iter()
    .flat_map(|l: &Line| l.spans.iter().map(|s: &Span|
        // We should really add colors from `s.modifiers` here!
        s.content.as_str()
    ))
    .collect();
assert_eq!(text, "emphasis and strong");
```

A custom mapper should implement the [`Mapper`] trait. For example, here we replace some
markdown delimiters with fancy symbols.
```rust
use mdfrier::{MdFrier, Mapper};

struct FancyMapper;
impl Mapper for FancyMapper {
    fn emphasis_open(&self) -> &str { "♥" }
    fn emphasis_close(&self) -> &str { "♥" }
    fn strong_open(&self) -> &str { "✦" }
    fn strong_close(&self) -> &str { "✦" }
    fn blockquote_bar(&self) -> &str { "➤ " }
}

let mut frier = MdFrier::new().unwrap();

let lines = frier.parse(80, "Hello *world*!\n\n> Quote\n\n**Bold**".to_owned(), &FancyMapper);
let mut output = String::new();
for line in lines {
    for span in line.spans {
        output.push_str(&span.content);
    }
    output.push('\n');
}
assert_eq!(output, "Hello ♥world♥!\n\n➤ Quote\n\n✦Bold✦\n");
```

A [`DefaultMapper`] exists, which could be used only style, preserving the markdown content.
Note that it would be much more efficient to use the
[`tree-sitter-md`](https://crates.io/crates/tree-sitter-md) crate directly instead,
since it operates with byte-ranges of the original text. Think editor syntax highlighting.
```rust
use mdfrier::{MdFrier, DefaultMapper};

let mut frier = MdFrier::new().unwrap();

let lines = frier.parse(80, "*emphasis* and **strong**".to_owned(), &DefaultMapper);
let text: String = lines.iter()
    .flat_map(|l| l.spans.iter().map(|s| s.content.as_str()))
    .collect();
assert_eq!(text, "*emphasis* and **strong**");

```

License: GPL-3.0-or-later
