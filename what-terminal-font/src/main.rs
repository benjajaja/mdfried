//! Command-line tool to detect and print the terminal font, if detected correctly.

use std::process;

fn main() {
    match what_terminal_font::detect_terminal_font() {
        Ok(font) => {
            println!("{}", font);
            process::exit(0);
        }
        Err(e) => {
            eprintln!("{e}");
            process::exit(1);
        }
    }
}
