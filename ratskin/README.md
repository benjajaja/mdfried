Ratskin is a wrapper around [termimad] that parses markdown into [ratatui::text::Line]s.

```rust
#
let rat_skin = RatSkin::default();
let text = RatSkin::parse_text("**cook it!**");
let lines: Vec<Line> = rat_skin.parse(text, 80);
assert_eq!(lines, vec![Line::from(Span::from("cook it!").bold())]);
```

This is all you need to know about Ratskin - for everything else, please see termimad:

* <https://github.com/Canop/termimad>
* <https://crates.io/crates/termimad>
* <https://docs.rs/termimad/latest/termimad/>

Because termimad is very streamlined for writing terminal output directly (for good reasons),
a small part of the logic had to be rewritten for ratatui Spans and Lines.

License: MIT
