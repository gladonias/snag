# snag

Screen OCR tool for Linux. Draw a selection rectangle on screen, extract the text inside it, print it to stdout.

## Why snag?

Linux has no good built-in equivalent to macOS TextSniper or Windows PowerToys Text Extractor. The existing options each have drawbacks:

- **NormCap** is a full GUI application with a Python runtime, clipboard integration, and desktop notifications — far heavier than needed for a simple text grab.
- **grim + slurp + tesseract** pipelines work on wlroots compositors, but they require stitching together multiple tools with shell glue, don't work on KDE/GNOME Wayland or X11, and break when any component changes its output format.

snag is a single compiled binary that handles selection, capture, and OCR internally. It works across Wayland (wlroots and portal-based compositors) and X11, outputs plain text to stdout, and gets out of the way.

## Philosophy

- **Do one thing**: draw a box, extract the text inside, print it.
- **Unix composability**: output is always plain text on stdout. The shell handles the rest — clipboard, files, notifications, pipes.
- **No GUI, no state**: no window that stays open, no clipboard integration, no history, no config file, no daemon.
- **Single binary**: no runtime dependencies beyond system libraries.

## Requirements

snag requires the following system libraries at runtime:

- **Tesseract** and at least one language data pack
- **Leptonica** (pulled in by Tesseract)
- **libwayland-client** (for Wayland compositors)
- **libX11** and **libXext** (for X11/Xorg)
- **D-Bus** (for the xdg-desktop-portal path on KDE/GNOME Wayland)

On Arch Linux:

```bash
sudo pacman -S --needed tesseract tesseract-data-eng leptonica wayland libx11 libxext dbus
```

Build dependencies (only needed for compiling from source):

```bash
sudo pacman -S --needed base-devel pkg-config rust
```

## Installation

Build from source:

```bash
git clone https://codeberg.org/gadonias/snag.git
cd snag
cargo build --release
```

Copy the binary to your PATH:

```bash
cp target/release/snag ~/.local/bin/
```

Or system-wide:

```bash
sudo cp target/release/snag /usr/local/bin/
```

## Usage

```
snag                      Select a region and extract text to stdout
snag -l <lang>            Tesseract language code (default: eng)
snag -l deu+eng           Multiple languages (Tesseract syntax)
snag --list-langs         Print available Tesseract language data and exit
snag --version            Print version
snag --help               Print help
```

During selection:

- **Click and drag** to draw the selection rectangle
- **Release** to confirm
- **Escape** or **right-click** to cancel

Exit codes:

| Code | Meaning |
|------|---------|
| 0    | Success — text was extracted and written to stdout |
| 1    | Selection was cancelled (Escape or right-click) |
| 2    | Error (OCR failure, missing language data, display server issue) |

## Composability examples

Copy extracted text to clipboard (Wayland):

```bash
snag | wl-copy
```

Copy to clipboard (X11):

```bash
snag | xclip -selection clipboard
```

Append to a notes file:

```bash
snag >> ~/notes.txt
```

Show a desktop notification with the result:

```bash
snag && notify-send "snag" "$(snag)"
```

Pipe through translation:

```bash
snag -l deu | trans -b de:en
```

Use in a script with error handling:

```bash
if text=$(snag); then
    echo "$text" | wl-copy
    notify-send "Copied" "$text"
else
    notify-send "snag" "Selection cancelled or failed"
fi
```

## Compositor compatibility

| Compositor | Backend | Selection UI |
|---|---|---|
| **Hyprland** | wlroots (layer shell + wlr-screencopy) | Built-in overlay |
| **Sway** | wlroots (layer shell + wlr-screencopy) | Built-in overlay |
| **KDE Plasma** (Wayland) | xdg-desktop-portal | Native portal picker |
| **GNOME** (Wayland) | xdg-desktop-portal | Native portal picker |
| **X11 / Xorg** | X11 (override-redirect window + XGetImage) | Built-in overlay |

On wlroots compositors, if `zwlr_screencopy_manager_v1` is not available, snag automatically falls back to the xdg-desktop-portal path.

## OCR language support

Tesseract requires language data packs. On Arch Linux, they follow the pattern `tesseract-data-<code>`:

```bash
# Install German language data
sudo pacman -S tesseract-data-deu

# Install Japanese
sudo pacman -S tesseract-data-jpn
```

List installed languages:

```bash
snag --list-langs
```

Use a specific language:

```bash
snag -l deu
```

Combine multiple languages (useful for mixed-language documents):

```bash
snag -l deu+eng
```

## Disclaimer

This project is built with the assistance of an AI assistant.

## License

[AGPL-3.0](LICENSE)
