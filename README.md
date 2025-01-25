# mdfried

You can [cook](https://ratatui.rs/) a terminal. But can you **deep fry** a terminal?
_YES!_ You can **cook _and_ fry** your `tty`! ~Run before it's too late!~

> The terminal is usually in "cooked" mode, or canonical mode.
> With `ratatui`🐁, it's in raw mode, but it "cooks" for you.

`mdfried` is a markdown viewer for the terminal that renders headers as bigger text than normal.

### Screenshots

![Screenshot](./assets/screenshot_1.png)

### Video

![Screenshot](./assets/demo.gif)

### How?

By rendering the headers as images, and using one of several terminal graphics protocols: Sixels,
Kitty, or iTerm2.

See [ratatui-image](https://github.com/benjajaja/ratatui-image?tab=readme-ov-file#compatibility-matrix)
to see if your terminal does even have graphics support, and for further details.

In general, Kitty, WezTerm, iTerm2, foot, `xterm -ti vt340`, _should_ work.

# Usage

```
mdfried ./path/to.md
```

The first time you run `mdfried`, you will have to pick a font, ideally the font your terminal is
using. As you type in the prompt, the first match is previewed directly. Once confirmed, this is
written into the configuration file. Use `--setup` to force the font-setup again if the font is not
right, or you switch terminals.

Press `q` to quit. `j`/`k` to scroll by lines, `Ctrl-d`/`Ctrl-u` to scroll by pages. `r` reloads
the file (if not using stdin pipe). Mouse scroll also works.

# Installation

- Rust cargo: `cargo install mdfried`
- Nix flake: `github:benjajaja/mdfried`
- Arch Linux: `paru -S mdfried` ([AUR](https://aur.archlinux.org/packages/mdfried))
- Windows: [Download .exe](https://github.com/benjajaja/mdfried/releases/latest)

# Configuration

`$XDG_CONFIG/mdfried/config.toml` is automatically created on first run.
The `[skin]` section can be configured to set various colors and styles.
See [termimad skin format](https://github.com/Canop/termimad/blob/main/examples/serialize-skin/skin.hjson)
for more information.
