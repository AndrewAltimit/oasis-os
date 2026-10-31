//! Rasterizer: dispatches `VectorOp`s to `SdiBackend` calls.
//!
//! The renderer walks a scene's operation list and translates each `VectorOp`
//! into the corresponding `SdiBackend` method call. Groups are handled by
//! pushing/popping translate state and modulating alpha.

use oasis_types::backend::SdiBackend;
use oasis_types::error::Result;

use oasis_types::backend::Color;
use oasis_types::rasterize::polygon_rows;

use crate::op::{VectorOp, modulate_color, modulate_gradient};
use crate::scene::VectorScene;

/// Render a complete scene to the given backend.
///
/// Operations are dispatched in order (back to front). The backend's current
/// translate and clip state is used as the base coordinate space.
pub fn render_scene(backend: &mut dyn SdiBackend, scene: &VectorScene) -> Result<()> {
    render_ops(backend, &scene.ops, 255)
}

/// Render a scene at a specific position with optional alpha.
pub fn render_scene_at(
    backend: &mut dyn SdiBackend,
    scene: &VectorScene,
    x: i32,
    y: i32,
    alpha: u8,
) -> Result<()> {
    backend.push_translate(x, y)?;
    let result = render_ops(backend, &scene.ops, alpha);
    backend.pop_translate()?;
    result
}

/// Render a slice of operations with a group-level alpha multiplier.
///
/// Public so callers that cache pre-built op lists (e.g. static background
/// or chrome layers) can render them without assembling a `VectorScene`.
pub fn render_ops(backend: &mut dyn SdiBackend, ops: &[VectorOp], group_alpha: u8) -> Result<()> {
    for op in ops {
        render_op(backend, op, group_alpha)?;
    }
    Ok(())
}

/// Blend an op color with the group alpha.
fn apply_alpha(color: Color, group_alpha: u8) -> Color {
    if group_alpha < 255 {
        modulate_color(color, group_alpha)
    } else {
        color
    }
}

/// Fill a polygon (even-odd, the same rows as `SdiVector::fill_polygon`)
/// with a vertical gradient: `top` on the polygon's first row, `bottom` on
/// its last, one `fill_rect` per span.
fn fill_polygon_vertical_gradient(
    backend: &mut dyn SdiBackend,
    points: &[(i32, i32)],
    top: Color,
    bottom: Color,
) -> Result<()> {
    if top == bottom {
        return backend.fill_polygon(points, top);
    }
    let y_min = points.iter().map(|p| p.1).min().unwrap_or(0);
    let y_max = points.iter().map(|p| p.1).max().unwrap_or(0);
    let span = (y_max as i64 - y_min as i64).max(1) as f32;
    let mut xs = Vec::new();
    let mut result = Ok(());
    let bounds = backend.draw_bounds();
    polygon_rows(points, &mut xs, bounds, |y, x0, x1| {
        if result.is_ok() {
            let t = (y as i64 - y_min as i64) as f32 / span;
            let color = top.lerp(bottom, t);
            result = backend.fill_rect(x0, y, (x1 - x0) as u32, 1, color);
        }
    });
    result
}

/// Dispatch a single `VectorOp` to backend calls.
fn render_op(backend: &mut dyn SdiBackend, op: &VectorOp, group_alpha: u8) -> Result<()> {
    match op {
        VectorOp::FillRect { x, y, w, h, color } => {
            backend.fill_rect(*x, *y, *w, *h, apply_alpha(*color, group_alpha))
        },
        VectorOp::StrokeRect {
            x,
            y,
            w,
            h,
            width,
            color,
        } => backend.stroke_rect(*x, *y, *w, *h, *width, apply_alpha(*color, group_alpha)),
        VectorOp::FillRoundedRect {
            x,
            y,
            w,
            h,
            radius,
            color,
        } => backend.fill_rounded_rect(*x, *y, *w, *h, *radius, apply_alpha(*color, group_alpha)),
        VectorOp::StrokeRoundedRect {
            x,
            y,
            w,
            h,
            radius,
            width,
            color,
        } => backend.stroke_rounded_rect(
            *x,
            *y,
            *w,
            *h,
            *radius,
            *width,
            apply_alpha(*color, group_alpha),
        ),
        VectorOp::FillPolygon { points, color } => {
            backend.fill_polygon(points, apply_alpha(*color, group_alpha))
        },
        VectorOp::StrokePolygon {
            points,
            width,
            color,
        } => backend.stroke_polygon(points, *width, apply_alpha(*color, group_alpha)),
        VectorOp::FillCircle {
            cx,
            cy,
            radius,
            color,
        } => backend.fill_circle(*cx, *cy, *radius, apply_alpha(*color, group_alpha)),
        VectorOp::StrokeCircle {
            cx,
            cy,
            radius,
            width,
            color,
        } => backend.stroke_circle(*cx, *cy, *radius, *width, apply_alpha(*color, group_alpha)),
        VectorOp::FillArc {
            cx,
            cy,
            radius,
            start_angle,
            end_angle,
            color,
        } => backend.fill_arc(
            *cx,
            *cy,
            *radius,
            *start_angle,
            *end_angle,
            apply_alpha(*color, group_alpha),
        ),
        VectorOp::StrokeArc {
            cx,
            cy,
            radius,
            start_angle,
            end_angle,
            width,
            color,
        } => backend.stroke_arc(
            *cx,
            *cy,
            *radius,
            *start_angle,
            *end_angle,
            *width,
            apply_alpha(*color, group_alpha),
        ),
        VectorOp::Line {
            x1,
            y1,
            x2,
            y2,
            width,
            color,
        } => backend.draw_line(*x1, *y1, *x2, *y2, *width, apply_alpha(*color, group_alpha)),
        VectorOp::DashedLine {
            x1,
            y1,
            x2,
            y2,
            width,
            color,
            dash,
            gap,
        } => backend.stroke_line_dashed(
            *x1,
            *y1,
            *x2,
            *y2,
            *width,
            apply_alpha(*color, group_alpha),
            *dash,
            *gap,
        ),
        VectorOp::RectGradient {
            x,
            y,
            w,
            h,
            gradient,
        } => {
            if group_alpha < 255 {
                let faded = modulate_gradient(*gradient, group_alpha);
                backend.fill_rect_gradient(*x, *y, *w, *h, &faded)
            } else {
                backend.fill_rect_gradient(*x, *y, *w, *h, gradient)
            }
        },
        VectorOp::PolygonGradient {
            points,
            color_start,
            color_end,
        } => fill_polygon_vertical_gradient(
            backend,
            points,
            apply_alpha(*color_start, group_alpha),
            apply_alpha(*color_end, group_alpha),
        ),
        VectorOp::FillTriangle { points, color } => backend.fill_triangle(
            points[0].0,
            points[0].1,
            points[1].0,
            points[1].1,
            points[2].0,
            points[2].1,
            apply_alpha(*color, group_alpha),
        ),
        VectorOp::Text {
            text,
            x,
            y,
            font_size,
            color,
        } => backend.draw_text(text, *x, *y, *font_size, apply_alpha(*color, group_alpha)),
        VectorOp::Group {
            ops,
            translate,
            opacity,
        } => {
            let combined_alpha = if group_alpha == 255 {
                *opacity
            } else {
                ((*opacity as u16 * group_alpha as u16) / 255) as u8
            };
            if combined_alpha == 0 {
                return Ok(());
            }
            let (dx, dy) = *translate;
            if dx != 0 || dy != 0 {
                backend.push_translate(dx, dy)?;
            }
            let result = render_ops(backend, ops, combined_alpha);
            if dx != 0 || dy != 0 {
                backend.pop_translate()?;
            }
            result
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oasis_types::backend::{Color, SdiClipTransform, SdiCore, SdiShapes, SdiVector, TextureId};
    use oasis_types::error::Result;

    /// Minimal mock backend that records calls.
    struct MockBackend {
        calls: Vec<String>,
        translate_stack: Vec<(i32, i32)>,
    }

    impl MockBackend {
        fn new() -> Self {
            Self {
                calls: Vec::new(),
                translate_stack: Vec::new(),
            }
        }
    }

    impl SdiCore for MockBackend {
        fn init(&mut self, _w: u32, _h: u32) -> Result<()> {
            Ok(())
        }
        fn clear(&mut self, _color: Color) -> Result<()> {
            Ok(())
        }
        fn blit(&mut self, _tex: TextureId, _x: i32, _y: i32, _w: u32, _h: u32) -> Result<()> {
            Ok(())
        }
        fn fill_rect(&mut self, x: i32, y: i32, w: u32, h: u32, color: Color) -> Result<()> {
            self.calls
                .push(format!("fill_rect({x},{y},{w},{h},a={})", color.a));
            Ok(())
        }
        fn draw_text(
            &mut self,
            text: &str,
            x: i32,
            y: i32,
            font_size: u16,
            color: Color,
        ) -> Result<()> {
            self.calls.push(format!(
                "draw_text({text},{x},{y},{font_size},a={})",
                color.a
            ));
            Ok(())
        }
        fn swap_buffers(&mut self) -> Result<()> {
            Ok(())
        }
        fn load_texture(&mut self, _w: u32, _h: u32, _data: &[u8]) -> Result<TextureId> {
            Ok(TextureId(0))
        }
        fn destroy_texture(&mut self, _tex: TextureId) -> Result<()> {
            Ok(())
        }
        fn set_clip_rect(&mut self, _x: i32, _y: i32, _w: u32, _h: u32) -> Result<()> {
            Ok(())
        }
        fn reset_clip_rect(&mut self) -> Result<()> {
            Ok(())
        }
        fn measure_text(&self, _text: &str, _font_size: u16) -> u32 {
            0
        }
        fn read_pixels(&self, _x: i32, _y: i32, _w: u32, _h: u32) -> Result<Vec<u8>> {
            Ok(Vec::new())
        }
        fn shutdown(&mut self) -> Result<()> {
            Ok(())
        }
    }

    impl SdiShapes for MockBackend {
        fn fill_circle(&mut self, cx: i32, cy: i32, radius: u16, color: Color) -> Result<()> {
            self.calls
                .push(format!("fill_circle({cx},{cy},{radius},a={})", color.a));
            Ok(())
        }
        fn stroke_circle(
            &mut self,
            cx: i32,
            cy: i32,
            radius: u16,
            width: u16,
            color: Color,
        ) -> Result<()> {
            self.calls.push(format!(
                "stroke_circle({cx},{cy},{radius},{width},a={})",
                color.a
            ));
            Ok(())
        }
        fn draw_line(
            &mut self,
            x1: i32,
            y1: i32,
            x2: i32,
            y2: i32,
            width: u16,
            color: Color,
        ) -> Result<()> {
            self.calls.push(format!(
                "draw_line({x1},{y1},{x2},{y2},{width},a={})",
                color.a
            ));
            Ok(())
        }
    }
    impl SdiVector for MockBackend {
        fn fill_polygon(&mut self, points: &[(i32, i32)], color: Color) -> Result<()> {
            self.calls
                .push(format!("fill_polygon(n={},a={})", points.len(), color.a));
            Ok(())
        }
        fn stroke_polygon(
            &mut self,
            points: &[(i32, i32)],
            width: u16,
            color: Color,
        ) -> Result<()> {
            self.calls.push(format!(
                "stroke_polygon(n={},{width},a={})",
                points.len(),
                color.a
            ));
            Ok(())
        }
    }
    impl SdiClipTransform for MockBackend {
        fn push_translate(&mut self, dx: i32, dy: i32) -> Result<()> {
            self.calls.push(format!("push_translate({dx},{dy})"));
            self.translate_stack.push((dx, dy));
            Ok(())
        }
        fn pop_translate(&mut self) -> Result<()> {
            self.calls.push("pop_translate".to_string());
            self.translate_stack.pop();
            Ok(())
        }
    }
    // Records the high-level shape calls themselves (not their default
    // decomposition), so it overrides three traits and defaults the rest.
    oasis_types::impl_sdi_defaults!(MockBackend; except SdiShapes, SdiVector, SdiClipTransform);

    /// Regression: `PolygonGradient` went to `fill_polygon_gradient`,
    /// whose only implementation drew the start colour and ignored the
    /// end colour. It now fills each row with the interpolated colour.
    #[test]
    fn test_render_polygon_gradient_interpolates_rows() {
        let mut backend = MockBackend::new();
        let mut scene = VectorScene::new(100, 100);
        scene.push(VectorOp::PolygonGradient {
            points: vec![(0, 0), (10, 0), (10, 10), (0, 10)],
            color_start: Color::rgba(0, 0, 0, 0),
            color_end: Color::rgba(0, 0, 0, 200),
        });
        render_scene(&mut backend, &scene).ok();
        assert_eq!(
            backend.calls.first().map(String::as_str),
            Some("fill_rect(0,0,11,1,a=0)")
        );
        assert!(
            backend
                .calls
                .iter()
                .any(|c| c == "fill_rect(0,5,11,1,a=100)")
        );
        assert!(backend.calls.iter().all(|c| c.starts_with("fill_rect(")));

        // Equal colours keep the single polygon call.
        let mut backend = MockBackend::new();
        let mut scene = VectorScene::new(100, 100);
        scene.push(VectorOp::PolygonGradient {
            points: vec![(0, 0), (10, 0), (5, 10)],
            color_start: Color::WHITE,
            color_end: Color::WHITE,
        });
        render_scene(&mut backend, &scene).ok();
        assert_eq!(backend.calls, vec!["fill_polygon(n=3,a=255)"]);
    }

    #[test]
    fn test_render_empty_scene() {
        let mut backend = MockBackend::new();
        let scene = VectorScene::new(100, 100);
        render_scene(&mut backend, &scene).ok();
        assert!(backend.calls.is_empty());
    }

    #[test]
    fn test_render_fill_rect() {
        let mut backend = MockBackend::new();
        let mut scene = VectorScene::new(100, 100);
        scene.push(VectorOp::FillRect {
            x: 10,
            y: 20,
            w: 30,
            h: 40,
            color: Color::WHITE,
        });
        render_scene(&mut backend, &scene).ok();
        assert_eq!(backend.calls, vec!["fill_rect(10,20,30,40,a=255)"]);
    }

    #[test]
    fn test_render_group_with_translate() {
        let mut backend = MockBackend::new();
        let mut scene = VectorScene::new(100, 100);
        scene.push(VectorOp::translated(
            50,
            60,
            vec![VectorOp::FillCircle {
                cx: 0,
                cy: 0,
                radius: 10,
                color: Color::WHITE,
            }],
        ));
        render_scene(&mut backend, &scene).ok();
        assert_eq!(
            backend.calls,
            vec![
                "push_translate(50,60)",
                "fill_circle(0,0,10,a=255)",
                "pop_translate",
            ]
        );
    }

    #[test]
    fn test_render_group_alpha_modulation() {
        let mut backend = MockBackend::new();
        let mut scene = VectorScene::new(100, 100);
        scene.push(VectorOp::group_with(
            0,
            0,
            128,
            vec![VectorOp::FillRect {
                x: 0,
                y: 0,
                w: 10,
                h: 10,
                color: Color::rgba(255, 255, 255, 200),
            }],
        ));
        render_scene(&mut backend, &scene).ok();
        // 200 * 128 / 255 = 100
        assert_eq!(backend.calls, vec!["fill_rect(0,0,10,10,a=100)"]);
    }

    #[test]
    fn test_render_zero_alpha_group_skipped() {
        let mut backend = MockBackend::new();
        let mut scene = VectorScene::new(100, 100);
        scene.push(VectorOp::group_with(
            10,
            10,
            0,
            vec![VectorOp::FillCircle {
                cx: 0,
                cy: 0,
                radius: 5,
                color: Color::WHITE,
            }],
        ));
        render_scene(&mut backend, &scene).ok();
        assert!(backend.calls.is_empty());
    }

    #[test]
    fn test_render_scene_at() {
        let mut backend = MockBackend::new();
        let mut scene = VectorScene::new(100, 100);
        scene.push(VectorOp::FillCircle {
            cx: 10,
            cy: 10,
            radius: 5,
            color: Color::WHITE,
        });
        render_scene_at(&mut backend, &scene, 200, 100, 128).ok();
        assert_eq!(
            backend.calls,
            vec![
                "push_translate(200,100)",
                "fill_circle(10,10,5,a=128)",
                "pop_translate",
            ]
        );
    }

    #[test]
    fn test_render_text() {
        let mut backend = MockBackend::new();
        let mut scene = VectorScene::new(100, 100);
        scene.push(VectorOp::Text {
            text: "HELLO".to_string(),
            x: 10,
            y: 20,
            font_size: 8,
            color: Color::WHITE,
        });
        render_scene(&mut backend, &scene).ok();
        assert_eq!(backend.calls, vec!["draw_text(HELLO,10,20,8,a=255)"]);
    }
    #[test]
    fn test_render_gradient_honours_group_alpha() {
        let mut backend = MockBackend::new();
        let ops = vec![VectorOp::RectGradient {
            x: 0,
            y: 0,
            w: 4,
            h: 4,
            gradient: oasis_types::backend::GradientStyle::Vertical {
                top: Color::WHITE,
                bottom: Color::BLACK,
            },
        }];
        render_ops(&mut backend, &ops, 128).ok();
        assert_eq!(backend.calls, vec!["fill_rect(0,0,4,4,a=128)"]);
    }
}
