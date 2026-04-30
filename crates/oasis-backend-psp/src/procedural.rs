//! Procedural generation: wallpaper gradients and cursor sprites.

/// Width of the procedural cursor sprite.
pub const CURSOR_W: u32 = 8;
/// Height of the procedural cursor sprite.
pub const CURSOR_H: u32 = 12;

/// Reduced-resolution wallpaper dimensions for GE-scaled blitting.
///
/// A 64x64 texture (16KB RGBA) is scaled up to 480x272 by the GE with
/// bilinear filtering. This avoids the 1MB uncached RAM read that a
/// full-resolution wallpaper requires, which was the #1 performance
/// bottleneck (~41ms/frame GE stall).
pub const WALLPAPER_TEX_W: u32 = 64;
pub const WALLPAPER_TEX_H: u32 = 64;

/// Five RGB anchors describing a PSIX-style horizontal gradient sweep.
///
/// The original PSIX wallpaper interpolates orange → bright orange →
/// yellow → yellow-green → lime → light green over the X axis. Other
/// themes pass their own 5-stop palette to recolor the same shape
/// (vertical brightness ramp + lower-left wave arcs are preserved).
pub type GradientStops = [(u8, u8, u8); 5];

/// PSIX original orange→lime gradient.
pub const GRADIENT_PSIX: GradientStops = [
    (245, 110, 15),
    (255, 170, 15),
    (255, 230, 30),
    (230, 245, 40),
    (140, 235, 50),
];

/// Generate a PSIX-style gradient wallpaper as RGBA bytes (defaults to
/// the original orange-to-lime palette). Backwards-compat wrapper around
/// [`generate_gradient_with`].
pub fn generate_gradient(w: u32, h: u32) -> Vec<u8> {
    generate_gradient_with(w, h, &GRADIENT_PSIX)
}

/// Generate a PSIX-shaped wallpaper with a custom 5-stop color palette.
///
/// Same horizontal sweep + vertical brightness + lower-left wave arcs as
/// the original PSIX gradient, just colored differently. Used by the PSP
/// theme system so each non-shader skin gets a recognisable wallpaper.
pub fn generate_gradient_with(w: u32, h: u32, stops: &GradientStops) -> Vec<u8> {
    let mut buf = vec![0u8; (w * h * 4) as usize];

    for y in 0..h {
        for x in 0..w {
            let offset = ((y * w + x) * 4) as usize;

            let nx = x as f32 / w as f32;
            let ny = y as f32 / h as f32;

            // Horizontal sweep using the caller's 5-stop palette. The PSP
            // gradient breakpoints are the same as the original PSIX
            // wallpaper so existing themes look unchanged when wired up.
            let t = nx * 0.88 + ny * 0.12;

            let (r, g, b) = if t < 0.15 {
                let s = t / 0.15;
                lerp_rgb(stops[0], stops[1], s)
            } else if t < 0.32 {
                let s = (t - 0.15) / 0.17;
                lerp_rgb(stops[1], stops[2], s)
            } else if t < 0.48 {
                let s = (t - 0.32) / 0.16;
                lerp_rgb(stops[2], stops[3], s)
            } else if t < 0.65 {
                let s = (t - 0.48) / 0.17;
                lerp_rgb(stops[3], stops[4], s)
            } else {
                // Past the last anchor: hold the final color.
                stops[4]
            };

            // Vertical brightness: lighter toward top, darker at bottom.
            let vert = 1.0 + (0.5 - ny) * 0.18;

            // Wave arcs from lower-left (characteristic PSIX pattern).
            let dx = nx + 0.05;
            let dy = ny - 1.3;
            let dist = libm::sqrtf(dx * dx + dy * dy);
            let arc1 = libm::sinf(dist * 12.0) * 0.18;
            let arc2 = libm::sinf(dist * 22.0 + 1.2) * 0.09;
            let arc3 = libm::sinf(dist * 36.0 + nx * 2.5) * 0.04;

            // Arcs fade toward the right.
            let arc_fade = (1.0 - nx * 0.45).clamp(0.0, 1.0);
            let wave = 1.0 + (arc1 + arc2 + arc3) * arc_fade;

            let scale = vert * wave;
            buf[offset] = (r as f32 * scale).clamp(0.0, 255.0) as u8;
            buf[offset + 1] = (g as f32 * scale).clamp(0.0, 255.0) as u8;
            buf[offset + 2] = (b as f32 * scale).clamp(0.0, 255.0) as u8;
            buf[offset + 3] = 255;
        }
    }

    buf
}

/// Linear interpolation between two RGB colors.
fn lerp_rgb(a: (u8, u8, u8), b: (u8, u8, u8), t: f32) -> (u8, u8, u8) {
    let r = a.0 as f32 + (b.0 as f32 - a.0 as f32) * t;
    let g = a.1 as f32 + (b.1 as f32 - a.1 as f32) * t;
    let bv = a.2 as f32 + (b.2 as f32 - a.2 as f32) * t;
    (r as u8, g as u8, bv as u8)
}

/// Generate a white arrow cursor with black outline as RGBA pixels.
pub fn generate_cursor_pixels() -> Vec<u8> {
    // 8x12 arrow cursor bitmap: 1 = white fill, 2 = black outline, 0 = transparent.
    #[rustfmt::skip]
    let bitmap: [[u8; 8]; 12] = [
        [2,0,0,0,0,0,0,0],
        [2,2,0,0,0,0,0,0],
        [2,1,2,0,0,0,0,0],
        [2,1,1,2,0,0,0,0],
        [2,1,1,1,2,0,0,0],
        [2,1,1,1,1,2,0,0],
        [2,1,1,1,1,1,2,0],
        [2,1,1,1,2,2,2,0],
        [2,1,2,1,2,0,0,0],
        [2,2,0,2,1,2,0,0],
        [2,0,0,0,2,1,2,0],
        [0,0,0,0,0,2,0,0],
    ];
    let mut data = vec![0u8; (CURSOR_W * CURSOR_H * 4) as usize];
    for (y, row) in bitmap.iter().enumerate() {
        for (x, &val) in row.iter().enumerate() {
            let offset = (y * CURSOR_W as usize + x) * 4;
            match val {
                1 => {
                    data[offset] = 255;
                    data[offset + 1] = 255;
                    data[offset + 2] = 255;
                    data[offset + 3] = 255;
                },
                2 => {
                    data[offset] = 0;
                    data[offset + 1] = 0;
                    data[offset + 2] = 0;
                    data[offset + 3] = 255;
                },
                _ => {}, // transparent (alpha stays 0)
            }
        }
    }
    data
}
