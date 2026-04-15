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

use clap::Parser;
use error::SnagError;

#[derive(Parser)]
#[command(
    name = "snag",
    version,
    about = "Screen OCR tool for Linux — draw a selection, extract text to stdout"
)]
struct Cli {
    /// Tesseract language code (default: eng). Multiple languages: -l deu+eng
    #[arg(short = 'l', long = "lang", default_value = "eng")]
    lang: String,

    /// Print available Tesseract language data and exit
    #[arg(long = "list-langs")]
    list_langs: bool,
}

fn run() -> Result<String, SnagError> {
    let cli = Cli::parse();

    if cli.list_langs {
        return list_languages();
    }

    let capture = detect_and_capture()?;

    let engine = ocr::OcrEngine::new(&cli.lang)?;
    engine.extract(
        &capture.pixels,
        capture.width,
        capture.height,
        capture.bytes_per_pixel,
    )
}

fn detect_and_capture() -> Result<capture::CaptureResult, SnagError> {
    if std::env::var("WAYLAND_DISPLAY").is_ok() {
        let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
        let is_wlroots = matches!(
            desktop.to_lowercase().as_str(),
            "hyprland" | "sway" | "wlroots"
        );

        if is_wlroots {
            match wayland::run() {
                Ok(result) => Ok(result),
                Err(SnagError::ProtocolUnsupported) => portal::run(),
                Err(e) => Err(e),
            }
        } else {
            portal::run()
        }
    } else if std::env::var("DISPLAY").is_ok() {
        x11::run()
    } else {
        Err(SnagError::NoDisplayServer)
    }
}

fn list_languages() -> Result<String, SnagError> {
    let output = std::process::Command::new("tesseract")
        .arg("--list-langs")
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                SnagError::OcrError("Tesseract is not installed or not in $PATH".into())
            } else {
                SnagError::IoError(e)
            }
        })?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Tesseract prints the list to stderr on some versions, stdout on others
    let text = if stdout.trim().is_empty() {
        stderr.to_string()
    } else {
        stdout.to_string()
    };

    // Print directly and return empty success
    eprint!("{}", text);
    Ok(String::new())
}

fn main() {
    match run() {
        Ok(text) => {
            if !text.is_empty() {
                print!("{}", text);
            }
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
