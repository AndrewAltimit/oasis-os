//! Procedural generation: wallpaper gradients and cursor sprites.

/// Width of the procedural cursor sprite.
pub const CURSOR_W: u32 = 12;
/// Height of the procedural cursor sprite.
pub const CURSOR_H: u32 = 18;

/// Generate a PSIX-style gradient wallpaper as RGBA bytes.
///
/// Produces the characteristic orange-to-green sweep with wave arcs emanating
/// from the lower-left, matching `oasis-core`'s `wallpaper::generate_gradient`.
pub fn generate_gradient(w: u32, h: u32) -> Vec<u8> {
    let mut buf = vec![0u8; (w * h * 4) as usize];

    for y in 0..h {
        for x in 0..w {
            let offset = ((y * w + x) * 4) as usize;

            let nx = x as f32 / w as f32;
            let ny = y as f32 / h as f32;

            // Horizontal sweep: hot orange (left) -> bright lime green (right).
            let t = nx * 0.88 + ny * 0.12;

            let (r, g, b) = if t < 0.15 {
                let s = t / 0.15;
                lerp_rgb((245, 110, 15), (255, 170, 15), s)
            } else if t < 0.32 {
                let s = (t - 0.15) / 0.17;
                lerp_rgb((255, 170, 15), (255, 230, 30), s)
            } else if t < 0.48 {
                let s = (t - 0.32) / 0.16;
                lerp_rgb((255, 230, 30), (230, 245, 40), s)
            } else if t < 0.65 {
                let s = (t - 0.48) / 0.17;
                lerp_rgb((230, 245, 40), (140, 235, 50), s)
            } else {
                let s = (t - 0.65) / 0.35;
                lerp_rgb((140, 235, 50), (200, 252, 130), s)
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
    // 12x18 arrow cursor bitmap: 1 = white fill, 2 = black outline, 0 = transparent.
    #[rustfmt::skip]
    let bitmap: [[u8; 12]; 18] = [
        [2,0,0,0,0,0,0,0,0,0,0,0],
        [2,2,0,0,0,0,0,0,0,0,0,0],
        [2,1,2,0,0,0,0,0,0,0,0,0],
        [2,1,1,2,0,0,0,0,0,0,0,0],
        [2,1,1,1,2,0,0,0,0,0,0,0],
        [2,1,1,1,1,2,0,0,0,0,0,0],
        [2,1,1,1,1,1,2,0,0,0,0,0],
        [2,1,1,1,1,1,1,2,0,0,0,0],
        [2,1,1,1,1,1,1,1,2,0,0,0],
        [2,1,1,1,1,1,1,1,1,2,0,0],
        [2,1,1,1,1,1,1,1,1,1,2,0],
        [2,1,1,1,1,1,2,2,2,2,2,0],
        [2,1,1,1,2,1,2,0,0,0,0,0],
        [2,1,1,2,0,2,1,2,0,0,0,0],
        [2,1,2,0,0,2,1,2,0,0,0,0],
        [2,2,0,0,0,0,2,1,2,0,0,0],
        [2,0,0,0,0,0,2,1,2,0,0,0],
        [0,0,0,0,0,0,0,2,0,0,0,0],
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
                }
                2 => {
                    data[offset] = 0;
                    data[offset + 1] = 0;
                    data[offset + 2] = 0;
                    data[offset + 3] = 255;
                }
                _ => {} // transparent (alpha stays 0)
            }
        }
    }
    data
}
