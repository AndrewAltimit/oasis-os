//! Altimit-inspired vector icon definitions.
//!
//! Each icon is a factory function that returns an `IconDef` parameterized by
//! theme colors. Icons are designed at a 22x22px base size (matching the
//! Altimit sidebar layout at 480x272 PSP resolution) and can be scaled by
//! embedding into a translated group.
//!
//! Icon geometries are derived from the reference SVGs in `/Downloads/svg/`,
//! particularly `altimit3.svg` (480x272 PSP-native resolution).

use oasis_types::backend::Color;

use crate::op::VectorOp;

/// A named vector icon with its operations and bounding dimensions.
#[derive(Debug, Clone)]
pub struct IconDef {
    /// Icon identifier (e.g. "the_world", "mailer").
    pub name: &'static str,
    /// Drawing operations in paint order.
    pub ops: Vec<VectorOp>,
    /// Bounding width in design pixels.
    pub width: u32,
    /// Bounding height in design pixels.
    pub height: u32,
}

impl IconDef {
    /// Recolor all ops in this icon.
    pub fn recolor(&mut self, color: Color) {
        for op in &mut self.ops {
            op.recolor(color);
        }
    }

    /// Convert this icon into a `VectorOp::Group` at the given position.
    pub fn as_op(&self, x: i32, y: i32) -> VectorOp {
        VectorOp::translated(x, y, self.ops.clone())
    }

    /// Convert this icon into a `VectorOp::Group` with opacity.
    pub fn as_op_alpha(&self, x: i32, y: i32, alpha: u8) -> VectorOp {
        VectorOp::group_with(x, y, alpha, self.ops.clone())
    }
}

// ---------------------------------------------------------------------------
// Icon: THE WORLD
// ---------------------------------------------------------------------------
// Outer square outline with an inner filled square.
// Based on altimit3.svg: rect outline 22x22 + inner filled 12x12.

/// THE WORLD icon — outer square border with inner filled square.
///
/// Represents the main desktop / home view. The inner square is meant to
/// rotate when active (animation handled separately in Phase 4).
pub fn icon_the_world(color: Color) -> IconDef {
    IconDef {
        name: "the_world",
        ops: vec![
            // Outer border
            VectorOp::StrokeRect {
                x: 0,
                y: 0,
                w: 22,
                h: 22,
                width: 2,
                color,
            },
            // Inner filled square (centered)
            VectorOp::FillRect {
                x: 5,
                y: 5,
                w: 12,
                h: 12,
                color,
            },
        ],
        width: 22,
        height: 22,
    }
}

// ---------------------------------------------------------------------------
// Icon: MAILER
// ---------------------------------------------------------------------------
// Envelope: rectangle outline with V-shaped flap.
// Based on altimit3.svg: rect M21,8 to 47,22 + flap path.

/// MAILER icon — envelope with V-shaped flap.
pub fn icon_mailer(color: Color) -> IconDef {
    IconDef {
        name: "mailer",
        ops: vec![
            // Envelope body
            VectorOp::StrokeRect {
                x: 0,
                y: 0,
                w: 22,
                h: 16,
                width: 2,
                color,
            },
            // Flap (V from top-left to center to top-right)
            VectorOp::Line {
                x1: 0,
                y1: 0,
                x2: 11,
                y2: 8,
                width: 2,
                color,
            },
            VectorOp::Line {
                x1: 11,
                y1: 8,
                x2: 22,
                y2: 0,
                width: 2,
                color,
            },
        ],
        width: 22,
        height: 16,
    }
}

// ---------------------------------------------------------------------------
// Icon: NEWS
// ---------------------------------------------------------------------------
// Stylized bold "N" as a filled polygon.
// Based on altimit3.svg: path forming block letter N.

/// NEWS icon — stylized bold letter "N" polygon.
pub fn icon_news(color: Color) -> IconDef {
    IconDef {
        name: "news",
        ops: vec![VectorOp::FillPolygon {
            points: vec![
                (0, 22),  // bottom-left
                (0, 0),   // top-left
                (6, 0),   // top of left stroke
                (15, 14), // diagonal midpoint
                (15, 0),  // top of right stroke
                (22, 0),  // top-right
                (22, 22), // bottom-right
                (16, 22), // bottom of right stroke
                (7, 8),   // diagonal midpoint (return)
                (7, 22),  // bottom of left stroke
            ],
            color,
        }],
        width: 22,
        height: 22,
    }
}

// ---------------------------------------------------------------------------
// Icon: ACCESSORY
// ---------------------------------------------------------------------------
// Stylized pen nib / plugin shape: pentagon + center line + dot.
// Based on altimit3.svg: path M34,3 L43,12 L36,26 L32,26 L25,12 Z.

/// ACCESSORY icon — pen nib / plugin connector shape.
pub fn icon_accessory(color: Color, detail_color: Color) -> IconDef {
    IconDef {
        name: "accessory",
        ops: vec![
            // Pentagon body
            VectorOp::FillPolygon {
                points: vec![
                    (11, 0),  // top center
                    (22, 11), // right
                    (16, 24), // bottom-right
                    (6, 24),  // bottom-left
                    (0, 11),  // left
                ],
                color,
            },
            // Center line
            VectorOp::Line {
                x1: 11,
                y1: 8,
                x2: 11,
                y2: 20,
                width: 2,
                color: detail_color,
            },
            // Center dot
            VectorOp::FillCircle {
                cx: 11,
                cy: 16,
                radius: 2,
                color: detail_color,
            },
        ],
        width: 22,
        height: 24,
    }
}

// ---------------------------------------------------------------------------
// Icon: AUDIO
// ---------------------------------------------------------------------------
// Play button: outer triangle outline + inner filled triangle.
// Based on altimit3.svg: outer triangle + inner pulsing triangle.

/// AUDIO icon — play button with outer outline and inner filled triangle.
///
/// The inner triangle is meant to pulse when active (Phase 4 animation).
pub fn icon_audio(color: Color) -> IconDef {
    IconDef {
        name: "audio",
        ops: vec![
            // Outer triangle outline
            VectorOp::StrokePolygon {
                points: vec![
                    (0, 0),   // top-left
                    (0, 22),  // bottom-left
                    (22, 11), // right center
                ],
                width: 2,
                color,
            },
            // Inner filled triangle (smaller, offset inward)
            VectorOp::FillPolygon {
                points: vec![
                    (4, 5),   // top
                    (4, 17),  // bottom
                    (15, 11), // right
                ],
                color,
            },
        ],
        width: 22,
        height: 22,
    }
}

// ---------------------------------------------------------------------------
// Icon: DATA
// ---------------------------------------------------------------------------
// Memory card / save icon: pentagon outline with detail lines + LED dot.
// Based on altimit3.svg: path M23,5 L38,5 L45,12 L45,26 L23,26 Z.

/// DATA icon — memory card with contact lines and status LED.
///
/// The LED dot blinks when active (Phase 4 animation).
pub fn icon_data(color: Color, led_color: Color) -> IconDef {
    IconDef {
        name: "data",
        ops: vec![
            // Card body outline (clipped corner top-right)
            VectorOp::StrokePolygon {
                points: vec![
                    (0, 0),   // top-left
                    (15, 0),  // top-right before notch
                    (22, 7),  // notch corner
                    (22, 22), // bottom-right
                    (0, 22),  // bottom-left
                ],
                width: 2,
                color,
            },
            // Contact line 1
            VectorOp::FillRect {
                x: 4,
                y: 6,
                w: 10,
                h: 2,
                color,
            },
            // Contact line 2
            VectorOp::FillRect {
                x: 4,
                y: 10,
                w: 10,
                h: 2,
                color,
            },
            // Status LED
            VectorOp::FillCircle {
                cx: 18,
                cy: 17,
                radius: 2,
                color: led_color,
            },
        ],
        width: 22,
        height: 22,
    }
}

// ---------------------------------------------------------------------------
// Background elements
// ---------------------------------------------------------------------------

/// Wireframe sphere (spinning globe effect).
///
/// Circle + horizontal ellipse + vertical ellipse + cross lines.
/// Based on altimit3.svg wireframe sphere at (390, 130) r=38.
pub fn wireframe_sphere(radius: u16, color: Color) -> IconDef {
    let r = radius as i32;
    let cx = r;
    let cy = r;
    // Approximate ellipse with arcs -- horizontal ellipse is a flat arc
    let full_circle = core::f32::consts::TAU;
    IconDef {
        name: "wireframe_sphere",
        ops: vec![
            // Main circle
            VectorOp::StrokeArc {
                cx,
                cy,
                radius,
                start_angle: 0.0,
                end_angle: full_circle,
                width: 1,
                color,
            },
            // Horizontal cross line
            VectorOp::Line {
                x1: cx - r,
                y1: cy,
                x2: cx + r,
                y2: cy,
                width: 1,
                color,
            },
            // Vertical cross line
            VectorOp::Line {
                x1: cx,
                y1: cy - r,
                x2: cx,
                y2: cy + r,
                width: 1,
                color,
            },
            // Horizontal ellipse approximation (flattened arc top half)
            VectorOp::StrokeArc {
                cx,
                cy,
                radius: radius / 3,
                start_angle: 0.0,
                end_angle: full_circle,
                width: 1,
                color,
            },
        ],
        width: (r * 2) as u32,
        height: (r * 2) as u32,
    }
}

/// Active indicator tab — thin vertical bar drawn to the left of the active
/// sidebar icon.
///
/// Based on altimit3.svg: rect x=0 y=4 width=3 height=38.
pub fn active_indicator(height: u32, color: Color) -> VectorOp {
    VectorOp::FillRect {
        x: 0,
        y: 0,
        w: 3,
        h: height,
        color,
    }
}

/// Floating glass polygon for background decoration.
///
/// Returns a translucent polygon with the given vertices and opacity.
pub fn glass_polygon(points: Vec<(i32, i32)>, color: Color, alpha: u8) -> VectorOp {
    VectorOp::FillPolygon {
        points,
        color: color.with_alpha(alpha),
    }
}

/// Grid pattern overlay — renders a sparse grid of thin lines.
///
/// Based on altimit3.svg grid pattern (30px spacing, white at 5% opacity).
pub fn grid_overlay(width: u32, height: u32, spacing: u32, color: Color) -> Vec<VectorOp> {
    let mut ops = Vec::new();
    // Vertical lines
    let mut x = spacing as i32;
    while x < width as i32 {
        ops.push(VectorOp::Line {
            x1: x,
            y1: 0,
            x2: x,
            y2: height as i32,
            width: 1,
            color,
        });
        x += spacing as i32;
    }
    // Horizontal lines
    let mut y = spacing as i32;
    while y < height as i32 {
        ops.push(VectorOp::Line {
            x1: 0,
            y1: y,
            x2: width as i32,
            y2: y,
            width: 1,
            color,
        });
        y += spacing as i32;
    }
    ops
}

/// Radar sweep arc for HUD/status display.
///
/// A filled arc wedge that represents a scanning sweep.
pub fn radar_sweep(
    cx: i32,
    cy: i32,
    radius: u16,
    sweep_angle: f32,
    rotation: f32,
    color: Color,
) -> VectorOp {
    VectorOp::FillArc {
        cx,
        cy,
        radius,
        start_angle: rotation,
        end_angle: rotation + sweep_angle,
        color,
    }
}

/// Equalizer bar — single vertical bar for audio visualizer.
pub fn eq_bar(x: i32, y: i32, width: u32, height: u32, color: Color) -> VectorOp {
    VectorOp::FillRect {
        x,
        y,
        w: width,
        h: height,
        color,
    }
}

// ---------------------------------------------------------------------------
// Sidebar layout helper
// ---------------------------------------------------------------------------

/// Standard Altimit sidebar icon set with default positioning.
///
/// Returns a scene-ready group of 6 icons laid out vertically in a 70px-wide
/// sidebar, matching the altimit3.svg layout (480x272).
///
/// `color` is the default inactive icon color. `active_color` is for the
/// active icon. `active_index` selects which icon (0-5) is highlighted.
pub fn altimit_sidebar(
    color: Color,
    active_color: Color,
    led_color: Color,
    active_index: usize,
) -> Vec<VectorOp> {
    let icons: Vec<IconDef> = vec![
        icon_the_world(color),
        icon_mailer(color),
        icon_news(color),
        icon_accessory(color, Color::BLACK),
        icon_audio(color),
        icon_data(color, led_color),
    ];

    let labels = ["THE WORLD", "MAILER", "NEWS", "ACCESSORY", "AUDIO", "DATA"];
    let y_offsets = [5, 48, 91, 134, 177, 220];
    let icon_x = 23; // left padding for icon within sidebar

    let mut ops = Vec::new();
    for (i, (icon, label)) in icons.iter().zip(labels.iter()).enumerate() {
        let y = y_offsets[i];
        let is_active = i == active_index;
        let c = if is_active { active_color } else { color };

        // Active indicator bar
        if is_active {
            ops.push(VectorOp::translated(
                0,
                y + 4,
                vec![active_indicator(38, active_color)],
            ));
        }

        // Icon (recolored if active)
        let mut icon_ops = icon.ops.clone();
        if is_active {
            for op in &mut icon_ops {
                op.recolor(c);
            }
        }
        ops.push(VectorOp::translated(icon_x, y, icon_ops));

        // Label text
        ops.push(VectorOp::Text {
            text: label.to_string(),
            x: 34,
            y: y + 38,
            font_size: 8,
            color: c,
        });
    }

    ops
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_icon_the_world() {
        let icon = icon_the_world(Color::WHITE);
        assert_eq!(icon.name, "the_world");
        assert_eq!(icon.width, 22);
        assert_eq!(icon.height, 22);
        assert_eq!(icon.ops.len(), 2);
    }

    #[test]
    fn test_icon_mailer() {
        let icon = icon_mailer(Color::WHITE);
        assert_eq!(icon.name, "mailer");
        assert_eq!(icon.ops.len(), 3);
    }

    #[test]
    fn test_icon_news() {
        let icon = icon_news(Color::WHITE);
        assert_eq!(icon.name, "news");
        assert_eq!(icon.ops.len(), 1);
        // Should be a polygon with 10 vertices
        match &icon.ops[0] {
            VectorOp::FillPolygon { points, .. } => assert_eq!(points.len(), 10),
            _ => panic!("expected FillPolygon"),
        }
    }

    #[test]
    fn test_icon_accessory() {
        let icon = icon_accessory(Color::WHITE, Color::BLACK);
        assert_eq!(icon.name, "accessory");
        assert_eq!(icon.ops.len(), 3);
    }

    #[test]
    fn test_icon_audio() {
        let icon = icon_audio(Color::WHITE);
        assert_eq!(icon.name, "audio");
        assert_eq!(icon.ops.len(), 2);
    }

    #[test]
    fn test_icon_data() {
        let icon = icon_data(Color::WHITE, Color::rgb(85, 204, 221));
        assert_eq!(icon.name, "data");
        assert_eq!(icon.ops.len(), 4);
    }

    #[test]
    fn test_icon_recolor() {
        let mut icon = icon_the_world(Color::WHITE);
        let red = Color::rgb(255, 0, 0);
        icon.recolor(red);
        match &icon.ops[0] {
            VectorOp::StrokeRect { color, .. } => assert_eq!(*color, red),
            _ => panic!("expected StrokeRect"),
        }
    }

    #[test]
    fn test_icon_as_op() {
        let icon = icon_the_world(Color::WHITE);
        let op = icon.as_op(100, 50);
        match op {
            VectorOp::Group { translate, ops, .. } => {
                assert_eq!(translate, (100, 50));
                assert_eq!(ops.len(), 2);
            },
            _ => panic!("expected Group"),
        }
    }

    #[test]
    fn test_wireframe_sphere() {
        let sphere = wireframe_sphere(38, Color::WHITE);
        assert_eq!(sphere.name, "wireframe_sphere");
        assert_eq!(sphere.width, 76);
        assert_eq!(sphere.height, 76);
        assert_eq!(sphere.ops.len(), 4);
    }

    #[test]
    fn test_grid_overlay() {
        let grid = grid_overlay(100, 100, 30, Color::WHITE);
        // 3 vertical lines (30, 60, 90) + 3 horizontal lines (30, 60, 90) = 6
        assert_eq!(grid.len(), 6);
    }

    #[test]
    fn test_altimit_sidebar() {
        let ops = altimit_sidebar(
            Color::rgb(136, 136, 136),
            Color::rgb(85, 204, 221),
            Color::rgb(85, 204, 221),
            0,
        );
        // 6 icons * (icon group + label text) + 1 active indicator = 13
        assert_eq!(ops.len(), 13);
    }

    #[test]
    fn test_altimit_sidebar_different_active() {
        let ops = altimit_sidebar(
            Color::rgb(136, 136, 136),
            Color::rgb(85, 204, 221),
            Color::rgb(85, 204, 221),
            3,
        );
        // Same count: 6 icons + 6 labels + 1 active indicator
        assert_eq!(ops.len(), 13);
    }

    #[test]
    fn test_glass_polygon() {
        let op = glass_polygon(vec![(50, 272), (200, -20), (350, 272)], Color::WHITE, 38);
        match op {
            VectorOp::FillPolygon { color, points } => {
                assert_eq!(color.a, 38);
                assert_eq!(points.len(), 3);
            },
            _ => panic!("expected FillPolygon"),
        }
    }

    #[test]
    fn test_radar_sweep() {
        let op = radar_sweep(100, 100, 50, 0.5, 1.0, Color::rgb(255, 128, 0));
        match op {
            VectorOp::FillArc {
                start_angle,
                end_angle,
                ..
            } => {
                assert!((start_angle - 1.0).abs() < 0.001);
                assert!((end_angle - 1.5).abs() < 0.001);
            },
            _ => panic!("expected FillArc"),
        }
    }
}
