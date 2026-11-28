# Changelog

## [Unreleased]

### Added
- `table_row_separators` method on `Mapper` trait, to draw a line between every table row.
- `table_proportional_columns` method on `Mapper` trait, to shrink a too-wide table in proportion
  to its content instead of the greedy shrink.

### Changed
- **Breaking:** the 11 `table_*` border methods on `Mapper` (`table_vertical`, `table_top_left`, ...)
  are replaced by a single `table_symbol(TableSymbol)`. Defaults are unchanged (ASCII), but
  implementors that overrode them must switch to matching on `TableSymbol`.
- `StyledMapper` has rounded table corners (`╭╮╰╯`).

## [3.0.9] - 2026-10-12

## [3.0.8] - 2026-09-18

### Fixed

- Fix cell widths (especially for URLs), 
- Track cell URLs like other URLs for navigation.
- Fix `q`/`quit` commands not existing.

## [3.0.7] - 2026-08-06

Change to `doc_cfg` to fix docs.rs builds.

## [3.0.6] - 2026-07-12

### Added
- `preserve_list_ordinals` method on `Mapper` trait  
  When `true`, ordered list items keep their original source numbers instead of being renumbered
  sequentially.

## [3.0.5] - 2026-06-21

## [3.0.4] - 2026-06-07

## [3.0.3] - 2026-05-31

## [3.0.2] - 2026-05-29

## [3.0.1] - 2026-05-24

## [3.0.0] - 2026-05-19

## [2.0.0] - 2026-05-18

## [1.0.1] - 2026-05-12

### Fixed
- Fix wrapping: wrap once by remaining line width, then with full width.

## [1.0.0] - 2026-04-26

### Removed
- mdfrier::ratatui::Tag
- Span::get_source_content

  Removed source_content field entirely, the URL of a link should be reconstructed by scanning over
  the Link* modifiers instead.

- Span::link constructor

### Changed
- mdfrier::ratatui::render_line takes additional `hide_url` arg.

## [0.3.2] - 2026-04-20

## [0.3.1] - 2026-04-10

## [0.3.0] - 2026-04-08

## [0.2.0] - 2026-01-24

### Added
- Links don't display de URL part by default. Can be disabled by overriding `mapper::Mapper`'s (and if necessary `ratatui::Theme`'s) `hide_url` methods.

