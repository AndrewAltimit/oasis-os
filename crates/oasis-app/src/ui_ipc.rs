//! Host side of the terminal's UI request files.
//!
//! The `notify`, `screenshot`, `theme` and `browse` terminal commands
//! (`oasis-terminal`'s `ui_commands`, `oasis-core`'s `browser_commands`)
//! only talk to the VFS: they queue a request file or read a status file.
//! [`poll`] runs once per `Shell::step` (next to the `wm` IPC) and makes
//! them do what they say:
//!
//! | File | Written by | Host action |
//! |---|---|---|
//! | `/var/notify/message` (`level:message`) | `notify` | show a toast |
//! | `/var/screenshot/request` (VFS path) | `screenshot` | capture the next presented frame to that path (PNG for `.png`, else BMP) |
//! | `/var/theme/current` | host | status the `theme` command prints |
//! | `/var/browser/request` (`op [arg]`) | `browse` | open the browser, navigate / back / forward / reload / home / reader / bookmarks / history |

use oasis_core::backend::Color;
use oasis_core::dashboard::AppEntry;
use oasis_core::sdi::SdiRegistry;
use oasis_core::toast::ToastLevel;
use oasis_core::vfs::{MemoryVfs, Vfs};

use crate::app_state::AppState;
use crate::launch;

pub const NOTIFY_PATH: &str = "/var/notify/message";
pub const SCREENSHOT_REQUEST_PATH: &str = "/var/screenshot/request";
pub const THEME_STATUS_PATH: &str = "/var/theme/current";
pub use oasis_core::terminal::BROWSER_REQUEST_PATH;

/// Service every UI request file. Returns the VFS path of a requested
/// screenshot: the caller captures the next presented frame there
/// ([`save_screenshot`]).
pub fn poll(state: &mut AppState, sdi: &mut SdiRegistry, vfs: &mut MemoryVfs) -> Option<String> {
    for dir in [
        "/var/notify",
        "/var/screenshot",
        "/var/theme",
        "/var/browser",
    ] {
        if !vfs.exists(dir) && vfs.mkdir(dir).is_err() {
            return None;
        }
    }
    if let Some(msg) = take_request(vfs, NOTIFY_PATH) {
        show_notification(state, &msg);
    }
    if let Some(req) = take_request(vfs, BROWSER_REQUEST_PATH) {
        apply_browse_request(&req, state, sdi, vfs);
    }
    publish_theme(state, vfs);
    take_request(vfs, SCREENSHOT_REQUEST_PATH)
}

/// Read and clear a request file. `None` when it is missing or empty.
fn take_request(vfs: &mut MemoryVfs, path: &str) -> Option<String> {
    let data = vfs.read(path).ok().filter(|d| !d.is_empty())?;
    let _ = vfs.write(path, b"");
    let text = String::from_utf8_lossy(&data).trim().to_string();
    (!text.is_empty()).then_some(text)
}

/// `level:message` (as `notify` writes it) to a toast.
fn show_notification(state: &mut AppState, payload: &str) {
    let (level, message) = match payload.split_once(':') {
        Some(("info", m)) => (ToastLevel::Info, m),
        Some(("success", m)) => (ToastLevel::Success, m),
        Some(("warning", m)) => (ToastLevel::Warning, m),
        Some(("error", m)) => (ToastLevel::Error, m),
        _ => (ToastLevel::Info, payload),
    };
    let ttl = state.active_theme.toast.ttl;
    state.toasts.show(message.trim(), level, ttl);
}

/// Keep `/var/theme/current` describing the active skin (only rewritten
/// when it changes).
fn publish_theme(state: &AppState, vfs: &mut MemoryVfs) {
    let t = &state.skin.theme;
    let status = format!(
        "skin: {}\nresolution: {}x{}\nbackground: {}\nprimary: {}\nsecondary: {}\n\
         text: {}\ndim_text: {}\nstatus_bar: {}\nprompt: {}\noutput: {}\nerror: {}",
        state.skin.manifest.name,
        state.active_theme.screen_w,
        state.active_theme.screen_h,
        t.background,
        t.primary,
        t.secondary,
        t.text,
        t.dim_text,
        t.status_bar,
        t.prompt,
        t.output,
        t.error,
    );
    if vfs.read(THEME_STATUS_PATH).ok().as_deref() != Some(status.as_bytes()) {
        let _ = vfs.write(THEME_STATUS_PATH, status.as_bytes());
    }
}

/// Open the browser window if needed, focus it and apply `request`.
fn apply_browse_request(
    request: &str,
    state: &mut AppState,
    sdi: &mut SdiRegistry,
    vfs: &mut MemoryVfs,
) {
    let (op, arg) = match request.split_once(' ') {
        Some((op, arg)) => (op, arg.trim()),
        None => (request, ""),
    };
    let entry = AppEntry {
        title: "Browser".to_string(),
        path: "/apps/Browser".to_string(),
        icon_png: Vec::new(),
        color: Color::rgb(100, 100, 100),
    };
    // Opens the window (at the home page) or focuses the existing one.
    let result = launch::launch_app_window(
        &entry,
        &mut state.wm,
        sdi,
        &mut state.content.open_runners,
        &mut state.content.browser,
        &state.browser_config,
        vfs,
        &state.net.tls_provider,
        state.skin.features.window_manager,
        &state.plugin_manager,
    );
    launch::apply_launch(result, &mut state.mode);
    let Some(bw) = state.content.browser.as_mut() else {
        return;
    };
    match op {
        "open" if !arg.is_empty() => bw.navigate_vfs(arg, vfs),
        "sandbox" if !arg.is_empty() => {
            bw.config.features.sandbox_only = true;
            bw.navigate_vfs(arg, vfs);
        },
        "back" => bw.go_back(vfs),
        "forward" => bw.go_forward(vfs),
        "reload" => bw.reload(vfs),
        "home" => bw.go_home(vfs),
        "reader" => bw.toggle_reader_mode(),
        "bookmarks" => bw.navigate_vfs("vfs://bookmarks", vfs),
        "history" => bw.navigate_vfs("vfs://history", vfs),
        other => log::warn!("unknown browse request {other:?}"),
    }
}

/// Encode `rgba` (`w`x`h`, RGBA8) for `path`: PNG when it ends in
/// `.png`, else a 24-bit BMP.
pub fn encode_screenshot(path: &str, rgba: &[u8], w: u32, h: u32) -> Result<Vec<u8>, String> {
    if path.to_ascii_lowercase().ends_with(".png") {
        let mut opaque = rgba.to_vec();
        for px in opaque.as_chunks_mut::<4>().0 {
            px[3] = 255;
        }
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, w, h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        writer
            .write_image_data(&opaque)
            .map_err(|e| e.to_string())?;
        drop(writer);
        return Ok(out);
    }
    Ok(encode_bmp(rgba, w, h))
}

/// Uncompressed 24-bit bottom-up BMP.
fn encode_bmp(rgba: &[u8], w: u32, h: u32) -> Vec<u8> {
    let row = (w as usize * 3).div_ceil(4) * 4;
    let data_len = row * h as usize;
    let file_len = 54 + data_len;
    let mut out = Vec::with_capacity(file_len);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(file_len as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&(h as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&24u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&(data_len as u32).to_le_bytes());
    out.extend_from_slice(&2835u32.to_le_bytes());
    out.extend_from_slice(&2835u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    for y in (0..h as usize).rev() {
        let start = out.len();
        for x in 0..w as usize {
            let i = (y * w as usize + x) * 4;
            let px = rgba.get(i..i + 4).unwrap_or(&[0, 0, 0, 0]);
            out.extend_from_slice(&[px[2], px[1], px[0]]);
        }
        out.resize(start + row, 0);
    }
    out
}

/// Encode the captured frame and write it to `path` in the VFS; the
/// result line is printed to the terminal.
pub fn save_screenshot(
    state: &mut AppState,
    vfs: &mut MemoryVfs,
    path: &str,
    rgba: oasis_core::error::Result<Vec<u8>>,
    w: u32,
    h: u32,
) {
    let line = match rgba
        .map_err(|e| e.to_string())
        .and_then(|px| encode_screenshot(path, &px, w, h))
        .and_then(|bytes| vfs.write(path, &bytes).map_err(|e| e.to_string()))
    {
        Ok(()) => format!("Screenshot saved: {path} ({w}x{h})"),
        Err(e) => format!("screenshot: {path}: {e}"),
    };
    state.terminal.output_lines.push(line);
    state.terminal.dirty = true;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bmp_header_and_pixel_order() {
        // 2x1: red, green.
        let rgba = [255, 0, 0, 255, 0, 255, 0, 255];
        let bmp = encode_bmp(&rgba, 2, 1);
        assert_eq!(&bmp[..2], b"BM");
        assert_eq!(bmp.len(), 54 + 8); // 6 bytes padded to 8
        assert_eq!(&bmp[54..60], &[0, 0, 255, 0, 255, 0]); // BGR
    }

    #[test]
    fn png_is_chosen_by_extension() {
        let rgba = [1, 2, 3, 4];
        let png = encode_screenshot("/tmp/a.PNG", &rgba, 1, 1).expect("png");
        assert_eq!(&png[1..4], b"PNG");
        let bmp = encode_screenshot("/tmp/a.bmp", &rgba, 1, 1).expect("bmp");
        assert_eq!(&bmp[..2], b"BM");
    }
}
