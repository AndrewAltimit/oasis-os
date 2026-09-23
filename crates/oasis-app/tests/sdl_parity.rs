//! The real boot splash and shell frames, rendered through the SDL
//! backend (headless) and through the software framebuffer the e2e
//! harness uses, must look the same.
//!
//! The shell harness (`oasis_app::harness`) can only catch bugs above the
//! `SdiBackend` boundary because it renders with the software rasterizer.
//! This suite closes the gap for the frames users see first: every splash
//! phase (BIOS screen, logo reveal with its vignette / glow textures,
//! fade-out) and settled dashboards, drawn by the same code on both
//! backends and compared pixel by pixel.
//!
//! Alpha blending rounds differently by a level or two (SDL's blitters
//! truncate, the rasterizer rounds), so each comparison allows a per-channel
//! delta of 3 and a tiny budget of pixels beyond it. A structural bug
//! (sheared texture, missing layer, wrong text size, double-darkened
//! translucency, square instead of rounded corners) blows far past both.
//!
//! The SDL backend runs headless (offscreen video driver, software
//! renderer, forced in-process); set `OASIS_PARITY_RENDER_DRIVER` (e.g.
//! `direct3d11`) to compare a hardware renderer instead. The primitive-level
//! conformance suite lives in `oasis-backend-sdl/tests/backend_parity.rs`.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::Instant;

use oasis_app::boot_splash::{BootSplash, SPLASH_DURATION_S, SplashTheme};
use oasis_app::harness::{FIXED_TIME, FRAME, RecordingAudio};
use oasis_app::headless::HeadlessBackend;
use oasis_app::{BootOptions, NoSplash, Shell, ShellBackend};
use oasis_backend_sdl::SdlBackend;
use oasis_backend_ue5::Ue5Backend;
use oasis_core::backend::SdiBackend;
use oasis_test_backend::conformance::{Diff, Frame, Tolerance};

/// SDL may only be initialized from one thread at a time; `cargo test`
/// runs each test on its own thread.
static SDL_LOCK: Mutex<()> = Mutex::new(());

/// Run `f` with a fresh headless `w` x `h` SDL backend (see module docs).
/// Everything holding SDL resources must be dropped inside `f`.
fn with_sdl<R>(w: u32, h: u32, f: impl FnOnce(SdlBackend) -> R) -> R {
    let _guard = SDL_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    let driver = std::env::var("OASIS_PARITY_RENDER_DRIVER")
        .ok()
        .filter(|s| !s.is_empty());
    let backend = SdlBackend::new_headless(w, h, driver.as_deref())
        .unwrap_or_else(|e| panic!("headless SDL backend ({driver:?}) failed: {e}"));
    f(backend)
}

fn dump_dir() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("sdl-parity");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Write `frame` as an opaque PNG (best effort; failure dumps only).
fn save_png(path: &Path, frame: &Frame) {
    let Ok(file) = std::fs::File::create(path) else {
        return;
    };
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), frame.w, frame.h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut rgba = frame.rgba.clone();
    for px in rgba.as_chunks_mut::<4>().0.iter_mut() {
        px[3] = 255;
    }
    if let Ok(mut w) = enc.write_header() {
        let _ = w.write_image_data(&rgba);
    }
}

/// Red where the frames differ by more than `channel`, dimmed `a` elsewhere.
fn diff_frame(a: &Frame, b: &Frame, channel: u8) -> Frame {
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

/// Allowed difference between an SDL frame and the software frame.
#[derive(Clone, Copy, Debug)]
struct Budget {
    /// Per-channel delta that counts as equal (blend rounding).
    channel: u8,
    /// Pixels allowed beyond `channel`. Kept tiny: one mis-shaped rounded
    /// corner on a 1280x720 frame is already a few hundred pixels.
    max_pixels: usize,
}

fn capture(b: &dyn SdiBackend, w: u32, h: u32) -> Frame {
    Frame::capture(b, w, h).expect("read_pixels")
}

/// Number of distinct RGB colors in `f` (guards against comparing two
/// blank frames).
fn distinct_colors(f: &Frame) -> usize {
    let mut seen = std::collections::HashSet::new();
    for px in f.rgba.as_chunks::<4>().0.iter() {
        seen.insert([px[0], px[1], px[2]]);
    }
    seen.len()
}

/// Compare, dumping PNGs on failure. Returns an error line or `None`.
fn compare(name: &str, sdl: &Frame, soft: &Frame, budget: Budget) -> Option<String> {
    let colors = distinct_colors(soft);
    assert!(
        colors >= 4,
        "{name}: frame is nearly blank ({colors} colors)"
    );
    if std::env::var_os("OASIS_PARITY_DUMP").is_some() {
        save_png(&dump_dir().join(format!("{name}-sdl.png")), sdl);
        save_png(&dump_dir().join(format!("{name}-soft.png")), soft);
    }
    let tol = Tolerance {
        channel: budget.channel,
        pixels: 0,
    };
    let d: Diff = sdl.diff(soft, tol);
    eprintln!(
        "{name}: {} px beyond {} (max delta {})",
        d.mismatched, budget.channel, d.max_delta
    );
    if d.mismatched <= budget.max_pixels {
        return None;
    }
    let dir = dump_dir();
    save_png(&dir.join(format!("{name}-sdl.png")), sdl);
    save_png(&dir.join(format!("{name}-soft.png")), soft);
    save_png(
        &dir.join(format!("{name}-diff.png")),
        &diff_frame(sdl, soft, budget.channel),
    );
    Some(format!(
        "{name}: {} px differ by more than {} (budget {} px, max delta {}); \
         first (x, y, sdl, soft): {:?}; dumps in {}",
        d.mismatched,
        budget.channel,
        budget.max_pixels,
        d.max_delta,
        d.samples,
        dir.display()
    ))
}

/// Splash phases: BIOS text reveal, BIOS complete, logo fade-in (vignette
/// + glow textures), logo hold, horizon glow, fade-out.
const SPLASH_TIMES: [f32; 7] = [0.4, 1.6, 3.2, 3.9, 4.6, 5.4, 6.3];

const SPLASH_BUDGET: Budget = Budget {
    channel: 3,
    max_pixels: 16,
};

#[test]
fn boot_splash_matches_software_render() {
    assert!(SPLASH_TIMES.iter().all(|&t| t < SPLASH_DURATION_S));
    // 1280x720 is where PSP-native skins run on the desktop; 800x600 has
    // a different aspect (letterboxed layout, odd texture sizes).
    for (w, h) in [(1280u32, 720u32), (800, 600)] {
        let sdl_frames = with_sdl(w, h, |mut sdl| {
            let splash =
                BootSplash::start_themed(&mut sdl, w, h, SplashTheme::default()).expect("splash");
            SPLASH_TIMES
                .iter()
                .map(|&t| {
                    splash.render_at(&mut sdl, t).expect("render");
                    capture(&sdl, w, h)
                })
                .collect::<Vec<Frame>>()
        });
        let mut soft = Ue5Backend::new(w, h);
        let splash =
            BootSplash::start_themed(&mut soft, w, h, SplashTheme::default()).expect("splash");
        let mut failures = Vec::new();
        for (i, &t) in SPLASH_TIMES.iter().enumerate() {
            splash.render_at(&mut soft, t).expect("render");
            let soft_frame = capture(&soft, w, h);
            let name = format!("splash-{w}x{h}-t{t:.1}");
            if let Some(e) = compare(&name, &sdl_frames[i], &soft_frame, SPLASH_BUDGET) {
                failures.push(e);
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }
}

/// Boot `skin` hermetically on `backend`, run `frames` frames of virtual
/// time (presenting the ones the shell asks for, as the desktop loop and
/// the e2e harness do: transitions advance per presented frame) and
/// render one more frame.
fn boot_and_render<B: ShellBackend>(skin: &str, backend: B, frames: u32) -> Shell<B> {
    let mut opts = BootOptions::hermetic(skin).expect("skin");
    opts.fixed_time = Some(FIXED_TIME);
    let mut shell = Shell::boot(
        opts,
        backend,
        || Box::new(RecordingAudio::new()),
        &mut NoSplash,
    )
    .expect("boot");
    let start = Instant::now();
    shell.reset_clocks(start);
    let mut now = start;
    for _ in 0..frames {
        now += FRAME;
        if shell.step(&[], now).redraw {
            shell.render(now).expect("render");
        }
    }
    shell.render(now).expect("render");
    shell
}

fn screen_size(skin: &str) -> (u32, u32) {
    let opts = BootOptions::hermetic(skin).expect("skin");
    (opts.config.screen_width, opts.config.screen_height)
}

const SHELL_BUDGET: Budget = Budget {
    channel: 3,
    max_pixels: 16,
};

#[test]
fn dashboard_frames_match_software_render() {
    let mut failures = Vec::new();
    // Settled dashboards (3 s of virtual time) of a PSP-style skin and two
    // desktop-style skins. Mid-transition frames are not compared: how far
    // the entrance fade has progressed depends on how many frames the
    // shell chose to present, which is not deterministic across runs
    // (seen with the Direct3D renderer), so it would test the shell's
    // pacing rather than the backend.
    const SETTLE_FRAMES: u32 = 180;
    for skin in ["classic", "xp", "modern"] {
        let (w, h) = screen_size(skin);
        let sdl_frame = with_sdl(w, h, |sdl| {
            let shell = boot_and_render(skin, sdl, SETTLE_FRAMES);
            capture(&shell.backend, w, h)
        });
        let shell = boot_and_render(skin, HeadlessBackend::new(w, h), SETTLE_FRAMES);
        let soft_frame = capture(&shell.backend, w, h);
        let name = format!("shell-{skin}");
        if let Some(e) = compare(&name, &sdl_frame, &soft_frame, SHELL_BUDGET) {
            failures.push(e);
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
