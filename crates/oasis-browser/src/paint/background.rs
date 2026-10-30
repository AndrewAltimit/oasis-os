//! Background geometry helpers shared by the recorder (`record.rs`)
//! and the mask rasterizer (`display_list/mask.rs`): background-image
//! tiling and gradient-stop sampling.

use crate::css::values::{BackgroundPosition, BackgroundRepeat, BackgroundSize};
use oasis_types::backend::Color;

fn apply_opacity(color: Color, opacity: f32) -> Color {
    color.apply_opacity(opacity)
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
/// Used by `record.rs::record_background` and the mask rasterizer.
/// Returns an empty Vec when the image would be invisible (zero size).
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
