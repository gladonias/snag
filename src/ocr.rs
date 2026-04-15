//! Tesseract OCR wrapper.
//!
//! Takes a raw pixel buffer and image dimensions, returns extracted text
//! as a `String`. Uses the `leptess` crate which wraps `libtesseract`.
