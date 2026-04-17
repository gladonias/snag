# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.2] - 2026-04-17

### Fixed

- X11 selection rectangle flickering caused by processing each queued MotionNotify event individually; now coalesces pending motion events and only redraws for the latest cursor position

## [0.1.1] - 2026-04-17

### Fixed

- Integer overflow in Wayland buffer size calculations using checked arithmetic
- Selection starting at (0,0) instead of actual cursor position when clicking without prior mouse movement
- Out-of-bounds reads in Wayland pixel conversion by validating stride and adding bounds checks
- Arbitrary file deletion via unsanitized portal screenshot file paths; now restricted to temp directories
- OOM from oversized portal images by rejecting screenshots exceeding 50MP
- Wrong-monitor capture on multi-monitor setups; overlay now targets the focused output and screencopy uses the active output via wl_surface::enter tracking
- Incorrect capture region on HiDPI displays by applying output scale factors to screencopy coordinates
- OCR postprocessing producing extra blank lines due to off-by-one in the collapsing logic
- Portal signal handler silently dropping results on mutex poisoning

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
