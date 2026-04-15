//! Tesseract OCR wrapper.
//!
//! Takes a raw pixel buffer and image dimensions, returns extracted text
//! as a `String`. Uses the `leptess` crate which wraps `libtesseract`.

use crate::error::SnagError;
use std::ffi::CString;

pub struct OcrEngine {
    lang: String,
}

impl OcrEngine {
    pub fn new(lang: &str) -> Result<Self, SnagError> {
        // Verify we can initialise Tesseract with this language before proceeding.
        let test = leptess::tesseract::TessApi::new(None, lang);
        if test.is_err() {
            return Err(SnagError::OcrError(format!(
                "Tesseract language data for '{}' not found. Install with: sudo pacman -S tesseract-data-{}",
                lang,
                lang.split('+').next().unwrap_or(lang)
            )));
        }
        Ok(Self {
            lang: lang.to_string(),
        })
    }

    pub fn extract(
        &self,
        pixels: &[u8],
        width: u32,
        height: u32,
        bytes_per_pixel: u32,
    ) -> Result<String, SnagError> {
        let mut api = leptess::tesseract::TessApi::new(None, &self.lang)
            .map_err(|e| SnagError::OcrError(format!("Failed to initialise Tesseract: {}", e)))?;

        // Suppress Tesseract debug output
        let name = CString::new("debug_file").map_err(|e| SnagError::OcrError(e.to_string()))?;
        let value = CString::new("/dev/null").map_err(|e| SnagError::OcrError(e.to_string()))?;
        let _ = api.raw.set_variable(&name, &value);

        let bytes_per_line = (width * bytes_per_pixel) as i32;
        api.raw
            .set_image(
                pixels,
                width as i32,
                height as i32,
                bytes_per_pixel as i32,
                bytes_per_line,
            )
            .map_err(|e| SnagError::OcrError(format!("Failed to set image: {:?}", e)))?;

        // Set a reasonable resolution to avoid Tesseract warnings
        api.raw.set_source_resolution(72);

        let text = api
            .raw
            .get_utf8_text()
            .map_err(|e| SnagError::OcrError(format!("Failed to get text: {:?}", e)))?;

        let cstr: &std::ffi::CStr = text.as_ref();
        let text_str = cstr
            .to_str()
            .map_err(|e| SnagError::OcrError(format!("UTF-8 conversion failed: {}", e)))?;

        Ok(postprocess(text_str))
    }
}

/// Light post-processing: trim whitespace, collapse runs of >2 blank lines to 2.
fn postprocess(text: &str) -> String {
    let trimmed = text.trim();
    let mut result = String::with_capacity(trimmed.len());
    let mut consecutive_blank = 0;

    for line in trimmed.lines() {
        if line.trim().is_empty() {
            consecutive_blank += 1;
            if consecutive_blank <= 2 {
                result.push('\n');
            }
        } else {
            consecutive_blank = 0;
            if !result.is_empty() {
                result.push('\n');
            }
            result.push_str(line);
        }
    }

    result
}
