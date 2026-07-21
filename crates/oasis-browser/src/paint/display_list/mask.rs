//! CSS `mask-*` rasterization for compositor pop.
//!
//! Painted compositor layers can carry a mask source (URL, linear or
//! radial gradient). On `PopCompositingLayer` the replay path reads the
//! offscreen layer pixels, asks this module to rasterize the mask into
//! a 0..=255 alpha grid, then combines the two via [`apply_mask`]
//! according to the parsed `mask-mode` and `mask-composite` operators.

use std::sync::Arc;

use oasis_types::backend::Color;

use crate::css::values::types::{
    BackgroundImage, BackgroundPosition, BackgroundRepeat, BackgroundSize, MaskComposite, MaskMode,
};
use crate::image::DecodedImage;

/// Parameters for a CSS `mask-*` pass applied on `PopCompositingLayer`.
///
/// Captured at display-list recording time so the replay path can
/// rasterize the mask into an alpha buffer and combine it with the
/// layer pixels (via destination-in / mask-mode semantics) before the
/// layer is composited back into the parent surface.
///
/// The mask image itself is carried as a [`BackgroundImage`] so that
/// both URL and gradient forms flow through the same compositor path.
/// For URL sources, the decoded RGBA bytes are additionally attached
/// via `texture` — the recorder fills this in from
/// `LayoutBox::mask_image_data` so the replay path can sample the
/// image without a GPU round-trip. URL masks with no attached
/// texture (still loading, or decode failed) reduce to a no-op and
/// the layer composites unchanged.
#[derive(Debug, Clone)]
pub struct MaskParams {
    /// The mask source. `BackgroundImage::None` is never stored here
    /// (the recorder skips emitting `mask` in that case) — keeping the
    /// enum form lets us share parse/gradient paint plumbing.
    pub image: BackgroundImage,
    /// `mask-mode` — alpha / luminance / match-source.
    pub mode: MaskMode,
    /// `mask-composite` — single-layer semantics only today.
    pub composite: MaskComposite,
    /// Decoded pixel data for URL-backed masks. Only populated when
    /// `image` is `BackgroundImage::Url(_)` AND the decoder has
    /// produced a texture. Shared via `Arc` so recording cost is a
    /// pointer copy.
    pub texture: Option<Arc<DecodedImage>>,
    /// `mask-size` — sizing for URL-backed masks. Reuses the
    /// background-size vocabulary (auto / cover / contain / explicit).
    pub size: BackgroundSize,
    /// `mask-position` — placement within the layer bounds.
    pub position: BackgroundPosition,
    /// `mask-repeat` — tiling behavior for URL-backed masks.
    pub repeat: BackgroundRepeat,
}

impl PartialEq for MaskParams {
    fn eq(&self, other: &Self) -> bool {
        // `Arc<DecodedImage>` pointer equality is sufficient for the
        // display-list equality contract — two mask params from the
        // same record pass share the same underlying allocation.
        self.image == other.image
            && self.mode == other.mode
            && self.composite == other.composite
            && self.size == other.size
            && self.position == other.position
            && self.repeat == other.repeat
            && match (self.texture.as_ref(), other.texture.as_ref()) {
                (None, None) => true,
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                _ => false,
            }
    }
}

/// Rasterize a mask source into an alpha grid and apply it to a BGRA
/// layer buffer.
///
/// The mask is painted over the full layer rect; each pixel's mask
/// value is combined with the layer pixel's alpha according to
/// `mask.mode` (alpha / luminance / match-source) and `mask.composite`
/// (destination-in / destination-out / destination-xor). URL-backed
/// masks are sampled from the attached `Arc<DecodedImage>` via
/// [`rasterize_url_mask`] (nearest-neighbor, stretched to layer
/// bounds); gradient masks go through [`rasterize_linear_mask`] /
/// [`rasterize_radial_mask`]. If a URL mask's texture hasn't been
/// resolved yet (still fetching / decoding) the function early-
/// returns so the layer composites unchanged and a later frame can
/// pick up the mask once the pixels land.
///
/// The buffer layout matches `SdiBackend::load_texture`: RGBA8,
/// row-major, no padding.
pub(super) fn apply_mask(buf: &mut [u8], w: u32, h: u32, mask: &MaskParams) {
    if w == 0 || h == 0 || buf.len() < (w * h * 4) as usize {
        return;
    }
    // Rasterize the mask source into a per-pixel 0..=255 mask value.
    let mask_alpha: Vec<u8> = match &mask.image {
        BackgroundImage::None => return,
        BackgroundImage::Url(_) => match mask.texture.as_ref() {
            Some(tex) => rasterize_url_mask(
                w,
                h,
                tex,
                mask.mode,
                &mask.size,
                &mask.position,
                mask.repeat,
            ),
            // URL masks with no resolved texture — the fetch/decode
            // pipeline hasn't produced pixels yet (or the decode
            // failed). Composite unchanged; a later frame picks the
            // mask up once the image lands in `decoded_images`.
            None => return,
        },
        BackgroundImage::Gradient(grad) => rasterize_linear_mask(w, h, grad, mask.mode),
        BackgroundImage::RadialGradient(grad) => rasterize_radial_mask(w, h, grad, mask.mode),
    };
    // Combine per the `mask-composite` operator. The layer alpha `la`
    // starts as the already-painted layer's alpha; the mask alpha
    // `ma` comes from the rasterized buffer.
    for (pixel, ma) in buf
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .zip(mask_alpha.iter().copied())
    {
        let la = pixel[3] as u16;
        let ma = ma as u16;
        let out = match mask.composite {
            // `Add` and `Intersect` both collapse to destination-in
            // for a single mask layer. Per spec, `mask-composite:
            // add` is source-over *between mask layers*, and
            // `intersect` is destination-in between layers. With
            // only one mask layer there is no second layer for
            // `add` to be source-over against — the effective
            // operation is "apply this layer's mask to the content",
            // which is destination-in. The branch collapse is
            // correct for single-layer masks but must be un-
            // collapsed if/when multi-layer mask composition lands
            // (the existing `MaskComposite` variants carry the
            // information forward — the pop path just ignores the
            // distinction today).
            MaskComposite::Add | MaskComposite::Intersect => (la * ma) / 255,
            // Destination-out: keep layer where mask is 0.
            MaskComposite::Subtract => (la * (255 - ma)) / 255,
            // Alpha xor — keep the layer where exactly one of layer
            // and mask is opaque.
            MaskComposite::Exclude => {
                let a = (la * (255 - ma)) / 255;
                let b = ((255 - la) * ma) / 255;
                (a + b).min(255)
            },
        } as u8;
        pixel[3] = out;
    }
}

/// Rasterize a linear-gradient mask into a per-pixel 0..=255 grid.
///
/// The mask value at each pixel is derived from the gradient stop at
/// that pixel's parametric position along the gradient axis, converted
/// to a single 0..=255 channel via `mask.mode`:
///
/// - `MaskMode::Alpha` — use the stop color's alpha component.
/// - `MaskMode::Luminance` — use Rec.601 luminance of the RGB, scaled
///   by the stop color's alpha.
/// - `MaskMode::MatchSource` — alpha when the gradient has any
///   non-opaque stop, otherwise luminance.
pub(super) fn rasterize_linear_mask(
    w: u32,
    h: u32,
    grad: &crate::css::values::types::LinearGradient,
    mode: MaskMode,
) -> Vec<u8> {
    use crate::css::values::types::GradientDirection;
    let mut out = vec![0u8; (w * h) as usize];
    if grad.stops.is_empty() || w == 0 || h == 0 {
        return out;
    }
    // Axis unit vector (in pixel space, y-down).
    let angle_rad = match grad.direction {
        GradientDirection::Angle(deg) => deg.to_radians(),
        GradientDirection::ToTop => 0.0,
        GradientDirection::ToRight => 90.0_f32.to_radians(),
        GradientDirection::ToBottom => 180.0_f32.to_radians(),
        GradientDirection::ToLeft => 270.0_f32.to_radians(),
    };
    // CSS gradient angle: 0deg = to top, positive = clockwise.
    let dx = angle_rad.sin();
    let dy = -angle_rad.cos();
    // Project each corner onto the axis to find the gradient length.
    let fw = w as f32;
    let fh = h as f32;
    let corners = [(0.0, 0.0), (fw, 0.0), (0.0, fh), (fw, fh)];
    let mut min_proj = f32::INFINITY;
    let mut max_proj = f32::NEG_INFINITY;
    for (cx, cy) in corners {
        let p = cx * dx + cy * dy;
        if p < min_proj {
            min_proj = p;
        }
        if p > max_proj {
            max_proj = p;
        }
    }
    let len = (max_proj - min_proj).max(1.0);
    let match_alpha = grad.stops.iter().any(|s| s.color.a < 255);
    let use_alpha = match mode {
        MaskMode::Alpha => true,
        MaskMode::Luminance => false,
        MaskMode::MatchSource => match_alpha,
    };
    for y in 0..h {
        for x in 0..w {
            let proj = x as f32 * dx + y as f32 * dy;
            let raw = (proj - min_proj) / len;
            // `f32::fract()` preserves sign in Rust (unlike
            // `rem_euclid`), so a projection that lands at `raw =
            // -0.3` would give `fract = -0.3` and then clamp to 0 —
            // silently collapsing the negative wrap range to the
            // first stop. Use `rem_euclid(1.0)` so the repeat wraps
            // properly into `[0, 1)` regardless of sign.
            let t = if grad.repeating {
                raw.rem_euclid(1.0)
            } else {
                raw.clamp(0.0, 1.0)
            };
            let color = sample_gradient_stops(&grad.stops, t);
            out[(y * w + x) as usize] = color_to_mask_channel(color, use_alpha);
        }
    }
    out
}

/// Rasterize a radial-gradient mask into a per-pixel 0..=255 grid.
///
/// The radial center is fixed at the layer center (`RadialGradient`
/// doesn't parse an explicit `at <position>` today — see the browser
/// backlog). Radius defaults to CSS `farthest-corner`:
///
/// - **Circle**: radius is the distance from the center to the
///   farthest corner — `sqrt((w/2)^2 + (h/2)^2)`. The box center
///   is equidistant from all four corners so this is unambiguous.
/// - **Ellipse**: the axis-aligned ellipse passing through all four
///   corners has `rx = (w/2) * sqrt(2)`, `ry = (h/2) * sqrt(2)` —
///   plug `(w/2, h/2)` into `(x/rx)^2 + (y/ry)^2 = 1` and the sum
///   evaluates to `1/2 + 1/2 = 1` as required.
///
/// Earlier revisions used `closest-side` (`min(w, h) / 2` for
/// circles, `w/2`, `h/2` for ellipses), which terminated the
/// gradient before it reached the corners of non-square layers and
/// left those regions filled with the final stop color.
pub(super) fn rasterize_radial_mask(
    w: u32,
    h: u32,
    grad: &crate::css::values::types::RadialGradient,
    mode: MaskMode,
) -> Vec<u8> {
    let mut out = vec![0u8; (w * h) as usize];
    if grad.stops.is_empty() || w == 0 || h == 0 {
        return out;
    }
    let fw = w as f32;
    let fh = h as f32;
    let cx = fw * 0.5;
    let cy = fh * 0.5;
    let (rx, ry) = if grad.shape_circle {
        let r = ((fw * fw + fh * fh).sqrt() * 0.5).max(1.0);
        (r, r)
    } else {
        let sqrt2 = std::f32::consts::SQRT_2;
        ((fw * 0.5 * sqrt2).max(1.0), (fh * 0.5 * sqrt2).max(1.0))
    };
    let match_alpha = grad.stops.iter().any(|s| s.color.a < 255);
    let use_alpha = match mode {
        MaskMode::Alpha => true,
        MaskMode::Luminance => false,
        MaskMode::MatchSource => match_alpha,
    };
    for y in 0..h {
        for x in 0..w {
            let nx = (x as f32 - cx) / rx;
            let ny = (y as f32 - cy) / ry;
            let t = (nx * nx + ny * ny).sqrt().clamp(0.0, 1.0);
            let color = sample_gradient_stops(&grad.stops, t);
            out[(y * w + x) as usize] = color_to_mask_channel(color, use_alpha);
        }
    }
    out
}

/// Rasterize a URL-backed mask into a per-pixel 0..=255 grid.
///
/// Uses `background_image_tiles` to compute tile positions based on
/// `mask-size`, `mask-position`, and `mask-repeat`. Each tile is
/// sampled from the source image via nearest-neighbor scaling. Pixels
/// not covered by any tile remain at 0 (fully transparent mask).
///
/// `mask-mode: alpha` reads the source's alpha channel; `luminance`
/// converts RGB via Rec.601 scaled by alpha. `match-source` chooses
/// alpha when the source has any non-opaque pixel, else luminance.
pub(super) fn rasterize_url_mask(
    layer_w: u32,
    layer_h: u32,
    src: &DecodedImage,
    mode: MaskMode,
    size: &BackgroundSize,
    position: &BackgroundPosition,
    repeat: BackgroundRepeat,
) -> Vec<u8> {
    use super::super::background::background_image_tiles;

    let mut out = vec![0u8; (layer_w * layer_h) as usize];
    if src.width == 0 || src.height == 0 || src.pixels.len() < (src.width * src.height * 4) as usize
    {
        return out;
    }

    let use_alpha = match mode {
        MaskMode::Alpha => true,
        MaskMode::Luminance => false,
        MaskMode::MatchSource => src.has_transparency,
    };

    let tiles = background_image_tiles(
        0,
        0,
        layer_w,
        layer_h,
        (src.width, src.height),
        size,
        position,
        repeat,
    );

    for tile in &tiles {
        if tile.w == 0 || tile.h == 0 {
            continue;
        }
        // Skip tiles entirely outside the layer bounds.
        if tile.x + tile.w as i32 <= 0 || tile.y + tile.h as i32 <= 0 {
            continue;
        }
        if tile.x >= layer_w as i32 || tile.y >= layer_h as i32 {
            continue;
        }
        let sx_scale = src.width as f32 / tile.w as f32;
        let sy_scale = src.height as f32 / tile.h as f32;

        // Clamp tile to layer bounds.
        let x_start = tile.x.max(0) as u32;
        let y_start = tile.y.max(0) as u32;
        let x_end = (tile.x + tile.w as i32).max(0) as u32;
        let x_end = x_end.min(layer_w);
        let y_end = (tile.y + tile.h as i32).max(0) as u32;
        let y_end = y_end.min(layer_h);

        for y in y_start..y_end {
            let local_y = (y as i32 - tile.y) as f32;
            let sy = ((local_y + 0.5) * sy_scale) as u32;
            let sy = sy.min(src.height - 1);
            let row_start = (sy * src.width * 4) as usize;

            for x in x_start..x_end {
                let local_x = (x as i32 - tile.x) as f32;
                let sx = ((local_x + 0.5) * sx_scale) as u32;
                let sx = sx.min(src.width - 1);
                let i = row_start + (sx * 4) as usize;
                let r = src.pixels[i];
                let g = src.pixels[i + 1];
                let b = src.pixels[i + 2];
                let a = src.pixels[i + 3];
                let c = Color { r, g, b, a };
                out[(y * layer_w + x) as usize] = color_to_mask_channel(c, use_alpha);
            }
        }
    }
    out
}

/// Linear interpolation across a sorted stop list at parametric
/// position `t` (already clamped to `0..=1`).
fn sample_gradient_stops(stops: &[crate::css::values::types::GradientStop], t: f32) -> Color {
    if stops.is_empty() {
        return Color {
            r: 0,
            g: 0,
            b: 0,
            a: 0,
        };
    }
    if t <= stops[0].position {
        return stops[0].color;
    }
    if t >= stops[stops.len() - 1].position {
        return stops[stops.len() - 1].color;
    }
    for pair in stops.windows(2) {
        let a = &pair[0];
        let b = &pair[1];
        if t >= a.position && t <= b.position {
            let span = (b.position - a.position).max(f32::EPSILON);
            let u = (t - a.position) / span;
            return Color {
                r: lerp_u8(a.color.r, b.color.r, u),
                g: lerp_u8(a.color.g, b.color.g, u),
                b: lerp_u8(a.color.b, b.color.b, u),
                a: lerp_u8(a.color.a, b.color.a, u),
            };
        }
    }
    stops[stops.len() - 1].color
}

fn lerp_u8(a: u8, b: u8, t: f32) -> u8 {
    let af = a as f32;
    let bf = b as f32;
    (af + (bf - af) * t).round().clamp(0.0, 255.0) as u8
}

fn color_to_mask_channel(c: Color, use_alpha: bool) -> u8 {
    if use_alpha {
        c.a
    } else {
        // Rec.601 luma weighted by alpha.
        let luma = 0.299 * c.r as f32 + 0.587 * c.g as f32 + 0.114 * c.b as f32;
        ((luma * (c.a as f32 / 255.0)).round()).clamp(0.0, 255.0) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css::values::types::{
        BackgroundImage as CssBgImage, GradientDirection, GradientStop, LinearGradient,
        RadialGradient,
    };

    fn opaque_white_to_transparent() -> LinearGradient {
        LinearGradient {
            direction: GradientDirection::ToBottom,
            repeating: false,
            stops: vec![
                GradientStop {
                    color: Color::rgba(255, 255, 255, 255),
                    position: 0.0,
                },
                GradientStop {
                    color: Color::rgba(255, 255, 255, 0),
                    position: 1.0,
                },
            ],
        }
    }

    #[test]
    fn linear_mask_produces_monotonic_alpha_ramp() {
        let grad = opaque_white_to_transparent();
        let buf = rasterize_linear_mask(4, 8, &grad, MaskMode::Alpha);
        assert_eq!(buf.len(), 32);
        // Column 0 should fade from 255 at the top toward 0 at the
        // bottom. The bottom pixel doesn't reach exactly zero because
        // the gradient axis spans the full rect while pixel samples
        // hit the top-left corner of each cell — that's the same
        // behaviour as CSS `background: linear-gradient(...)`.
        let col = (0..8).map(|y| buf[(y * 4) as usize]).collect::<Vec<_>>();
        assert_eq!(col[0], 255, "top row opaque");
        assert!(col[7] < 64, "bottom row mostly transparent");
        for pair in col.windows(2) {
            assert!(pair[0] >= pair[1], "mask alpha must be monotonic");
        }
    }

    #[test]
    fn repeating_linear_mask_wraps_negative_projection() {
        // A gradient whose natural axis range doesn't line up with
        // the layer origin — the `raw` projection value at y=0 is
        // `(proj - min_proj) / len` which can be exactly zero but
        // neighboring rows can land on fractional positions. We
        // construct a contrived case by using `ToTop` (dy = -1) so
        // that `min_proj = -h` and row 0 projects to `raw = 1.0`
        // (i.e. the end of the first period). Without the old
        // `fract().clamp(0.0, 1.0)` bug, every row's `raw` lands in
        // a well-defined period and produces a monotonic sweep per
        // period.
        let grad = LinearGradient {
            direction: GradientDirection::ToTop,
            repeating: true,
            stops: vec![
                GradientStop {
                    color: Color::rgba(255, 255, 255, 255),
                    position: 0.0,
                },
                GradientStop {
                    color: Color::rgba(255, 255, 255, 0),
                    position: 1.0,
                },
            ],
        };
        // Any row should produce a valid 0..=255 value — the old
        // `fract() + clamp` path could silently collapse negative
        // wraparound to 0 for some orientations, which this test
        // would catch as "every pixel is 0".
        let buf = rasterize_linear_mask(1, 8, &grad, MaskMode::Alpha);
        assert_eq!(buf.len(), 8);
        assert!(
            buf.iter().any(|&v| v > 0),
            "repeating gradient should not collapse to all-zero under any axis"
        );
    }

    #[test]
    fn radial_mask_farthest_corner_covers_full_rect() {
        // Previously the rasterizer used `closest-side` for the
        // default radius, so a non-square layer had a "dead zone"
        // beyond the nearest edge where the gradient had already
        // terminated. With `farthest-corner` every corner sits on
        // the t=1 locus and the gradient extends to the full box.
        let grad = RadialGradient {
            shape_circle: true,
            stops: vec![
                GradientStop {
                    color: Color::rgba(255, 255, 255, 255),
                    position: 0.0,
                },
                GradientStop {
                    color: Color::rgba(255, 255, 255, 0),
                    position: 1.0,
                },
            ],
        };
        let buf = rasterize_radial_mask(16, 4, &grad, MaskMode::Alpha);
        // Center pixel at (8, 2) is fully opaque.
        let center = buf[(2 * 16 + 8) as usize];
        assert_eq!(center, 255, "center should be fully opaque");
        // A corner pixel (0, 0) lands near t=1 so it's near-
        // transparent. With the old closest-side radius of 2, corner
        // distance was ~8.25 against radius 2 → clamped to 1 but the
        // area beyond t=1 was also 0. Here we assert the corner is
        // close to zero AND that the midpoint between center and
        // corner is non-trivially opaque — i.e. the gradient
        // actually ramps across the width, not just near the center.
        let corner = buf[0];
        assert!(corner < 16, "corner should be near transparent: {corner}");
        let midpoint = buf[(2 * 16 + 4) as usize];
        assert!(
            midpoint > 32 && midpoint < 224,
            "midpoint should land in the gradient middle: {midpoint}"
        );
    }

    #[test]
    fn radial_mask_center_brighter_than_edge() {
        let grad = RadialGradient {
            shape_circle: true,
            stops: vec![
                GradientStop {
                    color: Color::rgba(255, 255, 255, 255),
                    position: 0.0,
                },
                GradientStop {
                    color: Color::rgba(255, 255, 255, 0),
                    position: 1.0,
                },
            ],
        };
        let buf = rasterize_radial_mask(8, 8, &grad, MaskMode::Alpha);
        let center = buf[(4 * 8 + 4) as usize];
        let corner = buf[0];
        assert!(center > corner);
        assert_eq!(center, 255);
    }

    #[test]
    fn apply_mask_destination_in_bites_out_transparent_half() {
        // 8-row-tall opaque red layer so the vertical gradient has
        // enough resolution to reach near-zero at the bottom.
        let mut layer: Vec<u8> = (0..8).flat_map(|_| [255u8, 0, 0, 255]).collect();
        let mask = MaskParams {
            image: CssBgImage::Gradient(opaque_white_to_transparent()),
            mode: MaskMode::Alpha,
            composite: MaskComposite::Add,
            texture: None,
            size: BackgroundSize::Auto,
            position: BackgroundPosition::default(),
            repeat: BackgroundRepeat::NoRepeat,
        };
        apply_mask(&mut layer, 1, 8, &mask);
        // Top row keeps full alpha (mask=255).
        assert_eq!(layer[3], 255, "top-row alpha preserved");
        // Alpha must decrease monotonically down the column.
        let alphas: Vec<u8> = (0..8).map(|y| layer[(y * 4 + 3) as usize]).collect();
        for pair in alphas.windows(2) {
            assert!(pair[0] >= pair[1], "alpha monotonic: {alphas:?}");
        }
        // Bottom row should be almost gone.
        assert!(alphas[7] < 64, "bottom alpha mostly cleared: {alphas:?}");
    }

    #[test]
    fn apply_mask_subtract_inverts_result() {
        let mut layer: Vec<u8> = (0..8).flat_map(|_| [0u8, 0, 255, 255]).collect();
        let mask = MaskParams {
            image: CssBgImage::Gradient(opaque_white_to_transparent()),
            mode: MaskMode::Alpha,
            composite: MaskComposite::Subtract,
            texture: None,
            size: BackgroundSize::Auto,
            position: BackgroundPosition::default(),
            repeat: BackgroundRepeat::NoRepeat,
        };
        apply_mask(&mut layer, 1, 8, &mask);
        // Subtract = destination-out: opaque mask REMOVES the layer.
        // Top row should be cleared, bottom row mostly preserved.
        let alphas: Vec<u8> = (0..8).map(|y| layer[(y * 4 + 3) as usize]).collect();
        assert_eq!(alphas[0], 0, "top alpha cleared: {alphas:?}");
        assert!(alphas[7] > 192, "bottom alpha mostly preserved: {alphas:?}");
        for pair in alphas.windows(2) {
            assert!(pair[0] <= pair[1], "alpha monotonic up: {alphas:?}");
        }
    }

    #[test]
    fn apply_mask_none_image_is_noop() {
        let mut layer: Vec<u8> = (0..16).flat_map(|_| [10u8, 20, 30, 128]).collect();
        let original = layer.clone();
        let mask = MaskParams {
            image: CssBgImage::None,
            mode: MaskMode::Alpha,
            composite: MaskComposite::Add,
            texture: None,
            size: BackgroundSize::Auto,
            position: BackgroundPosition::default(),
            repeat: BackgroundRepeat::NoRepeat,
        };
        apply_mask(&mut layer, 2, 2, &mask);
        assert_eq!(layer, original);
    }

    #[test]
    fn apply_mask_url_without_texture_is_noop() {
        // URL mask still fetching / decoding — the layer should
        // composite unchanged until the pixels land in a later frame.
        let mut layer: Vec<u8> = (0..16).flat_map(|_| [10u8, 20, 30, 128]).collect();
        let original = layer.clone();
        let mask = MaskParams {
            image: CssBgImage::Url("some-mask.png".to_string()),
            mode: MaskMode::Alpha,
            composite: MaskComposite::Add,
            texture: None,
            size: BackgroundSize::Auto,
            position: BackgroundPosition::default(),
            repeat: BackgroundRepeat::NoRepeat,
        };
        apply_mask(&mut layer, 2, 2, &mask);
        assert_eq!(layer, original);
    }

    #[test]
    fn apply_mask_url_with_texture_samples_alpha_channel() {
        // 2x2 mask source: opaque white on the left column,
        // transparent white on the right column. Alpha mode should
        // keep the left column and drop the right.
        let src = Arc::new(DecodedImage::new(
            2,
            2,
            vec![
                255, 255, 255, 255, 255, 255, 255, 0, 255, 255, 255, 255, 255, 255, 255, 0,
            ],
        ));
        let mut layer: Vec<u8> = (0..4).flat_map(|_| [255u8, 0, 0, 255]).collect();
        let mask = MaskParams {
            image: CssBgImage::Url("left-half.png".to_string()),
            mode: MaskMode::Alpha,
            composite: MaskComposite::Add,
            texture: Some(src),
            size: BackgroundSize::Auto,
            position: BackgroundPosition::default(),
            repeat: BackgroundRepeat::NoRepeat,
        };
        apply_mask(&mut layer, 2, 2, &mask);
        // Left column opaque, right column cleared.
        assert_eq!(layer[3], 255, "top-left kept");
        assert_eq!(layer[7], 0, "top-right cleared");
        assert_eq!(layer[11], 255, "bottom-left kept");
        assert_eq!(layer[15], 0, "bottom-right cleared");
    }

    #[test]
    fn apply_mask_url_luminance_uses_brightness() {
        // 2x1 source: black pixel left, white pixel right, both
        // fully opaque. Luminance mode should keep the right (white)
        // half and drop the left (black) half, even though both
        // alphas are identical.
        let src = Arc::new(DecodedImage::new(
            2,
            1,
            vec![0, 0, 0, 255, 255, 255, 255, 255],
        ));
        let mut layer: Vec<u8> = (0..2).flat_map(|_| [0u8, 128, 255, 255]).collect();
        let mask = MaskParams {
            image: CssBgImage::Url("bw.png".to_string()),
            mode: MaskMode::Luminance,
            composite: MaskComposite::Add,
            texture: Some(src),
            size: BackgroundSize::Auto,
            position: BackgroundPosition::default(),
            repeat: BackgroundRepeat::NoRepeat,
        };
        apply_mask(&mut layer, 2, 1, &mask);
        assert_eq!(layer[3], 0, "black→mask 0→layer cleared");
        assert_eq!(layer[7], 255, "white→mask 255→layer kept");
    }

    #[test]
    fn rasterize_url_mask_stretches_to_layer_bounds() {
        // 1x1 opaque white source with auto size + no-repeat — the mask
        // covers only the 1x1 intrinsic region. Use Cover to stretch.
        let src = DecodedImage::new(1, 1, vec![255, 255, 255, 255]);
        let buf = rasterize_url_mask(
            4,
            4,
            &src,
            MaskMode::Alpha,
            &BackgroundSize::Cover,
            &BackgroundPosition::default(),
            BackgroundRepeat::NoRepeat,
        );
        assert!(buf.iter().all(|&v| v == 255));
    }

    #[test]
    fn luminance_mask_uses_rgb_not_alpha() {
        // A black→white gradient, both stops fully opaque. With
        // `MaskMode::Luminance` the ramp should still go 0→255
        // (darkness → brightness) even though alpha is constant.
        let grad = LinearGradient {
            direction: GradientDirection::ToBottom,
            repeating: false,
            stops: vec![
                GradientStop {
                    color: Color::rgba(0, 0, 0, 255),
                    position: 0.0,
                },
                GradientStop {
                    color: Color::rgba(255, 255, 255, 255),
                    position: 1.0,
                },
            ],
        };
        let buf = rasterize_linear_mask(1, 8, &grad, MaskMode::Luminance);
        assert_eq!(buf[0], 0, "top row pitch black → mask 0");
        assert!(buf[7] > 192, "bottom row near-white → mask high");
        for pair in buf.windows(2) {
            assert!(pair[0] <= pair[1], "luminance monotonic: {buf:?}");
        }
    }

    #[test]
    fn url_mask_auto_size_no_repeat_covers_intrinsic_area_only() {
        // 2x2 opaque white source on a 4x4 layer with auto size and
        // no-repeat: only the top-left 2x2 should be opaque.
        let src = DecodedImage::new(2, 2, vec![255u8; 2 * 2 * 4]);
        let buf = rasterize_url_mask(
            4,
            4,
            &src,
            MaskMode::Alpha,
            &BackgroundSize::Auto,
            &BackgroundPosition::default(),
            BackgroundRepeat::NoRepeat,
        );
        // Top-left 2x2 should be 255.
        for y in 0..2u32 {
            for x in 0..2u32 {
                assert_eq!(buf[(y * 4 + x) as usize], 255, "({x},{y}) should be opaque");
            }
        }
        // Bottom-right 2x2 should be 0 (not covered by the mask).
        for y in 2..4u32 {
            for x in 2..4u32 {
                assert_eq!(
                    buf[(y * 4 + x) as usize],
                    0,
                    "({x},{y}) should be transparent"
                );
            }
        }
    }

    #[test]
    fn url_mask_repeat_tiles_across_layer() {
        // 2x2 opaque white source repeated across a 4x4 layer:
        // every pixel should be covered.
        let src = DecodedImage::new(2, 2, vec![255u8; 2 * 2 * 4]);
        let buf = rasterize_url_mask(
            4,
            4,
            &src,
            MaskMode::Alpha,
            &BackgroundSize::Auto,
            &BackgroundPosition::default(),
            BackgroundRepeat::Repeat,
        );
        assert!(
            buf.iter().all(|&v| v == 255),
            "all pixels covered by repeat"
        );
    }

    #[test]
    fn url_mask_contain_scales_within_bounds() {
        // 4x2 source on a 4x4 layer with contain: should scale to 4x2,
        // leaving the bottom 2 rows uncovered.
        let src = DecodedImage::new(4, 2, vec![255u8; 4 * 2 * 4]);
        let buf = rasterize_url_mask(
            4,
            4,
            &src,
            MaskMode::Alpha,
            &BackgroundSize::Contain,
            &BackgroundPosition::default(),
            BackgroundRepeat::NoRepeat,
        );
        // Top 2 rows covered.
        for y in 0..2u32 {
            for x in 0..4u32 {
                assert_eq!(buf[(y * 4 + x) as usize], 255, "({x},{y}) should be opaque");
            }
        }
        // Bottom 2 rows uncovered.
        for y in 2..4u32 {
            for x in 0..4u32 {
                assert_eq!(
                    buf[(y * 4 + x) as usize],
                    0,
                    "({x},{y}) should be transparent"
                );
            }
        }
    }

    #[test]
    fn url_mask_position_centers_mask() {
        // 2x2 opaque source on a 6x6 layer with center position.
        let src = DecodedImage::new(2, 2, vec![255u8; 2 * 2 * 4]);
        let buf = rasterize_url_mask(
            6,
            6,
            &src,
            MaskMode::Alpha,
            &BackgroundSize::Auto,
            &BackgroundPosition {
                x: 0.5,
                y: 0.5,
                x_is_px: false,
                y_is_px: false,
            },
            BackgroundRepeat::NoRepeat,
        );
        // Center of 6x6 with 2x2 image: offset = (6-2)*0.5 = 2.
        // Pixels at (2,2), (3,2), (2,3), (3,3) should be opaque.
        assert_eq!(buf[2 * 6 + 2], 255, "(2,2) opaque");
        assert_eq!(buf[2 * 6 + 3], 255, "(3,2) opaque");
        assert_eq!(buf[3 * 6 + 2], 255, "(2,3) opaque");
        assert_eq!(buf[3 * 6 + 3], 255, "(3,3) opaque");
        // Corners should be transparent.
        assert_eq!(buf[0], 0, "(0,0) transparent");
        assert_eq!(buf[5], 0, "(5,0) transparent");
        assert_eq!(buf[5 * 6], 0, "(0,5) transparent");
        assert_eq!(buf[5 * 6 + 5], 0, "(5,5) transparent");
    }
}
