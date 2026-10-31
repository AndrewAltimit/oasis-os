//! Painting a parsed [`SvgElement`] onto an `SdiBackend`: shapes,
//! strokes, gradients and patterns.

use super::*;

// -------------------------------------------------------------------
// Rendering
// -------------------------------------------------------------------

use oasis_types::backend::SdiBackend;
use oasis_types::error::Result;

/// Paint an SVG element into the given backend at the specified position.
///
/// `content_x` and `content_y` are the top-left corner of the content
/// box in screen coordinates. `content_w` and `content_h` are the
/// layout dimensions of the SVG box.
pub fn paint_svg(
    svg: &SvgElement,
    backend: &mut dyn SdiBackend,
    content_x: i32,
    content_y: i32,
    content_w: f32,
    content_h: f32,
) -> Result<()> {
    // Compute scale factors from viewBox (or intrinsic size) to layout box.
    let (vb_x, vb_y, vb_w, vb_h) = svg.viewbox.unwrap_or((0.0, 0.0, svg.width, svg.height));
    let scale_x = if vb_w > 0.0 { content_w / vb_w } else { 1.0 };
    let scale_y = if vb_h > 0.0 { content_h / vb_h } else { 1.0 };

    let xf = SvgTransform {
        ox: content_x,
        oy: content_y,
        vb_x,
        vb_y,
        sx: scale_x,
        sy: scale_y,
        pattern_depth: 0,
    };

    for shape in &svg.shapes {
        paint_shape(shape, backend, &xf, &svg.defs)?;
    }

    Ok(())
}

/// Pre-computed transform from SVG viewBox coordinates to screen pixels.
pub(super) struct SvgTransform {
    ox: i32,
    oy: i32,
    vb_x: f32,
    vb_y: f32,
    sx: f32,
    sy: f32,
    /// How many `<pattern>` fills enclose this paint. Bounded by
    /// [`MAX_PATTERN_DEPTH`] so self-referencing patterns terminate.
    pattern_depth: u8,
}

/// Deepest `<pattern>`-inside-`<pattern>` nesting painted. A pattern
/// whose content fills with itself (`fill="url(#p)"` inside `#p`), or
/// an A -> B -> A cycle, used to recurse until the stack overflowed.
const MAX_PATTERN_DEPTH: u8 = 2;
/// Most pattern tiles painted for one fill.
const MAX_PATTERN_TILES: i64 = 4096;
/// Half-extent of the screen-space square strokes are clipped to before
/// they are stepped: a line to `x2="1e9"` used to issue ~1e9 rects.
const STROKE_CLIP: f64 = 16_384.0;
/// Longest `fill-rule: evenodd` outline triangulated by ear clipping.
const MAX_EAR_CLIP_POINTS: usize = 2048;

/// Paint a single SVG shape.
pub(super) fn paint_shape(
    shape: &SvgShape,
    backend: &mut dyn SdiBackend,
    xf: &SvgTransform,
    defs: &SvgDefs,
) -> Result<()> {
    let (sx, sy) = (xf.sx, xf.sy);
    match shape {
        SvgShape::Rect {
            x,
            y,
            width,
            height,
            fill,
            stroke,
            stroke_width,
            rx,
            opacity,
        } => {
            let (px, py) = xf_point(*x, *y, xf);
            let pw = (width * sx) as u32;
            let ph = (height * sy) as u32;
            if pw == 0 || ph == 0 {
                return Ok(());
            }
            let r_val = (rx * sx.min(sy)) as u16;
            match fill {
                SvgPaint::Color(fc) => {
                    let fc = apply_opacity(*fc, *opacity);
                    if r_val > 0 {
                        backend.fill_rounded_rect(px, py, pw, ph, r_val, fc)?;
                    } else {
                        backend.fill_rect(px, py, pw, ph, fc)?;
                    }
                },
                SvgPaint::GradientRef(id) => {
                    if let Some(grad) = defs.gradients.get(id.as_str()) {
                        let grad = normalize_gradient_for_rect(grad, *x, *y, *width, *height);
                        paint_rect_gradient(backend, px, py, pw, ph, &grad, *opacity)?;
                    } else if let Some(pat) = defs.patterns.get(id.as_str()) {
                        paint_pattern_fill(backend, px, py, pw, ph, pat, xf, defs, *opacity)?;
                    }
                },
                SvgPaint::None => {},
            }
            if let Some(sc) = stroke.as_color() {
                let sc = apply_opacity(sc, *opacity);
                let sw = (stroke_width * sx.min(sy)).max(1.0) as u32;
                backend.fill_rect(px, py, pw, sw, sc)?;
                backend.fill_rect(px, py + ph as i32 - sw as i32, pw, sw, sc)?;
                backend.fill_rect(px, py + sw as i32, sw, ph.saturating_sub(sw * 2), sc)?;
                backend.fill_rect(
                    px + pw as i32 - sw as i32,
                    py + sw as i32,
                    sw,
                    ph.saturating_sub(sw * 2),
                    sc,
                )?;
            }
        },
        SvgShape::Circle {
            cx,
            cy,
            r,
            fill,
            stroke,
            stroke_width,
            opacity,
        } => {
            let (px, py) = xf_point(*cx, *cy, xf);
            let radius = (r * sx.min(sy)) as u16;
            if radius == 0 {
                return Ok(());
            }
            if let Some(fc) = fill.as_color() {
                backend.fill_circle(px, py, radius, apply_opacity(fc, *opacity))?;
            }
            if let Some(sc) = stroke.as_color() {
                let sw = (stroke_width * sx.min(sy)).max(1.0) as u16;
                backend.stroke_circle(px, py, radius, sw, apply_opacity(sc, *opacity))?;
            }
        },
        SvgShape::Ellipse {
            cx,
            cy,
            rx,
            ry,
            fill,
            stroke,
            stroke_width,
            opacity,
        } => {
            // Clamped so `2 * r` and `centre - r` stay in range; a
            // negative radius disables rendering (SVG 2 10.4).
            let erx = ((rx * sx) as i32).clamp(0, i32::MAX / 4);
            let ery = ((ry * sy) as i32).clamp(0, i32::MAX / 4);
            if erx == 0 || ery == 0 {
                return Ok(());
            }
            let (cpx, cpy) = xf_point(*cx, *cy, xf);
            let (px, py) = (cpx.saturating_sub(erx), cpy.saturating_sub(ery));
            let (pw, ph) = (erx as u32 * 2, ery as u32 * 2);
            let r_val = (erx as u16).min(ery as u16);
            if let Some(fc) = fill.as_color() {
                backend.fill_rounded_rect(px, py, pw, ph, r_val, apply_opacity(fc, *opacity))?;
            }
            if let Some(sc) = stroke.as_color() {
                let sw = (stroke_width * sx.min(sy)).max(1.0) as u16;
                backend.stroke_rounded_rect(
                    px,
                    py,
                    pw,
                    ph,
                    r_val,
                    sw,
                    apply_opacity(sc, *opacity),
                )?;
            }
        },
        SvgShape::Line {
            x1,
            y1,
            x2,
            y2,
            stroke,
            stroke_width,
            opacity,
        } => {
            let sc = match stroke.as_color() {
                Some(c) => apply_opacity(c, *opacity),
                None => return Ok(()),
            };
            let (px1, py1) = xf_point(*x1, *y1, xf);
            let (px2, py2) = xf_point(*x2, *y2, xf);
            let sw = (stroke_width * sx.min(sy)).max(1.0) as u32;
            stroke_line_bresenham(backend, px1, py1, px2, py2, sw, sc)?;
        },
        SvgShape::Text {
            x,
            y,
            text,
            fill,
            font_size,
            text_anchor,
            letter_spacing,
            font_weight,
            opacity,
            spans,
        } => {
            let scaled_fs = (font_size * sy).max(1.0) as u16;
            let fc = fill.as_color().unwrap_or(Color::rgb(0, 0, 0));
            let fc = apply_opacity(fc, *opacity);

            // Compute total text (main + spans) for text-anchor measurement.
            let mut full = text.clone();
            for span in spans {
                full.push_str(&span.text);
            }

            let (mut px, py) = xf_point(*x, *y, xf);
            let py = py.saturating_sub(i32::from(scaled_fs));

            // text-anchor adjustment.
            match text_anchor {
                TextAnchor::Middle => {
                    let tw = backend.measure_text(&full, scaled_fs);
                    px -= tw as i32 / 2;
                },
                TextAnchor::End => {
                    let tw = backend.measure_text(&full, scaled_fs);
                    px -= tw as i32;
                },
                TextAnchor::Start => {},
            }

            let bold = *font_weight >= 700;
            if *letter_spacing > 0.1 {
                // Character-by-character rendering with spacing.
                let ls = (*letter_spacing * sx) as i32;
                let mut cx = px;
                for ch in full.chars() {
                    let mut buf = [0u8; 4];
                    let s = ch.encode_utf8(&mut buf);
                    backend.draw_text_styled(s, cx, py, scaled_fs, fc, bold, false)?;
                    let cw = backend.measure_text(s, scaled_fs) as i32;
                    cx += cw + ls;
                }
            } else {
                backend.draw_text_styled(&full, px, py, scaled_fs, fc, bold, false)?;
            }
        },
        SvgShape::Path {
            points,
            fill,
            stroke,
            stroke_width,
            fill_rule,
            stroke_linecap,
            stroke_linejoin,
            opacity,
        }
        | SvgShape::Polygon {
            points,
            fill,
            stroke,
            stroke_width,
            fill_rule,
            stroke_linecap,
            stroke_linejoin,
            opacity,
        } => {
            paint_polygon_shape(
                points,
                fill,
                stroke,
                *stroke_width,
                *fill_rule,
                *stroke_linecap,
                *stroke_linejoin,
                *opacity,
                backend,
                xf,
                defs,
            )?;
        },
        SvgShape::Polyline {
            points,
            stroke,
            stroke_width,
            opacity,
        } => {
            if let Some(sc) = stroke.as_color() {
                let sc = apply_opacity(sc, *opacity);
                let sw = (stroke_width * sx.min(sy)).max(1.0) as u32;
                for window in points.windows(2) {
                    let (px1, py1) = xf_point(window[0].0, window[0].1, xf);
                    let (px2, py2) = xf_point(window[1].0, window[1].1, xf);
                    stroke_line_bresenham(backend, px1, py1, px2, py2, sw, sc)?;
                }
            }
        },
    }
    Ok(())
}

/// Transform a point from SVG viewBox coordinates to screen pixels.
///
/// Saturating: hostile coordinates (`1e30`) pin to the `i32` range
/// instead of overflowing.
pub(super) fn xf_point(x: f32, y: f32, xf: &SvgTransform) -> (i32, i32) {
    (
        xf.ox.saturating_add(((x - xf.vb_x) * xf.sx) as i32),
        xf.oy.saturating_add(((y - xf.vb_y) * xf.sy) as i32),
    )
}

/// Paint a filled and/or stroked polygon (used by Path and Polygon shapes).
#[allow(clippy::too_many_arguments)]
pub(super) fn paint_polygon_shape(
    points: &[(f32, f32)],
    fill: &SvgPaint,
    stroke: &SvgPaint,
    stroke_width: f32,
    fill_rule: FillRule,
    linecap: LineCap,
    linejoin: LineJoin,
    opacity: f32,
    backend: &mut dyn SdiBackend,
    xf: &SvgTransform,
    defs: &SvgDefs,
) -> Result<()> {
    if points.len() < 2 {
        return Ok(());
    }
    let screen_pts: Vec<(i32, i32)> = points.iter().map(|&(x, y)| xf_point(x, y, xf)).collect();

    match fill {
        SvgPaint::Color(fc) => {
            let fc = apply_opacity(*fc, opacity);
            // Ear clipping is O(n^3) worst case; long outlines fall back
            // to the backend's polygon fill.
            if fill_rule == FillRule::EvenOdd
                && (3..=MAX_EAR_CLIP_POINTS).contains(&screen_pts.len())
            {
                let float_pts: Vec<(f32, f32)> = screen_pts
                    .iter()
                    .map(|&(x, y)| (x as f32, y as f32))
                    .collect();
                let triangles = crate::transform::ear_clip_triangulate(&float_pts);
                for tri in &triangles {
                    let p0 = screen_pts[tri[0]];
                    let p1 = screen_pts[tri[1]];
                    let p2 = screen_pts[tri[2]];
                    backend.fill_polygon(&[p0, p1, p2], fc)?;
                }
            } else {
                backend.fill_polygon(&screen_pts, fc)?;
            }
        },
        SvgPaint::GradientRef(id) => {
            // Gradient fill for polygons: compute AABB and fill with
            // the gradient's midpoint color (approximation — full
            // per-scanline gradient clipped to polygon is a follow-up).
            if let Some(grad) = defs.gradients.get(id.as_str()) {
                let mid_color = sample_svg_gradient_at(grad, 0.5, opacity);
                backend.fill_polygon(&screen_pts, mid_color)?;
            } else if let Some(pat) = defs.patterns.get(id.as_str())
                && let Some((px, py, pw, ph)) = polygon_aabb(&screen_pts)
            {
                paint_pattern_fill(backend, px, py, pw, ph, pat, xf, defs, opacity)?;
            }
        },
        SvgPaint::None => {},
    }
    if let Some(sc) = stroke.as_color() {
        let sc = apply_opacity(sc, opacity);
        let sw = (stroke_width * xf.sx.min(xf.sy)).max(1.0) as u32;
        for window in screen_pts.windows(2) {
            stroke_line_bresenham(
                backend,
                window[0].0,
                window[0].1,
                window[1].0,
                window[1].1,
                sw,
                sc,
            )?;

            // Line join at vertices (skip first segment).
            if linejoin == LineJoin::Round {
                let r = (sw / 2).max(1) as u16;
                backend.fill_circle(window[1].0, window[1].1, r, sc)?;
            }
        }
        // Close the path (last → first).
        if let (Some(last), Some(first)) = (screen_pts.last(), screen_pts.first())
            && last != first
        {
            stroke_line_bresenham(backend, last.0, last.1, first.0, first.1, sw, sc)?;
        }

        // Line caps at endpoints.
        match linecap {
            LineCap::Round => {
                let r = (sw / 2).max(1) as u16;
                if let Some(first) = screen_pts.first() {
                    backend.fill_circle(first.0, first.1, r, sc)?;
                }
                if let Some(last) = screen_pts.last() {
                    backend.fill_circle(last.0, last.1, r, sc)?;
                }
            },
            LineCap::Square => {
                // Square cap extends by half stroke width beyond endpoints.
                // Approximated by drawing a small rect at each endpoint.
                let half = (sw / 2).max(1);
                if let Some(first) = screen_pts.first() {
                    backend.fill_rect(first.0 - half as i32, first.1 - half as i32, sw, sw, sc)?;
                }
                if let Some(last) = screen_pts.last() {
                    backend.fill_rect(last.0 - half as i32, last.1 - half as i32, sw, sw, sc)?;
                }
            },
            LineCap::Butt => {}, // Default: no extension
        }
    }
    Ok(())
}

// -------------------------------------------------------------------
// Gradient and pattern rendering
// -------------------------------------------------------------------

pub(super) fn polygon_aabb(pts: &[(i32, i32)]) -> Option<(i32, i32, u32, u32)> {
    let first = pts.first()?;
    let (mut min_x, mut min_y) = *first;
    let (mut max_x, mut max_y) = *first;
    for &(x, y) in &pts[1..] {
        if x < min_x {
            min_x = x;
        }
        if y < min_y {
            min_y = y;
        }
        if x > max_x {
            max_x = x;
        }
        if y > max_y {
            max_y = y;
        }
    }
    let w = (max_x - min_x).max(0) as u32;
    let h = (max_y - min_y).max(0) as u32;
    if w == 0 || h == 0 {
        return None;
    }
    Some((min_x, min_y, w, h))
}

/// Apply opacity to a color by multiplying its alpha channel.
pub(super) fn apply_opacity(c: Color, opacity: f32) -> Color {
    if (opacity - 1.0).abs() < f32::EPSILON {
        return c;
    }
    Color::rgba(
        c.r,
        c.g,
        c.b,
        (c.a as f32 * opacity.clamp(0.0, 1.0)).round() as u8,
    )
}

/// Sample a gradient at position `t` (0..=1) with opacity applied.
pub(super) fn sample_svg_gradient_at(grad: &SvgGradientDef, t: f32, opacity: f32) -> Color {
    let stops = match grad {
        SvgGradientDef::Linear { stops, .. } | SvgGradientDef::Radial { stops, .. } => stops,
    };
    let c = sample_svg_stops(stops, t);
    apply_opacity(c, opacity)
}

/// Linearly interpolate between gradient stops at position `t`.
pub(super) fn sample_svg_stops(stops: &[SvgGradientStop], t: f32) -> Color {
    if stops.is_empty() {
        return Color::rgba(0, 0, 0, 0);
    }
    if t <= stops[0].offset {
        return stops[0].color;
    }
    let last = stops.len() - 1;
    if t >= stops[last].offset {
        return stops[last].color;
    }
    for i in 0..last {
        if t >= stops[i].offset && t <= stops[i + 1].offset {
            let range = stops[i + 1].offset - stops[i].offset;
            let local_t = if range > 0.0 {
                (t - stops[i].offset) / range
            } else {
                0.0
            };
            return lerp_color(stops[i].color, stops[i + 1].color, local_t);
        }
    }
    stops[last].color
}

/// Linear color interpolation.
pub(super) fn lerp_color(a: Color, b: Color, t: f32) -> Color {
    let inv = 1.0 - t;
    Color::rgba(
        (a.r as f32 * inv + b.r as f32 * t).round() as u8,
        (a.g as f32 * inv + b.g as f32 * t).round() as u8,
        (a.b as f32 * inv + b.b as f32 * t).round() as u8,
        (a.a as f32 * inv + b.a as f32 * t).round() as u8,
    )
}

/// Render a gradient fill for a rectangular region.
/// Normalize a gradient's coordinates to objectBoundingBox (0..1) for a rect
/// in viewBox-space `(rx, ry, rw, rh)`. When `user_space` is true the
/// gradient coords are absolute viewBox coords; divide out the rect origin
/// and size so downstream band sampling can use 0..1 positions uniformly.
/// When the rect is degenerate the gradient is returned unchanged.
pub(super) fn normalize_gradient_for_rect(
    grad: &SvgGradientDef,
    rx: f32,
    ry: f32,
    rw: f32,
    rh: f32,
) -> SvgGradientDef {
    match grad {
        SvgGradientDef::Linear {
            x1,
            y1,
            x2,
            y2,
            stops,
            user_space,
        } => {
            if !*user_space || rw.abs() < f32::EPSILON || rh.abs() < f32::EPSILON {
                return grad.clone();
            }
            SvgGradientDef::Linear {
                x1: (*x1 - rx) / rw,
                y1: (*y1 - ry) / rh,
                x2: (*x2 - rx) / rw,
                y2: (*y2 - ry) / rh,
                stops: stops.clone(),
                user_space: false,
            }
        },
        SvgGradientDef::Radial {
            cx,
            cy,
            r,
            stops,
            user_space,
        } => {
            if !*user_space || rw.abs() < f32::EPSILON || rh.abs() < f32::EPSILON {
                return grad.clone();
            }
            let denom = rw.max(rh);
            SvgGradientDef::Radial {
                cx: (*cx - rx) / rw,
                cy: (*cy - ry) / rh,
                r: *r / denom,
                stops: stops.clone(),
                user_space: false,
            }
        },
    }
}

pub(super) fn paint_rect_gradient(
    backend: &mut dyn SdiBackend,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    grad: &SvgGradientDef,
    opacity: f32,
) -> Result<()> {
    match grad {
        SvgGradientDef::Linear {
            x1,
            y1,
            x2,
            y2,
            stops,
            ..
        } => {
            if stops.is_empty() || w == 0 || h == 0 {
                return Ok(());
            }
            // Determine gradient direction.
            let is_vertical = (*x2 - *x1).abs() < 0.001;
            let is_horizontal = (*y2 - *y1).abs() < 0.001;
            if is_vertical {
                // Vertical gradient: render horizontal bands.
                let bands = (h as usize).clamp(1, 128);
                let band_h = h as f32 / bands as f32;
                for i in 0..bands {
                    let t_top = *y1 + (*y2 - *y1) * (i as f32 / bands as f32);
                    let t_bot = *y1 + (*y2 - *y1) * ((i as f32 + 1.0) / bands as f32);
                    let t = (t_top + t_bot) / 2.0;
                    let c = sample_svg_gradient_at(grad, t, opacity);
                    let by = y + (i as f32 * band_h) as i32;
                    let bh = (band_h.ceil() as u32).max(1);
                    backend.fill_rect(x, by, w, bh, c)?;
                }
            } else if is_horizontal {
                // Horizontal gradient: render vertical bands.
                let bands = (w as usize).clamp(1, 128);
                let band_w = w as f32 / bands as f32;
                for i in 0..bands {
                    let t_l = *x1 + (*x2 - *x1) * (i as f32 / bands as f32);
                    let t_r = *x1 + (*x2 - *x1) * ((i as f32 + 1.0) / bands as f32);
                    let t = (t_l + t_r) / 2.0;
                    let c = sample_svg_gradient_at(grad, t, opacity);
                    let bx = x + (i as f32 * band_w) as i32;
                    let bw = (band_w.ceil() as u32).max(1);
                    backend.fill_rect(bx, y, bw, h, c)?;
                }
            } else {
                // Diagonal: approximate with vertical bands.
                let bands = (w.max(h) as usize).clamp(1, 128);
                for i in 0..bands {
                    let t = (i as f32 + 0.5) / bands as f32;
                    let c = sample_svg_gradient_at(grad, t, opacity);
                    let bx = x + (w as f32 * i as f32 / bands as f32) as i32;
                    let bw = (w as f32 / bands as f32).ceil() as u32;
                    backend.fill_rect(bx, y, bw.max(1), h, c)?;
                }
            }
        },
        SvgGradientDef::Radial {
            cx, cy, r, stops, ..
        } => {
            if stops.is_empty() || w == 0 || h == 0 {
                return Ok(());
            }
            // Radial gradient: concentric bands from outside in.
            let center_x = x + (w as f32 * cx) as i32;
            let center_y = y + (h as f32 * cy) as i32;
            let max_r = (w.max(h) as f32 * r).max(1.0);
            let bands = (max_r as usize).clamp(8, 64);
            // Paint outside-in so inner bands overwrite outer.
            for i in 0..bands {
                let t = 1.0 - (i as f32 / bands as f32);
                let c = sample_svg_gradient_at(grad, t, opacity);
                let band_r = (max_r * (1.0 - i as f32 / bands as f32)) as i32;
                if band_r <= 0 {
                    continue;
                }
                let bx = center_x - band_r;
                let by = center_y - band_r;
                let bw = (band_r * 2) as u32;
                let bh = bw;
                let corner_r = band_r as u16;
                backend.fill_rounded_rect(bx, by, bw, bh, corner_r, c)?;
            }
        },
    }
    Ok(())
}

/// Render a pattern tile fill over a rectangular region.
#[allow(clippy::too_many_arguments)]
pub(super) fn paint_pattern_fill(
    backend: &mut dyn SdiBackend,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    pattern: &SvgPatternDef,
    parent_xf: &SvgTransform,
    defs: &SvgDefs,
    opacity: f32,
) -> Result<()> {
    if pattern.width <= 0.0 || pattern.height <= 0.0 {
        return Ok(());
    }
    let tile_w = (pattern.width * parent_xf.sx) as i32;
    let tile_h = (pattern.height * parent_xf.sy) as i32;
    if tile_w <= 0 || tile_h <= 0 {
        return Ok(());
    }

    if parent_xf.pattern_depth >= MAX_PATTERN_DEPTH {
        return Ok(());
    }

    let cols = (i64::from(w) / i64::from(tile_w) + 1).min(MAX_PATTERN_TILES);
    let rows = (i64::from(h) / i64::from(tile_h) + 1).min((MAX_PATTERN_TILES / cols).max(1));

    for row in 0..rows {
        for col in 0..cols {
            let tile_x = (i64::from(x) + col * i64::from(tile_w)) as i32;
            let tile_y = (i64::from(y) + row * i64::from(tile_h)) as i32;
            // Create a mini-transform for this tile.
            let tile_xf = SvgTransform {
                ox: tile_x,
                oy: tile_y,
                vb_x: 0.0,
                vb_y: 0.0,
                sx: parent_xf.sx,
                sy: parent_xf.sy,
                pattern_depth: parent_xf.pattern_depth + 1,
            };
            for shape in &pattern.shapes {
                // Apply opacity to pattern contents.
                paint_shape_with_extra_opacity(shape, backend, &tile_xf, defs, opacity)?;
            }
        }
    }
    Ok(())
}

/// Paint a shape with an additional opacity multiplier from a parent pattern.
pub(super) fn paint_shape_with_extra_opacity(
    shape: &SvgShape,
    backend: &mut dyn SdiBackend,
    xf: &SvgTransform,
    defs: &SvgDefs,
    extra_opacity: f32,
) -> Result<()> {
    if (extra_opacity - 1.0).abs() < f32::EPSILON {
        return paint_shape(shape, backend, xf, defs);
    }
    let mut shape = shape.clone();
    match &mut shape {
        SvgShape::Rect { opacity, .. }
        | SvgShape::Circle { opacity, .. }
        | SvgShape::Ellipse { opacity, .. }
        | SvgShape::Line { opacity, .. }
        | SvgShape::Text { opacity, .. }
        | SvgShape::Path { opacity, .. }
        | SvgShape::Polygon { opacity, .. }
        | SvgShape::Polyline { opacity, .. } => {
            *opacity = (*opacity * extra_opacity).clamp(0.0, 1.0);
        },
    }
    paint_shape(&shape, backend, xf, defs)
}

/// Stroke a line between two points using filled rectangles (Bresenham-like).
pub(super) fn stroke_line_bresenham(
    backend: &mut dyn SdiBackend,
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
    sw: u32,
    color: Color,
) -> Result<()> {
    let Some((x1, y1, x2, y2)) = clip_segment(x1, y1, x2, y2) else {
        return Ok(());
    };
    let dx = (x2 - x1).abs();
    let dy = (y2 - y1).abs();
    if dx == 0 && dy == 0 {
        backend.fill_rect(x1, y1, sw, sw, color)?;
    } else if dx == 0 || dy == 0 {
        let lx = x1.min(x2);
        let ly = y1.min(y2);
        let lw = (dx as u32).max(sw);
        let lh = (dy as u32).max(sw);
        backend.fill_rect(lx, ly, lw, lh, color)?;
    } else {
        let steps = dx.max(dy);
        for s in 0..=steps {
            let t = s as f32 / steps.max(1) as f32;
            let px = x1 + ((x2 - x1) as f32 * t) as i32;
            let py = y1 + ((y2 - y1) as f32 * t) as i32;
            backend.fill_rect(px, py, sw, sw, color)?;
        }
    }
    Ok(())
}

/// Clip a segment to the [`STROKE_CLIP`] square (Liang-Barsky), so a
/// stroke is stepped only over the part that can reach a surface.
/// `None` when the segment misses the square entirely.
fn clip_segment(x1: i32, y1: i32, x2: i32, y2: i32) -> Option<(i32, i32, i32, i32)> {
    let (x1f, y1f) = (f64::from(x1), f64::from(y1));
    let (dx, dy) = (f64::from(x2) - x1f, f64::from(y2) - y1f);
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    for (p, q) in [
        (-dx, x1f + STROKE_CLIP),
        (dx, STROKE_CLIP - x1f),
        (-dy, y1f + STROKE_CLIP),
        (dy, STROKE_CLIP - y1f),
    ] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
        } else {
            let r = q / p;
            if p < 0.0 {
                t0 = t0.max(r);
            } else {
                t1 = t1.min(r);
            }
            if t0 > t1 {
                return None;
            }
        }
    }
    let at = |t: f64| ((x1f + dx * t).round() as i32, (y1f + dy * t).round() as i32);
    let (a, b) = (at(t0), at(t1));
    Some((a.0, a.1, b.0, b.1))
}
