//! Cross-backend abstraction for pixel buffer retrieval.
//!
//! Defines `CaptureResult` — the raw pixel data, dimensions, and format
//! returned by each capture backend.

/// Raw pixel data captured from a screen region.
pub struct CaptureResult {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub bytes_per_pixel: u32,
}
