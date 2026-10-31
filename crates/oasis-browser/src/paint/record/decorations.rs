//! Box decorations: backgrounds, gradients, borders, outlines and
//! box shadows.

use super::*;

pub(super) fn record_background(
    layout_box: &LayoutBox,
    dl: &mut DisplayList,
    offset_x: i32,
    offset_y: i32,
    ctx: &RecordContext,
    effective_opacity: f32,
) {
    let padding = layout_box.dimensions.padding_box();
    let x = (padding.x - ctx.scroll_x + offset_x as f32) as i32;
    let y = (padding.y - ctx.scroll_y + offset_y as f32) as i32;
    let w = padding.width as u32;
    let h = padding.height as u32;

    // Box-level `filter` is not baked in here: the element's
    // compositing layer applies it at replay (per-pixel on readback, or
    // the per-color approximation where the backend has no readback).
    let bg = apply_opacity(layout_box.style.background_color, effective_opacity);
    if bg.a > 0 {
        let sx = padding.x - ctx.scroll_x + offset_x as f32;
        let sy = padding.y - ctx.scroll_y + offset_y as f32;
        let square = layout_box.style.border_radius.is_zero();
        if square && let Some(m3d) = ctx.ambient_screen_matrix.as_ref() {
            // 3D-transformed ancestor: project all 4 padding-box corners
            // through the full 4x4 matrix for true perspective rendering.
            let fw = padding.width;
            let fh = padding.height;
            let p0 = m3d.apply_point_3d(sx, sy, 0.0);
            let p1 = m3d.apply_point_3d(sx + fw, sy, 0.0);
            let p2 = m3d.apply_point_3d(sx + fw, sy + fh, 0.0);
            let p3 = m3d.apply_point_3d(sx, sy + fh, 0.0);
            let safe = |p: (f32, f32, f32)| -> bool {
                p.0.is_finite() && p.1.is_finite() && p.0.abs() < 1.0e7 && p.1.abs() < 1.0e7
            };
            // Intentional: back-face / extreme-perspective cull. Non-finite or
            // out-of-bounds projected corners indicate the element is facing
            // away from the camera or past the perspective limit, so we drop
            // the fill rather than emit a degenerate polygon.
            if safe(p0) && safe(p1) && safe(p2) && safe(p3) {
                dl.push(DisplayItem::FillPolygon {
                    points: [
                        (p0.0 as i32, p0.1 as i32),
                        (p1.0 as i32, p1.1 as i32),
                        (p2.0 as i32, p2.1 as i32),
                        (p3.0 as i32, p3.1 as i32),
                    ],
                    color: bg,
                    node_id: ctx.current_node,
                });
            }
        } else if square && !ctx.transform.is_translation_only() {
            // Non-trivial 2D ancestor transform: a transformed quad.
            let points =
                ctx.transform
                    .transform_rect_to_quad(sx, sy, padding.width, padding.height);
            dl.push(DisplayItem::FillPolygon {
                points,
                color: bg,
                node_id: ctx.current_node,
            });
        } else if !square {
            dl.push(DisplayItem::FillRoundedRect {
                x,
                y,
                w,
                h,
                radius: layout_box.style.border_radius.max_radius() as u16,
                color: bg,
                node_id: ctx.current_node,
            });
        } else {
            dl.push(DisplayItem::FillRect {
                x,
                y,
                w,
                h,
                color: bg,
                node_id: ctx.current_node,
            });
        }
    }

    // Linear gradient.
    if let crate::css::values::BackgroundImage::Gradient(ref grad) =
        layout_box.style.background_image
    {
        record_linear_gradient(dl, x, y, w, h, grad, effective_opacity);
    }

    // Radial gradient.
    if let crate::css::values::BackgroundImage::RadialGradient(ref grad) =
        layout_box.style.background_image
    {
        record_radial_gradient(dl, x, y, w, h, grad, effective_opacity);
    }

    // Background image texture: emit one Blit per tile so background-size,
    // background-position, and background-repeat are honored.
    if let Some(tex) = layout_box.background_texture {
        let intrinsic = layout_box.background_texture_size.unwrap_or((w, h));
        let tiles = crate::paint::background::background_image_tiles(
            x,
            y,
            w,
            h,
            intrinsic,
            &layout_box.style.background_size,
            &layout_box.style.background_position,
            layout_box.style.background_repeat,
        );
        for tile in tiles {
            dl.push(DisplayItem::Blit {
                texture: tex,
                x: tile.x,
                y: tile.y,
                w: tile.w,
                h: tile.h,
            });
        }
    }
}

pub(super) fn record_linear_gradient(
    dl: &mut DisplayList,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    grad: &crate::css::values::LinearGradient,
    opacity: f32,
) {
    use crate::css::values::GradientDirection;
    use oasis_types::backend::GradientStyle;

    if grad.stops.len() < 2 || w == 0 || h == 0 {
        return;
    }

    let angle_deg = match grad.direction {
        GradientDirection::ToBottom => 180.0,
        GradientDirection::ToTop => 0.0,
        GradientDirection::ToRight => 90.0,
        GradientDirection::ToLeft => 270.0,
        GradientDirection::Angle(deg) => deg,
    };
    let norm = ((angle_deg % 360.0) + 360.0) % 360.0;

    let is_vertical =
        (norm - 0.0).abs() < 0.5 || (norm - 180.0).abs() < 0.5 || (norm - 360.0).abs() < 0.5;
    let is_horizontal = (norm - 90.0).abs() < 0.5 || (norm - 270.0).abs() < 0.5;

    if is_vertical || is_horizontal {
        let to_end = is_vertical && (norm - 0.0).abs() < 1.0 || (norm - 360.0).abs() < 1.0;
        let reverse = if is_vertical {
            to_end
        } else {
            (norm - 270.0).abs() < 1.0
        };

        let total_len = if is_vertical { h } else { w } as f32;
        let stops = &grad.stops;

        let last_pos = stops.last().map(|s| s.position).unwrap_or(1.0);
        let first_pos = stops.first().map(|s| s.position).unwrap_or(0.0);
        let pattern_range = last_pos - first_pos;
        // A pattern narrower than a pixel can't add detail, so at most
        // one repetition per pixel: `blue 1e-9%` would otherwise ask for
        // ~4e9 repetitions.
        let repetitions = if grad.repeating && pattern_range > 0.0 {
            (1.0 / pattern_range)
                .ceil()
                .min(total_len.max(0.0).ceil() + 1.0) as u32
        } else {
            1
        };

        for rep in 0..repetitions {
            let rep_offset = if grad.repeating {
                rep as f32 * pattern_range
            } else {
                0.0
            };

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
                    dl.push(DisplayItem::Gradient {
                        x,
                        y: y + start_px,
                        w,
                        h: seg_len,
                        style: GradientStyle::Vertical {
                            top: c0,
                            bottom: c1,
                        },
                    });
                } else {
                    dl.push(DisplayItem::Gradient {
                        x: x + start_px,
                        y,
                        w: seg_len,
                        h,
                        style: GradientStyle::Horizontal {
                            left: c0,
                            right: c1,
                        },
                    });
                }
            }
        }
    } else {
        // Diagonal gradient: render with horizontal bands matching background.rs.
        let rad = norm.to_radians();
        let dx = rad.sin();
        let dy = -rad.cos();
        let wf = w as f32;
        let hf = h as f32;

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
            return;
        }

        let num_bands = (h as usize).clamp(1, 32);
        let band_h_f = hf / num_bands as f32;

        for band in 0..num_bands {
            let by = band as f32 * band_h_f;
            let band_cy = by + band_h_f / 2.0;

            let t_left = ((0.0 - half_w) * dx + (band_cy - half_h) * dy - min_proj) / proj_range;
            let t_right = ((wf - half_w) * dx + (band_cy - half_h) * dy - min_proj) / proj_range;

            let c_left = crate::paint::background::sample_gradient(&grad.stops, t_left, opacity);
            let c_right = crate::paint::background::sample_gradient(&grad.stops, t_right, opacity);

            let start_y = by as i32;
            let end_y = if band == num_bands - 1 {
                h as i32
            } else {
                ((band + 1) as f32 * band_h_f) as i32
            };
            let band_h_px = (end_y - start_y).max(0) as u32;
            if band_h_px == 0 {
                continue;
            }

            dl.push(DisplayItem::Gradient {
                x,
                y: y + start_y,
                w,
                h: band_h_px,
                style: GradientStyle::Horizontal {
                    left: c_left,
                    right: c_right,
                },
            });
        }
    }
}

pub(super) fn record_radial_gradient(
    dl: &mut DisplayList,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    grad: &crate::css::values::RadialGradient,
    opacity: f32,
) {
    if grad.stops.len() < 2 || w == 0 || h == 0 {
        return;
    }

    let hw = w as f32 / 2.0;
    let hh = h as f32 / 2.0;
    let max_radius = if grad.shape_circle {
        hw.max(hh)
    } else {
        (hw * hw + hh * hh).sqrt()
    };

    let bands = (max_radius as u32).clamp(8, 48);

    for i in 0..bands {
        let frac = i as f32 / bands as f32;
        let t = 1.0 - frac;
        let color = crate::paint::background::sample_gradient(&grad.stops, t, opacity);
        let color = Color::rgba(color.r, color.g, color.b, 255);

        let bw = (w as f32 * (1.0 - frac)).max(1.0) as u32;
        let bh = (h as f32 * (1.0 - frac)).max(1.0) as u32;
        let bx = x + ((w - bw) / 2) as i32;
        let by = y + ((h - bh) / 2) as i32;
        let r = (bw.min(bh) / 2).max(1) as u16;

        dl.push(DisplayItem::FillRoundedRect {
            x: bx,
            y: by,
            w: bw,
            h: bh,
            radius: r,
            color,
            node_id: None,
        });
    }
}

pub(super) fn record_borders(
    layout_box: &LayoutBox,
    dl: &mut DisplayList,
    offset_x: i32,
    offset_y: i32,
    ctx: &RecordContext,
) {
    let d = &layout_box.dimensions;
    let style = &layout_box.style;
    let border = d.border_box();
    let bx = (border.x - ctx.scroll_x + offset_x as f32) as i32;
    let by = (border.y - ctx.scroll_y + offset_y as f32) as i32;
    let bw = border.width as u32;
    let bh = border.height as u32;

    if d.border.top > 0.0 && style.border_top_style != BorderStyle::None {
        dl.push(DisplayItem::BorderEdge {
            x: bx,
            y: by,
            w: bw,
            h: d.border.top as u32,
            color: style.border_top_color,
            style: style.border_top_style,
            horizontal: true,
        });
    }
    if d.border.right > 0.0 && style.border_right_style != BorderStyle::None {
        dl.push(DisplayItem::BorderEdge {
            x: bx + bw as i32 - d.border.right as i32,
            y: by,
            w: d.border.right as u32,
            h: bh,
            color: style.border_right_color,
            style: style.border_right_style,
            horizontal: false,
        });
    }
    if d.border.bottom > 0.0 && style.border_bottom_style != BorderStyle::None {
        dl.push(DisplayItem::BorderEdge {
            x: bx,
            y: by + bh as i32 - d.border.bottom as i32,
            w: bw,
            h: d.border.bottom as u32,
            color: style.border_bottom_color,
            style: style.border_bottom_style,
            horizontal: true,
        });
    }
    if d.border.left > 0.0 && style.border_left_style != BorderStyle::None {
        dl.push(DisplayItem::BorderEdge {
            x: bx,
            y: by,
            w: d.border.left as u32,
            h: bh,
            color: style.border_left_color,
            style: style.border_left_style,
            horizontal: false,
        });
    }
}

pub(super) fn record_outline(
    layout_box: &LayoutBox,
    dl: &mut DisplayList,
    offset_x: i32,
    offset_y: i32,
    ctx: &RecordContext,
) {
    let style = &layout_box.style;
    if style.outline_width <= 0.0 || style.outline_style == BorderStyle::None {
        return;
    }

    let border = layout_box.dimensions.border_box();
    let ow = style.outline_width;
    let oo = style.outline_offset;
    let ox = (border.x - ctx.scroll_x + offset_x as f32 - oo - ow) as i32;
    let oy = (border.y - ctx.scroll_y + offset_y as f32 - oo - ow) as i32;
    let total_w = (border.width + 2.0 * (oo + ow)).max(0.0) as u32;
    let total_h = (border.height + 2.0 * (oo + ow)).max(0.0) as u32;
    let thickness = ow as u32;
    let color = style.outline_color;
    let outline_style = style.outline_style;

    // Top
    dl.push(DisplayItem::BorderEdge {
        x: ox,
        y: oy,
        w: total_w,
        h: thickness,
        color,
        style: outline_style,
        horizontal: true,
    });
    // Bottom
    dl.push(DisplayItem::BorderEdge {
        x: ox,
        y: oy + total_h as i32 - thickness as i32,
        w: total_w,
        h: thickness,
        color,
        style: outline_style,
        horizontal: true,
    });
    // Left
    dl.push(DisplayItem::BorderEdge {
        x: ox,
        y: oy,
        w: thickness,
        h: total_h,
        color,
        style: outline_style,
        horizontal: false,
    });
    // Right
    dl.push(DisplayItem::BorderEdge {
        x: ox + total_w as i32 - thickness as i32,
        y: oy,
        w: thickness,
        h: total_h,
        color,
        style: outline_style,
        horizontal: false,
    });
}

pub(super) fn record_box_shadow(
    layout_box: &LayoutBox,
    dl: &mut DisplayList,
    offset_x: i32,
    offset_y: i32,
    ctx: &RecordContext,
    _effective_opacity: f32,
) {
    if layout_box.style.box_shadow.is_empty() {
        return;
    }

    let border = layout_box.dimensions.border_box();
    let radius = layout_box.style.border_radius.max_radius();

    for shadow in layout_box.style.box_shadow.iter().rev() {
        if shadow.inset {
            let padding = layout_box.dimensions.padding_box();
            let px = (padding.x - ctx.scroll_x + offset_x as f32) as i32;
            let py = (padding.y - ctx.scroll_y + offset_y as f32) as i32;
            let pw = padding.width as u32;
            let ph = padding.height as u32;

            let steps = (shadow.blur as i32).max(1);
            // Rings shrunk past the box's half-size draw nothing; only
            // visit the ones that can (`inset 0 0 2e9px` is 2e9 steps).
            let live = steps.min((pw.min(ph) / 2).min(i32::MAX as u32 - 1) as i32 + 1);
            for i in (0..live).rev() {
                let t = i as f32 / steps as f32;
                let alpha = ((shadow.color.a as f32) * (1.0 - t) * 0.4) as u8;
                if alpha == 0 {
                    continue;
                }
                let shrink = i as u32;
                let color = Color::rgba(shadow.color.r, shadow.color.g, shadow.color.b, alpha);
                let sx = px + shadow.offset_x as i32 + shrink as i32;
                let sy = py + shadow.offset_y as i32 + shrink as i32;
                let sw = pw.saturating_sub(shrink * 2);
                let sh = ph.saturating_sub(shrink * 2);
                if sw == 0 || sh == 0 {
                    continue;
                }
                let thickness = (shadow.spread as u32 + 1).min(sh / 2).min(sw / 2);
                // Top strip
                dl.push(DisplayItem::FillRect {
                    x: sx,
                    y: sy,
                    w: sw,
                    h: thickness,
                    color,
                    node_id: ctx.current_node,
                });
                // Bottom strip
                dl.push(DisplayItem::FillRect {
                    x: sx,
                    y: sy + sh as i32 - thickness as i32,
                    w: sw,
                    h: thickness,
                    color,
                    node_id: ctx.current_node,
                });
                // Left strip
                dl.push(DisplayItem::FillRect {
                    x: sx,
                    y: sy + thickness as i32,
                    w: thickness,
                    h: sh.saturating_sub(thickness * 2),
                    color,
                    node_id: ctx.current_node,
                });
                // Right strip
                dl.push(DisplayItem::FillRect {
                    x: sx + sw as i32 - thickness as i32,
                    y: sy + thickness as i32,
                    w: thickness,
                    h: sh.saturating_sub(thickness * 2),
                    color,
                    node_id: ctx.current_node,
                });
            }
        } else {
            // Emit a single Shadow display item — GPU backends can render
            // this with a Gaussian blur shader during replay.
            let sx = (border.x - ctx.scroll_x + offset_x as f32) as i32;
            let sy = (border.y - ctx.scroll_y + offset_y as f32) as i32;
            let sw = border.width as u32;
            let sh = border.height as u32;
            dl.push(DisplayItem::Shadow {
                x: sx,
                y: sy,
                w: sw,
                h: sh,
                blur: shadow.blur,
                spread: shadow.spread,
                offset_x: shadow.offset_x,
                offset_y: shadow.offset_y,
                color: shadow.color,
                radius,
            });
        }
    }
}
