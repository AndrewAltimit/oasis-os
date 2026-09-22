//! Shared helpers for the SDL parity suites.
//!
//! SDL can only be initialized from one thread at a time, and `cargo test`
//! runs every `#[test]` on its own thread. [`with_sdl`] serializes the SDL
//! tests of one binary behind a process-wide lock and tears the backend
//! down before releasing it.
//!
//! By default the backend is fully headless: the `offscreen` video driver
//! and the `software` renderer, forced in-process through SDL hints, so the
//! suites run in the CI Docker image and on a local Windows box alike. Set
//! `OASIS_PARITY_RENDER_DRIVER` (e.g. `direct3d11`, `opengl`, `vulkan`) to
//! run the same suites against a hardware renderer on a hidden window —
//! the only way to exercise padded texture pitches, which the software
//! renderer never produces.

#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

use oasis_backend_sdl::SdlBackend;
use oasis_test_backend::conformance::Frame;

static SDL_LOCK: Mutex<()> = Mutex::new(());

/// Renderer requested through `OASIS_PARITY_RENDER_DRIVER` (`None` =
/// headless software).
pub fn render_driver() -> Option<String> {
    std::env::var("OASIS_PARITY_RENDER_DRIVER")
        .ok()
        .filter(|s| !s.is_empty())
}

/// Run `f` with a fresh `w` x `h` SDL backend (see module docs).
pub fn with_sdl<R>(w: u32, h: u32, f: impl FnOnce(&mut SdlBackend) -> R) -> R {
    let _guard = SDL_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    let driver = render_driver();
    let mut backend = SdlBackend::new_headless(w, h, driver.as_deref())
        .unwrap_or_else(|e| panic!("headless SDL backend ({driver:?}) failed: {e}"));
    let expected = driver.as_deref().unwrap_or("software");
    assert_eq!(
        backend.renderer_name(),
        expected,
        "SDL picked a different renderer than requested"
    );
    let r = f(&mut backend);
    drop(backend);
    r
}

/// Directory failure dumps are written to.
pub fn dump_dir(suite: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(suite);
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Write `frame` as a PNG (best effort; failure dumps only).
pub fn save_png(path: &std::path::Path, frame: &Frame) {
    let Ok(file) = std::fs::File::create(path) else {
        return;
    };
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), frame.w, frame.h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    // Force opaque alpha: backends disagree on the alpha of an opaque
    // window surface and a transparent dump is unreadable.
    let mut rgba = frame.rgba.clone();
    for px in rgba.as_chunks_mut::<4>().0.iter_mut() {
        px[3] = 255;
    }
    if let Ok(mut w) = enc.write_header() {
        let _ = w.write_image_data(&rgba);
    }
}

/// Magnified per-pixel difference image (red where the frames differ by
/// more than `channel`, dimmed `a` elsewhere).
pub fn diff_frame(a: &Frame, b: &Frame, channel: u8) -> Frame {
    let mut rgba = Vec::with_capacity(a.rgba.len());
    for (pa, pb) in a
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .zip(b.rgba.as_chunks::<4>().0.iter())
    {
        let delta = (0..3).map(|c| pa[c].abs_diff(pb[c])).max().unwrap_or(0);
        if delta > channel {
            rgba.extend_from_slice(&[255, 0, 0, 255]);
        } else {
            rgba.extend_from_slice(&[pa[0] / 4, pa[1] / 4, pa[2] / 4, 255]);
        }
    }
    Frame {
        w: a.w,
        h: a.h,
        rgba,
    }
}
