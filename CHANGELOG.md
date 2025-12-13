# Changelog

## [Unreleased]

### Added
- Search mode

  Keycode `/` enters search mode, similar to Vim. User can enter search term and press `Enter` to
  enter "search mode", where matches are highlighted in green, and jump to first match. Pressing
  `n` and `N` navigates/jumps between matches. The current cursor position is highlighted in red.
  `Esc` clears "search mode".

### Changed
- Link search mode jumps beyond viewport

  Aligned with "search mode".

## [0.14.6]
### Fixed
- Missing link offsets

## [0.14.5]
### Added
- `debug_override_protocol_type` config/CLI option

## [0.14.4]
### Fixed
- Find links after parsing markdown

## [0.14.3]
### Added
- macOS binaries

## [0.14.2]
### Added
- `max_image_height` config option
### Fixed
- Find original URL of links that have been line-broken

## [0.14.1]
### Added
- Logger window (`l` key)
### Fixed
- Greedy regex matching additional `)`

## [0.14.0]
### Fixed
- Updates leaving double-lines

## [0.13.0]
### Added
- Link navigation mode (`f` key, `n`/`N` to navigate, Enter to open)
- `enable_mouse_capture` config option

  Mouse capture is nice-to-have for scrolling with the wheel, but it blocks text from being 
  selected.

- Detailed configuration error messages

## [0.12.2]
### Fixed
- Scrolling fixes
- Headers no longer rendered inside code blocks

## [0.12.1]
### Changed
- Code blocks fill whole lines

## [0.12.0]
### Added
- Kitty Text Sizing Protocol support

  Leverage the new Text Sizing Protocol for Big Headers™. Super fast in Kitty, falls back to
  rendering-as-images as before on other terminals.

## [0.11.0]
### Changed
- Use cosmic-text for font rendering

  Huge improvement on header rendering.

- Improved font picker UX

## [0.8.1]
### Added
- Deep fry mode

## [0.8.0]
### Changed
- UI on main thread, commands on tokio thread

## [0.7.0]
### Added
- Skin config from TOML config file

## [0.6.0]
### Changed
- Replace comrak with termimad parser
### Added
- Blockquotes
- Horizontal rules

## [0.5.0]
### Added
- Nested list support

## [0.4.0]
### Added
- List support
### Fixed
- Word breaking in styled spans

## [0.3.0]
### Added
- Windows cross-compilation
- Arch Linux installation
### Changed
- Use textwrap crate

## [0.2.0]
### Added
- Initial release
