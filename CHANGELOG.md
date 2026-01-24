# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.0] - 2026-01-24

### New Features

- **Advanced search syntax**: Support for recursive field matching and exact match queries.
- **Improved startup time**: Added caching and expiration logic for game data to significantly reduce startup time for already viewed versions.
- **Automatic data download**: Integrated downloading of game data with `--game` and `--force` options.
- **Game version listing**: New `--game-versions` flag to list available game builds.

## [0.2.0] - 2026-01-23

### New Features

- **Solarized Dark Theme**: Implemented a consistent default theme for the entire UI.
- **JSON Syntax Highlighting**: Added color coding for JSON keys, strings, numbers, and booleans/nulls in the details view.
- **Enhanced Navigation**: Added support for multi-pane focus switching, keyboard/mouse scrolling, and Page Up/Down functionality.
- **Smart Sorting**: Items in the list are now automatically sorted by type and ID.
- **Redraw Optimization**: Optimized terminal rendering to reduce flickering and improve responsiveness.

### Bugfixes

- Fixed JSON key order preservation in the details view.
- Improved JSON highlighting to correctly handle escaped quotes and special characters.

## [0.1.0] - 2026-01-23

### New Features

- **Initial TUI Application**: Basic terminal interface for filtering and viewing Cataclysm: Bright Nights JSON data.
- **Dual-Pane Layout**: Side-by-side list and details view for easy browsing.
- **Real-Time Filtering**: Dynamic filtering of entries based on user input.

[Unreleased]: https://github.com/ushkinaz/cbn-tui/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/ushkinaz/cbn-tui/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/ushkinaz/cbn-tui/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/ushkinaz/cbn-tui/releases/tag/v0.1.0
