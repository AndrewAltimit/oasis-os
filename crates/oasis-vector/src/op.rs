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
    /// The translate is applied relative to the parent coordinate space.
    /// Opacity (0-255) is applied to the group as a whole via `push_translate`
    /// and per-op alpha modulation.
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
    pub fn scale(&mut self, factor: f32) {
        let s = |v: &mut i32| *v = (*v as f32 * factor) as i32;
        let su = |v: &mut u32| *v = (*v as f32 * factor) as u32;
        let su16 = |v: &mut u16| *v = (*v as f32 * factor).max(1.0) as u16;
        match self {
            Self::FillRect { x, y, w, h, .. } => {
                s(x);
                s(y);
                su(w);
                su(h);
            },
            Self::StrokeRect {
                x, y, w, h, width, ..
            } => {
                s(x);
                s(y);
                su(w);
                su(h);
                su16(width);
            },
            Self::FillRoundedRect {
                x, y, w, h, radius, ..
            } => {
                s(x);
                s(y);
                su(w);
                su(h);
                su16(radius);
            },
            Self::StrokeRoundedRect {
                x,
                y,
                w,
                h,
                radius,
                width,
                ..
            } => {
                s(x);
                s(y);
                su(w);
                su(h);
                su16(radius);
                su16(width);
            },
            Self::FillPolygon { points, .. } | Self::StrokePolygon { points, .. } => {
                for (px, py) in points.iter_mut() {
                    s(px);
                    s(py);
                }
            },
            Self::FillCircle { cx, cy, radius, .. } | Self::StrokeCircle { cx, cy, radius, .. } => {
                s(cx);
                s(cy);
                su16(radius);
            },
            Self::FillArc { cx, cy, radius, .. } | Self::StrokeArc { cx, cy, radius, .. } => {
                s(cx);
                s(cy);
                su16(radius);
            },
            Self::Line { x1, y1, x2, y2, .. } | Self::DashedLine { x1, y1, x2, y2, .. } => {
                s(x1);
                s(y1);
                s(x2);
                s(y2);
            },
            Self::RectGradient { x, y, w, h, .. } => {
                s(x);
                s(y);
                su(w);
                su(h);
            },
            Self::PolygonGradient { points, .. } => {
                for (px, py) in points.iter_mut() {
                    s(px);
                    s(py);
                }
            },
            Self::FillTriangle { points, .. } => {
                for (px, py) in points.iter_mut() {
                    s(px);
                    s(py);
                }
            },
            Self::Text {
                x, y, font_size, ..
            } => {
                s(x);
                s(y);
                su16(font_size);
            },
            Self::Group { ops, translate, .. } => {
                let (dx, dy) = translate;
                *dx = (*dx as f32 * factor) as i32;
                *dy = (*dy as f32 * factor) as i32;
                for op in ops {
                    op.scale(factor);
                }
            },
        }
    }

    /// Apply an alpha multiplier to this op (modulates existing alpha).
    pub fn modulate_alpha(&mut self, alpha: u8) {
        let modulate = |c: &mut Color| {
            c.a = ((c.a as u16 * alpha as u16) / 255) as u8;
        };
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
            Self::RectGradient { .. } | Self::PolygonGradient { .. } => {},
            Self::Group { opacity, .. } => {
                *opacity = ((*opacity as u16 * alpha as u16) / 255) as u8;
            },
        }
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
        match g {
            VectorOp::Group {
                ops,
                translate,
                opacity,
            } => {
                assert_eq!(ops.len(), 1);
                assert_eq!(translate, (0, 0));
                assert_eq!(opacity, 255);
            },
            _ => panic!("expected Group"),
        }
    }

    #[test]
    fn test_translated() {
        let g = VectorOp::translated(10, 20, vec![]);
        match g {
            VectorOp::Group { translate, .. } => assert_eq!(translate, (10, 20)),
            _ => panic!("expected Group"),
        }
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
        match op {
            VectorOp::FillCircle { color, .. } => assert_eq!(color, Color::rgb(255, 0, 0)),
            _ => panic!("expected FillCircle"),
        }
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
        match g {
            VectorOp::Group { ops, .. } => {
                for op in &ops {
                    match op {
                        VectorOp::FillCircle { color, .. } | VectorOp::Line { color, .. } => {
                            assert_eq!(*color, red);
                        },
                        _ => panic!("unexpected op"),
                    }
                }
            },
            _ => panic!("expected Group"),
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
        match op {
            VectorOp::FillRect { color, .. } => {
                // 200 * 128 / 255 = 100
                assert_eq!(color.a, 100);
            },
            _ => panic!("expected FillRect"),
        }
    }
}
