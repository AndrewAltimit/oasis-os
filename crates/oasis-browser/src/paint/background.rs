//! Background painting: solid color, gradient, and background image.

use crate::css::values::BackgroundImage;
use crate::layout::box_model::LayoutBox;
use oasis_types::backend::{Color, SdiBackend};
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
        if layout_box.style.border_radius > 0.0 {
            backend.fill_rounded_rect(x, y, w, h, layout_box.style.border_radius as u16, bg)?;
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
        backend.blit(tex, x, y, w, h)?;
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

            let band_y = y + by as i32;
            let band_h = if band == num_bands - 1 {
                // Last band absorbs rounding remainder.
                (h as i32 - by as i32) as u32
            } else {
                band_h_f.ceil() as u32
            };
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
