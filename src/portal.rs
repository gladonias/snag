//! xdg-desktop-portal screenshot path.
//!
//! Uses `org.freedesktop.portal.Screenshot` for compositors that do not
//! support wlr-screencopy (KDE Plasma, GNOME Wayland). The portal
//! provides its own native region picker.

use crate::capture::CaptureResult;
use crate::error::SnagError;
use dbus::arg::{PropMap, RefArg, Variant};
use dbus::blocking::Connection;
use dbus::message::MatchRule;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const PORTAL_DEST: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const SCREENSHOT_IFACE: &str = "org.freedesktop.portal.Screenshot";
const REQUEST_IFACE: &str = "org.freedesktop.portal.Request";

/// Possible portal response outcomes.
enum PortalResponse {
    Success(String), // URI
    Cancelled,
    Failed(String),
}

/// Run the portal screenshot pipeline.
pub fn run() -> Result<CaptureResult, SnagError> {
    let conn = Connection::new_session().map_err(|e| SnagError::PortalError(e.to_string()))?;

    let handle_token = format!("snag_{}", std::process::id());

    // Build the request object path from our sender name
    let sender = conn.unique_name().trim_start_matches(':').replace('.', "_");
    let request_path = format!(
        "/org/freedesktop/portal/desktop/request/{}/{}",
        sender, handle_token
    );

    // Shared state for the signal callback
    let result: Arc<Mutex<Option<PortalResponse>>> = Arc::new(Mutex::new(None));
    let result_clone = result.clone();

    // Register signal handler before making the call
    let rule = MatchRule::new_signal(REQUEST_IFACE, "Response").with_path(request_path.clone());
    conn.add_match(rule, move |_: (), _conn, msg: &dbus::Message| {
        let response = parse_response(msg);
        let mut lock = result_clone.lock().unwrap_or_else(|e| e.into_inner());
        *lock = Some(response);
        true
    })
    .map_err(|e| SnagError::PortalError(e.to_string()))?;

    // Build options dict
    let mut options: HashMap<String, Variant<Box<dyn RefArg>>> = HashMap::new();
    options.insert(
        "handle_token".to_string(),
        Variant(Box::new(handle_token.clone())),
    );
    options.insert("interactive".to_string(), Variant(Box::new(true)));

    // Call Screenshot
    let proxy = conn.with_proxy(PORTAL_DEST, PORTAL_PATH, Duration::from_secs(5));
    let _reply: (dbus::Path,) = proxy
        .method_call(SCREENSHOT_IFACE, "Screenshot", ("", options))
        .map_err(|e| SnagError::PortalError(e.to_string()))?;

    // Wait for the Response signal (up to 120 seconds for user interaction)
    let deadline = std::time::Instant::now() + Duration::from_secs(120);

    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return Err(SnagError::PortalError(
                "Timed out waiting for screenshot".into(),
            ));
        }

        conn.process(remaining.min(Duration::from_millis(200)))
            .map_err(|e| SnagError::PortalError(e.to_string()))?;

        {
            let lock = result.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(ref response) = *lock {
                return match response {
                    PortalResponse::Success(uri) => decode_screenshot(uri),
                    PortalResponse::Cancelled => Err(SnagError::SelectionCancelled),
                    PortalResponse::Failed(msg) => Err(SnagError::PortalError(msg.clone())),
                };
            }
        }
    }
}

fn parse_response(msg: &dbus::Message) -> PortalResponse {
    let parsed: Result<(u32, PropMap), _> = msg.read2();
    match parsed {
        Ok((response_code, results)) => {
            if response_code != 0 {
                return PortalResponse::Cancelled;
            }
            if let Some(uri_variant) = results.get("uri") {
                if let Some(s) = uri_variant.0.as_str() {
                    return PortalResponse::Success(s.to_string());
                }
            }
            PortalResponse::Failed("No URI in portal response".into())
        }
        Err(e) => PortalResponse::Failed(format!("Failed to parse response: {}", e)),
    }
}

fn decode_screenshot(file_uri: &str) -> Result<CaptureResult, SnagError> {
    // Strip file:// prefix to get filesystem path
    let file_path = file_uri.strip_prefix("file://").unwrap_or(file_uri);

    // Read and decode the image
    let img = image::open(file_path)
        .map_err(|e| SnagError::PortalError(format!("Failed to decode image: {}", e)))?;

    let (width, height) = (img.width(), img.height());
    const MAX_PIXELS: u32 = 50_000_000;
    if width.saturating_mul(height) > MAX_PIXELS {
        return Err(SnagError::PortalError("Portal image too large".into()));
    }

    let rgb_img = img.to_rgb8();
    let pixels = rgb_img.into_raw();

    // Only remove files from temp directories
    let path = std::path::Path::new(file_path);
    let is_temp = path.starts_with("/tmp")
        || path.starts_with(std::env::temp_dir())
        || std::env::var("XDG_RUNTIME_DIR")
            .map(|dir| path.starts_with(&dir))
            .unwrap_or(false);
    if is_temp {
        let _ = std::fs::remove_file(file_path);
    }

    Ok(CaptureResult {
        pixels,
        width,
        height,
        bytes_per_pixel: 3,
    })
}
