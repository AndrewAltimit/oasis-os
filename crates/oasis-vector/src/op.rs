//! Vector drawing operations.
//!
//! Each `VectorOp` maps 1:1 to an `SdiBackend` method. Operations are stored
//! as data and dispatched at render time, enabling scenes to be built once
//! and replayed every frame with different transforms or theme colors.

use oasis_types::backend::{Color, GradientStyle};

/// A single vector drawing operation.
///
/// Operations are intentionally flat (no trait objects or callbacks) so they
/// can be cloned, serialized, and inspected. Grouping with transforms and
/// opacity is handled by [`Group`](#variant.Group).
#[derive(Debug, Clone)]
pub enum VectorOp {
    /// Filled rectangle.
    FillRect {
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        color: Color,
    },
    /// Rectangle outline.
    StrokeRect {
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        width: u16,
        color: Color,
    },
    /// Filled rounded rectangle.
    FillRoundedRect {
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        radius: u16,
        color: Color,
    },
    /// Rounded rectangle outline.
    StrokeRoundedRect {
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        radius: u16,
        width: u16,
        color: Color,
    },
    /// Filled convex polygon.
    FillPolygon {
        points: Vec<(i32, i32)>,
        color: Color,
    },
    /// Polygon outline.
    StrokePolygon {
        points: Vec<(i32, i32)>,
        width: u16,
        color: Color,
    },
    /// Filled circle.
    FillCircle {
        cx: i32,
        cy: i32,
        radius: u16,
        color: Color,
    },
    /// Circle outline.
    StrokeCircle {
        cx: i32,
        cy: i32,
        radius: u16,
        width: u16,
        color: Color,
    },
    /// Filled arc (pie wedge). Angles in radians, clockwise from 3 o'clock.
    FillArc {
        cx: i32,
        cy: i32,
        radius: u16,
        start_angle: f32,
        end_angle: f32,
        color: Color,
    },
    /// Arc outline. Angles in radians, clockwise from 3 o'clock.
    StrokeArc {
        cx: i32,
        cy: i32,
        radius: u16,
        start_angle: f32,
        end_angle: f32,
        width: u16,
        color: Color,
    },
    /// Solid line between two points.
    Line {
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        width: u16,
        color: Color,
    },
    /// Dashed line between two points.
    DashedLine {
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        width: u16,
        color: Color,
        dash: u16,
        gap: u16,
    },
    /// Rectangle with gradient fill.
    RectGradient {
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        gradient: GradientStyle,
    },
    /// Polygon with vertical gradient (top color to bottom color).
    PolygonGradient {
        points: Vec<(i32, i32)>,
        color_start: Color,
        color_end: Color,
    },
    /// Filled triangle.
    FillTriangle {
        points: [(i32, i32); 3],
        color: Color,
    },
    /// Text label (for icon labels and HUD text).
    Text {
        text: String,
        x: i32,
        y: i32,
        font_size: u16,
        color: Color,
    },
    /// Nested group with optional translate offset and opacity.
    ///
    /// The translate is applied relative to the parent coordinate space
    /// (via `push_translate`). Opacity (0-255) is multiplied into the alpha
    /// of every op in the group at render time.
    Group {
        ops: Vec<VectorOp>,
        translate: (i32, i32),
        opacity: u8,
    },
}

impl VectorOp {
    /// Create a group with default full opacity and no translation.
    pub fn group(ops: Vec<VectorOp>) -> Self {
        Self::Group {
            ops,
            translate: (0, 0),
            opacity: 255,
        }
    }

    /// Create a translated group at full opacity.
    pub fn translated(dx: i32, dy: i32, ops: Vec<VectorOp>) -> Self {
        Self::Group {
            ops,
            translate: (dx, dy),
            opacity: 255,
        }
    }

    /// Create a group with translation and opacity.
    pub fn group_with(dx: i32, dy: i32, opacity: u8, ops: Vec<VectorOp>) -> Self {
        Self::Group {
            ops,
            translate: (dx, dy),
            opacity,
        }
    }

    /// Apply a color transformation to this op (replaces all colors).
    ///
    /// Useful for theming: build an icon in a base color then recolor it.
    pub fn recolor(&mut self, color: Color) {
        match self {
            Self::FillRect { color: c, .. }
            | Self::FillRoundedRect { color: c, .. }
            | Self::FillPolygon { color: c, .. }
            | Self::FillCircle { color: c, .. }
            | Self::FillArc { color: c, .. }
            | Self::FillTriangle { color: c, .. }
            | Self::Text { color: c, .. } => *c = color,
            Self::StrokeRect { color: c, .. }
            | Self::StrokeRoundedRect { color: c, .. }
            | Self::StrokePolygon { color: c, .. }
            | Self::StrokeCircle { color: c, .. }
            | Self::StrokeArc { color: c, .. }
            | Self::Line { color: c, .. }
            | Self::DashedLine { color: c, .. } => *c = color,
            Self::RectGradient { .. } | Self::PolygonGradient { .. } => {},
            Self::Group { ops, .. } => {
                for op in ops {
                    op.recolor(color);
                }
            },
        }
    }

    /// Scale all coordinates and dimensions by the given factor.
    ///
    /// Used to resize icons from design-space (22x22) to display-space.
    ///
    /// Every coordinate is snapped as an *absolute* position: a point `p`
    /// lands on `round(p * factor)`, and a rectangle keeps the edges
    /// `round(x * factor)` and `round((x + w) * factor)` rather than
    /// scaling its width on its own. Shapes that share an edge or a vertex
    /// in design space (two abutting rects, a rect against a polygon, ops
    /// in differently translated groups) therefore still share it at any
    /// fractional scale. (Truncating `x` and `w` separately, as this used
    /// to, opened 1 px seams between abutting parts of an icon.)
    ///
    /// Radii round too (never below 1); font sizes and rectangle stroke
    /// widths scale down to whole pixels (never below 1); line, circle,
    /// arc and polygon stroke widths and dash patterns are kept as
    /// authored.
    pub fn scale(&mut self, factor: f32) {
        self.scale_at(factor, (0, 0));
    }

    /// [`scale`](Self::scale) for an op whose coordinates are relative to
    /// `origin` (the summed translation of its enclosing groups), so its
    /// absolute positions snap the same way as everyone else's.
    fn scale_at(&mut self, factor: f32, (ox, oy): (i32, i32)) {
        let f = f64::from(factor);
        let snap = |abs: i64| -> i64 { (abs as f64 * f).round() as i64 };
        let to_i32 = |v: i64| v.clamp(i32::MIN as i64, i32::MAX as i64) as i32;
        // A coordinate relative to `o`: snap its absolute position, then
        // express it relative to the snapped origin again.
        let rel = |o: i32, v: &mut i32| {
            *v = to_i32(snap(o as i64 + *v as i64) - snap(o as i64));
        };
        // A span `v .. v + len` relative to `o`: both edges snapped.
        let span = |o: i32, v: &mut i32, len: &mut u32| {
            let start = o as i64 + *v as i64;
            let (a, b) = (snap(start), snap(start + *len as i64));
            *v = to_i32(a - snap(o as i64));
            *len = (b - a).clamp(0, u32::MAX as i64) as u32;
        };
        let radius = |v: &mut u16| *v = (f64::from(*v) * f).round().clamp(1.0, 65535.0) as u16;
        let weight = |v: &mut u16| *v = (f64::from(*v) * f).clamp(1.0, 65535.0) as u16;
        let rect = |x: &mut i32, y: &mut i32, w: &mut u32, h: &mut u32| {
            span(ox, x, w);
            span(oy, y, h);
        };
        let point = |x: &mut i32, y: &mut i32| {
            rel(ox, x);
            rel(oy, y);
        };
        match self {
            Self::FillRect { x, y, w, h, .. } | Self::RectGradient { x, y, w, h, .. } => {
                rect(x, y, w, h);
            },
            Self::StrokeRect {
                x, y, w, h, width, ..
            } => {
                rect(x, y, w, h);
                weight(width);
            },
            Self::FillRoundedRect {
                x,
                y,
                w,
                h,
                radius: r,
                ..
            } => {
                rect(x, y, w, h);
                radius(r);
            },
            Self::StrokeRoundedRect {
                x,
                y,
                w,
                h,
                radius: r,
                width,
                ..
            } => {
                rect(x, y, w, h);
                radius(r);
                weight(width);
            },
            Self::FillPolygon { points, .. }
            | Self::StrokePolygon { points, .. }
            | Self::PolygonGradient { points, .. } => {
                for (px, py) in points.iter_mut() {
                    point(px, py);
                }
            },
            Self::FillTriangle { points, .. } => {
                for (px, py) in points.iter_mut() {
                    point(px, py);
                }
            },
            Self::FillCircle {
                cx, cy, radius: r, ..
            }
            | Self::StrokeCircle {
                cx, cy, radius: r, ..
            }
            | Self::FillArc {
                cx, cy, radius: r, ..
            }
            | Self::StrokeArc {
                cx, cy, radius: r, ..
            } => {
                point(cx, cy);
                radius(r);
            },
            Self::Line { x1, y1, x2, y2, .. } | Self::DashedLine { x1, y1, x2, y2, .. } => {
                point(x1, y1);
                point(x2, y2);
            },
            Self::Text {
                x, y, font_size, ..
            } => {
                point(x, y);
                weight(font_size);
            },
            Self::Group { ops, translate, .. } => {
                // The children's origin is this group's absolute offset.
                let (dx, dy) = *translate;
                let inner = (ox.saturating_add(dx), oy.saturating_add(dy));
                let (tx, ty) = translate;
                rel(ox, tx);
                rel(oy, ty);
                for op in ops {
                    op.scale_at(factor, inner);
                }
            },
        }
    }

    /// Apply an alpha multiplier to this op (modulates existing alpha).
    pub fn modulate_alpha(&mut self, alpha: u8) {
        let modulate = |c: &mut Color| *c = modulate_color(*c, alpha);
        match self {
            Self::FillRect { color, .. }
            | Self::FillRoundedRect { color, .. }
            | Self::FillPolygon { color, .. }
            | Self::FillCircle { color, .. }
            | Self::FillArc { color, .. }
            | Self::FillTriangle { color, .. }
            | Self::Text { color, .. }
            | Self::StrokeRect { color, .. }
            | Self::StrokeRoundedRect { color, .. }
            | Self::StrokePolygon { color, .. }
            | Self::StrokeCircle { color, .. }
            | Self::StrokeArc { color, .. }
            | Self::Line { color, .. }
            | Self::DashedLine { color, .. } => modulate(color),
            Self::RectGradient { gradient, .. } => {
                *gradient = modulate_gradient(*gradient, alpha);
            },
            Self::PolygonGradient {
                color_start,
                color_end,
                ..
            } => {
                modulate(color_start);
                modulate(color_end);
            },
            Self::Group { opacity, .. } => {
                *opacity = ((*opacity as u16 * alpha as u16) / 255) as u8;
            },
        }
    }
}

/// `color` with its alpha multiplied by `alpha / 255`.
pub(crate) fn modulate_color(mut color: Color, alpha: u8) -> Color {
    color.a = ((color.a as u16 * alpha as u16) / 255) as u8;
    color
}

/// `gradient` with every stop's alpha multiplied by `alpha / 255`.
pub(crate) fn modulate_gradient(gradient: GradientStyle, alpha: u8) -> GradientStyle {
    let m = |c: Color| modulate_color(c, alpha);
    match gradient {
        GradientStyle::Vertical { top, bottom } => GradientStyle::Vertical {
            top: m(top),
            bottom: m(bottom),
        },
        GradientStyle::Horizontal { left, right } => GradientStyle::Horizontal {
            left: m(left),
            right: m(right),
        },
        GradientStyle::FourCorner {
            top_left,
            top_right,
            bottom_left,
            bottom_right,
        } => GradientStyle::FourCorner {
            top_left: m(top_left),
            top_right: m(top_right),
            bottom_left: m(bottom_left),
            bottom_right: m(bottom_right),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_group_constructors() {
        let g = VectorOp::group(vec![VectorOp::FillCircle {
            cx: 10,
            cy: 10,
            radius: 5,
            color: Color::WHITE,
        }]);
        let VectorOp::Group {
            ops,
            translate,
            opacity,
        } = g
        else {
            panic!("expected Group, got {g:?}");
        };
        assert_eq!(ops.len(), 1);
        assert_eq!(translate, (0, 0));
        assert_eq!(opacity, 255);
    }

    #[test]
    fn test_translated() {
        let g = VectorOp::translated(10, 20, vec![]);
        let VectorOp::Group { translate, .. } = g else {
            panic!("expected Group, got {g:?}");
        };
        assert_eq!(translate, (10, 20));
    }

    #[test]
    fn test_recolor() {
        let mut op = VectorOp::FillCircle {
            cx: 0,
            cy: 0,
            radius: 5,
            color: Color::WHITE,
        };
        op.recolor(Color::rgb(255, 0, 0));
        let VectorOp::FillCircle { color, .. } = op else {
            panic!("expected FillCircle, got {op:?}");
        };
        assert_eq!(color, Color::rgb(255, 0, 0));
    }

    #[test]
    fn test_recolor_group() {
        let mut g = VectorOp::group(vec![
            VectorOp::FillCircle {
                cx: 0,
                cy: 0,
                radius: 5,
                color: Color::WHITE,
            },
            VectorOp::Line {
                x1: 0,
                y1: 0,
                x2: 10,
                y2: 10,
                width: 1,
                color: Color::WHITE,
            },
        ]);
        let red = Color::rgb(255, 0, 0);
        g.recolor(red);
        let VectorOp::Group { ops, .. } = g else {
            panic!("expected Group, got {g:?}");
        };
        for op in &ops {
            match op {
                VectorOp::FillCircle { color, .. } | VectorOp::Line { color, .. } => {
                    assert_eq!(*color, red);
                },
                _ => panic!("unexpected op"),
            }
        }
    }

    #[test]
    fn test_modulate_alpha() {
        let mut op = VectorOp::FillRect {
            x: 0,
            y: 0,
            w: 10,
            h: 10,
            color: Color::rgba(255, 255, 255, 200),
        };
        op.modulate_alpha(128);
        let VectorOp::FillRect { color, .. } = op else {
            panic!("expected FillRect, got {op:?}");
        };
        // 200 * 128 / 255 = 100
        assert_eq!(color.a, 100);
    }

    fn rect_x_span(op: &VectorOp) -> (i32, i32) {
        match op {
            VectorOp::FillRect { x, w, .. } => (*x, *x + *w as i32),
            other => panic!("not a FillRect: {other:?}"),
        }
    }

    /// Abutting rects keep sharing their edge at any scale (truncating
    /// `x` and `w` separately opened 1 px seams), also across groups.
    #[test]
    fn scale_keeps_shared_edges_shared() {
        let rect = |x: i32, w: u32| VectorOp::FillRect {
            x,
            y: 0,
            w,
            h: 4,
            color: Color::WHITE,
        };
        for i in 1..=300 {
            let f = i as f32 / 97.0;
            let mut parts = vec![rect(0, 5), rect(5, 6), rect(11, 11)];
            for p in &mut parts {
                p.scale(f);
            }
            let spans: Vec<_> = parts.iter().map(rect_x_span).collect();
            assert_eq!(spans[0].1, spans[1].0, "f={f}: {spans:?}");
            assert_eq!(spans[1].1, spans[2].0, "f={f}: {spans:?}");
            assert_eq!(spans[0].0, 0);

            // The same edge inside a translated (nested) group.
            let mut outside = rect(0, 7);
            let mut grouped =
                VectorOp::translated(3, 0, vec![VectorOp::translated(1, 0, vec![rect(3, 9)])]);
            outside.scale(f);
            grouped.scale(f);
            let VectorOp::Group {
                translate: (t0, _),
                ops,
                ..
            } = &grouped
            else {
                unreachable!()
            };
            let VectorOp::Group {
                translate: (t1, _),
                ops: inner,
                ..
            } = &ops[0]
            else {
                unreachable!()
            };
            let (gx0, _) = rect_x_span(&inner[0]);
            assert_eq!(rect_x_span(&outside).1, t0 + t1 + gx0, "f={f}");
        }
    }

    #[test]
    fn scale_snaps_points_and_radii_by_rounding() {
        let mut c = VectorOp::FillCircle {
            cx: 3,
            cy: 5,
            radius: 3,
            color: Color::WHITE,
        };
        c.scale(1.5);
        assert!(matches!(
            c,
            VectorOp::FillCircle {
                cx: 5,
                cy: 8,
                radius: 5,
                ..
            }
        ));
        // Radii never vanish.
        let mut tiny = VectorOp::FillCircle {
            cx: 0,
            cy: 0,
            radius: 1,
            color: Color::WHITE,
        };
        tiny.scale(0.1);
        assert!(matches!(tiny, VectorOp::FillCircle { radius: 1, .. }));
    }
}
