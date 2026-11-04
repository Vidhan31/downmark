# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.2.0] - 2026-11-04

### Added
- Command-line interface with options to select the view layout (`--preview`, `--editor`, `--split`), jump to a line number (`--line`, `file:line`, or `file +line`), and jump to a heading (`--heading`).
- Single-instance window reuse. Opening a document from the terminal or file manager brings the active window forward and creates a new tab instead of launching a separate instance. Unix systems communicate through local domain sockets; Windows systems use named pipes.
- Flag `--new-window` (`-n`) to bypass single-instance reuse and create a separate window.
- Menu action to install the `downmark` shell command into `~/.local/bin` on Linux or `/usr/local/bin` on macOS.
- Automatic handling for macOS process serial number flags (`-psn_*`) passed by Finder or the Dock.

## [1.1.0] - 2026-11-02

### Added
- Glance mode for distraction-free reading. It switches to preview layout and hides the title bar, tab strip, and status bar. Toggle it with F11 or Ctrl+Shift+D, or Cmd+Shift+D on macOS, or press Escape to exit.
- Table action buttons and expanded overlay. Tables in preview mode now provide buttons to copy table Markdown and expand the table into a full-width modal dialog.
- Horizontal table scrolling. Tables wider than the viewport can now scroll horizontally in the preview pane and in the modal overlay, with support for Shift plus mouse wheel and arrow keys.
- Theme syntax colors for inline code. Inline code snippets in the preview and in tables now use syntax colors from the active theme instead of standard text color.
- Layout switcher menu. A bottom-right dropdown menu button allows quick switching between Editor, Split, and Preview layouts.

### Changed
- Default view mode. Documents now open in Preview mode by default instead of Editor view.
- Tabbed navigation. Tabs are now enabled by default when opening multiple documents.
- Modal dialog dismiss. Pressing Escape or clicking outside a dialog now dismisses search and settings dialogs consistently.

### Removed
- Focus mode. Focus mode was removed in favor of Glance mode.

### Fixed
- Table search runaway. Fixed an autoscroll runaway and crash when searching for text contained inside tables.
- Table view mask clipping. Resolved nested mask conflicts that interfered with table scrolling.
- Tab bar clicks. Creating a new tab from the title bar now triggers immediately on click.
- CI test execution. Fixed an issue where the welcome document caused test suites to stall during automated test runs.

## [1.0.0] - 2026-10-28

### Added
- Initial release of Downmark, a fast, native Markdown editor and viewer built with GPUI.

[Unreleased]: https://github.com/Vidhan31/downmark/compare/v1.2.0...HEAD
[1.2.0]: https://github.com/Vidhan31/downmark/compare/v1.1.0...v1.2.0
[1.1.0]: https://github.com/Vidhan31/downmark/compare/v1.0.0...v1.1.0
[1.0.0]: https://github.com/Vidhan31/downmark/releases/tag/v1.0.0
