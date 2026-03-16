//! Background painting: solid color, gradient, and background image.

use crate::css::values::BackgroundImage;
use crate::layout::box_model::LayoutBox;
use oasis_types::backend::SdiBackend;
use oasis_types::error::Result;

use super::{PaintContext, apply_opacity};

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

    // Paint background color.
    let bg = apply_opacity(layout_box.style.background_color, layout_box.style.opacity);
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

/// Render a CSS `linear-gradient(...)` using the backend's gradient fill.
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

    let first = apply_opacity(grad.stops[0].color, opacity);
    let last = apply_opacity(grad.stops[grad.stops.len() - 1].color, opacity);

    match grad.direction {
        GradientDirection::ToBottom | GradientDirection::Angle(180.0) => {
            backend.fill_rect_gradient(
                x,
                y,
                w,
                h,
                &GradientStyle::Vertical {
                    top: first,
                    bottom: last,
                },
            )?;
        },
        GradientDirection::ToTop | GradientDirection::Angle(0.0) => {
            backend.fill_rect_gradient(
                x,
                y,
                w,
                h,
                &GradientStyle::Vertical {
                    top: last,
                    bottom: first,
                },
            )?;
        },
        GradientDirection::ToRight | GradientDirection::Angle(90.0) => {
            backend.fill_rect_gradient(
                x,
                y,
                w,
                h,
                &GradientStyle::Horizontal {
                    left: first,
                    right: last,
                },
            )?;
        },
        GradientDirection::ToLeft | GradientDirection::Angle(270.0) => {
            backend.fill_rect_gradient(
                x,
                y,
                w,
                h,
                &GradientStyle::Horizontal {
                    left: last,
                    right: first,
                },
            )?;
        },
        GradientDirection::Angle(deg) => {
            // For arbitrary angles, approximate with the closest axis.
            let norm = ((deg % 360.0) + 360.0) % 360.0;
            if !(45.0..315.0).contains(&norm) {
                // ~to top
                backend.fill_rect_gradient(
                    x,
                    y,
                    w,
                    h,
                    &GradientStyle::Vertical {
                        top: last,
                        bottom: first,
                    },
                )?;
            } else if norm < 135.0 {
                // ~to right
                backend.fill_rect_gradient(
                    x,
                    y,
                    w,
                    h,
                    &GradientStyle::Horizontal {
                        left: first,
                        right: last,
                    },
                )?;
            } else if norm < 225.0 {
                // ~to bottom
                backend.fill_rect_gradient(
                    x,
                    y,
                    w,
                    h,
                    &GradientStyle::Vertical {
                        top: first,
                        bottom: last,
                    },
                )?;
            } else {
                // ~to left
                backend.fill_rect_gradient(
                    x,
                    y,
                    w,
                    h,
                    &GradientStyle::Horizontal {
                        left: last,
                        right: first,
                    },
                )?;
            }
        },
    }

    Ok(())
}
