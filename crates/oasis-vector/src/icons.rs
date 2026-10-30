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

use crate::anim;
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
// Animated icon variants
// ---------------------------------------------------------------------------

/// THE WORLD with spinning inner square.
///
/// The outer border is static; the inner filled square rotates by `angle`.
pub fn icon_the_world_animated(color: Color, angle: f32) -> IconDef {
    IconDef {
        name: "the_world",
        ops: vec![
            // Outer border (static)
            VectorOp::StrokeRect {
                x: 0,
                y: 0,
                w: 22,
                h: 22,
                width: 2,
                color,
            },
            // Inner filled square (rotated)
            anim::rotate_rect(5, 5, 12, 12, angle, color),
        ],
        width: 22,
        height: 22,
    }
}

/// AUDIO with pulsing inner triangle.
///
/// The outer triangle outline is static; the inner triangle's alpha oscillates.
pub fn icon_audio_animated(color: Color, alpha: u8) -> IconDef {
    let mut inner_color = color;
    inner_color.a = ((color.a as u16 * alpha as u16) / 255) as u8;
    IconDef {
        name: "audio",
        ops: vec![
            // Outer triangle outline (static)
            VectorOp::StrokePolygon {
                points: vec![
                    (0, 0),   // top-left
                    (0, 22),  // bottom-left
                    (22, 11), // right center
                ],
                width: 2,
                color,
            },
            // Inner filled triangle (pulsing alpha)
            VectorOp::FillPolygon {
                points: vec![
                    (4, 5),   // top
                    (4, 17),  // bottom
                    (15, 11), // right
                ],
                color: inner_color,
            },
        ],
        width: 22,
        height: 22,
    }
}

/// DATA with blinking LED.
///
/// The card body is static; the LED circle visibility is controlled by `led_visible`.
pub fn icon_data_animated(color: Color, led_color: Color, led_visible: bool) -> IconDef {
    let mut ops = vec![
        // Card body outline
        VectorOp::StrokePolygon {
            points: vec![(0, 0), (15, 0), (22, 7), (22, 22), (0, 22)],
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
    ];
    if led_visible {
        ops.push(VectorOp::FillCircle {
            cx: 18,
            cy: 17,
            radius: 2,
            color: led_color,
        });
    }
    IconDef {
        name: "data",
        ops,
        width: 22,
        height: 22,
    }
}

// ---------------------------------------------------------------------------
// Background elements
// ---------------------------------------------------------------------------

/// Grid pattern overlay — renders a sparse grid of thin lines.
///
/// Based on altimit3.svg grid pattern (30px spacing, white at 5% opacity).
pub fn grid_overlay(width: u32, height: u32, spacing: u32, color: Color) -> Vec<VectorOp> {
    crate::backgrounds::grid(width, height, spacing, color)
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
        let __out = &icon.ops[0];
        let VectorOp::FillPolygon { points, .. } = __out else {
            panic!("expected FillPolygon, got {__out:?}");
        };
        assert_eq!(points.len(), 10);
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
        let __out = &icon.ops[0];
        let VectorOp::StrokeRect { color, .. } = __out else {
            panic!("expected StrokeRect, got {__out:?}");
        };
        assert_eq!(*color, red);
    }

    #[test]
    fn test_icon_as_op() {
        let icon = icon_the_world(Color::WHITE);
        let op = icon.as_op(100, 50);
        let VectorOp::Group { translate, ops, .. } = op else {
            panic!("expected Group, got {op:?}");
        };
        assert_eq!(translate, (100, 50));
        assert_eq!(ops.len(), 2);
    }

    #[test]
    fn test_grid_overlay_zero_spacing_is_empty() {
        assert!(grid_overlay(100, 100, 0, Color::WHITE).is_empty());
    }

    #[test]
    fn test_grid_overlay() {
        let grid = grid_overlay(100, 100, 30, Color::WHITE);
        // 3 vertical lines (30, 60, 90) + 3 horizontal lines (30, 60, 90) = 6
        assert_eq!(grid.len(), 6);
    }
}
