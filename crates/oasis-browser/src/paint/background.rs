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
            };
            let end_frac = if reverse {
                1.0 - s1.position
            } else {
                s1.position
            };
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
    } else {
        // Diagonal gradient: render row-by-row for true arbitrary angles.
        // CSS gradient angle: 0deg = to top, 90deg = to right.
        // Convert to math angle: math_angle = 90 - css_angle.
        let rad = (90.0 - norm).to_radians();
        let cos = rad.cos();
        let sin = rad.sin();

        // Project corners onto the gradient line to find the extent.
        let hw = w as f32 / 2.0;
        let hh = h as f32 / 2.0;
        let max_proj = (hw * cos.abs() + hh * sin.abs()).max(1.0);

        for row in 0..h {
            for col in 0..w {
                // Distance from center along gradient direction, normalized.
                let dx = col as f32 - hw;
                let dy = -(row as f32 - hh); // Y-up for math
                let proj = dx * cos + dy * sin;
                let t = (proj / max_proj + 1.0) / 2.0; // 0..1
                let color = sample_gradient(&grad.stops, t, opacity);
                backend.fill_rect(x + col as i32, y + row as i32, 1, 1, color)?;
            }
        }
    }

    Ok(())
}
