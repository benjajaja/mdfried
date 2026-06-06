# Configuration

The configuration file is created and stored automatically the first time mdfried is run.

The get the exact location on your OS, run `mdfried --print-config`, this will print the location followed by a sample config.

The format is TOML. The sections are explained here.

## Basic

```toml
font_family = "your-font-name"
```
The font that has been autodetected or selected, can be changed to any font.
Run `mdfried --setup` to go through font setup *if setup is available*, i.e. if headers would be rendered as images.

```toml
stdio_query_timeout_ms = 2000
```
The timeout in milliseconds to wait for a TTY response. It may be necessary to increaso on older or exotic machines, OSs or terminals, such as Windows.

```toml
max_image_height = 30
```
The maximum image height as terminal row count. The width is kept proportional at the aspect ratio, and capped at the viewport width.

```toml
watch_debounce_milliseconds = 100
```
The watch "debounce" milliseconds, only used in watch mode (`-w`).

```toml
enable_mouse_capture = false
```
Enables mouse capture and mouse scroll, but loses the ability to select text normally on some terminals.
However, most terminals allow to select text holding the shift key, in this mode.

```toml
url_transform_command = "readable | html2text"
```
Transform URLs with a shell command before parsing as markdown. Used when opening a URL that does not end in `.md`.

```toml
mermaid = "mmdc -i - -o - -e png"
```
Mermaid option, can be `true`, `false` or omitted to disable, or a custom external mermaid-cli command.
Renders codeblocks with `mermaid` language as mermaid diagram images.

If `true`, a fast internal renderer is used, but it's not as accurate as using a mermaid-cli command.

```toml
osc8_links = true
```
Render OSC8 hyperlink escape sequences over links, making them clickable in supporting terminals.

## Padding

```toml
[padding]
type = "centered"
value = 100
```
The viewport padding type and maximum width.

## Theme

```toml
[theme]
blockquote_bar = "▌ "
link_desc_open = ""
link_desc_close = ""
link_url_open = "◖"
link_url_close = "◗"
horizontal_rule_char = "─"
task_checked_mark = "[✓] "
blockquote_colors = [
    "202",
    "203",
    "204",
    "205",
    "206",
    "207",
]
link_bg = "237"
link_fg = "4"
prefix_color = "222"
emphasis_color = "220"
code_bg = "236"
code_fg = "203"
hr_color = "240"
table_border_color = "240"
table_header_color = "255"
header_color = "#FFFFFF"
hide_urls = true
hard_softbreaks = false
```
The theme, including colors, replacement strings, and some markdown options.

