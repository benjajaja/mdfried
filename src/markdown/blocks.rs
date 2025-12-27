use tree_sitter::Parser;

// Crude "pre-parsing" of markdown by lines.
// Headers are always on a line of their own.
// Images are only processed if it appears on a line by itself, to avoid having to deal with text
// wrapping around some area.
#[derive(Debug, PartialEq)]
pub enum Block {
    Header(u8, String),
    Image(String, String),
    Paragraph(String),
    FencedCodeBlock(String),
}

pub fn split_headers_and_images(text: &str) -> Vec<Block> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_md::LANGUAGE.into())
        .unwrap();

    let mut inline_parser = Parser::new();
    inline_parser
        .set_language(&tree_sitter_md::INLINE_LANGUAGE.into())
        .unwrap();

    let tree = parser.parse(text, None).unwrap();

    let mut blocks = Vec::new();

    fn walk_dbg(node: tree_sitter::Node, source: &str, inline_parser: &mut Parser, depth: usize) {
        let text = &source[node.byte_range()];
        println!(
            "{:indent$}{} [{}-{}]: {:?}",
            "",
            node.kind(),
            node.start_byte(),
            node.end_byte(),
            text,
            indent = depth * 2
        );
        if node.kind() == "inline" {
            if let Some(inline_tree) = inline_parser.parse(text, None) {
                println!("{:indent$}  [inline parse]:", "", indent = depth * 2);
                walk_inline(inline_tree.root_node(), text, depth + 2);
            }
        }
        for child in node.children(&mut node.walk()) {
            walk_dbg(child, source, inline_parser, depth + 1);
        }
    }
    fn walk_inline(node: tree_sitter::Node, source: &str, depth: usize) {
        let text = &source[node.byte_range()];
        println!(
            "{:indent$}{}: {:?}",
            "",
            node.kind(),
            text,
            indent = depth * 2
        );
        for child in node.children(&mut node.walk()) {
            walk_inline(child, source, depth + 1);
        }
    }
    // walk_dbg(tree.root_node(), text, &mut inline_parser, 0);

    fn walk(
        node: tree_sitter::Node,
        source: &str,
        depth: usize,
        inline_parser: &mut Parser,
        blocks: &mut Vec<Block>,
    ) {
        match node.kind() {
            "atx_heading" => {
                let mut tier = 0;
                let mut text: &str = "";
                for child in node.children(&mut node.walk()) {
                    match child.kind() {
                        "inline" => text = &source[child.byte_range()],
                        "atx_h1_marker" => tier = 1,
                        "atx_h2_marker" => tier = 2,
                        "atx_h3_marker" => tier = 3,
                        "atx_h4_marker" => tier = 4,
                        "atx_h5_marker" => tier = 5,
                        "atx_h6_marker" => tier = 6,
                        _ => {
                            debug_assert!(false);
                        }
                    }
                }
                blocks.push(Block::Header(tier, text.to_owned()));
            }
            "paragraph" => {
                let cursor = &mut node.walk();
                let mut children = node.children(cursor);
                if children.len() == 1 {
                    // Try to catch paragraphs with only a single image.
                    // Horrible, yes, rip out later and improve to catch all images.
                    let node = children.next().unwrap();
                    if node.kind() == "inline" {
                        let inline_source = &source[node.byte_range()];
                        if let Some(inline_tree) = inline_parser.parse(inline_source, None) {
                            let inline_root = inline_tree.root_node();
                            if inline_root.kind() == "inline" {
                                let cursor = &mut inline_root.walk();
                                let mut children = inline_root.children(cursor);
                                if children.len() == 1 {
                                    let inline_node = children.next().unwrap();
                                    if inline_node.kind() == "image" {
                                        let mut image_description: &str = "";
                                        let mut link_destination: &str = "";
                                        for child in inline_node.children(&mut inline_node.walk()) {
                                            match child.kind() {
                                                "image_description" => {
                                                    image_description =
                                                        &inline_source[child.byte_range()]
                                                }
                                                "link_destination" => {
                                                    link_destination =
                                                        &inline_source[child.byte_range()]
                                                }
                                                _ => {}
                                            }
                                        }
                                        blocks.push(Block::Image(
                                            image_description.to_owned(),
                                            link_destination.to_owned(),
                                        ));
                                        return;
                                    }
                                }
                            }
                        }
                    }
                }

                blocks.push(Block::Paragraph(source[node.byte_range()].to_owned()));
            }
            "fenced_code_block" => {
                blocks.push(Block::FencedCodeBlock(source[node.byte_range()].to_owned()));
            }
            _ => {
                for child in node.children(&mut node.walk()) {
                    walk(child, source, depth + 1, inline_parser, blocks);
                }
            }
        }
    }

    walk(tree.root_node(), text, 0, &mut inline_parser, &mut blocks);
    blocks
}

#[cfg(test)]
mod tests {
    use crate::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn new_style() {
        let blocks = markdown::split_headers_and_images(
            r#"
# header *stronk*!

blablagraph

## a baby
"#,
        );
        assert_eq!(
            blocks,
            vec![
                markdown::Block::Header(1, "header *stronk*!".to_owned()),
                markdown::Block::Paragraph("blablagraph\n".to_owned()),
                markdown::Block::Header(2, "a baby".to_owned()),
            ]
        );
    }

    #[test]
    fn split_headers_and_paragraphs() {
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
                markdown::Block::Paragraph("paragraph\n".to_owned()),
                markdown::Block::Paragraph("paragraph\n".to_owned()),
                markdown::Block::Header(1, "header".to_owned()),
                markdown::Block::Paragraph("paragraph\nparagraph\n".to_owned()),
                markdown::Block::Header(1, "header".to_owned()),
                markdown::Block::Paragraph("paragraph\n".to_owned()),
                markdown::Block::Header(1, "header".to_owned()),
            ]
        );
    }

    #[test]
    fn split_headers_and_paragraphs_without_space() {
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
                markdown::Block::Paragraph("paragraph\n".to_owned()),
                markdown::Block::Header(1, "header".to_owned()),
                markdown::Block::Header(1, "header".to_owned()),
                markdown::Block::Paragraph("paragraph\n".to_owned()),
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
                markdown::Block::Paragraph("paragraph\n".to_owned()),
                markdown::Block::FencedCodeBlock(
                    r#"```c
#ifdef FOO
bar();
#endif
```
"#
                    .to_owned()
                ),
                markdown::Block::Paragraph("paragraph\n".to_owned()),
                markdown::Block::FencedCodeBlock(
                    r#"  ~~~~
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
                markdown::Block::Paragraph("paragraph\n".to_owned()),
            ]
        );
    }

    #[test]
    fn split_headers_and_images() {
        let blocks = markdown::split_headers_and_images(
            r#"
# header

paragraph

![zherkalo](./mirror.jpg)
"#,
        );
        assert_eq!(
            blocks,
            vec![
                markdown::Block::Header(1, "header".to_owned()),
                markdown::Block::Paragraph("paragraph\n".to_owned()),
                markdown::Block::Image("zherkalo".to_owned(), "./mirror.jpg".to_owned()),
            ]
        );
    }
}
