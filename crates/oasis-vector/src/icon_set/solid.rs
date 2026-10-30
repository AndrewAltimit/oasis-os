//! `solid` preset: filled glyphs on the 24x24 grid.
use super::*;

// ---------------------------------------------------------------------------
// SOLID SET — filled shapes, high-contrast.
// ---------------------------------------------------------------------------

/// Return the solid-style icon for a category.
pub fn solid_icon(category: IconCategory, color: Color) -> IconDef {
    match category {
        IconCategory::Browser => solid_browser(color),
        IconCategory::Files => solid_files(color),
        IconCategory::Audio => solid_audio(color),
        IconCategory::Tv => solid_tv(color),
        IconCategory::Radio => solid_radio(color),
        IconCategory::Settings => solid_settings(color),
        IconCategory::Video => solid_video(color),
        IconCategory::Home => solid_home(color),
        IconCategory::Network => solid_network(color),
        IconCategory::Power => solid_power(color),
        IconCategory::Gallery => solid_gallery(color),
        IconCategory::Weather => solid_weather(color),
        IconCategory::Terminal => solid_terminal(color),
        IconCategory::Packages => solid_packages(color),
        IconCategory::Monitor => solid_monitor(color),
        IconCategory::Calculator => solid_calculator(color),
        IconCategory::Paint => solid_paint(color),
        IconCategory::TextEditor => solid_text_editor(color),
        IconCategory::Games => solid_games(color),
        IconCategory::Generic => solid_generic(color),
    }
}

/// Solid terminal: filled window + contrasting ">_" glyph.
fn solid_terminal(color: Color) -> IconDef {
    IconDef {
        name: "solid_terminal",
        ops: vec![
            VectorOp::FillRoundedRect {
                x: 2,
                y: 5,
                w: 20,
                h: 14,
                radius: 2,
                color,
            },
            // ">" prompt.
            VectorOp::FillPolygon {
                points: vec![(6, 10), (9, 12), (6, 14)],
                color: Color::rgba(255, 255, 255, 230),
            },
            // Underscore cursor.
            VectorOp::FillRect {
                x: 11,
                y: 13,
                w: 5,
                h: 2,
                color: Color::rgba(255, 255, 255, 230),
            },
        ],
        width: SIZE,
        height: SIZE,
    }
}

/// Solid globe: filled disc with a longitude wedge cut by a crosshair.
fn solid_browser(color: Color) -> IconDef {
    IconDef {
        name: "solid_browser",
        ops: vec![
            VectorOp::FillCircle {
                cx: 12,
                cy: 12,
                radius: 10,
                color,
            },
            // Knock-out longitudes / equator so the filled disc reads as a globe.
            VectorOp::Line {
                x1: 2,
                y1: 12,
                x2: 22,
                y2: 12,
                width: 2,
                color: Color::rgba(0, 0, 0, 110),
            },
            VectorOp::Line {
                x1: 12,
                y1: 2,
                x2: 12,
                y2: 22,
                width: 2,
                color: Color::rgba(0, 0, 0, 110),
            },
            VectorOp::StrokeCircle {
                cx: 12,
                cy: 12,
                radius: 5,
                width: 2,
                color: Color::rgba(0, 0, 0, 110),
            },
        ],
        width: SIZE,
        height: SIZE,
    }
}

/// Solid folder: filled tabbed shape.
fn solid_files(color: Color) -> IconDef {
    IconDef {
        name: "solid_files",
        ops: vec![VectorOp::FillPolygon {
            points: vec![(2, 5), (9, 5), (11, 8), (22, 8), (22, 21), (2, 21)],
            color,
        }],
        width: SIZE,
        height: SIZE,
    }
}

/// Headphones: filled U-band + two earcups.
fn solid_audio(color: Color) -> IconDef {
    IconDef {
        name: "solid_audio",
        ops: vec![
            // Band arc (top half of an outer ring).
            VectorOp::StrokeArc {
                cx: 12,
                cy: 12,
                radius: 9,
                start_angle: core::f32::consts::PI,
                end_angle: core::f32::consts::TAU,
                width: 3,
                color,
            },
            // Left earcup.
            VectorOp::FillRoundedRect {
                x: 3,
                y: 12,
                w: 5,
                h: 9,
                radius: 2,
                color,
            },
            // Right earcup.
            VectorOp::FillRoundedRect {
                x: 16,
                y: 12,
                w: 5,
                h: 9,
                radius: 2,
                color,
            },
        ],
        width: SIZE,
        height: SIZE,
    }
}

/// Solid TV: filled casing with cut-out screen + antenna.
fn solid_tv(color: Color) -> IconDef {
    IconDef {
        name: "solid_tv",
        ops: vec![
            VectorOp::FillRoundedRect {
                x: 3,
                y: 7,
                w: 18,
                h: 14,
                radius: 2,
                color,
            },
            // Screen knock-out.
            VectorOp::FillRect {
                x: 5,
                y: 9,
                w: 11,
                h: 8,
                color: Color::rgba(0, 0, 0, 130),
            },
            // Knob dots.
            VectorOp::FillCircle {
                cx: 18,
                cy: 11,
                radius: 1,
                color: Color::rgba(0, 0, 0, 130),
            },
            VectorOp::FillCircle {
                cx: 18,
                cy: 15,
                radius: 1,
                color: Color::rgba(0, 0, 0, 130),
            },
            // Antennas.
            VectorOp::Line {
                x1: 17,
                y1: 2,
                x2: 12,
                y2: 7,
                width: 2,
                color,
            },
            VectorOp::Line {
                x1: 12,
                y1: 7,
                x2: 7,
                y2: 2,
                width: 2,
                color,
            },
        ],
        width: SIZE,
        height: SIZE,
    }
}

/// Boombox: filled body + cut-out speaker + handle strap.
fn solid_radio(color: Color) -> IconDef {
    IconDef {
        name: "solid_radio",
        ops: vec![
            // Handle strap (top).
            VectorOp::FillRoundedRect {
                x: 6,
                y: 3,
                w: 12,
                h: 4,
                radius: 1,
                color,
            },
            // Body.
            VectorOp::FillRoundedRect {
                x: 2,
                y: 6,
                w: 20,
                h: 15,
                radius: 2,
                color,
            },
            // Speaker cut-out.
            VectorOp::FillCircle {
                cx: 9,
                cy: 15,
                radius: 3,
                color: Color::rgba(0, 0, 0, 130),
            },
            VectorOp::StrokeCircle {
                cx: 9,
                cy: 15,
                radius: 1,
                width: 1,
                color,
            },
            // LED row.
            VectorOp::FillRect {
                x: 14,
                y: 12,
                w: 5,
                h: 2,
                color: Color::rgba(0, 0, 0, 130),
            },
            VectorOp::FillRect {
                x: 14,
                y: 16,
                w: 5,
                h: 2,
                color: Color::rgba(0, 0, 0, 130),
            },
        ],
        width: SIZE,
        height: SIZE,
    }
}

/// Solid gear: radial teeth + cut-out hub.
fn solid_settings(color: Color) -> IconDef {
    let teeth = gear_teeth_polygon(12, 12, 11, 8, 8);
    IconDef {
        name: "solid_settings",
        ops: vec![
            VectorOp::FillPolygon {
                points: teeth,
                color,
            },
            VectorOp::FillCircle {
                cx: 12,
                cy: 12,
                radius: 3,
                color: Color::rgba(0, 0, 0, 150),
            },
        ],
        width: SIZE,
        height: SIZE,
    }
}

/// YouTube-style play badge: filled rounded rect + white play triangle.
fn solid_video(color: Color) -> IconDef {
    IconDef {
        name: "solid_video",
        ops: vec![
            VectorOp::FillRoundedRect {
                x: 2,
                y: 5,
                w: 20,
                h: 14,
                radius: 3,
                color,
            },
            VectorOp::FillPolygon {
                points: vec![(10, 9), (16, 12), (10, 15)],
                color: Color::rgba(255, 255, 255, 230),
            },
        ],
        width: SIZE,
        height: SIZE,
    }
}

/// Solid home: filled house silhouette with chimney notch.
fn solid_home(color: Color) -> IconDef {
    IconDef {
        name: "solid_home",
        ops: vec![
            VectorOp::FillPolygon {
                points: vec![
                    (2, 12),
                    (12, 3),
                    (22, 12),
                    (19, 12),
                    (19, 21),
                    (5, 21),
                    (5, 12),
                ],
                color,
            },
            // Door knock-out.
            VectorOp::FillRect {
                x: 10,
                y: 14,
                w: 4,
                h: 7,
                color: Color::rgba(0, 0, 0, 150),
            },
        ],
        width: SIZE,
        height: SIZE,
    }
}

/// Solid Wi-Fi: three filled wedges, strongest at outer.
fn solid_network(color: Color) -> IconDef {
    // Two thick arcs over a dot. The sweep and radii keep the outer arc's
    // ends (and its 3px stroke) inside the 24x24 box.
    let start = core::f32::consts::PI + 0.5;
    let end = core::f32::consts::TAU - 0.5;
    let arc = |radius| VectorOp::StrokeArc {
        cx: 12,
        cy: 19,
        radius,
        start_angle: start,
        end_angle: end,
        width: 3,
        color,
    };
    IconDef {
        name: "solid_network",
        ops: vec![
            arc(12),
            arc(7),
            VectorOp::FillCircle {
                cx: 12,
                cy: 19,
                radius: 2,
                color,
            },
        ],
        width: SIZE,
        height: SIZE,
    }
}

/// Solid power: filled ring-with-gap + bar.
fn solid_power(color: Color) -> IconDef {
    let start = -core::f32::consts::FRAC_PI_2 + 0.7;
    let end = -core::f32::consts::FRAC_PI_2 + core::f32::consts::TAU - 0.7;
    IconDef {
        name: "solid_power",
        ops: vec![
            VectorOp::StrokeArc {
                cx: 12,
                cy: 13,
                radius: 9,
                start_angle: start,
                end_angle: end,
                width: 3,
                color,
            },
            VectorOp::FillRect {
                x: 11,
                y: 2,
                w: 2,
                h: 11,
                color,
            },
        ],
        width: SIZE,
        height: SIZE,
    }
}

/// Solid gallery: filled frame with cut-out mountain + sun.
fn solid_gallery(color: Color) -> IconDef {
    IconDef {
        name: "solid_gallery",
        ops: vec![
            VectorOp::FillRoundedRect {
                x: 3,
                y: 3,
                w: 18,
                h: 18,
                radius: 2,
                color,
            },
            // Sun.
            VectorOp::FillCircle {
                cx: 8,
                cy: 8,
                radius: 2,
                color: Color::rgba(255, 255, 255, 220),
            },
            // Mountain knock-out.
            VectorOp::FillPolygon {
                points: vec![(3, 20), (9, 12), (13, 16), (17, 10), (21, 20)],
                color: Color::rgba(0, 0, 0, 150),
            },
        ],
        width: SIZE,
        height: SIZE,
    }
}

/// Solid cloud: three overlapping filled circles + base rect.
fn solid_weather(color: Color) -> IconDef {
    IconDef {
        name: "solid_weather",
        ops: vec![
            VectorOp::FillCircle {
                cx: 8,
                cy: 14,
                radius: 5,
                color,
            },
            VectorOp::FillCircle {
                cx: 13,
                cy: 11,
                radius: 5,
                color,
            },
            VectorOp::FillCircle {
                cx: 17,
                cy: 14,
                radius: 4,
                color,
            },
            VectorOp::FillRect {
                x: 6,
                y: 14,
                w: 13,
                h: 5,
                color,
            },
        ],
        width: SIZE,
        height: SIZE,
    }
}

/// Solid generic: filled rounded square with a cut-out dot.
fn solid_generic(color: Color) -> IconDef {
    IconDef {
        name: "solid_generic",
        ops: vec![
            VectorOp::FillRoundedRect {
                x: 3,
                y: 3,
                w: 18,
                h: 18,
                radius: 3,
                color,
            },
            VectorOp::FillCircle {
                cx: 12,
                cy: 12,
                radius: 3,
                color: Color::rgba(0, 0, 0, 150),
            },
        ],
        width: SIZE,
        height: SIZE,
    }
}

/// Solid package: filled isometric box with dark seams.
fn solid_packages(color: Color) -> IconDef {
    let cut = Color::rgba(0, 0, 0, 150);
    IconDef {
        name: "solid_packages",
        ops: vec![
            VectorOp::FillPolygon {
                points: vec![(12, 2), (21, 7), (21, 17), (12, 22), (3, 17), (3, 7)],
                color,
            },
            line(3, 7, 12, 12, 1, cut),
            line(12, 12, 21, 7, 1, cut),
            line(12, 12, 12, 22, 1, cut),
        ],
        width: SIZE,
        height: SIZE,
    }
}

/// Solid system monitor: filled display with a knocked-out trace.
fn solid_monitor(color: Color) -> IconDef {
    let cut = Color::rgba(0, 0, 0, 150);
    IconDef {
        name: "solid_monitor",
        ops: vec![
            VectorOp::FillRoundedRect {
                x: 2,
                y: 3,
                w: 20,
                h: 14,
                radius: 2,
                color,
            },
            line(5, 10, 8, 10, STROKE, cut),
            line(8, 10, 10, 6, STROKE, cut),
            line(10, 6, 13, 14, STROKE, cut),
            line(13, 14, 15, 10, STROKE, cut),
            line(15, 10, 19, 10, STROKE, cut),
            VectorOp::FillRect {
                x: 11,
                y: 17,
                w: 2,
                h: 3,
                color,
            },
            VectorOp::FillRect {
                x: 7,
                y: 20,
                w: 10,
                h: 2,
                color,
            },
        ],
        width: SIZE,
        height: SIZE,
    }
}

/// Solid calculator: filled body with a dark display and key grid.
fn solid_calculator(color: Color) -> IconDef {
    let cut = Color::rgba(0, 0, 0, 150);
    let mut ops = vec![
        VectorOp::FillRoundedRect {
            x: 5,
            y: 2,
            w: 14,
            h: 20,
            radius: 2,
            color,
        },
        VectorOp::FillRect {
            x: 8,
            y: 5,
            w: 8,
            h: 3,
            color: cut,
        },
    ];
    for row in 0..3 {
        for col in 0..3 {
            ops.push(VectorOp::FillRect {
                x: 8 + col * 3,
                y: 10 + row * 3,
                w: 2,
                h: 2,
                color: cut,
            });
        }
    }
    IconDef {
        name: "solid_calculator",
        ops,
        width: SIZE,
        height: SIZE,
    }
}

/// Solid paint: filled palette disc with knocked-out dots and thumb hole.
fn solid_paint(color: Color) -> IconDef {
    let cut = Color::rgba(0, 0, 0, 150);
    let dot = |cx, cy, radius| VectorOp::FillCircle {
        cx,
        cy,
        radius,
        color: cut,
    };
    IconDef {
        name: "solid_paint",
        ops: vec![
            VectorOp::FillCircle {
                cx: 12,
                cy: 12,
                radius: 10,
                color,
            },
            dot(8, 9, 2),
            dot(12, 6, 2),
            dot(16, 9, 2),
            dot(14, 16, 2),
        ],
        width: SIZE,
        height: SIZE,
    }
}

/// Solid text editor: filled page with dark ruled lines.
fn solid_text_editor(color: Color) -> IconDef {
    let cut = Color::rgba(0, 0, 0, 150);
    IconDef {
        name: "solid_text_editor",
        ops: vec![
            VectorOp::FillPolygon {
                points: vec![(5, 2), (14, 2), (19, 7), (19, 22), (5, 22)],
                color,
            },
            VectorOp::FillTriangle {
                points: [(14, 2), (19, 7), (14, 7)],
                color: cut,
            },
            line(8, 11, 16, 11, 1, cut),
            line(8, 14, 16, 14, 1, cut),
            line(8, 17, 16, 17, 1, cut),
            line(8, 20, 13, 20, 1, cut),
        ],
        width: SIZE,
        height: SIZE,
    }
}

/// Solid games: filled gamepad with a dark d-pad and buttons.
fn solid_games(color: Color) -> IconDef {
    let cut = Color::rgba(0, 0, 0, 150);
    IconDef {
        name: "solid_games",
        ops: vec![
            VectorOp::FillRoundedRect {
                x: 2,
                y: 7,
                w: 20,
                h: 11,
                radius: 5,
                color,
            },
            VectorOp::FillRect {
                x: 4,
                y: 11,
                w: 6,
                h: 2,
                color: cut,
            },
            VectorOp::FillRect {
                x: 6,
                y: 9,
                w: 2,
                h: 6,
                color: cut,
            },
            VectorOp::FillCircle {
                cx: 15,
                cy: 11,
                radius: 1,
                color: cut,
            },
            VectorOp::FillCircle {
                cx: 18,
                cy: 14,
                radius: 1,
                color: cut,
            },
        ],
        width: SIZE,
        height: SIZE,
    }
}
