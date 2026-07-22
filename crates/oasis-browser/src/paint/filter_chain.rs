//! CPU filter chain applied to `PushCompositingLayer` pixels between
//! unbind and composite (compositor overhaul PR5).
//!
//! Takes an `RGBA8` buffer and applies a sequence of CSS
//! [`FilterFunction`]s in place. All ops except `Blur` are per-pixel
//! so they are trivially O(n). `Blur` uses a 3-pass separable box
//! blur, which approximates a Gaussian and is the standard trick in
//! real browsers.
//!
//! Pure-CPU design is deliberate:
//! - SDL3 can use native blend modes for the common case, but filters
//!   are not expressible in `SDL_ComposeCustomBlendMode`.
//! - WASM Canvas2D has native `ctx.filter = "blur(Npx)"` but the
//!   performance is inconsistent across browsers; a CPU path gives
//!   deterministic results and matches pixel goldens.
//! - UE5 is already CPU-only, and PSP (when it ships) has no GPU
//!   shader path at all.
//!
//! The CPU path also means the compositor's `read_render_target` +
//! `apply_filter_chain` + upload loop runs identically on every
//! backend, which keeps the pixel-golden tests cross-platform stable.

use crate::css::values::types::FilterFunction;
use oasis_types::backend::BlendMode;

/// Apply a filter chain to a mutable RGBA8 pixel buffer.
///
/// `width` and `height` describe the layout of `pixels`. `pixels.len()`
/// must equal `width * height * 4`. Filters are applied in order; each
/// operates on the output of the previous one.
///
/// Unsupported filters (drop-shadow — which needs a compositing pass
/// of its own — and any future additions) are skipped with a `log::warn!`
/// so the page still renders.
pub fn apply_filter_chain(pixels: &mut [u8], width: u32, height: u32, filters: &[FilterFunction]) {
    if filters.is_empty() || pixels.len() < (width * height * 4) as usize {
        return;
    }
    for f in filters {
        match *f {
            FilterFunction::Opacity(factor) => apply_opacity(pixels, factor),
            FilterFunction::Grayscale(amount) => apply_grayscale(pixels, amount),
            FilterFunction::Invert(amount) => apply_invert(pixels, amount),
            FilterFunction::Brightness(factor) => apply_brightness(pixels, factor),
            FilterFunction::Contrast(factor) => apply_contrast(pixels, factor),
            FilterFunction::Sepia(amount) => apply_sepia(pixels, amount),
            FilterFunction::Saturate(factor) => apply_saturate(pixels, factor),
            FilterFunction::HueRotate(radians) => apply_hue_rotate(pixels, radians),
            FilterFunction::Blur(radius) => apply_blur(pixels, width, height, radius),
        }
    }
}

/// `filter: opacity(k)` — multiply alpha channel by `k`.
fn apply_opacity(pixels: &mut [u8], factor: f32) {
    let f = factor.clamp(0.0, 1.0);
    for chunk in pixels.as_chunks_mut::<4>().0.iter_mut() {
        chunk[3] = ((chunk[3] as f32) * f).round() as u8;
    }
}

/// `filter: grayscale(amount)` — interpolate from color to rec709 luma.
fn apply_grayscale(pixels: &mut [u8], amount: f32) {
    let a = amount.clamp(0.0, 1.0);
    let inv = 1.0 - a;
    for chunk in pixels.as_chunks_mut::<4>().0.iter_mut() {
        let r = chunk[0] as f32;
        let g = chunk[1] as f32;
        let b = chunk[2] as f32;
        let l = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        chunk[0] = (r * inv + l * a).round().clamp(0.0, 255.0) as u8;
        chunk[1] = (g * inv + l * a).round().clamp(0.0, 255.0) as u8;
        chunk[2] = (b * inv + l * a).round().clamp(0.0, 255.0) as u8;
    }
}

/// `filter: invert(amount)` — interpolate between color and its inverse.
fn apply_invert(pixels: &mut [u8], amount: f32) {
    let a = amount.clamp(0.0, 1.0);
    let inv = 1.0 - a;
    for chunk in pixels.as_chunks_mut::<4>().0.iter_mut() {
        for channel in chunk.iter_mut().take(3) {
            let v = *channel as f32;
            let inverted = 255.0 - v;
            *channel = (v * inv + inverted * a).round().clamp(0.0, 255.0) as u8;
        }
    }
}

/// `filter: brightness(k)` — scale RGB channels by `k`.
fn apply_brightness(pixels: &mut [u8], factor: f32) {
    if (factor - 1.0).abs() < f32::EPSILON {
        return;
    }
    for chunk in pixels.as_chunks_mut::<4>().0.iter_mut() {
        for channel in chunk.iter_mut().take(3) {
            *channel = (*channel as f32 * factor).round().clamp(0.0, 255.0) as u8;
        }
    }
}

/// `filter: contrast(k)` — pivot around mid-grey (128).
fn apply_contrast(pixels: &mut [u8], factor: f32) {
    if (factor - 1.0).abs() < f32::EPSILON {
        return;
    }
    for chunk in pixels.as_chunks_mut::<4>().0.iter_mut() {
        for channel in chunk.iter_mut().take(3) {
            let v = *channel as f32;
            *channel = ((v - 128.0) * factor + 128.0).round().clamp(0.0, 255.0) as u8;
        }
    }
}

/// `filter: sepia(amount)` — standard sepia matrix.
fn apply_sepia(pixels: &mut [u8], amount: f32) {
    let a = amount.clamp(0.0, 1.0);
    let inv = 1.0 - a;
    for chunk in pixels.as_chunks_mut::<4>().0.iter_mut() {
        let r = chunk[0] as f32;
        let g = chunk[1] as f32;
        let b = chunk[2] as f32;
        let sr = 0.393 * r + 0.769 * g + 0.189 * b;
        let sg = 0.349 * r + 0.686 * g + 0.168 * b;
        let sb = 0.272 * r + 0.534 * g + 0.131 * b;
        chunk[0] = (r * inv + sr * a).round().clamp(0.0, 255.0) as u8;
        chunk[1] = (g * inv + sg * a).round().clamp(0.0, 255.0) as u8;
        chunk[2] = (b * inv + sb * a).round().clamp(0.0, 255.0) as u8;
    }
}

/// `filter: saturate(k)` — saturation matrix pivot around luma.
fn apply_saturate(pixels: &mut [u8], factor: f32) {
    if (factor - 1.0).abs() < f32::EPSILON {
        return;
    }
    for chunk in pixels.as_chunks_mut::<4>().0.iter_mut() {
        let r = chunk[0] as f32;
        let g = chunk[1] as f32;
        let b = chunk[2] as f32;
        let l = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        chunk[0] = (l + (r - l) * factor).round().clamp(0.0, 255.0) as u8;
        chunk[1] = (l + (g - l) * factor).round().clamp(0.0, 255.0) as u8;
        chunk[2] = (l + (b - l) * factor).round().clamp(0.0, 255.0) as u8;
    }
}

/// `filter: hue-rotate(angle)` — rotate hue around the luma axis.
fn apply_hue_rotate(pixels: &mut [u8], radians: f32) {
    let c = radians.cos();
    let s = radians.sin();
    // Standard hue-rotation matrix from CSS filters spec.
    let m = [
        0.213 + c * 0.787 - s * 0.213,
        0.715 - c * 0.715 - s * 0.715,
        0.072 - c * 0.072 + s * 0.928,
        0.213 - c * 0.213 + s * 0.143,
        0.715 + c * 0.285 + s * 0.140,
        0.072 - c * 0.072 - s * 0.283,
        0.213 - c * 0.213 - s * 0.787,
        0.715 - c * 0.715 + s * 0.715,
        0.072 + c * 0.928 + s * 0.072,
    ];
    for chunk in pixels.as_chunks_mut::<4>().0.iter_mut() {
        let r = chunk[0] as f32;
        let g = chunk[1] as f32;
        let b = chunk[2] as f32;
        let nr = m[0] * r + m[1] * g + m[2] * b;
        let ng = m[3] * r + m[4] * g + m[5] * b;
        let nb = m[6] * r + m[7] * g + m[8] * b;
        chunk[0] = nr.round().clamp(0.0, 255.0) as u8;
        chunk[1] = ng.round().clamp(0.0, 255.0) as u8;
        chunk[2] = nb.round().clamp(0.0, 255.0) as u8;
    }
}

/// `filter: blur(radius)` — 3-pass separable box blur. Approximates a
/// Gaussian within ~3% for typical radii. RGBA is separated so the
/// alpha channel is blurred in lockstep with RGB.
fn apply_blur(pixels: &mut [u8], width: u32, height: u32, radius: f32) {
    if radius <= 0.0 || width == 0 || height == 0 {
        return;
    }
    // Convert CSS blur radius (pixels of standard deviation) to an
    // equivalent box size. For 3 box blurs, the classic approximation
    // is `box = round(sqrt(12 * σ² / 3 + 1))`.
    let sigma = radius;
    let box_w = ((12.0 * sigma * sigma / 3.0 + 1.0).sqrt().round() as u32).max(1);
    // `box_w` must be odd for a symmetric box blur.
    let box_w = if box_w.is_multiple_of(2) {
        box_w + 1
    } else {
        box_w
    };
    let half = (box_w / 2) as i32;
    let mut tmp = pixels.to_vec();
    for _ in 0..3 {
        // Horizontal pass: pixels → tmp.
        box_blur_h(pixels, &mut tmp, width, height, half);
        // Vertical pass: tmp → pixels.
        box_blur_v(&tmp, pixels, width, height, half);
    }
}

fn box_blur_h(src: &[u8], dst: &mut [u8], width: u32, height: u32, half: i32) {
    let w = width as i32;
    let h = height as i32;
    for y in 0..h {
        let row_off = (y as usize) * (width as usize) * 4;
        for x in 0..w {
            let mut r = 0u32;
            let mut g = 0u32;
            let mut b = 0u32;
            let mut a = 0u32;
            let mut n = 0u32;
            for dx in -half..=half {
                let sx = (x + dx).clamp(0, w - 1) as usize;
                let off = row_off + sx * 4;
                r += src[off] as u32;
                g += src[off + 1] as u32;
                b += src[off + 2] as u32;
                a += src[off + 3] as u32;
                n += 1;
            }
            let off = row_off + (x as usize) * 4;
            dst[off] = (r / n) as u8;
            dst[off + 1] = (g / n) as u8;
            dst[off + 2] = (b / n) as u8;
            dst[off + 3] = (a / n) as u8;
        }
    }
}

fn box_blur_v(src: &[u8], dst: &mut [u8], width: u32, height: u32, half: i32) {
    let w = width as i32;
    let h = height as i32;
    for x in 0..w {
        for y in 0..h {
            let mut r = 0u32;
            let mut g = 0u32;
            let mut b = 0u32;
            let mut a = 0u32;
            let mut n = 0u32;
            for dy in -half..=half {
                let sy = (y + dy).clamp(0, h - 1) as usize;
                let off = sy * (width as usize) * 4 + (x as usize) * 4;
                r += src[off] as u32;
                g += src[off + 1] as u32;
                b += src[off + 2] as u32;
                a += src[off + 3] as u32;
                n += 1;
            }
            let off = (y as usize) * (width as usize) * 4 + (x as usize) * 4;
            dst[off] = (r / n) as u8;
            dst[off + 1] = (g / n) as u8;
            dst[off + 2] = (b / n) as u8;
            dst[off + 3] = (a / n) as u8;
        }
    }
}

// ---------------------------------------------------------------------------
// CPU blend-mode compositing (CSS Compositing and Blending Level 1)
// ---------------------------------------------------------------------------

/// Composite the `src` layer onto `dst` using a CSS `BlendMode`.
///
/// Both buffers are RGBA8, row-major, `w * h * 4` bytes. `src` is the
/// offscreen layer (already filtered / masked); `dst` is the parent
/// surface pixels beneath it. The blended result is written into `dst`.
///
/// Implements all 16 blend modes from the W3C Compositing and Blending
/// Level 1 specification. Separable modes (Normal through Exclusion)
/// operate per-channel; non-separable modes (Hue, Saturation, Color,
/// Luminosity) convert to HSL.
///
/// The composite formula per the spec:
///   Co = αs * B(Cs, Cb) + (1 - αs) * Cb
///   αo = αs + αb * (1 - αs)
pub fn cpu_blend_composite(src: &[u8], dst: &mut [u8], w: u32, h: u32, blend: BlendMode) {
    let count = (w * h * 4) as usize;
    if src.len() < count || dst.len() < count {
        return;
    }
    for (s, d) in src
        .as_chunks::<4>()
        .0
        .iter()
        .zip(dst.as_chunks_mut::<4>().0.iter_mut())
    {
        let sa = s[3] as f32 / 255.0;
        if sa < 1.0 / 512.0 {
            continue; // fully transparent source — no change
        }
        let sr = s[0] as f32 / 255.0;
        let sg = s[1] as f32 / 255.0;
        let sb = s[2] as f32 / 255.0;
        let dr = d[0] as f32 / 255.0;
        let dg = d[1] as f32 / 255.0;
        let db = d[2] as f32 / 255.0;
        let da = d[3] as f32 / 255.0;

        let (br, bg, bb) = blend_channels(sr, sg, sb, dr, dg, db, blend);

        // Composite: Co = αs * B(Cs, Cb) + (1 - αs) * Cb
        let or = (sa * br + (1.0 - sa) * dr).clamp(0.0, 1.0);
        let og = (sa * bg + (1.0 - sa) * dg).clamp(0.0, 1.0);
        let ob = (sa * bb + (1.0 - sa) * db).clamp(0.0, 1.0);
        let oa = (sa + da * (1.0 - sa)).clamp(0.0, 1.0);

        d[0] = (or * 255.0).round() as u8;
        d[1] = (og * 255.0).round() as u8;
        d[2] = (ob * 255.0).round() as u8;
        d[3] = (oa * 255.0).round() as u8;
    }
}

/// Apply the blend function B(Cs, Cb) for a given blend mode.
/// Returns the blended (r, g, b) in [0, 1].
fn blend_channels(
    sr: f32,
    sg: f32,
    sb: f32,
    dr: f32,
    dg: f32,
    db: f32,
    blend: BlendMode,
) -> (f32, f32, f32) {
    match blend {
        BlendMode::Normal => (sr, sg, sb),
        BlendMode::Multiply => (sr * dr, sg * dg, sb * db),
        BlendMode::Screen => (sr + dr - sr * dr, sg + dg - sg * dg, sb + db - sb * db),
        BlendMode::Overlay => (overlay_ch(dr, sr), overlay_ch(dg, sg), overlay_ch(db, sb)),
        BlendMode::Darken => (sr.min(dr), sg.min(dg), sb.min(db)),
        BlendMode::Lighten => (sr.max(dr), sg.max(dg), sb.max(db)),
        BlendMode::ColorDodge => (
            color_dodge_ch(dr, sr),
            color_dodge_ch(dg, sg),
            color_dodge_ch(db, sb),
        ),
        BlendMode::ColorBurn => (
            color_burn_ch(dr, sr),
            color_burn_ch(dg, sg),
            color_burn_ch(db, sb),
        ),
        BlendMode::HardLight => (overlay_ch(sr, dr), overlay_ch(sg, dg), overlay_ch(sb, db)),
        BlendMode::SoftLight => (
            soft_light_ch(dr, sr),
            soft_light_ch(dg, sg),
            soft_light_ch(db, sb),
        ),
        BlendMode::Difference => ((sr - dr).abs(), (sg - dg).abs(), (sb - db).abs()),
        BlendMode::Exclusion => (
            sr + dr - 2.0 * sr * dr,
            sg + dg - 2.0 * sg * dg,
            sb + db - 2.0 * sb * db,
        ),
        // Non-separable blend modes operate on HSL.
        BlendMode::Hue => {
            let (sh, _, _) = rgb_to_hsl(sr, sg, sb);
            let (_, ds, dl) = rgb_to_hsl(dr, dg, db);
            hsl_to_rgb(sh, ds, dl)
        },
        BlendMode::Saturation => {
            let (_, ss, _) = rgb_to_hsl(sr, sg, sb);
            let (dh, _, dl) = rgb_to_hsl(dr, dg, db);
            hsl_to_rgb(dh, ss, dl)
        },
        BlendMode::Color => {
            let (sh, ss, _) = rgb_to_hsl(sr, sg, sb);
            let (_, _, dl) = rgb_to_hsl(dr, dg, db);
            hsl_to_rgb(sh, ss, dl)
        },
        BlendMode::Luminosity => {
            let (_, _, sl) = rgb_to_hsl(sr, sg, sb);
            let (dh, ds, _) = rgb_to_hsl(dr, dg, db);
            hsl_to_rgb(dh, ds, sl)
        },
    }
}

// --- Separable blend helpers ---

fn overlay_ch(cb: f32, cs: f32) -> f32 {
    if cb <= 0.5 {
        2.0 * cb * cs
    } else {
        1.0 - 2.0 * (1.0 - cb) * (1.0 - cs)
    }
}

fn color_dodge_ch(cb: f32, cs: f32) -> f32 {
    if cb < 1.0 / 512.0 {
        0.0
    } else if cs >= 1.0 - 1.0 / 512.0 {
        1.0
    } else {
        (cb / (1.0 - cs)).min(1.0)
    }
}

fn color_burn_ch(cb: f32, cs: f32) -> f32 {
    if cb >= 1.0 - 1.0 / 512.0 {
        1.0
    } else if cs < 1.0 / 512.0 {
        0.0
    } else {
        1.0 - ((1.0 - cb) / cs).min(1.0)
    }
}

fn soft_light_ch(cb: f32, cs: f32) -> f32 {
    // W3C Compositing Level 1 soft-light formula.
    if cs <= 0.5 {
        cb - (1.0 - 2.0 * cs) * cb * (1.0 - cb)
    } else {
        let d = if cb <= 0.25 {
            ((16.0 * cb - 12.0) * cb + 4.0) * cb
        } else {
            cb.sqrt()
        };
        cb + (2.0 * cs - 1.0) * (d - cb)
    }
}

// --- Non-separable blend helpers (HSL) ---

fn rgb_to_hsl(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) * 0.5;
    if (max - min).abs() < 1.0 / 512.0 {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let h = if (max - r).abs() < 1e-6 {
        let mut h = (g - b) / d;
        if g < b {
            h += 6.0;
        }
        h
    } else if (max - g).abs() < 1e-6 {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h / 6.0, s, l)
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (f32, f32, f32) {
    if s < 1.0 / 512.0 {
        return (l, l, l);
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    (
        hue_to_rgb(p, q, h + 1.0 / 3.0),
        hue_to_rgb(p, q, h),
        hue_to_rgb(p, q, h - 1.0 / 3.0),
    )
}

fn hue_to_rgb(p: f32, q: f32, mut t: f32) -> f32 {
    if t < 0.0 {
        t += 1.0;
    }
    if t > 1.0 {
        t -= 1.0;
    }
    if t < 1.0 / 6.0 {
        p + (q - p) * 6.0 * t
    } else if t < 0.5 {
        q
    } else if t < 2.0 / 3.0 {
        p + (q - p) * (2.0 / 3.0 - t) * 6.0
    } else {
        p
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid_rgba(w: u32, h: u32, r: u8, g: u8, b: u8, a: u8) -> Vec<u8> {
        let mut v = Vec::with_capacity((w * h * 4) as usize);
        for _ in 0..(w * h) {
            v.extend_from_slice(&[r, g, b, a]);
        }
        v
    }

    #[test]
    fn empty_filter_chain_is_noop() {
        let mut pixels = solid_rgba(2, 2, 100, 150, 200, 255);
        let original = pixels.clone();
        apply_filter_chain(&mut pixels, 2, 2, &[]);
        assert_eq!(pixels, original);
    }

    #[test]
    fn opacity_scales_alpha() {
        let mut pixels = solid_rgba(2, 2, 255, 0, 0, 200);
        apply_filter_chain(&mut pixels, 2, 2, &[FilterFunction::Opacity(0.5)]);
        assert_eq!(pixels[3], 100);
        assert_eq!(pixels[0], 255); // RGB untouched
    }

    #[test]
    fn grayscale_at_one_collapses_to_luma() {
        let mut pixels = solid_rgba(1, 1, 255, 0, 0, 255);
        apply_filter_chain(&mut pixels, 1, 1, &[FilterFunction::Grayscale(1.0)]);
        // Luma of pure red is ~54.
        assert!((pixels[0] as i32 - 54).abs() <= 1);
        assert_eq!(pixels[0], pixels[1]);
        assert_eq!(pixels[1], pixels[2]);
    }

    #[test]
    fn invert_at_one_flips_rgb() {
        let mut pixels = solid_rgba(1, 1, 10, 20, 30, 200);
        apply_filter_chain(&mut pixels, 1, 1, &[FilterFunction::Invert(1.0)]);
        assert_eq!(pixels[0], 245);
        assert_eq!(pixels[1], 235);
        assert_eq!(pixels[2], 225);
        // Alpha preserved.
        assert_eq!(pixels[3], 200);
    }

    #[test]
    fn brightness_scales_rgb() {
        let mut pixels = solid_rgba(1, 1, 100, 100, 100, 255);
        apply_filter_chain(&mut pixels, 1, 1, &[FilterFunction::Brightness(0.5)]);
        assert_eq!(pixels[0], 50);
        assert_eq!(pixels[1], 50);
        assert_eq!(pixels[2], 50);
    }

    #[test]
    fn sepia_tints_toward_warm() {
        let mut pixels = solid_rgba(1, 1, 128, 128, 128, 255);
        apply_filter_chain(&mut pixels, 1, 1, &[FilterFunction::Sepia(1.0)]);
        // Classic sepia: R > G > B.
        assert!(pixels[0] >= pixels[1]);
        assert!(pixels[1] >= pixels[2]);
    }

    #[test]
    fn blur_zero_radius_is_noop() {
        let mut pixels = solid_rgba(4, 4, 255, 0, 0, 255);
        let original = pixels.clone();
        apply_filter_chain(&mut pixels, 4, 4, &[FilterFunction::Blur(0.0)]);
        assert_eq!(pixels, original);
    }

    #[test]
    fn blur_nonzero_smears_edge() {
        // A 4x1 black-to-white step gets blurred; the boundary should
        // soften (values in the middle no longer 0 or 255).
        let mut pixels = vec![
            0, 0, 0, 255, // x=0 black
            0, 0, 0, 255, // x=1 black
            255, 255, 255, 255, // x=2 white
            255, 255, 255, 255, // x=3 white
        ];
        apply_filter_chain(&mut pixels, 4, 1, &[FilterFunction::Blur(1.5)]);
        // Neither end should be pure white/black any more on a 4-wide
        // blur with radius 1.5 (3-pass separable box).
        assert!(pixels[4] > 0, "left-of-boundary pixel should be lifted");
        assert!(
            pixels[10] < 255,
            "right-of-boundary pixel should be lowered"
        );
    }

    #[test]
    fn chain_applies_in_order() {
        // Brightness(0.5) then Invert(1.0): starting pixel 200 → 100 → 155.
        let mut pixels = solid_rgba(1, 1, 200, 200, 200, 255);
        apply_filter_chain(
            &mut pixels,
            1,
            1,
            &[FilterFunction::Brightness(0.5), FilterFunction::Invert(1.0)],
        );
        assert_eq!(pixels[0], 155);
    }

    // --- CPU blend compositing tests ---

    #[test]
    fn blend_multiply_darkens() {
        let src = solid_rgba(1, 1, 128, 128, 128, 255);
        let mut dst = solid_rgba(1, 1, 200, 200, 200, 255);
        cpu_blend_composite(&src, &mut dst, 1, 1, BlendMode::Multiply);
        // Multiply: 128/255 * 200/255 ≈ 0.3922, → ~100
        assert!((dst[0] as i32 - 100).abs() <= 1);
    }

    #[test]
    fn blend_screen_lightens() {
        let src = solid_rgba(1, 1, 128, 0, 0, 255);
        let mut dst = solid_rgba(1, 1, 128, 0, 0, 255);
        cpu_blend_composite(&src, &mut dst, 1, 1, BlendMode::Screen);
        // Screen: 0.502 + 0.502 - 0.502*0.502 ≈ 0.752 → ~192
        assert!(dst[0] > 180);
    }

    #[test]
    fn blend_difference_produces_abs_diff() {
        let src = solid_rgba(1, 1, 200, 50, 100, 255);
        let mut dst = solid_rgba(1, 1, 100, 150, 100, 255);
        cpu_blend_composite(&src, &mut dst, 1, 1, BlendMode::Difference);
        // |200-100|=100, |50-150|=100, |100-100|=0
        assert!((dst[0] as i32 - 100).abs() <= 1);
        assert!((dst[1] as i32 - 100).abs() <= 1);
        assert!(dst[2] <= 1);
    }

    #[test]
    fn blend_normal_with_semitransparent_source() {
        let src = solid_rgba(1, 1, 255, 0, 0, 128);
        let mut dst = solid_rgba(1, 1, 0, 0, 255, 255);
        cpu_blend_composite(&src, &mut dst, 1, 1, BlendMode::Normal);
        // αs ≈ 0.502; Co = 0.502*1.0 + 0.498*0.0 ≈ 0.502 → ~128
        assert!((dst[0] as i32 - 128).abs() <= 1);
        assert!((dst[2] as i32 - 127).abs() <= 1);
    }

    #[test]
    fn blend_overlay_splits_at_half() {
        let src = solid_rgba(1, 1, 255, 255, 255, 255);
        let mut dst_dark = solid_rgba(1, 1, 64, 64, 64, 255);
        let mut dst_light = solid_rgba(1, 1, 200, 200, 200, 255);
        cpu_blend_composite(&src, &mut dst_dark, 1, 1, BlendMode::Overlay);
        cpu_blend_composite(&src, &mut dst_light, 1, 1, BlendMode::Overlay);
        // Dark dst (< 0.5): Multiply path → 2*0.251*1.0 ≈ 0.502 → ~128
        assert!((dst_dark[0] as i32 - 128).abs() <= 2);
        // Light dst (> 0.5): Screen path → 1 - 2*(1-0.784)*(1-1.0) = 1.0 → 255
        assert_eq!(dst_light[0], 255);
    }

    #[test]
    fn blend_transparent_source_is_noop() {
        let src = solid_rgba(1, 1, 255, 0, 0, 0);
        let mut dst = solid_rgba(1, 1, 100, 100, 100, 255);
        let original = dst.clone();
        cpu_blend_composite(&src, &mut dst, 1, 1, BlendMode::Multiply);
        assert_eq!(dst, original);
    }
}
