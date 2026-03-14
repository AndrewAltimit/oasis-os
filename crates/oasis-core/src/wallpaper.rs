//! Procedural wallpaper generation.
//!
//! Generates PSIX-style gradient wallpapers as raw RGBA pixel buffers.
//! No external PNG files needed -- keeps CI clean and the binary self-contained.

use crate::backend::Color;
use oasis_types::color::lerp_color;

/// Generate a vibrant gradient wallpaper matching PSIX's orange->yellow->green style.
///
/// Returns an RGBA pixel buffer of `w * h * 4` bytes.
pub fn generate_gradient(w: u32, h: u32) -> Vec<u8> {
    let mut buf = vec![0u8; (w * h * 4) as usize];

    for y in 0..h {
        for x in 0..w {
            let offset = ((y * w + x) * 4) as usize;

            let nx = x as f32 / w as f32;
            let ny = y as f32 / h as f32;

            // PSIX gradient: hot orange (left) -> vivid yellow -> bright lime green (right).
            // Strong horizontal sweep with subtle vertical tint.
            let t = nx * 0.88 + ny * 0.12;

            let c = if t < 0.15 {
                // Hot orange-red -> vivid orange.
                let s = t / 0.15;
                lerp_color(Color::rgb(245, 110, 15), Color::rgb(255, 170, 15), s)
            } else if t < 0.32 {
                // Vivid orange -> bright golden yellow.
                let s = (t - 0.15) / 0.17;
                lerp_color(Color::rgb(255, 170, 15), Color::rgb(255, 230, 30), s)
            } else if t < 0.48 {
                // Golden yellow -> yellow-green.
                let s = (t - 0.32) / 0.16;
                lerp_color(Color::rgb(255, 230, 30), Color::rgb(230, 245, 40), s)
            } else if t < 0.65 {
                // Yellow-green -> bright green.
                let s = (t - 0.48) / 0.17;
                lerp_color(Color::rgb(230, 245, 40), Color::rgb(140, 235, 50), s)
            } else {
                // Bright green -> vivid lime.
                let s = (t - 0.65) / 0.35;
                lerp_color(Color::rgb(140, 235, 50), Color::rgb(200, 252, 130), s)
            };

            // Vertical brightness: lighter toward top, slightly darker at bottom.
            let vert = 1.0 + (0.5 - ny) * 0.18;

            // PSIX-style curved stripe arcs emanating from the lower-left.
            // Prominent overlapping wave bands -- the characteristic PSIX pattern.
            let dx = nx + 0.05;
            let dy = ny - 1.3;
            let dist = (dx * dx + dy * dy).sqrt();

            // Primary wave arcs (wide bands, high amplitude).
            let arc1 = (dist * 12.0).sin() * 0.18;
            // Secondary bands (medium frequency, visible layering).
            let arc2 = (dist * 22.0 + 1.2).sin() * 0.09;
            // Tertiary fine ripple.
            let arc3 = (dist * 36.0 + nx * 2.5).sin() * 0.04;

            // Arcs fade out toward the right (strongest on left).
            let arc_fade = (1.0 - nx * 0.45).clamp(0.0, 1.0);
            let wave = 1.0 + (arc1 + arc2 + arc3) * arc_fade;

            let scale = vert * wave;
            buf[offset] = (c.r as f32 * scale).clamp(0.0, 255.0) as u8;
            buf[offset + 1] = (c.g as f32 * scale).clamp(0.0, 255.0) as u8;
            buf[offset + 2] = (c.b as f32 * scale).clamp(0.0, 255.0) as u8;
            buf[offset + 3] = 255;
        }
    }

    buf
}

/// Generate a wallpaper from the active theme's wallpaper configuration.
///
/// Supports "gradient" (multi-stop with optional angle and wave arcs),
/// "solid" (first stop color fill), "none" (black fill), "grid" (bg with
/// grid lines), "scanlines" (alternate row darkening), "noise" (gradient
/// with per-pixel noise), and "dots" (bg with dots at grid intersections).
pub fn generate_from_config(w: u32, h: u32, at: &crate::active_theme::ActiveTheme) -> Vec<u8> {
    generate_from_config_with_phase(w, h, at, 0.0)
}

/// Same as [`generate_from_config`] but with an animation phase offset.
///
/// Pass `phase = 0.0` for static wallpapers. For animated wallpapers,
/// increment phase each frame (e.g., `+= 0.02`).
pub fn generate_from_config_with_phase(
    w: u32,
    h: u32,
    at: &crate::active_theme::ActiveTheme,
    phase: f32,
) -> Vec<u8> {
    match at.wallpaper.style.as_str() {
        "solid" => {
            let c = at
                .wallpaper
                .stops
                .first()
                .copied()
                .unwrap_or(crate::backend::Color::BLACK);
            generate_solid(w, h, c)
        },
        "none" => {
            let mut buf = vec![0u8; (w * h * 4) as usize];
            for i in (0..buf.len()).step_by(4) {
                buf[i + 3] = 255;
            }
            buf
        },
        "grid" => generate_grid(w, h, at),
        "scanlines" => generate_scanlines(w, h, at),
        "noise" => generate_noise(w, h, at),
        "dots" => generate_dots(w, h, at),
        "stripes" => generate_stripes(w, h, at),
        "checkerboard" => generate_checkerboard(w, h, at),
        _ => {
            // "gradient" -- multi-stop gradient with angle and optional wave.
            generate_gradient_config(
                w,
                h,
                &at.wallpaper.stops,
                at.wallpaper.angle,
                at.wallpaper.wave,
                at.wallpaper.wave_intensity,
                phase,
            )
        },
    }
}

/// Fill a solid color.
fn generate_solid(w: u32, h: u32, c: crate::backend::Color) -> Vec<u8> {
    let mut buf = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let offset = ((y * w + x) * 4) as usize;
            buf[offset] = c.r;
            buf[offset + 1] = c.g;
            buf[offset + 2] = c.b;
            buf[offset + 3] = 255;
        }
    }
    buf
}

/// Solid background with 1px horizontal and vertical grid lines.
fn generate_grid(w: u32, h: u32, at: &crate::active_theme::ActiveTheme) -> Vec<u8> {
    let bg = at
        .wallpaper
        .stops
        .first()
        .copied()
        .unwrap_or(crate::backend::Color::BLACK);
    let gc = at.wallpaper.grid_color;
    let spacing = at.wallpaper.grid_spacing.max(2);

    let mut buf = generate_solid(w, h, bg);
    for y in 0..h {
        for x in 0..w {
            if x % spacing == 0 || y % spacing == 0 {
                let offset = ((y * w + x) * 4) as usize;
                buf[offset] = blend_channel(bg.r, gc.r, gc.a);
                buf[offset + 1] = blend_channel(bg.g, gc.g, gc.a);
                buf[offset + 2] = blend_channel(bg.b, gc.b, gc.a);
            }
        }
    }
    buf
}

/// Solid background with every-other-row darkened for a CRT scanline effect.
fn generate_scanlines(w: u32, h: u32, at: &crate::active_theme::ActiveTheme) -> Vec<u8> {
    let bg = at
        .wallpaper
        .stops
        .first()
        .copied()
        .unwrap_or(crate::backend::Color::BLACK);

    let mut buf = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        let darken = if y % 2 == 1 { 0.85f32 } else { 1.0 };
        for x in 0..w {
            let offset = ((y * w + x) * 4) as usize;
            buf[offset] = (bg.r as f32 * darken) as u8;
            buf[offset + 1] = (bg.g as f32 * darken) as u8;
            buf[offset + 2] = (bg.b as f32 * darken) as u8;
            buf[offset + 3] = 255;
        }
    }
    buf
}

/// Gradient base with per-pixel noise applied on top.
fn generate_noise(w: u32, h: u32, at: &crate::active_theme::ActiveTheme) -> Vec<u8> {
    let mut buf = generate_gradient_config(
        w,
        h,
        &at.wallpaper.stops,
        at.wallpaper.angle,
        at.wallpaper.wave,
        at.wallpaper.wave_intensity,
        0.0,
    );
    let intensity = at.wallpaper.noise_intensity.clamp(0.0, 1.0);
    let max_offset = (intensity * 40.0) as i16;
    if max_offset == 0 {
        return buf;
    }
    // Simple PRNG (xorshift32) for deterministic noise.
    let mut rng: u32 = 0xDEAD_BEEF;
    for i in (0..buf.len()).step_by(4) {
        rng ^= rng << 13;
        rng ^= rng >> 17;
        rng ^= rng << 5;
        let noise = (rng % (2 * max_offset as u32 + 1)) as i16 - max_offset;
        buf[i] = (buf[i] as i16 + noise).clamp(0, 255) as u8;
        buf[i + 1] = (buf[i + 1] as i16 + noise).clamp(0, 255) as u8;
        buf[i + 2] = (buf[i + 2] as i16 + noise).clamp(0, 255) as u8;
    }
    buf
}

/// Solid background with small filled circles at grid intersections.
fn generate_dots(w: u32, h: u32, at: &crate::active_theme::ActiveTheme) -> Vec<u8> {
    let bg = at
        .wallpaper
        .stops
        .first()
        .copied()
        .unwrap_or(crate::backend::Color::BLACK);
    let gc = at.wallpaper.grid_color;
    let spacing = at.wallpaper.grid_spacing.max(4);
    let radius = (spacing / 6).max(1);
    let r2 = (radius * radius) as i32;

    let mut buf = generate_solid(w, h, bg);
    // Iterate grid intersection points and fill circles.
    let mut cy = spacing;
    while cy < h {
        let mut cx = spacing;
        while cx < w {
            // Draw filled circle around (cx, cy).
            let y_start = cy.saturating_sub(radius);
            let y_end = (cy + radius + 1).min(h);
            for y in y_start..y_end {
                let dy = y as i32 - cy as i32;
                let x_start = cx.saturating_sub(radius);
                let x_end = (cx + radius + 1).min(w);
                for x in x_start..x_end {
                    let dx = x as i32 - cx as i32;
                    if dx * dx + dy * dy <= r2 {
                        let offset = ((y * w + x) * 4) as usize;
                        buf[offset] = blend_channel(bg.r, gc.r, gc.a);
                        buf[offset + 1] = blend_channel(bg.g, gc.g, gc.a);
                        buf[offset + 2] = blend_channel(bg.b, gc.b, gc.a);
                    }
                }
            }
            cx += spacing;
        }
        cy += spacing;
    }
    buf
}

/// Alternating diagonal color bands.
///
/// Uses `wallpaper_angle` for band direction and `wallpaper_grid_spacing` for
/// band width. Colors from first two `wallpaper_stops`.
fn generate_stripes(w: u32, h: u32, at: &crate::active_theme::ActiveTheme) -> Vec<u8> {
    let c0 = at
        .wallpaper
        .stops
        .first()
        .copied()
        .unwrap_or(crate::backend::Color::BLACK);
    let c1 = at
        .wallpaper
        .stops
        .get(1)
        .copied()
        .unwrap_or(crate::backend::Color::rgb(40, 40, 40));
    let spacing = at.wallpaper.grid_spacing.max(2) as f32;
    let angle_rad = at.wallpaper.angle.to_radians();
    let cos_a = angle_rad.cos();
    let sin_a = angle_rad.sin();

    let mut buf = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let offset = ((y * w + x) * 4) as usize;
            let proj = x as f32 * cos_a + y as f32 * sin_a;
            let band = (proj / spacing) as i32;
            let c = if band % 2 == 0 { c0 } else { c1 };
            buf[offset] = c.r;
            buf[offset + 1] = c.g;
            buf[offset + 2] = c.b;
            buf[offset + 3] = 255;
        }
    }
    buf
}

/// Grid of alternating-color cells (checkerboard pattern).
///
/// Uses first two `wallpaper_stops` and `wallpaper_grid_spacing` for cell size.
fn generate_checkerboard(w: u32, h: u32, at: &crate::active_theme::ActiveTheme) -> Vec<u8> {
    let c0 = at
        .wallpaper
        .stops
        .first()
        .copied()
        .unwrap_or(crate::backend::Color::BLACK);
    let c1 = at
        .wallpaper
        .stops
        .get(1)
        .copied()
        .unwrap_or(crate::backend::Color::rgb(40, 40, 40));
    let spacing = at.wallpaper.grid_spacing.max(2);

    let mut buf = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let offset = ((y * w + x) * 4) as usize;
            let cx = x / spacing;
            let cy = y / spacing;
            let c = if (cx + cy).is_multiple_of(2) { c0 } else { c1 };
            buf[offset] = c.r;
            buf[offset + 1] = c.g;
            buf[offset + 2] = c.b;
            buf[offset + 3] = 255;
        }
    }
    buf
}

/// Alpha-blend a foreground channel over a background channel.
fn blend_channel(bg: u8, fg: u8, alpha: u8) -> u8 {
    let a = alpha as u16;
    ((fg as u16 * a + bg as u16 * (255 - a)) / 255) as u8
}

/// Multi-stop gradient with configurable angle and optional wave arcs.
fn generate_gradient_config(
    w: u32,
    h: u32,
    stops: &[crate::backend::Color],
    angle: f32,
    wave: bool,
    wave_intensity: f32,
    phase: f32,
) -> Vec<u8> {
    if stops.is_empty() {
        return generate_gradient(w, h);
    }
    if stops.len() == 1 {
        let c = stops[0];
        let mut buf = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let offset = ((y * w + x) * 4) as usize;
                buf[offset] = c.r;
                buf[offset + 1] = c.g;
                buf[offset + 2] = c.b;
                buf[offset + 3] = 255;
            }
        }
        return buf;
    }

    let mut buf = vec![0u8; (w * h * 4) as usize];
    let angle_rad = angle.to_radians();
    let cos_a = angle_rad.cos();
    let sin_a = angle_rad.sin();

    for y in 0..h {
        for x in 0..w {
            let offset = ((y * w + x) * 4) as usize;

            let nx = x as f32 / w as f32;
            let ny = y as f32 / h as f32;

            // Project along the angle direction.
            let t = (nx * cos_a + ny * sin_a).clamp(0.0, 1.0);

            // Multi-stop interpolation.
            let c = multi_stop_lerp(stops, t);

            // Vertical brightness variation (only for wave-style wallpapers).
            let vert = if wave { 1.0 + (0.5 - ny) * 0.18 } else { 1.0 };

            // Optional PSIX-style wave arcs (with animation phase).
            let wave_factor = if wave && wave_intensity > 0.0 {
                let dx = nx + 0.05;
                let dy = ny - 1.3;
                let dist = (dx * dx + dy * dy).sqrt();
                let arc1 = (dist * 12.0 + phase).sin() * 0.18;
                let arc2 = (dist * 22.0 + 1.2 + phase * 0.7).sin() * 0.09;
                let arc3 = (dist * 36.0 + nx * 2.5 + phase * 0.4).sin() * 0.04;
                let arc_fade = (1.0 - nx * 0.45).clamp(0.0, 1.0);
                1.0 + (arc1 + arc2 + arc3) * arc_fade * wave_intensity
            } else {
                1.0
            };

            let scale = vert * wave_factor;
            buf[offset] = (c.r as f32 * scale).clamp(0.0, 255.0) as u8;
            buf[offset + 1] = (c.g as f32 * scale).clamp(0.0, 255.0) as u8;
            buf[offset + 2] = (c.b as f32 * scale).clamp(0.0, 255.0) as u8;
            buf[offset + 3] = 255;
        }
    }

    buf
}

/// Interpolate between multiple color stops.
fn multi_stop_lerp(stops: &[Color], t: f32) -> Color {
    let n = stops.len();
    if n == 0 {
        return Color::BLACK;
    }
    if n == 1 {
        return stops[0];
    }
    let segment = t * (n - 1) as f32;
    let idx = (segment as usize).min(n - 2);
    let local_t = segment - idx as f32;
    lerp_color(stops[idx], stops[idx + 1], local_t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::active_theme::ActiveTheme;

    #[test]
    fn gradient_correct_size() {
        let buf = generate_gradient(480, 272);
        assert_eq!(buf.len(), 480 * 272 * 4);
    }

    #[test]
    fn gradient_all_opaque() {
        let buf = generate_gradient(16, 16);
        for y in 0..16u32 {
            for x in 0..16u32 {
                let offset = ((y * 16 + x) * 4 + 3) as usize;
                assert_eq!(buf[offset], 255, "pixel ({x},{y}) should be fully opaque");
            }
        }
    }

    #[test]
    fn gradient_not_uniform() {
        let buf = generate_gradient(480, 272);
        // Top-left and bottom-right should differ.
        let tl = (buf[0], buf[1], buf[2]);
        let idx = ((271 * 480 + 479) * 4) as usize;
        let br = (buf[idx], buf[idx + 1], buf[idx + 2]);
        assert_ne!(tl, br);
    }

    fn at_with_style(style: &str) -> ActiveTheme {
        let mut at = ActiveTheme::default();
        at.wallpaper.style = style.to_string();
        at
    }

    #[test]
    fn grid_correct_size() {
        let at = at_with_style("grid");
        let buf = generate_from_config(64, 64, &at);
        assert_eq!(buf.len(), 64 * 64 * 4);
    }

    #[test]
    fn grid_all_opaque() {
        let at = at_with_style("grid");
        let buf = generate_from_config(32, 32, &at);
        for i in (3..buf.len()).step_by(4) {
            assert_eq!(buf[i], 255);
        }
    }

    #[test]
    fn scanlines_alternate_rows() {
        let at = at_with_style("scanlines");
        let buf = generate_from_config(4, 4, &at);
        // Row 0 (even) and row 1 (odd) should differ.
        let r0 = (buf[0], buf[1], buf[2]);
        let r1_offset = (1 * 4 * 4) as usize; // row 1, col 0
        let r1 = (buf[r1_offset], buf[r1_offset + 1], buf[r1_offset + 2]);
        // Odd rows are darkened.
        assert_ne!(r0, r1);
    }

    #[test]
    fn noise_correct_size() {
        let at = at_with_style("noise");
        let buf = generate_from_config(32, 32, &at);
        assert_eq!(buf.len(), 32 * 32 * 4);
    }

    #[test]
    fn dots_correct_size() {
        let at = at_with_style("dots");
        let buf = generate_from_config(64, 64, &at);
        assert_eq!(buf.len(), 64 * 64 * 4);
    }

    #[test]
    fn stripes_correct_size() {
        let at = at_with_style("stripes");
        let buf = generate_from_config(64, 64, &at);
        assert_eq!(buf.len(), 64 * 64 * 4);
    }

    #[test]
    fn stripes_all_opaque() {
        let at = at_with_style("stripes");
        let buf = generate_from_config(32, 32, &at);
        for i in (3..buf.len()).step_by(4) {
            assert_eq!(buf[i], 255);
        }
    }

    #[test]
    fn checkerboard_correct_size() {
        let at = at_with_style("checkerboard");
        let buf = generate_from_config(64, 64, &at);
        assert_eq!(buf.len(), 64 * 64 * 4);
    }

    #[test]
    fn checkerboard_alternates() {
        let mut at = at_with_style("checkerboard");
        at.wallpaper.grid_spacing = 16;
        at.wallpaper.stops = vec![
            crate::backend::Color::rgb(255, 0, 0),
            crate::backend::Color::rgb(0, 0, 255),
        ];
        let buf = generate_from_config(32, 32, &at);
        // Cell (0,0) should be color 0, cell (1,0) should be color 1.
        let p00 = (buf[0], buf[1], buf[2]); // x=0,y=0
        let p16 = (
            (16 * 4) as usize,
            (16 * 4 + 1) as usize,
            (16 * 4 + 2) as usize,
        );
        let p16_val = (buf[p16.0], buf[p16.1], buf[p16.2]); // x=16,y=0
        assert_ne!(p00, p16_val);
    }

    #[test]
    fn config_phase_changes_output() {
        let at = ActiveTheme::default(); // "gradient" with wave
        let buf0 = generate_from_config_with_phase(32, 32, &at, 0.0);
        let buf1 = generate_from_config_with_phase(32, 32, &at, 1.0);
        // With phase change, at least some pixels should differ.
        assert_ne!(buf0, buf1);
    }
}
