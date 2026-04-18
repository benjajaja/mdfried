#![expect(clippy::unwrap_used)]

use super::*;
use std::fs::File;
use std::io::{self, BufReader};

struct TestTerminal;

impl TerminalConfig for TestTerminal {
    fn ghostty(&self) -> Result<String, WtfError> {
        Ok(r#"command = /run/current-system/sw/bin/fish
click-repeat-interval = 500
gtk-single-instance = false
font-family = GhostFaceKilla
auto-update-channel = stable
"#
        .to_owned())
    }

    fn kitty(&self) -> Result<String, WtfError> {
        Ok(r#"name: kitty
version: 0.44.0
allow_hyperlinks: yes
font_family: PussyKatNFM
bold_font: PussyKatNFM-Bold
italic_font: PussyKatNFM-Italic
bold_italic_font: PussyKatNFM-Italic
font_size: 12
dpi_x: 144
dpi_y: 144
foreground: #dddddd
background: #1c1a23
background_opacity: 1
clipboard_control: write-clipboard write-primary read-clipboard-ask read-primary-ask
os_name: linux"#
            .to_owned())
    }

    fn rio(&self) -> Result<BufReader<File>, WtfError> {
        let mut temp_file = tempfile::NamedTempFile::new()?;
        io::Write::write_all(
            &mut temp_file,
            b"[colors]\nbackground = '#1e1e2e'\n[font]\nfamily = \"Rio De Janeiro\"\nsize = 14.0",
        )?;
        let file = temp_file.into_file();
        Ok(BufReader::new(file))
    }

    fn wezterm(&self) -> Result<String, WtfError> {
        Ok(r#"{
  "font": {
    "family": "JetBrains Mono",
    "size": 12.0
  }
}"#
        .to_owned())
    }

    fn foot(&self) -> Result<BufReader<File>, WtfError> {
        let mut temp_file = tempfile::NamedTempFile::new()?;
        io::Write::write_all(&mut temp_file, b"xxx\nfont=FootPrint:size=12\nblablabla\n")?;
        let file = temp_file.into_file();
        Ok(BufReader::new(file))
    }
}

fn unrelated() -> Result<String, env::VarError> {
    Ok("unrelated".to_owned())
}

#[test]
fn ghostty() {
    let result = detect(TestTerminal, Ok("ghostty".to_owned()), unrelated());
    assert_eq!(result.unwrap(), "GhostFaceKilla");
}

#[test]
fn wezterm() {
    let result = detect(TestTerminal, Ok("ghostty".to_owned()), unrelated());
    assert_eq!(result.unwrap(), "GhostFaceKilla");
}

#[test]
fn rio() {
    let result = detect(TestTerminal, Ok("rio".to_owned()), unrelated());
    assert_eq!(result.unwrap(), "Rio De Janeiro");
}

#[test]
fn foot() {
    let test_terminal = TestTerminal;
    let result = detect(test_terminal, unrelated(), Ok("foot".to_owned()));
    assert_eq!(result.unwrap(), "FootPrint");
}

#[test]
fn kitty() {
    let result = detect(TestTerminal, unrelated(), Ok("xterm-kitty".to_owned()));
    assert_eq!(result.unwrap(), "PussyKatNFM");
}

#[test]
fn unknown_terminal() {
    let result = detect_with_term_program(Ok("unknown-terminal".to_owned()), &TestTerminal);
    assert!(matches!(result.unwrap_err(), WtfError::UnknownTerminal));
}
