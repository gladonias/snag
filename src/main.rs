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

use error::SnagError;

fn run() -> Result<String, SnagError> {
    let lang = "eng";

    let capture = if std::env::var("WAYLAND_DISPLAY").is_ok() {
        // Wayland path — stub for now
        return Err(SnagError::ProtocolUnsupported);
    } else if std::env::var("DISPLAY").is_ok() {
        x11::run()?
    } else {
        return Err(SnagError::NoDisplayServer);
    };

    let engine = ocr::OcrEngine::new(lang)?;
    engine.extract(
        &capture.pixels,
        capture.width,
        capture.height,
        capture.bytes_per_pixel,
    )
}

fn main() {
    match run() {
        Ok(text) => {
            print!("{}", text);
        }
        Err(SnagError::SelectionCancelled) => {
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("snag: {}", e);
            std::process::exit(2);
        }
    }
}
