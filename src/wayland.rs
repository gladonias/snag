//! Wayland wlroots backend.
//!
//! Uses `wlr_layer_shell_v1` for the overlay surface and
//! `zwlr_screencopy_manager_v1` for pixel capture on wlroots compositors
//! (Hyprland, Sway).

use crate::capture::CaptureResult;
use crate::error::SnagError;
use std::os::unix::io::AsFd;
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_keyboard, wl_output, wl_pointer, wl_registry, wl_seat, wl_shm,
    wl_shm_pool, wl_surface,
};
use wayland_client::{delegate_noop, Connection, Dispatch, QueueHandle, WEnum};
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1, zwlr_layer_surface_v1,
};
use wayland_protocols_wlr::screencopy::v1::client::{
    zwlr_screencopy_frame_v1, zwlr_screencopy_manager_v1,
};

/// Run the wlroots Wayland selection + capture pipeline.
pub fn run() -> Result<CaptureResult, SnagError> {
    let conn =
        Connection::connect_to_env().map_err(|e| SnagError::WaylandError(e.to_string()))?;
    let mut event_queue = conn.new_event_queue::<State>();
    let qh = event_queue.handle();

    let display = conn.display();
    display.get_registry(&qh, ());

    let mut state = State::new();

    // First roundtrip: discover globals
    event_queue
        .roundtrip(&mut state)
        .map_err(|e| SnagError::WaylandError(e.to_string()))?;

    // Verify required protocols
    let compositor = state
        .compositor
        .as_ref()
        .ok_or_else(|| SnagError::WaylandError("wl_compositor not found".into()))?
        .clone();
    let layer_shell = state
        .layer_shell
        .as_ref()
        .ok_or(SnagError::ProtocolUnsupported)?
        .clone();
    let _screencopy_manager = state
        .screencopy_manager
        .as_ref()
        .ok_or(SnagError::ProtocolUnsupported)?
        .clone();
    let shm = state
        .shm
        .as_ref()
        .ok_or_else(|| SnagError::WaylandError("wl_shm not found".into()))?
        .clone();

    if state.outputs.is_empty() {
        return Err(SnagError::WaylandError("No outputs found".into()));
    }

    // Second roundtrip: get output info, seat capabilities
    event_queue
        .roundtrip(&mut state)
        .map_err(|e| SnagError::WaylandError(e.to_string()))?;

    // Use first output dimensions (or fallback)
    let (out_width, out_height) = state
        .output_size
        .unwrap_or((1920, 1080));

    // Create the overlay surface
    let surface = compositor.create_surface(&qh, ());
    let layer_surface = layer_shell.get_layer_surface(
        &surface,
        Some(&state.outputs[0]),
        zwlr_layer_shell_v1::Layer::Overlay,
        "snag".into(),
        &qh,
        (),
    );
    layer_surface.set_size(out_width, out_height);
    layer_surface.set_anchor(
        zwlr_layer_surface_v1::Anchor::Top
            | zwlr_layer_surface_v1::Anchor::Bottom
            | zwlr_layer_surface_v1::Anchor::Left
            | zwlr_layer_surface_v1::Anchor::Right,
    );
    layer_surface.set_exclusive_zone(-1);
    layer_surface.set_keyboard_interactivity(
        zwlr_layer_surface_v1::KeyboardInteractivity::Exclusive,
    );
    surface.commit();

    state.surface = Some(surface.clone());
    state.layer_surface = Some(layer_surface);
    state.shm = Some(shm);
    state.overlay_width = out_width;
    state.overlay_height = out_height;

    // Wait for configure
    while !state.configured {
        event_queue
            .blocking_dispatch(&mut state)
            .map_err(|e| SnagError::WaylandError(e.to_string()))?;
    }

    // Draw the initial overlay
    draw_overlay(&mut state, &qh)?;

    // Event loop: wait for selection or cancel
    while state.running && !state.selection_done {
        event_queue
            .blocking_dispatch(&mut state)
            .map_err(|e| SnagError::WaylandError(e.to_string()))?;

        if state.needs_redraw {
            draw_overlay(&mut state, &qh)?;
            state.needs_redraw = false;
        }
    }

    // Destroy the overlay
    if let Some(layer_surface) = state.layer_surface.take() {
        layer_surface.destroy();
    }
    surface.destroy();

    // Flush to make sure overlay is gone before screencopy
    event_queue
        .flush()
        .map_err(|e| SnagError::WaylandError(e.to_string()))?;

    if state.cancelled {
        return Err(SnagError::SelectionCancelled);
    }

    // Normalize selection coordinates
    let (sel_x, sel_y, sel_w, sel_h) = normalize_selection(&state);
    if sel_w == 0 || sel_h == 0 {
        return Err(SnagError::SelectionCancelled);
    }

    // Use screencopy to capture the selected region
    let screencopy_manager = state
        .screencopy_manager
        .as_ref()
        .ok_or(SnagError::ProtocolUnsupported)?
        .clone();
    let output = state.outputs[0].clone();

    let frame = screencopy_manager.capture_output_region(
        0,
        &output,
        sel_x,
        sel_y,
        sel_w as i32,
        sel_h as i32,
        &qh,
        (),
    );

    state.screencopy_frame = Some(frame);
    state.screencopy_done = false;
    state.screencopy_failed = false;
    state.frame_info = None;

    // Wait for buffer event from screencopy
    while state.frame_info.is_none() && !state.screencopy_failed {
        event_queue
            .blocking_dispatch(&mut state)
            .map_err(|e| SnagError::WaylandError(e.to_string()))?;
    }

    if state.screencopy_failed {
        return Err(SnagError::WaylandError("Screencopy failed".into()));
    }

    let frame_info = state.frame_info.as_ref().ok_or_else(|| {
        SnagError::WaylandError("No frame info received".into())
    })?;

    let buf_width = frame_info.width;
    let buf_height = frame_info.height;
    let buf_stride = frame_info.stride;
    let buf_format = frame_info.format;

    // Create shm buffer for screencopy
    let shm = state
        .shm
        .as_ref()
        .ok_or_else(|| SnagError::WaylandError("wl_shm not found".into()))?
        .clone();

    let buf_size = (buf_stride * buf_height) as usize;
    let file = create_shm_file(buf_size)?;
    let pool = shm.create_pool(file.as_fd(), buf_size as i32, &qh, ());
    let buffer = pool.create_buffer(
        0,
        buf_width as i32,
        buf_height as i32,
        buf_stride as i32,
        match buf_format {
            0 => wl_shm::Format::Argb8888,
            1 => wl_shm::Format::Xrgb8888,
            _ => wl_shm::Format::Xrgb8888,
        },
        &qh,
        (),
    );

    // Copy the frame into our buffer
    if let Some(ref frame) = state.screencopy_frame {
        frame.copy(&buffer);
    }

    event_queue
        .flush()
        .map_err(|e| SnagError::WaylandError(e.to_string()))?;

    // Wait for screencopy ready/failed
    while !state.screencopy_done && !state.screencopy_failed {
        event_queue
            .blocking_dispatch(&mut state)
            .map_err(|e| SnagError::WaylandError(e.to_string()))?;
    }

    if state.screencopy_failed {
        return Err(SnagError::WaylandError("Screencopy copy failed".into()));
    }

    // Read pixels from shm
    let mmap = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            buf_size,
            libc::PROT_READ,
            libc::MAP_SHARED,
            std::os::unix::io::AsRawFd::as_raw_fd(&file),
            0,
        )
    };
    if mmap == libc::MAP_FAILED {
        return Err(SnagError::WaylandError("mmap failed".into()));
    }

    let raw_pixels =
        unsafe { std::slice::from_raw_parts(mmap as *const u8, buf_size) };

    // Convert BGRA/BGRX to RGB for Tesseract
    let pixel_count = (buf_width * buf_height) as usize;
    let mut rgb = Vec::with_capacity(pixel_count * 3);
    for y in 0..buf_height as usize {
        let row_start = y * buf_stride as usize;
        for x in 0..buf_width as usize {
            let offset = row_start + x * 4;
            rgb.push(raw_pixels[offset + 2]); // R
            rgb.push(raw_pixels[offset + 1]); // G
            rgb.push(raw_pixels[offset]);     // B
        }
    }

    unsafe {
        libc::munmap(mmap, buf_size);
    }

    // Clean up screencopy objects
    if let Some(frame) = state.screencopy_frame.take() {
        frame.destroy();
    }
    buffer.destroy();
    pool.destroy();

    Ok(CaptureResult {
        pixels: rgb,
        width: buf_width,
        height: buf_height,
        bytes_per_pixel: 3,
    })
}

fn normalize_selection(state: &State) -> (i32, i32, u32, u32) {
    let x1 = state.sel_start_x;
    let y1 = state.sel_start_y;
    let x2 = state.sel_end_x;
    let y2 = state.sel_end_y;
    let x = x1.min(x2);
    let y = y1.min(y2);
    let w = (x1 - x2).unsigned_abs();
    let h = (y1 - y2).unsigned_abs();
    (x as i32, y as i32, w as u32, h as u32)
}

fn create_shm_file(size: usize) -> Result<std::fs::File, SnagError> {
    use std::io::Seek;
    let mut file = tempfile().map_err(|e| SnagError::IoError(e))?;
    file.set_len(size as u64)
        .map_err(|e| SnagError::IoError(e))?;
    file.seek(std::io::SeekFrom::Start(0))
        .map_err(|e| SnagError::IoError(e))?;
    // Zero-fill is handled by ftruncate on Linux
    Ok(file)
}

fn tempfile() -> Result<std::fs::File, std::io::Error> {
    // Use memfd_create if available, otherwise tmpfile
    let name = std::ffi::CString::new("snag-shm").map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::Other, "CString error")
    })?;
    let fd = unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { std::os::unix::io::FromRawFd::from_raw_fd(fd) })
}

fn draw_overlay(state: &mut State, qh: &QueueHandle<State>) -> Result<(), SnagError> {
    let width = state.overlay_width;
    let height = state.overlay_height;
    let stride = width * 4;
    let size = (stride * height) as usize;

    let file = create_shm_file(size)?;
    let shm = state
        .shm
        .as_ref()
        .ok_or_else(|| SnagError::WaylandError("wl_shm not found".into()))?
        .clone();

    // Map the file for writing
    let mmap = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            size,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            std::os::unix::io::AsRawFd::as_raw_fd(&file),
            0,
        )
    };
    if mmap == libc::MAP_FAILED {
        return Err(SnagError::WaylandError("mmap failed".into()));
    }

    let pixels =
        unsafe { std::slice::from_raw_parts_mut(mmap as *mut u8, size) };

    // Fill with semi-transparent dark overlay (ARGB8888)
    let overlay_color: [u8; 4] = [0x00, 0x00, 0x00, 0x60]; // B G R A
    for chunk in pixels.chunks_exact_mut(4) {
        chunk.copy_from_slice(&overlay_color);
    }

    // Draw selection rectangle if dragging
    if state.dragging {
        let (sx, sy, sw, sh) = normalize_selection(state);
        let sx = sx.max(0) as u32;
        let sy = sy.max(0) as u32;
        let sw = sw.min(width.saturating_sub(sx));
        let sh = sh.min(height.saturating_sub(sy));

        // Clear the selected area (make it transparent to show through)
        for y in sy..sy.saturating_add(sh).min(height) {
            for x in sx..sx.saturating_add(sw).min(width) {
                let offset = ((y * stride) + x * 4) as usize;
                if offset + 3 < size {
                    pixels[offset] = 0x00;     // B
                    pixels[offset + 1] = 0x00; // G
                    pixels[offset + 2] = 0x00; // R
                    pixels[offset + 3] = 0x10; // A (nearly transparent)
                }
            }
        }

        // Draw border (2px white with some alpha)
        let border_color: [u8; 4] = [0xFF, 0xFF, 0xFF, 0xCC]; // B G R A
        let border_width = 2u32;
        for b in 0..border_width {
            // Top and bottom edges
            for x in sx..sx.saturating_add(sw).min(width) {
                let ty = sy.saturating_add(b);
                let by = sy.saturating_add(sh).saturating_sub(1).saturating_sub(b);
                if ty < height {
                    let offset = ((ty * stride) + x * 4) as usize;
                    if offset + 3 < size {
                        pixels[offset..offset + 4].copy_from_slice(&border_color);
                    }
                }
                if by < height && by >= sy {
                    let offset = ((by * stride) + x * 4) as usize;
                    if offset + 3 < size {
                        pixels[offset..offset + 4].copy_from_slice(&border_color);
                    }
                }
            }
            // Left and right edges
            for y in sy..sy.saturating_add(sh).min(height) {
                let lx = sx.saturating_add(b);
                let rx = sx.saturating_add(sw).saturating_sub(1).saturating_sub(b);
                if lx < width {
                    let offset = ((y * stride) + lx * 4) as usize;
                    if offset + 3 < size {
                        pixels[offset..offset + 4].copy_from_slice(&border_color);
                    }
                }
                if rx < width && rx >= sx {
                    let offset = ((y * stride) + rx * 4) as usize;
                    if offset + 3 < size {
                        pixels[offset..offset + 4].copy_from_slice(&border_color);
                    }
                }
            }
        }
    }

    unsafe {
        libc::munmap(mmap, size);
    }

    let pool = shm.create_pool(file.as_fd(), size as i32, qh, ());
    let buffer = pool.create_buffer(
        0,
        width as i32,
        height as i32,
        stride as i32,
        wl_shm::Format::Argb8888,
        qh,
        (),
    );

    if let Some(ref surface) = state.surface {
        surface.attach(Some(&buffer), 0, 0);
        surface.damage_buffer(0, 0, width as i32, height as i32);
        surface.commit();
    }

    // Store for cleanup (buffer/pool need to stay alive until next attach)
    state.current_buffer = Some(buffer);
    state.current_pool = Some(pool);

    Ok(())
}

// -- State --

struct FrameInfo {
    format: u32,
    width: u32,
    height: u32,
    stride: u32,
}

struct State {
    // Globals
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    layer_shell: Option<zwlr_layer_shell_v1::ZwlrLayerShellV1>,
    screencopy_manager: Option<zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1>,
    outputs: Vec<wl_output::WlOutput>,
    output_size: Option<(u32, u32)>,

    // Overlay state
    surface: Option<wl_surface::WlSurface>,
    layer_surface: Option<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1>,
    configured: bool,
    overlay_width: u32,
    overlay_height: u32,
    current_buffer: Option<wl_buffer::WlBuffer>,
    current_pool: Option<wl_shm_pool::WlShmPool>,

    // Selection state
    running: bool,
    selection_done: bool,
    cancelled: bool,
    dragging: bool,
    needs_redraw: bool,
    sel_start_x: i32,
    sel_start_y: i32,
    sel_end_x: i32,
    sel_end_y: i32,

    // Screencopy state
    screencopy_frame: Option<zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1>,
    screencopy_done: bool,
    screencopy_failed: bool,
    frame_info: Option<FrameInfo>,
}

impl State {
    fn new() -> Self {
        Self {
            compositor: None,
            shm: None,
            layer_shell: None,
            screencopy_manager: None,
            outputs: Vec::new(),
            output_size: None,
            surface: None,
            layer_surface: None,
            configured: false,
            overlay_width: 0,
            overlay_height: 0,
            current_buffer: None,
            current_pool: None,
            running: true,
            selection_done: false,
            cancelled: false,
            dragging: false,
            needs_redraw: false,
            sel_start_x: 0,
            sel_start_y: 0,
            sel_end_x: 0,
            sel_end_y: 0,
            screencopy_frame: None,
            screencopy_done: false,
            screencopy_failed: false,
            frame_info: None,
        }
    }
}

// -- Dispatch implementations --

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name, interface, version,
        } = event
        {
            match &interface[..] {
                "wl_compositor" => {
                    state.compositor =
                        Some(registry.bind::<wl_compositor::WlCompositor, _, _>(name, version.min(4), qh, ()));
                }
                "wl_shm" => {
                    state.shm =
                        Some(registry.bind::<wl_shm::WlShm, _, _>(name, version.min(1), qh, ()));
                }
                "wl_output" => {
                    let output =
                        registry.bind::<wl_output::WlOutput, _, _>(name, version.min(3), qh, ());
                    state.outputs.push(output);
                }
                "wl_seat" => {
                    registry.bind::<wl_seat::WlSeat, _, _>(name, version.min(5), qh, ());
                }
                "zwlr_layer_shell_v1" => {
                    state.layer_shell = Some(
                        registry.bind::<zwlr_layer_shell_v1::ZwlrLayerShellV1, _, _>(
                            name,
                            version.min(4),
                            qh,
                            (),
                        ),
                    );
                }
                "zwlr_screencopy_manager_v1" => {
                    state.screencopy_manager = Some(
                        registry
                            .bind::<zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1, _, _>(
                                name,
                                version.min(3),
                                qh,
                                (),
                            ),
                    );
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<wl_output::WlOutput, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_output::WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Mode { width, height, flags, .. } = event {
            if let WEnum::Value(flags) = flags {
                if flags.contains(wl_output::Mode::Current) {
                    state.output_size = Some((width as u32, height as u32));
                }
            }
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(
        _: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(capabilities),
        } = event
        {
            if capabilities.contains(wl_seat::Capability::Pointer) {
                seat.get_pointer(qh, ());
            }
            if capabilities.contains(wl_seat::Capability::Keyboard) {
                seat.get_keyboard(qh, ());
            }
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_pointer::Event::Button {
                button,
                state: btn_state,
                ..
            } => {
                let pressed = btn_state == WEnum::Value(wl_pointer::ButtonState::Pressed);
                match button {
                    // BTN_LEFT = 0x110
                    0x110 => {
                        if pressed {
                            state.dragging = true;
                            state.sel_start_x = state.sel_end_x;
                            state.sel_start_y = state.sel_end_y;
                        } else if state.dragging {
                            state.dragging = false;
                            state.selection_done = true;
                        }
                    }
                    // BTN_RIGHT = 0x111, BTN_MIDDLE = 0x112 — cancel
                    _ => {
                        if pressed {
                            state.cancelled = true;
                            state.running = false;
                        }
                    }
                }
            }
            wl_pointer::Event::Motion {
                surface_x,
                surface_y,
                ..
            } => {
                let x = surface_x as i32;
                let y = surface_y as i32;
                state.sel_end_x = x;
                state.sel_end_y = y;
                if state.dragging {
                    state.needs_redraw = true;
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_keyboard::Event::Key { key, .. } = event {
            // Escape key (evdev keycode 1)
            if key == 1 {
                state.cancelled = true;
                state.running = false;
            }
        }
    }
}

impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, ()> for State {
    fn event(
        state: &mut Self,
        layer_surface: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwlr_layer_surface_v1::Event::Configure {
            serial,
            width,
            height,
        } = event
        {
            layer_surface.ack_configure(serial);
            if width > 0 {
                state.overlay_width = width;
            }
            if height > 0 {
                state.overlay_height = height;
            }
            state.configured = true;
        }
    }
}

impl Dispatch<zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1,
        event: zwlr_screencopy_frame_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_screencopy_frame_v1::Event::Buffer {
                format,
                width,
                height,
                stride,
            } => {
                if let WEnum::Value(fmt) = format {
                    state.frame_info = Some(FrameInfo {
                        format: fmt as u32,
                        width,
                        height,
                        stride,
                    });
                }
            }
            zwlr_screencopy_frame_v1::Event::Ready { .. } => {
                state.screencopy_done = true;
            }
            zwlr_screencopy_frame_v1::Event::Failed => {
                state.screencopy_failed = true;
            }
            _ => {}
        }
    }
}

// Noop dispatch for types we don't handle events on
delegate_noop!(State: ignore wl_compositor::WlCompositor);
delegate_noop!(State: ignore wl_surface::WlSurface);
delegate_noop!(State: ignore wl_shm::WlShm);
delegate_noop!(State: ignore wl_shm_pool::WlShmPool);
delegate_noop!(State: ignore wl_buffer::WlBuffer);
delegate_noop!(State: ignore zwlr_layer_shell_v1::ZwlrLayerShellV1);
delegate_noop!(State: ignore zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1);
