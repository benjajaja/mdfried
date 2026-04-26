# Changelog

## [Unreleased]

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

