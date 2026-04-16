//! Animated boot splash screen rendered from an embedded SVG.
//!
//! Displays a BIOS-style boot sequence followed by a splash logo
//! before handing off to the dashboard. Uses the SVG renderer
//! directly — no browser engine or CSS animation engine required.
//!
//! The animation timing is hardcoded to match the CSS `animation-delay`
//! values in `boot_8.svg`:
//!
//! | Element              | Delay (s) | Effect            |
//! |----------------------|-----------|-------------------|
//! | BIOS line 1          | 0.4       | opacity 0 → 1    |
//! | BIOS line 2          | 0.8       | opacity 0 → 1    |
//! | BIOS line 3          | 1.2       | opacity 0 → 1    |
//! | BIOS line 4          | 1.6       | opacity 0 → 1    |
//! | BIOS line 5          | 2.1       | opacity 0 → 1    |
//! | BIOS line 6          | 2.6       | opacity 0 → 1    |
//! | BIOS line 7          | 3.0       | opacity 0 → 1    |
//! | CRT flicker          | 3.4       | white flash       |
//! | BIOS screen hide     | 3.6       | opacity 1 → 0    |
//! | Splash BG reveal     | 3.5       | opacity 0 → 1    |
//! | Horizon glow         | 3.8       | pulse start       |
//! | Logo group           | 4.0       | opacity 0 → 1    |
//! | Subtitle             | 4.8       | opacity 0 → 1    |
//! | Loading text          | 5.5       | opacity 0 → 1    |
//! | End                  | 6.5       | fade to dashboard |

use oasis_core::backend::{Color, InputBackend, SdiBackend};
use oasis_core::error::Result;

/// Total duration of the boot splash in seconds.
const SPLASH_DURATION_S: f32 = 6.5;

/// Minimum frame time to cap at ~60 FPS.
const MIN_FRAME_MS: u128 = 16;

/// Run the boot splash animation, blocking until complete.
///
/// Returns `Ok(())` when the animation finishes or the user presses
/// a button to skip. The caller should proceed to normal init after
/// this returns.
pub fn run_boot_splash(
    backend: &mut (impl SdiBackend + InputBackend),
    screen_w: u32,
    screen_h: u32,
) -> Result<()> {
    let start = std::time::Instant::now();

    // Precompute layout constants scaled to screen.
    let sx = screen_w as f32 / 1280.0;
    let sy = screen_h as f32 / 720.0;
    let scale = sx.min(sy);

    loop {
        let frame_start = std::time::Instant::now();
        let elapsed = start.elapsed().as_secs_f32();

        // Allow skipping with any button press.
        let events = backend.poll_events();
        if events
            .iter()
            .any(|e| matches!(e, oasis_core::input::InputEvent::ButtonPress(_)))
        {
            break;
        }

        if elapsed >= SPLASH_DURATION_S {
            break;
        }

        backend.clear(Color::rgb(5, 5, 5))?;

        // Phase 1: BIOS screen (0.0 - 3.6s)
        let bios_opacity = if elapsed < 3.6 { 1.0 } else { 0.0 };
        if bios_opacity > 0.0 {
            paint_bios_screen(backend, elapsed, sx, sy, scale)?;
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
            backend.fill_rect(0, 0, screen_w, screen_h, flicker_color)?;
        }

        // Phase 2: Splash screen (3.5s+)
        if elapsed >= 3.5 {
            paint_splash_screen(backend, elapsed, screen_w, screen_h, sx, sy, scale)?;
        }

        backend.swap_buffers()?;

        // Frame rate limiting.
        let frame_time = frame_start.elapsed().as_millis();
        if frame_time < MIN_FRAME_MS {
            std::thread::sleep(std::time::Duration::from_millis(
                (MIN_FRAME_MS - frame_time) as u64,
            ));
        }
    }

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
    backend.clear(Color::rgb(0, 0, 0))?;
    backend.swap_buffers()?;

    Ok(())
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
) -> Result<()> {
    let fs = (22.0 * scale).max(8.0) as u16;
    let c = Color::rgb(204, 204, 204);

    let lines: &[(&str, f32)] = &[
        ("ALTIMIT_KERNEL_V.7.0.4 BOOT SEQUENCE INITIATED", 0.4),
        ("COPYRIGHT (C) 199X ALTIMIT CORP. ALL RIGHTS RESERVED.", 0.8),
        ("SYSTEM RAM CHECK... 1024000K OK", 1.2),
        ("INITIALIZING VIRTUAL FILE SYSTEM... OK", 1.6),
        ("MOUNTING BOOT DRIVE /DEV/HDA1... OK", 2.1),
        ("LOADING FRAGMENT... OK", 2.6),
        ("STARTING DISPLAY MANAGER_", 3.0),
    ];

    let x = (40.0 * sx) as i32;
    let y_positions = [60.0, 95.0, 150.0, 185.0, 220.0, 275.0, 310.0];

    for (i, (text, delay)) in lines.iter().enumerate() {
        if elapsed >= *delay {
            let py = (y_positions[i] * sy) as i32;
            backend.draw_text(text, x, py, fs, c)?;
        }
    }

    // Blinking cursor on last line.
    if elapsed >= 3.0 {
        let blink = ((elapsed * 2.0) as u32).is_multiple_of(2);
        if blink {
            let cursor_x = x + backend.measure_text("STARTING DISPLAY MANAGER", fs) as i32;
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

    // Ground gradient (4 stops, vertical).
    let ground_y = sky_h as i32;
    let ground_h = (220.0 * sy) as u32;
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

    // Horizon line.
    let horizon_y = (500.0 * sy) as i32;
    let line_c = apply_alpha(Color::rgb(255, 255, 255), splash_opacity);
    backend.fill_rect(
        0,
        horizon_y,
        screen_w,
        (2.0 * scale).max(1.0) as u32,
        line_c,
    )?;

    // Horizon glow (pulsing, starts at 3.8s).
    if elapsed >= 3.8 {
        let pulse_t = elapsed - 3.8;
        let glow_alpha = 0.6 + 0.4 * (pulse_t * std::f32::consts::PI / 4.0).sin();
        let glow_c = apply_alpha(Color::rgb(255, 255, 255), glow_alpha * splash_opacity);
        let glow_h = (4.0 * scale).max(2.0) as u32;
        backend.fill_rect(0, horizon_y - glow_h as i32, screen_w, glow_h * 2, glow_c)?;
    }

    // Logo group (appears at 4.0s).
    if elapsed >= 4.0 {
        let logo_t = (elapsed - 4.0) / 0.6;
        let logo_opacity = logo_t.clamp(0.0, 1.0) * splash_opacity;
        paint_logo(backend, sx, sy, scale, logo_opacity)?;
    }

    // Subtitle "OPERATING SYSTEMS" (appears at 4.8s).
    if elapsed >= 4.8 {
        let sub_t = (elapsed - 4.8) / 1.0;
        let sub_opacity = sub_t.clamp(0.0, 1.0) * splash_opacity;
        let fs = (20.0 * scale).max(8.0) as u16;
        let text = "OPERATING SYSTEMS";
        let tw = backend.measure_text(text, fs);
        let x = (screen_w as i32 - tw as i32) / 2;
        let y = (452.0 * sy) as i32;
        let c = apply_alpha(Color::rgb(255, 255, 255), sub_opacity);
        // Letter-spacing rendering.
        let ls = (14.0 * scale) as i32;
        let mut cx = x;
        for ch in text.chars() {
            let mut buf = [0u8; 4];
            let s = ch.encode_utf8(&mut buf);
            backend.draw_text(s, cx, y, fs, c)?;
            let cw = backend.measure_text(s, fs) as i32;
            cx += cw + ls;
        }
    }

    // Loading text (appears at 5.5s).
    if elapsed >= 5.5 {
        let load_t = (elapsed - 5.5) / 0.5;
        let load_opacity = load_t.clamp(0.0, 1.0) * splash_opacity;
        let fs = (14.0 * scale).max(8.0) as u16;
        let text = "SYSTEM MODULES INITIALIZED";
        let tw = backend.measure_text(text, fs);
        let x = (screen_w as i32 - tw as i32) / 2;
        let y = (660.0 * sy) as i32;
        let c = apply_alpha(Color::rgb(170, 136, 255), load_opacity);
        backend.draw_text(text, x, y, fs, c)?;
    }

    // Scanline overlay (subtle).
    paint_scanlines(backend, screen_w, screen_h, sy, splash_opacity * 0.35)?;

    Ok(())
}

// -----------------------------------------------------------------------
// Logo rendering
// -----------------------------------------------------------------------

fn paint_logo(
    backend: &mut dyn SdiBackend,
    sx: f32,
    sy: f32,
    scale: f32,
    opacity: f32,
) -> Result<()> {
    let c = apply_alpha(Color::rgb(255, 255, 255), opacity);
    let sw = (7.0 * scale).max(2.0) as u32;

    // Left bracket: [
    draw_line_thick(backend, 315.0, 290.0, 275.0, 290.0, sx, sy, sw, c)?;
    draw_line_thick(backend, 275.0, 290.0, 275.0, 445.0, sx, sy, sw, c)?;
    draw_line_thick(backend, 275.0, 445.0, 420.0, 445.0, sx, sy, sw, c)?;

    // A (inverted V)
    draw_line_thick(backend, 355.0, 410.0, 395.0, 290.0, sx, sy, sw, c)?;
    draw_line_thick(backend, 395.0, 290.0, 435.0, 410.0, sx, sy, sw, c)?;

    // L
    draw_line_thick(backend, 465.0, 290.0, 465.0, 410.0, sx, sy, sw, c)?;
    draw_line_thick(backend, 465.0, 410.0, 515.0, 410.0, sx, sy, sw, c)?;

    // Aperture O (simplified as hexagon outline)
    let ocx = 555.0 * sx;
    let ocy = 350.0 * sy;
    let or = 25.0 * scale;
    let hex_r = or as u16;
    if hex_r > 0 {
        backend.fill_circle(ocx as i32, ocy as i32, hex_r, c)?;
        // Inner hole.
        let inner_c = apply_alpha(Color::rgb(5, 0, 68), opacity);
        let inner_r = (5.0 * scale) as u16;
        if inner_r > 0 {
            backend.fill_circle(ocx as i32, ocy as i32, inner_r, inner_c)?;
        }
    }

    // T (first)
    draw_line_thick(backend, 595.0, 290.0, 655.0, 290.0, sx, sy, sw, c)?;
    draw_line_thick(backend, 625.0, 290.0, 625.0, 410.0, sx, sy, sw, c)?;

    // I (first)
    draw_line_thick(backend, 685.0, 290.0, 685.0, 410.0, sx, sy, sw, c)?;

    // M
    draw_line_thick(backend, 715.0, 410.0, 715.0, 290.0, sx, sy, sw, c)?;
    draw_line_thick(backend, 715.0, 290.0, 760.0, 355.0, sx, sy, sw, c)?;
    draw_line_thick(backend, 760.0, 355.0, 805.0, 290.0, sx, sy, sw, c)?;
    draw_line_thick(backend, 805.0, 290.0, 805.0, 410.0, sx, sy, sw, c)?;

    // I (second)
    draw_line_thick(backend, 835.0, 290.0, 835.0, 410.0, sx, sy, sw, c)?;

    // T (second)
    draw_line_thick(backend, 865.0, 290.0, 925.0, 290.0, sx, sy, sw, c)?;
    draw_line_thick(backend, 895.0, 290.0, 895.0, 410.0, sx, sy, sw, c)?;

    // Right bracket: ]
    draw_line_thick(backend, 965.0, 290.0, 1005.0, 290.0, sx, sy, sw, c)?;
    draw_line_thick(backend, 1005.0, 290.0, 1005.0, 445.0, sx, sy, sw, c)?;
    draw_line_thick(backend, 1005.0, 445.0, 860.0, 445.0, sx, sy, sw, c)?;

    Ok(())
}

// -----------------------------------------------------------------------
// Helpers
// -----------------------------------------------------------------------

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

#[allow(clippy::too_many_arguments)]
fn draw_line_thick(
    backend: &mut dyn SdiBackend,
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    sx: f32,
    sy: f32,
    sw: u32,
    color: Color,
) -> Result<()> {
    let px1 = (x1 * sx) as i32;
    let py1 = (y1 * sy) as i32;
    let px2 = (x2 * sx) as i32;
    let py2 = (y2 * sy) as i32;
    let dx = (px2 - px1).abs();
    let dy = (py2 - py1).abs();
    if dx == 0 || dy == 0 {
        let lx = px1.min(px2);
        let ly = py1.min(py2);
        let lw = (dx as u32).max(sw);
        let lh = (dy as u32).max(sw);
        backend.fill_rect(lx, ly, lw, lh, color)?;
    } else {
        let steps = dx.max(dy);
        for s in 0..=steps {
            let t = s as f32 / steps.max(1) as f32;
            let px = px1 + ((px2 - px1) as f32 * t) as i32;
            let py = py1 + ((py2 - py1) as f32 * t) as i32;
            backend.fill_rect(px, py, sw, sw, color)?;
        }
    }
    Ok(())
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
