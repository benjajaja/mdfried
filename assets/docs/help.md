# Help

**Press `Esc` or type `:back` to exit this help document.**

### Key bindings

Key | Alt Key(s) | Description
----|------------|------------
`q` | `Ctrl-c`   | Quit and leave contents on terminal
`r` |            | Reload the file (unless piped stdin)
`j` | `↑`        | Scroll down one line
`k` | `↓`        | Scroll up one line
`d` | `Ctrl-d`   | Scroll down half page
`u` | `Ctrl-u`   | Scroll up half page
`f` | `PageDown`, `Space` | Scroll down a page
`b` | `PageUp`   | Scroll up a page
`g` |            | Go to start of file
`G` |            | Go to end of file
`<number>G` | `<number>g` | Jump to line #\<number>
`/` |            | Search text
`n` |            | Jump to next match or link
`N` |            | Jump to previous match or link
`Enter` |        | Open or follow selected link
`Esc` |          | Leave search or link modes

Entering a number before motion applies the motion that many times.

### Link Navigation

Upon pressing `n` or `N`, "link mode" is activated. 
The nearest hyperlink is highlighted.
Pressing `n` again highlights the next hyperlink, and `N` goes backwards.
Pressing `Enter` opens the hyperlink.
"Opens" means different things, depending on the current source and the link itself.

* If the link ends in `.md`, it will be opened as new document.
* If the link is relative, it will be opened with the same current base URL (if any).
* If the link does not end in `.md`, and `url_transform_command` is configured, it will be fetched, transformed, and openend as new document.

Pressing `Esc` exits "link mode".

### Search

Upon pressing `/`, "search mode input" is activated.
This is indicated by a green `/` in the status bar.

You can then type and erase the search term, highlights in the viewport will be made visible immediately.
Press enter to complete the input, then "search mode" is activated, which works just like [Link Navigation](#link-navigation).

### Commands

Command        | Description
---------------|------------
`:help`        | Opens this help markdown document
`:back`        | Go back one entry in history
`:open <path>` | Open file

### Command Line Interface

```bash
mdfried [OPTIONS] [SOURCE]
```

* `[SOURCE]`
  A file path, a URL, or a `github:<owner>/<repo>` source to open.
  If ommitted, tries to read from stdin, i.e. you can pipe markdown into mdfried.
* `--help`
  CLI help.
* `--version`, `-V`
  Print the version.
* `--watch`, `-w`
  Watch the file for changes and reload.
* `--deep-fry`, `-d`
  Deep fry images.
* `--setup`
  Run the font setup again, if applicable.
* `--print-config`
  Print an example configuration, and the path to the configuration file on your system.
* `--log`
  Log to stderr, useful for debugging (e.g. redirect to another tty: `mdfried --log 2>/dev/pts/7`).

### Configuration

Type `:help configuration` to open [configuration.md](./help_configuration.md).

