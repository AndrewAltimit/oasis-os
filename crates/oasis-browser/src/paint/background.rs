//! Background painting: solid color, gradient, and background image.

use crate::css::values::{BackgroundImage, BackgroundPosition, BackgroundRepeat, BackgroundSize};
use crate::layout::box_model::LayoutBox;
use oasis_types::backend::{Color, SdiBackend, TextureId};
use oasis_types::error::Result;

use super::{PaintContext, apply_filters_and_opacity, apply_opacity};

pub(super) fn paint_background(
    layout_box: &LayoutBox,
    backend: &mut dyn SdiBackend,
    offset_x: i32,
    offset_y: i32,
    ctx: &PaintContext,
) -> Result<()> {
    let padding = layout_box.dimensions.padding_box();
    let x = (padding.x - ctx.scroll_x + offset_x as f32) as i32;
    let y = (padding.y - ctx.scroll_y + offset_y as f32) as i32;
    let w = padding.width as u32;
    let h = padding.height as u32;

    // Paint background color (with filters + opacity applied).
    let bg = apply_filters_and_opacity(
        layout_box.style.background_color,
        layout_box.style.opacity,
        &layout_box.style.filters,
    );
    if bg.a > 0 {
        let sx = padding.x - ctx.scroll_x + offset_x as f32;
        let sy = padding.y - ctx.scroll_y + offset_y as f32;
        if let Some(m3d) = ctx.ambient_screen_matrix.as_ref()
            && layout_box.style.border_radius.is_zero()
        {
            // A 3D-transformed ancestor went through the screen
            // path. Project all 4 padding-box corners through its
            // full 4×4 matrix so steep perspective rotations like
            // `rotateY(75deg) perspective(200px)` paint as a true
            // trapezoid instead of the 3-corner-fit parallelogram
            // used by `AffineTransform2D::transform_rect_to_quad`.
            let (w, h) = (padding.width, padding.height);
            let p0 = m3d.apply_point_3d(sx, sy, 0.0);
            let p1 = m3d.apply_point_3d(sx + w, sy, 0.0);
            let p2 = m3d.apply_point_3d(sx + w, sy + h, 0.0);
            let p3 = m3d.apply_point_3d(sx, sy + h, 0.0);
            // Guard against points at/behind the camera plane.
            // When `w` falls just above `apply_point_3d`'s 1e-6
            // divide-by-zero threshold, the perspective divide
            // produces finite-but-astronomical screen coordinates
            // (think `10000 / 2e-6 ≈ 5e9`) that saturate on the
            // `as i32` cast below — `NaN → 0`, `±Inf → i32::MIN/MAX`,
            // huge finite → `i32::MAX`. Without this check, an
            // element grazing the camera plane paints a full-screen
            // garbage polygon or a degenerate line instead of just
            // silently disappearing. Skip the polygon when any
            // corner's projection is non-finite or outside a
            // sane screen-range bound — elements fully clipped by
            // the camera frustum should not paint a background.
            let safe = |p: (f32, f32, f32)| -> bool {
                p.0.is_finite() && p.1.is_finite() && p.0.abs() < 1.0e7 && p.1.abs() < 1.0e7
            };
            if safe(p0) && safe(p1) && safe(p2) && safe(p3) {
                let quad = [
                    (p0.0 as i32, p0.1 as i32),
                    (p1.0 as i32, p1.1 as i32),
                    (p2.0 as i32, p2.1 as i32),
                    (p3.0 as i32, p3.1 as i32),
                ];
                backend.fill_polygon(&quad, bg)?;
            }
        } else if !ctx.transform.is_translation_only() && layout_box.style.border_radius.is_zero() {
            // Non-trivial 2D transform: render as transformed quadrilateral.
            let quad = ctx
                .transform
                .transform_rect_to_quad(sx, sy, padding.width, padding.height);
            backend.fill_polygon(&quad, bg)?;
        } else if !layout_box.style.border_radius.is_zero() {
            let r = layout_box.style.border_radius.max_radius() as u16;
            backend.fill_rounded_rect(x, y, w, h, r, bg)?;
        } else {
            backend.fill_rect(x, y, w, h, bg)?;
        }
    }

    // Paint linear gradient background.
    if let BackgroundImage::Gradient(ref grad) = layout_box.style.background_image {
        paint_linear_gradient(backend, x, y, w, h, grad, layout_box.style.opacity)?;
    }

    // Paint radial gradient background.
    if let BackgroundImage::RadialGradient(ref grad) = layout_box.style.background_image {
        paint_radial_gradient(backend, x, y, w, h, grad, layout_box.style.opacity)?;
    }

    // Paint background image (if texture has been resolved).
    if let Some(tex) = layout_box.background_texture {
        let tex_size = layout_box.background_texture_size.unwrap_or((w, h));
        paint_background_image(
            backend,
            tex,
            x,
            y,
            w,
            h,
            tex_size,
            &layout_box.style.background_size,
            &layout_box.style.background_position,
            layout_box.style.background_repeat,
        )?;
    }

    Ok(())
}

/// A single tile blit position computed by [`background_image_tiles`].
#[derive(Debug, Clone, Copy)]
pub(crate) struct BgTile {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

/// Compute the list of background-image tiles to blit.
///
/// Shared between immediate-mode painting (`paint_background_image`) and the
/// display-list recording path (`record.rs::record_background`). Returns an
/// empty Vec when the image would be invisible (zero size).
#[allow(clippy::too_many_arguments)]
pub(crate) fn background_image_tiles(
    container_x: i32,
    container_y: i32,
    container_w: u32,
    container_h: u32,
    intrinsic_size: (u32, u32),
    size: &BackgroundSize,
    position: &BackgroundPosition,
    repeat: BackgroundRepeat,
) -> Vec<BgTile> {
    let (tex_w, tex_h) = intrinsic_size;
    if tex_w == 0 || tex_h == 0 {
        return Vec::new();
    }

    // Compute the rendered image dimensions based on background-size.
    let (img_w, img_h) = match size {
        BackgroundSize::Auto => (tex_w, tex_h),
        BackgroundSize::Cover => {
            let scale_x = container_w as f32 / tex_w as f32;
            let scale_y = container_h as f32 / tex_h as f32;
            let scale = scale_x.max(scale_y);
            ((tex_w as f32 * scale) as u32, (tex_h as f32 * scale) as u32)
        },
        BackgroundSize::Contain => {
            let scale_x = container_w as f32 / tex_w as f32;
            let scale_y = container_h as f32 / tex_h as f32;
            let scale = scale_x.min(scale_y);
            ((tex_w as f32 * scale) as u32, (tex_h as f32 * scale) as u32)
        },
        BackgroundSize::Explicit(w, h) => {
            let iw = match w {
                Some(v) if v.is_sign_negative() => (container_w as f32 * (-*v / 100.0)) as u32,
                Some(v) => *v as u32,
                None => {
                    if let Some(hv) = h {
                        let hpx = if hv.is_sign_negative() {
                            container_h as f32 * (-*hv / 100.0)
                        } else {
                            *hv
                        };
                        (tex_w as f32 * hpx / tex_h as f32) as u32
                    } else {
                        tex_w
                    }
                },
            };
            let ih = match h {
                Some(v) if v.is_sign_negative() => (container_h as f32 * (-*v / 100.0)) as u32,
                Some(v) => *v as u32,
                None => (tex_h as f32 * iw as f32 / tex_w as f32) as u32,
            };
            if iw == 0 || ih == 0 {
                return Vec::new();
            }
            (iw, ih)
        },
    };

    if img_w == 0 || img_h == 0 {
        return Vec::new();
    }

    let pos_x = if position.x_is_px {
        position.x as i32
    } else {
        ((container_w as f32 - img_w as f32) * position.x) as i32
    };
    let pos_y = if position.y_is_px {
        position.y as i32
    } else {
        ((container_h as f32 - img_h as f32) * position.y) as i32
    };

    let repeat_x = matches!(repeat, BackgroundRepeat::Repeat | BackgroundRepeat::RepeatX);
    let repeat_y = matches!(repeat, BackgroundRepeat::Repeat | BackgroundRepeat::RepeatY);

    let mut tiles = Vec::new();

    if !repeat_x && !repeat_y {
        tiles.push(BgTile {
            x: container_x + pos_x,
            y: container_y + pos_y,
            w: img_w,
            h: img_h,
        });
        return tiles;
    }

    let start_x = if repeat_x {
        pos_x.rem_euclid(img_w as i32) - img_w as i32
    } else {
        pos_x
    };
    let start_y = if repeat_y {
        pos_y.rem_euclid(img_h as i32) - img_h as i32
    } else {
        pos_y
    };

    let mut ty = start_y;
    while ty < container_h as i32 {
        let mut tx = start_x;
        loop {
            if tx >= container_w as i32 {
                break;
            }
            tiles.push(BgTile {
                x: container_x + tx,
                y: container_y + ty,
                w: img_w,
                h: img_h,
            });
            if !repeat_x {
                break;
            }
            tx += img_w as i32;
        }
        if !repeat_y {
            break;
        }
        ty += img_h as i32;
    }

    tiles
}

/// Paint a background image with size, position, and repeat properties.
#[allow(clippy::too_many_arguments)]
fn paint_background_image(
    backend: &mut dyn SdiBackend,
    tex: TextureId,
    container_x: i32,
    container_y: i32,
    container_w: u32,
    container_h: u32,
    intrinsic_size: (u32, u32),
    size: &BackgroundSize,
    position: &BackgroundPosition,
    repeat: BackgroundRepeat,
) -> Result<()> {
    let tiles = background_image_tiles(
        container_x,
        container_y,
        container_w,
        container_h,
        intrinsic_size,
        size,
        position,
        repeat,
    );
    for tile in tiles {
        backend.blit(tex, tile.x, tile.y, tile.w, tile.h)?;
    }
    Ok(())
}

/// Interpolate between two colors at a given factor `t` (0.0 = `a`, 1.0 = `b`).
fn lerp_color(a: Color, b: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    let inv = 1.0 - t;
    Color::rgba(
        (a.r as f32 * inv + b.r as f32 * t) as u8,
        (a.g as f32 * inv + b.g as f32 * t) as u8,
        (a.b as f32 * inv + b.b as f32 * t) as u8,
        (a.a as f32 * inv + b.a as f32 * t) as u8,
    )
}

/// Sample the gradient color at a normalized position `t` (0.0..=1.0).
///
/// Exposed as `pub(crate)` for the display list recorder.
pub(crate) fn sample_gradient_pub(
    stops: &[crate::css::values::GradientStop],
    t: f32,
    opacity: f32,
) -> Color {
    sample_gradient(stops, t, opacity)
}

/// Sample the gradient color at a normalized position `t` (0.0..=1.0).
fn sample_gradient(stops: &[crate::css::values::GradientStop], t: f32, opacity: f32) -> Color {
    if stops.is_empty() {
        return Color::rgba(0, 0, 0, 0);
    }
    if t <= stops[0].position {
        return apply_opacity(stops[0].color, opacity);
    }
    let last = stops.len() - 1;
    if t >= stops[last].position {
        return apply_opacity(stops[last].color, opacity);
    }
    for i in 0..last {
        if t >= stops[i].position && t <= stops[i + 1].position {
            let range = stops[i + 1].position - stops[i].position;
            let local_t = if range > 0.0 {
                (t - stops[i].position) / range
            } else {
                0.0
            };
            let c = lerp_color(stops[i].color, stops[i + 1].color, local_t);
            return apply_opacity(c, opacity);
        }
    }
    apply_opacity(stops[last].color, opacity)
}

/// Render a CSS `linear-gradient(...)` using the backend's gradient fill.
///
/// Multi-stop gradients are rendered by splitting into bands (one per pair
/// of adjacent stops). Arbitrary angles are rendered row-by-row for diagonal
/// directions.
pub(super) fn paint_linear_gradient(
    backend: &mut dyn SdiBackend,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    grad: &crate::css::values::LinearGradient,
    opacity: f32,
) -> Result<()> {
    use crate::css::values::GradientDirection;
    use oasis_types::backend::GradientStyle;

    if grad.stops.len() < 2 || w == 0 || h == 0 {
        return Ok(());
    }

    // Determine if the gradient is vertical, horizontal, or diagonal.
    let angle_deg = match grad.direction {
        GradientDirection::ToBottom => 180.0,
        GradientDirection::ToTop => 0.0,
        GradientDirection::ToRight => 90.0,
        GradientDirection::ToLeft => 270.0,
        GradientDirection::Angle(deg) => deg,
    };
    let norm = ((angle_deg % 360.0) + 360.0) % 360.0;

    // Check if this is an axis-aligned gradient.
    let is_vertical =
        (norm - 0.0).abs() < 0.5 || (norm - 180.0).abs() < 0.5 || (norm - 360.0).abs() < 0.5;
    let is_horizontal = (norm - 90.0).abs() < 0.5 || (norm - 270.0).abs() < 0.5;

    if is_vertical || is_horizontal {
        // Axis-aligned: render multi-stop by splitting into bands.
        let to_end = is_vertical && (norm - 0.0).abs() < 1.0 || (norm - 360.0).abs() < 1.0;
        let reverse = if is_vertical {
            // ToTop (0 deg): reverse (bottom-to-top in screen space).
            to_end
        } else {
            // ToLeft (270 deg): reverse.
            (norm - 270.0).abs() < 1.0
        };

        let total_len = if is_vertical { h } else { w } as f32;
        let stops = &grad.stops;

        // For repeating gradients, compute the pattern length and
        // tile across the element.
        let last_pos = stops.last().map(|s| s.position).unwrap_or(1.0);
        let first_pos = stops.first().map(|s| s.position).unwrap_or(0.0);
        let pattern_range = last_pos - first_pos;
        let repetitions = if grad.repeating && pattern_range > 0.0 {
            (1.0 / pattern_range).ceil() as u32
        } else {
            1
        };

        for rep in 0..repetitions {
            let rep_offset = if grad.repeating {
                rep as f32 * pattern_range
            } else {
                0.0
            };

            // Render each segment between adjacent stops.
            for i in 0..stops.len() - 1 {
                let (s0, s1) = if reverse {
                    let ri = stops.len() - 1 - i;
                    (&stops[ri], &stops[ri - 1])
                } else {
                    (&stops[i], &stops[i + 1])
                };

                let start_frac = if reverse {
                    1.0 - s0.position
                } else {
                    s0.position
                } + rep_offset;
                let end_frac = if reverse {
                    1.0 - s1.position
                } else {
                    s1.position
                } + rep_offset;
                if start_frac >= 1.0 {
                    break;
                }
                let end_frac = end_frac.min(1.0);
                let c0 = apply_opacity(s0.color, opacity);
                let c1 = apply_opacity(s1.color, opacity);

                let start_px = (start_frac * total_len) as i32;
                let end_px = ((end_frac * total_len) as i32).min(total_len as i32);
                let seg_len = (end_px - start_px).max(0) as u32;
                if seg_len == 0 {
                    continue;
                }

                if is_vertical {
                    backend.fill_rect_gradient(
                        x,
                        y + start_px,
                        w,
                        seg_len,
                        &GradientStyle::Vertical {
                            top: c0,
                            bottom: c1,
                        },
                    )?;
                } else {
                    backend.fill_rect_gradient(
                        x + start_px,
                        y,
                        seg_len,
                        h,
                        &GradientStyle::Horizontal {
                            left: c0,
                            right: c1,
                        },
                    )?;
                }
            }
        }
    } else {
        // Diagonal gradient: render with horizontal bands.
        // For each band we compute the gradient position `t` at the left
        // and right edges and emit a horizontal gradient fill with the
        // sampled colors. This produces correct diagonal gradients with
        // O(bands) draw calls (capped at 32).
        let rad = norm.to_radians();
        let dx = rad.sin();
        let dy = -rad.cos(); // CSS: 0deg = to-top = -Y
        let wf = w as f32;
        let hf = h as f32;

        // Project corners onto the gradient axis to find the full range.
        // Corners: (0,0), (w,0), (0,h), (w,h). The gradient line runs
        // through the center in direction (dx, dy). Projection of a
        // corner (cx, cy) relative to center: dot = (cx - w/2)*dx + (cy - h/2)*dy.
        let half_w = wf / 2.0;
        let half_h = hf / 2.0;
        let mut min_proj = f32::MAX;
        let mut max_proj = f32::MIN;
        for &(cx, cy) in &[(0.0, 0.0), (wf, 0.0), (0.0, hf), (wf, hf)] {
            let proj = (cx - half_w) * dx + (cy - half_h) * dy;
            if proj < min_proj {
                min_proj = proj;
            }
            if proj > max_proj {
                max_proj = proj;
            }
        }
        let proj_range = max_proj - min_proj;
        if proj_range < 0.001 {
            return Ok(());
        }

        let num_bands = (h as usize).clamp(1, 32);
        let band_h_f = hf / num_bands as f32;

        for band in 0..num_bands {
            let by = band as f32 * band_h_f;
            let band_cy = by + band_h_f / 2.0; // vertical center of band

            // Gradient position `t` at left edge (x=0) and right edge (x=w).
            let t_left = ((0.0 - half_w) * dx + (band_cy - half_h) * dy - min_proj) / proj_range;
            let t_right = ((wf - half_w) * dx + (band_cy - half_h) * dy - min_proj) / proj_range;

            let c_left = sample_gradient(&grad.stops, t_left, opacity);
            let c_right = sample_gradient(&grad.stops, t_right, opacity);

            let start_y = by as i32;
            let end_y = if band == num_bands - 1 {
                h as i32
            } else {
                ((band + 1) as f32 * band_h_f) as i32
            };
            let band_y = y + start_y;
            let band_h = (end_y - start_y).max(0) as u32;
            if band_h == 0 {
                continue;
            }

            backend.fill_rect_gradient(
                x,
                band_y,
                w,
                band_h,
                &GradientStyle::Horizontal {
                    left: c_left,
                    right: c_right,
                },
            )?;
        }
    }

    Ok(())
}

/// Render a CSS `radial-gradient(...)` using concentric bands.
///
/// Instead of per-pixel rendering (O(w*h) fill_rect calls), this uses
/// concentric rounded rectangles from the outermost stop inward to the
/// center, with O(bands) calls. Each band is sampled at its midpoint
/// radius.
pub(super) fn paint_radial_gradient(
    backend: &mut dyn SdiBackend,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    grad: &crate::css::values::RadialGradient,
    opacity: f32,
) -> Result<()> {
    if grad.stops.len() < 2 || w == 0 || h == 0 {
        return Ok(());
    }

    let hw = w as f32 / 2.0;
    let hh = h as f32 / 2.0;
    let max_radius = if grad.shape_circle {
        hw.max(hh)
    } else {
        // Ellipse: use the diagonal as the effective radius.
        (hw * hw + hh * hh).sqrt()
    };

    // Use enough bands for smooth appearance but cap to limit draw calls.
    // 48 bands is visually indistinguishable from 128 for typical radii
    // while cutting draw calls by up to ~60%.
    let bands = (max_radius as u32).clamp(8, 48);

    // Paint from outside in so inner bands overlay outer.
    for i in 0..bands {
        // Fraction from outer (0.0) to center (1.0).
        let frac = i as f32 / bands as f32;
        // Gradient position: 0.0 at center, 1.0 at edge.
        let t = 1.0 - frac;
        let color = sample_gradient(&grad.stops, t, opacity);
        // Force opaque to prevent alpha over-accumulation from
        // overlapping concentric filled rounded rects. Element-level
        // opacity is handled by `apply_opacity` at the call site.
        let color = Color::rgba(color.r, color.g, color.b, 255);

        // Size of this band's rect.
        let bw = (w as f32 * (1.0 - frac)).max(1.0) as u32;
        let bh = (h as f32 * (1.0 - frac)).max(1.0) as u32;
        let bx = x + ((w - bw) / 2) as i32;
        let by = y + ((h - bh) / 2) as i32;
        let r = (bw.min(bh) / 2).max(1) as u16;

        backend.fill_rounded_rect(bx, by, bw, bh, r, color)?;
    }

    Ok(())
}
