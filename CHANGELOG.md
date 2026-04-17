# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-04-15

### Added

- Interactive selection overlay for drawing capture rectangles
- Wayland support via layer shell and wlr-screencopy (wlroots compositors)
- Wayland support via xdg-desktop-portal (KDE Plasma, GNOME)
- Automatic fallback from wlr-screencopy to portal when unavailable
- X11 support via override-redirect window and XGetImage
- OCR via Tesseract through leptess bindings
- Language selection with `-l` flag (single or combined, e.g. `deu+eng`)
- `--list-langs` to print available Tesseract language data
- Structured error types with distinct exit codes (0, 1, 2)
- Automatic display server detection (Wayland vs X11)
- Plain text output to stdout for Unix composability
