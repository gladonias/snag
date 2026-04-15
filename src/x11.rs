//! X11 backend.
//!
//! Creates a fullscreen overlay window for selection, captures the
//! selected region via `XGetImage` (ZPixmap format).

use crate::capture::CaptureResult;
use crate::error::SnagError;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;
use x11rb::{COPY_DEPTH_FROM_PARENT, CURRENT_TIME};

/// Crosshair cursor glyph index in the X11 "cursor" font.
const XC_CROSSHAIR: u16 = 34;

/// Run the X11 selection + capture pipeline.
pub fn run() -> Result<CaptureResult, SnagError> {
    let (conn, screen_num) =
        RustConnection::connect(None).map_err(|e| SnagError::X11Error(e.to_string()))?;

    let screen = &conn.setup().roots[screen_num];
    let root = screen.root;
    let root_visual = screen.root_visual;
    let screen_width = screen.width_in_pixels;
    let screen_height = screen.height_in_pixels;

    // Create crosshair cursor from the "cursor" font
    let cursor = create_crosshair_cursor(&conn)?;

    // Create a fullscreen, transparent, override-redirect input-only window
    let win = conn
        .generate_id()
        .map_err(|e| SnagError::X11Error(e.to_string()))?;
    let win_aux = CreateWindowAux::new()
        .override_redirect(1u32)
        .event_mask(
            EventMask::BUTTON_PRESS
                | EventMask::BUTTON_RELEASE
                | EventMask::POINTER_MOTION
                | EventMask::KEY_PRESS
                | EventMask::EXPOSURE,
        )
        .cursor(cursor);

    conn.create_window(
        COPY_DEPTH_FROM_PARENT,
        win,
        root,
        0,
        0,
        screen_width,
        screen_height,
        0,
        WindowClass::INPUT_ONLY,
        root_visual,
        &win_aux,
    )
    .map_err(|e| SnagError::X11Error(e.to_string()))?;

    conn.map_window(win)
        .map_err(|e| SnagError::X11Error(e.to_string()))?;

    // Create a GC for XOR rectangle drawing on the root window
    let gc = conn
        .generate_id()
        .map_err(|e| SnagError::X11Error(e.to_string()))?;
    let gc_aux = CreateGCAux::new()
        .function(GX::XOR)
        .foreground(0xFFFFFF)
        .subwindow_mode(SubwindowMode::INCLUDE_INFERIORS)
        .line_width(2);

    conn.create_gc(gc, root, &gc_aux)
        .map_err(|e| SnagError::X11Error(e.to_string()))?;

    // Grab the pointer
    conn.grab_pointer(
        true,
        win,
        EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE | EventMask::POINTER_MOTION,
        GrabMode::ASYNC,
        GrabMode::ASYNC,
        win,
        cursor,
        CURRENT_TIME,
    )
    .map_err(|e| SnagError::X11Error(e.to_string()))?
    .reply()
    .map_err(|e| SnagError::X11Error(e.to_string()))?;

    conn.flush()
        .map_err(|e| SnagError::X11Error(e.to_string()))?;

    // Event loop for selection
    let selection = run_selection_loop(&conn, root, gc, win)?;

    // Clean up
    conn.ungrab_pointer(CURRENT_TIME)
        .map_err(|e| SnagError::X11Error(e.to_string()))?;
    conn.free_gc(gc)
        .map_err(|e| SnagError::X11Error(e.to_string()))?;
    conn.destroy_window(win)
        .map_err(|e| SnagError::X11Error(e.to_string()))?;
    conn.free_cursor(cursor)
        .map_err(|e| SnagError::X11Error(e.to_string()))?;
    conn.flush()
        .map_err(|e| SnagError::X11Error(e.to_string()))?;

    let (x, y, w, h) = selection;
    if w == 0 || h == 0 {
        return Err(SnagError::SelectionCancelled);
    }

    // Capture the selected region from the root window
    capture_region(&conn, root, x, y, w, h)
}

/// Selection rectangle coordinates (x, y, width, height).
type SelectionRect = (i16, i16, u16, u16);

fn run_selection_loop(
    conn: &RustConnection,
    root: Window,
    gc: Gcontext,
    _win: Window,
) -> Result<SelectionRect, SnagError> {
    let mut start_x: i16 = 0;
    let mut start_y: i16 = 0;
    let mut dragging = false;
    let mut last_rect: Option<(i16, i16, u16, u16)> = None;

    loop {
        let event = conn
            .wait_for_event()
            .map_err(|e| SnagError::X11Error(e.to_string()))?;

        match event {
            Event::ButtonPress(ev) => {
                // Right-click or middle-click cancels
                if ev.detail != 1 {
                    // Erase any drawn rectangle
                    if let Some((rx, ry, rw, rh)) = last_rect {
                        draw_rect(conn, root, gc, rx, ry, rw, rh)?;
                    }
                    return Err(SnagError::SelectionCancelled);
                }
                start_x = ev.root_x;
                start_y = ev.root_y;
                dragging = true;
            }
            Event::MotionNotify(ev) => {
                if !dragging {
                    continue;
                }
                // Erase the old rectangle
                if let Some((rx, ry, rw, rh)) = last_rect {
                    draw_rect(conn, root, gc, rx, ry, rw, rh)?;
                }
                // Draw the new rectangle
                let (rx, ry, rw, rh) = normalize_rect(start_x, start_y, ev.root_x, ev.root_y);
                if rw > 0 && rh > 0 {
                    draw_rect(conn, root, gc, rx, ry, rw, rh)?;
                    last_rect = Some((rx, ry, rw, rh));
                } else {
                    last_rect = None;
                }
                conn.flush()
                    .map_err(|e| SnagError::X11Error(e.to_string()))?;
            }
            Event::ButtonRelease(ev) => {
                if ev.detail != 1 {
                    continue;
                }
                // Erase the final rectangle
                if let Some((rx, ry, rw, rh)) = last_rect {
                    draw_rect(conn, root, gc, rx, ry, rw, rh)?;
                    conn.flush()
                        .map_err(|e| SnagError::X11Error(e.to_string()))?;
                }
                let rect = normalize_rect(start_x, start_y, ev.root_x, ev.root_y);
                return Ok(rect);
            }
            Event::KeyPress(ev) => {
                // Escape key (keycode 9 on most X11 systems)
                if ev.detail == 9 {
                    // Erase any drawn rectangle
                    if let Some((rx, ry, rw, rh)) = last_rect {
                        draw_rect(conn, root, gc, rx, ry, rw, rh)?;
                        conn.flush()
                            .map_err(|e| SnagError::X11Error(e.to_string()))?;
                    }
                    return Err(SnagError::SelectionCancelled);
                }
            }
            _ => {}
        }
    }
}

/// Normalize two corner points into (x, y, width, height).
fn normalize_rect(x1: i16, y1: i16, x2: i16, y2: i16) -> (i16, i16, u16, u16) {
    let x = x1.min(x2);
    let y = y1.min(y2);
    let w = (x1 - x2).unsigned_abs();
    let h = (y1 - y2).unsigned_abs();
    (x, y, w, h)
}

/// Draw (or erase, via XOR) a rectangle on the given drawable.
fn draw_rect(
    conn: &RustConnection,
    drawable: Drawable,
    gc: Gcontext,
    x: i16,
    y: i16,
    w: u16,
    h: u16,
) -> Result<(), SnagError> {
    let rect = Rectangle {
        x,
        y,
        width: w,
        height: h,
    };
    conn.poly_rectangle(drawable, gc, &[rect])
        .map_err(|e| SnagError::X11Error(e.to_string()))?;
    Ok(())
}

/// Create a crosshair cursor from the X11 "cursor" font.
fn create_crosshair_cursor(conn: &RustConnection) -> Result<Cursor, SnagError> {
    let font = conn
        .generate_id()
        .map_err(|e| SnagError::X11Error(e.to_string()))?;
    conn.open_font(font, b"cursor")
        .map_err(|e| SnagError::X11Error(e.to_string()))?;

    let cursor = conn
        .generate_id()
        .map_err(|e| SnagError::X11Error(e.to_string()))?;
    conn.create_glyph_cursor(
        cursor,
        font,
        font,
        XC_CROSSHAIR,
        XC_CROSSHAIR + 1,
        // Foreground: white
        0xFFFF,
        0xFFFF,
        0xFFFF,
        // Background: black
        0,
        0,
        0,
    )
    .map_err(|e| SnagError::X11Error(e.to_string()))?;

    conn.close_font(font)
        .map_err(|e| SnagError::X11Error(e.to_string()))?;

    Ok(cursor)
}

/// Capture a region of the root window as raw pixel data.
fn capture_region(
    conn: &RustConnection,
    root: Window,
    x: i16,
    y: i16,
    w: u16,
    h: u16,
) -> Result<CaptureResult, SnagError> {
    let reply = conn
        .get_image(ImageFormat::Z_PIXMAP, root, x, y, w, h, u32::MAX)
        .map_err(|e| SnagError::X11Error(e.to_string()))?
        .reply()
        .map_err(|e| SnagError::X11Error(e.to_string()))?;

    // X11 ZPixmap typically returns pixels as BGRA (32-bit depth).
    // Convert BGRA to RGB for Tesseract.
    let depth = reply.depth;
    let pixels = &reply.data;

    if depth == 24 || depth == 32 {
        // 4 bytes per pixel: B G R A (or B G R X)
        let mut rgb = Vec::with_capacity((w as usize) * (h as usize) * 3);
        for chunk in pixels.chunks_exact(4) {
            rgb.push(chunk[2]); // R
            rgb.push(chunk[1]); // G
            rgb.push(chunk[0]); // B
        }
        Ok(CaptureResult {
            pixels: rgb,
            width: w as u32,
            height: h as u32,
            bytes_per_pixel: 3,
        })
    } else {
        Err(SnagError::X11Error(format!(
            "Unsupported screen depth: {}",
            depth
        )))
    }
}
