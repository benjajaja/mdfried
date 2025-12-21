# Changelog

## [Unreleased]

## [0.17.1] - 2025-12-21

### Fixed
- Text Sizing Protocol spacing
  All tiers above #1 had letter spacing too wide.

## [0.17.0] - 2025-12-21

### Added
- Watch mode
  Use `-w` to watch the file for changes and reload.
- Print config
  Use `--print-config` to write a default config file to stdout.

### Changed
- Config defaults
  All config entries are now optional.

## [0.16.0] - 2025-12-20

### Added
- Build a static binary
- [Chafa](https://hpjansson.org/chafa/)
  Loaded at runtime, falls back to the existing primitive halfblocks implementation if not found.
- `chafa-dyn` (default) and `chafa-static` features
  Statically building and linking is tricky, so the safe choice it to just stick to `chafa-dyn` and
  then optionally provide `libchafa` on the user's system via distribution means.
  The flake.nix of the projects is an example of how to use `chafa-static`.
- `--no-cap-checks` to entirely skip querying the terminal's stdio for capabilities
  Useful only for running and testing in pseudoterminals.

### Fixed
- Handle panics gracefully (restore terminal mode)

## [0.15.0] - 2025-12-14

### Added
- Search mode

  Keycode `/` enters search mode, similar to Vim. User can enter search term and press `Enter` to
  enter "search mode", where matches are highlighted in green, and jump to first match. Pressing
  `n` and `N` navigates/jumps between matches. The current cursor position is highlighted in red.
  `Esc` clears "search mode".

### Changed
- Link search mode jumps beyond viewport

  Aligned with "search mode".

## [0.14.6] - 2025-11-17
### Fixed
- Missing link offsets

## [0.14.5] - 2025-11-15
### Added
- `debug_override_protocol_type` config/CLI option

## [0.14.4] - 2025-11-10
### Fixed
- Find links after parsing markdown

## [0.14.3] - 2025-11-09
### Added
- macOS binaries

## [0.14.2] - 2025-11-08
### Added
- `max_image_height` config option
### Fixed
- Find original URL of links that have been line-broken

## [0.14.1] - 2025-11-07
### Added
- Logger window (`l` key)
### Fixed
- Greedy regex matching additional `)`

## [0.14.0] - 2025-11-05
### Fixed
- Updates leaving double-lines

## [0.13.0] - 2025-11-03
### Added
- Link navigation mode (`f` key, `n`/`N` to navigate, Enter to open)
- `enable_mouse_capture` config option

  Mouse capture is nice-to-have for scrolling with the wheel, but it blocks text from being 
  selected.

- Detailed configuration error messages

## [0.12.2] - 2025-06-10
### Fixed
- Scrolling fixes
- Headers no longer rendered inside code blocks

## [0.12.1] - 2025-05-23
### Changed
- Code blocks fill whole lines

## [0.12.0] - 2025-05-20
### Added
- Kitty Text Sizing Protocol support

  Leverage the new Text Sizing Protocol for Big Headers™. Super fast in Kitty, falls back to
  rendering-as-images as before on other terminals.

## [0.11.0] - 2025-05-17
### Changed
- Use cosmic-text for font rendering

  Huge improvement on header rendering.

- Improved font picker UX

## [0.8.1] - 2025-01-26
### Added
- Deep fry mode

## [0.8.0] - 2025-01-26
### Changed
- UI on main thread, commands on tokio thread

## [0.7.0] - 2025-01-25
### Added
- Skin config from TOML config file

## [0.6.0] - 2025-01-19
### Changed
- Replace comrak with termimad parser
### Added
- Blockquotes
- Horizontal rules

## [0.5.0] - 2024-12-30
### Added
- Nested list support

## [0.4.0] - 2024-12-28
### Added
- List support
### Fixed
- Word breaking in styled spans

## [0.3.0] - 2024-12-25
### Added
- Windows cross-compilation
- Arch Linux installation
### Changed
- Use textwrap crate

## [0.2.0] - 2024-12-21
### Added
- Initial release
