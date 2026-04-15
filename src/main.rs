//! snag — screen OCR tool for Linux.
//!
//! CLI entry point: parses arguments via clap, detects the display server
//! session, dispatches to the correct backend, and prints extracted text
//! to stdout.

mod capture;
mod error;
mod ocr;
mod portal;
mod select;
mod wayland;
mod x11;

fn main() {
    // TODO: CLI args, session detection, dispatch
}
