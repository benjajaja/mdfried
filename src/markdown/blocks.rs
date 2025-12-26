use regex::Regex;

// Crude "pre-parsing" of markdown by lines.
// Headers are always on a line of their own.
// Images are only processed if it appears on a line by itself, to avoid having to deal with text
// wrapping around some area.
#[derive(Debug, PartialEq)]
pub enum Block {
    Header(u8, String),
    Image(String, String),
    Markdown(String),
}

pub fn split_headers_and_images(text: &str) -> Vec<Block> {
    // Regex to match lines starting with 1-6 `#` characters
    let header_re = Regex::new(r"^(#+)\s*(.*)").expect("regex");
    // Regex to match standalone image lines: ![alt](url)
    let image_re = Regex::new(r"^!\[(.*?)\]\((.*?)\)$").expect("regex");
    // Regex to match beginning or end of code fence
    let codefence_re = Regex::new(r"^ {0,3}(`{3,}|~{3,})").expect("regex");

    let mut blocks = Vec::new();
    let mut current_block = String::new();
    let mut current_codefence: Option<String> = None;

    for line in text.lines() {
        if let Some(codefence_str) = &current_codefence {
            if !current_block.is_empty() {
                current_block.push('\n');
            }
            current_block.push_str(line);
            if let Some(captures) = codefence_re.captures(line) {
                // End of codefence must match start, with at least as many characters
                if captures[1].starts_with(codefence_str) {
                    current_codefence = None;
                }
            }
        } else if let Some(captures) = header_re.captures(line) {
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
        } else if let Some(captures) = codefence_re.captures(line) {
            if !current_block.is_empty() {
                current_block.push('\n');
            }
            current_block.push_str(line);
            current_codefence = Some(captures[1].to_string());
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

#[cfg(test)]
mod tests {
    use crate::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn split_headers_and_images() {
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
                markdown::Block::Header(1, "header".to_owned()),
                markdown::Block::Markdown("paragraph\n\nparagraph\n".to_owned()),
                markdown::Block::Header(1, "header".to_owned()),
                markdown::Block::Markdown("paragraph\nparagraph\n".to_owned()),
                markdown::Block::Header(1, "header".to_owned()),
                markdown::Block::Markdown("paragraph\n".to_owned()),
                markdown::Block::Header(1, "header".to_owned()),
            ]
        );
    }

    #[test]
    fn split_headers_and_images_without_space() {
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
                markdown::Block::Header(1, "header".to_owned()),
                markdown::Block::Markdown("paragraph".to_owned()),
                markdown::Block::Header(1, "header".to_owned()),
                markdown::Block::Header(1, "header".to_owned()),
                markdown::Block::Markdown("paragraph".to_owned()),
                markdown::Block::Header(1, "header".to_owned()),
            ]
        );
    }

    #[test]
    fn codefence() {
        let blocks = markdown::split_headers_and_images(
            r#"
# header

paragraph

```c
#ifdef FOO
bar();
#endif
```

paragraph

  ~~~~
  x("
  ~~~
  ");
  #define Y
  z();
  ~~~~

# header

paragraph
"#,
        );
        assert_eq!(
            blocks,
            vec![
                markdown::Block::Header(1, "header".to_owned()),
                markdown::Block::Markdown(
                    r#"paragraph

```c
#ifdef FOO
bar();
#endif
```

paragraph

  ~~~~
  x("
  ~~~
  ");
  #define Y
  z();
  ~~~~
"#
                    .to_owned()
                ),
                markdown::Block::Header(1, "header".to_owned()),
                markdown::Block::Markdown("paragraph".to_owned()),
            ]
        );
    }
}
