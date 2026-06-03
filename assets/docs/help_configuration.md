# Configuration

The configuration is created and stored automatically the first time mdfried is run.

The get the exact location on your OS, run `mdfried --print-config`, it will print the location followed by a sample config.

The config is a TOML file. The sections are explained here.

## Basic

The font that has been autodetected or selected, can be changed at will.
Run `mdfried --setup` to go through font setup *if setup is available*, e.g. headers would be rendered as images.
```toml
font_family = "your-font-name"
```

The timeout in milliseconds to wait for a TTY response, may be necessary to increaso on older machines or terminals.
```toml
stdio_query_timeout_ms = 2000
```

The maximum image height as terminal row count.
```toml
max_image_height = 30
```

The watch "debounce" milliseconds, only used in watch mode (`-w`).
```toml
watch_debounce_milliseconds = 100
```

Enables mouse capture and mouse scroll, but loses the ability to select text on some terminals.
```toml
enable_mouse_capture = false
```

Transform URLs with a shell command before parsing as markdown.
```toml
url_transform_command = "readable | html2text"
```

Mermaid option, can be `true`, `false` or omitted, or a custom external mermaid-cli command.
```toml
mermaid = "mmdc -i - -o - -e png"
```

The padding type and width.
```toml
[padding]
type = "centered"
value = 100
```

The theme, including replacement strings and options.
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
