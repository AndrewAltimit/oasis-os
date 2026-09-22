//! Status-bar indicator glyphs: battery (5 levels + charging), AC plug and
//! Wi-Fi.
//!
//! Tiny pixel-aligned designs (10-11px tall) meant to sit next to the
//! status bar's small font; callers scale them with [`VectorOp::scale`]
//! when the bar font is larger. Every glyph recolours through its `color`
//! argument so skins drive them from their bar palette.

use oasis_types::backend::Color;

use crate::icons::IconDef;
use crate::op::VectorOp;

/// Number of discrete battery fill levels.
pub const BATTERY_LEVELS: u8 = 5;

/// Design size of [`battery`].
pub const BATTERY_SIZE: (u32, u32) = (20, 10);
/// Design size of [`ac_plug`].
pub const AC_PLUG_SIZE: (u32, u32) = (10, 10);
/// Design size of [`wifi`].
pub const WIFI_SIZE: (u32, u32) = (12, 11);

/// Map a charge percentage to a fill level in `1..=BATTERY_LEVELS`
/// (0-20% → 1, 21-40% → 2, … 81-100% → 5). An empty battery still shows
/// one sliver so the glyph never reads as "no battery".
pub fn battery_level(percent: u8) -> u8 {
    match percent {
        0..=20 => 1,
        21..=40 => 2,
        41..=60 => 3,
        61..=80 => 4,
        _ => 5,
    }
}

/// Battery: 1px outline with a terminal nub and `level` of
/// [`BATTERY_LEVELS`] cells filled. While `charging`, the cells dim and a
/// lightning bolt is drawn over them.
pub fn battery(level: u8, charging: bool, color: Color) -> IconDef {
    let level = level.min(BATTERY_LEVELS);
    let cell_color = if charging {
        Color::rgba(
            color.r,
            color.g,
            color.b,
            (color.a as u16 * 110 / 255) as u8,
        )
    } else {
        color
    };
    let mut ops = vec![
        VectorOp::StrokeRect {
            x: 0,
            y: 0,
            w: 18,
            h: 10,
            width: 1,
            color,
        },
        // Terminal nub.
        VectorOp::FillRect {
            x: 18,
            y: 3,
            w: 2,
            h: 4,
            color,
        },
    ];
    for cell in 0..level as i32 {
        ops.push(VectorOp::FillRect {
            x: 2 + cell * 3,
            y: 2,
            w: 2,
            h: 6,
            color: cell_color,
        });
    }
    if charging {
        // Zig-zag bolt from top-right to bottom-left.
        ops.push(VectorOp::FillTriangle {
            points: [(11, 1), (6, 6), (10, 6)],
            color,
        });
        ops.push(VectorOp::FillTriangle {
            points: [(8, 4), (12, 4), (7, 9)],
            color,
        });
    }
    IconDef {
        name: if charging {
            "status_battery_charging"
        } else {
            "status_battery"
        },
        ops,
        width: BATTERY_SIZE.0,
        height: BATTERY_SIZE.1,
    }
}

/// AC power: a two-prong plug with its cord (wall power, no battery).
pub fn ac_plug(color: Color) -> IconDef {
    IconDef {
        name: "status_ac_plug",
        ops: vec![
            VectorOp::FillRect {
                x: 3,
                y: 0,
                w: 1,
                h: 3,
                color,
            },
            VectorOp::FillRect {
                x: 6,
                y: 0,
                w: 1,
                h: 3,
                color,
            },
            VectorOp::FillRoundedRect {
                x: 1,
                y: 3,
                w: 8,
                h: 4,
                radius: 1,
                color,
            },
            VectorOp::FillRect {
                x: 4,
                y: 7,
                w: 2,
                h: 3,
                color,
            },
        ],
        width: AC_PLUG_SIZE.0,
        height: AC_PLUG_SIZE.1,
    }
}

/// Wi-Fi: two arcs over a dot. Disconnected (radio on, no access point)
/// draws the same shape dimmed.
pub fn wifi(connected: bool, color: Color) -> IconDef {
    let color = if connected {
        color
    } else {
        Color::rgba(color.r, color.g, color.b, (color.a as u16 * 90 / 255) as u8)
    };
    let start = core::f32::consts::PI + 0.75;
    let end = core::f32::consts::TAU - 0.75;
    let arc = |radius| VectorOp::StrokeArc {
        cx: 6,
        cy: 9,
        radius,
        start_angle: start,
        end_angle: end,
        width: 1,
        color,
    };
    IconDef {
        name: if connected {
            "status_wifi"
        } else {
            "status_wifi_off"
        },
        ops: vec![
            arc(7),
            arc(4),
            VectorOp::FillCircle {
                cx: 6,
                cy: 9,
                radius: 1,
                color,
            },
        ],
        width: WIFI_SIZE.0,
        height: WIFI_SIZE.1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::icon_set::ops_bounds;

    fn assert_inside(icon: &IconDef) {
        let (x0, y0, x1, y1) = ops_bounds(&icon.ops).expect("glyph has ops");
        assert!(
            x0 >= 0 && y0 >= 0 && x1 <= icon.width as i32 && y1 <= icon.height as i32,
            "{} bounds ({x0},{y0})-({x1},{y1}) escape {}x{}",
            icon.name,
            icon.width,
            icon.height
        );
    }

    fn cell_count(icon: &IconDef) -> usize {
        icon.ops
            .iter()
            .filter(|op| matches!(op, VectorOp::FillRect { y: 2, h: 6, .. }))
            .count()
    }

    #[test]
    fn battery_levels_cover_percent_range() {
        assert_eq!(battery_level(0), 1);
        assert_eq!(battery_level(20), 1);
        assert_eq!(battery_level(21), 2);
        assert_eq!(battery_level(50), 3);
        assert_eq!(battery_level(75), 4);
        assert_eq!(battery_level(100), BATTERY_LEVELS);
    }

    #[test]
    fn battery_draws_one_cell_per_level() {
        for level in 1..=BATTERY_LEVELS {
            let icon = battery(level, false, Color::WHITE);
            assert_eq!(cell_count(&icon), level as usize);
            assert_inside(&icon);
        }
        // Out-of-range levels clamp.
        assert_eq!(cell_count(&battery(9, false, Color::WHITE)), 5);
    }

    #[test]
    fn charging_adds_bolt_and_dims_cells() {
        let plain = battery(3, false, Color::WHITE);
        let charging = battery(3, true, Color::WHITE);
        assert_eq!(charging.name, "status_battery_charging");
        assert_eq!(charging.ops.len(), plain.ops.len() + 2);
        assert_inside(&charging);
        let dimmed = charging
            .ops
            .iter()
            .any(|op| matches!(op, VectorOp::FillRect { color, h: 6, .. } if color.a < 255));
        assert!(dimmed);
    }

    #[test]
    fn ac_and_wifi_stay_inside_their_boxes() {
        assert_inside(&ac_plug(Color::WHITE));
        assert_inside(&wifi(true, Color::WHITE));
        assert_inside(&wifi(false, Color::WHITE));
    }

    #[test]
    fn glyphs_use_the_given_color() {
        let c = Color::rgb(10, 200, 30);
        let icon = ac_plug(c);
        assert!(
            icon.ops
                .iter()
                .all(|op| matches!(op, VectorOp::FillRect { color, .. }
                    | VectorOp::FillRoundedRect { color, .. } if *color == c))
        );
        let off = wifi(false, c);
        assert!(off.ops.iter().all(|op| match op {
            VectorOp::StrokeArc { color, .. } | VectorOp::FillCircle { color, .. } => {
                color.a < 255 && color.g == 200
            },
            _ => false,
        }));
    }
}
