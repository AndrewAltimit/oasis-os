//! Animated boot splash screen rendered from an embedded SVG.
//!
//! Displays a BIOS-style boot sequence followed by a splash logo
//! before handing off to the dashboard. Uses the SVG renderer
//! directly — no browser engine or CSS animation engine required.
//!
//! ## Functional boot
//!
//! Unlike a pure decoration, the BIOS-phase lines are caller-driven:
//! [`BootSplash::set_bios_line`] overwrites a line's text so `main.rs`
//! can report actual system probes (RAM, VFS file count, skin name,
//! command/plugin counts). The caller interleaves real init work
//! with animation frames via [`BootSplash::run_until`] — advancing
//! the splash to the next BIOS reveal time while the main thread
//! runs one unit of startup work.
//!
//! The animation timing mirrors the CSS `animation-delay` values in
//! the original `boot_8.svg`:
//!
//! | Element              | Delay (s) | Effect            |
//! |----------------------|-----------|-------------------|
//! | BIOS line 0          | 0.4       | opacity 0 → 1    |
//! | BIOS line 1          | 0.8       | opacity 0 → 1    |
//! | BIOS line 2          | 1.2       | opacity 0 → 1    |
//! | BIOS line 3          | 1.6       | opacity 0 → 1    |
//! | BIOS line 4          | 2.1       | opacity 0 → 1    |
//! | BIOS line 5          | 2.6       | opacity 0 → 1    |
//! | BIOS line 6          | 3.0       | opacity 0 → 1    |
//! | CRT flicker          | 3.4       | white flash       |
//! | BIOS screen hide     | 3.6       | opacity 1 → 0    |
//! | Splash BG reveal     | 3.5       | opacity 0 → 1    |
//! | Horizon glow         | 3.8       | pulse start       |
//! | Logo group           | 4.0       | opacity 0 → 1    |
//! | Loading text         | 5.5       | opacity 0 → 1    |
//! | End                  | 6.5       | fade to dashboard |

use oasis_core::backend::{Color, InputBackend, SdiBackend, TextureId};
use oasis_core::error::Result;

/// Total duration of the boot splash in seconds.
pub const SPLASH_DURATION_S: f32 = 6.5;

/// Reveal times for the 7 BIOS lines (seconds since splash start).
pub const BIOS_REVEAL_TIMES: [f32; 7] = [0.4, 0.8, 1.2, 1.6, 2.1, 2.6, 3.0];

/// Minimum frame time to cap at ~60 FPS.
const MIN_FRAME_MS: u128 = 16;

/// Default BIOS lines — overridden by the caller via `set_bios_line`
/// once real system probes complete.
const DEFAULT_BIOS_LINES: [&str; 7] = [
    "OASIS_KERNEL_V.7.0.4 BOOT SEQUENCE INITIATED",
    "COPYRIGHT (C) 199X OASIS CORP. ALL RIGHTS RESERVED.",
    "SYSTEM RAM CHECK... OK",
    "INITIALIZING VIRTUAL FILE SYSTEM... OK",
    "MOUNTING BOOT DRIVE /DEV/HDA1... OK",
    "LOADING FRAGMENT... OK",
    "STARTING DISPLAY MANAGER",
];

/// Default progress note shown during the splash phase (5.5s+).
const DEFAULT_PROGRESS_NOTE: &str = "SYSTEM MODULES INITIALIZED";

/// Pre-computed GPU textures for splash effects.
struct SplashTextures {
    /// Full-screen CRT vignette (dark edges, clear center).
    vignette: Option<TextureId>,
    /// Blurred logo strokes for glow effect.
    logo_glow: Option<TextureId>,
    logo_glow_w: u32,
    logo_glow_h: u32,
    /// Screen-space position of the glow texture.
    logo_glow_x: i32,
    logo_glow_y: i32,
    /// Blurred horizon line for bloom effect.
    horizon_glow: Option<TextureId>,
    horizon_glow_w: u32,
    horizon_glow_h: u32,
    horizon_glow_y: i32,
}

impl SplashTextures {
    /// Pre-compute and upload effect textures.
    fn create(backend: &mut dyn SdiBackend, screen_w: u32, screen_h: u32, scale: f32) -> Self {
        let vignette = generate_vignette_texture(backend, screen_w, screen_h);
        let (logo_glow, gw, gh, gx, gy) =
            generate_logo_glow_texture(backend, screen_w, screen_h, scale);
        let (horizon_glow, hw, hh, hy) =
            generate_horizon_glow_texture(backend, screen_w, screen_h, scale);
        SplashTextures {
            vignette,
            logo_glow,
            logo_glow_w: gw,
            logo_glow_h: gh,
            logo_glow_x: gx,
            logo_glow_y: gy,
            horizon_glow,
            horizon_glow_w: hw,
            horizon_glow_h: hh,
            horizon_glow_y: hy,
        }
    }

    /// Release GPU textures.
    fn destroy(&self, backend: &mut dyn SdiBackend) {
        if let Some(tex) = self.vignette {
            let _ = backend.destroy_texture(tex);
        }
        if let Some(tex) = self.logo_glow {
            let _ = backend.destroy_texture(tex);
        }
        if let Some(tex) = self.horizon_glow {
            let _ = backend.destroy_texture(tex);
        }
    }
}

/// Stateful boot splash. Keeps the animation running across multiple
/// `run_until` calls while the caller runs real init work in between.
pub struct BootSplash {
    start: std::time::Instant,
    screen_w: u32,
    screen_h: u32,
    sx: f32,
    sy: f32,
    scale: f32,
    textures: SplashTextures,
    bios_lines: [String; 7],
    progress_note: String,
    skipped: bool,
}

impl BootSplash {
    /// Start the splash. Pre-computes GPU textures for the animation.
    ///
    /// The caller should call [`run_until`](Self::run_until) repeatedly,
    /// interleaving real init work between calls. When done (or skipped),
    /// call [`finish`](Self::finish) to fade out and release textures.
    pub fn start(backend: &mut impl SdiBackend, screen_w: u32, screen_h: u32) -> Result<Self> {
        let sx = screen_w as f32 / 1280.0;
        let sy = screen_h as f32 / 720.0;
        let scale = sx.min(sy);
        let textures = SplashTextures::create(backend, screen_w, screen_h, scale);

        Ok(Self {
            start: std::time::Instant::now(),
            screen_w,
            screen_h,
            sx,
            sy,
            scale,
            textures,
            bios_lines: DEFAULT_BIOS_LINES.map(String::from),
            progress_note: DEFAULT_PROGRESS_NOTE.to_string(),
            skipped: false,
        })
    }

    /// Seconds since the splash started.
    pub fn elapsed(&self) -> f32 {
        self.start.elapsed().as_secs_f32()
    }

    /// Overwrite a BIOS line's text. Takes effect on the next render.
    ///
    /// `idx` must be 0..7. If the line is already revealed when updated,
    /// the new text replaces the old in-place with no re-animation.
    pub fn set_bios_line(&mut self, idx: usize, text: impl Into<String>) {
        if let Some(slot) = self.bios_lines.get_mut(idx) {
            *slot = text.into();
        }
    }

    /// Set the text shown at the bottom of the splash phase (replaces
    /// "SYSTEM MODULES INITIALIZED"). Visible from 5.5s onward.
    pub fn set_progress_note(&mut self, text: impl Into<String>) {
        self.progress_note = text.into();
    }

    /// Render frames until `target_secs` is reached, the user skips, or
    /// the splash ends. Returns `Ok(true)` if the user skipped (now or
    /// in a prior call), `Ok(false)` otherwise.
    ///
    /// Once skipped, this call returns immediately (no rendering).
    pub fn run_until(
        &mut self,
        backend: &mut (impl SdiBackend + InputBackend),
        target_secs: f32,
    ) -> Result<bool> {
        if self.skipped {
            return Ok(true);
        }
        let target = target_secs.min(SPLASH_DURATION_S);
        while self.elapsed() < target {
            let frame_start = std::time::Instant::now();

            // Skip on any button press.
            let events = backend.poll_events();
            if events
                .iter()
                .any(|e| matches!(e, oasis_core::input::InputEvent::ButtonPress(_)))
            {
                self.skipped = true;
                return Ok(true);
            }

            self.render_frame(backend)?;
            backend.swap_buffers()?;

            // Frame rate cap.
            let ft = frame_start.elapsed().as_millis();
            if ft < MIN_FRAME_MS {
                std::thread::sleep(std::time::Duration::from_millis((MIN_FRAME_MS - ft) as u64));
            }
        }
        Ok(false)
    }

    /// Run the animation to completion.
    pub fn run_to_end(&mut self, backend: &mut (impl SdiBackend + InputBackend)) -> Result<bool> {
        self.run_until(backend, SPLASH_DURATION_S)
    }

    /// Fade out to black, then release GPU textures. Consumes self.
    ///
    /// If the splash was skipped, fades directly to black without the
    /// usual 0.3s transition.
    pub fn finish(self, backend: &mut impl SdiBackend) -> Result<()> {
        let skipped = self.skipped;
        let screen_w = self.screen_w;
        let screen_h = self.screen_h;

        let result: Result<()> = (|| {
            if !skipped {
                // Fade out to black over ~0.3s.
                let fade_start = std::time::Instant::now();
                while fade_start.elapsed().as_secs_f32() < 0.3 {
                    let t = fade_start.elapsed().as_secs_f32() / 0.3;
                    let alpha = (t * 255.0).min(255.0) as u8;
                    backend.clear(Color::rgb(0, 0, 0))?;
                    backend.fill_rect(0, 0, screen_w, screen_h, Color::rgba(0, 0, 0, alpha))?;
                    backend.swap_buffers()?;
                    std::thread::sleep(std::time::Duration::from_millis(16));
                }
            }
            backend.clear(Color::rgb(0, 0, 0))?;
            backend.swap_buffers()?;
            Ok(())
        })();

        self.textures.destroy(backend);
        result
    }

    /// Render a single splash frame at the current elapsed time.
    fn render_frame(&self, backend: &mut impl SdiBackend) -> Result<()> {
        let elapsed = self.elapsed();
        backend.clear(Color::rgb(5, 5, 5))?;

        // Phase 1: BIOS screen (0.0 - 3.6s)
        let bios_opacity = if elapsed < 3.6 { 1.0 } else { 0.0 };
        if bios_opacity > 0.0 {
            paint_bios_screen(
                backend,
                elapsed,
                self.sx,
                self.sy,
                self.scale,
                &self.bios_lines,
            )?;
        }

        // Phase transition: CRT flicker (3.4 - 3.9s)
        if (3.4..3.9).contains(&elapsed) {
            let t = (elapsed - 3.4) / 0.5;
            let alpha = if t < 0.1 {
                (t / 0.1 * 255.0) as u8
            } else if t < 0.6 {
                255
            } else {
                ((1.0 - (t - 0.6) / 0.4) * 255.0).max(0.0) as u8
            };
            let flicker_color = if t < 0.3 {
                Color::rgba(255, 255, 255, alpha)
            } else if t < 0.6 {
                Color::rgba(191, 0, 255, alpha)
            } else {
                Color::rgba(255, 255, 255, alpha)
            };
            backend.fill_rect(0, 0, self.screen_w, self.screen_h, flicker_color)?;
        }

        // Phase 2: Splash screen (3.5s+)
        if elapsed >= 3.5 {
            paint_splash_screen(
                backend,
                elapsed,
                self.screen_w,
                self.screen_h,
                self.sx,
                self.sy,
                self.scale,
                &self.textures,
                &self.progress_note,
            )?;
        }

        Ok(())
    }
}

/// Generate a full-screen vignette texture with per-pixel radial alpha.
///
/// Matches SVG: `<radialGradient id="vignette" cx="50%" cy="50%" r="75%">`
/// with stops at 50% (transparent) → 100% (black at 0.85 opacity).
fn generate_vignette_texture(
    backend: &mut dyn SdiBackend,
    screen_w: u32,
    screen_h: u32,
) -> Option<TextureId> {
    // Render at half resolution to save memory + upload time.
    let w = screen_w / 2;
    let h = screen_h / 2;
    let mut pixels = vec![0u8; (w * h * 4) as usize];
    let cx = w as f32 / 2.0;
    let cy = h as f32 / 2.0;
    let max_r = (cx * cx + cy * cy).sqrt();
    let inner_frac = 0.50; // 50% stop — fully transparent
    let outer_frac = 1.00; // 100% stop — 0.85 black
    let inner_r = max_r * inner_frac;

    for y in 0..h {
        for x in 0..w {
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let dist = (dx * dx + dy * dy).sqrt();
            let alpha = if dist <= inner_r {
                0.0
            } else {
                let t = ((dist - inner_r) / (max_r * outer_frac - inner_r)).clamp(0.0, 1.0);
                // Smooth ease-in for natural vignette falloff.
                t * t * 0.85
            };
            let idx = ((y * w + x) * 4) as usize;
            pixels[idx] = 0; // R
            pixels[idx + 1] = 0; // G
            pixels[idx + 2] = 0; // B
            pixels[idx + 3] = (alpha * 255.0) as u8; // A
        }
    }
    backend.load_texture(w, h, &pixels).ok()
}

/// Generate a blurred logo glow texture.
///
/// Renders the logo letter strokes into a buffer, applies 3-pass box
/// blur (approximating Gaussian stdDeviation=4), and uploads as a
/// texture with alpha for additive-style glow compositing.
fn generate_logo_glow_texture(
    backend: &mut dyn SdiBackend,
    screen_w: u32,
    screen_h: u32,
    scale: f32,
) -> (Option<TextureId>, u32, u32, i32, i32) {
    let sx = screen_w as f32 / 1280.0;
    let sy = screen_h as f32 / 720.0;

    // Logo bounding box in SVG coords: x=265..1015, y=280..455.
    // Add padding for blur spread.
    let pad = 20.0 * scale;
    let logo_x = (265.0 * sx - pad) as i32;
    let logo_y = (280.0 * sy - pad) as i32;
    let logo_w = ((1015.0 - 265.0) * sx + pad * 2.0) as u32;
    let logo_h = ((455.0 - 280.0) * sy + pad * 2.0) as u32;

    if logo_w == 0 || logo_h == 0 {
        return (None, 0, 0, 0, 0);
    }

    // Render logo strokes into a local buffer.
    let mut buf = vec![0u8; (logo_w * logo_h * 4) as usize];
    let sw = (7.0 * scale).max(2.0) as i32;

    // Helper: draw a thick line into the buffer.
    let mut draw_line = |x1: f32, y1: f32, x2: f32, y2: f32| {
        let px1 = (x1 * sx) as i32 - logo_x;
        let py1 = (y1 * sy) as i32 - logo_y;
        let px2 = (x2 * sx) as i32 - logo_x;
        let py2 = (y2 * sy) as i32 - logo_y;
        let dx = (px2 - px1).abs();
        let dy = (py2 - py1).abs();
        let steps = dx.max(dy).max(1);
        for s in 0..=steps {
            let t = s as f32 / steps as f32;
            let px = px1 + ((px2 - px1) as f32 * t) as i32;
            let py = py1 + ((py2 - py1) as f32 * t) as i32;
            for oy in 0..sw {
                for ox in 0..sw {
                    let bx = (px + ox) as u32;
                    let by = (py + oy) as u32;
                    if bx < logo_w && by < logo_h {
                        let idx = ((by * logo_w + bx) * 4) as usize;
                        buf[idx] = 255;
                        buf[idx + 1] = 255;
                        buf[idx + 2] = 255;
                        buf[idx + 3] = 255;
                    }
                }
            }
        }
    };

    // Render all logo strokes into buffer (same coordinates as paint_logo_scaled).
    // Left bracket
    draw_line(315.0, 290.0, 275.0, 290.0);
    draw_line(275.0, 290.0, 275.0, 445.0);
    draw_line(275.0, 445.0, 315.0, 445.0);
    // O (angular rectangle)
    draw_line(410.0, 290.0, 490.0, 290.0);
    draw_line(490.0, 290.0, 490.0, 410.0);
    draw_line(490.0, 410.0, 410.0, 410.0);
    draw_line(410.0, 410.0, 410.0, 290.0);
    // A
    draw_line(520.0, 410.0, 560.0, 290.0);
    draw_line(560.0, 290.0, 600.0, 410.0);
    // S (first — upper-left vertical, lower-right vertical)
    draw_line(630.0, 290.0, 710.0, 290.0); // top
    draw_line(630.0, 290.0, 630.0, 350.0); // upper-left
    draw_line(630.0, 350.0, 710.0, 350.0); // middle
    draw_line(710.0, 350.0, 710.0, 410.0); // lower-right
    draw_line(630.0, 410.0, 710.0, 410.0); // bottom
    // I
    draw_line(740.0, 290.0, 740.0, 410.0);
    // S (second)
    draw_line(770.0, 290.0, 850.0, 290.0); // top
    draw_line(770.0, 290.0, 770.0, 350.0); // upper-left
    draw_line(770.0, 350.0, 850.0, 350.0); // middle
    draw_line(850.0, 350.0, 850.0, 410.0); // lower-right
    draw_line(770.0, 410.0, 850.0, 410.0); // bottom
    // Right bracket
    draw_line(965.0, 290.0, 1005.0, 290.0);
    draw_line(1005.0, 290.0, 1005.0, 445.0);
    draw_line(1005.0, 445.0, 965.0, 445.0);

    // Apply 3-pass separable box blur (approximates Gaussian stdDeviation≈4).
    let blur_radius = (4.0 * scale).max(2.0) as i32;
    box_blur_rgba(&mut buf, logo_w, logo_h, blur_radius);
    box_blur_rgba(&mut buf, logo_w, logo_h, blur_radius);
    box_blur_rgba(&mut buf, logo_w, logo_h, blur_radius);

    let tex = backend.load_texture(logo_w, logo_h, &buf).ok();
    (tex, logo_w, logo_h, logo_x, logo_y)
}

/// Generate a blurred horizon line texture for bloom effect.
///
/// Renders a bright white line at the center of the texture, applies
/// two blur passes (stdDeviation=8 and stdDeviation=2 from heavyGlow
/// filter), and merges the results.
fn generate_horizon_glow_texture(
    backend: &mut dyn SdiBackend,
    screen_w: u32,
    screen_h: u32,
    scale: f32,
) -> (Option<TextureId>, u32, u32, i32) {
    let sy = screen_h as f32 / 720.0;
    let w = screen_w;
    let h = (64.0 * scale).max(16.0) as u32; // tall enough for blur spread
    let center_y = h / 2;
    let horizon_screen_y = (500.0 * sy) as i32 - center_y as i32;

    let mut buf = vec![0u8; (w * h * 4) as usize];

    // Draw a bright white line at the center (4px wide for the source).
    let line_hw = (4.0 * scale).max(2.0) as u32;
    for x in 0..w {
        for dy in 0..line_hw {
            let y = center_y + dy - line_hw / 2;
            if y < h {
                let idx = ((y * w + x) * 4) as usize;
                buf[idx] = 255;
                buf[idx + 1] = 255;
                buf[idx + 2] = 255;
                buf[idx + 3] = 255;
            }
        }
    }

    // Blur pass 1: stdDeviation=8 (heavyGlow's first feGaussianBlur).
    let mut blur1 = buf.clone();
    let r1 = (8.0 * scale).max(2.0) as i32;
    box_blur_rgba(&mut blur1, w, h, r1);
    box_blur_rgba(&mut blur1, w, h, r1);
    box_blur_rgba(&mut blur1, w, h, r1);

    // Blur pass 2: stdDeviation=2 (heavyGlow's second feGaussianBlur).
    let mut blur2 = buf.clone();
    let r2 = (2.0 * scale).max(1.0) as i32;
    box_blur_rgba(&mut blur2, w, h, r2);
    box_blur_rgba(&mut blur2, w, h, r2);
    box_blur_rgba(&mut blur2, w, h, r2);

    // feMerge: blur1 + blur2 + source (additive alpha blending).
    for i in (0..buf.len()).step_by(4) {
        let a = (blur1[i + 3] as u16 + blur2[i + 3] as u16 + buf[i + 3] as u16).min(255) as u8;
        buf[i] = 255; // white
        buf[i + 1] = 255;
        buf[i + 2] = 255;
        buf[i + 3] = a;
    }

    let tex = backend.load_texture(w, h, &buf).ok();
    (tex, w, h, horizon_screen_y)
}

/// 1-pass separable box blur on an RGBA buffer (horizontal then vertical).
fn box_blur_rgba(buf: &mut [u8], w: u32, h: u32, half: i32) {
    let mut tmp = buf.to_vec();
    let wi = w as i32;
    let hi = h as i32;

    // Horizontal pass: buf → tmp.
    for y in 0..hi {
        for x in 0..wi {
            let (mut r, mut g, mut b, mut a) = (0u32, 0u32, 0u32, 0u32);
            let mut n = 0u32;
            for dx in -half..=half {
                let sx = (x + dx).clamp(0, wi - 1) as usize;
                let off = (y as usize) * (w as usize) * 4 + sx * 4;
                r += buf[off] as u32;
                g += buf[off + 1] as u32;
                b += buf[off + 2] as u32;
                a += buf[off + 3] as u32;
                n += 1;
            }
            let off = (y as usize) * (w as usize) * 4 + (x as usize) * 4;
            tmp[off] = (r / n) as u8;
            tmp[off + 1] = (g / n) as u8;
            tmp[off + 2] = (b / n) as u8;
            tmp[off + 3] = (a / n) as u8;
        }
    }

    // Vertical pass: tmp → buf.
    for x in 0..wi {
        for y in 0..hi {
            let (mut r, mut g, mut b, mut a) = (0u32, 0u32, 0u32, 0u32);
            let mut n = 0u32;
            for dy in -half..=half {
                let sy = (y + dy).clamp(0, hi - 1) as usize;
                let off = sy * (w as usize) * 4 + (x as usize) * 4;
                r += tmp[off] as u32;
                g += tmp[off + 1] as u32;
                b += tmp[off + 2] as u32;
                a += tmp[off + 3] as u32;
                n += 1;
            }
            let off = (y as usize) * (w as usize) * 4 + (x as usize) * 4;
            buf[off] = (r / n) as u8;
            buf[off + 1] = (g / n) as u8;
            buf[off + 2] = (b / n) as u8;
            buf[off + 3] = (a / n) as u8;
        }
    }
}

// -----------------------------------------------------------------------
// BIOS screen painting
// -----------------------------------------------------------------------

fn paint_bios_screen(
    backend: &mut dyn SdiBackend,
    elapsed: f32,
    sx: f32,
    sy: f32,
    scale: f32,
    lines: &[String; 7],
) -> Result<()> {
    let fs = (22.0 * scale).max(8.0) as u16;
    let c = Color::rgb(204, 204, 204);

    let x = (40.0 * sx) as i32;
    let y_positions = [60.0, 95.0, 150.0, 185.0, 220.0, 275.0, 310.0];

    let ls = (1.0 * scale).round() as i32; // SVG: letter-spacing="1"
    for (i, text) in lines.iter().enumerate() {
        let delay = BIOS_REVEAL_TIMES[i];
        if elapsed >= delay {
            // Each line fades in over 100ms (SVG: animation 0.1s forwards).
            let line_alpha = ((elapsed - delay) / 0.1).clamp(0.0, 1.0);
            let lc = apply_alpha(c, line_alpha);
            let py = (y_positions[i] * sy) as i32;
            if ls > 0 {
                let mut cx = x;
                for ch in text.chars() {
                    let mut buf = [0u8; 4];
                    let s = ch.encode_utf8(&mut buf);
                    backend.draw_text(s, cx, py, fs, lc)?;
                    let cw = backend.measure_text(s, fs) as i32;
                    cx += cw + ls;
                }
            } else {
                backend.draw_text(text, x, py, fs, lc)?;
            }
        }
    }

    // Blinking cursor on last line.
    if elapsed >= BIOS_REVEAL_TIMES[6] {
        let blink = ((elapsed * 2.0) as u32).is_multiple_of(2);
        if blink {
            let last_line = &lines[6];
            let cursor_x = x + backend.measure_text(last_line, fs) as i32;
            let py = (310.0 * sy) as i32;
            backend.draw_text("_", cursor_x, py, fs, c)?;
        }
    }

    Ok(())
}

// -----------------------------------------------------------------------
// Splash screen painting
// -----------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn paint_splash_screen(
    backend: &mut dyn SdiBackend,
    elapsed: f32,
    screen_w: u32,
    screen_h: u32,
    sx: f32,
    sy: f32,
    scale: f32,
    textures: &SplashTextures,
    progress_note: &str,
) -> Result<()> {
    let splash_t = elapsed - 3.5;
    let splash_opacity = (splash_t / 0.01).clamp(0.0, 1.0); // instant reveal

    // Sky gradient (6 stops, vertical).
    let sky_h = (500.0 * sy) as u32;
    let sky_stops: &[(f32, Color)] = &[
        (0.00, Color::rgb(2, 0, 26)),
        (0.20, Color::rgb(5, 0, 68)),
        (0.50, Color::rgb(21, 0, 136)),
        (0.80, Color::rgb(85, 0, 204)),
        (0.95, Color::rgb(170, 85, 255)),
        (1.00, Color::rgb(255, 255, 255)),
    ];
    paint_vertical_gradient(backend, 0, 0, screen_w, sky_h, sky_stops, splash_opacity)?;

    // Ground gradient (4 stops, vertical) — extends to screen bottom.
    let ground_y = sky_h as i32;
    let ground_h = screen_h.saturating_sub(sky_h);
    let ground_stops: &[(f32, Color)] = &[
        (0.00, Color::rgb(255, 255, 255)),
        (0.05, Color::rgb(106, 0, 204)),
        (0.30, Color::rgb(21, 0, 136)),
        (1.00, Color::rgb(2, 0, 26)),
    ];
    paint_vertical_gradient(
        backend,
        0,
        ground_y,
        screen_w,
        ground_h,
        ground_stops,
        splash_opacity,
    )?;

    // Horizon glow + line (SVG uses heavyGlow filter: stdDeviation 8 + 2).
    let horizon_y = (500.0 * sy) as i32;
    if elapsed >= 3.8 {
        let pulse_t = elapsed - 3.8;
        let glow_alpha = 0.6 + 0.4 * (pulse_t * std::f32::consts::PI / 4.0).sin();
        // Pre-computed blurred horizon texture.
        if let Some(h_tex) = textures.horizon_glow {
            let _ = backend.blit_tinted(
                h_tex,
                0,
                textures.horizon_glow_y,
                textures.horizon_glow_w,
                textures.horizon_glow_h,
                Color::rgba(255, 255, 255, (glow_alpha * splash_opacity * 255.0) as u8),
            );
        }
    }
    // Crisp horizon line (1.5px stroke-width in SVG).
    let line_c = apply_alpha(Color::rgb(255, 255, 255), splash_opacity);
    let line_h = (1.5 * scale).max(1.0) as u32;
    backend.fill_rect(0, horizon_y, screen_w, line_h, line_c)?;

    // Logo group (appears at 4.0s with scale + brightness entrance).
    // SVG: animation logoTurnOn 0.6s 4.0s cubic-bezier(0.2,0.9,0.3,1.1)
    //   0%: opacity 0, scale(1.05), brightness(2) blur(4px)
    //  50%: opacity 1, brightness(1.5) blur(1px)
    // 100%: opacity 1, scale(1), brightness(1) blur(0)
    if elapsed >= 4.0 {
        let raw_t = ((elapsed - 4.0) / 0.6).clamp(0.0, 1.0);
        // Approximate cubic-bezier(0.2, 0.9, 0.3, 1.1) — overshoots slightly.
        let t = cubic_bezier_approx(raw_t, 0.2, 0.9, 0.3, 1.1);
        let logo_opacity = t.clamp(0.0, 1.0) * splash_opacity;

        // Scale: 1.05 → 1.0 (interpolated, slight overshoot from bezier).
        let logo_scale = 1.05 + (1.0 - 1.05) * t;

        // Brightness: 2.0 → 1.0 (boost toward white).
        let brightness = 2.0 + (1.0 - 2.0) * t.clamp(0.0, 1.0);

        // Pre-computed blurred glow texture behind the sharp strokes.
        // Scale the glow blit to match the logo scale.
        if let Some(glow_tex) = textures.logo_glow {
            let gw = (textures.logo_glow_w as f32 * logo_scale) as u32;
            let gh = (textures.logo_glow_h as f32 * logo_scale) as u32;
            let gcx = textures.logo_glow_x + textures.logo_glow_w as i32 / 2;
            let gcy = textures.logo_glow_y + textures.logo_glow_h as i32 / 2;
            let gx = gcx - gw as i32 / 2;
            let gy = gcy - gh as i32 / 2;
            let _ = backend.blit_tinted(
                glow_tex,
                gx,
                gy,
                gw,
                gh,
                Color::rgba(255, 255, 255, (logo_opacity * 255.0) as u8),
            );
        }

        // Crisp logo strokes with scale + brightness.
        let sw = (7.0 * scale).max(2.0) as u32;
        paint_logo_scaled(backend, sx, sy, logo_opacity, sw, logo_scale, brightness)?;
    }

    // Loading text (appears at 5.5s, base opacity 0.8 per SVG).
    if elapsed >= 5.5 {
        let load_t = (elapsed - 5.5) / 0.5;
        let load_opacity = load_t.clamp(0.0, 1.0) * splash_opacity * 0.8;
        let fs = (14.0 * scale).max(8.0) as u16;
        let text = progress_note;
        // Letter-spacing rendering (letter-spacing="2" in SVG).
        let ls = (2.0 * scale) as i32;
        // Measure total width with letter-spacing for centering.
        let total_w: i32 = text
            .chars()
            .map(|ch| {
                let mut buf = [0u8; 4];
                let s = ch.encode_utf8(&mut buf);
                backend.measure_text(s, fs) as i32 + ls
            })
            .sum::<i32>()
            - ls; // don't count trailing space
        let mut cx = (screen_w as i32 - total_w) / 2;
        let y = (660.0 * sy) as i32;
        let c = apply_alpha(Color::rgb(170, 136, 255), load_opacity);
        for ch in text.chars() {
            let mut buf = [0u8; 4];
            let s = ch.encode_utf8(&mut buf);
            backend.draw_text(s, cx, y, fs, c)?;
            let cw = backend.measure_text(s, fs) as i32;
            cx += cw + ls;
        }
    }

    // Scanline overlay (subtle).
    paint_scanlines(backend, screen_w, screen_h, sy, splash_opacity * 0.35)?;

    // CRT vignette overlay — pre-computed per-pixel radial texture.
    if let Some(vig_tex) = textures.vignette {
        // Blit at 2x size since vignette was rendered at half resolution.
        let _ = backend.blit(vig_tex, 0, 0, screen_w, screen_h);
    }

    Ok(())
}

// -----------------------------------------------------------------------
// Logo rendering
// -----------------------------------------------------------------------

/// Render the logo with scale-from-center and brightness boost.
///
/// `logo_scale`: 1.0 = normal, 1.05 = 5% larger from center.
/// `brightness`: 1.0 = normal white, 2.0 = double brightness (clamped).
fn paint_logo_scaled(
    backend: &mut dyn SdiBackend,
    sx: f32,
    sy: f32,
    opacity: f32,
    sw: u32,
    logo_scale: f32,
    brightness: f32,
) -> Result<()> {
    // Logo center in SVG coords: roughly (640, 367).
    let center_x = 640.0;
    let center_y = 367.0;

    // Scale-adjusted sx/sy: scale all coords from the logo center.
    let adj_sx = sx * logo_scale;
    let adj_sy = sy * logo_scale;
    // Offset to keep the center fixed after scaling.
    let offset_x = (center_x * sx) - (center_x * adj_sx);
    let offset_y = (center_y * sy) - (center_y * adj_sy);

    // Apply brightness: boost white toward (255,255,255) beyond 1.0.
    // At brightness=2.0 the color is still white (clamped), but opacity
    // effectively increases since the source is already white. To simulate
    // brightness > 1 on a white logo against dark background, we don't
    // need to change the color — the visual effect is from the glow layer
    // being more prominent. We do slightly increase opacity to simulate.
    let bright_opacity = (opacity * brightness.min(2.0)).clamp(0.0, 1.0);
    let c = apply_alpha(Color::rgb(255, 255, 255), bright_opacity);

    // Helper: draw line with adjusted scale.
    let draw = |b: &mut dyn SdiBackend, x1: f32, y1: f32, x2: f32, y2: f32| -> Result<()> {
        let px1 = (x1 * adj_sx + offset_x) as i32;
        let py1 = (y1 * adj_sy + offset_y) as i32;
        let px2 = (x2 * adj_sx + offset_x) as i32;
        let py2 = (y2 * adj_sy + offset_y) as i32;
        let dx = (px2 - px1).abs();
        let dy = (py2 - py1).abs();
        if dx == 0 || dy == 0 {
            let lx = px1.min(px2);
            let ly = py1.min(py2);
            let lw = (dx as u32).max(sw);
            let lh = (dy as u32).max(sw);
            b.fill_rect(lx, ly, lw, lh, c)?;
        } else {
            let steps = dx.max(dy);
            for s in 0..=steps {
                let t = s as f32 / steps.max(1) as f32;
                let px = px1 + ((px2 - px1) as f32 * t) as i32;
                let py = py1 + ((py2 - py1) as f32 * t) as i32;
                b.fill_rect(px, py, sw, sw, c)?;
            }
        }
        Ok(())
    };

    // Left bracket: [
    draw(backend, 315.0, 290.0, 275.0, 290.0)?;
    draw(backend, 275.0, 290.0, 275.0, 445.0)?;
    draw(backend, 275.0, 445.0, 315.0, 445.0)?;
    // O (angular rectangle outline)
    draw(backend, 410.0, 290.0, 490.0, 290.0)?;
    draw(backend, 490.0, 290.0, 490.0, 410.0)?;
    draw(backend, 490.0, 410.0, 410.0, 410.0)?;
    draw(backend, 410.0, 410.0, 410.0, 290.0)?;
    // A (inverted V)
    draw(backend, 520.0, 410.0, 560.0, 290.0)?;
    draw(backend, 560.0, 290.0, 600.0, 410.0)?;
    // S (angular — upper-left vertical, lower-right vertical)
    draw(backend, 630.0, 290.0, 710.0, 290.0)?; // top
    draw(backend, 630.0, 290.0, 630.0, 350.0)?; // upper-left
    draw(backend, 630.0, 350.0, 710.0, 350.0)?; // middle
    draw(backend, 710.0, 350.0, 710.0, 410.0)?; // lower-right
    draw(backend, 630.0, 410.0, 710.0, 410.0)?; // bottom
    // I
    draw(backend, 740.0, 290.0, 740.0, 410.0)?;
    // S (second)
    draw(backend, 770.0, 290.0, 850.0, 290.0)?; // top
    draw(backend, 770.0, 290.0, 770.0, 350.0)?; // upper-left
    draw(backend, 770.0, 350.0, 850.0, 350.0)?; // middle
    draw(backend, 850.0, 350.0, 850.0, 410.0)?; // lower-right
    draw(backend, 770.0, 410.0, 850.0, 410.0)?; // bottom
    // Right bracket: ]
    draw(backend, 965.0, 290.0, 1005.0, 290.0)?;
    draw(backend, 1005.0, 290.0, 1005.0, 445.0)?;
    draw(backend, 1005.0, 445.0, 965.0, 445.0)?;

    Ok(())
}

// -----------------------------------------------------------------------
// Helpers
// -----------------------------------------------------------------------

/// Approximate a cubic-bezier timing function.
///
/// Uses a simple polynomial approximation for the common case where
/// P1 and P2 control the easing shape. Exact cubic-bezier evaluation
/// requires iterative root-finding (Newton's method on the X curve);
/// this approximation uses direct evaluation of the parametric curve
/// at `t` which is close enough for animation purposes.
fn cubic_bezier_approx(t: f32, _x1: f32, y1: f32, _x2: f32, y2: f32) -> f32 {
    // Evaluate the Y component of the cubic Bezier at parameter t.
    // Control points: P0=(0,0), P1=(x1,y1), P2=(x2,y2), P3=(1,1).
    let u = 1.0 - t;
    3.0 * u * u * t * y1 + 3.0 * u * t * t * y2 + t * t * t
}

fn apply_alpha(c: Color, opacity: f32) -> Color {
    Color::rgba(
        c.r,
        c.g,
        c.b,
        (c.a as f32 * opacity.clamp(0.0, 1.0)).round() as u8,
    )
}

fn paint_vertical_gradient(
    backend: &mut dyn SdiBackend,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    stops: &[(f32, Color)],
    opacity: f32,
) -> Result<()> {
    if h == 0 || stops.is_empty() {
        return Ok(());
    }
    let bands = (h as usize).clamp(1, 128);
    let band_h = h as f32 / bands as f32;
    for i in 0..bands {
        let t = (i as f32 + 0.5) / bands as f32;
        let c = sample_stops(stops, t);
        let c = apply_alpha(c, opacity);
        let by = y + (i as f32 * band_h) as i32;
        let bh = (band_h.ceil() as u32).max(1);
        backend.fill_rect(x, by, w, bh, c)?;
    }
    Ok(())
}

fn sample_stops(stops: &[(f32, Color)], t: f32) -> Color {
    if stops.is_empty() {
        return Color::rgb(0, 0, 0);
    }
    if t <= stops[0].0 {
        return stops[0].1;
    }
    let last = stops.len() - 1;
    if t >= stops[last].0 {
        return stops[last].1;
    }
    for i in 0..last {
        if t >= stops[i].0 && t <= stops[i + 1].0 {
            let range = stops[i + 1].0 - stops[i].0;
            let local_t = if range > 0.0 {
                (t - stops[i].0) / range
            } else {
                0.0
            };
            return lerp_color(stops[i].1, stops[i + 1].1, local_t);
        }
    }
    stops[last].1
}

fn lerp_color(a: Color, b: Color, t: f32) -> Color {
    let inv = 1.0 - t;
    Color::rgba(
        (a.r as f32 * inv + b.r as f32 * t).round() as u8,
        (a.g as f32 * inv + b.g as f32 * t).round() as u8,
        (a.b as f32 * inv + b.b as f32 * t).round() as u8,
        (a.a as f32 * inv + b.a as f32 * t).round() as u8,
    )
}

fn paint_scanlines(
    backend: &mut dyn SdiBackend,
    screen_w: u32,
    screen_h: u32,
    sy: f32,
    opacity: f32,
) -> Result<()> {
    if opacity < 0.01 {
        return Ok(());
    }
    let c = apply_alpha(Color::rgb(0, 0, 0), opacity);
    let step = (4.0 * sy).max(2.0) as u32;
    let line_h = (step / 2).max(1);
    let mut y = 0u32;
    while y < screen_h {
        backend.fill_rect(0, y as i32, screen_w, line_h, c)?;
        y += step;
    }
    Ok(())
}
