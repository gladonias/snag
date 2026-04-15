//! Unified error type for snag.
//!
//! All fallible operations across backends return `SnagError`.

#[derive(Debug, thiserror::Error)]
pub enum SnagError {
    #[error("No display server detected. Set $WAYLAND_DISPLAY or $DISPLAY.")]
    NoDisplayServer,

    #[error("Wayland compositor does not support wlr-screencopy protocol")]
    ProtocolUnsupported,

    #[error("Wayland error: {0}")]
    WaylandError(String),

    #[error("X11 error: {0}")]
    X11Error(String),

    #[error("Portal error: {0}")]
    PortalError(String),

    #[error("OCR error: {0}")]
    OcrError(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Selection cancelled")]
    SelectionCancelled,
}
