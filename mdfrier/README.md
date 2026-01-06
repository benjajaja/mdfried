# mdfrier

MdFrier - A markdown parser specialized for terminals

⚠️ WARNING ⚠️ This crate is fundamentally not ready for usage out of mdfried (the app).

This crate parses markdown with tree-sitter-md to lines output with a width limit.
Each line consists of "spans", which are stylized (and tagged) fragments.

The optional `ratatui` feature provides conversion to styled ratatui `Line` widgets directly.

## Example

```rust
use mdfrier::{MdFrier, Container, MdModifier};

let mut frier = MdFrier::new().unwrap();
let lines: Vec<_> = frier.parse(80, "Hello *world*!\n\n> This could be really great!\n\n  **--Socrates, probably**".to_owned()).collect();

let mut test_output = String::new();
for line in lines {
    for container in line.meta.nesting {
        match container {
            Container::Blockquote => test_output.push_str("| "),
            _ => {},
        }
    }
    for span in line.spans {
        match span.extra {
            MdModifier::Emphasis => test_output.push('·'),
            MdModifier::StrongEmphasis => test_output.push('˙'),
            _ => {}
        }
        test_output.push_str(&span.content);
        match span.extra {
            MdModifier::Emphasis => test_output.push('·'),
            MdModifier::StrongEmphasis => test_output.push('˙'),
            _ => {}
        }
    }
    test_output.push('\n');
}

assert_eq!(test_output, r#"Hello ·world·!

| This could be really great!

  ˙--Socrates, probably˙
"#);
```

License: GPL-3.0-or-later
