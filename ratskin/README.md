Render and style markdown in a terminal.

**NOTE**: This crate is not used by [mdfried](https://github.com/benjajaja/mdfried) anymore, so it is not actively maintained. The replacement is [mdfrier](https://crates.io/crates/mdfrier), which has slightly more features and should be well-tested by mdfried.

> How hard can it be?

Well, as it turns out, rendering from markdown *AST* (which you get from the myriad of parsing
libraries) is not trivial - at all!
A terminal is nothing like HTML - we don't need to use special elements like lists, tables,
blocks... we just need to display the text *exactly* as it is layed out, but with some styles
(colors, bold, italics, underline...).
When rendering from AST, the original layout has been lost.
It's extremely tedious to implement all edge cases, and because the specs are not very strict,
it's not really possible to re-create the original.

Luckily, somebody has already solved it: [termimad] produces "terminal-styled" output for markdown.
`rat_skin` leverages [termimad] for [ratatui].
Line wrapping is taken care of, and the styles can be customized with a "skin" (hence the name).

```rust
let rat_skin = RatSkin::default();
let lines: Vec<Line> = rat_skin.parse(RatSkin::parse_text("**cook it!**"), 80);
assert_eq!(lines, vec![Line::from(Span::from("cook it!").bold())]);
```

You can set a [termimad::MadSkin] (re-exported from [termimad]) on [RatSkin] to customize appearances.
The output is a list of [ratatui::text::Line]s, wrapped to the given width.

This is all you need to know about ratskin - for everything else, please see termimad:

* <https://github.com/Canop/termimad/>
* <https://crates.io/crates/termimad/>
* <https://docs.rs/termimad/latest/termimad/>

Because termimad is very streamlined for writing terminal output directly (for good reasons),
a small part of the logic had to be rewritten for ratatui Spans and Lines.

License: MIT
